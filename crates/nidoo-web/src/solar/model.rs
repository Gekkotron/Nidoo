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
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;

    fn view(fetched_at: u64, days: &[&str]) -> SolarView {
        let days: Vec<_> = days
            .iter()
            .map(|d| serde_json::json!({"date": d, "estimate": 5.0, "low": 3.0, "high": 7.0}))
            .collect();
        serde_json::from_value(serde_json::json!({
            "fetched_at": fetched_at, "stale": false, "next_refresh_at": null,
            "calls_today": 1, "daily_limit": 10,
            "now": {"power": null, "remaining_today": null},
            "today": {"date": "2026-10-24", "estimate": null, "low": null, "high": null,
                      "peak": null, "best_hours": null},
            "days": days, "sites": [], "periods": []
        }))
        .expect("solar view")
    }

    fn ent(id: &str, unit: &str) -> EnergyEntity {
        EnergyEntity {
            id: id.to_string(),
            device_id: None,
            name: id.to_string(),
            label: None,
            unit: Some(unit.to_string()),
            value: None,
            available: true,
            role: None,
            invert: false,
            order: None,
            icon: None,
            meter: None,
        }
    }

    fn solar(id: &str, unit: &str) -> EnergyEntity {
        EnergyEntity {
            role: Some(EnergyRole::Solar),
            ..ent(id, unit)
        }
    }

    fn period(end: u64, minutes: u32, estimate: f64) -> SolarPeriod {
        SolarPeriod {
            end,
            minutes,
            estimate,
            low: 0.0,
            high: 0.0,
        }
    }

    fn src(id: &str, kw_per_unit: f64, sign: f64) -> PowerSource {
        PowerSource {
            id: id.to_string(),
            kw_per_unit,
            sign,
        }
    }

    fn series(points: &[(i64, f64)]) -> HistorySeries {
        HistorySeries {
            window: 300,
            points: points.iter().map(|&(t, v)| (t, v, v, v)).collect(),
        }
    }

    fn map(items: &[(&str, Option<f64>)]) -> BTreeMap<String, Option<f64>> {
        items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn card_hidden_when_missing_or_older_than_a_day() {
        let now = 1_000_000;
        assert!(!solar_card_visible(None, now));
        assert!(solar_card_visible(Some(&view(now - 86_400, &[])), now));
        assert!(!solar_card_visible(Some(&view(now - 86_401, &[])), now));
    }

    #[test]
    fn day_bounds_on_dst_change() {
        let (from, to) = day_bounds(Paris, date(2026, 10, 25));
        assert_eq!(to - from, 25 * 3600);
        let (from, to) = day_bounds(Paris, date(2026, 3, 29));
        assert_eq!(to - from, 23 * 3600);
        assert_eq!(local_date(Paris, from), date(2026, 3, 29));
        assert_eq!(local_date(Paris, from - 1), date(2026, 3, 28));
    }

    #[test]
    fn sources_pair_power_with_its_counter() {
        let on_device = |e: EnergyEntity, label: &str| EnergyEntity {
            device_id: Some("inv".into()),
            label: Some(label.into()),
            ..e
        };
        let all = vec![
            on_device(solar("p", "W"), "Power"),
            on_device(ent("e", "kWh"), "Energy"),
            ent("other", "kWh"),
        ];
        assert_eq!(
            solar_sources(&all),
            SolarSources {
                counters: vec!["e".into()],
                powers: vec![src("p", 0.001, 1.0)],
            }
        );
    }

    #[test]
    fn energy_unit_solar_entity_is_a_counter() {
        let s = solar_sources(&[solar("c", "Wh")]);
        assert_eq!(s.counters, vec!["c".to_string()]);
        assert!(s.powers.is_empty());
    }

    #[test]
    fn inverted_power_source_is_positive() {
        let all = [EnergyEntity {
            invert: true,
            value: Some(-800.0),
            ..solar("p", "W")
        }];
        let s = solar_sources(&all);
        assert_eq!(s.powers, vec![src("p", 0.001, -1.0)]);
        let kw = measured_power_kw(&all).unwrap();
        assert!((kw - 0.8).abs() < 1e-9);
        let periods = [period(1000 + 1800, 30, 1.0)];
        let r = resample(
            &[(s.powers[0].clone(), series(&[(1000, -1500.0)]))],
            &periods,
            0,
            10_000,
        );
        assert_eq!(r.len(), 1);
        assert!((r[0].1 - 1.5).abs() < 1e-9);
    }

    #[test]
    fn measured_today_prefers_counters() {
        let s = SolarSources {
            counters: vec!["e".into()],
            powers: vec![src("p", 0.001, 1.0)],
        };
        let v = measured_today(&s, &map(&[("e", Some(4.2))]), &map(&[("p", Some(9.0))]));
        assert_eq!(v, Some(4.2));
    }

    #[test]
    fn measured_today_none_when_a_counter_is_missing() {
        let s = SolarSources {
            counters: vec!["e".into()],
            powers: vec![],
        };
        assert_eq!(measured_today(&s, &map(&[("e", None)]), &map(&[])), None);
        assert_eq!(measured_today(&s, &map(&[]), &map(&[])), None);
    }

    #[test]
    fn measured_today_from_power_when_no_counter() {
        let s = SolarSources {
            counters: vec![],
            powers: vec![src("p", 0.001, -1.0)],
        };
        assert_eq!(
            measured_today(&s, &map(&[]), &map(&[("p", Some(-3.0))])),
            Some(3.0)
        );
        assert_eq!(
            measured_today(&SolarSources::default(), &map(&[]), &map(&[])),
            None
        );
    }

    #[test]
    fn resample_means_points_per_period_and_sums_sources() {
        let periods = [
            period(1000 + 1800, 30, 1.0),
            period(1000 + 3600, 30, 1.0),
            period(1000 + 5400, 30, 1.0),
        ];
        let a = (
            src("a", 0.001, 1.0),
            series(&[(1000, 1000.0), (1900, 2000.0)]),
        );
        let b = (src("b", 0.001, 1.0), series(&[(1500, 500.0)]));
        let r = resample(&[a.clone(), b.clone()], &periods, 0, 100_000);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, 2800);
        assert!((r[0].1 - 2.0).abs() < 1e-9);
        // Periods ending at or before `from` or starting at or after `until` are absent.
        assert!(resample(&[a.clone(), b.clone()], &periods, 2800, 100_000).is_empty());
        assert!(resample(&[a, b], &periods, 0, 1000).is_empty());
    }

    #[test]
    fn forecast_so_far_prorates_current_period() {
        let periods = [period(1800, 30, 2.0), period(3600, 30, 2.0)];
        assert!((forecast_so_far(&periods, 2700) - 1.5).abs() < 1e-9);
        assert_eq!(periods_in(&periods, 1800, 3600).len(), 1);
    }

    #[test]
    fn tomorrow_is_matched_by_date() {
        let s = view(0, &["2026-10-25", "2026-10-26"]);
        assert_eq!(
            tomorrow(&s, date(2026, 10, 24)).map(|d| d.date.as_str()),
            Some("2026-10-25")
        );
        assert!(tomorrow(&view(0, &["2026-10-26"]), date(2026, 10, 24)).is_none());
    }

    fn fc(d: &str, estimate: f64, made_at: u64) -> SolarForecastDay {
        SolarForecastDay {
            date: d.into(),
            estimate,
            low: estimate * 0.8,
            high: estimate * 1.2,
            made_at,
        }
    }

    fn meas(items: &[(NaiveDate, f64)]) -> BTreeMap<NaiveDate, f64> {
        items.iter().cloned().collect()
    }

    #[test]
    fn gap_is_signed_and_hidden_on_tiny_forecast() {
        assert_eq!(gap_percent(Some(6.4), 7.2), Some(-11));
        assert_eq!(gap_percent(Some(1.0), 0.05), None);
        assert_eq!(gap_percent(None, 7.2), None);
    }

    #[test]
    fn best_hours_floor_is_sixty_percent_of_peak() {
        assert!((best_hours_floor(5.0) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn accuracy_needs_three_morning_days_over_half_a_kwh() {
        let m = meas(&[
            (date(2026, 10, 20), 9.0),
            (date(2026, 10, 21), 11.0),
            (date(2026, 10, 22), 12.0),
            (date(2026, 10, 23), 0.3),
        ]);
        let sunrise = |_d: NaiveDate| Some(1_000u64);
        let mut f = vec![
            fc("2026-10-20", 10.0, 500),
            fc("2026-10-21", 10.0, 500),
            fc("2026-10-22", 10.0, 500),
            fc("2026-10-23", 0.4, 500),
        ];
        assert_eq!(accuracy(&m, &f, sunrise), Some(13));
        f[2].made_at = 1_000 + 7_200;
        assert_eq!(accuracy(&m, &f, sunrise), None);
    }

    #[test]
    fn yesterday_marks_a_late_forecast() {
        let d = date(2026, 10, 23);
        let m = meas(&[(d, 9.0)]);
        let f = [fc("2026-10-23", 10.0, 1_000 + 7_200)];
        let y = yesterday(&m, &f, d, Some(1_000)).unwrap();
        assert_eq!(y.measured, 9.0);
        assert_eq!(y.forecast, Some(10.0));
        assert_eq!(y.gap, Some(-10));
        assert_eq!(y.made_late_at, Some(1_000 + 7_200));
        let early = [fc("2026-10-23", 10.0, 900)];
        assert_eq!(
            yesterday(&m, &early, d, Some(1_000)).unwrap().made_late_at,
            None
        );
        assert!(yesterday(&BTreeMap::new(), &f, d, Some(1_000)).is_none());
    }

    #[test]
    fn daily_by_date_uses_house_zone_and_sign() {
        let (midnight, _) = day_bounds(Paris, date(2026, 10, 24));
        let out = daily_by_date(Paris, &[(midnight as i64, -3.0), (-5, 1.0)], -1.0);
        assert_eq!(out.get(&date(2026, 10, 24)), Some(&3.0));
        assert_eq!(out.len(), 1);
        let a = meas(&[(date(2026, 10, 1), 1.0), (date(2026, 10, 2), 2.0)]);
        let b = meas(&[(date(2026, 10, 2), 3.0)]);
        assert_eq!(sum_daily(&[a, b]), meas(&[(date(2026, 10, 2), 5.0)]));
    }

    #[test]
    fn month_so_far_on_the_first() {
        let m = meas(&[(date(2026, 9, 30), 4.0)]);
        assert_eq!(month_so_far(&m, date(2026, 10, 1), Some(2.0)), Some(2.0));
        assert_eq!(month_so_far(&m, date(2026, 10, 1), None), None);
        let m = meas(&[(date(2026, 10, 1), 4.0), (date(2026, 10, 2), 5.0)]);
        assert_eq!(month_so_far(&m, date(2026, 10, 3), Some(1.0)), Some(10.0));
    }
}
