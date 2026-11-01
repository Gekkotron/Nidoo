//! Hourly rollup task. Every 5 min (plus one run 30 s after start) it fills
//! [`history_hourly`] for the window `[from, hour_floor(now))`, where `from` is
//! `min(done_until, dirty_from, hour_floor(now) − HOUR)` and `done_until` falls back to
//! `hour_floor(first_time)` on first run. The window is walked in 24 h slices.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time;

use nidoo_core::history::buckets::{Bucket, Sample, rollup};
use nidoo_core::history::{HOUR, Micros};

use super::History;
use super::db::Db;

const START_DELAY: Duration = Duration::from_secs(30);
const INTERVAL: Duration = Duration::from_secs(5 * 60);
const DAY_STEP: Micros = 24 * HOUR;

pub async fn run(db: Db, history: Arc<History>) {
    time::sleep(START_DELAY).await;
    loop {
        let now = crate::clock::now_micros();
        // Take the current mark atomically. On failure, keep the lowest of the mark we
        // took and anything the writer has set in the meantime, so late rows that arrived
        // during the pass are not dropped on the floor.
        let dirty_from = history.dirty_from.swap(i64::MAX, Ordering::Relaxed);
        if let Err(e) = rollup_once(&db, now, dirty_from).await {
            tracing::warn!(error = %e, "history rollup failed");
            history.dirty_from.fetch_min(dirty_from, Ordering::Relaxed);
        }
        time::sleep(INTERVAL).await;
    }
}

/// Public so tests can drive the rollup synchronously.
pub async fn rollup_once(db: &Db, now: Micros, dirty_from: Micros) -> Result<(), sqlx::Error> {
    let to = hour_floor(now);

    let done_until = db.done_until().await?;
    let starting_point = match done_until {
        Some(d) => d,
        None => match db.first_time().await? {
            Some(first) => hour_floor(first),
            None => return Ok(()),
        },
    };

    // from = min(done_until | hour_floor(first_time), dirty_from, hour_floor(now) − HOUR)
    let last_closed = to.saturating_sub(HOUR);
    let from = hour_floor(starting_point.min(dirty_from).min(last_closed));

    if from >= to {
        db.set_done_until(to).await?;
        return Ok(());
    }

    let mut cursor = from;
    while cursor < to {
        let step_end = (cursor + DAY_STEP).min(to);
        let samples = db.samples(cursor, step_end, None).await?;

        let mut by_entity: HashMap<String, Vec<Sample>> = HashMap::new();
        for (id, sample) in samples {
            by_entity.entry(id).or_default().push(sample);
        }

        let entities: Vec<String> = by_entity.keys().cloned().collect();
        let carries = db.hourly_carry(cursor, &entities).await?;

        let mut upserts: Vec<(String, Bucket)> = Vec::new();
        for (id, samples) in by_entity {
            let carry = carries.get(&id).copied();
            for b in rollup(carry, &samples, cursor, step_end, HOUR) {
                upserts.push((id.clone(), b));
            }
        }
        db.upsert_hourly(&upserts).await?;
        cursor = step_end;
    }

    db.set_done_until(to).await?;
    Ok(())
}

fn hour_floor(t: Micros) -> Micros {
    t.div_euclid(HOUR) * HOUR
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    #[test]
    fn dirty_from_mark_during_rollup_is_preserved() {
        // Mimics the live race: a writer marks hour `late` while the rollup pass is in
        // flight. The pass must not clobber the writer's mark on completion.
        let dirty_from = AtomicI64::new(super::HOUR);

        // Pass starts: take the current mark atomically.
        let for_pass = dirty_from.swap(i64::MAX, Ordering::Relaxed);
        assert_eq!(for_pass, super::HOUR);
        assert_eq!(dirty_from.load(Ordering::Relaxed), i64::MAX);

        // Writer concurrently flushes a late row for an earlier hour.
        let late_hour = super::HOUR / 2;
        dirty_from.fetch_min(late_hour, Ordering::Relaxed);

        // Pass completes successfully: it does NOT touch dirty_from — the writer's mark
        // stays set so the next pass picks it up.
        assert_eq!(dirty_from.load(Ordering::Relaxed), late_hour);
    }
}
