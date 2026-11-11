//! A scratch TimescaleDB per test, shared by the database-backed integration tests.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use nidoo::history::db::Db;
use sqlx::{PgPool, postgres::PgPoolOptions};
use url::Url;

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TestDb {
    pub db: Db,
    pub admin: PgPool,
    pub name: String,
    /// The URL of the scratch database, for a hub that opens its own pool.
    pub url: String,
}

impl TestDb {
    pub async fn drop_db(self) {
        self.db.close().await;
        let _ = sqlx::query(&format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
            self.name
        ))
        .execute(&self.admin)
        .await;
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\"", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
    }
}

pub async fn fresh_db() -> Option<TestDb> {
    let base_url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipping: TEST_DATABASE_URL unset");
            return None;
        }
    };
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let name = format!("nidoo_test_{}_{}", std::process::id(), n);
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&base_url)
        .await
        .ok()?;
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&admin)
        .await
        .ok()?;
    let mut url = Url::parse(&base_url).ok()?;
    url.set_path(&format!("/{name}"));
    let db = Db::connect_lazy(url.as_str()).ok()?;
    db.migrate().await.ok()?;
    Some(TestDb {
        db,
        admin,
        name,
        url: url.to_string(),
    })
}

/// Compresses every chunk of `table`, returning how many there are. The test rows are old, so
/// the database's own compression policy may compress a chunk at the same moment;
/// `if_not_compressed` only checks first, so "already compressed" (42710) still means the chunk
/// is compressed, which is all a test needs.
pub async fn compress_chunks(pool: &PgPool, table: &str) -> usize {
    let chunks: Vec<String> =
        sqlx::query_scalar(&format!("SELECT c::text FROM show_chunks('{table}') c"))
            .fetch_all(pool)
            .await
            .unwrap();
    for chunk in &chunks {
        let compressed = sqlx::query(&format!(
            "SELECT compress_chunk('{chunk}'::regclass, if_not_compressed => true)"
        ))
        .execute(pool)
        .await;
        match compressed {
            Ok(_) => {}
            Err(sqlx::Error::Database(err)) if err.code().as_deref() == Some("42710") => {}
            Err(err) => panic!("compress {chunk}: {err}"),
        }
    }
    chunks.len()
}
