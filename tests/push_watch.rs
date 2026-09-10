use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::tempdir;

#[tokio::test]
async fn notify_watch_reports_one_burst_after_debounce() {
    let directory = tempdir().unwrap();
    let file = directory.path().join("main.js");
    std::fs::write(&file, "function main() {}\n").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&seen);
    let stop = crsp::commands::push::watch_files_filtered(
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
    let stop = crsp::commands::push::watch_files_filtered(
        directory.path(),
        Duration::from_millis(40),
        |path| path.extension().and_then(|ext| ext.to_str()) != Some("txt"),
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
    let result = crsp::commands::push::watch_files_filtered(
        &directory.path().join("missing"),
        Duration::from_millis(10),
        |_| true,
        |_| Box::pin(async { Ok(false) }),
    )
    .await;
    assert!(result.is_err());
}
