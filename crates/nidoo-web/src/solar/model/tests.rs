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
