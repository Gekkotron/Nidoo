//! Solar forecast from Solcast.

pub mod entities;
pub mod hub;
pub mod poller;
pub mod view;

use nidoo_solcast::Forecast;

/// Seconds a planned refresh may keep failing before the forecast is called stale.
const FAILING_LIMIT: u64 = 3600;
/// Seconds since the last success after which the forecast is called stale: longer than the
/// longest winter night between two refreshes.
const STALE_AFTER: u64 = 18 * 3600;
/// Seconds since the last success after which the entities go offline.
const EXPIRED_AFTER: u64 = 24 * 3600;

/// Whether the forecast is overdue: a planned refresh has been failing for over an hour, or the
/// last success is over 18 hours old.
pub fn is_stale(fetched_at: u64, failing_since: Option<u64>, now: u64) -> bool {
    failing_since.is_some_and(|since| now.saturating_sub(since) > FAILING_LIMIT)
        || now.saturating_sub(fetched_at) > STALE_AFTER
}

/// Whether the entities go offline: no period covers `now`, or the last success is over 24
/// hours old.
pub fn is_expired(forecast: &Forecast, now: u64) -> bool {
    let fetched_at = u64::try_from(forecast.fetched_at).unwrap_or(0);
    let now_i = i64::try_from(now).unwrap_or(i64::MAX);
    let covered = forecast
        .periods
        .iter()
        .any(|p| p.start().timestamp() <= now_i && now_i < p.end.timestamp());
    !covered || now.saturating_sub(fetched_at) > EXPIRED_AFTER
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use nidoo_solcast::{Period, merge};

    /// Half-hour periods ending every 30 minutes from `first_end` (unix seconds), `count` of them.
    fn forecast(fetched_at: i64, first_end: i64, count: i64) -> Forecast {
        let periods = (0..count)
            .map(|i| Period {
                end: DateTime::<Utc>::from_timestamp(first_end + 1800 * i, 0).unwrap(),
                minutes: 30,
                estimate: 1.0,
                low: 0.5,
                high: 1.5,
            })
            .collect();
        merge(vec![periods], fetched_at)
    }

    #[test]
    fn winter_night_is_not_stale() {
        // Fetched 15:30Z, now 07:30Z the next day (16 h); a period covers now.
        let fetched: i64 = 1_792_000_000;
        let now = fetched + 16 * 3600;
        let f = forecast(fetched, now - 1800 + 900, 4);
        assert!(!is_stale(fetched as u64, None, now as u64));
        assert!(!is_expired(&f, now as u64));
    }

    #[test]
    fn failing_over_an_hour_is_stale() {
        assert!(!is_stale(1000, Some(1000), 1000 + 3600));
        assert!(is_stale(1000, Some(1000), 1000 + 3601));
        assert!(!is_stale(1000, None, 1000 + 18 * 3600));
        assert!(is_stale(1000, None, 1000 + 18 * 3600 + 1));
    }

    #[test]
    fn expired_after_24h_or_past_last_period() {
        let start = 1_792_000_000;
        // 100 periods = 50 h of coverage from `start`.
        let f = forecast(start, start + 1800, 100);
        let mid = (start + 3600) as u64;
        assert!(!is_expired(&f, mid));
        assert!(is_expired(&f, (start + 50 * 3600) as u64)); // past the last period
        assert!(is_expired(&f, (start - 1) as u64)); // before the first
        // Exactly 24 h old is still online, one second more is not.
        assert!(!is_expired(&f, (start + 24 * 3600) as u64));
        assert!(is_expired(&f, (start + 24 * 3600 + 1) as u64));
    }
}
