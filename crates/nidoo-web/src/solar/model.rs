//! Solar figures: when the card shows, which entities measure the production, and the sums and
//! resampling the card and page draw from. Pure, no UI.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use chrono_tz::Tz;
use nidoo_api::{HistorySeries, SolarDay, SolarForecastDay, SolarPeriod, SolarView};
use nidoo_core::energy::{EnergyEntity, EnergyMeters, EnergyRole, EnergyUnits, PowerUnits};

/// Age, in seconds, past which the solar card is hidden (24 h).
pub const SOLAR_MAX_AGE_S: u64 = 86_400;

/// Whether the solar card shows: there is a forecast and it is at most 24 h old.
pub fn solar_card_visible(s: Option<&SolarView>, now_s: u64) -> bool {
    s.is_some_and(|s| now_s.saturating_sub(s.fetched_at) <= SOLAR_MAX_AGE_S)
}

/// Unix seconds of local midnight of `date` and of the next day, in `zone`.
pub fn day_bounds(zone: Tz, date: NaiveDate) -> (u64, u64) {
    let midnight = |d: NaiveDate| {
        let naive = d.and_hms_opt(0, 0, 0).unwrap_or_default();
        // A zone whose midnight falls in a clock gap resumes at the first valid instant after it.
        let local = (0..=3)
            .find_map(|h| {
                zone.from_local_datetime(&(naive + Duration::hours(h)))
                    .earliest()
            })
            .map(|t| t.timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        local.max(0) as u64
    };
    (midnight(date), midnight(date + Duration::days(1)))
}

/// The local date of Unix second `t` in `zone`.
pub fn local_date(zone: Tz, t: u64) -> NaiveDate {
    zone.timestamp_opt(t as i64, 0)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

/// A solar power entity and how to read it in kW.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerSource {
    pub id: String,
    /// kW per unit of the entity's value (W is 0.001).
    pub kw_per_unit: f64,
    /// -1 when the sensor reports production as negative.
    pub sign: f64,
}

/// Where the measured production comes from: lifetime counters when there are any, else powers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SolarSources {
    /// Counter entity ids, sorted and unique.
    pub counters: Vec<String>,
    pub powers: Vec<PowerSource>,
}

/// The solar entities among `all`: energy-unit ones are counters; power-unit ones are power
/// sources, and the counter they pair with (if any) is a counter too.
pub fn solar_sources(all: &[EnergyEntity]) -> SolarSources {
    let mut out = SolarSources::default();
    for e in all.iter().filter(|e| e.role == Some(EnergyRole::Solar)) {
        if EnergyUnits::factor(e.unit.as_deref()).is_some() {
            out.counters.push(e.id.clone());
        } else if let Some(f) = PowerUnits::factor(e.unit.as_deref()) {
            out.powers.push(PowerSource {
                id: e.id.clone(),
                kw_per_unit: f / 1000.0,
                sign: if e.invert { -1.0 } else { 1.0 },
            });
            if let Some(c) = EnergyMeters::meter_for(e, all) {
                out.counters.push(c.id.clone());
            }
        }
    }
    out.counters.sort();
    out.counters.dedup();
    out
}

/// Sums `sign x kwh[id]` over `ids`; `None` when any is missing or unknown.
fn sum_known<'a>(
    ids: impl Iterator<Item = (&'a str, f64)>,
    kwh: &BTreeMap<String, Option<f64>>,
) -> Option<f64> {
    let mut total = 0.0;
    for (id, sign) in ids {
        total += sign * (*kwh.get(id)?)?;
    }
    Some(total)
}

/// kWh produced since local midnight: the counters' sum when there are counters, else the power
/// sources' integrals; `None` when nothing is known (never a made-up zero).
pub fn measured_today(
    src: &SolarSources,
    counter_kwh: &BTreeMap<String, Option<f64>>,
    power_kwh: &BTreeMap<String, Option<f64>>,
) -> Option<f64> {
    let total = if !src.counters.is_empty() {
        sum_known(src.counters.iter().map(|c| (c.as_str(), 1.0)), counter_kwh)?
    } else if !src.powers.is_empty() {
        sum_known(
            src.powers.iter().map(|p| (p.id.as_str(), p.sign)),
            power_kwh,
        )?
    } else {
        return None;
    };
    Some(total.max(0.0))
}

/// The production right now, kW: the live solar power sensors summed; `None` without one.
pub fn measured_power_kw(all: &[EnergyEntity]) -> Option<f64> {
    let mut total: Option<f64> = None;
    for e in all
        .iter()
        .filter(|e| e.role == Some(EnergyRole::Solar) && e.available)
    {
        let (Some(f), Some(v)) = (PowerUnits::factor(e.unit.as_deref()), e.value) else {
            continue;
        };
        let sign = if e.invert { -1.0 } else { 1.0 };
        *total.get_or_insert(0.0) += v * f / 1000.0 * sign;
    }
    total.map(|t| t.max(0.0))
}

/// The measured curve on the forecast's grid: per period, the mean of each source's points in
/// `[end - minutes, end)` in kW, summed over the sources that have points. Periods outside
/// `(from, until)` or with no point are absent. Returns `(period end, kW)`, sorted by end.
pub fn resample(
    series: &[(PowerSource, HistorySeries)],
    periods: &[SolarPeriod],
    from: u64,
    until: u64,
) -> Vec<(u64, f64)> {
    let mut out: Vec<(u64, f64)> = periods
        .iter()
        .filter_map(|p| {
            let dur = u64::from(p.minutes) * 60;
            let start = p.end.saturating_sub(dur);
            if p.end <= from || start >= until {
                return None;
            }
            let mut total: Option<f64> = None;
            for (src, s) in series {
                let (sum, n) = s
                    .points
                    .iter()
                    .filter(|pt| pt.0 >= start as i64 && pt.0 < p.end as i64)
                    .fold((0.0, 0u32), |(sum, n), pt| (sum + pt.1, n + 1));
                if n > 0 {
                    *total.get_or_insert(0.0) += sum / f64::from(n) * src.kw_per_unit * src.sign;
                }
            }
            Some((p.end, total?.max(0.0)))
        })
        .collect();
    out.sort_by_key(|(end, _)| *end);
    out
}

/// The periods that start in `[from, to)`.
pub fn periods_in(periods: &[SolarPeriod], from: u64, to: u64) -> Vec<SolarPeriod> {
    periods
        .iter()
        .filter(|p| {
            let start = p.end.saturating_sub(u64::from(p.minutes) * 60);
            start >= from && start < to
        })
        .cloned()
        .collect()
}

/// kWh the forecast expected up to `now`; a period containing `now` counts in proportion.
pub fn forecast_so_far(periods: &[SolarPeriod], now: u64) -> f64 {
    periods
        .iter()
        .map(|p| {
            let dur = u64::from(p.minutes) * 60;
            let start = p.end.saturating_sub(dur);
            let done = now.saturating_sub(start).min(dur);
            // estimate is mean kW over the period, so kWh = kW x hours elapsed.
            p.estimate * (done as f64 / 3600.0)
        })
        .sum()
}

/// The forecast day dated `today + 1`.
pub fn tomorrow(s: &SolarView, today: NaiveDate) -> Option<&SolarDay> {
    let next = today + Duration::days(1);
    s.days
        .iter()
        .find(|d| NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").ok() == Some(next))
}

/// Measured so far against forecast so far, as a signed whole percent. Hidden (`None`) when
/// nothing is measured or the forecast so far is under 0.1 kWh.
pub fn gap_percent(measured: Option<f64>, forecast_so_far: f64) -> Option<i32> {
    let m = measured?;
    (forecast_so_far >= 0.1)
        .then(|| ((m - forecast_so_far) / forecast_so_far * 100.0).round() as i32)
}

/// The kW above which the best hours are listed: 60 % of the day's peak.
pub fn best_hours_floor(peak_kw: f64) -> f64 {
    0.6 * peak_kw
}

/// kWh per house-local date from `/history/daily` rows (`t` is the day's start in unix seconds),
/// each value multiplied by `sign`. Rows with a negative `t` are skipped.
pub fn daily_by_date(zone: Tz, days: &[(i64, f64)], sign: f64) -> BTreeMap<NaiveDate, f64> {
    days.iter()
        .filter(|(t, _)| *t >= 0)
        .map(|(t, v)| (local_date(zone, *t as u64), v * sign))
        .collect()
}

/// Sum per date of several maps; a date missing from any map is dropped.
pub fn sum_daily(maps: &[BTreeMap<NaiveDate, f64>]) -> BTreeMap<NaiveDate, f64> {
    let Some((first, rest)) = maps.split_first() else {
        return BTreeMap::new();
    };
    first
        .iter()
        .filter_map(|(d, v)| {
            rest.iter()
                .try_fold(*v, |acc, m| m.get(d).map(|x| acc + x))
                .map(|total| (*d, total))
        })
        .collect()
}

/// Yesterday's tile: measured, the forecast made for that day, the gap and whether the forecast
/// was made late.
#[derive(Clone, Debug, PartialEq)]
pub struct Yesterday {
    pub measured: f64,
    pub forecast: Option<f64>,
    pub gap: Option<i32>,
    /// `made_at` of the forecast when it came more than an hour after sunrise.
    pub made_late_at: Option<u64>,
}

fn forecast_for(
    forecasts: &[SolarForecastDay],
    date: NaiveDate,
) -> impl Iterator<Item = &SolarForecastDay> {
    forecasts
        .iter()
        .filter(move |f| NaiveDate::parse_from_str(&f.date, "%Y-%m-%d").ok() == Some(date))
}

/// Yesterday's figures; `None` without a measured value for `date`. With several forecast rows
/// for the day, the earliest one is used.
pub fn yesterday(
    measured: &BTreeMap<NaiveDate, f64>,
    forecasts: &[SolarForecastDay],
    date: NaiveDate,
    sunrise_utc: Option<u64>,
) -> Option<Yesterday> {
    let m = *measured.get(&date)?;
    let row = forecast_for(forecasts, date).min_by_key(|f| f.made_at);
    let forecast = row.map(|f| f.estimate);
    Some(Yesterday {
        measured: m,
        forecast,
        gap: forecast.and_then(|f| gap_percent(Some(m), f)),
        made_late_at: row
            .zip(sunrise_utc)
            .filter(|(f, sr)| f.made_at > sr + 3600)
            .map(|(f, _)| f.made_at),
    })
}

/// Mean absolute error, in percent of the forecast, over the days with a morning forecast above
/// 0.5 kWh and a measured value. `None` under three such days.
pub fn accuracy(
    measured: &BTreeMap<NaiveDate, f64>,
    forecasts: &[SolarForecastDay],
    sunrise_of: impl Fn(NaiveDate) -> Option<u64>,
) -> Option<u32> {
    let errors: Vec<f64> = forecasts
        .iter()
        .filter_map(|f| {
            let date = NaiveDate::parse_from_str(&f.date, "%Y-%m-%d").ok()?;
            let m = *measured.get(&date)?;
            let morning = sunrise_of(date).is_none_or(|sr| f.made_at <= sr + 3600);
            (f.estimate > 0.5 && morning).then(|| (m - f.estimate).abs() / f.estimate * 100.0)
        })
        .collect();
    (errors.len() >= 3).then(|| (errors.iter().sum::<f64>() / errors.len() as f64).round() as u32)
}

/// Production since the 1st of `today`'s month: the measured days before `today` plus
/// `today_kwh`. `None` when both are empty.
pub fn month_so_far(
    measured: &BTreeMap<NaiveDate, f64>,
    today: NaiveDate,
    today_kwh: Option<f64>,
) -> Option<f64> {
    let first = today.with_day(1)?;
    let past: Vec<f64> = measured.range(first..today).map(|(_, v)| *v).collect();
    if past.is_empty() && today_kwh.is_none() {
        return None;
    }
    Some(past.iter().sum::<f64>() + today_kwh.unwrap_or(0.0))
}

#[cfg(test)]
mod tests;
