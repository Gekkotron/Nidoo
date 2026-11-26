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
        // Far enough from the right edge for a name such as "Pompe à chaleur" beside the circle.
        one: [(630.0, 214.0)],
        two: [(620.0, 140.0), (620.0, 288.0)],
        three: [(610.0, 80.0), (635.0, 214.0), (610.0, 348.0)],
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

/// The gap between a consumer's circle and its name beside it, on a wide screen.
pub const SIDE_LABEL_GAP: f64 = 20.0;
/// What a name keeps clear of the diagram's edge.
const LABEL_EDGE: f64 = 4.0;
/// The widest a name centred under its circle may be: the phone's three consumers stand 124
/// apart, so two neighbouring names never meet.
const CENTRED_LABEL_MAX: f64 = 120.0;

/// The room, in diagram px, for the name of a circle centred at `at` with radius `r`: beside it
/// up to the right edge when `beside`, else centred under it and clear of both edges.
pub fn label_room(layout: &CirclesLayout, beside: bool, at: (f64, f64), r: f64) -> f64 {
    if beside {
        layout.view_w - (at.0 + r + SIDE_LABEL_GAP) - LABEL_EDGE
    } else {
        (2.0 * (at.0.min(layout.view_w - at.0) - LABEL_EDGE)).min(CENTRED_LABEL_MAX)
    }
}

/// A character's rough advance in Inter SemiBold, in em: a hair wider than the real glyph, so a
/// name judged to fit does fit.
fn advance_em(c: char) -> f64 {
    match c {
        'i' | 'l' | 'j' | 'I' | '.' | ',' | '\'' | '’' | '|' | '!' | ':' | ';' | ' ' => 0.28,
        'f' | 't' | 'r' | '-' | '(' | ')' => 0.38,
        'W' => 1.05,
        'm' | 'M' => 0.9,
        'w' => 0.8,
        '…' => 0.85,
        c if c.is_uppercase() => 0.68,
        _ => 0.6,
    }
}

fn width_em(s: &str) -> f64 {
    s.chars().map(advance_em).sum()
}

/// `name` as drawn in `room_px` at `font_px`: whole when it fits, else cut with an ellipsis.
pub fn fit_label(name: String, room_px: f64, font_px: f64) -> String {
    let room = room_px / font_px;
    if width_em(&name) <= room {
        return name;
    }
    let budget = room - advance_em('…');
    let mut used = 0.0;
    let cut: String = name
        .chars()
        .take_while(|c| {
            used += advance_em(*c);
            used <= budget
        })
        .collect();
    format!("{}…", cut.trim_end())
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
mod tests;
