//! Hub-side history: the Postgres pool and schema (`db`), the bounded writer task that
//! feeds it without blocking the ingest path (`writer`), and the hourly rollup task that
//! fills `history_hourly` (`rollup`). `restore` puts the last recorded value back on an entity
//! that has not spoken since boot.

pub mod climate;
pub mod db;
pub mod query;
pub mod restore;
pub mod rollup;
pub mod solar;
pub mod writer;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use chrono_tz::Tz;
use tokio::sync::mpsc;

use nidoo_core::history::Incoming;

use self::db::Db;
use crate::config::HistoryConfig;

/// The channel the ingest path hands [`Incoming`]s to. The writer task drains it; a full
/// channel drops the state via `try_send` so `Hub::ingest` never blocks.
const CHANNEL_CAPACITY: usize = 4096;

/// Why a history read failed.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history unavailable: {0}")]
    Unavailable(String),
    #[error("not recorded")]
    NotRecorded,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("no grid meter")]
    NoGridMeter,
}

impl HistoryError {
    /// The wire code used in HTTP and MCP error bodies.
    pub fn code(&self) -> &'static str {
        match self {
            HistoryError::Unavailable(_) => "history_unavailable",
            HistoryError::NotRecorded => "not_recorded",
            HistoryError::BadRequest(_) => "bad_request",
            HistoryError::NoGridMeter => "no_grid_meter",
        }
    }
}

/// Shared history handle: the sender into the writer, the pool (`None` for test-only
/// instances), the house zone and the rollup watermark.
pub struct History {
    sender: mpsc::Sender<Incoming>,
    db: Option<Db>,
    zone: Tz,
    drop_count: AtomicU64,
    /// Lowest hour (in microseconds) whose rollup may now be stale. The rollup task reads
    /// this and resets it to `i64::MAX` after a successful pass.
    pub(crate) dirty_from: AtomicI64,
}

impl History {
    /// Starts the writer and rollup tasks on a lazy pool. The pool connects on the first
    /// query, so the hub boots whether the database is up or not.
    pub fn start(cfg: &HistoryConfig, zone: Tz) -> Result<Arc<Self>, sqlx::Error> {
        let db = Db::connect_lazy(&cfg.url)?;
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let history = Arc::new(Self {
            sender: tx,
            db: Some(db.clone()),
            zone,
            drop_count: AtomicU64::new(0),
            dirty_from: AtomicI64::new(i64::MAX),
        });
        tokio::spawn(writer::run(
            db.clone(),
            history.clone(),
            rx,
            cfg.buffer_rows,
        ));
        tokio::spawn(rollup::run(db, history.clone()));
        Ok(history)
    }

    /// Builds a History wrapping an existing [`Db`] with no spawned tasks — the caller
    /// drives the recorder loop. Mostly used by tests on a per-test database.
    #[doc(hidden)]
    pub fn with_db(db: Db, zone: Tz) -> Arc<Self> {
        let (tx, _rx) = mpsc::channel(16);
        Arc::new(Self {
            sender: tx,
            db: Some(db),
            zone,
            drop_count: AtomicU64::new(0),
            dirty_from: AtomicI64::new(i64::MAX),
        })
    }

    /// Starts just the writer task against the given config, returning the handle so a
    /// test can observe whether the writer survives a database that is down at boot.
    #[doc(hidden)]
    pub fn spawn_writer_for_tests(
        cfg: &HistoryConfig,
        zone: Tz,
    ) -> Result<(Arc<Self>, tokio::task::JoinHandle<()>), sqlx::Error> {
        let db = Db::connect_lazy(&cfg.url)?;
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let history = Arc::new(Self {
            sender: tx,
            db: Some(db.clone()),
            zone,
            drop_count: AtomicU64::new(0),
            dirty_from: AtomicI64::new(i64::MAX),
        });
        let handle = tokio::spawn(writer::run(db, history.clone(), rx, cfg.buffer_rows));
        Ok((history, handle))
    }

    /// Builds a History backed by nothing but a channel of `capacity`, for tests.
    pub fn detached(capacity: usize, zone: Tz) -> (Arc<Self>, mpsc::Receiver<Incoming>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        let h = Arc::new(Self {
            sender: tx,
            db: None,
            zone,
            drop_count: AtomicU64::new(0),
            dirty_from: AtomicI64::new(i64::MAX),
        });
        (h, rx)
    }

    /// Hands an incoming state to the writer. Never blocks; a full channel drops it and
    /// counts the drop. The first drop of each outage logs a warning; the next success
    /// logs a short "resumed" line.
    pub fn offer(&self, incoming: Incoming) {
        match self.sender.try_send(incoming) {
            Ok(()) => {
                let prior = self.drop_count.swap(0, Ordering::Relaxed);
                if prior > 0 {
                    tracing::info!(dropped = prior, "history writer resumed");
                }
            }
            Err(_) => {
                let n = self.drop_count.fetch_add(1, Ordering::Relaxed);
                if n == 0 {
                    tracing::warn!("history channel full; dropping incoming state(s)");
                }
            }
        }
    }

    pub fn db(&self) -> Result<&Db, HistoryError> {
        self.db
            .as_ref()
            .ok_or_else(|| HistoryError::Unavailable("history not configured".into()))
    }

    pub fn zone(&self) -> Tz {
        self.zone
    }
}
