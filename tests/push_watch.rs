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
    let stop = crsp::commands::push::watch_files(
        directory.path(),
        Duration::from_millis(50),
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
