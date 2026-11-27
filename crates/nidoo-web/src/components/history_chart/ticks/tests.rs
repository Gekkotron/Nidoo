use super::*;
use crate::i18n::Lang;
use chrono::NaiveDate;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn req(from: NaiveDate, to: NaiveDate) -> WindowReq {
    WindowReq {
        from,
        to,
        upto: None,
    }
}

fn labels(t: &[(f64, String)]) -> Vec<&str> {
    t.iter().map(|(_, l)| l.as_str()).collect()
}

#[test]
fn a_past_month_has_day_numbers_and_no_now() {
    let t = window_ticks(
        Lang::En,
        Unit::Month,
        &req(d(2025, 11, 1), d(2025, 11, 30)),
        None,
        false,
    );
    assert_eq!(labels(&t), ["01", "08", "15", "22", "29"]);
    assert!((t[0].0 - 0.5 / 30.0).abs() < 1e-9);
    assert!(t.iter().all(|(p, _)| (0.0..1.0).contains(p)));
}

#[test]
fn a_calendar_year_starts_at_january() {
    let t = window_ticks(
        Lang::En,
        Unit::Year,
        &req(d(2026, 1, 1), d(2026, 12, 31)),
        None,
        false,
    );
    assert_eq!(t.len(), 12);
    assert_eq!(t[0].1, "Jan");
    assert_eq!(t[11].1, "Dec");
    assert!(t.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn a_year_window_starts_at_its_own_first_month() {
    let t = window_ticks(
        Lang::En,
        Unit::Year,
        &req(d(2025, 11, 1), d(2026, 10, 31)),
        None,
        false,
    );
    assert_eq!(t.len(), 12);
    assert_eq!(t[0].1, "Nov");
    assert_eq!(t[11].1, "Oct");
}

#[test]
fn a_week_names_each_day() {
    let t = window_ticks(
        Lang::En,
        Unit::Week,
        &req(d(2026, 11, 23), d(2026, 11, 29)),
        None,
        false,
    );
    assert_eq!(
        labels(&t),
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    );
}

#[test]
fn a_day_is_hours() {
    let t = window_ticks(
        Lang::En,
        Unit::Day,
        &req(d(2026, 11, 26), d(2026, 11, 26)),
        None,
        false,
    );
    assert_eq!(labels(&t), ["00:00", "06:00", "12:00", "18:00"]);
    assert!((t[2].0 - 0.5).abs() < 1e-9);
}

#[test]
fn a_custom_window_follows_its_length() {
    let week = window_ticks(
        Lang::En,
        Unit::Custom,
        &req(d(2026, 11, 2), d(2026, 11, 5)),
        None,
        false,
    );
    assert_eq!(labels(&week), ["Mon", "Tue", "Wed", "Thu"]);
    let month = window_ticks(
        Lang::En,
        Unit::Custom,
        &req(d(2026, 11, 3), d(2026, 12, 2)),
        None,
        false,
    );
    assert_eq!(month[0].1, "03");
    assert!(month.len() <= 6);
    let long = window_ticks(
        Lang::En,
        Unit::Custom,
        &req(d(2024, 1, 1), d(2026, 11, 30)),
        None,
        false,
    );
    assert!(long.len() <= 14, "{}", long.len());
}

#[test]
fn a_running_window_gets_now_and_drops_close_ticks() {
    let t = window_ticks(
        Lang::En,
        Unit::Month,
        &req(d(2026, 11, 1), d(2026, 11, 30)),
        Some(0.5),
        false,
    );
    let now: Vec<_> = t.iter().filter(|(_, l)| l == "now").collect();
    assert_eq!(now.len(), 1);
    assert!((now[0].0 - 0.5).abs() < 1e-9);
    // "15" sits at 14.5/30 = 0.483, too close to now.
    assert!(!labels(&t).contains(&"15"));
}

#[test]
fn month_bars_put_each_month_on_its_slot() {
    // 28 Nov to 27 Nov touches 13 months: the labels sit on 13 equal slots.
    let t = window_ticks(
        Lang::En,
        Unit::Year,
        &req(d(2025, 11, 28), d(2026, 11, 27)),
        None,
        true,
    );
    assert_eq!(t.len(), 13);
    assert!((t[0].0 - 0.5 / 13.0).abs() < 1e-9);
    assert_eq!(t[0].1, "Nov");
}
