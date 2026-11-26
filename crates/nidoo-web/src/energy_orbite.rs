//! Geometry of the Orbite energy view: two concentric rings of arcs (inner = where power comes
//! from, outer = where it goes) and the dots that drift along them. Pure Rust, no DOM, so it is
//! unit-tested natively.

use crate::energy_sankey::{Sankey, SankeyNode, SankeyRole};

/// Empty angle between two neighbouring arcs, in degrees.
pub const GAP_DEG: f64 = 1.6;
/// Arcs spanning less than this carry no dots.
pub const MIN_DOT_ARC_DEG: f64 = 6.0;
/// An arc is never drawn narrower than this, in degrees.
const MIN_SPAN_DEG: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ring {
    Inner,
    Outer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arc {
    /// The `SankeyNode` id.
    pub id: String,
    pub ring: Ring,
    pub role: SankeyRole,
    /// Consumer colour slot.
    pub slot: usize,
    pub name: Option<String>,
    /// Measured, for labels.
    pub watts: f64,
    /// Drawn, from `SankeyNode::size`.
    pub size: f64,
    /// Percent of its own ring's sum, rounded.
    pub share: u8,
    /// Degrees, 0 = top, clockwise, gaps applied.
    pub a0: f64,
    pub a1: f64,
    pub sensor: Option<String>,
    /// Others only: how many consumers it folds.
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rings {
    pub inner: Vec<Arc>,
    pub outer: Vec<Arc>,
    pub total: f64,
}

/// The nodes of one ring, in drawing order, without the empty ones.
fn pick<'a>(s: &'a Sankey, roles: &[SankeyRole]) -> Vec<&'a SankeyNode> {
    roles
        .iter()
        .flat_map(|r| s.nodes.iter().filter(move |n| n.role == *r))
        .filter(|n| n.size > 0.0)
        .collect()
}

fn lay_out(nodes: &[&SankeyNode], ring: Ring, total: f64) -> Vec<Arc> {
    let sum: f64 = nodes.iter().map(|n| n.size).sum();
    let single_full = nodes.len() == 1 && nodes[0].size >= total;
    let mut start = 0.0;
    nodes
        .iter()
        .map(|n| {
            let span = n.size / total * 360.0;
            let (a0, a1) = if single_full {
                (0.0, 360.0)
            } else if span - GAP_DEG < MIN_SPAN_DEG {
                let mid = start + span / 2.0;
                (mid - MIN_SPAN_DEG / 2.0, mid + MIN_SPAN_DEG / 2.0)
            } else {
                (start + GAP_DEG / 2.0, start + span - GAP_DEG / 2.0)
            };
            start += span;
            Arc {
                id: n.id.clone(),
                ring,
                role: n.role,
                slot: n.slot,
                name: n.name.clone(),
                watts: n.watts,
                size: n.size,
                share: (n.size / sum * 100.0).round() as u8,
                a0,
                a1: a1.min(360.0),
                sensor: n.sensor.clone(),
                count: n.count,
            }
        })
        .collect()
}

/// The two rings of a Sankey; both are scaled to the larger of the two sums so arcs line up.
pub fn rings(s: &Sankey) -> Rings {
    use SankeyRole::*;
    let inner = pick(s, &[Solar, Battery, Grid]);
    let outer = pick(s, &[Export, BatteryCharge, Consumer, Others, Unitemised]);
    let sum = |v: &[&SankeyNode]| v.iter().map(|n| n.size).sum::<f64>();
    let total = sum(&inner).max(sum(&outer));
    if total <= 0.0 || !total.is_finite() {
        return Rings::default();
    }
    Rings {
        inner: lay_out(&inner, Ring::Inner, total),
        outer: lay_out(&outer, Ring::Outer, total),
        total,
    }
}

/// The CSS variable (shared energy palette) an arc is painted with.
pub fn colour_var(role: SankeyRole, slot: usize) -> &'static str {
    match role {
        SankeyRole::Solar => "--energy-solar",
        SankeyRole::Grid | SankeyRole::Export => "--energy-grid",
        SankeyRole::Battery | SankeyRole::BatteryCharge => "--energy-battery",
        SankeyRole::Consumer => [
            "--energy-c0",
            "--energy-c1",
            "--energy-c2",
            "--energy-c3",
            "--energy-c4",
        ][slot % 5],
        SankeyRole::House | SankeyRole::Others | SankeyRole::Unitemised => "--energy-muted",
    }
}

/// The point at `deg` (0 = top, clockwise) on the circle of radius `r`.
pub fn point(cx: f64, cy: f64, r: f64, deg: f64) -> (f64, f64) {
    let a = deg.to_radians();
    (cx + r * a.sin(), cy - r * a.cos())
}

/// An SVG path for the arc from `a0` to `a1`; a full circle becomes two half-arcs.
pub fn arc_path(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> String {
    let span = a1 - a0;
    let (x0, y0) = point(cx, cy, r, a0);
    if span >= 359.99 {
        let (x1, y1) = point(cx, cy, r, a0 + 180.0);
        return format!(
            "M{x0:.2} {y0:.2}A{r} {r} 0 1 1 {x1:.2} {y1:.2}A{r} {r} 0 1 1 {x0:.2} {y0:.2}"
        );
    }
    let (x1, y1) = point(cx, cy, r, a1);
    let large = u8::from(span > 180.0);
    format!("M{x0:.2} {y0:.2}A{r} {r} 0 {large} 1 {x1:.2} {y1:.2}")
}

/// Degrees per second a dot drifts at: faster for more power.
pub fn dot_speed(watts: f64) -> f64 {
    10.0 + 0.9 * watts.max(0.0).sqrt()
}

/// `phase` (a turn of the arc, 0..1) after `dt_s` seconds on `arc`. The phase is kept per arc, so
/// a new reading changes how fast the dots go, never where they are.
pub fn advance_phase(phase: f64, dt_s: f64, arc: &Arc) -> f64 {
    let span = arc.a1 - arc.a0;
    if span <= 0.0 {
        return phase;
    }
    (phase + dt_s.clamp(0.0, 0.1) * dot_speed(arc.watts) / span).rem_euclid(1.0)
}

/// The dots on an arc at `phase`: (angle in degrees, opacity 0..=1). Dots fade in and out at
/// the arc's ends.
pub fn arc_dots(arc: &Arc, phase: f64) -> Vec<(f64, f64)> {
    let span = arc.a1 - arc.a0;
    if matches!(arc.role, SankeyRole::Others | SankeyRole::Unitemised) || span < MIN_DOT_ARC_DEG {
        return Vec::new();
    }
    let n = ((span / 42.0).round() as usize).max(1);
    (0..n)
        .map(|j| {
            let f = (phase + j as f64 / n as f64).rem_euclid(1.0);
            let opacity = (f.min(1.0 - f) * span / 6.0).clamp(0.0, 1.0);
            (arc.a0 + f * span, opacity)
        })
        .collect()
}

#[cfg(test)]
mod tests;
