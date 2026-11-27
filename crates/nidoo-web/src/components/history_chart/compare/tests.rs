use super::*;

const DAY: i64 = 86_400;

#[test]
fn compared_points_land_on_the_shown_axis_and_are_cut_at_its_length() {
    let cmp: Vec<(i64, f64)> = (0..5).map(|i| (1_000_000 + i * DAY, i as f64)).collect();
    let out = place(&cmp, 1_000_000, 0, 3 * DAY, Bucketing::Nominal(DAY));
    assert_eq!(out.len(), 3);
    assert_eq!(out[1].t, DAY);
    assert_eq!(out[1].orig, 1_000_000 + DAY);
    assert_eq!(out[2].idx, 2);
}

#[test]
fn a_dst_day_still_lands_on_its_index() {
    // The compared bucket starts 23 h after its window start.
    let out = place(
        &[(23 * 3_600, 1.0)],
        0,
        5_000_000,
        10 * DAY,
        Bucketing::Nominal(DAY),
    );
    assert_eq!(out[0].idx, 1);
    assert_eq!(out[0].t, 5_000_000 + DAY);
}

#[test]
fn a_part_of_a_bucket_is_cut_when_the_shown_period_is_running() {
    // The shown reply ran 14 h 20 min into its day: the 15:00 bucket is beyond it.
    let hours: Vec<(i64, f64)> = (0..24).map(|h| (h * 3_600, 1.0)).collect();
    let out = place(&hours, 0, 0, 14 * 3_600 + 1_200, Bucketing::Nominal(3_600));
    assert_eq!(out.len(), 15);
}

#[test]
fn no_nominal_keeps_the_raw_offset() {
    let out = place(&[(120, 1.0)], 100, 1_000, 10_000, Bucketing::Nominal(0));
    assert_eq!(out[0].t, 1_020);
}

#[test]
fn before_the_start_is_dropped() {
    assert!(place(&[(-5 * DAY, 1.0)], 0, 0, 10 * DAY, Bucketing::Nominal(DAY)).is_empty());
}

fn req(from: (i32, u32, u32), to: (i32, u32, u32)) -> WindowReq {
    WindowReq {
        from: chrono::NaiveDate::from_ymd_opt(from.0, from.1, from.2).unwrap(),
        to: chrono::NaiveDate::from_ymd_opt(to.0, to.1, to.2).unwrap(),
        upto: None,
    }
}

#[test]
fn slots_follow_the_period_not_the_reply() {
    assert_eq!(
        slots_for(DailyPer::Day, &req((2026, 11, 1), (2026, 11, 30))),
        30
    );
    assert_eq!(
        slots_for(DailyPer::Hour, &req((2026, 11, 26), (2026, 11, 26))),
        24
    );
    assert_eq!(
        slots_for(DailyPer::Month, &req((2026, 1, 1), (2026, 12, 31))),
        12
    );
    // A year starting mid-month touches 13 calendar months.
    assert_eq!(
        slots_for(DailyPer::Month, &req((2025, 11, 28), (2026, 11, 27))),
        13
    );
    assert_eq!(
        slots_for(DailyPer::Month, &req((2026, 1, 15), (2027, 1, 14))),
        13
    );
}

#[test]
fn a_clock_change_day_has_its_real_hours() {
    // 25 October 2026 falls back (25 h), 29 March 2026 springs forward (23 h).
    assert_eq!(
        slots_for(DailyPer::Hour, &req((2026, 10, 25), (2026, 10, 25))),
        25
    );
    assert_eq!(
        slots_for(DailyPer::Hour, &req((2026, 3, 29), (2026, 3, 29))),
        23
    );
}

#[test]
fn month_buckets_are_placed_by_calendar_month() {
    // Window 28 Nov 2025 .. 27 Nov 2026: stub, then each 1st. Local midnight is UTC-1 in winter.
    let mid = |y: i32, m: u32, d: u32| {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days()
            * DAY
            - 3_600
    };
    let items = [
        (mid(2025, 11, 28), 1.0),
        (mid(2025, 12, 1), 2.0),
        (mid(2026, 1, 1), 3.0),
    ];
    let start = items[0].0;
    let out = place(&items, start, start, 365 * DAY, Bucketing::Months);
    assert_eq!(out.iter().map(|s| s.idx).collect::<Vec<_>>(), [0, 1, 2]);
    let slots = bar_slots(&items, &out, start, Bucketing::Months, 13);
    assert_eq!(slots[1].shown.map(|s| s.1), Some(2.0));
}

#[test]
fn a_running_month_leaves_the_rest_empty() {
    let shown: Vec<(i64, f64)> = (0..10).map(|i| (i * DAY, 1.0)).collect();
    let cmp = place(
        &(0..30).map(|i| (i * DAY, 2.0)).collect::<Vec<_>>(),
        0,
        0,
        10 * DAY,
        Bucketing::Nominal(DAY),
    );
    let slots = bar_slots(&shown, &cmp, 0, Bucketing::Nominal(DAY), 30);
    assert_eq!(slots.len(), 30);
    assert!(
        slots[..10]
            .iter()
            .all(|s| s.shown.is_some() && s.cmp.is_some())
    );
    assert!(
        slots[10..]
            .iter()
            .all(|s| s.shown.is_none() && s.cmp.is_none())
    );
}

#[test]
fn a_compared_bar_is_wider_and_both_share_the_scale() {
    let (x, w) = bar_box(3, 30, SHOWN_W);
    assert!((x - 0.1).abs() < 1e-9);
    assert!((w - 0.8 / 30.0).abs() < 1e-9);
    let (_, wc) = bar_box(3, 30, COMPARED_W);
    assert!(wc > w);
    let slots = vec![
        BarSlot {
            shown: Some((0, 2.0)),
            cmp: Some((0, 8.0)),
        },
        BarSlot {
            shown: None,
            cmp: None,
        },
    ];
    assert!((slots_max(&slots) - 8.0).abs() < 1e-9);
}

#[test]
fn the_compared_point_at_a_bucket_is_found_by_index() {
    let cmp = place(
        &[(0, (1.0, 0.5, 1.5)), (DAY, (2.0, 1.5, 2.5))],
        0,
        100 * DAY,
        5 * DAY,
        Bucketing::Nominal(DAY),
    );
    let hit = compared_at(&cmp, 100 * DAY + DAY, 100 * DAY, Bucketing::Nominal(DAY)).unwrap();
    assert_eq!(hit.orig, DAY);
    assert!(
        compared_at(
            &cmp,
            100 * DAY + 3 * DAY,
            100 * DAY,
            Bucketing::Nominal(DAY)
        )
        .is_none()
    );
}

#[test]
fn the_tooltip_dates_carry_the_year() {
    use crate::i18n::Lang;
    // 2026-11-26 12:00 UTC
    let t = 1_795_694_400;
    assert!(
        day_when(Lang::En, t).contains("2026"),
        "{}",
        day_when(Lang::En, t)
    );
}

#[test]
fn a_series_in_another_step_is_not_comparable() {
    let mk = |window| HistorySeries {
        window,
        points: vec![],
        span: None,
    };
    let (a, b) = (mk(86_400), mk(18_000));
    assert!(same_step(&a, Some(&b)).is_none());
    assert!(same_step(&a, Some(&a)).is_some());
    assert!(same_step(&a, None).is_none());
}
