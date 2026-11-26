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
mod tests;
