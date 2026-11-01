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
mod tests {
    use super::*;
    use crate::energy_sankey::{FOLD_WIDE, build};
    use nidoo_core::energy::{EnergyNode, EnergyUi, Flow, NodeKind, NodeStatus};

    fn node(id: &str, kind: NodeKind, watts: f64, flow: Flow) -> EnergyNode {
        EnergyNode {
            id: id.into(),
            kind,
            name: None,
            value: Some(watts),
            unit: None,
            flow,
            status: NodeStatus::Live,
            count: 0,
            sensor_ids: Vec::new(),
            icon: None,
            total_kwh: None,
            total_status: NodeStatus::Live,
            has_meter: false,
            water_total: None,
            water_total_unit: None,
        }
    }

    fn consumer(name: &str, watts: f64) -> EnergyNode {
        EnergyNode {
            name: Some(name.into()),
            ..node(
                &format!("consumer:{}", name.to_lowercase().replace(' ', "-")),
                NodeKind::Consumer,
                watts,
                Flow::In,
            )
        }
    }

    fn ui(nodes: &[EnergyNode], consumers: &[EnergyNode]) -> EnergyUi {
        EnergyUi {
            diagram: nodes.to_vec(),
            cards: consumers.to_vec(),
            consumers: consumers.to_vec(),
            ..EnergyUi::default()
        }
    }

    fn ids(v: &[Arc]) -> Vec<&str> {
        v.iter().map(|a| a.id.as_str()).collect()
    }

    fn balanced() -> Sankey {
        build(
            &ui(
                &[
                    node("solar", NodeKind::Solar, 2140.0, Flow::In),
                    node("grid", NodeKind::Grid, 380.0, Flow::In),
                    node("battery", NodeKind::Battery, 260.0, Flow::Out),
                    node("house", NodeKind::House, 2260.0, Flow::In),
                ],
                &[
                    consumer("Heater", 1200.0),
                    consumer("Oven", 540.0),
                    consumer("Fridge", 210.0),
                ],
            ),
            FOLD_WIDE,
        )
    }

    #[test]
    fn balanced_rings_line_up() {
        let r = rings(&balanced());
        assert_eq!(ids(&r.inner), ["solar", "grid"]);
        assert_eq!(r.outer[0].id, "battery-charge");
        assert!(r.outer[1].id.starts_with("consumer:"));
        assert_eq!(r.outer.last().unwrap().id, "unitemised");
        let end = 360.0 - GAP_DEG / 2.0;
        assert!((r.inner.last().unwrap().a1 - end).abs() < 0.01);
        assert!((r.outer.last().unwrap().a1 - end).abs() < 0.01);
    }

    #[test]
    fn export_sits_first_and_in_grid_colour() {
        let s = build(
            &ui(
                &[
                    node("solar", NodeKind::Solar, 3000.0, Flow::In),
                    node("grid", NodeKind::Grid, 1200.0, Flow::Out),
                    node("house", NodeKind::House, 1800.0, Flow::In),
                ],
                &[consumer("Heater", 900.0)],
            ),
            FOLD_WIDE,
        );
        let r = rings(&s);
        assert_eq!(r.outer[0].id, "export");
        assert_eq!(colour_var(r.outer[0].role, 0), "--energy-grid");
    }

    #[test]
    fn meters_disagree_the_short_ring_stops_short() {
        let s = build(
            &ui(
                &[
                    node("solar", NodeKind::Solar, 500.0, Flow::In),
                    node("house", NodeKind::House, 2000.0, Flow::In),
                ],
                &[consumer("Heater", 300.0)],
            ),
            FOLD_WIDE,
        );
        let r = rings(&s);
        assert!(r.inner.last().unwrap().a1 < 350.0, "{:?}", r.inner);
        assert!((r.outer.last().unwrap().a1 - (360.0 - GAP_DEG / 2.0)).abs() < 0.01);
        assert!(r.inner.iter().chain(&r.outer).all(|a| a.a1 <= 360.0));
    }

    #[test]
    fn zero_size_nodes_give_no_arc() {
        let s = build(
            &ui(
                &[
                    node("solar", NodeKind::Solar, 800.0, Flow::In),
                    node("grid", NodeKind::Grid, 0.0, Flow::In),
                    node("house", NodeKind::House, 800.0, Flow::In),
                ],
                &[],
            ),
            FOLD_WIDE,
        );
        let r = rings(&s);
        assert_eq!(ids(&r.inner), ["solar"]);
        assert!(r.inner.iter().chain(&r.outer).all(|a| a.size > 0.0));
    }

    #[test]
    fn empty_sankey_gives_no_arcs() {
        let r = rings(&Sankey::default());
        assert!(r.inner.is_empty() && r.outer.is_empty());
        assert!(r.total.is_finite());
    }

    #[test]
    fn full_ring_path_is_drawn() {
        let p = arc_path(170.0, 170.0, 104.0, 0.0, 360.0);
        assert_eq!(p.matches('A').count(), 2);
        assert!(!p.contains("NaN"));
    }

    #[test]
    fn dots_stay_inside_their_arc() {
        let r = rings(&balanced());
        let mut t: f64 = 0.0;
        while t < 10.0 {
            for a in r.inner.iter().chain(&r.outer) {
                for (deg, op) in arc_dots(a, t.rem_euclid(1.0)) {
                    assert!(deg >= a.a0 - 1e-9 && deg <= a.a1 + 1e-9, "{deg} in {a:?}");
                    assert!((0.0..=1.0).contains(&op));
                }
            }
            t += 0.37;
        }
        let un = r.outer.iter().find(|a| a.id == "unitemised").unwrap();
        assert!(arc_dots(un, 1.0).is_empty());
        let short = Arc {
            a0: 10.0,
            a1: 15.0,
            ..r.inner[0].clone()
        };
        assert!(arc_dots(&short, 1.0).is_empty());
    }

    #[test]
    fn a_new_reading_changes_the_speed_not_the_place() {
        let r = rings(&balanced());
        let a = r.outer[1].clone();
        let phase = advance_phase(0.0, 0.05, &a);
        let busier = Arc {
            watts: a.watts * 3.0,
            a0: a.a0 + 2.0,
            a1: a.a1 + 8.0,
            ..a.clone()
        };
        // Same phase, so the dots stay at the same share of their arc.
        let frac = |arc: &Arc| {
            arc_dots(arc, phase)
                .iter()
                .map(|(deg, _)| (deg - arc.a0) / (arc.a1 - arc.a0))
                .collect::<Vec<_>>()
        };
        for (x, y) in frac(&a).iter().zip(frac(&busier)) {
            assert!((x - y).abs() < 1e-9);
        }
        // More power moves the phase faster; a long pause moves it a bounded step.
        assert!(advance_phase(0.0, 0.05, &busier) > 0.0);
        assert!(advance_phase(0.0, 60.0, &a) <= advance_phase(0.0, 0.1, &a) + 1e-12);
    }

    #[test]
    fn night_grid_only_fills_both_rings() {
        let s = build(
            &ui(
                &[
                    node("grid", NodeKind::Grid, 300.0, Flow::In),
                    node("house", NodeKind::House, 300.0, Flow::In),
                ],
                &[],
            ),
            FOLD_WIDE,
        );
        let r = rings(&s);
        assert_eq!(ids(&r.inner), ["grid"]);
        assert_eq!(ids(&r.outer), ["unitemised"]);
        for a in r.inner.iter().chain(&r.outer) {
            assert_eq!((a.a0, a.a1), (0.0, 360.0));
        }
    }

    #[test]
    fn consumer_colours_match_flux_slots() {
        assert_eq!(colour_var(SankeyRole::Consumer, 6), "--energy-c1");
    }
}
