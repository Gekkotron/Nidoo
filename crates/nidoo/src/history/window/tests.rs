use chrono::NaiveDate;
use chrono_tz::Europe::Paris;
use nidoo_api::DailyPer;

use super::*;
use crate::history::query::local_midnight;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn win(from: &str, to: &str, upto: Option<i64>) -> DateWindow {
    DateWindow {
        from: d(from),
        to: d(to),
        upto,
    }
}

fn now() -> Micros {
    local_midnight(Paris, d("2026-11-26")) + (15 * 3600 + 40 * 60) * 1_000_000
}

#[test]
fn parse_window_rules() {
    assert!(parse_window(None, None, None).unwrap().is_none());
    assert!(parse_window(Some("2025-11-01"), None, None).is_err());
    assert!(parse_window(None, Some("2025-11-01"), None).is_err());
    assert!(parse_window(Some("2025-11-30"), Some("2025-11-01"), None).is_err());
    assert!(parse_window(Some("nope"), Some("2025-11-01"), None).is_err());
    assert!(parse_window(Some("2025-11-01"), Some("2025-11-30"), Some("0")).is_err());
    let w = parse_window(Some("2025-11-01"), Some("2025-11-30"), Some("3600"))
        .unwrap()
        .unwrap();
    assert_eq!(w.upto, Some(3600));
}

#[test]
fn resolve_clips_at_now_and_upto() {
    let r = resolve(Paris, &win("2026-11-01", "2026-11-30", None), now()).unwrap();
    assert_eq!(r.end, now());
    let r = resolve(Paris, &win("2026-11-01", "2026-11-30", Some(86_400)), now()).unwrap();
    assert_eq!(r.end, r.start + 86_400 * 1_000_000);
    assert!(resolve(Paris, &win("2026-11-27", "2026-11-27", None), now()).is_err());
}

#[test]
fn resolve_spans_dst_days() {
    let h = 3_600_000_000;
    let r = resolve(Paris, &win("2026-03-29", "2026-03-29", None), now()).unwrap();
    assert_eq!(r.end - r.start, 23 * h);
    let r = resolve(Paris, &win("2026-10-25", "2026-10-25", None), now()).unwrap();
    assert_eq!(r.end - r.start, 25 * h);
}

#[test]
fn step_secs_by_length() {
    for (n, s) in [
        (1, 600),
        (7, 3_600),
        (8, 3_600),
        (9, 18_000),
        (31, 18_000),
        (365, 86_400),
        (400, 86_400),
        (401, 604_800),
    ] {
        assert_eq!(step_secs(n), s, "{n} days");
    }
}

fn span(n: i64) -> DateWindow {
    let from = d("2020-01-01");
    DateWindow {
        from,
        to: from + chrono::Duration::days(n - 1),
        upto: None,
    }
}

#[test]
fn check_limits_caps() {
    assert!(check_limits(&span(401), WindowUse::Fields).is_err());
    assert!(check_limits(&span(400), WindowUse::Fields).is_ok());
    assert!(check_limits(&span(3), WindowUse::Daily(DailyPer::Hour)).is_err());
    assert!(check_limits(&span(2), WindowUse::Daily(DailyPer::Hour)).is_ok());
    assert!(check_limits(&span(1_101), WindowUse::Daily(DailyPer::Day)).is_err());
    assert!(check_limits(&span(1_100), WindowUse::Daily(DailyPer::Day)).is_ok());
    assert!(check_limits(&span(3_000), WindowUse::Daily(DailyPer::Month)).is_ok());
    assert!(check_limits(&span(3_000), WindowUse::Series).is_ok());
}

#[test]
fn boundaries_per_day_month_hour() {
    let w = win("2025-11-01", "2025-11-03", None);
    let r = resolve(Paris, &w, now()).unwrap();
    let b = boundaries(Paris, &w, &r, DailyPer::Day);
    assert_eq!(b.len(), 4);
    assert_eq!(b[1], local_midnight(Paris, d("2025-11-02")));
    assert_eq!(b[3], r.end);

    let w = win("2024-12-15", "2025-02-10", None);
    let r = resolve(Paris, &w, now()).unwrap();
    let b = boundaries(Paris, &w, &r, DailyPer::Month);
    assert_eq!(
        b,
        vec![
            local_midnight(Paris, d("2024-12-15")),
            local_midnight(Paris, d("2025-01-01")),
            local_midnight(Paris, d("2025-02-01")),
            r.end,
        ]
    );

    let w = win("2026-03-29", "2026-03-29", None);
    let r = resolve(Paris, &w, now()).unwrap();
    let b = boundaries(Paris, &w, &r, DailyPer::Hour);
    assert_eq!(b.len(), 24);
    assert_eq!(*b.last().unwrap(), r.end);
}
