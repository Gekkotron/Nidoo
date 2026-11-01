//! Geometry of the "glowing circles" Classic energy diagram: circle layouts, node placement and
//! the quadratic curves of the wires. Pure Rust, no DOM, so it is unit-tested natively.

use std::collections::BTreeMap;

use nidoo_core::energy::{EnergyNode, Flow, IDLE_W, NodeKind, NodeStatus};

use crate::energy::level;
use crate::energy_sankey::Sankey;

/// One viewBox's constants. Positions are in viewBox units (px).
#[derive(Debug, PartialEq)]
pub struct CirclesLayout {
    pub view_w: f64,
    pub view_h: f64,
    pub r_big: f64,
    pub r_house: f64,
    pub r_small: f64,
    pub house: (f64, f64),
    /// Solar / grid without a battery or water.
    pub solar: (f64, f64),
    pub grid: (f64, f64),
    /// Solar / grid when the side slot is taken.
    pub solar_side: (f64, f64),
    pub grid_side: (f64, f64),
    /// Battery or water.
    pub side: (f64, f64),
    pub one: [(f64, f64); 1],
    pub two: [(f64, f64); 2],
    pub three: [(f64, f64); 3],
    pub bend_solar: f64,
    pub bend_grid: f64,
    pub bend_battery: f64,
    pub bend_three: [f64; 3],
}

impl CirclesLayout {
    pub const WIDE: CirclesLayout = CirclesLayout {
        view_w: 800.0,
        view_h: 410.0,
        r_big: 46.0,
        r_house: 54.0,
        r_small: 30.0,
        house: (400.0, 214.0),
        solar: (260.0, 74.0),
        grid: (96.0, 214.0),
        solar_side: (260.0, 74.0),
        grid_side: (96.0, 214.0),
        side: (190.0, 330.0),
        one: [(650.0, 214.0)],
        two: [(640.0, 140.0), (640.0, 288.0)],
        three: [(630.0, 80.0), (655.0, 214.0), (630.0, 348.0)],
        bend_solar: 26.0,
        bend_grid: -22.0,
        bend_battery: 0.0,
        bend_three: [-18.0, 0.0, 18.0],
    };

    pub const NARROW: CirclesLayout = CirclesLayout {
        view_w: 360.0,
        view_h: 484.0,
        r_big: 44.0,
        r_house: 54.0,
        r_small: 30.0,
        house: (180.0, 240.0),
        solar: (84.0, 74.0),
        grid: (276.0, 74.0),
        solar_side: (60.0, 74.0),
        grid_side: (300.0, 74.0),
        side: (180.0, 66.0),
        one: [(180.0, 404.0)],
        two: [(110.0, 404.0), (250.0, 404.0)],
        three: [(56.0, 400.0), (180.0, 412.0), (304.0, 400.0)],
        bend_solar: 22.0,
        bend_grid: -22.0,
        bend_battery: 0.0,
        bend_three: [16.0, 0.0, -16.0],
    };
}

pub fn for_width(narrow: bool) -> &'static CirclesLayout {
    if narrow {
        &CirclesLayout::NARROW
    } else {
        &CirclesLayout::WIDE
    }
}

/// A placed circle: its centre and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub at: (f64, f64),
    pub r: f64,
}

/// Places every node of the diagram, keyed by node id.
pub fn place(diagram: &[EnergyNode], layout: &CirclesLayout) -> BTreeMap<String, Placed> {
    let side_taken = diagram
        .iter()
        .any(|n| matches!(n.kind, NodeKind::Battery | NodeKind::Water));
    let slots = diagram
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Consumer | NodeKind::More))
        .count();
    let mut arc = 0usize;
    let mut out = BTreeMap::new();
    for n in diagram {
        let (at, r) = match n.kind {
            NodeKind::Solar => (
                if side_taken {
                    layout.solar_side
                } else {
                    layout.solar
                },
                layout.r_big,
            ),
            NodeKind::Grid => (
                if side_taken {
                    layout.grid_side
                } else {
                    layout.grid
                },
                layout.r_big,
            ),
            NodeKind::Battery => (layout.side, layout.r_big),
            NodeKind::Water => (layout.side, layout.r_small),
            NodeKind::House => (layout.house, layout.r_house),
            NodeKind::Consumer | NodeKind::More => {
                let i = arc;
                arc += 1;
                let at = match slots {
                    0 | 1 => layout.one[0],
                    2 => layout.two[i.min(1)],
                    _ => layout.three[i.min(2)],
                };
                (at, layout.r_small)
            }
        };
        out.insert(n.id.clone(), Placed { at, r });
    }
    out
}

/// A quadratic bezier wire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub from: (f64, f64),
    pub ctrl: (f64, f64),
    pub to: (f64, f64),
}

impl Curve {
    /// SVG path data, `M x y Q cx cy x y`.
    pub fn path(&self) -> String {
        format!(
            "M {} {} Q {} {} {} {}",
            self.from.0, self.from.1, self.ctrl.0, self.ctrl.1, self.to.0, self.to.1
        )
    }

    /// The point at `t` in 0..=1.
    pub fn point(&self, t: f64) -> (f64, f64) {
        let u = 1.0 - t;
        let (a, b, c) = (u * u, 2.0 * u * t, t * t);
        (
            a * self.from.0 + b * self.ctrl.0 + c * self.to.0,
            a * self.from.1 + b * self.ctrl.1 + c * self.to.1,
        )
    }

    /// Length, as a 16-segment polyline.
    pub fn length(&self) -> f64 {
        let mut prev = self.point(0.0);
        let mut sum = 0.0;
        for i in 1..=16 {
            let p = self.point(i as f64 / 16.0);
            sum += (p.0 - prev.0).hypot(p.1 - prev.1);
            prev = p;
        }
        sum
    }

    /// The same wire run the other way.
    pub fn reversed(&self) -> Curve {
        Curve {
            from: self.to,
            ctrl: self.ctrl,
            to: self.from,
        }
    }
}

/// The wire from a node's rim to the house's rim along the centre line; the control point is the
/// chord midpoint pushed `bend` px along the left normal of node -> house. Coincident centres
/// give a finite curve (no direction), never `NaN`.
pub fn rim_curve(node: Placed, house: Placed, bend: f64) -> Curve {
    let (dx, dy) = (house.at.0 - node.at.0, house.at.1 - node.at.1);
    let len = dx.hypot(dy);
    let (ux, uy) = if len > 1e-9 {
        (dx / len, dy / len)
    } else {
        (0.0, 0.0)
    };
    let from = (node.at.0 + ux * node.r, node.at.1 + uy * node.r);
    let to = (house.at.0 - ux * house.r, house.at.1 - uy * house.r);
    // Left normal on screen (y down) of the direction (ux, uy).
    let (nx, ny) = (uy, -ux);
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    Curve {
        from,
        ctrl: (mid.0 + nx * bend, mid.1 + ny * bend),
        to,
    }
}

/// The bend of a node's wire. `slot` is the consumer's index in the arc of `slots` consumers.
pub fn bend_for(
    layout: &CirclesLayout,
    node: &EnergyNode,
    slot: Option<usize>,
    slots: usize,
) -> f64 {
    match node.kind {
        NodeKind::Solar => layout.bend_solar,
        NodeKind::Grid => layout.bend_grid,
        NodeKind::Battery => layout.bend_battery,
        NodeKind::Consumer | NodeKind::More if slots == 3 => slot
            .and_then(|i| layout.bend_three.get(i).copied())
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

/// A node id as an SVG id fragment.
pub(crate) fn sanitise(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The colour tone of a node; a consumer's is `c{slot}`.
pub fn tone(node: &EnergyNode, slot: Option<usize>) -> &'static str {
    match node.kind {
        NodeKind::Solar => "solar",
        NodeKind::Grid if node.flow == Flow::Out => "export",
        NodeKind::Grid => "grid",
        NodeKind::Battery => "battery",
        NodeKind::House => "house",
        NodeKind::More => "more",
        NodeKind::Water => "water",
        NodeKind::Consumer => match slot {
            Some(0) => "c0",
            Some(1) => "c1",
            _ => "c2",
        },
    }
}

/// The 0-based index of a consumer among the `Consumer` nodes, in diagram order.
pub fn consumer_slot(diagram: &[EnergyNode], id: &str) -> Option<usize> {
    diagram
        .iter()
        .filter(|n| n.kind == NodeKind::Consumer)
        .position(|n| n.id == id)
}

/// One drawn wire; `from` / `to` are node ids in flow direction.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: NodeKind,
    pub tone: &'static str,
    pub from_tone: &'static str,
    pub to_tone: &'static str,
    pub curve: Curve,
    pub watts: f64,
    pub active: bool,
    pub width: f64,
}

/// Whether a node's wire carries power: live, at least `IDLE_W`, and not idle.
fn is_active(n: &EnergyNode) -> bool {
    n.status == NodeStatus::Live && n.flow != Flow::Idle && n.value.is_some_and(|w| w >= IDLE_W)
}

/// One wire per Solar, Grid, Battery, Consumer and More node; none for House or Water.
pub fn wires(
    diagram: &[EnergyNode],
    placed: &BTreeMap<String, Placed>,
    layout: &CirclesLayout,
) -> Vec<Wire> {
    let Some(house) = placed.get("house").copied() else {
        return Vec::new();
    };
    let slots = diagram
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Consumer | NodeKind::More))
        .count();
    let mut arc = 0usize;
    let mut out = Vec::new();
    for n in diagram {
        let arc_slot = if matches!(n.kind, NodeKind::Consumer | NodeKind::More) {
            arc += 1;
            Some(arc - 1)
        } else {
            None
        };
        if matches!(n.kind, NodeKind::House | NodeKind::Water) {
            continue;
        }
        let Some(node) = placed.get(&n.id).copied() else {
            continue;
        };
        let bend = bend_for(layout, n, arc_slot, slots);
        let curve = rim_curve(node, house, bend);
        let to_house = match n.kind {
            NodeKind::Solar => true,
            NodeKind::Grid | NodeKind::Battery => n.flow != Flow::Out,
            _ => false,
        };
        let node_tone = tone(n, consumer_slot(diagram, &n.id));
        let active = is_active(n);
        let (from, to, from_tone, to_tone, curve) = if to_house {
            (n.id.clone(), "house".to_string(), node_tone, "house", curve)
        } else {
            (
                "house".to_string(),
                n.id.clone(),
                "house",
                node_tone,
                curve.reversed(),
            )
        };
        out.push(Wire {
            id: n.id.clone(),
            from,
            to,
            kind: n.kind,
            tone: node_tone,
            from_tone,
            to_tone,
            curve,
            watts: n.value.unwrap_or(0.0),
            active,
            width: wire_width(n.value, active),
        });
    }
    out
}

/// Stroke width: thin when idle, else 2.5..7.5 with the log power level.
pub fn wire_width(watts: Option<f64>, active: bool) -> f64 {
    if active {
        2.5 + 5.0 * level(watts)
    } else {
        1.5
    }
}

/// `(extra_radius, opacity)` of a node's halo; `(0, 0)` when not live or idle.
pub fn halo(watts: Option<f64>, live: bool) -> (f64, f64) {
    if !live || !watts.is_some_and(|w| w >= IDLE_W) {
        return (0.0, 0.0);
    }
    let l = level(watts);
    (8.0 + 12.0 * l, 0.18 + 0.5 * l)
}

/// Fractions of the house's supply, summing to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub solar: f64,
    pub battery: f64,
    pub grid: f64,
}

/// The mix from the `solar | battery | grid -> house` links; `None` when nothing feeds the house.
pub fn house_mix(s: &Sankey) -> Option<Mix> {
    let sum_from = |id: &str| -> f64 {
        s.links
            .iter()
            .filter(|l| l.to == "house" && l.from == id)
            .map(|l| l.watts)
            .sum()
    };
    let (solar, battery, grid) = (sum_from("solar"), sum_from("battery"), sum_from("grid"));
    let total = solar + battery + grid;
    if total <= 0.0 || !total.is_finite() {
        return None;
    }
    Some(Mix {
        solar: solar / total,
        battery: battery / total,
        grid: grid / total,
    })
}

/// `(dash_len, dash_offset)` for the solar, battery and grid arcs of a ring of radius `r`,
/// clockwise from the top in that order (the caller rotates the circle). `gap` px separates the
/// arcs, only when more than one is non-empty; an empty part has length 0. The offset is
/// `-start`, as `stroke-dashoffset` wants it.
pub fn ring_dashes(mix: &Mix, r: f64, gap: f64) -> [(f64, f64); 3] {
    let circ = std::f64::consts::TAU * r;
    let parts = [mix.solar, mix.battery, mix.grid];
    let n = parts.iter().filter(|p| **p > 0.0).count();
    let gap = if n > 1 { gap } else { 0.0 };
    let room = (circ - n as f64 * gap).max(0.0);
    let mut start = 0.0;
    let mut out = [(0.0, 0.0); 3];
    for (i, p) in parts.iter().enumerate() {
        if *p > 0.0 {
            let len = p * room;
            out[i] = (len, -start);
            start += len + gap;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::energy::{EnergyEntity, EnergyFlow, EnergyRole};

    fn ent(id: &str, name: &str, role: EnergyRole, unit: &str, value: f64) -> EnergyEntity {
        EnergyEntity {
            id: id.to_string(),
            device_id: None,
            name: name.to_string(),
            label: None,
            unit: Some(unit.to_string()),
            value: Some(value),
            available: true,
            role: Some(role),
            invert: false,
            order: None,
            icon: None,
            meter: None,
        }
    }

    fn consumer(n: usize) -> EnergyEntity {
        ent(
            &format!("c{n}"),
            &format!("C{n}"),
            EnergyRole::Consumer,
            "W",
            100.0 * (n as f64 + 1.0),
        )
    }

    fn full() -> Vec<EnergyEntity> {
        let mut es = vec![
            ent("s", "S", EnergyRole::Solar, "W", 100.0),
            ent("g", "G", EnergyRole::Grid, "W", 100.0),
            ent("w", "Water", EnergyRole::Water, "L", 5.0),
        ];
        es.extend((0..5).map(consumer));
        es
    }

    fn arc_of(diagram: &[EnergyNode], p: &BTreeMap<String, Placed>) -> Vec<(f64, f64)> {
        diagram
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Consumer | NodeKind::More))
            .map(|n| p[&n.id].at)
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn wide_places_the_triangle_and_the_arc() {
        let ui = EnergyFlow::build(&full(), 0, None);
        let p = place(&ui.diagram, &CirclesLayout::WIDE);
        assert_eq!(p["solar"].at, (260.0, 74.0));
        assert_eq!(p["grid"].at, (96.0, 214.0));
        assert_eq!(p["house"].at, (400.0, 214.0));
        assert_eq!(p["house"].r, 54.0);
        assert_eq!(p["solar"].r, 46.0);
        assert_eq!(p["water"].at, (190.0, 330.0));
        assert_eq!(p["water"].r, 30.0);
        assert_eq!(
            arc_of(&ui.diagram, &p),
            vec![(630.0, 80.0), (655.0, 214.0), (630.0, 348.0)]
        );
    }

    #[test]
    fn narrow_runs_top_to_bottom() {
        let ui = EnergyFlow::build(&full(), 0, None);
        let p = place(&ui.diagram, &CirclesLayout::NARROW);
        assert_eq!(p["solar"].at, (60.0, 74.0));
        assert_eq!(p["grid"].at, (300.0, 74.0));
        assert_eq!(p["water"].at, (180.0, 66.0));
        assert_eq!(p["house"].at, (180.0, 240.0));
        let arc = arc_of(&ui.diagram, &p);
        assert_eq!(arc.len(), 3);
        assert!(arc.iter().all(|a| a.1 >= 400.0));
    }

    #[test]
    fn battery_or_water_takes_the_side_slot() {
        let mut es = full();
        es.push(ent("b", "B", EnergyRole::Battery, "W", 50.0));
        let ui = EnergyFlow::build(&es, 0, None);
        let p = place(&ui.diagram, &CirclesLayout::WIDE);
        assert_eq!(p["battery"].at, (190.0, 330.0));
        assert_eq!(p["battery"].r, 46.0);
        assert!(!p.contains_key("water"));

        let es = vec![
            ent("s", "S", EnergyRole::Solar, "W", 100.0),
            ent("g", "G", EnergyRole::Grid, "W", 100.0),
            consumer(0),
        ];
        let ui = EnergyFlow::build(&es, 0, None);
        let p = place(&ui.diagram, &CirclesLayout::NARROW);
        assert_eq!(p["solar"].at, (84.0, 74.0));
        assert_eq!(p["grid"].at, (276.0, 74.0));
        assert_eq!(arc_of(&ui.diagram, &p), vec![(180.0, 404.0)]);
    }

    #[test]
    fn bends_follow_the_table() {
        let ui = EnergyFlow::build(&full(), 0, None);
        let l = &CirclesLayout::WIDE;
        let node = |k: NodeKind| ui.diagram.iter().find(|n| n.kind == k).unwrap();
        assert_eq!(bend_for(l, node(NodeKind::Solar), None, 3), 26.0);
        assert_eq!(bend_for(l, node(NodeKind::Grid), None, 3), -22.0);
        assert_eq!(bend_for(l, node(NodeKind::House), None, 3), 0.0);
        let c = node(NodeKind::Consumer);
        assert_eq!(bend_for(l, c, Some(0), 3), -18.0);
        assert_eq!(bend_for(l, c, Some(2), 3), 18.0);
        assert_eq!(bend_for(l, c, Some(0), 2), 0.0);
        assert_eq!(bend_for(&CirclesLayout::NARROW, c, Some(0), 3), 16.0);
    }

    #[test]
    fn rim_curve_leaves_and_meets_the_rims() {
        let node = Placed {
            at: (260.0, 74.0),
            r: 46.0,
        };
        let house = Placed {
            at: (400.0, 214.0),
            r: 54.0,
        };
        let c = rim_curve(node, house, 0.0);
        let d = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
        assert!(close(d(c.from, node.at), 46.0));
        assert!(close(d(c.to, house.at), 54.0));
        let mid = ((c.from.0 + c.to.0) / 2.0, (c.from.1 + c.to.1) / 2.0);
        assert!(close(c.ctrl.0, mid.0) && close(c.ctrl.1, mid.1));
        let b = rim_curve(node, house, 20.0);
        assert!(close(d(b.ctrl, mid), 20.0));
        assert_eq!(b.point(0.0), b.from);
        assert_eq!(b.point(1.0), b.to);
        let r = b.reversed();
        assert_eq!((r.from, r.to, r.ctrl), (b.to, b.from, b.ctrl));
        assert!(b.path().starts_with("M ") && b.path().contains(" Q "));
        assert!(b.length() >= d(b.from, b.to));
    }

    #[test]
    fn rim_curve_survives_coincident_centres() {
        let p = Placed {
            at: (100.0, 100.0),
            r: 40.0,
        };
        let c = rim_curve(p, p, 22.0);
        for v in [c.from.0, c.from.1, c.ctrl.0, c.ctrl.1, c.to.0, c.to.1] {
            assert!(v.is_finite());
        }
        let l = c.length();
        assert!(l.is_finite() && l >= 0.0);
        assert!(!c.path().contains("NaN"));
    }

    fn wires_for(es: &[EnergyEntity], l: &CirclesLayout) -> (Vec<EnergyNode>, Vec<Wire>) {
        let ui = EnergyFlow::build(es, 0, None);
        let p = place(&ui.diagram, l);
        let w = wires(&ui.diagram, &p, l);
        (ui.diagram, w)
    }

    fn base(solar: f64, grid: f64) -> Vec<EnergyEntity> {
        vec![
            ent("s", "S", EnergyRole::Solar, "W", solar),
            ent("g", "G", EnergyRole::Grid, "W", grid),
            consumer(0),
        ]
    }

    fn wire<'a>(ws: &'a [Wire], id: &str) -> &'a Wire {
        ws.iter().find(|w| w.id == id).unwrap()
    }

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        close(a.0, b.0) && close(a.1, b.1)
    }

    #[test]
    fn solar_only_feeds_the_house() {
        let (_, ws) = wires_for(&base(3000.0, -2000.0), &CirclesLayout::WIDE);
        assert!(
            ws.iter()
                .filter(|w| w.from == "solar")
                .all(|w| w.to == "house")
        );
        assert!(!ws.iter().any(|w| {
            (w.from == "solar" && w.to == "grid") || (w.from == "grid" && w.to == "solar")
        }));
        assert_eq!(ws.iter().filter(|w| w.id == "solar").count(), 1);
    }

    #[test]
    fn export_reverses_the_grid_wire() {
        let l = &CirclesLayout::WIDE;
        let es = base(3000.0, -2000.0);
        let ui = EnergyFlow::build(&es, 0, None);
        let p = place(&ui.diagram, l);
        let ws = wires(&ui.diagram, &p, l);
        let g = wire(&ws, "grid");
        assert_eq!((g.from.as_str(), g.to.as_str()), ("house", "grid"));
        assert_eq!(g.tone, "export");
        assert_eq!((g.from_tone, g.to_tone), ("house", "export"));
        let want = rim_curve(p["grid"], p["house"], l.bend_grid).reversed();
        assert!(near(g.curve.from, want.from) && near(g.curve.to, want.to));
        assert!(near(g.curve.ctrl, want.ctrl));
        let (_, ws) = wires_for(&base(3000.0, 500.0), l);
        let g = wire(&ws, "grid");
        assert_eq!((g.from.as_str(), g.to.as_str()), ("grid", "house"));
        assert_eq!(g.tone, "grid");
    }

    #[test]
    fn battery_charge_runs_to_the_battery() {
        let mut es = base(3000.0, 0.0);
        es.push(ent("b", "B", EnergyRole::Battery, "W", -500.0));
        let (_, ws) = wires_for(&es, &CirclesLayout::WIDE);
        let b = wire(&ws, "battery");
        assert_eq!((b.from.as_str(), b.to.as_str()), ("house", "battery"));
        let mut es = base(3000.0, 0.0);
        es.push(ent("b", "B", EnergyRole::Battery, "W", 500.0));
        let (_, ws) = wires_for(&es, &CirclesLayout::WIDE);
        let b = wire(&ws, "battery");
        assert_eq!((b.from.as_str(), b.to.as_str()), ("battery", "house"));
    }

    #[test]
    fn unknown_and_unavailable_are_idle() {
        let mut es = base(0.0, 100.0);
        es[0].value = None;
        es[1].available = false;
        let (d, ws) = wires_for(&es, &CirclesLayout::WIDE);
        for id in ["solar", "grid"] {
            let w = wire(&ws, id);
            assert!(!w.active, "{id}");
            assert!(close(w.width, 1.5));
            assert!(w.curve.path().find("NaN").is_none());
        }
        for n in d
            .iter()
            .filter(|n| matches!(n.id.as_str(), "solar" | "grid"))
        {
            assert_eq!(halo(n.value, n.status == NodeStatus::Live), (0.0, 0.0));
        }
        assert_eq!(halo(None, true), (0.0, 0.0));
        assert_eq!(halo(Some(5.0), true), (0.0, 0.0));
    }

    #[test]
    fn water_has_no_wire() {
        let (d, ws) = wires_for(&full(), &CirclesLayout::WIDE);
        assert!(d.iter().any(|n| n.id == "water"));
        assert!(!ws.iter().any(|w| w.id == "water" || w.id == "house"));
    }

    #[test]
    fn consumers_take_slot_tones() {
        let mut es = base(100.0, 100.0);
        es.push(consumer(1));
        es.push(consumer(2));
        let (_, ws) = wires_for(&es, &CirclesLayout::WIDE);
        let tones: Vec<_> = ws
            .iter()
            .filter(|w| w.kind == NodeKind::Consumer)
            .map(|w| w.tone)
            .collect();
        assert_eq!(tones.len(), 3);
        assert!(tones.contains(&"c0") && tones.contains(&"c1") && tones.contains(&"c2"));
        let (d, ws) = wires_for(&full(), &CirclesLayout::WIDE);
        let mut cons: Vec<_> = ws
            .iter()
            .filter(|w| w.kind != NodeKind::Solar && w.kind != NodeKind::Grid)
            .collect();
        cons.sort_by_key(|w| d.iter().position(|n| n.id == w.id));
        let t: Vec<_> = cons.iter().map(|w| w.tone).collect();
        assert_eq!(t, ["c0", "c1", "more"]);
        assert!(cons.iter().all(|w| w.to != "house"));
    }

    #[test]
    fn widths_and_halo_scale_with_power() {
        assert!(close(wire_width(Some(10.0), true), 2.5));
        assert!(close(wire_width(Some(10_000.0), true), 7.5));
        assert!(close(wire_width(Some(10_000.0), false), 1.5));
        let h = halo(Some(10_000.0), true);
        assert!(close(h.0, 20.0) && close(h.1, 0.68));
        assert_eq!(halo(Some(10_000.0), false), (0.0, 0.0));
    }

    #[test]
    fn rim_curve_bows_to_the_left_normal() {
        let a = Placed {
            at: (0.0, 0.0),
            r: 10.0,
        };
        let h = Placed {
            at: (100.0, 0.0),
            r: 10.0,
        };
        assert!(rim_curve(a, h, 10.0).ctrl.1 < 0.0);
    }

    fn sankey(es: &[EnergyEntity]) -> Sankey {
        crate::energy_sankey::build(&EnergyFlow::build(es, 0, None), 3)
    }

    #[test]
    fn mix_follows_the_house_links() {
        let mut es = base(600.0, 200.0);
        es.push(ent("h", "H", EnergyRole::House, "W", 800.0));
        let m = house_mix(&sankey(&es)).unwrap();
        assert!(close(m.solar, 0.75) && close(m.battery, 0.0) && close(m.grid, 0.25));
        let r = 40.0;
        let d = ring_dashes(&m, r, 6.0);
        let sum: f64 = d.iter().map(|x| x.0).sum();
        assert!(close(sum, std::f64::consts::TAU * r - 12.0));
        assert_eq!(d[1].0, 0.0);
        assert!(close(d[0].1, 0.0));
        assert!(close(d[2].1, -(d[0].0 + 6.0)));
        let solo = ring_dashes(
            &Mix {
                solar: 1.0,
                battery: 0.0,
                grid: 0.0,
            },
            r,
            6.0,
        );
        assert!(close(solo[0].0, std::f64::consts::TAU * r));
    }

    #[test]
    fn mix_is_none_when_nothing_feeds_the_house() {
        assert_eq!(house_mix(&Sankey::default()), None);
        let mut es = base(0.0, 0.0);
        es.push(ent("h", "H", EnergyRole::House, "W", 0.0));
        assert_eq!(house_mix(&sankey(&es)), None);
        let d = ring_dashes(
            &Mix {
                solar: 0.0,
                battery: 0.0,
                grid: 0.0,
            },
            40.0,
            6.0,
        );
        assert!(d.iter().all(|x| x.0.is_finite() && x.1.is_finite()));
    }
}
