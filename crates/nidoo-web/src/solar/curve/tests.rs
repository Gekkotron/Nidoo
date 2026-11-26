use super::*;

fn p(end: u64, estimate: f64, low: f64, high: f64) -> SolarPeriod {
    SolarPeriod {
        end,
        minutes: 30,
        estimate,
        low,
        high,
    }
}

fn clean(c: &Curve) {
    for s in [
        &c.forecast_area,
        &c.forecast_line,
        &c.band,
        &c.measured_line,
        &c.measured_area,
    ] {
        assert!(!s.contains("NaN") && !s.contains("inf"), "{s}");
    }
}

fn numbers(s: &str) -> Vec<f64> {
    s.replace(['M', 'L', 'C', 'Z'], " ")
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect()
}

#[test]
fn soften_blends_inner_points_and_keeps_the_ends() {
    let pts = [(0.0, 0.0), (1.0, 0.0), (2.0, 4.0), (3.0, 0.0), (4.0, 2.0)];
    let ys: Vec<f64> = soften(&pts).iter().map(|p| p.1).collect();
    assert_eq!(ys, vec![0.0, 1.0, 2.0, 1.5, 2.0]);
    // x never moves; two points or fewer are left as they are.
    assert!(soften(&pts).iter().zip(&pts).all(|(a, b)| a.0 == b.0));
    assert_eq!(soften(&pts[..2]), pts[..2].to_vec());
    assert!(soften(&[]).is_empty());
}

#[test]
fn smooth_line_goes_through_every_point() {
    let pts = [(0.0, 40.0), (25.0, 10.0), (50.0, 30.0), (75.0, 5.0)];
    let tail = smooth_tail(&pts);
    assert_eq!(tail.matches(" C ").count(), 3, "{tail}");
    for (x, y) in &pts[1..] {
        assert!(tail.contains(&pt(*x, *y)), "{tail}");
    }
    assert_eq!(smooth_tail(&pts[..1]), "");
    assert_eq!(smooth_tail(&[]), "");
}

#[test]
fn smooth_line_never_overshoots_its_points() {
    // A flat run then a jump: control points stay between their segment's two ends, so the
    // curve never dips below zero nor peaks above the data.
    let pts = [
        (0.0, 40.0),
        (10.0, 40.0),
        (20.0, 5.0),
        (30.0, 5.0),
        (40.0, 20.0),
    ];
    let tail = smooth_tail(&pts);
    for (i, seg) in tail.split(" C ").skip(1).enumerate() {
        let n = numbers(seg);
        let (lo, hi) = (pts[i].1.min(pts[i + 1].1), pts[i].1.max(pts[i + 1].1));
        for y in [n[1], n[3]] {
            assert!(y >= lo - 0.05 && y <= hi + 0.05, "segment {i}: {seg}");
        }
        for x in [n[0], n[2]] {
            assert!(x >= pts[i].0 && x <= pts[i + 1].0, "segment {i}: {seg}");
        }
    }
    // Two points at the same x stay finite.
    let tail = smooth_tail(&[(10.0, 5.0), (10.0, 20.0), (20.0, 10.0)]);
    assert!(!tail.contains("NaN") && !tail.contains("inf"), "{tail}");
}

#[test]
fn span_skips_dark_periods() {
    let day = vec![
        p(1800, 0.0, 0.0, 0.0),
        p(3600, 0.5, 0.2, 0.8),
        p(5400, 1.0, 0.5, 1.5),
        p(7200, 0.0, 0.0, 0.3),
        p(9000, 0.0, 0.0, 0.0),
    ];
    assert_eq!(producing_span(&day), Some((1800, 7200)));
}

#[test]
fn span_none_without_production() {
    assert_eq!(producing_span(&[p(1800, 0.0, 0.0, 0.0)]), None);
    assert_eq!(producing_span(&[]), None);
}

#[test]
fn points_sit_at_period_midpoints_and_scale_to_max() {
    let day = vec![p(1800, 1.0, 0.5, 2.0), p(3600, 2.0, 1.0, 4.0)];
    let c = day_curve(&day, &[], (0, 3600), 0, 100.0, 40.0);
    assert!(c.forecast_line.starts_with("M 25"), "{}", c.forecast_line);
    // max 4.4 kW: the 4.0 high sits at 40 - 40 * 4 / 4.4.
    assert!(c.band.contains("75.0 3.6"), "{}", c.band);
    for s in [&c.forecast_area, &c.forecast_line, &c.band] {
        for pair in numbers(s).chunks(2) {
            assert!((0.0..=40.0).contains(&pair[1]), "{s}");
        }
    }
    assert!(c.grid.len() <= 4);
    clean(&c);
}

#[test]
fn measured_area_stops_at_last_measured_point() {
    let day = vec![p(1800, 1.0, 0.5, 2.0), p(3600, 2.0, 1.0, 4.0)];
    let c = day_curve(&day, &[(1800, 1.0)], (0, 3600), 2000, 100.0, 40.0);
    assert!(
        c.measured_area.starts_with("M 25.0 40.0"),
        "{}",
        c.measured_area
    );
    assert!(
        c.measured_area.ends_with("25.0 40.0 Z"),
        "{}",
        c.measured_area
    );
    assert_eq!(c.measured_area.matches("25.0").count(), 3);
    assert!(!c.measured_area.contains("75.0"));
    assert_eq!(c.measured_line, "M 25.0 30.9");
    let none = day_curve(&day, &[], (0, 3600), 0, 100.0, 40.0);
    assert_eq!(none.measured_area, "");
    assert_eq!(none.measured_line, "");
}

#[test]
fn period_in_progress_ends_at_now() {
    let day = vec![p(1800, 1.0, 0.5, 2.0), p(3600, 2.0, 1.0, 4.0)];
    let measured = [(1800, 1.0), (3600, 1.5)];
    let c = day_curve(&day, &measured, (0, 3600), 2700, 100.0, 40.0);
    // First period is over (midpoint 25), the second is in progress and ends at now (75).
    assert!(c.measured_line.starts_with("M 25.0"), "{}", c.measured_line);
    assert!(c.measured_line.contains(" C "), "{}", c.measured_line);
    assert!(
        c.measured_line.ends_with(" 75.0 26.4"),
        "{}",
        c.measured_line
    );
    assert_eq!(c.now_x, Some(75.0));
    assert!(
        c.measured_area.ends_with("75.0 40.0 Z"),
        "{}",
        c.measured_area
    );
    // A period starting at or after now has no point.
    let c = day_curve(&day, &measured, (0, 3600), 1800, 100.0, 40.0);
    assert!(!c.measured_line.contains("75.0"), "{}", c.measured_line);
    assert!(!c.measured_line.contains('C'), "{}", c.measured_line);
    clean(&c);
}

#[test]
fn now_line_starts_at_the_highest_value() {
    let day = vec![p(1800, 1.0, 0.5, 2.0), p(3600, 2.0, 1.0, 4.0)];
    let c = day_curve(&day, &[], (0, 3600), 1800, 100.0, 40.0);
    // max 4.4 kW: the 4.0 high sits at 40 - 40 * 4 / 4.4.
    assert!((c.now_top - 3.6364).abs() < 0.01, "{}", c.now_top);
    let c = day_curve(&day, &[(1800, 6.0)], (0, 3600), 3000, 100.0, 40.0);
    assert!(
        (c.now_top - 40.0 * (1.0 - 6.0 / 6.6)).abs() < 0.01,
        "{}",
        c.now_top
    );
}

fn grid_of(peak: f64) -> Vec<f64> {
    let day = vec![
        p(1800, peak / 2.0, 0.0, peak / 2.0),
        p(3600, peak, 0.0, peak),
    ];
    let c = day_curve(&day, &[], (0, 3600), 0, 100.0, 40.0);
    c.grid.iter().map(|g| g.1).collect()
}

#[test]
fn grid_steps_fit_the_day() {
    // Weak day, max about 1.2 kW: 0.5 and 1.0.
    assert_eq!(grid_of(1.1), vec![0.5, 1.0]);
    // Strong day, max about 6.6 kW: at most 4 lines.
    let strong = grid_of(6.0);
    assert!(strong.len() >= 2 && strong.len() <= 4, "{strong:?}");
    assert_eq!(strong, vec![2.0, 4.0, 6.0]);
    // Very weak day: finer steps rather than no line.
    assert_eq!(grid_of(0.3), vec![0.25]);
    // A large installation keeps round steps rather than crowding the bottom.
    assert_eq!(grid_of(60.0), vec![20.0, 40.0, 60.0]);
    for peak in [0.3, 0.9, 2.0, 3.5, 5.0, 9.0, 15.0, 48.0, 120.0, 900.0] {
        let g = grid_of(peak);
        assert!(!g.is_empty() && g.len() <= 4, "{peak}: {g:?}");
        assert!(g.iter().all(|kw| *kw < peak * 1.1));
    }
}

fn at(zone: Tz, y: i32, m: u32, d: u32, h: u32) -> u64 {
    use chrono::TimeZone;
    zone.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap().timestamp() as u64
}

#[test]
fn hour_ticks_are_multiples_of_three_inside_the_span() {
    let zone: Tz = "Europe/Paris".parse().unwrap();
    let span = (at(zone, 2026, 10, 24, 8), at(zone, 2026, 10, 24, 19));
    let ticks = hour_ticks(span, zone, 600.0);
    let hours: Vec<u32> = ticks.iter().map(|t| t.1).collect();
    assert_eq!(hours, vec![9, 12, 15, 18]);
    assert!((ticks[0].0 - 600.0 / 11.0).abs() < 0.01, "{ticks:?}");
    // A tick on the very edge of the span is not strictly inside.
    let edge = (at(zone, 2026, 10, 24, 9), at(zone, 2026, 10, 24, 18));
    let hours: Vec<u32> = hour_ticks(edge, zone, 600.0).iter().map(|t| t.1).collect();
    assert_eq!(hours, vec![12, 15]);
    assert!(hour_ticks((10, 10), zone, 600.0).is_empty());
}

#[test]
fn hour_ticks_follow_local_time_across_dst() {
    let zone: Tz = "Europe/Paris".parse().unwrap();
    // 2026-10-25: clocks go back at 03:00; the day is 25 hours long.
    let span = (at(zone, 2026, 10, 25, 0), at(zone, 2026, 10, 26, 0));
    let hours: Vec<u32> = hour_ticks(span, zone, 600.0).iter().map(|t| t.1).collect();
    assert_eq!(hours, vec![3, 6, 9, 12, 15, 18, 21]);
    let spring = (at(zone, 2026, 3, 29, 6), at(zone, 2026, 3, 29, 20));
    let hours: Vec<u32> = hour_ticks(spring, zone, 600.0)
        .iter()
        .map(|t| t.1)
        .collect();
    assert_eq!(hours, vec![9, 12, 15, 18]);
}

#[test]
fn now_line_inside_span() {
    let day = vec![p(1800, 1.0, 0.5, 2.0), p(3600, 2.0, 1.0, 4.0)];
    let c = day_curve(&day, &[], (0, 3600), 1800, 100.0, 40.0);
    assert_eq!(c.now_x, Some(50.0));
}

#[test]
fn no_now_line_after_last_producing_period() {
    let day = vec![p(1800, 1.0, 0.5, 2.0), p(3600, 2.0, 1.0, 4.0)];
    assert_eq!(
        day_curve(&day, &[], (0, 3600), 4000, 100.0, 40.0).now_x,
        None
    );
    assert_eq!(
        day_curve(&day, &[], (1000, 3600), 500, 100.0, 40.0).now_x,
        None
    );
}

#[test]
fn empty_curve_is_safe() {
    let c = day_curve(&[], &[], (0, 0), 0, 100.0, 40.0);
    assert_eq!(c.forecast_area, "");
    assert_eq!(c.forecast_line, "");
    assert_eq!(c.band, "");
    assert_eq!(c.measured_area, "");
    assert_eq!(c.now_x, None);
    clean(&c);
    let zeros = vec![p(1800, 0.0, 0.0, 0.0)];
    let c = day_curve(&zeros, &[(1800, 0.0)], (0, 1800), 900, 100.0, 40.0);
    clean(&c);
}
