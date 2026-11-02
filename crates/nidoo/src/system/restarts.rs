//! The hub's restart log, the uptime percentage derived from it, and the retention purge
//! of the system sensors' history.

use nidoo_api::system::{RestartView, StopKind, ids};
use sqlx::Row;

use crate::history::db::Db;

/// The uptime percentage looks back this far.
pub const WINDOW_SECS: u64 = 30 * 86_400;
/// System sensors and the restart log are kept this long.
pub const KEEP_SECS: u64 = 90 * 86_400;
/// How far before its cutoff the daily purge deletes history rows.
pub const PURGE_SPAN_SECS: u64 = 7 * 86_400;

/// Share of the last `window_secs` the hub was up, 0–100. The window never starts before
/// `first_start`, so a young hub is not penalised for time before it existed.
pub fn uptime_pct(
    restarts: &[RestartView],
    first_start: Option<u64>,
    now: u64,
    window_secs: u64,
) -> f64 {
    let full_start = now.saturating_sub(window_secs);
    let start = full_start.max(first_start.unwrap_or(full_start));
    let window = now.saturating_sub(start);
    if window == 0 {
        return 100.0;
    }
    let down: u64 = restarts
        .iter()
        .filter(|r| r.started_at > start)
        .map(|r| r.down_secs.unwrap_or(0).min(r.started_at - start))
        .sum();
    let up = window.saturating_sub(down);
    (100.0 * up as f64 / window as f64).clamp(0.0, 100.0)
}

fn stop_str(s: StopKind) -> &'static str {
    match s {
        StopKind::First => "first",
        StopKind::Clean => "clean",
        StopKind::Unexpected => "unexpected",
    }
}

fn stop_from(s: &str) -> StopKind {
    match s {
        "first" => StopKind::First,
        "clean" => StopKind::Clean,
        _ => StopKind::Unexpected,
    }
}

fn secs(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl Db {
    /// Records one start of the hub; a start already logged at that second is kept as is.
    pub async fn insert_restart(&self, r: &RestartView) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO hub_restart (started_at, down_secs, stop)
             VALUES (to_timestamp($1::double precision), $2, $3)
             ON CONFLICT (started_at) DO NOTHING",
        )
        .bind(secs(r.started_at))
        .bind(r.down_secs.map(secs))
        .bind(stop_str(r.stop))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The starts at or after `since` (Unix seconds), newest first.
    pub async fn restarts_since(&self, since: u64) -> Result<Vec<RestartView>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT extract(epoch FROM started_at)::bigint, down_secs, stop
             FROM hub_restart
             WHERE started_at >= to_timestamp($1::double precision)
             ORDER BY started_at DESC",
        )
        .bind(secs(since))
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .map(|r| RestartView {
                started_at: u64::try_from(r.get::<i64, _>(0)).unwrap_or(0),
                down_secs: r.get::<Option<i64>, _>(1).map(|v| v.max(0) as u64),
                stop: stop_from(r.get::<&str, _>(2)),
            })
            .collect())
    }

    /// The oldest logged start, if any.
    pub async fn first_start(&self) -> Result<Option<u64>, sqlx::Error> {
        let row =
            sqlx::query("SELECT extract(epoch FROM min(started_at))::bigint FROM hub_restart")
                .fetch_one(self.pool())
                .await?;
        Ok(row
            .get::<Option<i64>, _>(0)
            .map(|v| u64::try_from(v).unwrap_or(0)))
    }

    /// Deletes the system sensors' history and the restart log older than `cutoff`
    /// (Unix seconds). Returns the number of rows removed. The history deletes only look
    /// [`PURGE_SPAN_SECS`] back from `cutoff`: the purge runs daily, and an unbounded delete
    /// would visit (and decompress) every old compressed chunk of the whole house's history.
    pub async fn purge_system_before(&self, cutoff: u64) -> Result<u64, sqlx::Error> {
        let ids: Vec<String> = ids::ALL.iter().map(|s| (*s).to_string()).collect();
        let floor = secs(cutoff.saturating_sub(PURGE_SPAN_SECS));
        let cutoff = secs(cutoff);
        let mut removed = 0;
        for sql in [
            "DELETE FROM entity_state
             WHERE entity_id = ANY($1)
               AND time >= to_timestamp($3::double precision)
               AND time < to_timestamp($2::double precision)",
            "DELETE FROM entity_availability
             WHERE entity_id = ANY($1)
               AND time >= to_timestamp($3::double precision)
               AND time < to_timestamp($2::double precision)",
            "DELETE FROM history_hourly
             WHERE entity_id = ANY($1)
               AND bucket >= to_timestamp($3::double precision)
               AND bucket < to_timestamp($2::double precision)",
        ] {
            removed += sqlx::query(sql)
                .bind(&ids)
                .bind(cutoff)
                .bind(floor)
                .execute(self.pool())
                .await?
                .rows_affected();
        }
        removed += sqlx::query(
            "DELETE FROM hub_restart WHERE started_at < to_timestamp($1::double precision)",
        )
        .bind(cutoff)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 100 * 86_400;

    fn restart(started_at: u64, down: Option<u64>) -> RestartView {
        RestartView {
            started_at,
            down_secs: down,
            stop: StopKind::Unexpected,
        }
    }

    #[test]
    fn uptime_full_month_one_hour_down() {
        let r = [restart(NOW - 86_400, Some(3600))];
        let p = uptime_pct(&r, Some(NOW - 60 * 86_400), NOW, WINDOW_SECS);
        assert!((p - 99.861).abs() < 0.001, "{p}");
    }

    #[test]
    fn uptime_young_hub_uses_first_start() {
        let r = [restart(NOW - 3600, Some(864))];
        let p = uptime_pct(&r, Some(NOW - 86_400), NOW, WINDOW_SECS);
        assert!((p - 99.0).abs() < 1e-9, "{p}");
    }

    #[test]
    fn uptime_without_restarts_is_100() {
        assert_eq!(uptime_pct(&[], Some(NOW - 86_400), NOW, WINDOW_SECS), 100.0);
        assert_eq!(uptime_pct(&[], None, NOW, WINDOW_SECS), 100.0);
        assert_eq!(uptime_pct(&[], None, NOW, 0), 100.0);
    }

    #[test]
    fn uptime_clamps_downtime_to_window() {
        // The hub just came back after a 40-day outage: the whole window was down.
        let r = [restart(NOW, Some(40 * 86_400))];
        assert_eq!(
            uptime_pct(&r, Some(NOW - 60 * 86_400), NOW, WINDOW_SECS),
            0.0
        );
        // Several long outages can never push it below zero.
        let r = [
            restart(NOW, Some(40 * 86_400)),
            restart(NOW - 7200, Some(40 * 86_400)),
        ];
        assert_eq!(uptime_pct(&r, None, NOW, WINDOW_SECS), 0.0);
    }
}
