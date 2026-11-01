//! Wall-clock bookkeeping for `trigger.schedule`: the house time zone, the armed ticks and the
//! no-catch-up re-arm rule. `nidoo-core` only computes ticks; every clock and zone lives here.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use nidoo_core::flow::Schedule;

use crate::config::FlowsConfig;

/// A schedule node armed for its next tick.
pub(super) struct Armed {
    pub node: String,
    /// The tick the node fires at next.
    pub at: DateTime<Tz>,
    pub schedule: Schedule,
    /// The expression `schedule` was parsed from: a changed one means a re-arm from now.
    pub cron: String,
}

/// Where the engine reads wall-clock time; tests inject their own.
pub type Now = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The system clock, kept in step with tokio's timer: the wall clock at the call plus the
/// tokio time elapsed since. Under real time this is the system clock; under a paused test
/// runtime the clock advances with `tokio::time::advance`, so schedules can be tested.
pub fn timer_clock() -> Now {
    let base = Utc::now();
    let started = tokio::time::Instant::now();
    Arc::new(move || {
        let elapsed = tokio::time::Instant::now().saturating_duration_since(started);
        base + chrono::Duration::from_std(elapsed).unwrap_or_default()
    })
}

/// The current time in `zone`, from `now`.
pub fn now_in(now: &Now, zone: Tz) -> DateTime<Tz> {
    now().with_timezone(&zone)
}

/// The zone schedules run in: `[flows] timezone`, else the system's, else UTC. An unusable
/// name is a warning, not an error: the hub must still start.
pub fn house_zone(cfg: &FlowsConfig) -> Tz {
    let name = match &cfg.timezone {
        Some(name) => name.clone(),
        None => match iana_time_zone::get_timezone() {
            Ok(name) => name,
            Err(err) => {
                tracing::warn!(error = %err, "cannot read the system time zone, using UTC");
                return Tz::UTC;
            }
        },
    };
    name.parse().unwrap_or_else(|_| {
        tracing::warn!(timezone = %name, "unknown time zone, using UTC");
        Tz::UTC
    })
}

/// The tick after one that fired (or was due) at `previous`, seen at `now`. Starting from the
/// later of the two is what forbids catch-up: ticks missed while the hub was busy or down are
/// never replayed, only the next future one is armed.
pub(crate) fn rearm(
    schedule: &Schedule,
    previous: DateTime<Tz>,
    now: DateTime<Tz>,
) -> Option<DateTime<Tz>> {
    schedule.next_after(&previous.max(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn paris(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Tz> {
        chrono_tz::Europe::Paris
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("unambiguous local time")
    }

    #[test]
    fn no_catch_up_after_a_long_gap() {
        let schedule = Schedule::parse("*/10 * * * *").unwrap();
        let previous = Tz::UTC.with_ymd_and_hms(2026, 10, 20, 18, 0, 0).unwrap();
        let now = Tz::UTC.with_ymd_and_hms(2026, 10, 20, 21, 3, 0).unwrap();
        assert_eq!(
            rearm(&schedule, previous, now),
            Some(Tz::UTC.with_ymd_and_hms(2026, 10, 20, 21, 10, 0).unwrap())
        );
        // On time: the tick after the one that just fired.
        assert_eq!(
            rearm(&schedule, previous, previous),
            Some(Tz::UTC.with_ymd_and_hms(2026, 10, 20, 18, 10, 0).unwrap())
        );
    }

    #[test]
    fn next_ticks_across_dst() {
        // Spring forward: on 2027-03-28 Paris jumps 02:00 -> 03:00, so 02:00 does not exist.
        let daily = Schedule::parse("0 2 * * *").unwrap();
        let mut at = paris(2027, 3, 27, 12, 0);
        let mut seen = Vec::new();
        for _ in 0..4 {
            at = rearm(&daily, at, at).expect("next tick");
            seen.push(at.to_rfc3339());
        }
        println!("daily 02:00: {seen:?}");
        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(unique, seen, "no instant twice");
        assert_eq!(seen, PINNED_DAILY, "{seen:?}");

        // Autumn fall back: 2026-10-25 03:00 -> 02:00 Paris (01:00Z). Every 10 minutes stays
        // 600 s apart in UTC, without a duplicate in the repeated hour.
        let every = Schedule::parse("*/10 * * * *").unwrap();
        let end = Tz::UTC
            .with_ymd_and_hms(2026, 10, 25, 2, 0, 0)
            .unwrap()
            .with_timezone(&chrono_tz::Europe::Paris);
        let mut at = Tz::UTC
            .with_ymd_and_hms(2026, 10, 25, 0, 0, 0)
            .unwrap()
            .with_timezone(&chrono_tz::Europe::Paris);
        let mut ticks = Vec::new();
        while at < end {
            at = rearm(&every, at, at).expect("next tick");
            ticks.push(at.timestamp());
        }
        assert!(ticks.len() >= 12, "{ticks:?}");
        for pair in ticks.windows(2) {
            assert_eq!(pair[1] - pair[0], 600, "{ticks:?}");
        }
    }

    /// croner's sequence, checked by hand: the skipped 02:00 runs once at 03:00 (a time that
    /// exists, once) and the days after are back to 02:00.
    const PINNED_DAILY: [&str; 4] = [
        "2027-03-28T03:00:00+02:00",
        "2027-03-29T02:00:00+02:00",
        "2027-03-30T02:00:00+02:00",
        "2027-03-31T02:00:00+02:00",
    ];
}
