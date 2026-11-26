//! Cron schedules for `trigger.schedule`: five fields (minute hour day-of-month month
//! day-of-week), no seconds and no year. Pure computation over `chrono` values; the hub owns the
//! clock and the timers.

use chrono::{DateTime, TimeDelta, TimeZone};
use croner::Cron;
use croner::parser::{CronParser, Seconds, Year};

/// Why a cron expression was rejected; the message is what the editor shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleError(pub String);

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScheduleError {}

/// A parsed five-field cron expression.
#[derive(Clone, Debug)]
pub struct Schedule {
    cron: Cron,
}

impl Schedule {
    /// Parses exactly five whitespace-separated fields; the seconds and year forms are errors.
    pub fn parse(expr: &str) -> Result<Schedule, ScheduleError> {
        let fields = expr.split_whitespace().count();
        if fields != 5 {
            return Err(ScheduleError(format!(
                "a cron expression has 5 fields (minute hour day month weekday), found {fields}"
            )));
        }
        CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build()
            .parse(expr)
            .map(|cron| Schedule { cron })
            .map_err(|e| ScheduleError(e.to_string()))
    }

    /// The first tick strictly after `after`, in `after`'s time zone.
    pub fn next_after<Tz: TimeZone>(&self, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
        self.cron.find_next_occurrence(after, false).ok()
    }

    /// The last tick `t` with `from <= t <= to`, or `None` when the window holds none. Walks
    /// forward from `from`, so a very dense schedule over a long window stops at
    /// [`MAX_WALK_TICKS`] ticks and returns the last one seen.
    pub fn latest_in<Tz: TimeZone>(
        &self,
        from: &DateTime<Tz>,
        to: &DateTime<Tz>,
    ) -> Option<DateTime<Tz>> {
        let mut cursor = from.clone() - TimeDelta::seconds(1);
        let mut latest = None;
        for _ in 0..MAX_WALK_TICKS {
            match self.next_after(&cursor) {
                Some(t) if t <= *to => {
                    cursor = t.clone();
                    latest = Some(t);
                }
                _ => break,
            }
        }
        latest
    }
}

/// The most ticks [`Schedule::latest_in`] walks, a guard for `* * * * *` over a long window.
pub const MAX_WALK_TICKS: usize = 10_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ten_minutes_next_after() {
        let s = Schedule::parse("*/10 * * * *").unwrap();
        let t = chrono::Utc
            .with_ymd_and_hms(2026, 10, 20, 18, 3, 0)
            .unwrap();
        assert_eq!(
            s.next_after(&t).unwrap(),
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 20, 18, 10, 0)
                .unwrap()
        );
        let on_tick = chrono::Utc
            .with_ymd_and_hms(2026, 10, 20, 18, 10, 0)
            .unwrap();
        assert_eq!(
            s.next_after(&on_tick).unwrap(),
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 20, 18, 20, 0)
                .unwrap()
        );
    }

    #[test]
    fn latest_in_picks_last_tick() {
        use chrono_tz::Europe::Paris;
        let s = Schedule::parse("0 * * * *").unwrap();
        let at = |h, m| Paris.with_ymd_and_hms(2026, 11, 25, h, m, 0).unwrap();
        assert_eq!(s.latest_in(&at(5, 10), &at(7, 30)), Some(at(7, 0)));
        // Both ends are inclusive.
        assert_eq!(s.latest_in(&at(6, 0), &at(7, 0)), Some(at(7, 0)));
        assert_eq!(s.latest_in(&at(5, 10), &at(5, 50)), None);
    }

    #[test]
    fn latest_in_includes_from() {
        use chrono_tz::Europe::Paris;
        let s = Schedule::parse("0 * * * *").unwrap();
        let at = |h, m| Paris.with_ymd_and_hms(2026, 11, 25, h, m, 0).unwrap();
        assert_eq!(s.latest_in(&at(7, 0), &at(7, 30)), Some(at(7, 0)));
    }

    #[test]
    fn rejects_non_five_field_and_garbage() {
        for bad in [
            "",
            "* * * *",
            "0 * * * * *",
            "0 0 * * * * 2026",
            "61 * * * *",
            "every ten",
        ] {
            assert!(Schedule::parse(bad).is_err(), "{bad}");
        }
    }
}
