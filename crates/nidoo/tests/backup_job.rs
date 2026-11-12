//! The backup job through the hub and its HTTP routes: back up now to a local libunftp server,
//! the browser download, the last-backup record, the failure push and the system sensor. The
//! tests that write a backup need `TEST_DATABASE_URL` (they skip without it); those whose job
//! ends before reading the database point it at one nobody answers on.

use crate::common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use common::db::fresh_db;
use common::ftp::ftp_server;
use nidoo::backup::archive::read_manifest;
use nidoo::backup::name;
use nidoo::backup::retained::{MemBroker, RetainedBroker};
use nidoo::backup::settings::BackupSection;
use nidoo::backup::z2m::{MemZ2m, Z2mSkip, Z2mSource};
use nidoo::config::Config;
use nidoo::hub::Hub;
use nidoo::setup::sections::Sections;
use nidoo::setup::{HubFile, Mode, RestartHook, SetupCtx};
use nidoo::system::sample::Resources;
use nidoo_api::ConnectionStatus;
use nidoo_api::system::ids;
use nidoo_core::command::Publish;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::sync::mpsc;

const PASSWORD: &str = "correct-horse";
const FTP_USER: &str = "nidoo";
const FTP_PASSWORD: &str = "ftp-secret";

struct Rig {
    hub: Arc<Hub>,
    ctx: Arc<SetupCtx>,
    rx: mpsc::Receiver<Publish>,
    addr: std::net::SocketAddr,
    ftp_dir: tempfile::TempDir,
    _data: tempfile::TempDir,
}

/// The backup section pointing at the local FTP server on `port`.
fn section(port: u16) -> BackupSection {
    let mut s = BackupSection {
        password: Some(PASSWORD.into()),
        ..BackupSection::default()
    };
    s.ftp.host = "127.0.0.1".into();
    s.ftp.port = port;
    s.ftp.user = FTP_USER.into();
    s.ftp.password = Some(FTP_PASSWORD.into());
    s
}

fn mem(docs: Vec<(String, Bytes)>) -> Arc<dyn RetainedBroker> {
    Arc::new(MemBroker(Mutex::new(docs.into_iter().collect())))
}

/// A broker whose collection waits until the gate opens: the job stays running meanwhile.
#[derive(Default)]
struct GateBroker {
    open: tokio::sync::watch::Sender<bool>,
}

impl GateBroker {
    fn release(&self) {
        self.open.send_replace(true);
    }
}

type Collected<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<(String, Bytes)>, String>> + Send + 'a>,
>;

impl RetainedBroker for GateBroker {
    fn collect(&self, _filters: Vec<String>, _quiet: Duration) -> Collected<'_> {
        let mut open = self.open.subscribe();
        Box::pin(async move {
            let _ = open.wait_for(|o| *o).await;
            Ok(one_dashboard())
        })
    }

    fn publish_retained(
        &self,
        _docs: Vec<(String, Bytes)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

/// A broker whose collection panics: the job dies in the middle.
struct PanicBroker;

impl RetainedBroker for PanicBroker {
    fn collect(&self, _filters: Vec<String>, _quiet: Duration) -> Collected<'_> {
        Box::pin(async { panic!("broker exploded (expected by the test)") })
    }

    fn publish_retained(
        &self,
        _docs: Vec<(String, Bytes)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

/// A Zigbee2MQTT source that must not be asked.
struct PanicZ2m;

impl Z2mSource for PanicZ2m {
    fn fetch(&self) -> futures_util::future::BoxFuture<'_, Result<Bytes, Z2mSkip>> {
        Box::pin(async { panic!("Zigbee2MQTT was asked (not expected by the test)") })
    }
}

fn z2m_ok(zip: &'static [u8]) -> Arc<dyn Z2mSource> {
    Arc::new(MemZ2m(Mutex::new(Ok(Bytes::from_static(zip)))))
}

fn z2m_err(skip: Z2mSkip) -> Arc<dyn Z2mSource> {
    Arc::new(MemZ2m(Mutex::new(Err(skip))))
}

fn bridge(hub: &Hub, state: &str) {
    hub.ingest(
        "zigbee2mqtt/bridge/state",
        format!(r#"{{"state":"{state}"}}"#).as_bytes(),
    );
}

/// A database nobody answers on: for the tests whose job ends before it reads the database.
const DEAD_DB: &str = "postgres://nidoo@127.0.0.1:1/none";

/// A served hub with the database at `db` (if any) in its setup context, the backup section
/// from `edit`, `broker` for the retained documents, and a local FTP server over a temp
/// directory.
async fn rig(
    db: Option<&str>,
    broker: Arc<dyn RetainedBroker>,
    edit: impl FnOnce(&mut BackupSection),
) -> Rig {
    let ftp_dir = tempfile::tempdir().unwrap();
    let port = ftp_server(ftp_dir.path(), Some((FTP_USER, FTP_PASSWORD))).await;
    let mut backup = section(port);
    edit(&mut backup);
    let sections = Sections {
        backup,
        ..Sections::default()
    };
    let data = tempfile::tempdir().unwrap();
    let ctx = Arc::new(SetupCtx::new(
        data.path().to_path_buf(),
        Mode::Run,
        HubFile::default(),
        db.map(|url| nidoo::history::db::Db::connect_lazy(url).unwrap()),
        sections,
        RestartHook::Record(Arc::default()),
    ));
    let mut cfg = Config::default();
    cfg.system.notify = false;
    let (tx, rx) = mpsc::channel(4096);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.set_setup(Arc::clone(&ctx));
    hub.set_backup_broker(broker);
    let app = nidoo::http::router(Arc::clone(&hub), Arc::clone(&ctx));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    Rig {
        hub,
        ctx,
        rx,
        addr,
        ftp_dir,
        _data: data,
    }
}

fn one_dashboard() -> Vec<(String, Bytes)> {
    vec![(
        "nidoo/ui/dashboards/main".to_owned(),
        Bytes::from_static(br#"{"id":"main"}"#),
    )]
}

/// About 12 MB of dashboards that do not compress: far more than the pipe and the sockets
/// hold, so the job is still writing when the browser leaves.
fn big_docs() -> Vec<(String, Bytes)> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..3_000)
        .map(|i| {
            let payload: Vec<u8> = (0..4_096)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect();
            (format!("nidoo/ui/dashboards/d{i}"), Bytes::from(payload))
        })
        .collect()
}

impl Rig {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1{path}", self.addr)
    }

    async fn post_run(&self) -> (StatusCode, Value) {
        let r = reqwest::Client::new()
            .post(self.url("/backup/runs"))
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    async fn current(&self) -> (StatusCode, Value) {
        let r = reqwest::get(self.url("/backup/current")).await.unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    async fn last(&self) -> Option<Value> {
        let r = reqwest::get(self.url("/backup/last")).await.unwrap();
        match r.status() {
            StatusCode::NO_CONTENT => None,
            StatusCode::OK => Some(r.json().await.unwrap()),
            other => panic!("GET /backup/last: {other}"),
        }
    }

    /// Waits until no backup runs (`GET /backup/current` is `204`).
    async fn wait_idle(&self, limit: Duration) {
        let until = tokio::time::Instant::now() + limit;
        loop {
            let (code, body) = self.current().await;
            if code == StatusCode::NO_CONTENT {
                return;
            }
            assert_eq!(code, StatusCode::OK, "{body}");
            assert!(tokio::time::Instant::now() < until, "still running: {body}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn drain(&mut self) -> Vec<Publish> {
        let mut out = Vec::new();
        while let Ok(p) = self.rx.try_recv() {
            out.push(p);
        }
        out
    }

    /// The backup failure alerts in the latest push history publish since the last drain.
    fn failure_alerts(&mut self) -> Vec<Value> {
        self.alerts("backup-failed-")
    }

    /// The alerts whose id starts with `prefix` in the latest push history publish since the
    /// last drain.
    fn alerts(&mut self, prefix: &str) -> Vec<Value> {
        let Some(last) = self
            .drain()
            .into_iter()
            .rev()
            .find(|p| p.topic == "nidoo/push/history")
        else {
            return Vec::new();
        };
        let list: Value = serde_json::from_str(&last.payload).unwrap();
        list.as_array()
            .unwrap()
            .iter()
            .filter(|a| a["id"].as_str().unwrap().starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Loads the push service, so a raised alert reaches `nidoo/push/history`.
    async fn load_push(&mut self) {
        self.hub.on_connected();
        self.hub.on_suback();
        tokio::time::sleep(Duration::from_millis(2_300)).await;
        self.drain();
    }
}

/// The backup files in `dir`, sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

/// A local port nothing listens on.
fn closed_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    probe.local_addr().unwrap().port()
}

/// Sleeps into the next second, so the next backup gets a new file name.
async fn next_second() {
    tokio::time::sleep(Duration::from_millis(1_100)).await;
}

#[tokio::test]
async fn backup_now_uploads_records_and_prunes() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    let (code, progress) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED, "{progress}");
    assert_eq!(progress["state"], "running");
    assert_eq!(progress["destination"], "ftp");
    let first = progress["file"].as_str().unwrap().to_owned();
    assert!(name::parse(&first).is_some(), "{first}");
    r.wait_idle(Duration::from_secs(60)).await;

    assert_eq!(files(r.ftp_dir.path()), vec![first.clone()]);
    let size = std::fs::metadata(r.ftp_dir.path().join(&first))
        .unwrap()
        .len();
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["destination"], "ftp");
    assert_eq!(last["file"], first.as_str());
    assert_eq!(last["size"], size);
    assert!(last["error"].is_null());
    assert!(last["last_ok_at"].as_i64().is_some());

    // The file opens with the backup password.
    let bytes = std::fs::read(r.ftp_dir.path().join(&first)).unwrap();
    let manifest = read_manifest(&bytes[..], PASSWORD).await.unwrap();
    assert_eq!(manifest.dashboards, 1);

    // Keep one: the second run leaves only itself.
    r.ctx.sections.lock().await.backup.keep = 1;
    next_second().await;
    let (code, progress) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED, "{progress}");
    let second = progress["file"].as_str().unwrap().to_owned();
    assert_ne!(second, first);
    r.wait_idle(Duration::from_secs(60)).await;
    assert_eq!(files(r.ftp_dir.path()), vec![second]);
    tdb.drop_db().await;
}

#[tokio::test]
async fn second_start_is_busy() {
    let Some(tdb) = fresh_db().await else { return };
    let gate = Arc::new(GateBroker::default());
    let r = rig(Some(&tdb.url), gate.clone(), |_| {}).await;
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let (code, body) = r.post_run().await;
    assert_eq!(code, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "a backup is already running");
    let download = reqwest::get(r.url("/backup/download")).await.unwrap();
    assert_eq!(download.status(), StatusCode::CONFLICT);
    let (code, current) = r.current().await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(current["state"], "running");
    gate.release();
    r.wait_idle(Duration::from_secs(60)).await;
    assert_eq!(r.last().await.unwrap()["ok"], true);
    tdb.drop_db().await;
}

#[tokio::test]
async fn failure_pushes_once_and_respects_the_switch() {
    let port = closed_port();
    let mut r = rig(Some(DEAD_DB), mem(one_dashboard()), |s| s.ftp.port = port).await;
    r.load_push().await;
    assert!(!r.hub.system_notify());

    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(30)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], false);
    let error = last["error"].as_str().unwrap();
    assert!(error.starts_with("can't reach"), "{error}");
    assert!(last["last_ok_at"].is_null());
    let alerts = r.failure_alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    let alert = &alerts[0];
    assert_eq!(alert["title"], "Backup failed");
    assert_eq!(alert["body"], format!("Backup failed: {error}"));
    assert_eq!(alert["channel"], "system");

    // The switch off: the record still moves, nothing is pushed.
    r.ctx.sections.lock().await.backup.notify_failure = false;
    next_second().await;
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(30)).await;
    let again = r.last().await.unwrap();
    assert_ne!(again["started_at"], last["started_at"]);
    assert_eq!(again["ok"], false);
    assert!(
        !r.drain().iter().any(|p| p.topic == "nidoo/push/history"),
        "pushed with the switch off"
    );
}

#[tokio::test]
async fn missing_password_or_ftp_is_refused_up_front() {
    let r = rig(Some(DEAD_DB), mem(one_dashboard()), |s| s.password = None).await;
    let (code, body) = r.post_run().await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["error"], "no backup password set");
    let download = reqwest::get(r.url("/backup/download")).await.unwrap();
    assert_eq!(download.status(), StatusCode::CONFLICT);
    let body: Value = download.json().await.unwrap();
    assert_eq!(body["error"], "no backup password set");

    {
        let mut held = r.ctx.sections.lock().await;
        held.backup.password = Some(PASSWORD.into());
        held.backup.ftp.host.clear();
    }
    let (code, body) = r.post_run().await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(body["error"], "FTP not configured");
    assert_eq!(r.current().await.0, StatusCode::NO_CONTENT);
    assert_eq!(r.last().await, None, "a refusal writes no record");
}

#[tokio::test]
async fn download_streams_a_backup_that_opens() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(one_dashboard()), |s| s.ftp.host.clear()).await;
    let resp = reqwest::get(r.url("/backup/download")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()["content-type"].to_str().unwrap(),
        "application/octet-stream"
    );
    let disposition = resp.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_owned();
    let file = disposition
        .strip_prefix("attachment; filename=\"")
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{disposition}"))
        .to_owned();
    assert!(name::parse(&file).is_some(), "{file}");
    let body = resp.bytes().await.unwrap();
    let manifest = read_manifest(&body[..], PASSWORD).await.unwrap();
    assert_eq!(manifest.dashboards, 1);

    r.wait_idle(Duration::from_secs(10)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["destination"], "download");
    assert_eq!(last["file"], file.as_str());
    assert_eq!(last["size"], body.len());
    tdb.drop_db().await;
}

#[tokio::test]
async fn download_closed_early_frees_the_slot() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(big_docs()), |_| {}).await;
    r.load_push().await;
    let mut resp = reqwest::get(r.url("/backup/download")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut got = 0;
    while got < 1_024 {
        got += resp.chunk().await.unwrap().expect("more body").len();
    }
    drop(resp);

    r.wait_idle(Duration::from_secs(5)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], false, "{last}");
    assert_eq!(last["error"], "cancelled");
    assert_eq!(last["destination"], "download");
    assert!(
        !r.drain().iter().any(|p| p.topic == "nidoo/push/history"),
        "a download never pushes"
    );
    let (code, body) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED, "{body}");
    r.wait_idle(Duration::from_secs(120)).await;
    tdb.drop_db().await;
}

#[tokio::test]
async fn cancel_ends_the_job_cancelled() {
    let r = rig(Some(DEAD_DB), Arc::new(GateBroker::default()), |_| {}).await;
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let cancel = || async {
        reqwest::Client::new()
            .delete(r.url("/backup/current"))
            .send()
            .await
            .unwrap()
            .status()
    };
    assert_eq!(cancel().await, StatusCode::NO_CONTENT);
    r.wait_idle(Duration::from_secs(5)).await;
    let last = r.last().await.unwrap();
    assert_eq!(
        (last["ok"].clone(), last["error"].clone()),
        (json!(false), json!("cancelled"))
    );
    assert_eq!(cancel().await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn panicking_job_frees_the_slot_and_records() {
    let mut r = rig(Some(DEAD_DB), Arc::new(PanicBroker), |_| {}).await;
    r.load_push().await;
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(10)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], false);
    assert_eq!(last["error"], "the backup stopped");
    assert_eq!(r.failure_alerts().len(), 1);
    let (code, body) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED, "{body}");
    r.wait_idle(Duration::from_secs(10)).await;
}

#[tokio::test]
async fn refused_on_the_recheck_leaves_no_record_and_no_push() {
    let mut r = rig(Some(DEAD_DB), mem(one_dashboard()), |s| s.password = None).await;
    r.load_push().await;
    // A settings save holds the lock: the up-front check is skipped, the job checks again.
    let held = r.ctx.sections.lock().await;
    r.hub.start_backup(Duration::from_secs(60)).unwrap();
    drop(held);
    r.wait_idle(Duration::from_secs(10)).await;
    assert_eq!(r.last().await, None, "a refusal writes no record");
    assert!(
        !r.drain().iter().any(|p| p.topic == "nidoo/push/history"),
        "a refusal never pushes"
    );
    let err = r.hub.backup_and_wait(Duration::from_secs(60)).await;
    assert_eq!(err, Err(nidoo::backup::BackupError::NoPassword));
}

#[tokio::test]
async fn stalled_download_is_cancelled() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(big_docs()), |_| {}).await;
    r.hub.set_backup_download_idle(Duration::from_millis(300));
    // The browser takes the headers and then reads nothing.
    let resp = reqwest::get(r.url("/backup/download")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    r.wait_idle(Duration::from_secs(30)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], false, "{last}");
    assert_eq!(last["error"], "cancelled");
    drop(resp);
    tdb.drop_db().await;
}

#[tokio::test]
async fn time_limit_fails_the_job() {
    let r = rig(Some(DEAD_DB), mem(one_dashboard()), |_| {}).await;
    let progress = r.hub.start_backup(Duration::from_millis(1)).unwrap();
    assert_eq!(progress.state, nidoo_api::BackupState::Running);
    r.wait_idle(Duration::from_secs(10)).await;
    let last = r.hub.last_backup().expect("a record");
    assert!(!last.ok);
    assert_eq!(last.error.as_deref(), Some("time limit reached"));
}

#[tokio::test]
async fn backup_and_wait_returns_the_upload() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    let done = r
        .hub
        .backup_and_wait(Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(done.host, "127.0.0.1");
    assert_eq!(files(r.ftp_dir.path()), vec![done.file.clone()]);
    let size = std::fs::metadata(r.ftp_dir.path().join(&done.file))
        .unwrap()
        .len();
    assert_eq!(done.size, size);
    tdb.drop_db().await;
}

#[tokio::test]
async fn last_backup_sensor_is_published() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    r.hub
        .backup_and_wait(Duration::from_secs(60))
        .await
        .unwrap();
    let last = r.hub.last_backup().unwrap();
    let at = last.last_ok_at.expect("a success");
    r.drain();
    let resources = Resources {
        cpu: None,
        ram: None,
        disk: None,
        temperature_c: None,
    };
    r.hub.tick_with(nidoo::clock::now(), resources).await;
    let published = r.drain();
    let state = published
        .iter()
        .find(|p| p.topic == format!("nidoo/system/{}", ids::LAST_BACKUP))
        .expect("the last backup state");
    let want = chrono::DateTime::from_timestamp(at, 0)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    assert_eq!(state.payload, want);
    tdb.drop_db().await;
}

// ---- No database needed ---------------------------------------------------------------

#[tokio::test]
async fn test_ftp_checks_stored_and_typed_settings() {
    let r = rig(None, mem(Vec::new()), |_| {}).await;
    let post = |body: Option<Value>| async {
        let mut req = reqwest::Client::new().post(r.url("/backup/test"));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        resp.json::<Vec<Value>>().await.unwrap()
    };
    let stored = post(None).await;
    assert_eq!(stored.len(), 4, "{stored:?}");
    assert!(stored.iter().all(|c| c["ok"] == true), "{stored:?}");

    // Same address, password kept: the stored password logs in.
    let port = r.ctx.sections.lock().await.backup.ftp.port;
    let same = json!({"host": "127.0.0.1", "port": port, "user": FTP_USER, "folder": "/"});
    let kept = post(Some(same)).await;
    assert!(kept.iter().all(|c| c["ok"] == true), "{kept:?}");

    // A new address with the password kept is refused before any connection.
    let moved = json!({"host": "127.0.0.2", "port": port, "user": FTP_USER});
    let refused = post(Some(moved)).await;
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["step"], "connect");
    assert_eq!(refused[0]["ok"], false);
    assert_eq!(
        refused[0]["detail"],
        "Enter the password for the new address"
    );

    // A wrong typed password fails at login.
    let wrong = json!({"host": "127.0.0.1", "port": port, "user": FTP_USER, "password": "nope"});
    let wrong = post(Some(wrong)).await;
    assert_eq!(wrong.len(), 2);
    assert_eq!(wrong[1]["step"], "login");
    assert_eq!(wrong[1]["ok"], false);
}

#[tokio::test]
async fn idle_routes_answer_no_content() {
    let r = rig(None, mem(Vec::new()), |_| {}).await;
    assert_eq!(r.current().await.0, StatusCode::NO_CONTENT);
    assert_eq!(r.last().await, None);
    let cancel = reqwest::Client::new()
        .delete(r.url("/backup/current"))
        .send()
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::NOT_FOUND);
}

/// `go` (manual) → `bk` (`backup.run`) → `yes` on port `ok`, `no` on port `failed`. The two
/// tail steps switch the flow on (it is on already), so their node id tells the port taken.
fn backup_flow(timeout_minutes: u32) -> String {
    let tail = |id: &str| json!({ "id": id, "type": "flow.enable", "params": { "enabled": true } });
    json!({
        "id": "nightly", "name": "nightly", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "go", "type": "trigger.manual", "params": {} },
            { "id": "bk", "type": "backup.run",
              "params": { "destination": "ftp", "timeout_minutes": timeout_minutes } },
            tail("yes"),
            tail("no"),
        ],
        "edges": [
            { "from": "go", "to": "bk" },
            { "from": "bk", "to": "yes", "port": "ok" },
            { "from": "bk", "to": "no", "port": "failed" },
        ],
    })
    .to_string()
}

/// `backup_flow` plus a `part` step on port `partial` when `wire_partial`.
fn backup_flow_partial(wire_partial: bool) -> String {
    let mut flow: serde_json::Value = serde_json::from_str(&backup_flow(120)).unwrap();
    if wire_partial {
        flow["nodes"].as_array_mut().unwrap().push(json!({
            "id": "part", "type": "flow.enable", "params": { "enabled": true }
        }));
        flow["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "from": "bk", "to": "part", "port": "partial" }));
    }
    flow.to_string()
}

/// The next run event the predicate accepts, within `limit`.
async fn next_run(
    rx: &mut tokio::sync::broadcast::Receiver<nidoo_api::WsEvent>,
    limit: Duration,
    accept: impl Fn(&nidoo_api::RunView) -> bool,
) -> nidoo_api::RunView {
    tokio::time::timeout(limit, async {
        loop {
            if let nidoo_api::WsEvent::FlowRunChanged { run, .. } =
                rx.recv().await.expect("open channel")
                && accept(&run)
            {
                return run;
            }
        }
    })
    .await
    .expect("the expected run event in time")
}

fn step_nodes(run: &nidoo_api::RunView) -> Vec<&str> {
    run.steps.iter().map(|s| s.node.as_str()).collect()
}

#[tokio::test]
async fn backup_flow_takes_ok() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    let _engine = r.hub.start_engine();
    r.hub
        .ingest("nidoo/automation/nightly", backup_flow(120).as_bytes());
    tokio::task::yield_now().await;
    let mut events = r.hub.subscribe();
    r.hub.run_flow("nightly").expect("run");
    let run = next_run(&mut events, Duration::from_secs(60), |r| {
        r.status == nidoo_api::RunStatus::Done
    })
    .await;

    assert_eq!(step_nodes(&run), ["go", "bk", "yes"]);
    let bk = &run.steps[1];
    assert_eq!(bk.outcome, nidoo_api::StepOutcome::Ok);
    assert_eq!(bk.ports.as_deref(), Some(&["ok".to_owned()][..]));
    let file = files(r.ftp_dir.path()).pop().expect("an uploaded file");
    let size = std::fs::metadata(r.ftp_dir.path().join(&file))
        .unwrap()
        .len();
    assert_eq!(
        bk.detail,
        format!(
            "Backed up {} to 127.0.0.1 as {file}",
            nidoo::backup::job::human_size(size)
        )
    );
    tdb.drop_db().await;
}

/// A run of `flow` against an offline Zigbee2MQTT bridge, ended.
async fn run_with_offline_z2m(r: &Rig, flow: String) -> nidoo_api::RunView {
    let _engine = r.hub.start_engine();
    bridge(&r.hub, "offline");
    r.hub.ingest("nidoo/automation/nightly", flow.as_bytes());
    tokio::task::yield_now().await;
    let mut events = r.hub.subscribe();
    r.hub.run_flow("nightly").expect("run");
    next_run(&mut events, Duration::from_secs(60), |r| {
        r.status == nidoo_api::RunStatus::Done
    })
    .await
}

#[tokio::test]
async fn backup_partial_takes_partial_when_wired() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    let run = run_with_offline_z2m(&r, backup_flow_partial(true)).await;

    assert_eq!(step_nodes(&run), ["go", "bk", "part"]);
    let bk = &run.steps[1];
    assert_eq!(bk.outcome, nidoo_api::StepOutcome::Ok);
    assert_eq!(bk.ports.as_deref(), Some(&["partial".to_owned()][..]));
    let file = files(r.ftp_dir.path()).pop().expect("an uploaded file");
    let size = std::fs::metadata(r.ftp_dir.path().join(&file))
        .unwrap()
        .len();
    assert_eq!(
        bk.detail,
        format!(
            "Backed up {} to 127.0.0.1 as {file}, without Zigbee2MQTT data: Zigbee2MQTT is offline",
            nidoo::backup::job::human_size(size)
        )
    );
    tdb.drop_db().await;
}

#[tokio::test]
async fn backup_partial_falls_back_to_ok() {
    let Some(tdb) = fresh_db().await else { return };
    let r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    let run = run_with_offline_z2m(&r, backup_flow_partial(false)).await;

    assert_eq!(step_nodes(&run), ["go", "bk", "yes"]);
    let bk = &run.steps[1];
    assert_eq!(bk.ports.as_deref(), Some(&["ok".to_owned()][..]));
    assert!(
        bk.detail
            .ends_with(", without Zigbee2MQTT data: Zigbee2MQTT is offline"),
        "{}",
        bk.detail
    );
    tdb.drop_db().await;
}

#[tokio::test]
async fn backup_flow_takes_failed() {
    let port = closed_port();
    let r = rig(Some(DEAD_DB), mem(one_dashboard()), |s| s.ftp.port = port).await;
    let _engine = r.hub.start_engine();
    r.hub
        .ingest("nidoo/automation/nightly", backup_flow(120).as_bytes());
    tokio::task::yield_now().await;
    let mut events = r.hub.subscribe();
    r.hub.run_flow("nightly").expect("run");
    let run = next_run(&mut events, Duration::from_secs(30), |r| {
        r.status != nidoo_api::RunStatus::Running
    })
    .await;

    // The step succeeded: `failed` is a port, not an error. The run ends `Done`.
    assert_eq!(run.status, nidoo_api::RunStatus::Done);
    assert_eq!(step_nodes(&run), ["go", "bk", "no"]);
    let bk = &run.steps[1];
    assert_eq!(bk.outcome, nidoo_api::StepOutcome::Ok);
    assert_eq!(bk.ports.as_deref(), Some(&["failed".to_owned()][..]));
    assert!(bk.detail.starts_with("can't reach"), "{}", bk.detail);
}

/// Review focus 4: a run cancelled while it waits does not cancel the backup.
#[tokio::test]
async fn cancelled_run_keeps_the_backup() {
    let Some(tdb) = fresh_db().await else { return };
    let gate = Arc::new(GateBroker::default());
    let r = rig(Some(&tdb.url), gate.clone(), |_| {}).await;
    let _engine = r.hub.start_engine();
    r.hub
        .ingest("nidoo/automation/nightly", backup_flow(120).as_bytes());
    tokio::task::yield_now().await;
    let mut events = r.hub.subscribe();
    r.hub.run_flow("nightly").expect("run");
    // The step is waiting on the job, which is held at the retained-documents gate.
    next_run(&mut events, Duration::from_secs(30), |run| {
        step_nodes(run) == ["go"]
    })
    .await;
    // The `go` step is recorded before `bk` launches the job: poll until it runs.
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while r.current().await.0 != StatusCode::OK {
        assert!(tokio::time::Instant::now() < until, "the job never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    r.hub
        .set_flow_enabled("nightly", false)
        .await
        .expect("disable");
    let cancelled = next_run(&mut events, Duration::from_secs(30), |run| {
        run.status == nidoo_api::RunStatus::Cancelled
    })
    .await;
    assert_eq!(step_nodes(&cancelled), ["go"]);
    assert_eq!(r.current().await.0, StatusCode::OK, "the job still runs");

    gate.release();
    r.wait_idle(Duration::from_secs(60)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(
        files(r.ftp_dir.path()),
        vec![last["file"].as_str().unwrap()]
    );
    tdb.drop_db().await;
}

/// The manifest of the only file on the FTP server.
async fn only_manifest(r: &Rig) -> nidoo::backup::archive::Manifest {
    let all = files(r.ftp_dir.path());
    assert_eq!(all.len(), 1, "{all:?}");
    let bytes = std::fs::read(r.ftp_dir.path().join(&all[0])).unwrap();
    read_manifest(&bytes[..], PASSWORD).await.unwrap()
}

#[tokio::test]
async fn online_z2m_is_in_the_file() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    r.load_push().await;
    bridge(&r.hub, "online");
    r.hub.set_backup_z2m(z2m_ok(b"PK-zip-bytes"));
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(60)).await;

    let manifest = only_manifest(&r).await;
    assert_eq!(manifest.z2m.map(|z| z.bytes), Some(12));
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert!(last["warning"].is_null(), "{last}");
    assert!(r.alerts("backup-").is_empty());
    tdb.drop_db().await;
}

/// A partial backup: saved, warned about, pushed once with `reason`.
async fn assert_partial(r: &mut Rig, reason: &str) {
    let manifest = only_manifest(r).await;
    assert!(manifest.z2m.is_none());
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["warning"], reason, "{last}");
    assert!(last["last_ok_at"].as_i64().is_some(), "{last}");
    let alerts = r.alerts("backup-partial-");
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(
        alerts[0]["id"],
        format!("backup-partial-{}", last["started_at"])
    );
    assert_eq!(alerts[0]["title"], "Backup incomplete");
    assert_eq!(
        alerts[0]["body"],
        format!("Backup saved without Zigbee2MQTT data: {reason}")
    );
}

#[tokio::test]
async fn offline_z2m_gives_a_partial_backup() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    r.load_push().await;
    bridge(&r.hub, "offline");
    r.hub.set_backup_z2m(Arc::new(PanicZ2m));
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(60)).await;
    assert_partial(&mut r, "Zigbee2MQTT is offline").await;
    tdb.drop_db().await;
}

#[tokio::test]
async fn silent_z2m_gives_a_partial_backup() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    r.load_push().await;
    bridge(&r.hub, "online");
    r.hub.set_backup_z2m(z2m_err(Z2mSkip::NoAnswer));
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(60)).await;
    assert_partial(&mut r, "Zigbee2MQTT didn't answer within 60 s").await;
    tdb.drop_db().await;
}

#[tokio::test]
async fn unknown_bridge_or_switch_off_is_a_plain_backup() {
    let Some(tdb) = fresh_db().await else { return };
    // No bridge state yet: not asked, nothing to say.
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |_| {}).await;
    r.load_push().await;
    r.hub.set_backup_z2m(Arc::new(PanicZ2m));
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(60)).await;
    assert!(only_manifest(&r).await.z2m.is_none());
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert!(last["warning"].is_null(), "{last}");
    assert!(r.alerts("backup-").is_empty());

    // The switch off with an online bridge: same.
    r.ctx.sections.lock().await.backup.include_z2m = false;
    bridge(&r.hub, "online");
    next_second().await;
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(60)).await;
    let again = r.last().await.expect("a record");
    assert_ne!(again["started_at"], last["started_at"]);
    assert_eq!(again["ok"], true, "{again}");
    assert!(again["warning"].is_null(), "{again}");
    assert!(r.alerts("backup-").is_empty());
    tdb.drop_db().await;
}

#[tokio::test]
async fn partial_push_follows_the_notify_switch() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |s| {
        s.notify_failure = false
    })
    .await;
    r.load_push().await;
    bridge(&r.hub, "offline");
    r.hub.set_backup_z2m(Arc::new(PanicZ2m));
    let (code, _) = r.post_run().await;
    assert_eq!(code, StatusCode::ACCEPTED);
    r.wait_idle(Duration::from_secs(60)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["warning"], "Zigbee2MQTT is offline", "{last}");
    assert!(
        !r.drain().iter().any(|p| p.topic == "nidoo/push/history"),
        "pushed with the switch off"
    );
    tdb.drop_db().await;
}

#[tokio::test]
async fn download_partial_does_not_push() {
    let Some(tdb) = fresh_db().await else { return };
    let mut r = rig(Some(&tdb.url), mem(one_dashboard()), |s| s.ftp.host.clear()).await;
    r.load_push().await;
    bridge(&r.hub, "offline");
    r.hub.set_backup_z2m(Arc::new(PanicZ2m));
    let resp = reqwest::get(r.url("/backup/download")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.bytes().await.unwrap();
    assert!(
        read_manifest(&body[..], PASSWORD)
            .await
            .unwrap()
            .z2m
            .is_none()
    );
    r.wait_idle(Duration::from_secs(10)).await;
    let last = r.last().await.expect("a record");
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["warning"], "Zigbee2MQTT is offline", "{last}");
    assert!(
        !r.drain().iter().any(|p| p.topic == "nidoo/push/history"),
        "a download pushed"
    );
    tdb.drop_db().await;
}
