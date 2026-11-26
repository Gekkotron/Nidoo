//! House-local date windows for history charts: parsing the `from`/`to`/`upto` query
//! parameters, resolving them to instants, and the limits and bucket boundaries that follow.

use chrono::{Datelike, NaiveDate};
use chrono_tz::Tz;

use nidoo_api::DailyPer;
use nidoo_core::history::{HOUR, Micros};

use super::HistoryError;
use super::query::{local_date, local_midnight};

const FIELDS_MAX_DAYS: i64 = 400;
const HOURLY_MAX_DAYS: i64 = 2;
const DAILY_MAX_DAYS: i64 = 1_100;

/// A requested period: `from..=to` house-local dates, optionally cut `upto` seconds after the
/// window's start (to line up a partial period with a complete one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateWindow {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub upto: Option<i64>,
}

/// A window as instants: `start` inclusive, `end` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub start: Micros,
    pub end: Micros,
}

/// What a window is asked to serve, which sets how long it may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowUse {
    Series,
    Fields,
    Daily(DailyPer),
}

fn bad(msg: impl Into<String>) -> HistoryError {
    HistoryError::BadRequest(msg.into())
}

fn date(name: &str, s: &str) -> Result<NaiveDate, HistoryError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad(format!("{name} must be YYYY-MM-DD")))
}

/// Parses the window parameters: `None` when neither `from` nor `to` is given.
pub fn parse_window(
    from: Option<&str>,
    to: Option<&str>,
    upto: Option<&str>,
) -> Result<Option<DateWindow>, HistoryError> {
    let (from, to) = match (from, to) {
        (None, None) => return Ok(None),
        (Some(f), Some(t)) => (date("from", f)?, date("to", t)?),
        _ => return Err(bad("from and to go together")),
    };
    if to < from {
        return Err(bad("to is before from"));
    }
    let upto = match upto {
        None => None,
        Some(s) => match s.parse::<i64>() {
            Ok(n) if n > 0 => Some(n),
            _ => return Err(bad("upto must be a positive number of seconds")),
        },
    };
    Ok(Some(DateWindow { from, to, upto }))
}

/// The window's instants, clipped at `upto` and at `now`. A `from` after today is rejected.
pub fn resolve(zone: Tz, w: &DateWindow, now: Micros) -> Result<Resolved, HistoryError> {
    if w.from > local_date(zone, now) {
        return Err(bad("from is in the future"));
    }
    let start = local_midnight(zone, w.from);
    let after_to = w.to.succ_opt().unwrap_or(w.to);
    let mut end = local_midnight(zone, after_to).min(now);
    if let Some(upto) = w.upto {
        end = end.min(start.saturating_add(upto.saturating_mul(1_000_000)));
    }
    Ok(Resolved { start, end })
}

/// Calendar days covered, both ends included.
pub fn days(w: &DateWindow) -> i64 {
    (w.to - w.from).num_days() + 1
}

/// Seconds per point of a series over `days` days.
pub fn step_secs(days: i64) -> i64 {
    match days {
        ..=1 => 600,
        2..=8 => 3_600,
        9..=32 => 18_000,
        33..=400 => 86_400,
        _ => 604_800,
    }
}

/// Rejects windows too long for what they are asked to serve.
pub fn check_limits(w: &DateWindow, what: WindowUse) -> Result<(), HistoryError> {
    let n = days(w);
    let (max, name) = match what {
        WindowUse::Series | WindowUse::Daily(DailyPer::Month) => return Ok(()),
        WindowUse::Fields => (FIELDS_MAX_DAYS, "fields"),
        WindowUse::Daily(DailyPer::Hour) => (HOURLY_MAX_DAYS, "hourly"),
        WindowUse::Daily(DailyPer::Day) => (DAILY_MAX_DAYS, "daily"),
    };
    if n > max {
        return Err(bad(format!("{name} window is limited to {max} days")));
    }
    Ok(())
}

/// Bucket starts for `per`, then `r.end` as the closing boundary. Starts at or after `r.end`
/// are dropped.
pub fn boundaries(zone: Tz, w: &DateWindow, r: &Resolved, per: DailyPer) -> Vec<Micros> {
    let mut out = Vec::new();
    match per {
        DailyPer::Hour => {
            let mut t = r.start;
            while t < r.end {
                out.push(t);
                t += HOUR;
            }
        }
        DailyPer::Day => {
            let mut day = w.from;
            loop {
                let t = local_midnight(zone, day);
                if t >= r.end {
                    break;
                }
                out.push(t);
                match day.succ_opt() {
                    Some(next) => day = next,
                    None => break,
                }
            }
        }
        DailyPer::Month => {
            out.push(r.start);
            let mut month = w.from.with_day(1).unwrap_or(w.from);
            loop {
                let (y, m) = (month.year(), month.month());
                month = if m == 12 {
                    NaiveDate::from_ymd_opt(y + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(y, m + 1, 1)
                }
                .unwrap_or(month);
                let t = local_midnight(zone, month);
                if t >= r.end || t <= r.start {
                    if t >= r.end {
                        break;
                    }
                    continue;
                }
                out.push(t);
            }
        }
    }
    out.push(r.end);
    out
}

#[cfg(test)]
mod tests;
