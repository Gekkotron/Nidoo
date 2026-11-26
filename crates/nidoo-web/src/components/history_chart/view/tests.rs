#[test]
fn only_a_percentage_field_fixes_the_extent() {
    let padded = YScale {
        min: 40.0,
        max: 60.0,
    };
    let fixed = percent_scale(Some("%"));
    assert_eq!(
        nice_input(fixed, 50.0, 55.0, padded),
        YScale {
            min: 0.0,
            max: 100.0
        }
    );
    // A headline percentage (humidity, battery) fits its data as before.
    assert_eq!(
        nice_input(None, 50.0, 55.0, padded),
        YScale {
            min: 50.0,
            max: 55.0
        }
    );
    assert_eq!(nice_input(None, 50.0, 50.0, padded), padded);
}

#[test]
fn a_bucket_ends_at_the_next_point_or_as_long_as_the_last_gap() {
    let pts = [
        (0, 1.0, 1.0, 1.0),
        (3600, 1.0, 1.0, 1.0),
        (7200, 1.0, 1.0, 1.0),
    ];
    assert_eq!(bucket_end(&pts, 0), 3600);
    assert_eq!(bucket_end(&pts, 2), 10_800);
    assert_eq!(bucket_end(&pts[..1], 0), 1);
}

#[test]
fn a_period_at_the_right_edge_stays_inside_the_box() {
    let style = period_style(0.999, 1.0);
    assert!(style.contains("min(99.90%, calc(100% - 3px))"), "{style}");
    assert!(style.contains("max(0.10%, 3px)"), "{style}");
}

use super::*;

#[test]
fn stale_guard_rejects_old_request_ids() {
    assert!(is_stale(1, 2));
    assert!(is_stale(3, 5));
    assert!(!is_stale(5, 5));
    assert!(!is_stale(1, 1));
}

#[test]
fn an_open_chart_refreshes_each_minute_on_day_ranges() {
    assert_eq!(refresh_period_s(HistoryRange::Hour), 60);
    assert_eq!(refresh_period_s(HistoryRange::SixHours), 60);
    assert_eq!(refresh_period_s(HistoryRange::Day), 60);
    assert_eq!(refresh_period_s(HistoryRange::Week), 600);
    assert_eq!(refresh_period_s(HistoryRange::Month), 600);
    assert_eq!(refresh_period_s(HistoryRange::Year), 600);
}

#[test]
fn daily_days_for_matches_range_windows() {
    assert_eq!(daily_days_for(HistoryRange::Week), 7);
    assert_eq!(daily_days_for(HistoryRange::Month), 30);
    assert_eq!(daily_days_for(HistoryRange::Year), 365);
    assert_eq!(daily_days_for(HistoryRange::Hour), 1);
    assert_eq!(daily_days_for(HistoryRange::SixHours), 1);
    assert_eq!(daily_days_for(HistoryRange::Day), 1);
}

#[test]
fn plan_fetch_series_flavor_uses_entity_series_for_every_range() {
    for r in [
        HistoryRange::Hour,
        HistoryRange::SixHours,
        HistoryRange::Day,
        HistoryRange::Week,
        HistoryRange::Month,
        HistoryRange::Year,
    ] {
        assert_eq!(
            plan_fetch("sensor.temp", r, Flavor::Series, None),
            FetchPlan::Series("sensor.temp".into())
        );
    }
}

#[test]
fn plan_fetch_counter_24h_uses_paired_power_series_when_present() {
    for r in [
        HistoryRange::Hour,
        HistoryRange::SixHours,
        HistoryRange::Day,
    ] {
        assert_eq!(
            plan_fetch("counter.e", r, Flavor::Counter, Some("sensor.power")),
            FetchPlan::Series("sensor.power".into())
        );
    }
}

#[test]
fn drawn_unit_is_the_paired_power_sensors_for_a_counter_day_chart() {
    let unit_of = |id: &str| (id == "sensor.power").then(|| "W".to_owned());
    let wh = || Some("Wh".to_owned());
    let paired = plan_fetch(
        "counter.e",
        HistoryRange::Day,
        Flavor::Counter,
        Some("sensor.power"),
    );
    assert_eq!(
        drawn_unit(&paired, "counter.e", wh(), unit_of),
        Some("W".into())
    );
    let own = plan_fetch(
        "counter.e",
        HistoryRange::Week,
        Flavor::Counter,
        Some("sensor.power"),
    );
    assert_eq!(drawn_unit(&own, "counter.e", wh(), unit_of), wh());
    let temp = plan_fetch("sensor.t", HistoryRange::Day, Flavor::Series, None);
    assert_eq!(
        drawn_unit(&temp, "sensor.t", Some("°C".into()), unit_of),
        Some("°C".into())
    );
}

#[test]
fn plan_fetch_counter_24h_falls_back_to_single_day_bar_without_power() {
    // Review Focus #2.
    for r in [
        HistoryRange::Hour,
        HistoryRange::SixHours,
        HistoryRange::Day,
    ] {
        assert_eq!(
            plan_fetch("counter.e", r, Flavor::Counter, None),
            FetchPlan::Daily("counter.e".into(), 1)
        );
    }
}

#[test]
fn plan_fetch_counter_long_ranges_use_daily_with_right_days() {
    assert_eq!(
        plan_fetch("counter.e", HistoryRange::Week, Flavor::Counter, None),
        FetchPlan::Daily("counter.e".into(), 7)
    );
    assert_eq!(
        plan_fetch("counter.e", HistoryRange::Month, Flavor::Counter, None),
        FetchPlan::Daily("counter.e".into(), 30)
    );
    assert_eq!(
        plan_fetch("counter.e", HistoryRange::Year, Flavor::Counter, None),
        FetchPlan::Daily("counter.e".into(), 365)
    );
}

#[test]
fn map_error_routes_each_variant_to_its_state() {
    // Review Focus #1: a 404 on a text/alarm/vacuum entity renders the empty copy.
    assert!(matches!(
        map_error(HistoryApiError::NotRecorded),
        State::NotRecorded
    ));
    assert!(matches!(
        map_error(HistoryApiError::Unavailable),
        State::Unavailable
    ));
    assert!(matches!(
        map_error(HistoryApiError::BadRequest("bad".into())),
        State::Error(_)
    ));
    assert!(matches!(
        map_error(HistoryApiError::Other("boom".into())),
        State::Error(_)
    ));
}

#[test]
fn series_tip_reads_value_with_unit_and_the_band_on_long_ranges() {
    let p = (1_791_288_000, 21.4, 19.0, 23.5);
    let day = series_tip(
        Lang::En,
        HistoryRange::Day,
        "Sensor",
        Some("°C"),
        1,
        1_791_300_000,
        &p,
    );
    assert_eq!(day.value, "21.4\u{a0}°C");
    assert_eq!(day.band, None);
    let week = series_tip(
        Lang::Fr,
        HistoryRange::Week,
        "Sensor",
        Some("°C"),
        1,
        1_791_300_000,
        &p,
    );
    assert_eq!(week.value, "21,4\u{a0}°C");
    assert_eq!(
        week.band.as_deref(),
        Some("min 19,0\u{a0}°C · max 23,5\u{a0}°C")
    );
    // A bucket whose min equals its max has no band line.
    let flat = (1_791_288_000, 5.0, 5.0, 5.0);
    let tip = series_tip(
        Lang::En,
        HistoryRange::Month,
        "Sensor",
        None,
        1,
        1_791_300_000,
        &flat,
    );
    assert_eq!(tip.band, None);
}

#[test]
fn the_band_is_keyed_only_where_it_is_drawn() {
    let spread = [(0, 20.0, 18.0, 22.0), (3_600, 21.0, 21.0, 21.0)];
    let flat = [(0, 20.0, 20.0, 20.0)];
    assert!(shows_band(HistoryRange::Week, false, &spread));
    assert!(shows_band(HistoryRange::Year, false, &spread));
    // Raw samples, a two-state entity and buckets that never moved have no band to explain.
    assert!(!shows_band(HistoryRange::Day, false, &spread));
    assert!(!shows_band(HistoryRange::Week, true, &spread));
    assert!(!shows_band(HistoryRange::Month, false, &flat));
}

#[test]
fn series_tip_words_a_discrete_state() {
    let p = (1_791_288_000, 1.0, 1.0, 1.0);
    let tip = series_tip(
        Lang::En,
        HistoryRange::Day,
        "Light",
        None,
        1,
        1_791_300_000,
        &p,
    );
    assert_eq!(tip.value, "On");
}

#[test]
fn gradient_stops_are_direct_children() {
    let m = gradient_markup("hc-fill-7");
    assert!(m.starts_with(r#"<linearGradient id="hc-fill-7""#));
    assert_eq!(m.matches("<stop ").count(), 2);
    assert!(!m.contains("<g>"));
}

#[test]
fn only_labels_just_before_now_are_near_the_end() {
    assert!(near_end(0.76));
    assert!(!near_end(0.6));
    assert!(!near_end(1.0));
}

#[test]
fn only_long_ranges_are_bucketed() {
    assert!(!is_bucketed(HistoryRange::Day));
    assert!(!is_bucketed(HistoryRange::Hour));
    assert!(is_bucketed(HistoryRange::Week));
    assert!(is_bucketed(HistoryRange::Year));
}

#[test]
fn gradient_ids_are_unique_per_chart() {
    assert_ne!(next_gradient_id(), next_gradient_id());
}

#[test]
fn series_min_max_handles_empty() {
    let (lo, hi) = series_min_max(&[]);
    assert_eq!((lo, hi), (0.0, 0.0));
}
