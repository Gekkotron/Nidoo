use super::*;
use chrono_tz::Europe::Paris;

fn micros_of_local(zone: Tz, s: &str) -> Micros {
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
    zone.from_local_datetime(&naive)
        .single()
        .unwrap()
        .timestamp_micros()
}

fn hours_between(a: Micros, b: Micros) -> i64 {
    (b - a) / HOUR
}

use nidoo_core::energy::{OffPeakEntry, OffPeakWindow};

fn nd(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn at(s: &str) -> Micros {
    micros_of_local(Paris, s)
}

fn sample(s: &str, num: f64) -> Sample {
    Sample { at: at(s), num }
}

fn history() -> Vec<OffPeakEntry> {
    let w = |a: &str, b: &str| OffPeakWindow {
        start: a.to_owned(),
        end: b.to_owned(),
    };
    vec![OffPeakEntry {
        since: "2026-01-01".to_owned(),
        windows: vec![w("01:34", "07:04"), w("12:05", "14:34")],
    }]
}

fn kwh(carry: Option<f64>, samples: Vec<Sample>) -> CounterDay {
    CounterDay {
        carry,
        samples,
        factor: 1.0,
    }
}

#[test]
fn grid_day_scales_wh_counters() {
    let h = history();
    let r = DayReadings {
        bought: vec![CounterDay {
            carry: Some(100_000.0),
            samples: vec![sample("2026-10-23 08:00:00", 100_400.0)],
            factor: 0.001,
        }],
        sold: vec![],
    };
    let day = build_grid_day(&input("2026-10-23", &h, &r, at("2026-10-24 09:00:00")));
    assert_eq!(day.totals.peak, 0.4);
    assert_eq!(day.slots[16].peak, 0.4);
}

fn readings() -> DayReadings {
    DayReadings {
        bought: vec![kwh(
            Some(100.0),
            vec![
                sample("2026-10-23 01:40:00", 100.4),
                sample("2026-10-23 08:00:00", 101.0),
                sample("2026-10-23 13:00:00", 101.5),
            ],
        )],
        sold: vec![kwh(Some(5.0), vec![sample("2026-10-23 13:10:00", 5.6)])],
    }
}

fn input<'a>(
    date: &str,
    hist: &'a [OffPeakEntry],
    readings: &'a DayReadings,
    now: Micros,
) -> DayInput<'a> {
    DayInput {
        zone: Paris,
        date: nd(date),
        first_date: Some(nd("2026-01-15")),
        history: hist,
        readings,
        has_export: true,
        now,
        device: None,
    }
}

fn windows_from_jan(ws: &[(&str, &str)]) -> Vec<OffPeakEntry> {
    vec![OffPeakEntry {
        since: "2026-01-01".to_owned(),
        windows: ws
            .iter()
            .map(|(a, b)| OffPeakWindow {
                start: (*a).to_owned(),
                end: (*b).to_owned(),
            })
            .collect(),
    }]
}

fn span(from: Micros, to: Micros, off_peak: Option<bool>) -> DeviceSpan {
    DeviceSpan { from, to, off_peak }
}

fn secs(s: &str) -> i64 {
    at(s) / 1_000_000
}

#[test]
fn device_span_splits_a_slot() {
    let h = Vec::new();
    let end = at("2026-10-24 00:00:00");
    let spans = [
        span(
            at("2026-10-23 00:00:00"),
            at("2026-10-23 00:10:00"),
            Some(false),
        ),
        span(at("2026-10-23 00:10:00"), end, Some(true)),
    ];
    let r = DayReadings {
        bought: vec![kwh(
            Some(100.0),
            vec![
                sample("2026-10-23 00:05:00", 100.2),
                sample("2026-10-23 00:20:00", 100.5),
            ],
        )],
        sold: vec![],
    };
    let day = build_grid_day(&DayInput {
        device: Some(&spans),
        ..input("2026-10-23", &h, &r, at("2026-10-24 09:00:00"))
    });
    assert_eq!((day.slots[0].peak, day.slots[0].off_peak), (0.2, 0.3));
    assert!(day.tariffs);
    assert_eq!(day.bands, [(secs("2026-10-23 00:10:00"), end / 1_000_000)]);
}

#[test]
fn device_gap_falls_back_to_windows() {
    let h = windows_from_jan(&[("00:00", "06:00")]);
    let spans = [
        span(
            at("2026-10-23 00:00:00"),
            at("2026-10-23 01:00:00"),
            Some(false),
        ),
        span(at("2026-10-23 01:00:00"), at("2026-10-23 02:00:00"), None),
        span(
            at("2026-10-23 02:00:00"),
            at("2026-10-24 00:00:00"),
            Some(false),
        ),
    ];
    let r = DayReadings {
        bought: vec![kwh(
            Some(10.0),
            vec![
                sample("2026-10-23 00:30:00", 10.1),
                sample("2026-10-23 01:30:00", 10.3),
                sample("2026-10-23 03:00:00", 10.7),
            ],
        )],
        sold: vec![],
    };
    let day = build_grid_day(&DayInput {
        device: Some(&spans),
        ..input("2026-10-23", &h, &r, at("2026-10-24 09:00:00"))
    });
    assert_eq!((day.totals.off_peak, day.totals.peak), (0.2, 0.5));
}

#[test]
fn rest_of_today_uses_windows() {
    let h = windows_from_jan(&[("22:00", "06:00")]);
    let r = DayReadings::default();
    let spans = [span(
        at("2026-10-23 00:00:00"),
        at("2026-10-23 12:00:00"),
        Some(false),
    )];
    let day = build_grid_day(&DayInput {
        device: Some(&spans),
        ..input("2026-10-23", &h, &r, at("2026-10-23 12:00:00"))
    });
    assert_eq!(
        day.bands,
        [(secs("2026-10-23 22:00:00"), secs("2026-10-24 00:00:00"))]
    );
}

#[test]
fn day_without_device_readings_matches_windows_only() {
    let h = history();
    let r = readings();
    let now = at("2026-10-24 09:00:00");
    let without = build_grid_day(&input("2026-10-23", &h, &r, now));
    let with = build_grid_day(&DayInput {
        device: Some(&[]),
        ..input("2026-10-23", &h, &r, now)
    });
    assert_eq!(with, without);
}

#[test]
fn device_spans_merge_value_and_availability() {
    use nidoo_core::energy::OffPeakDevice;
    use nidoo_core::model::{Availability, StateValue};
    let device = OffPeakDevice {
        entity: "sensor.tariff".to_owned(),
        values: vec!["OFF".to_owned()],
    };
    let (start, until) = (at("2026-10-23 00:00:00"), at("2026-10-23 12:00:00"));
    let spans = device_spans(
        &device,
        start,
        until,
        (Some(StateValue::Text("OFF".to_owned())), Vec::new()),
        (
            None,
            vec![
                (at("2026-10-23 03:00:00"), Availability::Offline),
                (at("2026-10-23 04:00:00"), Availability::Online),
            ],
        ),
    );
    assert_eq!(
        spans,
        [
            span(start, at("2026-10-23 03:00:00"), Some(true)),
            span(at("2026-10-23 03:00:00"), at("2026-10-23 04:00:00"), None),
            span(at("2026-10-23 04:00:00"), until, Some(true)),
        ]
    );
    assert!(device_spans(&device, until, start, (None, vec![]), (None, vec![])).is_empty());
}

#[test]
fn fall_back_day_has_50_slots() {
    let (a, b) = day_bounds(Paris, nd("2026-10-25"));
    assert_eq!(hours_between(a, b), 25);
    assert_eq!(slot_starts(a, b, i64::MAX).len(), 50);
    let (a, b) = day_bounds(Paris, nd("2026-03-29"));
    assert_eq!(slot_starts(a, b, i64::MAX).len(), 46);
    assert_eq!(
        day_bounds(Paris, nd("2026-10-23")).0,
        at("2026-10-23 00:00:00")
    );
}

#[test]
fn grid_day_totals_match_counter_increase() {
    let h = history();
    let r = readings();
    let day = build_grid_day(&input("2026-10-23", &h, &r, at("2026-10-24 09:00:00")));
    assert_eq!(day.date, "2026-10-23");
    assert_eq!(day.today, "2026-10-24");
    assert_eq!(day.first_date.as_deref(), Some("2026-01-15"));
    assert_eq!(day.slots.len(), 48);
    assert_eq!(day.day_slots, 48);
    assert_eq!(day.totals.off_peak, 0.9);
    assert_eq!(day.totals.peak, 0.6);
    assert_eq!(day.totals.export, 0.6);
    assert_eq!(day.slots[3].local, "01:30");
    assert_eq!(day.slots[3].off_peak, 0.4);
    assert_eq!(day.slots[16].peak, 0.6);
    assert!(day.tariffs && day.has_export);
}

#[test]
fn grid_day_today_is_partial() {
    let h = history();
    let r = readings();
    let day = build_grid_day(&input("2026-10-23", &h, &r, at("2026-10-23 10:15:00")));
    assert_eq!(day.slots.len(), 21);
    assert_eq!(day.day_slots, 48);
    assert_eq!(day.date, day.today);
    assert_eq!(day.totals.peak, 0.6);
    assert_eq!(day.totals.export, 0.0);
}

#[test]
fn grid_day_uses_windows_of_that_day() {
    let w = |a: &str, b: &str| OffPeakWindow {
        start: a.to_owned(),
        end: b.to_owned(),
    };
    let h = vec![
        OffPeakEntry {
            since: "2026-10-22".to_owned(),
            windows: vec![w("07:30", "09:00")],
        },
        OffPeakEntry {
            since: "2026-10-23".to_owned(),
            windows: vec![],
        },
    ];
    let r = DayReadings {
        bought: vec![kwh(Some(1.0), vec![sample("2026-10-22 08:00:00", 2.0)])],
        sold: vec![],
    };
    let day = build_grid_day(&input("2026-10-22", &h, &r, at("2026-10-24 09:00:00")));
    assert_eq!((day.totals.off_peak, day.totals.peak), (1.0, 0.0));
    let r = DayReadings {
        bought: vec![kwh(Some(1.0), vec![sample("2026-10-23 08:00:00", 2.0)])],
        sold: vec![],
    };
    let day = build_grid_day(&input("2026-10-23", &h, &r, at("2026-10-24 09:00:00")));
    assert_eq!((day.totals.off_peak, day.totals.peak), (0.0, 1.0));
    assert!(!day.tariffs && day.bands.is_empty());
}

#[test]
fn grid_day_bands_follow_windows() {
    let h = history();
    let r = readings();
    let day = build_grid_day(&input("2026-10-23", &h, &r, at("2026-10-24 09:00:00")));
    let secs = |s: &str| at(s) / 1_000_000;
    assert_eq!(
        day.bands,
        vec![
            (secs("2026-10-23 01:34:00"), secs("2026-10-23 07:04:00")),
            (secs("2026-10-23 12:05:00"), secs("2026-10-23 14:34:00"))
        ]
    );
}

#[test]
fn grid_day_band_crossing_midnight_is_clipped_to_the_day() {
    let h = vec![OffPeakEntry {
        since: "2026-01-01".to_owned(),
        windows: vec![OffPeakWindow {
            start: "22:00".to_owned(),
            end: "06:00".to_owned(),
        }],
    }];
    let r = DayReadings::default();
    let day = build_grid_day(&input("2026-10-23", &h, &r, at("2026-10-24 09:00:00")));
    let secs = |s: &str| at(s) / 1_000_000;
    assert_eq!(
        day.bands,
        vec![
            (secs("2026-10-23 00:00:00"), secs("2026-10-23 06:00:00")),
            (secs("2026-10-23 22:00:00"), secs("2026-10-24 00:00:00"))
        ]
    );
}

#[test]
fn day_starts_cross_dst() {
    // Fall-back: 2026-10-25 is 25 h in Europe/Paris.
    let now_fall = micros_of_local(Paris, "2026-10-26 10:00:00");
    let starts = day_starts(Paris, now_fall, 2);
    assert_eq!(starts.len(), 3);
    assert_eq!(starts[0], micros_of_local(Paris, "2026-10-25 00:00:00"));
    assert_eq!(starts[1], micros_of_local(Paris, "2026-10-26 00:00:00"));
    assert_eq!(starts[2], now_fall);
    assert_eq!(hours_between(starts[0], starts[1]), 25);

    // Spring-forward: 2026-03-29 is 23 h.
    let now_spring = micros_of_local(Paris, "2026-03-30 10:00:00");
    let starts = day_starts(Paris, now_spring, 2);
    assert_eq!(starts[0], micros_of_local(Paris, "2026-03-29 00:00:00"));
    assert_eq!(starts[1], micros_of_local(Paris, "2026-03-30 00:00:00"));
    assert_eq!(hours_between(starts[0], starts[1]), 23);
}

#[tokio::test]
async fn daily_window_rejects_a_bad_window_before_needing_the_db() {
    let (h, _rx) = History::detached(1, Paris);
    let w = DateWindow {
        from: nd("2026-11-27"),
        to: nd("2026-11-27"),
        upto: None,
    };
    let now = at("2026-11-26 12:00:00");
    let err = daily_window(&h, "e", &w, DailyPer::Day, EnergyKind::Counter, now)
        .await
        .unwrap_err();
    assert!(matches!(err, HistoryError::BadRequest(_)), "{err:?}");
}
