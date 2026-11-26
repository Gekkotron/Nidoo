//! Where the flow canvas draws things: pill geometry, the layered auto layout, the pins wires
//! leave and enter by, and wires routed around cards. Pure, tested natively.

use std::collections::BTreeMap;

use nidoo_core::flow::{Flow, NodeKind, Position};

/// Width of a card (pill) on the canvas.
pub const NODE_W: f64 = 232.0;
/// Height of a card (pill) on the canvas.
pub const NODE_H: f64 = 52.0;
/// Height each exit of a switch card needs, so its pins and their labels stay apart.
const EXIT_H: f64 = 20.0;
/// Padding a switch card adds around its exits.
const EXIT_PAD: f64 = 14.0;
/// Horizontal space between two columns.
pub const COL_GAP: f64 = 96.0;
/// Vertical space between two cards of a column.
pub const ROW_GAP: f64 = 28.0;
/// Horizontal space between two columns when a card on either side has several wires
/// crossing that gap, so the fanned-out wires have room to curve apart.
pub const FAN_COL_GAP: f64 = 160.0;
/// Vertical space between two cards of a column when either has several wires in or out, or
/// both are wired to the same card.
pub const FAN_ROW_GAP: f64 = 64.0;
/// Margin the auto layout leaves above and left of the graph.
pub const PAD: f64 = 40.0;
/// Vertical space between cards added one after another from the library.
pub const STACK_GAP: f64 = 16.0;

/// A point in canvas (world) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

/// An axis-aligned rectangle in canvas coordinates: top-left corner and size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One drawn wire: its SVG path, where its "yes"/"no" label sits (just right of its pin, at
/// the pin's height), where its ✕ goes (halfway along) and the dense polyline it follows.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub d: String,
    pub label_at: Pt,
    pub mid: Pt,
    pub points: Vec<Pt>,
}

/// An edge as `(from, port, to)`.
type Key = (String, Option<String>, String);

/// How far around a card a wire must stay clear.
const CLEARANCE: f64 = 6.0;
/// Points sampled along a wire segment to test it against the cards.
const SAMPLES: usize = 24;
/// Widest horizontal run of the curve that takes a wire onto, or off, its lane; kept inside
/// the gap between two columns.
const RAMP: f64 = COL_GAP * 0.75;
/// Horizontal distance from a wire's pin to the centre of its "yes"/"no" label. The label sits
/// at the pin's own height, so the two labels of an `if` stay as far apart as its two pins.
const LABEL_DX: f64 = 24.0;

/// The height of a card: [`NODE_H`], except a switch, which grows to give each exit a row.
pub fn node_height(kind: &NodeKind) -> f64 {
    match kind {
        NodeKind::Switch(_) | NodeKind::Ask(_) | NodeKind::BackupRun(_) => {
            NODE_H.max(EXIT_PAD + EXIT_H * kind.ports().len() as f64)
        }
        _ => NODE_H,
    }
}

/// The height of the card `id` in `flow`; [`NODE_H`] for an id the flow does not have.
fn height_of(flow: &Flow, id: &str) -> f64 {
    flow.node(id).map_or(NODE_H, |n| node_height(&n.kind))
}

/// Where a new card goes so it overlaps none of the cards in `taken` (their top-left corners)
/// and stays inside `area` (what is on screen): down the column from `start` in steps of one
/// card (`height` tall, the new card's) and [`STACK_GAP`], then from the top of that column, then in the columns beside it,
/// nearest first. `start` itself when nothing in `area` is free.
pub fn free_spot(
    flow: &Flow,
    taken: &BTreeMap<String, Pt>,
    start: Pt,
    area: Rect,
    height: f64,
) -> Pt {
    let free = |at: Pt| {
        !taken.iter().any(|(id, p)| {
            at.x < p.x + NODE_W
                && p.x < at.x + NODE_W
                && at.y < p.y + height_of(flow, id)
                && p.y < at.y + height
        })
    };
    let (right, bottom) = (area.x + area.w, area.y + area.h);
    let (step_x, step_y) = (NODE_W + COL_GAP, height + STACK_GAP);
    let column = |x: f64| {
        let below = std::iter::successors(Some(start.y), move |y| Some(y + step_y))
            .take_while(move |y| y + height <= bottom);
        let above = std::iter::successors(Some(area.y), move |y| Some(y + step_y))
            .take_while(move |y| *y < start.y);
        below
            .chain(above)
            .map(move |y| Pt { x, y })
            .find(|&at| free(at))
    };
    // Column offsets 0, +1, -1, +2, -2, … far enough to cover the area from `start`.
    let reach = ((area.w + (start.x - area.x).abs()) / step_x).ceil() as u32 + 1;
    std::iter::once(0.0)
        .chain((1..=reach).map(f64::from).flat_map(|k| [k, -k]))
        .map(|k| start.x + k * step_x)
        .filter(|x| *x >= area.x && x + NODE_W <= right)
        .find_map(column)
        .unwrap_or(start)
}

/// Where each card goes: the stored positions when every node has one (a flow arranged by hand
/// keeps its look), else [`auto_layout`].
pub fn positions(flow: &Flow) -> BTreeMap<String, Pt> {
    if !flow.nodes.is_empty() && flow.nodes.iter().all(|n| n.position.is_some()) {
        return flow
            .nodes
            .iter()
            .filter_map(|n| n.position.map(|p| (n.id.clone(), Pt { x: p.x, y: p.y })))
            .collect();
    }
    auto_layout(flow)
}

/// A place in a column: a card, or a point a long edge passes through on its way across.
struct Slot {
    node: Option<String>,
    col: usize,
    h: f64,
}

/// The layered layout, ignoring stored positions. Columns come from [`Flow::layers`]; an edge
/// that skips columns reserves a half-height slot in each column it crosses. Each column is
/// ordered by four down-and-up barycenter sweeps (fewer crossings), then stacked and centred on
/// its predecessors, never above `PAD`. Wherever a card is wired to several others the gaps
/// around it widen ([`FAN_ROW_GAP`], [`FAN_COL_GAP`]) so its wires stay apart.
pub fn auto_layout(flow: &Flow) -> BTreeMap<String, Pt> {
    let mut slots: Vec<Slot> = Vec::new();
    let mut columns: Vec<Vec<usize>> = Vec::new();
    let mut slot_of: BTreeMap<String, usize> = BTreeMap::new();
    for (col, ids) in flow.layers().into_iter().enumerate() {
        let mut column = Vec::new();
        for id in ids {
            let s = slots.len();
            slot_of.entry(id.clone()).or_insert(s);
            slots.push(Slot {
                h: height_of(flow, &id),
                node: Some(id),
                col,
            });
            column.push(s);
        }
        columns.push(column);
    }
    // Links between slots of consecutive columns, dummies included.
    let mut left: Vec<Vec<usize>> = vec![Vec::new(); slots.len()];
    let mut right: Vec<Vec<usize>> = vec![Vec::new(); slots.len()];
    for edge in &flow.edges {
        let (Some(&a), Some(&b)) = (slot_of.get(&edge.from), slot_of.get(&edge.to)) else {
            continue;
        };
        let (ca, cb) = (slots[a].col, slots[b].col);
        if cb <= ca {
            continue;
        }
        let mut chain = vec![a];
        for (col, column) in columns.iter_mut().enumerate().take(cb).skip(ca + 1) {
            column.push(slots.len());
            chain.push(slots.len());
            slots.push(Slot {
                node: None,
                col,
                h: NODE_H / 2.0,
            });
            left.push(Vec::new());
            right.push(Vec::new());
        }
        chain.push(b);
        for pair in chain.windows(2) {
            right[pair[0]].push(pair[1]);
            left[pair[1]].push(pair[0]);
        }
    }

    let mut row = vec![0usize; slots.len()];
    for column in &columns {
        for (i, &s) in column.iter().enumerate() {
            row[s] = i;
        }
    }
    for _ in 0..4 {
        for column in columns.iter_mut().skip(1) {
            order_by_barycenter(column, &left, &mut row);
        }
        let below_last = columns.len().saturating_sub(1);
        for column in columns.iter_mut().take(below_last).rev() {
            order_by_barycenter(column, &right, &mut row);
        }
    }

    // A card wired to several others, on either side, is a fan.
    let degree = |side: &[Vec<usize>], s: usize| side[s].len();
    let fans = |s: usize| degree(&left, s) > 1 || degree(&right, s) > 1;
    let shares_neighbour = |a: usize, b: usize| {
        left[a].iter().any(|n| left[b].contains(n)) || right[a].iter().any(|n| right[b].contains(n))
    };
    let mut top = vec![0.0; slots.len()];
    for column in &columns {
        let mut offsets = Vec::with_capacity(column.len());
        let mut height = 0.0;
        for (i, &s) in column.iter().enumerate() {
            if i > 0 {
                let above = column[i - 1];
                height += if fans(above) || fans(s) || shares_neighbour(above, s) {
                    FAN_ROW_GAP
                } else {
                    ROW_GAP
                };
            }
            offsets.push(height);
            height += slots[s].h;
        }
        let mut preds: Vec<usize> = column.iter().flat_map(|&s| left[s].clone()).collect();
        preds.sort_unstable();
        preds.dedup();
        let start = if preds.is_empty() {
            PAD
        } else {
            let centre = preds
                .iter()
                .map(|&p| top[p] + slots[p].h / 2.0)
                .sum::<f64>()
                / preds.len() as f64;
            (centre - height / 2.0).round().max(PAD)
        };
        for (&s, offset) in column.iter().zip(offsets) {
            top[s] = start + offset;
        }
    }

    // The gap after a column widens when a slot there has several wires leaving it, or a slot
    // of the next column several wires entering it.
    let mut lefts = Vec::with_capacity(columns.len());
    let mut x = PAD;
    for (col, column) in columns.iter().enumerate() {
        lefts.push(x);
        let fans_out = column.iter().any(|&s| degree(&right, s) > 1);
        let fans_in = columns
            .get(col + 1)
            .is_some_and(|next| next.iter().any(|&s| degree(&left, s) > 1));
        x += NODE_W
            + if fans_out || fans_in {
                FAN_COL_GAP
            } else {
                COL_GAP
            };
    }

    slots
        .iter()
        .zip(top)
        .filter_map(|(slot, y)| {
            let id = slot.node.clone()?;
            Some((
                id,
                Pt {
                    x: lefts[slot.col],
                    y,
                },
            ))
        })
        .collect()
}

/// Sorts `column` by the mean row of each slot's `neighbours` (a slot without any keeps its own
/// row); the sort is stable, so ties keep the current order. Updates `row` to match.
fn order_by_barycenter(column: &mut Vec<usize>, neighbours: &[Vec<usize>], row: &mut [usize]) {
    let mut keyed: Vec<(usize, f64)> = column
        .iter()
        .map(|&s| {
            let ns = &neighbours[s];
            let at = if ns.is_empty() {
                row[s] as f64
            } else {
                ns.iter().map(|&n| row[n] as f64).sum::<f64>() / ns.len() as f64
            };
            (s, at)
        })
        .collect();
    keyed.sort_by(|a, b| a.1.total_cmp(&b.1));
    *column = keyed.into_iter().map(|(s, _)| s).collect();
    for (i, &s) in column.iter().enumerate() {
        row[s] = i;
    }
}

/// `flow` with every node's `position` replaced by [`auto_layout`]'s: what **Tidy** does.
pub fn tidy(flow: &Flow) -> Flow {
    let auto = auto_layout(flow);
    let mut out = flow.clone();
    for node in &mut out.nodes {
        node.position = auto.get(&node.id).map(|p| Position { x: p.x, y: p.y });
    }
    out
}

/// `flow` with the missing positions filled from [`auto_layout`]; stored ones are kept. Used to
/// start edit mode from coordinates every node has.
pub fn with_resolved_positions(flow: &Flow) -> Flow {
    let auto = auto_layout(flow);
    let mut out = flow.clone();
    for node in &mut out.nodes {
        if node.position.is_none() {
            node.position = auto.get(&node.id).map(|p| Position { x: p.x, y: p.y });
        }
    }
    out
}

/// The rectangle a card `height` tall at `at` covers.
fn card(at: Pt, height: f64) -> Rect {
    Rect {
        x: at.x,
        y: at.y,
        w: NODE_W,
        h: height,
    }
}

/// The smallest rectangle holding every card, with no margin; all zero when there is none.
pub fn bounds(flow: &Flow, pos: &BTreeMap<String, Pt>) -> Rect {
    if pos.is_empty() {
        return Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
    }
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (id, at) in pos {
        x0 = x0.min(at.x);
        y0 = y0.min(at.y);
        x1 = x1.max(at.x + NODE_W);
        y1 = y1.max(at.y + height_of(flow, id));
    }
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

/// Where a wire leaves a card at `at`: its right edge, halfway down; for an `if` 30 % down for
/// `true` and 70 % down for `false`; for a switch, each exit evenly down the card; for a time
/// card 30 % down for `in` and 70 % down for `out`; for a back-up card, `ok`, `partial` and
/// `failed` evenly down the card, as a switch.
pub fn out_anchor(kind: &NodeKind, at: Pt, port: Option<&str>) -> Pt {
    let down = match (kind, port) {
        (NodeKind::If(_), Some("true")) | (NodeKind::TimeBetween(_), Some("in")) => 0.3,
        (NodeKind::If(_), Some("false")) | (NodeKind::TimeBetween(_), Some("out")) => 0.7,
        (NodeKind::Switch(_) | NodeKind::Ask(_) | NodeKind::BackupRun(_), Some(port)) => {
            let ports = kind.ports();
            ports
                .iter()
                .position(|p| p == port)
                .map_or(0.5, |i| (i as f64 + 0.5) / ports.len() as f64)
        }
        _ => 0.5,
    };
    Pt {
        x: at.x + NODE_W,
        y: at.y + node_height(kind) * down,
    }
}

/// Where a wire enters a card at `at`, `height` tall: the input pin, on its left edge halfway
/// down. Every wire into one card meets there, as the card has a single input.
pub fn in_anchor(at: Pt, height: f64) -> Pt {
    Pt {
        x: at.x,
        y: at.y + height / 2.0,
    }
}

/// A piece of a wire.
enum Seg {
    Cubic([Pt; 4]),
    Line([Pt; 2]),
}

impl Seg {
    fn at(&self, t: f64) -> Pt {
        let lerp = |a: Pt, b: Pt, t: f64| Pt {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
        };
        match self {
            Seg::Line([a, b]) => lerp(*a, *b, t),
            Seg::Cubic([p0, p1, p2, p3]) => {
                let a = lerp(lerp(*p0, *p1, t), lerp(*p1, *p2, t), t);
                let b = lerp(lerp(*p1, *p2, t), lerp(*p2, *p3, t), t);
                lerp(a, b, t)
            }
        }
    }
}

/// A cubic from `a` to `b` that leaves and arrives horizontally, its handles `k` long.
fn curve(a: Pt, b: Pt, k: f64) -> Seg {
    Seg::Cubic([a, Pt { x: a.x + k, y: a.y }, Pt { x: b.x - k, y: b.y }, b])
}

/// Whether any of [`SAMPLES`] points of any segment lies within [`CLEARANCE`] of a rect.
fn blocked(path: &[Seg], obstacles: &[Rect]) -> bool {
    path.iter().any(|seg| {
        (0..SAMPLES).any(|i| {
            let p = seg.at(i as f64 / (SAMPLES - 1) as f64);
            obstacles.iter().any(|r| {
                p.x > r.x - CLEARANCE
                    && p.x < r.x + r.w + CLEARANCE
                    && p.y > r.y - CLEARANCE
                    && p.y < r.y + r.h + CLEARANCE
            })
        })
    })
}

/// The detour of a blocked wire: onto a horizontal lane just above or below one of the cards
/// in its way, the lane closest to both ends that keeps the whole wire clear.
fn lane(from: Pt, to: Pt, obstacles: &[Rect]) -> Option<Vec<Seg>> {
    let (lo, hi) = (from.x.min(to.x), from.x.max(to.x));
    let mut lanes: Vec<f64> = obstacles
        .iter()
        .filter(|r| r.x < hi && r.x + r.w > lo)
        .flat_map(|r| [r.y - ROW_GAP / 2.0, r.y + r.h + ROW_GAP / 2.0])
        .collect();
    let cost = |y: f64| (y - from.y).abs() + (y - to.y).abs();
    lanes.sort_by(|a, b| cost(*a).total_cmp(&cost(*b)));
    let ramp = RAMP.min((to.x - from.x).abs() / 2.0);
    lanes.into_iter().find_map(|y| {
        let on = Pt {
            x: from.x + ramp,
            y,
        };
        let off = Pt { x: to.x - ramp, y };
        let path = vec![
            curve(from, on, ramp / 2.0),
            Seg::Line([on, off]),
            curve(off, to, ramp / 2.0),
        ];
        (!blocked(&path, obstacles)).then_some(path)
    })
}

/// The wire from `from` to `to`: a horizontal-handled cubic, or, when that passes within
/// [`CLEARANCE`] of an obstacle, a detour along a lane beside the cards in the way.
pub fn route(from: Pt, to: Pt, obstacles: &[Rect]) -> Route {
    let k = ((to.x - from.x).abs() / 2.0).max(40.0);
    let direct = vec![curve(from, to, k)];
    let path = if blocked(&direct, obstacles) {
        lane(from, to, obstacles).unwrap_or(direct)
    } else {
        direct
    };
    finish(from, &path)
}

/// The SVG path of `path`, its label point ([`LABEL_DX`] right of `from`) and the point halfway
/// along it.
fn finish(from: Pt, path: &[Seg]) -> Route {
    let mut d = format!("M{:.1} {:.1}", from.x, from.y);
    for seg in path {
        match seg {
            Seg::Cubic([_, a, b, c]) => d.push_str(&format!(
                " C{:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
                a.x, a.y, b.x, b.y, c.x, c.y
            )),
            Seg::Line([_, b]) => d.push_str(&format!(" L{:.1} {:.1}", b.x, b.y)),
        }
    }
    // A polyline dense enough to measure lengths along the wire.
    let points: Vec<Pt> = std::iter::once(from)
        .chain(
            path.iter()
                .flat_map(|seg| (1..=64).map(move |i| seg.at(f64::from(i) / 64.0))),
        )
        .collect();
    let mut walked = vec![0.0];
    for pair in points.windows(2) {
        let step = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        walked.push(walked.last().copied().unwrap_or(0.0) + step);
    }
    let total = walked.last().copied().unwrap_or(0.0);
    let along = |len: f64| {
        let i = walked
            .partition_point(|&w| w < len)
            .clamp(1, points.len() - 1);
        let span = walked[i] - walked[i - 1];
        let t = if span > 0.0 {
            (len - walked[i - 1]) / span
        } else {
            0.0
        };
        Pt {
            x: points[i - 1].x + (points[i].x - points[i - 1].x) * t,
            y: points[i - 1].y + (points[i].y - points[i - 1].y) * t,
        }
    };
    Route {
        d,
        label_at: Pt {
            x: from.x + LABEL_DX,
            y: from.y,
        },
        mid: along(total / 2.0),
        points,
    }
}

/// The wire that passes under a card at `at`, the one closest to the card's centre when
/// several do; `None` when none crosses it.
pub fn wire_under(wires: &[(Key, Route)], at: Pt, height: f64) -> Option<Key> {
    let rect = card(at, height);
    let centre = Pt {
        x: at.x + NODE_W / 2.0,
        y: at.y + height / 2.0,
    };
    wires
        .iter()
        .filter_map(|(key, route)| {
            route
                .points
                .iter()
                .filter(|p| {
                    p.x > rect.x && p.x < rect.x + rect.w && p.y > rect.y && p.y < rect.y + rect.h
                })
                .map(|p| (p.x - centre.x).hypot(p.y - centre.y))
                .min_by(f64::total_cmp)
                .map(|d| (d, key))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, key)| key.clone())
}

/// Every edge's wire, in edge order, keyed by `(from, port, to)`: from its [`out_anchor`] to
/// its [`in_anchor`], routed around every card but its own two ends. Edges whose ends
/// have no position are left out.
pub fn routes(flow: &Flow, pos: &BTreeMap<String, Pt>) -> Vec<(Key, Route)> {
    routes_past(flow, pos, None)
}

/// [`routes`], except that no wire steers around the card `loose` (the one being dragged), so
/// the wires stay put under it.
pub fn routes_past(
    flow: &Flow,
    pos: &BTreeMap<String, Pt>,
    loose: Option<&str>,
) -> Vec<(Key, Route)> {
    flow.edges
        .iter()
        .filter_map(|edge| {
            let from = flow.node(&edge.from)?;
            let start = out_anchor(&from.kind, *pos.get(&edge.from)?, edge.port.as_deref());
            let key = (edge.from.clone(), edge.port.clone(), edge.to.clone());
            let end = in_anchor(*pos.get(&edge.to)?, height_of(flow, &edge.to));
            let obstacles: Vec<Rect> = pos
                .iter()
                .filter(|(id, _)| {
                    **id != edge.from && **id != edge.to && Some(id.as_str()) != loose
                })
                .map(|(id, at)| card(*at, height_of(flow, id)))
                .collect();
            Some((key, route(start, end, &obstacles)))
        })
        .collect()
}

#[cfg(test)]
mod tests;
