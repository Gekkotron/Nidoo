//! SVG viewBox math for the history chart: auto-scale, nice ticks, smooth and step paths, daily
//! bars, and the pointer lookups behind the tooltip.

use crate::components::curve::{pt, smooth_tail, smooth_tail_rev};

/// A y-axis extent (data space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YScale {
    pub min: f64,
    pub max: f64,
}

/// SVG path strings for the step-line chart.
#[derive(Clone, Debug, PartialEq)]
pub struct StepPath {
    /// `d` attribute for the mean step line.
    pub d: String,
    /// `d` attribute for the min/max shaded band (closed polygon).
    pub band_d: String,
}

/// One daily bar in viewBox coordinates, with its data for the tooltip.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub t: i64,
    pub kwh: f64,
}

/// Auto-scale from the min/max band with 10% headroom top and bottom, so the band and the
/// legend's min/max stay inside the axis labels. Falls back to `0..1` on
/// empty input, and expands to a 1-unit range when every value is the same (so a flat line
/// doesn't collapse against a zero-height viewBox).
pub fn y_scale(points: &[(i64, f64, f64, f64)]) -> YScale {
    if points.is_empty() {
        return YScale { min: 0.0, max: 1.0 };
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in points {
        lo = lo.min(p.1).min(p.2);
        hi = hi.max(p.1).max(p.3);
    }
    let range = hi - lo;
    if range == 0.0 {
        return YScale {
            min: lo - 0.5,
            max: hi + 0.5,
        };
    }
    let headroom = range * 0.1;
    YScale {
        min: lo - headroom,
        max: hi + headroom,
    }
}

/// Build the step-line path and its min/max band in viewBox coordinates, with x spanning the
/// window `t_start..t_end` (bucket starts before it clamp to the left edge). Each value is held
/// until the next point, and the last one until `t_end`. Empty input yields empty strings.
pub fn step_path(
    points: &[(i64, f64, f64, f64)],
    scale: YScale,
    t_start: i64,
    t_end: i64,
    view_w: f64,
    view_h: f64,
) -> StepPath {
    if points.is_empty() {
        return StepPath {
            d: String::new(),
            band_d: String::new(),
        };
    }
    let t_span = (t_end - t_start).max(1) as f64;
    let y_span = (scale.max - scale.min).max(f64::EPSILON);
    let x_of = |t: i64| ((t - t_start) as f64 / t_span).clamp(0.0, 1.0) * view_w;
    let y_of = |v: f64| view_h - (v - scale.min) / y_span * view_h;
    let x_end = view_w;

    let (x0, y0) = (x_of(points[0].0), y_of(points[0].1));
    let mut d = format!("M{x0},{y0}");
    for p in &points[1..] {
        let x = x_of(p.0);
        let y = y_of(p.1);
        d.push('H');
        d.push_str(&format!("{x}"));
        d.push('V');
        d.push_str(&format!("{y}"));
    }
    d.push_str(&format!("H{x_end}"));

    // Band: step forward along max values to the window end, then step back along min values.
    let mut band = String::new();
    let ymax0 = y_of(points[0].3);
    band.push_str(&format!("M{x0},{ymax0}"));
    for p in &points[1..] {
        let x = x_of(p.0);
        let y_max = y_of(p.3);
        band.push('H');
        band.push_str(&format!("{x}"));
        band.push('V');
        band.push_str(&format!("{y_max}"));
    }
    let y_min_last = y_of(points[points.len() - 1].2);
    band.push_str(&format!("H{x_end}V{y_min_last}"));
    for i in (1..points.len()).rev() {
        let x = x_of(points[i].0);
        let y_min_prev = y_of(points[i - 1].2);
        band.push('H');
        band.push_str(&format!("{x}"));
        band.push('V');
        band.push_str(&format!("{y_min_prev}"));
    }
    band.push_str(&format!("H{x0}"));
    band.push('Z');

    StepPath { d, band_d: band }
}

/// A y scale rounded out to "nice" tick values, with the ticks it carries.
#[derive(Clone, Debug, PartialEq)]
pub struct NiceScale {
    pub scale: YScale,
    pub ticks: Vec<f64>,
    pub step: f64,
}

/// The most ticks a nice scale draws.
const MAX_TICKS: usize = 6;

/// Round `base` outward to multiples of a 1, 2 or 5 x 10^n step, so the axis carries at most
/// [`MAX_TICKS`] round ticks and still covers `base`. A degenerate or non-finite extent falls
/// back to `0..1`.
pub fn nice_scale(base: YScale) -> NiceScale {
    let range = base.max - base.min;
    if !(range.is_finite() && range > 0.0) {
        return NiceScale {
            scale: YScale { min: 0.0, max: 1.0 },
            ticks: vec![0.0, 1.0],
            step: 1.0,
        };
    }
    let mag = 10f64.powf((range / MAX_TICKS as f64).log10().floor());
    let mut chosen = None;
    'search: for decade in [1.0, 10.0, 100.0] {
        for m in [1.0, 2.0, 5.0] {
            let step = mag * decade * m;
            let lo = (base.min / step + 1e-9).floor();
            let hi = (base.max / step - 1e-9).ceil();
            if ((hi - lo) as usize) < MAX_TICKS {
                chosen = Some((step, lo as i64, hi as i64));
                break 'search;
            }
        }
    }
    let (step, lo, hi) = chosen.unwrap_or((range, 0, 1));
    let d = step_decimals(step);
    let round = |v: f64| {
        let k = 10f64.powi(d as i32);
        (v * k).round() / k
    };
    let ticks: Vec<f64> = (lo..=hi).map(|i| round(i as f64 * step)).collect();
    NiceScale {
        scale: YScale {
            min: ticks[0],
            max: ticks[ticks.len() - 1],
        },
        ticks,
        step,
    }
}

/// How many decimals a tick step needs to read exactly: 0 for 1, 10, 50; 1 for 0.5; 2 for 0.05.
pub fn step_decimals(step: f64) -> usize {
    if !(step.is_finite() && step > 0.0) || step >= 1.0 {
        return 0;
    }
    (-step.log10() - 1e-9).ceil().max(0.0) as usize
}

/// Decimals for a hovered value: the tick step's, at least one below a step of 10.
pub fn value_decimals(step: f64) -> usize {
    if step >= 10.0 {
        0
    } else {
        step_decimals(step).max(1)
    }
}

/// Paths of the smooth chart, in viewBox coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SmoothPath {
    /// The mean line, an open path.
    pub line: String,
    /// The mean line closed down to the bottom edge, for the gradient fill.
    pub area: String,
    /// The min/max band as one closed smooth shape; empty when min equals max everywhere.
    pub band: String,
}

/// Every point as `(x, y)` fractions of the plot, `x` across the window `t_start..t_end` (a bucket
/// before it clamps to the left edge) and `y` down from the top on the mean value.
pub fn plot_points(
    points: &[(i64, f64, f64, f64)],
    scale: YScale,
    t_start: i64,
    t_end: i64,
) -> Vec<(f64, f64)> {
    let t_span = (t_end - t_start).max(1) as f64;
    let y_span = (scale.max - scale.min).max(f64::EPSILON);
    points
        .iter()
        .map(|p| {
            (
                ((p.0 - t_start) as f64 / t_span).clamp(0.0, 1.0),
                (1.0 - (p.1 - scale.min) / y_span).clamp(0.0, 1.0),
            )
        })
        .collect()
}

/// The line through `pts` with the last value held to the right edge, one point per x (the
/// latest of buckets that share it).
fn extended(pts: Vec<(f64, f64)>, w: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pts.len() + 1);
    for p in pts {
        match out.last_mut() {
            Some(last) if last.0 >= p.0 => *last = p,
            _ => out.push(p),
        }
    }
    if let Some(&(x, y)) = out.last()
        && x < w
    {
        out.push((w, y));
    }
    out
}

/// Build the smooth line, its fill area and its min/max band, with x spanning `t_start..t_end`.
/// The line starts at the first point and its last value is held until `t_end`. The curve is a
/// monotone cubic, so it never overshoots the data. Empty input yields empty strings.
pub fn smooth_path(
    points: &[(i64, f64, f64, f64)],
    scale: YScale,
    t_start: i64,
    t_end: i64,
    view_w: f64,
    view_h: f64,
) -> SmoothPath {
    let empty = SmoothPath {
        line: String::new(),
        area: String::new(),
        band: String::new(),
    };
    if points.is_empty() {
        return empty;
    }
    let y_span = (scale.max - scale.min).max(f64::EPSILON);
    let y_of = |v: f64| ((1.0 - (v - scale.min) / y_span) * view_h).clamp(0.0, view_h);
    let to_px = |(x, y): (f64, f64)| (x * view_w, y * view_h);
    let mean = extended(
        plot_points(points, scale, t_start, t_end)
            .into_iter()
            .map(to_px)
            .collect(),
        view_w,
    );
    let first = mean[0];
    let line = format!("M{}{}", pt(first.0, first.1), smooth_tail(&mean));
    let last = mean[mean.len() - 1];
    let area = format!("{line} L{} L{} Z", pt(last.0, view_h), pt(first.0, view_h));

    let flat = points.iter().all(|p| (p.3 - p.2).abs() < f64::EPSILON);
    let band = if flat {
        String::new()
    } else {
        let edge = |pick: fn(&(i64, f64, f64, f64)) -> f64| {
            let xs = plot_points(points, scale, t_start, t_end);
            let raw = points
                .iter()
                .zip(xs)
                .map(|(p, (x, _))| (x * view_w, y_of(pick(p))))
                .collect();
            extended(raw, view_w)
        };
        let upper = edge(|p| p.3);
        let lower = edge(|p| p.2);
        format!(
            "M{}{} L{}{} Z",
            pt(upper[0].0, upper[0].1),
            smooth_tail(&upper),
            pt(lower[lower.len() - 1].0, lower[lower.len() - 1].1),
            smooth_tail_rev(&lower)
        )
    };
    SmoothPath { line, area, band }
}

/// The index of the x in `xs` (ascending) nearest to `x`; the later one on a tie.
pub fn nearest_index(xs: &[f64], x: f64) -> Option<usize> {
    xs.iter()
        .enumerate()
        .rev()
        .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
        .map(|(i, _)| i)
}

/// Which of `n` equal slots the x fraction `frac` falls in; `None` for no slots.
pub fn slot_index(n: usize, frac: f64) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(((frac.clamp(0.0, 1.0) * n as f64) as usize).min(n - 1))
}

/// Past this fraction of the chart width the tooltip opens to the left of the crosshair.
const FLIP_AT: f64 = 0.6;

/// Whether the tooltip at x fraction `frac` sits left of the crosshair, so it stays inside.
pub fn tooltip_flips(frac: f64) -> bool {
    frac > FLIP_AT
}

/// Lay bars evenly across `view_w`, scaling heights to the maximum kWh. Each slot carries one
/// bar at 80% of the slot width. Returns an empty `Vec` on empty input.
pub fn daily_bars(days: &[(i64, f64)], view_w: f64, view_h: f64) -> Vec<Bar> {
    if days.is_empty() {
        return Vec::new();
    }
    let slot = view_w / days.len() as f64;
    let max = days
        .iter()
        .map(|d| d.1)
        .fold(0.0f64, f64::max)
        .max(f64::EPSILON);
    let bar_w = slot * 0.8;
    days.iter()
        .enumerate()
        .map(|(i, &(t, kwh))| {
            let h = (kwh / max) * view_h;
            Bar {
                x: i as f64 * slot,
                y: view_h - h,
                w: bar_w,
                h,
                t,
                kwh,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn y_scale_adds_ten_percent_headroom() {
        let p = vec![(0, 20.0, 20.0, 20.0), (1, 30.0, 30.0, 30.0)];
        let s = y_scale(&p);
        assert!((s.min - 19.0).abs() < 1e-6);
        assert!((s.max - 31.0).abs() < 1e-6);
    }

    #[test]
    fn y_scale_covers_the_min_max_band() {
        let p = vec![(0, 20.0, 10.0, 25.0), (1, 22.0, 21.0, 30.0)];
        let s = y_scale(&p);
        assert!((s.min - 8.0).abs() < 1e-6);
        assert!((s.max - 32.0).abs() < 1e-6);
    }

    #[test]
    fn y_scale_falls_back_to_unit_range_on_empty() {
        let s = y_scale(&[]);
        assert_eq!((s.min, s.max), (0.0, 1.0));
    }

    #[test]
    fn y_scale_expands_a_flat_series() {
        let p = vec![(0, 5.0, 5.0, 5.0), (1, 5.0, 5.0, 5.0)];
        let s = y_scale(&p);
        assert!((s.min - 4.5).abs() < 1e-6);
        assert!((s.max - 5.5).abs() < 1e-6);
    }

    #[test]
    fn step_path_produces_horizontal_then_vertical_segments() {
        let p = vec![(0, 0.0, 0.0, 0.0), (10, 1.0, 1.0, 1.0)];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = step_path(&p, s, 0, 10, 100.0, 50.0);
        assert!(out.d.contains("M0,50"));
        assert!(out.d.contains("H") && out.d.contains("V"));
    }

    #[test]
    fn step_path_holds_a_single_point_until_the_window_end() {
        let p = vec![(50, 0.5, 0.0, 1.0)];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = step_path(&p, s, 0, 100, 100.0, 50.0);
        assert_eq!(out.d, "M50,25H100");
        assert_eq!(out.band_d, "M50,0H100V50H50Z");
    }

    #[test]
    fn step_path_places_short_history_at_the_right_of_the_window() {
        let p = vec![(80, 0.0, 0.0, 0.0), (90, 1.0, 1.0, 1.0)];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = step_path(&p, s, 0, 100, 100.0, 50.0);
        assert_eq!(out.d, "M80,50H90V0H100");
    }

    #[test]
    fn step_path_clamps_a_bucket_starting_before_the_window() {
        let p = vec![(-20, 0.0, 0.0, 0.0)];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = step_path(&p, s, 0, 100, 100.0, 50.0);
        assert_eq!(out.d, "M0,50H100");
    }

    #[test]
    fn step_path_band_steps_back_along_min_without_diagonals() {
        let p = vec![(0, 0.5, 0.0, 1.0), (50, 0.5, 0.25, 0.75)];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = step_path(&p, s, 0, 100, 100.0, 100.0);
        assert!(!out.band_d.contains('L'));
        assert_eq!(out.band_d, "M0,0H50V25H100V75H50V100H0Z");
    }

    #[test]
    fn step_path_empty_input_is_empty() {
        let out = step_path(&[], YScale { min: 0.0, max: 1.0 }, 0, 100, 100.0, 50.0);
        assert_eq!(out.d, "");
        assert_eq!(out.band_d, "");
    }

    #[test]
    fn nice_scale_picks_round_ticks_covering_the_data() {
        let n = nice_scale(YScale {
            min: 19.0,
            max: 31.0,
        });
        assert_eq!(n.ticks, vec![15.0, 20.0, 25.0, 30.0, 35.0]);
        assert_eq!((n.scale.min, n.scale.max), (15.0, 35.0));
        assert_eq!(n.step, 5.0);
    }

    #[test]
    fn nice_scale_hugs_the_data_with_even_steps() {
        let n = nice_scale(YScale {
            min: 6.5,
            max: 12.0,
        });
        assert_eq!(n.ticks, vec![6.0, 8.0, 10.0, 12.0]);
    }

    #[test]
    fn nice_scale_keeps_three_to_six_ticks_whatever_the_magnitude() {
        for (lo, hi) in [
            (0.0, 1.0),
            (0.3, 0.7),
            (-3.0, 8.0),
            (120.0, 4_700.0),
            (4.5, 5.5),
        ] {
            let n = nice_scale(YScale { min: lo, max: hi });
            assert!(
                (3..=6).contains(&n.ticks.len()),
                "{lo}..{hi}: {:?}",
                n.ticks
            );
            assert!(n.scale.min <= lo && n.scale.max >= hi, "{lo}..{hi}");
            assert!(n.ticks.windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn nice_scale_ticks_carry_no_float_noise() {
        let n = nice_scale(YScale {
            min: 0.05,
            max: 0.65,
        });
        assert_eq!(n.ticks, vec![0.0, 0.2, 0.4, 0.6, 0.8]);
    }

    #[test]
    fn nice_scale_falls_back_on_a_degenerate_extent() {
        let n = nice_scale(YScale { min: 3.0, max: 3.0 });
        assert_eq!((n.scale.min, n.scale.max), (0.0, 1.0));
    }

    #[test]
    fn step_decimals_follow_the_step() {
        assert_eq!(step_decimals(10.0), 0);
        assert_eq!(step_decimals(1.0), 0);
        assert_eq!(step_decimals(0.5), 1);
        assert_eq!(step_decimals(0.2), 1);
        assert_eq!(step_decimals(0.05), 2);
        assert_eq!(value_decimals(50.0), 0);
        assert_eq!(value_decimals(5.0), 1);
        assert_eq!(value_decimals(0.05), 2);
    }

    fn numbers(s: &str) -> Vec<f64> {
        s.replace(['M', 'L', 'C', 'Z'], " ")
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect()
    }

    #[test]
    fn smooth_path_goes_through_every_point_and_on_to_the_window_end() {
        let p = vec![
            (20, 0.0, 0.0, 0.0),
            (50, 1.0, 1.0, 1.0),
            (70, 0.5, 0.5, 0.5),
        ];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = smooth_path(&p, s, 0, 100, 100.0, 50.0);
        assert!(out.line.starts_with("M20.0 50.0"), "{}", out.line);
        assert!(out.line.contains("50.0 0.0"), "{}", out.line);
        assert!(out.line.contains("70.0 25.0"), "{}", out.line);
        assert!(out.line.ends_with("100.0 25.0"), "{}", out.line);
        assert!(!out.line.contains('H') && !out.line.contains('V'));
    }

    #[test]
    fn smooth_path_never_leaves_the_data_range() {
        let p = vec![
            (0, 40.0, 40.0, 40.0),
            (10, 40.0, 40.0, 40.0),
            (20, 5.0, 5.0, 5.0),
            (30, 5.0, 5.0, 5.0),
            (40, 20.0, 20.0, 20.0),
        ];
        let s = YScale {
            min: 0.0,
            max: 40.0,
        };
        let out = smooth_path(&p, s, 0, 40, 40.0, 40.0);
        // y grows downwards: the value 40 is y 0, the value 5 is y 35. No control point leaves 0..=35.
        for y in numbers(&out.line).chunks(2).map(|c| c[1]) {
            assert!((-0.05..=35.05).contains(&y), "{y}: {}", out.line);
        }
    }

    #[test]
    fn smooth_path_area_closes_down_to_the_bottom_edge() {
        let p = vec![(0, 1.0, 1.0, 1.0), (50, 0.0, 0.0, 0.0)];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = smooth_path(&p, s, 0, 100, 100.0, 50.0);
        assert!(out.area.starts_with(&out.line));
        assert!(
            out.area.ends_with("L100.0 50.0 L0.0 50.0 Z"),
            "{}",
            out.area
        );
    }

    #[test]
    fn smooth_path_band_is_one_closed_shape_without_stray_segments() {
        let p = vec![
            (0, 0.5, 0.25, 0.75),
            (30, 0.5, 0.5, 0.5),
            (60, 0.6, 0.4, 0.9),
        ];
        let s = YScale { min: 0.0, max: 1.0 };
        let out = smooth_path(&p, s, 0, 100, 100.0, 100.0);
        assert_eq!(out.band.matches('M').count(), 1, "{}", out.band);
        assert_eq!(out.band.matches('Z').count(), 1, "{}", out.band);
        assert!(!out.band.contains('H') && !out.band.contains('V'));
        assert!(out.band.starts_with("M0.0 25.0"), "{}", out.band);
        // It comes back along the lower edge to the start, and stays inside the plot.
        assert!(
            out.band.trim_end_matches(" Z").ends_with("0.0 75.0"),
            "{}",
            out.band
        );
        assert!(
            numbers(&out.band)
                .iter()
                .all(|v| (-0.05..=100.05).contains(v))
        );
    }

    #[test]
    fn smooth_path_hides_the_band_when_min_equals_max() {
        let p = vec![(0, 0.5, 0.5, 0.5), (50, 0.7, 0.7, 0.7)];
        let out = smooth_path(&p, YScale { min: 0.0, max: 1.0 }, 0, 100, 100.0, 100.0);
        assert_eq!(out.band, "");
        assert!(!out.line.is_empty());
    }

    #[test]
    fn smooth_path_handles_one_point_and_none() {
        let s = YScale { min: 0.0, max: 1.0 };
        let one = smooth_path(&[(50, 0.5, 0.5, 0.5)], s, 0, 100, 100.0, 50.0);
        assert!(one.line.starts_with("M50.0 25.0") && one.line.ends_with("100.0 25.0"));
        let none = smooth_path(&[], s, 0, 100, 100.0, 50.0);
        assert_eq!(
            (none.line.as_str(), none.area.as_str(), none.band.as_str()),
            ("", "", "")
        );
    }

    #[test]
    fn smooth_path_survives_buckets_sharing_the_left_edge() {
        let p = vec![
            (-20, 0.0, 0.0, 0.0),
            (-10, 1.0, 1.0, 1.0),
            (50, 0.5, 0.5, 0.5),
        ];
        let out = smooth_path(&p, YScale { min: 0.0, max: 1.0 }, 0, 100, 100.0, 50.0);
        assert!(
            !out.line.contains("NaN") && !out.line.contains("inf"),
            "{}",
            out.line
        );
        assert!(out.line.starts_with("M0.0 0.0"), "{}", out.line);
    }

    #[test]
    fn nearest_index_finds_the_closest_point_by_x() {
        let xs = [0.1, 0.4, 0.5, 0.9];
        assert_eq!(nearest_index(&xs, 0.0), Some(0));
        assert_eq!(nearest_index(&xs, 0.3), Some(1));
        assert_eq!(nearest_index(&xs, 0.46), Some(2));
        assert_eq!(nearest_index(&xs, 0.75), Some(3));
        assert_eq!(nearest_index(&xs, 5.0), Some(3));
        assert_eq!(nearest_index(&[], 0.5), None);
    }

    #[test]
    fn nearest_index_prefers_the_later_point_on_a_tie_and_on_a_shared_edge() {
        assert_eq!(nearest_index(&[0.25, 0.75], 0.5), Some(1));
        assert_eq!(nearest_index(&[0.0, 0.0, 0.5], 0.0), Some(1));
    }

    #[test]
    fn slot_index_maps_a_fraction_to_a_bar() {
        assert_eq!(slot_index(4, 0.0), Some(0));
        assert_eq!(slot_index(4, 0.26), Some(1));
        assert_eq!(slot_index(4, 1.0), Some(3));
        assert_eq!(slot_index(4, -1.0), Some(0));
        assert_eq!(slot_index(0, 0.5), None);
    }

    #[test]
    fn the_tooltip_flips_left_in_the_right_part_of_the_chart() {
        assert!(!tooltip_flips(0.0));
        assert!(!tooltip_flips(0.6));
        assert!(tooltip_flips(0.61));
        assert!(tooltip_flips(1.0));
    }

    #[test]
    fn daily_bars_lay_out_evenly_and_scale_to_max() {
        let days = vec![(0, 2.0), (86_400, 4.0), (172_800, 1.0)];
        let bars = daily_bars(&days, 300.0, 100.0);
        assert_eq!(bars.len(), 3);
        let w_each = 100.0;
        assert!((bars[0].x - 0.0).abs() < 1e-6);
        assert!((bars[1].x - 100.0).abs() < 1e-6);
        assert!((bars[1].h - 100.0).abs() < 1e-6);
        assert!((bars[0].h - 50.0).abs() < 1e-6);
        assert!((bars[0].w - w_each * 0.8).abs() < w_each * 0.21);
    }

    #[test]
    fn daily_bars_empty_input_is_empty() {
        assert!(daily_bars(&[], 300.0, 100.0).is_empty());
    }
}
