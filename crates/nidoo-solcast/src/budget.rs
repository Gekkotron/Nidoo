//! The daily call budget: a persistent ledger, the refresh slots of a day and the decision of
//! whether to call Solcast now. Calls are counted per UTC day and every attempt counts.

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// Calls kept back from the refresh plan for the account's other users and for retries.
pub const RESERVE: u32 = 2;

/// Calls made during one UTC day, persisted across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    /// The UTC day `calls` belongs to.
    pub utc_day: Option<NaiveDate>,
    /// Requests attempted on `utc_day`.
    pub calls: u32,
    /// Unix seconds of the last attempt.
    pub last_attempt: Option<i64>,
    /// Unix seconds before which no call may be made (quota exceeded).
    pub blocked_until: Option<i64>,
}

impl Ledger {
    /// Combines two ledgers without ever losing a recorded call: the later UTC day wins (the
    /// larger count when both are the same day), and the later attempt and block are kept.
    pub fn merge(&self, other: &Ledger) -> Ledger {
        let (utc_day, calls) = match self.utc_day.cmp(&other.utc_day) {
            std::cmp::Ordering::Equal => (self.utc_day, self.calls.max(other.calls)),
            std::cmp::Ordering::Greater => (self.utc_day, self.calls),
            std::cmp::Ordering::Less => (other.utc_day, other.calls),
        };
        Ledger {
            utc_day,
            calls,
            last_attempt: self.last_attempt.max(other.last_attempt),
            blocked_until: self.blocked_until.max(other.blocked_until),
        }
    }

    /// Calls counted on the UTC day of `now` (zero when the ledger is about another day).
    pub fn calls_on(&self, now: DateTime<Utc>) -> u32 {
        if self.utc_day == Some(now.date_naive()) {
            self.calls
        } else {
            0
        }
    }

    /// Counts one attempted request at `now`, starting a fresh count on a new UTC day.
    pub fn record_attempt(&mut self, now: DateTime<Utc>) {
        let day = now.date_naive();
        if self.utc_day != Some(day) {
            self.utc_day = Some(day);
            self.calls = 0;
        }
        self.calls += 1;
        self.last_attempt = Some(now.timestamp());
    }

    /// Marks the day's budget as spent (HTTP 429): no call until the next UTC midnight + 5 min.
    pub fn record_quota_exceeded(&mut self, now: DateTime<Utc>, daily_limit: u32) {
        self.utc_day = Some(now.date_naive());
        self.calls = daily_limit;
        self.blocked_until = Some(next_reset(now).timestamp());
    }
}

/// Next UTC midnight plus five minutes.
fn next_reset(now: DateTime<Utc>) -> DateTime<Utc> {
    let tomorrow = now.date_naive() + Duration::days(1);
    tomorrow.and_time(NaiveTime::MIN).and_utc() + Duration::minutes(5)
}

/// Refreshes planned per day: the limit minus [`RESERVE`], shared between the sites, at least 1.
pub fn refreshes_per_day(daily_limit: u32, sites: u32) -> u32 {
    (daily_limit.saturating_sub(RESERVE) / sites.max(1)).max(1)
}

/// `hour`:00 on `date` in `zone`, as UTC.
fn local_hour(date: NaiveDate, zone: Tz, hour: u32) -> DateTime<Utc> {
    let naive = date.and_hms_opt(hour, 0, 0).expect("valid hour");
    zone.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            zone.from_local_datetime(&(naive + Duration::hours(1)))
                .earliest()
        })
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| naive.and_utc())
}

/// Refresh instants of one local day: the first 30 min before sunrise, the rest spread evenly
/// up to 60 min before sunset. Missing sun times, or a sunset not after the sunrise, fall back
/// to 07:00 / 19:00 local.
pub fn slots(
    sunrise: Option<DateTime<Utc>>,
    sunset: Option<DateTime<Utc>>,
    date: NaiveDate,
    zone: Tz,
    refreshes: u32,
) -> Vec<DateTime<Utc>> {
    let mut rise = sunrise.unwrap_or_else(|| local_hour(date, zone, 7));
    let mut set = sunset.unwrap_or_else(|| local_hour(date, zone, 19));
    if set <= rise {
        // Inconsistent sun times (a sunset that belongs to another day): use the fallback.
        rise = local_hour(date, zone, 7);
        set = local_hour(date, zone, 19);
    }
    let first = rise - Duration::minutes(30);
    if refreshes <= 1 {
        return vec![first];
    }
    let last = set - Duration::minutes(60);
    let span = (last - rise).num_seconds();
    let mut out = vec![first];
    for i in 1..refreshes {
        let offset = span * i64::from(i) / i64::from(refreshes - 1);
        let slot = rise + Duration::seconds(offset);
        if slot > first && slot <= last {
            out.push(slot);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// What the refresher should do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Make a request now.
    CallNow,
    /// Do nothing until this instant, then decide again.
    SleepUntil(DateTime<Utc>),
}

/// Decides whether to call now. Never allows a call that could push the UTC day over
/// `daily_limit` (a refresh costs `sites` requests), whatever else is true. Only the very
/// first call (no attempt ever recorded) is made without a due slot; after a failed first
/// fetch the next planned slot is the retry, never an immediate one.
pub fn decide(
    now: DateTime<Utc>,
    ledger: &Ledger,
    has_forecast: bool,
    slots: &[DateTime<Utc>],
    daily_limit: u32,
    sites: u32,
) -> Decision {
    if let Some(until) = ledger.blocked_until
        && until > now.timestamp()
    {
        // An unrepresentable instant fails closed: wait for the next reset.
        return Decision::SleepUntil(
            DateTime::from_timestamp(until, 0).unwrap_or_else(|| next_reset(now)),
        );
    }
    if ledger.calls_on(now).saturating_add(sites) > daily_limit {
        return Decision::SleepUntil(next_reset(now));
    }
    if !has_forecast && ledger.last_attempt.is_none() {
        return Decision::CallNow;
    }
    let due = slots
        .iter()
        .any(|s| *s <= now && ledger.last_attempt.is_none_or(|l| s.timestamp() > l));
    if due {
        return Decision::CallNow;
    }
    match slots.iter().filter(|s| **s > now).min() {
        Some(next) => Decision::SleepUntil(*next),
        None => Decision::SleepUntil(now + Duration::hours(1)),
    }
}

#[cfg(test)]
mod tests;
