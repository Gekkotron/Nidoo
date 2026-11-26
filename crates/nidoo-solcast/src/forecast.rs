//! Merging per-site forecasts and the figures derived from them (day totals, peak, best hours).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::period::Period;

/// A merged forecast: the sum of all sites plus each site's own periods.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    /// When the forecast was fetched, in unix seconds.
    pub fetched_at: i64,
    /// Periods summed over all sites, sorted by `end`.
    pub periods: Vec<Period>,
    /// Each site's own periods, in the order the sites were given.
    pub sites: Vec<Vec<Period>>,
}

/// Sum the sites period by period (matched on `end`; a site missing a period counts as 0 there).
pub fn merge(sites: Vec<Vec<Period>>, fetched_at: i64) -> Forecast {
    Forecast {
        fetched_at,
        periods: sum_sites(&sites),
        sites,
    }
}

/// The sites summed period by period, sorted by `end`.
fn sum_sites(sites: &[Vec<Period>]) -> Vec<Period> {
    let mut sum: BTreeMap<DateTime<Utc>, Period> = BTreeMap::new();
    for p in sites.iter().flatten() {
        sum.entry(p.end)
            .and_modify(|acc| {
                acc.estimate += p.estimate;
                acc.low += p.low;
                acc.high += p.high;
            })
            .or_insert_with(|| p.clone());
    }
    sum.into_values().collect()
}

impl Forecast {
    /// Keep only the periods that start at or after `from` and end at or before `until`,
    /// in the merged series and in every site.
    pub fn trim(&mut self, from: DateTime<Utc>, until: DateTime<Utc>) {
        let keep = |p: &Period| p.start() >= from && p.end <= until;
        self.periods.retain(keep);
        for site in &mut self.sites {
            site.retain(keep);
        }
    }

    /// Prepend the periods of `previous` that a fresh fetch no longer returns: every period
    /// with `start() >= from` that ends at or before the first period of `self`. Done on the
    /// merged series and per site (a site missing from `previous` keeps nothing; a site with
    /// no period now keeps nothing). The merged series is then rebuilt from the sites, so it
    /// always equals their sum; every series stays sorted by `end` without duplicates.
    pub fn keep_past(&mut self, previous: &Forecast, from: DateTime<Utc>) {
        for (site, before) in self.sites.iter_mut().zip(&previous.sites) {
            prepend_past(site, before, from);
        }
        self.periods = sum_sites(&self.sites);
    }
}

/// Puts the periods of `before` that start at or after `from` and end by the first start of
/// `now` in front of `now`.
fn prepend_past(now: &mut Vec<Period>, before: &[Period], from: DateTime<Utc>) {
    let Some(first_start) = now.first().map(Period::start) else {
        return;
    };
    let mut kept: Vec<Period> = before
        .iter()
        .filter(|p| p.start() >= from && p.end <= first_start)
        .cloned()
        .collect();
    kept.sort_by_key(|p| p.end);
    kept.dedup_by_key(|p| p.end);
    kept.append(now);
    *now = kept;
}

/// Energy forecast for one local calendar day, in kWh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DayTotal {
    /// Local date.
    pub date: NaiveDate,
    /// Central estimate.
    pub estimate: f64,
    /// Pessimistic estimate.
    pub low: f64,
    /// Optimistic estimate.
    pub high: f64,
}

/// Local calendar date of an instant.
fn local_date(t: DateTime<Utc>, zone: Tz) -> NaiveDate {
    zone.from_utc_datetime(&t.naive_utc()).date_naive()
}

/// kWh of a mean-kW value over a period.
fn kwh(kw: f64, p: &Period) -> f64 {
    kw * f64::from(p.minutes) / 60.0
}

/// kWh per local date of each period's start, ascending.
pub fn day_totals(periods: &[Period], zone: Tz) -> Vec<DayTotal> {
    let mut days: BTreeMap<NaiveDate, DayTotal> = BTreeMap::new();
    for p in periods {
        let date = local_date(p.start(), zone);
        let day = days.entry(date).or_insert(DayTotal {
            date,
            estimate: 0.0,
            low: 0.0,
            high: 0.0,
        });
        day.estimate += kwh(p.estimate, p);
        day.low += kwh(p.low, p);
        day.high += kwh(p.high, p);
    }
    days.into_values().collect()
}

/// First instant of a local date (the first valid hour if midnight does not exist).
pub fn local_midnight(date: NaiveDate, zone: Tz) -> DateTime<Utc> {
    (0..4)
        .find_map(|h| {
            let naive = date.and_hms_opt(h, 0, 0)?;
            zone.from_local_datetime(&naive).earliest()
        })
        .map_or_else(
            || DateTime::from_naive_utc_and_offset(date.and_hms_opt(0, 0, 0).unwrap(), Utc),
            |t| t.with_timezone(&Utc),
        )
}

/// kWh from `now` to the end of now's local day; the period containing `now` is prorated.
pub fn remaining(periods: &[Period], zone: Tz, now: DateTime<Utc>) -> f64 {
    let day_end = local_date(now, zone)
        .succ_opt()
        .map_or(DateTime::<Utc>::MAX_UTC, |next| local_midnight(next, zone));
    periods
        .iter()
        .map(|p| {
            let from = p.start().max(now);
            let to = p.end.min(day_end);
            if to > from {
                p.estimate * (to - from).num_seconds() as f64 / 3600.0
            } else {
                0.0
            }
        })
        .sum()
}

/// Estimate (kW) of the period with `start() <= now < end`.
pub fn power_at(periods: &[Period], now: DateTime<Utc>) -> Option<f64> {
    periods
        .iter()
        .find(|p| p.start() <= now && now < p.end)
        .map(|p| p.estimate)
}

/// Periods whose start falls on the local `date`, in the order given.
fn on_date(periods: &[Period], date: NaiveDate, zone: Tz) -> impl Iterator<Item = &Period> {
    periods
        .iter()
        .filter(move |p| local_date(p.start(), zone) == date)
}

/// Maximum estimate (kW) of a local date and the midpoint of its period; earliest on ties.
/// `None` when the day has no period or the maximum is 0.
pub fn peak(periods: &[Period], date: NaiveDate, zone: Tz) -> Option<(f64, DateTime<Utc>)> {
    let mut best: Option<&Period> = None;
    for p in on_date(periods, date, zone) {
        let better = match best {
            None => true,
            Some(b) => {
                p.estimate > b.estimate || (p.estimate == b.estimate && p.start() < b.start())
            }
        };
        if better {
            best = Some(p);
        }
    }
    best.filter(|p| p.estimate > 0.0).map(|p| {
        (
            p.estimate,
            p.start() + Duration::minutes(i64::from(p.minutes) / 2),
        )
    })
}

/// Longest contiguous run of a local date's periods with `estimate >= 0.6 x peak`, as
/// (start of the first, end of the last); earliest on ties. `None` without a peak.
pub fn best_hours(
    periods: &[Period],
    date: NaiveDate,
    zone: Tz,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let (top, _) = peak(periods, date, zone)?;
    let threshold = 0.6 * top;
    let mut day: Vec<&Period> = on_date(periods, date, zone).collect();
    day.sort_by_key(|p| p.end);

    let mut best: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
    let mut run: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
    for p in day {
        if p.estimate >= threshold {
            run = match run {
                Some((start, end)) if end == p.start() => Some((start, p.end)),
                _ => Some((p.start(), p.end)),
            };
            if let Some((start, end)) = run {
                let longer = best.is_none_or(|(bs, be)| end - start > be - bs);
                if longer {
                    best = run;
                }
            }
        } else {
            run = None;
        }
    }
    best
}

#[cfg(test)]
mod tests;
