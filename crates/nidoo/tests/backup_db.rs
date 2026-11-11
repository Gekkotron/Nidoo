//! Copying the history tables out of one database and into another, as a backup and a restore
//! do.
//!
//! Needs `TEST_DATABASE_URL` (see `history_db.rs`); without it each test prints
//! `skipping: TEST_DATABASE_URL unset` and returns.

use crate::common;

use bytes::Bytes;
use common::db::fresh_db;
use futures_util::TryStreamExt;
use nidoo::backup::db::{
    CopyIn, Snapshot, TableInfo, fix_sequences, holds_history, map_columns, migration_level,
    target_columns, wipe,
};
use nidoo_core::history::{AvailabilityRow, Rows, ValueRow};
use nidoo_core::model::{Availability, StateValue};
use serde_json::{Value, json};
use sqlx::PgPool;

/// The tables a hub writes history to; each gets rows in the round trip.
const HISTORY: [&str; 5] = [
    "entity_state",
    "entity_availability",
    "climate_decision",
    "history_hourly",
    "assist_turn",
];

/// 2025-01-01T00:00:00Z in microseconds.
const T0: i64 = 1_735_689_600_000_000;
const DAY: i64 = 86_400_000_000;

fn value_row(id: &str, at: i64, num: f64) -> ValueRow {
    ValueRow {
        at,
        id: id.into(),
        value: StateValue::Number(num),
        num: Some(num),
    }
}

fn availability_row(id: &str, at: i64, availability: Availability) -> AvailabilityRow {
    AvailabilityRow {
        at,
        id: id.into(),
        availability,
    }
}

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap();
}

/// Rows in every history table, with text that needs COPY escaping and a NULL.
async fn fill(pool: &PgPool, db: &nidoo::history::db::Db) {
    db.insert(&Rows {
        values: vec![
            value_row("sensor.a", T0, 21.5),
            value_row("sensor.a", T0 + DAY, 0.1),
            value_row("sensor.b", T0 + 10 * DAY, -3.25),
        ],
        availability: vec![
            availability_row("sensor.a", T0, Availability::Online),
            availability_row("sensor.b", T0 + 10 * DAY, Availability::Offline),
        ],
    })
    .await
    .unwrap();
    exec(
        pool,
        r#"INSERT INTO entity_state (time, entity_id, num, value)
           VALUES ('2025-01-03T04:05:06.123456Z', 'text.note', NULL,
                   '"tab\there\nnew line \\ backslash é"')"#,
    )
    .await;
    exec(
        pool,
        "INSERT INTO climate_decision
             (time, zone, entity_id, mode, setpoint, source, reason, applied, adjust)
         VALUES ('2025-01-02T00:00:00Z', 'living', 'climate.living', 'heat', 20.5,
                 'schedule', 'morning', true, 0.5),
                ('2025-01-02T01:00:00Z', 'living', 'climate.living', 'off', NULL,
                 'manual', 'window open', false, NULL)",
    )
    .await;
    exec(
        pool,
        "INSERT INTO history_hourly
             (entity_id, bucket, covered_us, integral, min, max, last, increase)
         VALUES ('sensor.a', '2025-01-01T00:00:00Z', 3600000000, 21.5, 21, 22, 21.5, 0),
                ('sensor.a', '2025-01-01T01:00:00Z', 1800000000, 0.30000000000000004,
                 0.1, 0.2, 0.2, 1e-9)",
    )
    .await;
    exec(
        pool,
        r#"INSERT INTO assist_turn
               (id, at, client, text, simulate, outcome, reply, question, trace, wrong)
           VALUES ('t1', '2025-01-04T00:00:00Z', 'page-1', E'mets le salon\tà 25\n',
                   false, 'confirming', 'Passer le Salon à 25 °C ?',
                   '{"pending": "p1"}', '{"words": ["mets", "le"]}', false),
                  ('t2', '2025-01-04T00:00:01Z', 'satellite', 'ouvre les volets', true,
                   'done', 'C''est fait', NULL, '{}', true)"#,
    )
    .await;
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

/// What a backup carries: each table's description and its `COPY` output.
async fn export(pool: &PgPool) -> Vec<(TableInfo, Vec<Bytes>)> {
    let mut snap = Snapshot::begin(pool).await.unwrap();
    let mut out = Vec::new();
    for t in snap.tables().await.unwrap() {
        let data: Vec<Bytes> = snap.copy_out(&t).try_collect().await.unwrap();
        out.push((t, data));
    }
    out
}

/// What a restore does with it; every `finish` count must equal the exported row count.
async fn import(pool: &PgPool, data: &[(TableInfo, Vec<Bytes>)]) {
    for (t, chunks) in data {
        let target = target_columns(pool, &t.name).await.unwrap();
        let columns = map_columns(&t.name, &t.columns, &target).unwrap();
        let mut copy = CopyIn::start(pool.acquire().await.unwrap(), &t.name, &columns)
            .await
            .unwrap();
        for c in chunks {
            copy.send(c.clone()).await.unwrap();
        }
        assert_eq!(
            copy.finish().await.unwrap(),
            t.rows,
            "rows copied into {}",
            t.name
        );
    }
}

fn names(data: &[(TableInfo, Vec<Bytes>)]) -> Vec<&str> {
    data.iter().map(|(t, _)| t.name.as_str()).collect()
}

#[tokio::test]
async fn export_then_import_round_trips_every_table() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    let pa = a.db.pool();
    let pb = b.db.pool();
    fill(pa, &a.db).await;
    exec(
        pa,
        r#"INSERT INTO settings (key, value) VALUES ('mobile', '{"enabled": true}'),
                                                   ('home', '{"name": "Maison"}')"#,
    )
    .await;

    let data = export(pa).await;
    let tables = names(&data);
    assert!(tables.is_sorted(), "{tables:?}");
    assert!(!tables.contains(&"settings") && !tables.contains(&"_sqlx_migrations"));
    for h in HISTORY {
        let (t, _) = data.iter().find(|(t, _)| t.name == h).expect(h);
        assert!(t.rows > 0, "{h} has rows");
    }
    let (state, _) = data.iter().find(|(t, _)| t.name == "entity_state").unwrap();
    assert_eq!(state.columns, ["time", "seq", "entity_id", "num", "value"]);
    assert_eq!(state.rows, 4);

    let mut snap = Snapshot::begin(pa).await.unwrap();
    let settings = snap.settings_rows().await.unwrap();
    assert_eq!(settings.len(), 2);
    assert_eq!(settings["home"], json!({"name": "Maison"}));
    drop(snap);

    import(pb, &data).await;
    for (t, _) in &data {
        assert_eq!(
            contents(pb, &t.name).await,
            contents(pa, &t.name).await,
            "{}",
            t.name
        );
    }
    a.drop_db().await;
    b.drop_db().await;
}

#[tokio::test]
async fn compressed_chunks_export() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    let pa = a.db.pool();
    let pb = b.db.pool();
    fill(pa, &a.db).await;
    let compressed: Vec<String> =
        sqlx::query_scalar("SELECT compress_chunk(c)::text FROM show_chunks('entity_state') c")
            .fetch_all(pa)
            .await
            .unwrap();
    assert!(!compressed.is_empty());
    let before = contents(pa, "entity_state").await;

    let data = export(pa).await;
    import(pb, &data).await;
    assert_eq!(contents(pb, "entity_state").await, before);
    assert_eq!(before.as_array().unwrap().len(), 4);
    a.drop_db().await;
    b.drop_db().await;
}

/// Review Focus 1: after a restore the next history row gets a counter past every restored one.
#[tokio::test]
async fn sequences_move_past_restored_rows() {
    let Some(a) = fresh_db().await else { return };
    let Some(b) = fresh_db().await else { return };
    let pa = a.db.pool();
    let pb = b.db.pool();
    fill(pa, &a.db).await;
    import(pb, &export(pa).await).await;
    fix_sequences(pb).await.unwrap();

    b.db.insert(&Rows {
        values: vec![value_row("sensor.new", T0 + 20 * DAY, 1.0)],
        availability: vec![availability_row(
            "sensor.new",
            T0 + 20 * DAY,
            Availability::Online,
        )],
    })
    .await
    .unwrap();
    exec(
        pb,
        "INSERT INTO climate_decision (time, zone, entity_id, mode, source, reason, applied)
         VALUES ('2025-02-01T00:00:00Z', 'new', 'climate.new', 'off', 'manual', 'r', false)",
    )
    .await;
    exec(
        pb,
        "INSERT INTO assist_turn (id, at, client, text, simulate, outcome, reply, trace)
         VALUES ('new', '2025-02-01T00:00:00Z', 'c', 't', false, 'done', 'r', '{}')",
    )
    .await;

    for (table, key) in [
        ("entity_state", "entity_id"),
        ("entity_availability", "entity_id"),
        ("climate_decision", "entity_id"),
        ("assist_turn", "id"),
    ] {
        let new = if key == "id" { "new" } else { "%new" };
        let (fresh, restored): (i64, i64) = sqlx::query_as(&format!(
            "SELECT max(seq) FILTER (WHERE {key} LIKE $1),
                    max(seq) FILTER (WHERE {key} NOT LIKE $1)
             FROM {table}"
        ))
        .bind(new)
        .fetch_one(pb)
        .await
        .unwrap();
        assert!(
            fresh > restored,
            "{table}: new seq {fresh} <= restored {restored}"
        );
    }
    a.drop_db().await;
    b.drop_db().await;
}

#[tokio::test]
async fn snapshot_ignores_later_writes() {
    let Some(a) = fresh_db().await else { return };
    let pa = a.db.pool();
    exec(
        pa,
        "INSERT INTO entity_state (time, entity_id, num, value)
         VALUES ('2025-01-01T00:00:00Z', 'before', 1, '1')",
    )
    .await;

    let mut snap = Snapshot::begin(pa).await.unwrap();
    exec(
        pa,
        "INSERT INTO entity_state (time, entity_id, num, value)
         VALUES ('2025-01-01T00:00:01Z', 'after', 2, '2')",
    )
    .await;
    exec(
        pa,
        "INSERT INTO hub_restart (started_at, stop) VALUES ('2025-01-01T00:00:00Z', 'first')",
    )
    .await;

    let tables = snap.tables().await.unwrap();
    let state = tables.iter().find(|t| t.name == "entity_state").unwrap();
    let restart = tables.iter().find(|t| t.name == "hub_restart").unwrap();
    assert_eq!(state.rows, 1);
    assert_eq!(restart.rows, 0);
    let data: Vec<Bytes> = snap.copy_out(state).try_collect().await.unwrap();
    let text = String::from_utf8(data.concat()).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("before") && !text.contains("after"), "{text}");
    drop(snap);
    a.drop_db().await;
}

#[tokio::test]
async fn holds_history_and_wipe() {
    let Some(a) = fresh_db().await else { return };
    let pa = a.db.pool();
    assert_eq!(holds_history(pa).await.unwrap(), None);

    exec(
        pa,
        "INSERT INTO entity_state (time, entity_id, num, value)
         VALUES ('2025-01-01T00:00:00Z', 'sensor.a', 1, '1')",
    )
    .await;
    exec(
        pa,
        r#"INSERT INTO settings (key, value) VALUES ('home', '{}')"#,
    )
    .await;
    assert_eq!(
        holds_history(pa).await.unwrap().as_deref(),
        Some("entity_state")
    );

    fill(pa, &a.db).await;
    exec(
        pa,
        "SELECT compress_chunk(c) FROM show_chunks('entity_state') c",
    )
    .await;
    wipe(pa).await.unwrap();
    assert_eq!(holds_history(pa).await.unwrap(), None);
    let settings: i64 = sqlx::query_scalar("SELECT count(*) FROM settings")
        .fetch_one(pa)
        .await
        .unwrap();
    assert_eq!(settings, 0);
    a.drop_db().await;
}

#[tokio::test]
async fn migration_level_is_the_last_migration() {
    let Some(a) = fresh_db().await else { return };
    let last = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.split('_').next()?.parse::<i64>().ok()
        })
        .max()
        .unwrap();
    assert_eq!(migration_level(a.db.pool()).await.unwrap(), last);
    a.drop_db().await;
}
