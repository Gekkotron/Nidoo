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
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/forecasts.json");
    const PARIS: Tz = chrono_tz::Europe::Paris;

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn fixture() -> Vec<Period> {
        crate::parse_forecasts(FIXTURE).unwrap()
    }

    /// Consecutive 30-minute periods ending at `first_end`, `first_end + 30 min`, ...
    fn series(first_end: &str, estimates: &[f64]) -> Vec<Period> {
        let first = t(first_end);
        estimates
            .iter()
            .enumerate()
            .map(|(i, e)| Period {
                end: first + Duration::minutes(30 * i as i64),
                minutes: 30,
                estimate: *e,
                low: e * 0.5,
                high: e * 2.0,
            })
            .collect()
    }

    #[test]
    fn merge_sums_by_end_and_keeps_sites() {
        let a = series("2026-10-23T10:30:00Z", &[1.0, 2.0, 3.0]);
        let mut b = series("2026-10-23T10:30:00Z", &[0.5, 0.5, 0.5]);
        b.remove(1);
        let f = merge(vec![a.clone(), b.clone()], 42);
        assert_eq!(f.fetched_at, 42);
        assert_eq!(f.sites, vec![a, b]);
        let est: Vec<f64> = f.periods.iter().map(|p| p.estimate).collect();
        assert_eq!(est, vec![1.5, 2.0, 3.5]);
        assert_eq!(f.periods[0].low, 0.75);
        assert_eq!(f.periods[0].high, 3.0);
        assert!(f.periods.windows(2).all(|w| w[0].end < w[1].end));
    }

    #[test]
    fn fixture_day_total_is_12_4() {
        let days = day_totals(&fixture(), PARIS);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, d("2026-10-23"));
        assert!((days[0].estimate - 12.4).abs() < 1e-6);
        assert!((days[0].low - 0.7 * 12.4).abs() < 1e-6);
        // The fixture rounds each period's high value, so allow a small drift.
        assert!((days[0].high - 1.15 * 12.4).abs() < 1e-3);
    }

    #[test]
    fn period_straddling_local_midnight_counts_in_its_start_day() {
        // 22:00Z-22:30Z is 00:00-00:30 local (UTC+2) on the 24th: it starts exactly at midnight.
        let p = series("2026-10-23T22:30:00Z", &[2.0]);
        let days = day_totals(&p, PARIS);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, d("2026-10-24"));
        assert!((days[0].estimate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn dst_end_day_has_50_periods() {
        // 2026-10-25 local Paris is 25 h: 2026-10-24T22:00Z to 2026-10-25T23:00Z.
        let p = series("2026-10-24T22:30:00Z", &[1.0; 50]);
        let days = day_totals(&p, PARIS);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, d("2026-10-25"));
        assert!((days[0].estimate - 25.0).abs() < 1e-9);
    }

    #[test]
    fn remaining_prorates_current_period() {
        // Paris 2026-10-23 is UTC+2; the local day ends 2026-10-23T22:00Z.
        // Periods cover 20:00-22:00Z; now is 15 min into the first (2 kW x 0.25 h = 0.5 kWh left).
        let p = series("2026-10-23T20:30:00Z", &[2.0, 2.0, 2.0, 2.0]);
        let now = t("2026-10-23T20:15:00Z");
        let expected = 2.0 * 0.25 + 3.0 * 1.0;
        assert!((remaining(&p, PARIS, now) - expected).abs() < 1e-9);
        // A period after local midnight is not counted.
        let q = series("2026-10-23T22:30:00Z", &[2.0]);
        assert_eq!(remaining(&q, PARIS, now), 0.0);
    }

    #[test]
    fn power_at_and_none_outside() {
        let p = fixture();
        assert_eq!(power_at(&p, t("2026-10-23T11:10:00Z")), Some(2.4));
        // Start is inclusive, end exclusive.
        assert_eq!(power_at(&p, t("2026-10-22T22:00:00Z")), Some(p[0].estimate));
        assert_eq!(power_at(&p, t("2026-10-22T21:59:59Z")), None);
        assert_eq!(power_at(&p, t("2026-10-23T22:00:00Z")), None);
    }

    #[test]
    fn peak_is_midpoint_of_max_period() {
        let (kw, at) = peak(&fixture(), d("2026-10-23"), PARIS).unwrap();
        assert_eq!(kw, 2.4);
        assert_eq!(at, t("2026-10-23T11:15:00Z"));
        assert_eq!(peak(&fixture(), d("2026-10-24"), PARIS), None);
        let zero = series("2026-10-23T10:30:00Z", &[0.0, 0.0]);
        assert_eq!(peak(&zero, d("2026-10-23"), PARIS), None);
    }

    #[test]
    fn peak_ties_pick_earliest() {
        let p = series("2026-10-23T10:30:00Z", &[1.0, 3.0, 3.0]);
        let (_, at) = peak(&p, d("2026-10-23"), PARIS).unwrap();
        assert_eq!(at, t("2026-10-23T10:45:00Z"));
    }

    #[test]
    fn best_hours_longest_run_over_60_percent() {
        // Peak 10, threshold 6. Runs: [8, 10] (2 periods) and [7, 9, 6.5, 6.2] (4 periods).
        let p = series(
            "2026-10-23T08:30:00Z",
            &[1.0, 8.0, 10.0, 2.0, 7.0, 9.0, 6.5, 6.2, 1.0],
        );
        let got = best_hours(&p, d("2026-10-23"), PARIS).unwrap();
        assert_eq!(got, (t("2026-10-23T10:00:00Z"), t("2026-10-23T12:00:00Z")));
        let zero = series("2026-10-23T10:30:00Z", &[0.0; 4]);
        assert_eq!(best_hours(&zero, d("2026-10-23"), PARIS), None);
    }

    #[test]
    fn best_hours_ties_pick_earliest_and_gaps_break_runs() {
        let p = series("2026-10-23T08:30:00Z", &[10.0, 10.0, 1.0, 10.0, 10.0]);
        let got = best_hours(&p, d("2026-10-23"), PARIS).unwrap();
        assert_eq!(got, (t("2026-10-23T08:00:00Z"), t("2026-10-23T09:00:00Z")));
        // A missing period splits an otherwise high run: 2 + 1 periods, the first wins.
        let mut q = series("2026-10-23T08:30:00Z", &[10.0, 10.0, 10.0, 10.0]);
        q.remove(2);
        let got = best_hours(&q, d("2026-10-23"), PARIS).unwrap();
        assert_eq!(got, (t("2026-10-23T08:00:00Z"), t("2026-10-23T09:00:00Z")));
    }

    #[test]
    fn trim_drops_outside_window_everywhere() {
        let a = series("2026-10-23T10:30:00Z", &[1.0, 1.0, 1.0, 1.0]);
        let b = series("2026-10-23T10:30:00Z", &[2.0, 2.0, 2.0, 2.0]);
        let mut f = merge(vec![a, b], 0);
        // Keep 10:30-11:30 only: the periods ending 11:00 and 11:30.
        f.trim(t("2026-10-23T10:30:00Z"), t("2026-10-23T11:30:00Z"));
        let ends: Vec<_> = f.periods.iter().map(|p| p.end).collect();
        assert_eq!(
            ends,
            vec![t("2026-10-23T11:00:00Z"), t("2026-10-23T11:30:00Z")]
        );
        for site in &f.sites {
            assert_eq!(site.len(), 2);
            assert_eq!(site[0].end, t("2026-10-23T11:00:00Z"));
        }
    }

    fn paris_midnight() -> DateTime<Utc> {
        t("2026-10-22T22:00:00Z")
    }

    /// 48 periods of the fixture day, estimate 1.0 each.
    fn full_day() -> Vec<Period> {
        series("2026-10-22T22:30:00Z", &[1.0; 48])
    }

    #[test]
    fn keep_past_prepends_morning_from_previous() {
        let previous = merge(vec![full_day()], 1);
        // Noon local is 10:00Z; the new fetch starts there with doubled estimates.
        let mut new = merge(vec![series("2026-10-23T10:30:00Z", &[2.0; 24])], 2);
        new.keep_past(&previous, paris_midnight());
        assert_eq!(new.periods.len(), 48);
        assert_eq!(new.periods[0].end, t("2026-10-22T22:30:00Z"));
        assert!(new.periods.windows(2).all(|w| w[0].end < w[1].end));
        assert_eq!(new.periods[23].estimate, 1.0);
        assert_eq!(new.periods[24].estimate, 2.0);
        assert_eq!(new.sites[0].len(), 48);
        let total = day_totals(&new.periods, PARIS)[0].estimate;
        assert!((total - (24.0 * 0.5 + 24.0 * 1.0)).abs() < 1e-9, "{total}");
    }

    #[test]
    fn keep_past_ignores_periods_before_from() {
        let yesterday = series("2026-10-21T22:30:00Z", &[1.0; 48]);
        let previous = merge(vec![yesterday], 1);
        let mut new = merge(vec![series("2026-10-23T10:30:00Z", &[2.0; 4])], 2);
        new.keep_past(&previous, paris_midnight());
        assert_eq!(new.periods.len(), 4);
        assert_eq!(new.sites[0].len(), 4);
    }

    #[test]
    fn keep_past_per_site() {
        let a = series("2026-10-22T22:30:00Z", &[1.0; 48]);
        let b = series("2026-10-22T22:30:00Z", &[3.0; 48]);
        let previous = merge(vec![a, b], 1);
        let mut new = merge(
            vec![
                series("2026-10-23T10:30:00Z", &[2.0; 4]),
                // Site B's fetch starts later than A's.
                series("2026-10-23T11:30:00Z", &[6.0; 2]),
            ],
            2,
        );
        new.keep_past(&previous, paris_midnight());
        assert_eq!(new.sites[0].len(), 24 + 4);
        assert_eq!(new.sites[1].len(), 26 + 2);
        assert_eq!(new.sites[1][25].estimate, 3.0);
        assert_eq!(new.sites[1][26].estimate, 6.0);
        // The merged series is the sum of the sites, kept past included: it starts at the
        // earliest site start and covers site B's 26 kept periods.
        assert_eq!(new.periods, merge(new.sites.clone(), 2).periods);
        assert_eq!(new.periods.len(), 24 + 4);
        assert!(new.periods.windows(2).all(|w| w[0].end < w[1].end));
        // Site B's kept past (3.0) adds to site A's fresh periods where they overlap.
        assert!(new.periods.iter().any(|p| (p.estimate - 5.0).abs() < 1e-9));
    }

    #[test]
    fn keep_past_skips_sites_missing_from_previous() {
        let previous = merge(vec![full_day()], 1);
        let mut new = merge(
            vec![
                series("2026-10-23T10:30:00Z", &[2.0; 2]),
                series("2026-10-23T10:30:00Z", &[2.0; 2]),
            ],
            2,
        );
        new.keep_past(&previous, paris_midnight());
        assert_eq!(new.sites[0].len(), 26);
        assert_eq!(new.sites[1].len(), 2);
    }

    #[test]
    fn keep_past_with_empty_new_keeps_nothing() {
        let previous = merge(vec![full_day()], 1);
        let mut new = merge(vec![vec![]], 2);
        new.keep_past(&previous, paris_midnight());
        assert!(new.periods.is_empty());
        assert!(new.sites[0].is_empty());
    }
}
