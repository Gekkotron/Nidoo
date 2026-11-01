//! The solar day curve as SVG path strings: forecast line and fill, the low-high band, the measured
//! line and area, the "now" line, the gridlines and the hour ticks. Pure geometry, no UI.

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use nidoo_api::SolarPeriod;

use crate::components::curve::{finite, pt, smooth_tail};

/// Paths and marks of one day's curve, in a `w` x `h` pixel box.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub forecast_area: String,
    pub forecast_line: String,
    pub band: String,
    /// The measured points as an open line, `M … L …`.
    pub measured_line: String,
    /// The area between the measured line and the bottom of the box.
    pub measured_area: String,
    pub now_x: Option<f64>,
    /// `y px` of the highest plotted value: where the "now" line starts.
    pub now_top: f64,
    /// `(y px, kW)` gridlines at round kW steps, at most 4.
    pub grid: Vec<(f64, f64)>,
}

fn start(p: &SolarPeriod) -> u64 {
    p.end.saturating_sub(p.minutes as u64 * 60)
}

fn producing(p: &SolarPeriod) -> bool {
    p.estimate > 0.0 || p.high > 0.0
}

/// Start of the first producing period to the end of the last one; `None` without production.
pub fn producing_span(day: &[SolarPeriod]) -> Option<(u64, u64)> {
    let first = day.iter().find(|p| producing(p))?;
    let last = day.iter().rfind(|p| producing(p))?;
    Some((start(first), last.end))
}

fn mid(p: &SolarPeriod) -> f64 {
    p.end as f64 - p.minutes as f64 * 30.0
}

/// gentle dip; the ends stay as measured, so the line starts at sunrise and ends on today's value.
fn soften(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = pts.to_vec();
    for i in 1..pts.len() - 1 {
        out[i].1 = (pts[i - 1].1 + 2.0 * pts[i].1 + pts[i + 1].1) / 4.0;
    }
    out
}

/// The kW step between gridlines: the smallest round step that draws at most 4 lines strictly
/// below `max_kw`; 0.25 kW only when no larger step draws any line.
fn grid_step(max_kw: f64) -> f64 {
    let lines = |step: f64| {
        (1..=4)
            .take_while(|k| f64::from(*k) * step < max_kw)
            .count()
    };
    [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0]
        .into_iter()
        .find(|s| (max_kw / s).ceil() <= 5.0 && lines(*s) > 0)
        .or_else(|| (lines(0.25) > 0).then_some(0.25))
        .unwrap_or(500.0)
}

/// `(x px, local hour)` of every house-local whole hour that is a multiple of 3 strictly inside
/// `span`, in a box `w` px wide. Real instants are walked, so a DST change moves no label.
pub fn hour_ticks(span: (u64, u64), zone: Tz, w: f64) -> Vec<(f64, u32)> {
    if span.1 <= span.0 {
        return Vec::new();
    }
    let width = (span.1 - span.0) as f64;
    let mut ticks = Vec::new();
    let mut t = (span.0 / 1800 + 1) * 1800;
    while t < span.1 && ticks.len() < 8 {
        if let Some(utc) = DateTime::<Utc>::from_timestamp(t as i64, 0) {
            let local = utc.with_timezone(&zone);
            if local.minute() == 0 && local.hour() % 3 == 0 {
                ticks.push((finite((t - span.0) as f64 / width * w), local.hour()));
            }
        }
        t += 1800;
    }
    ticks
}

/// The curve of one day's periods over `span`, with the measured points `(period end, kW)`.
pub fn day_curve(
    day: &[SolarPeriod],
    measured: &[(u64, f64)],
    span: (u64, u64),
    now: u64,
    w: f64,
    h: f64,
) -> Curve {
    let mut curve = Curve {
        forecast_area: String::new(),
        forecast_line: String::new(),
        band: String::new(),
        measured_line: String::new(),
        measured_area: String::new(),
        now_x: None,
        now_top: h,
        grid: Vec::new(),
    };
    if span.1 <= span.0 {
        return curve;
    }
    let width = (span.1 - span.0) as f64;
    let x_of = |t: f64| finite((t - span.0 as f64) / width * w).clamp(0.0, w);
    let inside: Vec<&SolarPeriod> = day
        .iter()
        .filter(|p| start(p) >= span.0 && p.end <= span.1)
        .collect();

    let measured_pts: Vec<(f64, f64)> = measured
        .iter()
        .filter_map(|&(end, kw)| {
            let p = inside.iter().find(|p| p.end == end)?;
            if start(p) >= now {
                return None;
            }
            // The period in progress is a few minutes' average: it ends at "now", not at its middle.
            let at = if p.end > now { now as f64 } else { mid(p) };
            Some((x_of(at), kw.max(0.0)))
        })
        .collect();
    let measured_pts = soften(&measured_pts);

    let top = inside
        .iter()
        .flat_map(|p| [p.high, p.estimate])
        .chain(measured_pts.iter().map(|m| m.1))
        .fold(0.0_f64, f64::max);
    let max_kw = if top > 0.0 { top * 1.1 } else { 1.0 };
    let y_of = |kw: f64| finite(h - kw / max_kw * h).clamp(0.0, h);
    curve.now_top = y_of(top);

    if now >= span.0 && now <= span.1 {
        curve.now_x = Some(x_of(now as f64));
    }

    let step = grid_step(max_kw);
    curve.grid = (1..=4)
        .map(|k| f64::from(k) * step)
        .take_while(|kw| *kw < max_kw)
        .map(|kw| (y_of(kw), kw))
        .collect();

    if inside.is_empty() {
        return curve;
    }

    let series = |kw: fn(&SolarPeriod) -> f64| -> Vec<(f64, f64)> {
        inside.iter().map(|p| (x_of(mid(p)), y_of(kw(p)))).collect()
    };
    // A smooth line from its first point, `x y C …`.
    let smooth = |pts: &[(f64, f64)]| format!("{}{}", pt(pts[0].0, pts[0].1), smooth_tail(pts));

    let line = series(|p| p.estimate);
    curve.forecast_line = format!("M {}", smooth(&line));
    curve.forecast_area = format!(
        "M {} L {} L {} Z",
        pt(line[0].0, h),
        smooth(&line),
        pt(line[line.len() - 1].0, h)
    );
    let up = series(|p| p.high);
    let mut down = series(|p| p.low);
    down.reverse();
    curve.band = format!("M {} L {} Z", smooth(&up), smooth(&down));

    if let (Some(first), Some(last)) = (measured_pts.first(), measured_pts.last()) {
        let pts: Vec<(f64, f64)> = measured_pts.iter().map(|m| (m.0, y_of(m.1))).collect();
        curve.measured_line = format!("M {}", smooth(&pts));
        curve.measured_area = format!(
            "M {} L {} L {} Z",
            pt(first.0, h),
            smooth(&pts),
            pt(last.0, h)
        );
    }
    curve
}

#[cfg(test)]
mod tests {
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
}
