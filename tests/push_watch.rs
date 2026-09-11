use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::tempdir;

#[test]
fn push_watch_pins_the_clasp_debounce_timer_and_banner() {
    // clamp the spec §2.6 values: 500ms debounce and the `Waiting for
    // changes...` banner (clasp push.ts:141) so drift fails the suite.
    assert_eq!(google_clasp_rs::commands::push::WATCH_DEBOUNCE_MS, 500);
    assert_eq!(
        google_clasp_rs::commands::push::MESSAGE_WAITING_FOR_CHANGES,
        "Waiting for changes..."
    );
}

#[test]
fn watcher_ignore_matching_uses_project_root_for_src_dir() {
    let directory = tempdir().unwrap();
    let content = directory.path().join("src");
    std::fs::create_dir(&content).unwrap();
    let ignored = content.join("ignored.js");
    let tracked = content.join("tracked.js");
    let matcher =
        google_clasp_rs::core::ignore::IgnoreMatcher::from_patterns(["src/ignored.js"]).unwrap();

    assert!(!google_clasp_rs::commands::push::is_tracked_event_path(
        directory.path(),
        &matcher,
        &ignored,
    ));
    assert!(google_clasp_rs::commands::push::is_tracked_event_path(
        directory.path(),
        &matcher,
        &tracked,
    ));
    assert!(Path::new("src/ignored.js").starts_with("src"));
}

#[tokio::test]
async fn watcher_filter_rejects_ignored_src_dir_events() {
    let directory = tempdir().unwrap();
    let content = directory.path().join("src");
    std::fs::create_dir(&content).unwrap();
    let ignored = content.join("ignored.js");
    let seen = Arc::new(Mutex::new(0));
    let received = Arc::clone(&seen);
    let stop = google_clasp_rs::commands::push::watch_files_filtered(
        &content,
        Duration::from_millis(40),
        move |path| path.file_name().and_then(|name| name.to_str()) != Some("ignored.js"),
        move |_| {
            let received = Arc::clone(&received);
            Box::pin(async move {
                *received.lock().unwrap() += 1;
                Ok(false)
            })
        },
    );
    let writer = async {
        tokio::time::sleep(Duration::from_millis(80)).await;
        std::fs::write(&ignored, "ignored").unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
    };
    let (_, result) = tokio::join!(
        writer,
        tokio::time::timeout(Duration::from_millis(300), stop)
    );
    assert!(result.is_err());
    assert_eq!(*seen.lock().unwrap(), 0);
}

#[tokio::test]
async fn notify_watch_reports_one_burst_after_debounce() {
    let directory = tempdir().unwrap();
    let file = directory.path().join("main.js");
    std::fs::write(&file, "function main() {}\n").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&seen);
    let stop = google_clasp_rs::commands::push::watch_files_filtered(
        directory.path(),
        Duration::from_millis(50),
        |_| true,
        move |paths| {
            let received = Arc::clone(&received);
            Box::pin(async move {
                received.lock().unwrap().push(paths);
                Ok(false)
            })
        },
    );
    let writer = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::write(&file, "function main() { return 1; }\n").unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        std::fs::write(&file, "function main() { return 2; }\n").unwrap();
    };
    let (_, result) = tokio::join!(writer, stop);
    result.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(
        seen.lock().unwrap()[0]
            .iter()
            .any(|path| path.ends_with("main.js"))
    );
}

#[tokio::test]
async fn ignored_events_do_not_trigger_callback() {
    let directory = tempdir().unwrap();
    let file = directory.path().join("ignored.txt");
    let seen = Arc::new(Mutex::new(0));
    let received = Arc::clone(&seen);
    let stop = google_clasp_rs::commands::push::watch_files_filtered(
        directory.path(),
        Duration::from_millis(40),
        // Reject the ignored `.txt` file and directory-level events alike; a
        // bare `extension != "txt"` check lets FSEvents' directory events
        // through, which would spuriously stop the watcher on macOS.
        |path| path.is_file() && path.extension().and_then(|ext| ext.to_str()) != Some("txt"),
        move |_| {
            let received = Arc::clone(&received);
            Box::pin(async move {
                *received.lock().unwrap() += 1;
                Ok(false)
            })
        },
    );
    let writer = async {
        tokio::time::sleep(Duration::from_millis(80)).await;
        std::fs::write(&file, "ignored").unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
    };
    let (_, result) = tokio::join!(
        writer,
        tokio::time::timeout(Duration::from_millis(300), stop)
    );
    assert!(result.is_err());
    assert_eq!(*seen.lock().unwrap(), 0);
}

#[tokio::test]
async fn watcher_errors_are_propagated() {
    let directory = tempdir().unwrap();
    let result = google_clasp_rs::commands::push::watch_files_filtered(
        &directory.path().join("missing"),
        Duration::from_millis(10),
        |_| true,
        |_| Box::pin(async { Ok(false) }),
    )
    .await;
    assert!(result.is_err());
}
