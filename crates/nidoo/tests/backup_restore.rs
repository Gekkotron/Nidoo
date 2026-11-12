//! Restoring a backup through the setup routes, on a hub booted in restore mode: from a local
//! libunftp server or an upload, into a fresh database and an in-memory broker.
//!
//! The tests that restore need `TEST_DATABASE_URL` (they print `skipping: TEST_DATABASE_URL
//! unset` and return without it). The refusals and the upload inspect run without a database.

use crate::common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use common::db::{TestDb, fresh_db};
use common::ftp::ftp_server;
use nidoo::backup::archive::{Manifest, Sources, write_backup};
use nidoo::backup::crypt::seal;
use nidoo::backup::db::{exported_table_names, holds_history};
use nidoo::backup::name;
use nidoo::backup::retained::{Layout, MemBroker, RetainedBroker};
use nidoo::backup::settings::BackupSection;
use nidoo::backup::tar::TarWriter;
use nidoo::config::{Config, HttpConfig, LogConfig, MqttConfig};
use nidoo::history::db::Db;
use nidoo::setup::hub_file::{HubFields, RestoreState};
use nidoo::setup::sections::Sections;
use nidoo::setup::store::read_cache;
use nidoo::setup::{HubFile, Mode, RestartHook, SetupCtx};
use nidoo_api::{Location, SetupStep};
use nidoo_core::history::{Rows, ValueRow};
use nidoo_core::model::StateValue;
use reqwest::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;

const PASSWORD: &str = "correct horse battery";
/// [`PASSWORD`] as the `x-backup-password` header carries it.
const PASSWORD_HEADER: &str = "correct%20horse%20battery";
const FTP_USER: &str = "nidoo";
const FTP_PASSWORD: &str = "ftp-secret";
/// 2025-01-01T00:00:00Z in microseconds.
const T0: i64 = 1_735_689_600_000_000;
const MIB: usize = 1024 * 1024;

// ---- the source: a backup taken from database A ------------------------------------------

fn source_fields() -> HubFields {
    HubFields {
        location: Some(Location {
            city: "Paris".into(),
            country: Some("FR".into()),
            latitude: 48.85,
            longitude: 2.35,
            timezone: "Europe/Paris".into(),
        }),
        timezone: None,
        http: HttpConfig {
            bind: "0.0.0.0:9000".into(),
        },
        log: LogConfig {
            level: "debug".into(),
            json: true,
        },
        z2m_base: Some("zigbee2mqtt".into()),
    }
}

/// The source's backup section: its own NAS, not the server the tests restore from.
fn source_backup_section() -> BackupSection {
    let mut s = BackupSection {
        password: Some(PASSWORD.into()),
        keep: 3,
        ..BackupSection::default()
    };
    s.ftp.host = "nas.local".into();
    s.ftp.user = "house".into();
    s.ftp.password = Some("nas-pass".into());
    s.ftp.folder = "/backups".into();
    s
}

/// Settings rows and `rows` readings in `entity_state`, with values that compress badly.
async fn fill(a: &TestDb, rows: i64) {
    let pool = a.db.pool();
    sqlx::query("INSERT INTO settings (key, value) VALUES ('backup', $1), ('mobile', $2)")
        .bind(serde_json::to_value(source_backup_section()).unwrap())
        .bind(json!({"enabled": true}))
        .execute(pool)
        .await
        .unwrap();
    // Random values, one per second from T0, written in one statement.
    sqlx::query(
        "INSERT INTO entity_state (time, entity_id, num, value)
         SELECT to_timestamp($1::float8 + i), 'sensor.a', r, to_jsonb(r)
         FROM (SELECT i, random() * 1e6 AS r FROM generate_series(1, $2::bigint) i) s",
    )
    .bind((T0 / 1_000_000) as f64)
    .bind(rows)
    .execute(pool)
    .await
    .unwrap();
}

/// A flow, a dashboard and the areas (backed up), and a runtime document (left out).
fn source_docs() -> BTreeMap<String, Bytes> {
    [
        ("nidoo/automation/wake-up", r#"{"id":"wake-up"}"#),
        ("nidoo/ui/dashboards/main", r#"{"id":"main"}"#),
        ("nidoo/ui/_areas", r#"[{"id":"living"}]"#),
        ("nidoo/alarm/state", r#"{"state":"disarmed"}"#),
    ]
    .into_iter()
    .map(|(t, p)| (t.to_owned(), Bytes::from(p)))
    .collect()
}

/// The documents a restore brings back.
fn restored_docs() -> BTreeMap<String, Bytes> {
    let mut docs = source_docs();
    docs.remove("nidoo/alarm/state");
    docs
}

/// Database A filled with `rows` readings, and a backup of it.
async fn source(rows: i64) -> Option<(TestDb, Vec<u8>)> {
    let a = fresh_db().await?;
    fill(&a, rows).await;
    let broker = MemBroker(std::sync::Mutex::new(source_docs()));
    let mut file = Vec::new();
    write_backup(
        Sources {
            pool: a.db.pool(),
            broker: &broker,
            layout: Layout::from_config(&Config::default()),
            hub: source_fields(),
        },
        &mut file,
        PASSWORD,
        &|_, _| {},
    )
    .await
    .unwrap();
    Some((a, file))
}

// ---- the target: a hub in restore mode over database B --------------------------------------

struct Target {
    addr: std::net::SocketAddr,
    dir: tempfile::TempDir,
    broker: Arc<MemBroker>,
    restarts: Arc<AtomicUsize>,
    client: reqwest::Client,
}

fn target_mqtt() -> MqttConfig {
    MqttConfig {
        host: "192.0.2.1".into(),
        port: 1,
        user: Some("target".into()),
        password: Some("target-pass".into()),
        ..MqttConfig::default()
    }
}

/// The target's `hub.json`: its own broker and database, the setup on its restore step.
fn target_file(database_url: &str, mode: Mode) -> HubFile {
    HubFile {
        mqtt: Some(target_mqtt()),
        database_url: Some(database_url.into()),
        step: SetupStep::Restore,
        restore: (mode == Mode::Restore).then(RestoreState::default),
        ..HubFile::default()
    }
}

/// A hub in `mode` over `db` (none: no database at all), its `hub.json` saved, served on an
/// ephemeral port with an empty in-memory broker.
async fn target(mode: Mode, db: Option<&TestDb>) -> Target {
    let dir = tempfile::tempdir().unwrap();
    let url = db.map_or("postgres://u:p@192.0.2.2/nidoo", |d| d.url.as_str());
    let file = target_file(url, mode);
    file.save(dir.path()).unwrap();
    let restarts = Arc::new(AtomicUsize::new(0));
    let ctx = Arc::new(SetupCtx::new(
        dir.path().to_path_buf(),
        mode,
        file,
        db.map(|d| Db::connect_lazy(&d.url).unwrap()),
        Sections::default(),
        RestartHook::Record(Arc::clone(&restarts)),
    ));
    let broker = Arc::new(MemBroker::default());
    ctx.set_restore_broker(Arc::clone(&broker) as Arc<dyn RetainedBroker>);
    let app = nidoo::http::setup_router(Arc::clone(&ctx));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    Target {
        addr,
        dir,
        broker,
        restarts,
        client: reqwest::Client::new(),
    }
}

async fn answer(r: reqwest::Response) -> (StatusCode, Value) {
    (r.status(), r.json().await.unwrap_or(Value::Null))
}

impl Target {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1/setup/restore{path}", self.addr)
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        answer(
            self.client
                .post(self.url(path))
                .json(&body)
                .send()
                .await
                .unwrap(),
        )
        .await
    }

    async fn upload(&self, path: &str, password: &str, body: Vec<u8>) -> (StatusCode, Value) {
        let method = if path == "/upload" {
            reqwest::Method::PUT
        } else {
            reqwest::Method::POST
        };
        answer(
            self.client
                .request(method, self.url(path))
                .header("x-backup-password", password)
                .body(body)
                .send()
                .await
                .unwrap(),
        )
        .await
    }

    async fn progress(&self) -> (StatusCode, Value) {
        answer(self.client.get(self.url("/progress")).send().await.unwrap()).await
    }

    /// Polls the progress until the restore is no longer running.
    async fn wait_end(&self) -> Value {
        let until = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let (code, body) = self.progress().await;
            assert_eq!(code, StatusCode::OK, "{body}");
            if body["state"] != "running" {
                return body;
            }
            assert!(tokio::time::Instant::now() < until, "still running: {body}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn file(&self) -> HubFile {
        HubFile::load(self.dir.path()).unwrap().unwrap()
    }

    fn docs(&self) -> BTreeMap<String, Bytes> {
        self.broker.0.lock().unwrap().clone()
    }
}

fn ftp_source(port: u16) -> Value {
    json!({"host": "127.0.0.1", "port": port, "user": FTP_USER, "password": FTP_PASSWORD})
}

// ---- checks ------------------------------------------------------------------------------

/// Every row of `table` in a stable order, as JSON.
async fn contents(pool: &PgPool, table: &str) -> Value {
    sqlx::query_scalar(&format!(
        "SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY to_jsonb(x)::text), '[]'::jsonb) \
         FROM \"{table}\" x"
    ))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn settings_rows(pool: &PgPool) -> BTreeMap<String, Value> {
    let rows: Vec<(String, Value)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await
        .unwrap();
    rows.into_iter().collect()
}

/// Database B holds A's rows exactly once, the broker the backed-up documents and nothing
/// else, `hub.json` the source's place with the target's own broker and database, the
/// restore is over and one restart was asked for.
async fn assert_restored(t: &Target, a: &TestDb, b: &TestDb) {
    let (pa, pb) = (a.db.pool(), b.db.pool());
    for table in exported_table_names(pa).await.unwrap() {
        assert_eq!(
            contents(pb, &table).await,
            contents(pa, &table).await,
            "{table}"
        );
    }
    assert_eq!(t.docs(), restored_docs());
    let file = t.file();
    assert_eq!(file.location, source_fields().location);
    assert_eq!(file.http, source_fields().http);
    assert_eq!(file.mqtt, Some(target_mqtt()));
    assert_eq!(file.database_url.as_deref(), Some(b.url.as_str()));
    assert_eq!(file.restore, None);
    assert!(file.finished);
    assert_eq!(file.step, SetupStep::Done);
    assert_eq!(
        read_cache(t.dir.path()),
        Some(Sections::from_rows(settings_rows(pb).await))
    );
    assert_eq!(t.restarts.load(Ordering::SeqCst), 1);
}

// ---- the tests ---------------------------------------------------------------------------

#[tokio::test]
async fn ftp_list_inspect_restore_restart() {
    let Some((a, file)) = source(50).await else {
        return;
    };
    let Some(b) = fresh_db().await else { return };
    let ftp_dir = tempfile::tempdir().unwrap();
    let port = ftp_server(ftp_dir.path(), Some((FTP_USER, FTP_PASSWORD))).await;
    let older = name::file_name("2025-03-01T08:00:00Z".parse().unwrap());
    let newer = name::file_name("2025-03-02T08:00:00Z".parse().unwrap());
    for n in [&older, &newer] {
        std::fs::write(ftp_dir.path().join(n), &file).unwrap();
    }
    let t = target(Mode::Restore, Some(&b)).await;

    let (code, list) = t.post("/ftp/list", ftp_source(port)).await;
    assert_eq!(code, StatusCode::OK, "{list}");
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["file"].as_str().unwrap())
        .collect();
    assert_eq!(names, [newer.as_str(), older.as_str()]);
    assert_eq!(list[0]["size"], file.len() as u64);

    let request = json!({"source": ftp_source(port), "file": newer, "password": PASSWORD});
    let (code, summary) = t.post("/ftp/inspect", request.clone()).await;
    assert_eq!(code, StatusCode::OK, "{summary}");
    assert_eq!(summary["flows"], 1);
    assert_eq!(summary["dashboards"], 1);
    assert_eq!(summary["nidoo_version"], env!("CARGO_PKG_VERSION"));
    assert!(summary["history_bytes"].as_u64().unwrap() > 0);

    let (code, started) = t.post("/ftp", request).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["state"], "running");
    let end = t.wait_end().await;
    assert_eq!(end["state"], "done", "{end}");
    assert_eq!(end["total"], file.len() as u64);
    assert_restored(&t, &a, &b).await;
    a.drop_db().await;
    b.drop_db().await;
}

#[tokio::test]
async fn upload_restore_streams() {
    let Some((a, file)) = source(50).await else {
        return;
    };
    let Some(b) = fresh_db().await else { return };
    let t = target(Mode::Restore, Some(&b)).await;
    let size = file.len() as u64;

    let (code, body) = t.upload("/upload", PASSWORD_HEADER, file).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
    assert_eq!(body["total"], size);
    assert_eq!(body["bytes"], size);
    assert_restored(&t, &a, &b).await;
    // An upload keeps the backup section as the backup has it.
    assert_eq!(
        settings_rows(b.db.pool()).await,
        settings_rows(a.db.pool()).await
    );
    a.drop_db().await;
    b.drop_db().await;
}

/// A backup file written by hand: the manifest, then `filler` bytes that do not compress.
async fn crafted(manifest: &Manifest, filler: usize) -> Vec<u8> {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let noise: Vec<u8> = (0..filler)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let mut file = Vec::new();
    let mut tw = TarWriter::new(seal(&mut file, PASSWORD));
    tw.entry("manifest.json", &serde_json::to_vec(manifest).unwrap())
        .await
        .unwrap();
    tw.entry("hub.json", &noise).await.unwrap();
    tw.finish().await.unwrap().shutdown().await.unwrap();
    file
}

fn manifest() -> Manifest {
    Manifest {
        format: 1,
        nidoo_version: "9.8.7".into(),
        migration: 1,
        created_at: "2025-03-02T08:00:00Z".into(),
        host: "old-hub".into(),
        tables: BTreeMap::new(),
        retained: 4,
        flows: 2,
        dashboards: 1,
        history_bytes: 123_456,
    }
}

/// Runs without a database: the inspect reads the manifest from the head and writes nothing.
#[tokio::test]
async fn upload_inspect_reads_the_head_only() {
    let file = crafted(&manifest(), 3 * MIB).await;
    assert!(file.len() > 2 * MIB);
    let head = file[..MIB].to_vec();
    let t = target(Mode::Restore, None).await;
    let before = t.file();

    let (code, summary) = t
        .upload("/upload/inspect", PASSWORD_HEADER, head.clone())
        .await;
    assert_eq!(code, StatusCode::OK, "{summary}");
    assert_eq!(
        summary,
        json!({"created_at": "2025-03-02T08:00:00Z", "nidoo_version": "9.8.7",
               "host": "old-hub", "flows": 2, "dashboards": 1, "history_bytes": 123_456})
    );

    let (code, body) = t.upload("/upload/inspect", "wrong%20one", head).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, json!({"error": "Wrong password"}));

    assert_eq!(t.file(), before);
    assert!(t.docs().is_empty());
    assert_eq!(t.progress().await.0, StatusCode::NO_CONTENT);
    assert_eq!(t.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refuses_a_hub_with_data() {
    let Some((a, file)) = source(50).await else {
        return;
    };
    let Some(b) = fresh_db().await else { return };
    let t = target(Mode::Restore, Some(&b)).await;
    let before = t.file();

    // History in the database.
    b.db.insert(&Rows {
        values: vec![ValueRow {
            at: T0,
            id: "sensor.mine".into(),
            value: StateValue::Number(1.0),
            num: Some(1.0),
        }],
        availability: vec![],
    })
    .await
    .unwrap();
    let mine = contents(b.db.pool(), "entity_state").await;
    let (code, body) = t.upload("/upload", PASSWORD_HEADER, file.clone()).await;
    assert_eq!(code, StatusCode::CONFLICT, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("already holds data"), "{error}");
    assert_eq!(contents(b.db.pool(), "entity_state").await, mine);
    assert!(settings_rows(b.db.pool()).await.is_empty());
    assert!(t.docs().is_empty());
    assert_eq!(t.file(), before, "attempted is still false");
    assert_eq!(t.restarts.load(Ordering::SeqCst), 0);

    // A flow on the broker. The insert above also wrote `entity_latest`: empty both.
    sqlx::query("TRUNCATE entity_state, entity_latest")
        .execute(b.db.pool())
        .await
        .unwrap();
    assert_eq!(holds_history(b.db.pool()).await.unwrap(), None);
    let flow = (
        "nidoo/automation/mine".to_owned(),
        Bytes::from_static(br#"{"id":"mine"}"#),
    );
    t.broker.publish_retained(vec![flow.clone()]).await.unwrap();
    let (code, body) = t.upload("/upload", PASSWORD_HEADER, file).await;
    assert_eq!(code, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"],
        "This hub already holds data (flows or settings on the broker)"
    );
    assert_eq!(t.docs(), BTreeMap::from([flow]));
    assert_eq!(holds_history(b.db.pool()).await.unwrap(), None);
    assert_eq!(t.file(), before);
    a.drop_db().await;
    b.drop_db().await;
}

#[tokio::test]
async fn retry_after_a_failure_starts_clean() {
    // Enough history for the file to span many encrypted chunks.
    let Some((a, file)) = source(20_000).await else {
        return;
    };
    let Some(b) = fresh_db().await else { return };
    let t = target(Mode::Restore, Some(&b)).await;
    let before = t.file();

    let cut = file[..file.len() * 8 / 10].to_vec();
    let (code, body) = t.upload("/upload", PASSWORD_HEADER, cut).await;
    assert!(
        code.is_client_error() || code.is_server_error(),
        "{code} {body}"
    );
    let (_, progress) = t.progress().await;
    assert_eq!(progress["state"], "failed", "{progress}");
    assert!(progress["error"].is_string(), "{progress}");
    let file_after = t.file();
    assert_eq!(file_after.restore, Some(RestoreState { attempted: true }));
    assert_eq!(file_after.location, before.location);
    // The failed attempt cleaned up after itself.
    assert_eq!(holds_history(b.db.pool()).await.unwrap(), None);
    assert!(settings_rows(b.db.pool()).await.is_empty());
    assert!(t.docs().is_empty());
    assert_eq!(t.restarts.load(Ordering::SeqCst), 0);

    // Leftovers a clean-up could have missed: a row, settings with a prefix of their own, and
    // a document under it.
    let mut left = Sections::default();
    left.advanced.topics.ui_prefix = "custom/ui".into();
    for (key, value) in left.to_rows() {
        sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2)")
            .bind(key)
            .bind(value)
            .execute(b.db.pool())
            .await
            .unwrap();
    }
    b.db.insert(&Rows {
        values: vec![ValueRow {
            at: T0,
            id: "sensor.a".into(),
            value: StateValue::Number(0.5),
            num: Some(0.5),
        }],
        availability: vec![],
    })
    .await
    .unwrap();
    t.broker
        .publish_retained(vec![
            (
                "custom/ui/dashboards/stale".to_owned(),
                Bytes::from_static(b"{}"),
            ),
            (
                "nidoo/automation/stale".to_owned(),
                Bytes::from_static(b"{}"),
            ),
        ])
        .await
        .unwrap();

    let (code, body) = t.upload("/upload", PASSWORD_HEADER, file).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_restored(&t, &a, &b).await;
    assert_eq!(
        settings_rows(b.db.pool()).await,
        settings_rows(a.db.pool()).await
    );
    a.drop_db().await;
    b.drop_db().await;
}

/// Runs without a database.
#[tokio::test]
async fn routes_refused_outside_restore_mode() {
    let t = target(Mode::Setup, None).await;
    let source = json!({"host": "127.0.0.1", "port": 1});
    let request = json!({"source": source, "file": "f", "password": PASSWORD});
    let refusals = [
        t.post("/ftp/list", source.clone()).await,
        t.post("/ftp/inspect", request.clone()).await,
        t.post("/ftp", request).await,
        t.upload("/upload/inspect", PASSWORD_HEADER, vec![1, 2, 3])
            .await,
        t.upload("/upload", PASSWORD_HEADER, vec![1, 2, 3]).await,
        t.progress().await,
    ];
    for (code, body) in refusals {
        assert_eq!(code, StatusCode::CONFLICT, "{body}");
        assert_eq!(body, json!({"error": "not waiting for a restore"}));
    }
}

/// Runs without a database: restore mode whose database URL could not be used.
#[tokio::test]
async fn restoring_needs_the_database() {
    let t = target(Mode::Restore, None).await;
    let before = t.file();
    let request = json!({"source": {"host": "127.0.0.1", "port": 1}, "file": "f",
                         "password": PASSWORD});
    for (code, body) in [
        t.post("/ftp", request).await,
        t.upload("/upload", PASSWORD_HEADER, vec![1, 2, 3]).await,
    ] {
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body, json!({"error": "database unreachable"}));
    }
    assert_eq!(t.file(), before);
    assert_eq!(t.progress().await.0, StatusCode::NO_CONTENT);
}

/// Runs without a database: nothing listens on the port.
#[tokio::test]
async fn unreachable_ftp_server_is_a_bad_gateway() {
    let t = target(Mode::Restore, None).await;
    let closed = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };
    let (code, body) = t.post("/ftp/list", ftp_source(closed)).await;
    assert_eq!(code, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(
        body,
        json!({"error": "Can't reach the FTP server. Is it switched on?"})
    );
}

#[tokio::test]
async fn typed_ftp_source_replaces_a_different_backup_ftp() {
    let Some((a, file)) = source(50).await else {
        return;
    };
    let Some(b) = fresh_db().await else { return };
    let ftp_dir = tempfile::tempdir().unwrap();
    let port = ftp_server(ftp_dir.path(), Some((FTP_USER, FTP_PASSWORD))).await;
    let newest = name::file_name("2025-03-02T08:00:00Z".parse().unwrap());
    std::fs::write(ftp_dir.path().join(&newest), &file).unwrap();
    let t = target(Mode::Restore, Some(&b)).await;

    let request = json!({"source": ftp_source(port), "file": newest, "password": PASSWORD});
    let (code, body) = t.post("/ftp", request).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{body}");
    assert_eq!(t.wait_end().await["state"], "done");

    let mut expected = settings_rows(a.db.pool()).await;
    let mut section = source_backup_section();
    section.ftp.host = "127.0.0.1".into();
    section.ftp.port = port;
    section.ftp.user = FTP_USER.into();
    section.ftp.password = Some(FTP_PASSWORD.into());
    section.ftp.folder = "/".into();
    expected.insert("backup".into(), serde_json::to_value(&section).unwrap());
    let restored = settings_rows(b.db.pool()).await;
    assert_eq!(restored, expected);
    let cached = read_cache(t.dir.path()).unwrap();
    assert_eq!(cached.backup, section);
    assert_eq!(cached.backup.password.as_deref(), Some(PASSWORD));
    assert_eq!(cached.backup.keep, 3);
    assert_restored(&t, &a, &b).await;
    a.drop_db().await;
    b.drop_db().await;
}
