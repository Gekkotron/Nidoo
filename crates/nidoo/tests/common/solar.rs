//! A Solcast forecast that covers today, for the hub and poller tests.
#![allow(dead_code)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use nidoo_solcast::{Forecast, merge, parse_forecasts};

/// The Task 1 fixture: 48 half-hour periods of 2026-10-23, Europe/Paris.
const FIXTURE: &str = include_str!("../../../nidoo-solcast/tests/fixtures/forecasts.json");

/// Local midnight (Europe/Paris) that starts `day`, in UTC.
fn paris_midnight(day: chrono::NaiveDate) -> DateTime<Utc> {
    Paris
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .expect("midnight exists")
        .with_timezone(&Utc)
}

/// The fixture shifted so its first period starts at local midnight (Europe/Paris) of the day
/// of `at`, merged into a one-site forecast fetched at `at`. On a 25-hour day (the autumn clock
/// change) the 48 periods stop at 23:00, so empty night periods fill the day up to midnight.
pub fn forecast_covering(at: DateTime<Utc>) -> Forecast {
    let mut periods = parse_forecasts(FIXTURE).expect("fixture parses");
    let first_start = periods[0].start();
    let today = at.with_timezone(&Paris).date_naive();
    let midnight = paris_midnight(today);
    let shift: Duration = midnight - first_start;
    for period in &mut periods {
        period.end += shift;
    }
    let next_midnight = paris_midnight(today.succ_opt().expect("a next day"));
    while let Some(last) = periods.last().filter(|p| p.end < next_midnight) {
        let mut night = last.clone();
        night.end += Duration::minutes(i64::from(night.minutes));
        night.estimate = 0.0;
        night.low = 0.0;
        night.high = 0.0;
        periods.push(night);
    }
    merge(vec![periods], at.timestamp())
}

/// [`forecast_covering`] now.
pub fn forecast_today() -> Forecast {
    forecast_covering(Utc::now())
}
