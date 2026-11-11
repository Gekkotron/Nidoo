//! A whole backup file: written from one database and broker, read back into another.
//!
//! Needs `TEST_DATABASE_URL` (see `history_db.rs`); without it each test prints
//! `skipping: TEST_DATABASE_URL unset` and returns. The file format itself is also covered
//! without a database by the unit tests in `src/backup/archive.rs`.

use crate::common;

use std::collections::BTreeMap;
use std::sync::Mutex;

use bytes::Bytes;
use common::db::fresh_db;
use nidoo::backup::BackupError;
use nidoo::backup::archive::{
    Manifest, Sources, check_compatible, read_backup, read_manifest, write_backup,
};
use nidoo::backup::db::{holds_history, migration_level};
use nidoo::backup::retained::{Layout, MemBroker};
use nidoo::config::{Config, HttpConfig, LogConfig};
use nidoo::setup::hub_file::HubFields;
use nidoo_api::{BackupPart, Location};
use nidoo_core::history::{Rows, ValueRow};
use nidoo_core::model::StateValue;
use serde_json::{Value, json};
use sqlx::PgPool;

const PASSWORD: &str = "correct horse battery";

/// 2025-01-01T00:00:00Z in microseconds.
const T0: i64 = 1_735_689_600_000_000;

fn layout() -> Layout {
    Layout::from_config(&Config::default())
}

fn hub_fields() -> HubFields {
    HubFields {
        location: Some(Location {
            city: "Paris".into(),
            country: Some("France".into()),
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

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap();
}

/// Settings rows and rows in two history tables.
async fn fill(pool: &PgPool, db: &nidoo::history::db::Db) {
    exec(
        pool,
        r#"INSERT INTO settings (key, value) VALUES ('mobile', '{"enabled": true}'),
                                                   ('home', '{"name": "Maison"}')"#,
    )
    .await;
    db.insert(&Rows {
        values: (0..50)
            .map(|i| ValueRow {
                at: T0 + i * 60_000_000,
                id: "sensor.a".into(),
                value: StateValue::Number(i as f64),
                num: Some(i as f64),
            })
            .collect(),
        availability: vec![],
    })
    .await
    .unwrap();
    exec(
        pool,
        "INSERT INTO climate_decision
             (time, zone, entity_id, mode, setpoint, source, reason, applied, adjust)
         VALUES ('2025-01-02T00:00:00Z', 'living', 'climate.living', 'heat', 20.5,
                 'schedule', 'morning', true, 0.5)",
    )
    .await;
}

/// Three backed-up documents (a flow, a dashboard, the areas) and two runtime ones.
fn source_broker() -> MemBroker {
    let docs: BTreeMap<String, Bytes> = [
        ("nidoo/automation/wake-up", r#"{"id":"wake-up"}"#),
        ("nidoo/ui/dashboards/main", r#"{"id":"main"}"#),
        ("nidoo/ui/_areas", r#"[{"id":"living"}]"#),
        ("nidoo/alarm/state", r#"{"state":"disarmed"}"#),
        ("nidoo/climate/zone/z/decision", r#"{"mode":"heat"}"#),
    ]
    .into_iter()
    .map(|(t, p)| (t.to_owned(), Bytes::from(p)))
    .collect();
    MemBroker(Mutex::new(docs))
}

fn broker_docs(b: &MemBroker) -> BTreeMap<String, Bytes> {
    b.0.lock().unwrap().clone()
}

async fn backup(pool: &PgPool, broker: &MemBroker) -> (Vec<u8>, Manifest) {
    let mut out = Vec::new();
    let manifest = write_backup(
        Sources {
            pool,
            broker,
            layout: layout(),
            hub: hub_fields(),
        },
        &mut out,
        PASSWORD,
        &|_, _| {},
    )
    .await
    .unwrap();
    (out, manifest)
}

/// Every table's rows in a stable order, as JSON.
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

/// Nothing at all was written to the target.
async fn assert_untouched(pool: &PgPool, broker: &MemBroker) {
    assert_eq!(holds_history(pool).await.unwrap(), None);
    assert!(settings_rows(pool).await.is_empty());
    assert!(broker_docs(broker).is_empty());
}

#[tokio::test]
async fn backup_then_restore_brings_everything_back() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    let pa = a.db.pool();
    let pb = b.db.pool();
    fill(pa, &a.db).await;
    let source = source_broker();
    let (file, manifest) = backup(pa, &source).await;

    assert_eq!(manifest.format, 1);
    assert_eq!(manifest.migration, migration_level(pa).await.unwrap());
    assert_eq!(manifest.retained, 3);
    assert_eq!(manifest.flows, 1);
    assert_eq!(manifest.dashboards, 1);
    assert_eq!(manifest.tables["entity_state"].rows, 50);
    assert_eq!(manifest.tables["climate_decision"].rows, 1);
    assert!(!manifest.tables.contains_key("settings"));
    assert!(manifest.history_bytes > 0);
    assert!(!manifest.host.is_empty());

    let target = MemBroker::default();
    let parts = Mutex::new(Vec::new());
    let restored = read_backup(&file[..], PASSWORD, pb, &target, &|part, bytes| {
        parts.lock().unwrap().push((part, bytes));
    })
    .await
    .unwrap();

    assert_eq!(restored.manifest, manifest);
    assert_eq!(restored.hub, hub_fields());
    assert_eq!(restored.settings, settings_rows(pa).await);
    assert_eq!(settings_rows(pb).await, settings_rows(pa).await);
    assert_eq!(
        restored.settings["home"],
        json!({"name": "Maison"}),
        "{:?}",
        restored.settings
    );
    for table in manifest.tables.keys() {
        assert_eq!(
            contents(pb, table).await,
            contents(pa, table).await,
            "{table}"
        );
    }
    let mut expected = broker_docs(&source);
    expected.remove("nidoo/alarm/state");
    expected.remove("nidoo/climate/zone/z/decision");
    assert_eq!(broker_docs(&target), expected);

    let parts = parts.into_inner().unwrap();
    for part in [
        BackupPart::Settings,
        BackupPart::Retained,
        BackupPart::History,
    ] {
        assert!(
            parts.iter().any(|(p, _)| *p == part),
            "{part:?} in {parts:?}"
        );
    }
    assert!(parts.windows(2).all(|w| w[0].1 <= w[1].1), "{parts:?}");
    a.drop_db().await;
    b.drop_db().await;
}

#[tokio::test]
async fn runtime_topics_are_left_out() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    let (file, manifest) = backup(a.db.pool(), &source_broker()).await;
    assert_eq!(manifest.retained, 3);

    let target = MemBroker::default();
    read_backup(&file[..], PASSWORD, b.db.pool(), &target, &|_, _| {})
        .await
        .unwrap();
    let restored = broker_docs(&target);
    assert!(!restored.contains_key("nidoo/alarm/state"), "{restored:?}");
    assert!(
        !restored.contains_key("nidoo/climate/zone/z/decision"),
        "{restored:?}"
    );
    assert_eq!(restored.len(), 3);
    a.drop_db().await;
    b.drop_db().await;
}

/// Ruling A: the head of the file is 256 KiB, the first age chunk (64 KiB + tag) and more.
#[tokio::test]
async fn manifest_is_the_first_entry() {
    let Some(a) = fresh_db().await else { return };
    fill(a.db.pool(), &a.db).await;
    let (file, manifest) = backup(a.db.pool(), &source_broker()).await;
    let head = &file[..file.len().min(256 * 1024)];
    assert_eq!(read_manifest(head, PASSWORD).await.unwrap(), manifest);
    a.drop_db().await;
}

/// Review Focus 2.
#[tokio::test]
async fn wrong_password_writes_nothing() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    fill(a.db.pool(), &a.db).await;
    let (file, _) = backup(a.db.pool(), &source_broker()).await;

    let target = MemBroker::default();
    let err = read_backup(
        &file[..],
        "not the password",
        b.db.pool(),
        &target,
        &|_, _| {},
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err, BackupError::WrongPassword);
    assert_untouched(b.db.pool(), &target).await;
    a.drop_db().await;
    b.drop_db().await;
}

/// Review Focus 2.
#[tokio::test]
async fn newer_backup_is_refused() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    let pa = a.db.pool();
    let pb = b.db.pool();
    fill(pa, &a.db).await;
    let target_level = migration_level(pb).await.unwrap();
    // The source hub has run a migration this one does not know yet.
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES ($1, 'from the future', true, '\\x00', 0)",
    )
    .bind(target_level + 1)
    .execute(pa)
    .await
    .unwrap();
    let (file, manifest) = backup(pa, &source_broker()).await;
    assert_eq!(manifest.migration, target_level + 1);
    assert_eq!(
        check_compatible(&manifest, target_level),
        Err(BackupError::TooNew)
    );

    let target = MemBroker::default();
    let err = read_backup(&file[..], PASSWORD, pb, &target, &|_, _| {})
        .await
        .err()
        .unwrap();
    assert_eq!(err, BackupError::TooNew);
    assert_untouched(pb, &target).await;
    a.drop_db().await;
    b.drop_db().await;
}

#[tokio::test]
async fn truncated_file_fails() {
    let Some(a) = fresh_db().await else { return };
    fill(a.db.pool(), &a.db).await;
    let (file, _) = backup(a.db.pool(), &source_broker()).await;
    let cut = &file[..file.len() * 7 / 10];

    let Some(b) = fresh_db().await else { return };
    let result = read_backup(
        cut,
        PASSWORD,
        b.db.pool(),
        &MemBroker::default(),
        &|_, _| {},
    )
    .await;
    assert!(result.is_err(), "a file cut at 70 % restored");
    a.drop_db().await;
    b.drop_db().await;
}
