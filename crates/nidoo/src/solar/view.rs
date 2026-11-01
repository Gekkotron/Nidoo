//! The solar forecast view the hub serves, built from a merged forecast.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use nidoo_api::{
    SolarDay, SolarHistory, SolarNow, SolarPeak, SolarPeriod, SolarSite, SolarSpan, SolarToday,
    SolarView,
};
use nidoo_solcast::{
    Forecast, Ledger, best_hours, day_totals, local_midnight, peak, power_at, remaining,
};

/// Number of days after today the view lists.
const DAYS_AHEAD: usize = 7;

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn secs(t: DateTime<Utc>) -> u64 {
    u64::try_from(t.timestamp()).unwrap_or(0)
}

fn date_string(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn local_date(t: DateTime<Utc>, zone: Tz) -> NaiveDate {
    zone.from_utc_datetime(&t.naive_utc()).date_naive()
}

/// The view at `now`: today's figures, the next seven forecast days, each site's share of today
/// and the periods of today and tomorrow. `history` is left empty for the caller to fill.
pub fn build_view(
    forecast: &Forecast,
    ledger: &Ledger,
    zone: Tz,
    now: DateTime<Utc>,
    next_refresh_at: Option<u64>,
    daily_limit: u32,
    stale: bool,
) -> SolarView {
    let today = local_date(now, zone);
    let tomorrow = today.succ_opt().unwrap_or(today);
    let totals = day_totals(&forecast.periods, zone);
    let today_total = totals.iter().find(|d| d.date == today);

    // Where each day's periods stop: a day that ends before its local end is only partly
    // forecast and is not offered as a day figure.
    let mut last_end: BTreeMap<NaiveDate, DateTime<Utc>> = BTreeMap::new();
    for p in &forecast.periods {
        let end = last_end.entry(local_date(p.start(), zone)).or_insert(p.end);
        *end = (*end).max(p.end);
    }
    let complete = |date: NaiveDate| {
        let next = date.succ_opt().map(|next| local_midnight(next, zone));
        match (last_end.get(&date), next) {
            (Some(end), Some(next)) => *end >= next,
            _ => false,
        }
    };

    let days = totals
        .iter()
        .filter(|d| d.date > today && complete(d.date))
        .take(DAYS_AHEAD)
        .map(|d| SolarDay {
            date: date_string(d.date),
            estimate: round2(d.estimate),
            low: round2(d.low),
            high: round2(d.high),
        })
        .collect();

    let sites = forecast
        .sites
        .iter()
        .enumerate()
        .map(|(index, periods)| SolarSite {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            today: day_totals(periods, zone)
                .iter()
                .find(|d| d.date == today)
                .map(|d| round2(d.estimate)),
        })
        .collect();

    let periods = forecast
        .periods
        .iter()
        .filter(|p| {
            let date = local_date(p.start(), zone);
            date == today || date == tomorrow
        })
        .map(|p| SolarPeriod {
            end: secs(p.end),
            minutes: p.minutes,
            estimate: round2(p.estimate),
            low: round2(p.low),
            high: round2(p.high),
        })
        .collect();

    SolarView {
        fetched_at: u64::try_from(forecast.fetched_at).unwrap_or(0),
        stale,
        next_refresh_at,
        calls_today: ledger.calls_on(now),
        daily_limit,
        now: SolarNow {
            power: power_at(&forecast.periods, now).map(round2),
            remaining_today: today_total.map(|_| round2(remaining(&forecast.periods, zone, now))),
        },
        today: SolarToday {
            date: date_string(today),
            estimate: today_total.map(|d| round2(d.estimate)),
            low: today_total.map(|d| round2(d.low)),
            high: today_total.map(|d| round2(d.high)),
            peak: peak(&forecast.periods, today, zone).map(|(power, at)| SolarPeak {
                power: round2(power),
                at: secs(at),
            }),
            best_hours: best_hours(&forecast.periods, today, zone).map(|(from, to)| SolarSpan {
                from: secs(from),
                to: secs(to),
            }),
        },
        days,
        sites,
        periods,
        history: SolarHistory::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use nidoo_solcast::{Period, merge, parse_forecasts};

    const PARIS: Tz = chrono_tz::Europe::Paris;

    fn fixture() -> Forecast {
        let periods = parse_forecasts(include_str!(
            "../../../nidoo-solcast/tests/fixtures/forecasts.json"
        ))
        .unwrap();
        merge(vec![periods], 1_000)
    }

    fn noon() -> DateTime<Utc> {
        "2026-10-23T10:00:00Z".parse().unwrap()
    }

    fn view_at(f: &Forecast, now: DateTime<Utc>) -> SolarView {
        build_view(f, &Ledger::default(), PARIS, now, Some(5), 10, false)
    }

    #[test]
    fn view_from_fixture_at_noon_paris() {
        let f = fixture();
        let v = view_at(&f, noon());
        assert_eq!(v.today.date, "2026-10-23");
        assert_eq!(v.today.estimate, Some(12.4));
        let peak = v.today.peak.unwrap();
        assert_eq!(peak.power, 2.4);
        assert_eq!(peak.at, secs("2026-10-23T11:15:00Z".parse().unwrap()));
        assert_eq!(
            v.now.remaining_today,
            Some(round2(remaining(&f.periods, PARIS, noon())))
        );
        assert_eq!(v.sites.len(), 1);
        assert_eq!(v.sites[0].today, Some(12.4));
        assert_eq!(v.next_refresh_at, Some(5));
        assert!(v.history.forecasts.is_empty());
    }

    fn week(days: i64) -> Forecast {
        let periods = (0..days * 48)
            .map(|i| Period {
                end: "2026-10-23T00:30:00Z".parse::<DateTime<Utc>>().unwrap()
                    + Duration::minutes(30 * i),
                minutes: 30,
                estimate: 1.0,
                low: 0.5,
                high: 1.5,
            })
            .collect();
        merge(vec![periods], 0)
    }

    #[test]
    fn view_days_start_tomorrow_max_seven() {
        let v = view_at(&week(12), noon());
        assert_eq!(v.days.len(), 7);
        assert_eq!(v.days[0].date, "2026-10-24");
        assert_eq!(v.days[6].date, "2026-10-30");
    }

    #[test]
    fn view_days_leave_out_a_partial_last_day() {
        // week(5) stops at 2026-10-28T00:00Z, an hour into the local 28th.
        let v = view_at(&week(5), noon());
        let dates: Vec<_> = v.days.iter().map(|d| d.date.as_str()).collect();
        assert_eq!(
            dates,
            ["2026-10-24", "2026-10-25", "2026-10-26", "2026-10-27"]
        );
    }

    #[test]
    fn view_periods_cover_today_and_tomorrow_only() {
        let v = view_at(&week(5), noon());
        let first = v.periods.first().unwrap().end;
        let last = v.periods.last().unwrap().end;
        // Paris is UTC+2 on 2026-10-23: local midnight is 22:00Z the day before.
        assert_eq!(first, secs("2026-10-23T00:30:00Z".parse().unwrap()));
        assert_eq!(last, secs("2026-10-24T22:00:00Z".parse().unwrap()));
        assert_eq!(v.periods.len(), 92);
    }
}
