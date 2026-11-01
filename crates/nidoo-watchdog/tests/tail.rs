use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nidoo_watchdog::tail;
use tokio::sync::mpsc;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nidoo-watchdog-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join("mosquitto.log")
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .expect("open");
    f.write_all(text.as_bytes()).expect("write");
}

async fn next(rx: &mut mpsc::Receiver<String>) -> String {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a line within 5 s")
        .expect("channel open")
}

#[tokio::test]
async fn tail_emits_appended_lines_only() {
    let path = scratch("appended");
    std::fs::write(&path, "history 1\nhistory 2\n").expect("seed");
    let (tx, mut rx) = mpsc::channel(8);
    let task = tail(path.clone(), tx);

    append(&path, "first\nsecond\n");
    assert_eq!(next(&mut rx).await, "first");
    assert_eq!(next(&mut rx).await, "second");
    assert!(rx.try_recv().is_err(), "history must not replay");
    task.abort();
}

#[tokio::test]
async fn tail_follows_truncation() {
    let path = scratch("truncated");
    std::fs::write(&path, "history line\n").expect("seed");
    let (tx, mut rx) = mpsc::channel(8);
    let task = tail(path.clone(), tx);

    append(&path, "before\n");
    assert_eq!(next(&mut rx).await, "before");
    std::fs::write(&path, "after\n").expect("truncate");
    assert_eq!(next(&mut rx).await, "after");
    task.abort();
}

#[tokio::test]
async fn tail_waits_for_the_end_of_a_partial_line() {
    let path = scratch("partial");
    std::fs::write(&path, "").expect("seed");
    let (tx, mut rx) = mpsc::channel(8);
    let task = tail(path.clone(), tx);

    append(&path, "half");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(rx.try_recv().is_err(), "no line before its newline");
    append(&path, " done\n");
    assert_eq!(next(&mut rx).await, "half done");
    task.abort();
}

#[tokio::test]
async fn tail_reads_a_replaced_file_from_the_start() {
    let path = scratch("rotated");
    std::fs::write(&path, "prelude\n").expect("seed");
    let (tx, mut rx) = mpsc::channel(8);
    let task = tail(path.clone(), tx);

    std::fs::remove_file(&path).expect("unlink");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    std::fs::write(
        &path,
        "after rotation\nDenied PUBLISH from X (q=0, 'y/z')\n",
    )
    .expect("recreate");
    assert_eq!(next(&mut rx).await, "after rotation");
    assert_eq!(next(&mut rx).await, "Denied PUBLISH from X (q=0, 'y/z')");
    task.abort();
}
