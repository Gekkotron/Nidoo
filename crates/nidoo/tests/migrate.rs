//! Database side of the `oikos` to `nidoo` switch-over. Needs `TEST_DATABASE_URL` (a
//! superuser); each test skips without it, like the other database tests.

mod common;

use common::db::fresh_db;
use nidoo::migrate::db::{rename_db_and_role, rename_entity_ids, rename_settings};
use sqlx::{Connection, PgConnection, PgPool};

/// `rename_db_and_role` creates and drops one fixed-name temp role, so two of these tests in
/// parallel race on the same `pg_authid` row ("tuple concurrently updated"). They take this
/// lock for their whole body.
static ROLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// Compresses every chunk of the two history hypertables now. Rows older than the policy age
/// are compressed by a background job at an arbitrary moment; doing it here keeps the tests
/// deterministic.
async fn compress_all(pool: &PgPool) {
    for table in ["entity_state", "entity_availability"] {
        sqlx::query(&format!(
            "SELECT compress_chunk(c, if_not_compressed => true) FROM show_chunks('{table}') c"
        ))
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn renames_entity_ids_in_every_table() {
    let Some(t) = fresh_db().await else { return };
    let pool = t.db.pool();
    for sql in [
        "INSERT INTO entity_state (time, entity_id, value) VALUES (now(), 'oikos_alarm_house', '1')",
        "INSERT INTO entity_latest (entity_id, time, value) VALUES ('oikos_alarm_house', now(), '1')",
        "INSERT INTO history_hourly (entity_id, bucket, covered_us, integral, min, max, last, increase) \
         VALUES ('oikos_alarm_house', now(), 1, 1, 1, 1, 1, 1)",
        "INSERT INTO entity_availability (time, entity_id, availability) VALUES (now(), 'oikos_alarm_house', 'online')",
        "INSERT INTO availability_latest (entity_id, time, availability) VALUES ('oikos_alarm_house', now(), 'online')",
        "INSERT INTO climate_decision (time, zone, entity_id, mode, source, reason, applied) \
         VALUES (now(), 'z', 'oikos_alarm_house', 'off', 's', 'r', true)",
    ] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    let report = rename_entity_ids(pool, false).await.unwrap();
    for table in [
        "entity_state",
        "entity_latest",
        "history_hourly",
        "entity_availability",
        "availability_latest",
        "climate_decision",
    ] {
        let renamed = report.iter().find(|(n, _)| n == table).map(|(_, n)| *n);
        assert_eq!(renamed, Some(1), "{table}");
        let sql = format!("SELECT count(*) FROM {table} WHERE entity_id LIKE 'oikos%'");
        assert_eq!(count(pool, &sql).await, 0, "{table}");
        let sql = format!("SELECT count(*) FROM {table} WHERE entity_id = 'nidoo_alarm_house'");
        assert_eq!(count(pool, &sql).await, 1, "{table}");
    }
    t.drop_db().await;
}

#[tokio::test]
async fn renames_rows_in_compressed_chunk() {
    let Some(t) = fresh_db().await else { return };
    let pool = t.db.pool();
    sqlx::query(
        "INSERT INTO entity_state (time, entity_id, value) \
         VALUES (now() - interval '10 days', 'oikos_alarm_house', '1')",
    )
    .execute(pool)
    .await
    .unwrap();
    compress_all(pool).await;
    rename_entity_ids(pool, false).await.unwrap();
    let id: String = sqlx::query_scalar("SELECT entity_id FROM entity_state")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(id, "nidoo_alarm_house");
    t.drop_db().await;
}

#[tokio::test]
async fn entity_id_collision_keeps_new_row() {
    let Some(t) = fresh_db().await else { return };
    let pool = t.db.pool();
    sqlx::query(
        "INSERT INTO entity_state (time, entity_id, value) VALUES \
         ('2026-01-01T00:00:00Z', 'oikos_x', '1'), ('2026-01-01T00:00:00Z', 'nidoo_x', '2')",
    )
    .execute(pool)
    .await
    .unwrap();
    compress_all(pool).await;
    rename_entity_ids(pool, false).await.unwrap();
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT entity_id, value FROM entity_state")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(rows, vec![("nidoo_x".to_string(), serde_json::json!(2))]);
    t.drop_db().await;
}

#[tokio::test]
async fn availability_collision_keeps_new_row_when_compressed() {
    let Some(t) = fresh_db().await else { return };
    let pool = t.db.pool();
    sqlx::query(
        "INSERT INTO entity_availability (time, entity_id, availability) VALUES \
         ('2026-01-01T00:00:00Z', 'oikos_x', 'offline'), ('2026-01-01T00:00:00Z', 'nidoo_x', 'online')",
    )
    .execute(pool)
    .await
    .unwrap();
    compress_all(pool).await;
    rename_entity_ids(pool, false).await.unwrap();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT entity_id, availability FROM entity_availability")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(rows, vec![("nidoo_x".to_string(), "online".to_string())]);
    t.drop_db().await;
}

#[tokio::test]
async fn dry_run_changes_nothing() {
    let Some(t) = fresh_db().await else { return };
    let pool = t.db.pool();
    sqlx::query("INSERT INTO entity_state (time, entity_id, value) VALUES (now(), 'oikos_x', '1')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings (key, value) VALUES ('advanced', '{\"p\":\"oikos/ui\"}')")
        .execute(pool)
        .await
        .unwrap();
    let report = rename_entity_ids(pool, true).await.unwrap();
    assert_eq!(
        report
            .iter()
            .find(|(n, _)| n == "entity_state")
            .map(|(_, n)| *n),
        Some(1)
    );
    let changes = rename_settings(pool, true).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM entity_state WHERE entity_id = 'oikos_x'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM settings WHERE value::text LIKE '%oikos/ui%'"
        )
        .await,
        1
    );
    t.drop_db().await;
}

#[tokio::test]
async fn renames_settings_prefixes() {
    let Some(t) = fresh_db().await else { return };
    let pool = t.db.pool();
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES \
         ('advanced', '{\"topics\":{\"ui_prefix\":\"oikos/ui\",\"discovery_prefixes\":[\"oikos\"]}}'), \
         ('other', '{\"a\":1}')",
    )
    .execute(pool)
    .await
    .unwrap();
    let changes = rename_settings(pool, false).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, "advanced");
    let v: serde_json::Value =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'advanced'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"topics":{"ui_prefix":"nidoo/ui","discovery_prefixes":["nidoo"]}})
    );
    t.drop_db().await;
}

#[tokio::test]
async fn renames_database_and_role() {
    let _lock = ROLE_LOCK.lock().await;
    let Some(t) = fresh_db().await else { return };
    let mut base = url::Url::parse(&std::env::var("TEST_DATABASE_URL").unwrap()).unwrap();
    let suffix = format!("{}_{}", std::process::id(), t.name.len());
    let from = format!("oikos_t{suffix}");
    let to = format!("nidoo_t{suffix}");
    let pw = "s3cret-pw";
    let admin = &t.admin;
    sqlx::query(&format!(
        "CREATE ROLE \"{from}\" SUPERUSER LOGIN PASSWORD '{pw}'"
    ))
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(&format!("CREATE DATABASE \"{from}\" OWNER \"{from}\""))
        .execute(admin)
        .await
        .unwrap();
    base.set_username(&from).unwrap();
    base.set_password(Some(pw)).unwrap();
    base.set_path(&format!("/{from}"));
    let url = base.to_string();

    rename_db_and_role(&url, &from, &to).await.unwrap();
    // A second call has nothing left to do.
    rename_db_and_role(&url, &from, &to).await.unwrap();

    base.set_username(&to).unwrap();
    base.set_path(&format!("/{to}"));
    let mut conn = PgConnection::connect(base.as_str()).await.unwrap();
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(one, 1);
    conn.close().await.unwrap();
    assert_eq!(
        count(
            admin,
            "SELECT count(*) FROM pg_roles WHERE rolname = 'nidoo_migrate_tmp'"
        )
        .await,
        0
    );

    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{to}\""))
        .execute(admin)
        .await;
    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{from}\""))
        .execute(admin)
        .await;
    let _ = sqlx::query(&format!("DROP ROLE IF EXISTS \"{to}\""))
        .execute(admin)
        .await;
    let _ = sqlx::query(&format!("DROP ROLE IF EXISTS \"{from}\""))
        .execute(admin)
        .await;
    t.drop_db().await;
}

async fn throwaway(t: &common::db::TestDb, tag: &str) -> (url::Url, String, String) {
    let mut base = url::Url::parse(&std::env::var("TEST_DATABASE_URL").unwrap()).unwrap();
    let suffix = format!("{tag}{}", t.name.trim_start_matches("oikos_test_"));
    let from = format!("oikos_t{suffix}");
    let to = format!("nidoo_t{suffix}");
    sqlx::query(&format!(
        "CREATE ROLE \"{from}\" SUPERUSER LOGIN PASSWORD 'pw'"
    ))
    .execute(&t.admin)
    .await
    .unwrap();
    sqlx::query(&format!("CREATE DATABASE \"{from}\" OWNER \"{from}\""))
        .execute(&t.admin)
        .await
        .unwrap();
    base.set_username(&from).unwrap();
    base.set_password(Some("pw")).unwrap();
    base.set_path(&format!("/{from}"));
    (base, from, to)
}

async fn cleanup(admin: &PgPool, names: &[&str]) {
    for n in names {
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{n}\""))
            .execute(admin)
            .await;
    }
    for n in names {
        let _ = sqlx::query(&format!("DROP ROLE IF EXISTS \"{n}\""))
            .execute(admin)
            .await;
    }
}

const NO_TMP: &str = "SELECT count(*) FROM pg_roles WHERE rolname = 'nidoo_migrate_tmp'";

#[tokio::test]
async fn failed_rename_still_drops_the_temp_role() {
    let _lock = ROLE_LOCK.lock().await;
    let Some(t) = fresh_db().await else { return };
    let (base, from, to) = throwaway(&t, "f").await;
    // An open session on the database makes ALTER DATABASE ... RENAME fail.
    let held = PgConnection::connect(base.as_str()).await.unwrap();
    let r = rename_db_and_role(base.as_str(), &from, &to).await;
    assert!(r.is_err());
    assert_eq!(count(&t.admin, NO_TMP).await, 0);
    held.close().await.unwrap();
    cleanup(&t.admin, &[&from, &to]).await;
    t.drop_db().await;
}

#[tokio::test]
async fn refuses_an_existing_target() {
    let _lock = ROLE_LOCK.lock().await;
    let Some(t) = fresh_db().await else { return };
    let (base, from, to) = throwaway(&t, "e").await;
    sqlx::query(&format!("CREATE DATABASE \"{to}\""))
        .execute(&t.admin)
        .await
        .unwrap();
    let r = rename_db_and_role(base.as_str(), &from, &to).await;
    assert!(r.is_err());
    assert_eq!(count(&t.admin, NO_TMP).await, 0);
    let dbs = format!("SELECT count(*) FROM pg_database WHERE datname = '{from}'");
    assert_eq!(count(&t.admin, &dbs).await, 1);
    let role = format!("SELECT count(*) FROM pg_roles WHERE rolname = '{from}'");
    assert_eq!(count(&t.admin, &role).await, 1);
    cleanup(&t.admin, &[&from, &to]).await;
    t.drop_db().await;
}
