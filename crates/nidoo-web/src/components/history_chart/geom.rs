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
    step_path_held(points, scale, t_start, t_end, view_w, view_w, view_h)
}

/// [`step_path`] with the last value held only as far as `hold_px` (a running period stops at
/// its now line instead of running on into the days to come).
pub fn step_path_held(
    points: &[(i64, f64, f64, f64)],
    scale: YScale,
    t_start: i64,
    t_end: i64,
    view_w: f64,
    hold_px: f64,
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
    let x_end = hold_px;

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
    smooth_path_held(points, scale, t_start, t_end, view_w, view_w, view_h)
}

/// [`smooth_path`] with the last value held only as far as `hold_px`.
pub fn smooth_path_held(
    points: &[(i64, f64, f64, f64)],
    scale: YScale,
    t_start: i64,
    t_end: i64,
    view_w: f64,
    hold_px: f64,
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
        hold_px,
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
            extended(raw, hold_px)
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
mod tests;
