use super::*;

const DAY: i64 = 86_400;

#[test]
fn compared_points_land_on_the_shown_axis_and_are_cut_at_its_length() {
    let cmp: Vec<(i64, f64)> = (0..5).map(|i| (1_000_000 + i * DAY, i as f64)).collect();
    let out = place(&cmp, 1_000_000, 0, 3 * DAY, DAY);
    assert_eq!(out.len(), 3);
    assert_eq!(out[1].t, DAY);
    assert_eq!(out[1].orig, 1_000_000 + DAY);
    assert_eq!(out[2].idx, 2);
}

#[test]
fn a_dst_day_still_lands_on_its_index() {
    // The compared bucket starts 23 h after its window start.
    let out = place(&[(23 * 3_600, 1.0)], 0, 5_000_000, 10 * DAY, DAY);
    assert_eq!(out[0].idx, 1);
    assert_eq!(out[0].t, 5_000_000 + DAY);
}

#[test]
fn a_part_of_a_bucket_is_cut_when_the_shown_period_is_running() {
    // The shown reply ran 14 h 20 min into its day: the 15:00 bucket is beyond it.
    let hours: Vec<(i64, f64)> = (0..24).map(|h| (h * 3_600, 1.0)).collect();
    let out = place(&hours, 0, 0, 14 * 3_600 + 1_200, 3_600);
    assert_eq!(out.len(), 15);
}

#[test]
fn no_nominal_keeps_the_raw_offset() {
    let out = place(&[(120, 1.0)], 100, 1_000, 10_000, 0);
    assert_eq!(out[0].t, 1_020);
}

#[test]
fn before_the_start_is_dropped() {
    assert!(place(&[(-5 * DAY, 1.0)], 0, 0, 10 * DAY, DAY).is_empty());
}

#[test]
fn slots_follow_the_period_not_the_reply() {
    assert_eq!(slot_count(30 * DAY, DAY), 30);
    assert_eq!(slot_count(24 * 3_600, 3_600), 24);
    assert_eq!(slot_count(365 * DAY, nominal(DailyPer::Month)), 12);
    assert_eq!(slot_count(366 * DAY, nominal(DailyPer::Month)), 12);
    assert_eq!(slot_count(0, DAY), 1);
}

#[test]
fn a_running_month_leaves_the_rest_empty() {
    let shown: Vec<(i64, f64)> = (0..10).map(|i| (i * DAY, 1.0)).collect();
    let cmp = place(
        &(0..30).map(|i| (i * DAY, 2.0)).collect::<Vec<_>>(),
        0,
        0,
        10 * DAY,
        DAY,
    );
    let slots = bar_slots(&shown, &cmp, 0, DAY, 30);
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
        DAY,
    );
    let hit = compared_at(&cmp, 100 * DAY + DAY, 100 * DAY, DAY).unwrap();
    assert_eq!(hit.orig, DAY);
    assert!(compared_at(&cmp, 100 * DAY + 3 * DAY, 100 * DAY, DAY).is_none());
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
