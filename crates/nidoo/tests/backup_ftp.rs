//! The FTP store against a local libunftp server over a temp directory. No database needed.

use crate::common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use common::ftp::{ftp_server, ftp_server_idle};
use nidoo::backup::BackupError;
use nidoo::backup::ftp::{FtpSettings, FtpStore, Timeouts};
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

/// Accepts connections, says hello, then never answers again.
async fn silent_server() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut held = Vec::new();
        while let Ok((mut sock, _)) = listener.accept().await {
            let _ = sock.write_all(b"220 hello\r\n").await;
            held.push(sock);
        }
    });
    port
}

fn quick() -> Timeouts {
    Timeouts {
        connect: Duration::from_millis(300),
        control: Duration::from_millis(300),
        stall: Duration::from_millis(300),
    }
}

#[tokio::test]
async fn a_stalled_server_times_out_instead_of_hanging() {
    let port = silent_server().await;
    let overall = Duration::from_secs(5);

    let err = tokio::time::timeout(
        overall,
        FtpStore::connect_with(&settings(port, "/"), quick()),
    )
    .await
    .expect("connect returned")
    .err()
    .unwrap();
    assert_eq!(err, BackupError::Unreachable("timed out".into()));

    let checks = tokio::time::timeout(overall, FtpStore::probe_with(&settings(port, "/"), quick()))
        .await
        .expect("probe returned");
    let seen: Vec<_> = checks.iter().map(|c| (c.step, c.ok)).collect();
    assert_eq!(
        seen,
        vec![(FtpStep::Connect, true), (FtpStep::Login, false)]
    );
    assert_eq!(
        checks[1].detail.as_deref(),
        Some("can't reach the FTP server (timed out)")
    );
}

/// A scripted FTP server that records every command it receives, for checks libunftp cannot
/// make (it accepts any transfer type). Passive data connections are served on a port of its
/// own; a listing is empty and a stored file is read and dropped.
async fn recording_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(serve_scripted(sock, Arc::clone(&log)));
        }
    });
    (port, seen)
}

async fn serve_scripted(sock: tokio::net::TcpStream, log: Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (read, mut write) = sock.into_split();
    let mut lines = BufReader::new(read).lines();
    let _ = write.write_all(b"220 hello\r\n").await;
    let mut data: Option<tokio::net::TcpListener> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        log.lock().unwrap().push(line.clone());
        let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
        let reply: String = match verb.as_str() {
            "USER" => "331 password please".into(),
            "PASS" => "230 in".into(),
            "TYPE" => "200 type set".into(),
            "CWD" => "250 ok".into(),
            "NOOP" => "200 ok".into(),
            "SIZE" => "213 0".into(),
            "DELE" | "RNTO" => "250 ok".into(),
            "RNFR" => "350 go on".into(),
            "PASV" => {
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let p = l.local_addr().unwrap().port();
                data = Some(l);
                format!(
                    "227 Entering Passive Mode (127,0,0,1,{},{})",
                    p / 256,
                    p % 256
                )
            }
            "NLST" | "STOR" | "RETR" => {
                let Some(l) = data.take() else {
                    let _ = write.write_all(b"425 no data connection\r\n").await;
                    continue;
                };
                let _ = write.write_all(b"150 here it comes\r\n").await;
                if let Ok((mut d, _)) = l.accept().await {
                    if verb == "STOR" {
                        let mut sink = Vec::new();
                        let _ = tokio::io::AsyncReadExt::read_to_end(&mut d, &mut sink).await;
                    }
                    let _ = d.shutdown().await;
                }
                "226 done".into()
            }
            "QUIT" => {
                let _ = write.write_all(b"221 bye\r\n").await;
                return;
            }
            _ => "502 not here".into(),
        };
        let _ = write.write_all(format!("{reply}\r\n").as_bytes()).await;
    }
}

/// `TYPE I` comes after the login and before the first command that moves file bytes or reads
/// a size (ASCII mode rewrites line ends, and some servers refuse SIZE in it).
fn assert_binary_before_transfers(seen: &[String]) {
    let at = |pred: &dyn Fn(&str) -> bool| seen.iter().position(|c| pred(c));
    let login = at(&|c| c.starts_with("PASS")).expect("a login");
    let binary = at(&|c| c == "TYPE I").unwrap_or_else(|| panic!("no TYPE I in {seen:?}"));
    let first_transfer = at(&|c| {
        ["SIZE", "STOR", "RETR", "NLST"]
            .iter()
            .any(|v| c.starts_with(v))
    })
    .expect("a transfer");
    assert!(login < binary && binary < first_transfer, "order: {seen:?}");
}

#[tokio::test]
async fn the_session_is_binary_before_any_transfer() {
    let (port, seen) = recording_server().await;
    let mut store = FtpStore::connect_with(&settings(port, "/"), quick())
        .await
        .unwrap();
    assert!(store.list().await.unwrap().is_empty());
    store
        .upload(&file_name(at(11)), &b"x\r\ny\n"[..])
        .await
        .unwrap();
    assert_binary_before_transfers(&seen.lock().unwrap());

    let (port, seen) = recording_server().await;
    let checks = FtpStore::probe_with(&settings(port, "/"), quick()).await;
    assert!(checks.iter().all(|c| c.ok), "{checks:?}");
    assert_binary_before_transfers(&seen.lock().unwrap());
}

#[tokio::test]
async fn keep_alive_reconnects_once_the_server_dropped_an_idle_session() {
    let dir = tempfile::tempdir().unwrap();
    let port = ftp_server_idle(dir.path(), 1).await;
    let s = settings(port, "/");
    let mut store = FtpStore::connect_with(&s, quick()).await.unwrap();
    // Longer than the server's idle timeout: it has closed the session.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    store.keep_alive(&s).await.unwrap();
    let name = file_name(at(11));
    store.upload(&name, &b"data"[..]).await.unwrap();
    assert_eq!(names(dir.path()), vec![name]);

    // A live session is kept as it is.
    store.keep_alive(&s).await.unwrap();
    assert_eq!(store.list().await.unwrap().len(), 1);
}
