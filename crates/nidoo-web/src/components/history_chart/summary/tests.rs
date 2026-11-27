use super::*;
use nidoo_api::{HistoryDaily, HistorySeries};

const DAY: i64 = 86_400;

fn bars(start: i64, vals: &[f64]) -> State {
    State::Bars(HistoryDaily {
        days: vals
            .iter()
            .enumerate()
            .map(|(i, v)| (start + i as i64 * DAY, *v))
            .collect(),
        span: Some((start, start + vals.len() as i64 * DAY)),
    })
}

fn series(start: i64, means: &[f64]) -> State {
    State::Series(HistorySeries {
        window: 3_600,
        points: means
            .iter()
            .enumerate()
            .map(|(i, m)| (start + i as i64 * 3_600, *m, *m - 1.0, *m + 1.0))
            .collect(),
        span: Some((start, start + means.len() as i64 * 3_600)),
    })
}

#[test]
fn without_a_comparison_the_shown_period_is_summed() {
    let s = summary(&bars(0, &[1.0, 2.0, 3.0]), None, false, DailyPer::Day).unwrap();
    assert_eq!(s.shown, Figures::Total(6.0));
    assert_eq!(s.compared, None);
    assert_eq!(s.diff, None);
}

#[test]
fn a_comparison_shows_both_and_the_difference() {
    let s = summary(
        &bars(0, &[2.0, 2.0]),
        Some(&bars(500 * DAY, &[1.0, 1.0])),
        true,
        DailyPer::Day,
    )
    .unwrap();
    assert_eq!(s.shown, Figures::Total(4.0));
    assert_eq!(s.compared, Some(Some(Figures::Total(2.0))));
    match s.diff {
        Some(Diff::Total { diff, pct }) => {
            assert!((diff - 2.0).abs() < 1e-9);
            assert!((pct.unwrap() - 100.0).abs() < 1e-9);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.left_out, 0);
}

#[test]
fn a_missing_compared_reply_has_no_difference_card() {
    let s = summary(&bars(0, &[2.0, 2.0]), None, true, DailyPer::Day).unwrap();
    assert_eq!(s.shown, Figures::Total(4.0));
    assert_eq!(s.compared, Some(None));
    assert_eq!(s.diff, None);
}

#[test]
fn only_shared_buckets_are_compared_and_the_rest_is_counted() {
    let s = summary(
        &bars(0, &[2.0, 2.0, 2.0]),
        Some(&bars(500 * DAY, &[1.0])),
        true,
        DailyPer::Day,
    )
    .unwrap();
    assert_eq!(s.shown, Figures::Total(2.0));
    assert_eq!(s.left_out, 2);
}

#[test]
fn a_sensor_gives_mean_range_and_difference() {
    let s = summary(
        &series(0, &[20.0, 22.0]),
        Some(&series(500 * DAY, &[18.0, 18.0])),
        true,
        DailyPer::Day,
    )
    .unwrap();
    match (s.shown, s.diff) {
        (Figures::Stats { mean, min, max }, Some(Diff::Mean(d))) => {
            assert!((mean - 21.0).abs() < 1e-9);
            assert!((min - 19.0).abs() < 1e-9 && (max - 23.0).abs() < 1e-9);
            assert!((d - 3.0).abs() < 1e-9);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn nothing_shown_means_no_summary() {
    assert!(summary(&bars(0, &[]), None, false, DailyPer::Day).is_none());
    assert!(summary(&State::Loading, None, false, DailyPer::Day).is_none());
}

#[test]
fn a_signed_number_has_its_sign() {
    use crate::i18n::Lang;
    assert_eq!(signed(Lang::En, 1.5, 1), "+1.5");
    assert_eq!(signed(Lang::En, -1.5, 1), "\u{2212}1.5");
    assert_eq!(signed(Lang::En, 0.0, 1), "0.0");
}

#[test]
fn a_sensors_left_out_buckets_are_counted_in_the_nearest_unit() {
    assert_eq!(sensor_left_out(5, 3_600), (5, DailyPer::Hour));
    // Ten-minute buckets: 12 of them are 2 hours.
    assert_eq!(sensor_left_out(12, 600), (2, DailyPer::Hour));
    assert_eq!(sensor_left_out(3, 86_400), (3, DailyPer::Day));
    assert_eq!(sensor_left_out(2, 604_800), (14, DailyPer::Day));
}

#[test]
fn a_sensor_reports_what_it_left_out() {
    let s = summary(
        &series(0, &[20.0, 22.0, 23.0]),
        Some(&series(500 * DAY, &[18.0])),
        true,
        DailyPer::Day,
    )
    .unwrap();
    assert_eq!((s.left_out, s.left_per), (2, DailyPer::Hour));
}

#[test]
fn sensors_in_different_steps_are_not_compared() {
    let shown = series(0, &[20.0, 22.0]);
    let mut other = series(500 * DAY, &[18.0, 18.0]);
    if let State::Series(c) = &mut other {
        c.window = 18_000;
    }
    let s = summary(&shown, Some(&other), true, DailyPer::Day).unwrap();
    assert_eq!(s.compared, Some(None));
    assert_eq!(s.diff, None);
}
