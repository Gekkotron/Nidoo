//! Writer task: drains the writer channel into a bounded [`Pending`] buffer, diffs each
//! batch against the dedupe cache, and inserts the resulting rows.
//!
//! The task survives a database that is down at boot or mid-run: migrations and the
//! initial cache seed are retried on the tick, and until they succeed the loop keeps
//! draining `rx` into `pending`. On an insert error the batch is re-queued at the front
//! and the task backs off exponentially up to 30 s.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{self, MissedTickBehavior};

use nidoo_core::history::{HOUR, Incoming, LastWritten, Pending, Rows};

use super::History;
use super::db::Db;

/// How many pending incoming states trigger an early flush. The 1 s tick covers slow
/// streams; this covers busy ones.
const BATCH_THRESHOLD: usize = 500;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
pub(crate) const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub async fn run(
    db: Db,
    history: Arc<History>,
    mut rx: mpsc::Receiver<Incoming>,
    buffer_rows: usize,
) {
    let mut pending = Pending::new(buffer_rows.max(1));
    let mut last_written: Option<LastWritten> = None;
    let mut backoff = INITIAL_BACKOFF;
    let mut tick = time::interval(FLUSH_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut pending_drops: u64 = 0;
    let mut drop_logged = false;

    loop {
        tokio::select! {
            maybe = rx.recv() => {
                let Some(inc) = maybe else {
                    if let Some(ref mut lw) = last_written {
                        flush(&db, &mut pending, lw, &history, &mut backoff).await;
                    }
                    return;
                };
                if !pending.push(inc) {
                    pending_drops += 1;
                    if !drop_logged {
                        tracing::warn!("history pending buffer full; dropping oldest states");
                        drop_logged = true;
                    }
                }
                if let Some(lw) = last_written.as_mut()
                    && pending.len() >= BATCH_THRESHOLD
                {
                    flush(&db, &mut pending, lw, &history, &mut backoff).await;
                    note_pending_resume(&pending, &mut pending_drops, &mut drop_logged);
                }
            }
            _ = tick.tick() => {
                if last_written.is_none() {
                    match init(&db).await {
                        Ok(lw) => {
                            last_written = Some(lw);
                            backoff = INITIAL_BACKOFF;
                            tracing::info!("history writer ready");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "history init failed; will retry");
                            continue;
                        }
                    }
                }
                if let Some(lw) = last_written.as_mut()
                    && !pending.is_empty()
                {
                    flush(&db, &mut pending, lw, &history, &mut backoff).await;
                    note_pending_resume(&pending, &mut pending_drops, &mut drop_logged);
                }
            }
        }
    }
}

/// Migration + initial cache seed. Both must succeed before any insert.
async fn init(db: &Db) -> Result<LastWritten, InitError> {
    db.migrate().await.map_err(InitError::Migrate)?;
    let lw = db.load_latest().await.map_err(InitError::Load)?;
    Ok(lw)
}

#[derive(Debug, thiserror::Error)]
enum InitError {
    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("load_latest: {0}")]
    Load(#[from] sqlx::Error),
}

fn note_pending_resume(pending: &Pending, drops: &mut u64, logged: &mut bool) {
    if pending.is_empty() && *drops > 0 {
        tracing::info!(dropped = *drops, "history pending buffer drained");
        *drops = 0;
        *logged = false;
    }
}

/// Flush as much of `pending` as the batch size allows, in order. Returns after one
/// batch's attempt — the caller re-enters the loop and tries again next tick.
async fn flush(
    db: &Db,
    pending: &mut Pending,
    last_written: &mut LastWritten,
    history: &Arc<History>,
    backoff: &mut Duration,
) {
    let batch = pending.take(BATCH_THRESHOLD);
    if batch.is_empty() {
        return;
    }
    let mut tentative = last_written.clone();
    let mut rows = Rows::default();
    for inc in &batch {
        tentative.diff(inc, &mut rows);
    }
    let oldest_at = batch.iter().map(|i| i.at).min();

    if rows.is_empty() {
        // Nothing to insert but we still consumed the batch (all duplicates).
        *last_written = tentative;
        *backoff = INITIAL_BACKOFF;
        return;
    }
    match db.insert(&rows).await {
        Ok(()) => {
            *last_written = tentative;
            *backoff = INITIAL_BACKOFF;
            if let Some(oldest) = oldest_at {
                let oldest_hour = (oldest.div_euclid(HOUR)) * HOUR;
                let now = crate::clock::now_micros();
                let current_hour = (now.div_euclid(HOUR)) * HOUR;
                if oldest_hour < current_hour {
                    history.dirty_from.fetch_min(oldest_hour, Ordering::Relaxed);
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, batch = batch.len(), "history insert failed; backing off");
            pending.requeue_front(batch);
            time::sleep(*backoff).await;
            *backoff = (*backoff * 2).min(MAX_BACKOFF);
        }
    }
}
