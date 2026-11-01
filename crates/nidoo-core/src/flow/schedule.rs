//! Cron schedules for `trigger.schedule`: five fields (minute hour day-of-month month
//! day-of-week), no seconds and no year. Pure computation over `chrono` values; the hub owns the
//! clock and the timers.

use chrono::{DateTime, TimeZone};
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
}

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
