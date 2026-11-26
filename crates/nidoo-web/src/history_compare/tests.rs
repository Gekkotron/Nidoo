use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn bucket_index_tolerates_dst_hour() {
    assert_eq!(bucket_index(0, 86_400), 0);
    assert_eq!(bucket_index(82_800, 86_400), 1);
    assert_eq!(bucket_index(169_200, 86_400), 2);
    assert_eq!(bucket_index(5, 0), 0);
}

#[test]
fn counter_summary_matches_days() {
    let d = 86_400;
    let shown = [(0, 10.0), (d, 12.0), (2 * d, 14.0)];
    let cmp = [(1000, 12.0), (1000 + d, 12.0), (1000 + 2 * d, 16.0)];
    let s = counter_summary(&shown, 0, &cmp, 1000, DailyPer::Day).unwrap();
    assert!(close(s.shown, 36.0));
    assert!(close(s.compared, 40.0));
    assert!(close(s.diff, -4.0));
    assert!(close(s.pct.unwrap(), -10.0));
    assert_eq!(s.left_out, 0);
}

#[test]
fn counter_summary_leaves_out_unmatched_days() {
    let d = 86_400;
    let shown = [(0, 10.0), (d, 12.0), (28 * d, 9.0)];
    let cmp = [(0, 12.0), (d, 12.0)];
    let s = counter_summary(&shown, 0, &cmp, 0, DailyPer::Day).unwrap();
    assert!(close(s.shown, 22.0));
    assert!(close(s.compared, 24.0));
    assert_eq!(s.left_out, 1);
}

#[test]
fn summary_none_when_compared_empty() {
    assert_eq!(counter_summary(&[(0, 1.0)], 0, &[], 0, DailyPer::Day), None);
}

#[test]
fn sensor_summary_means_matched() {
    let shown = [(0, 20.0, 18.0, 22.0), (3600, 21.0, 19.0, 25.0)];
    let cmp = [(0, 19.0, 17.0, 21.0), (3600, 19.0, 16.0, 23.0)];
    let s = sensor_summary(&shown, 0, &cmp, 0, 3600).unwrap();
    assert!(close(s.mean, 20.5));
    assert!(close(s.cmp_mean, 19.0));
    assert!(close(s.diff, 1.5));
    assert!(close(s.min, 18.0) && close(s.max, 25.0));
    assert!(close(s.cmp_min, 16.0) && close(s.cmp_max, 23.0));
    assert_eq!(s.left_out, 0);
}

#[test]
fn stats_without_comparison() {
    assert!(close(counter_total(&[(0, 1.5), (86_400, 2.5)]), 4.0));
    assert_eq!(sensor_stats(&[]), None);
    assert_eq!(
        sensor_stats(&[(0, 1.0, 0.0, 2.0), (1, 3.0, 1.0, 5.0)]),
        Some((2.0, 0.0, 5.0))
    );
}
