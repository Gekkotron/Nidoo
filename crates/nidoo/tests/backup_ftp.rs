//! The FTP store against a local libunftp server over a temp directory. No database needed.

use crate::common;

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use common::ftp::ftp_server;
use nidoo::backup::BackupError;
use nidoo::backup::ftp::{FtpSettings, FtpStore};
use nidoo::backup::name::file_name;
use nidoo_api::FtpStep;
use tokio::io::AsyncReadExt;

fn settings(port: u16, folder: &str) -> FtpSettings {
    FtpSettings {
        host: "127.0.0.1".into(),
        port,
        user: "nidoo".into(),
        password: "hunter2-secret".into(),
        folder: folder.into(),
    }
}

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, 22, 45, 12).unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn upload_lists_and_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let port = ftp_server(dir.path(), None).await;
    let mut store = FtpStore::connect(&settings(port, "/")).await.unwrap();

    let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let name = file_name(at(11));
    let sent = store.upload(&name, &payload[..]).await.unwrap();
    assert_eq!(sent, payload.len() as u64);
    assert_eq!(names(dir.path()), vec![name.clone()], "no .partial left");

    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].file, name);
    assert_eq!(listed[0].size, payload.len() as u64);
    assert_eq!(listed[0].created_at, at(11).timestamp());

    let mut stream = store.download(&name).await.unwrap();
    let mut got = Vec::new();
    stream.read_to_end(&mut got).await.unwrap();
    store.finish_download(stream).await.unwrap();
    assert_eq!(got, payload);

    // The connection is still good afterwards.
    assert_eq!(store.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn partial_is_never_listed() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join(format!("{}.partial", file_name(at(10)))),
        b"half",
    )
    .unwrap();
    let port = ftp_server(dir.path(), None).await;
    let mut store = FtpStore::connect(&settings(port, "/")).await.unwrap();
    assert!(store.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn list_is_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    for d in [3, 9, 5] {
        fs::write(dir.path().join(file_name(at(d))), b"x").unwrap();
    }
    let port = ftp_server(dir.path(), None).await;
    let mut store = FtpStore::connect(&settings(port, "/")).await.unwrap();
    let files: Vec<String> = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.file)
        .collect();
    assert_eq!(
        files,
        vec![file_name(at(9)), file_name(at(5)), file_name(at(3))]
    );
}

#[tokio::test]
async fn prune_keeps_n_and_ignores_strangers() {
    let dir = tempfile::tempdir().unwrap();
    for d in 1..=9 {
        fs::write(dir.path().join(file_name(at(d))), b"x").unwrap();
    }
    fs::write(dir.path().join("notes.txt"), b"mine").unwrap();
    let fresh = format!("{}.partial", file_name(at(10)));
    fs::write(dir.path().join(&fresh), b"half").unwrap();
    let old = format!("{}.partial", file_name(at(8)));
    let old_file = fs::File::create(dir.path().join(&old)).unwrap();
    old_file
        .set_modified(SystemTime::now() - Duration::from_secs(2 * 24 * 3600))
        .unwrap();
    drop(old_file);

    let port = ftp_server(dir.path(), None).await;
    let mut store = FtpStore::connect(&settings(port, "/")).await.unwrap();
    let mut deleted = store.prune(7, Utc::now()).await.unwrap();
    deleted.sort();

    let mut expected = vec![file_name(at(1)), file_name(at(2)), old.clone()];
    expected.sort();
    assert_eq!(deleted, expected);

    let left = names(dir.path());
    assert!(left.contains(&"notes.txt".to_string()));
    assert!(left.contains(&fresh));
    assert!(!left.contains(&old));
    assert_eq!(left.len(), 7 + 2);
    for d in 3..=9 {
        assert!(left.contains(&file_name(at(d))));
    }
}

#[tokio::test]
async fn prune_with_fewer_backups_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(file_name(at(1))), b"x").unwrap();
    let port = ftp_server(dir.path(), None).await;
    let mut store = FtpStore::connect(&settings(port, "/")).await.unwrap();
    assert!(store.prune(7, Utc::now()).await.unwrap().is_empty());
    assert_eq!(names(dir.path()).len(), 1);
}

#[tokio::test]
async fn connect_errors_map_to_their_variants() {
    let dir = tempfile::tempdir().unwrap();
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    assert!(matches!(
        FtpStore::connect(&settings(closed, "/")).await,
        Err(BackupError::Unreachable(_))
    ));

    let port = ftp_server(dir.path(), Some(("nidoo", "right"))).await;
    let err = FtpStore::connect(&settings(port, "/")).await.err().unwrap();
    assert_eq!(err, BackupError::LoginRefused);

    let port = ftp_server(dir.path(), None).await;
    let err = FtpStore::connect(&settings(port, "/missing"))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, BackupError::Folder(_)));
    assert!(!err.to_string().contains("hunter2"));
}

#[tokio::test]
async fn probe_reports_the_failing_step() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("backups")).unwrap();

    // Closed port.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let checks = FtpStore::probe(&settings(closed, "/")).await;
    assert_eq!(checks.len(), 1);
    assert_eq!((checks[0].step, checks[0].ok), (FtpStep::Connect, false));
    assert!(checks[0].detail.is_some());

    // Refused login.
    let port = ftp_server(dir.path(), Some(("nidoo", "right"))).await;
    let checks = FtpStore::probe(&settings(port, "/backups")).await;
    let seen: Vec<_> = checks.iter().map(|c| (c.step, c.ok)).collect();
    assert_eq!(
        seen,
        vec![(FtpStep::Connect, true), (FtpStep::Login, false)]
    );
    assert!(
        !checks
            .iter()
            .any(|c| c.detail.as_deref().unwrap_or("").contains("hunter2")),
        "no password in a detail"
    );

    // Missing folder.
    let port = ftp_server(dir.path(), None).await;
    let checks = FtpStore::probe(&settings(port, "/nope")).await;
    let seen: Vec<_> = checks.iter().map(|c| (c.step, c.ok)).collect();
    assert_eq!(
        seen,
        vec![
            (FtpStep::Connect, true),
            (FtpStep::Login, true),
            (FtpStep::Folder, false)
        ]
    );

    // All good, and nothing left behind.
    let checks = FtpStore::probe(&settings(port, "/backups")).await;
    let seen: Vec<_> = checks.iter().map(|c| (c.step, c.ok)).collect();
    assert_eq!(
        seen,
        vec![
            (FtpStep::Connect, true),
            (FtpStep::Login, true),
            (FtpStep::Folder, true),
            (FtpStep::Write, true)
        ]
    );
    assert!(names(&dir.path().join("backups")).is_empty());
}

/// A source that yields some bytes and then fails.
struct Breaks(bool);

impl tokio::io::AsyncRead for Breaks {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.0 {
            return std::task::Poll::Ready(Err(std::io::Error::other("source broke")));
        }
        self.0 = true;
        buf.put_slice(&[7; 1000]);
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn failed_upload_leaves_nothing_and_the_store_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    let port = ftp_server(dir.path(), None).await;
    let mut store = FtpStore::connect(&settings(port, "/")).await.unwrap();
    let name = file_name(at(11));
    let err = store.upload(&name, Breaks(false)).await.unwrap_err();
    assert_eq!(err, BackupError::Io("source broke".into()));
    assert!(names(dir.path()).is_empty(), "no backup and no .partial");
    assert!(store.list().await.unwrap().is_empty());
}
