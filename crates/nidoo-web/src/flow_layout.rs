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
        NodeKind::Switch(_) => NODE_H.max(EXIT_PAD + EXIT_H * kind.ports().len() as f64),
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
/// card 30 % down for `in` and 70 % down for `out`.
pub fn out_anchor(kind: &NodeKind, at: Pt, port: Option<&str>) -> Pt {
    let down = match (kind, port) {
        (NodeKind::If(_), Some("true")) | (NodeKind::TimeBetween(_), Some("in")) => 0.3,
        (NodeKind::If(_), Some("false")) | (NodeKind::TimeBetween(_), Some("out")) => 0.7,
        (NodeKind::Switch(_), Some(port)) => {
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
mod tests {
    use super::*;

    #[test]
    fn free_spot_keeps_a_free_start() {
        let taken = BTreeMap::from([("a".to_owned(), Pt { x: 500.0, y: 500.0 })]);
        assert_eq!(
            free_spot(
                &Flow::default(),
                &taken,
                Pt { x: 0.0, y: 0.0 },
                WIDE,
                NODE_H
            ),
            Pt { x: 0.0, y: 0.0 }
        );
    }

    #[test]
    fn free_spot_steps_below_overlapping_cards() {
        let taken = BTreeMap::from([
            ("a".to_owned(), Pt { x: 0.0, y: 0.0 }),
            (
                "b".to_owned(),
                Pt {
                    x: 10.0,
                    y: NODE_H + STACK_GAP,
                },
            ),
        ]);
        // Two cards in the way: it lands two steps down.
        assert_eq!(
            free_spot(
                &Flow::default(),
                &taken,
                Pt { x: 0.0, y: 0.0 },
                WIDE,
                NODE_H
            ),
            Pt {
                x: 0.0,
                y: 2.0 * (NODE_H + STACK_GAP)
            }
        );
    }

    /// An area much larger than any test graph.
    const WIDE: Rect = Rect {
        x: -5000.0,
        y: -5000.0,
        w: 10000.0,
        h: 10000.0,
    };

    fn overlaps_any(taken: &BTreeMap<String, Pt>, at: Pt) -> bool {
        taken.values().any(|p| {
            intersects(
                Rect {
                    x: at.x,
                    y: at.y,
                    w: NODE_W,
                    h: NODE_H,
                },
                Rect {
                    x: p.x,
                    y: p.y,
                    w: NODE_W,
                    h: NODE_H,
                },
            )
        })
    }

    #[test]
    fn free_spot_stays_inside_the_area() {
        // A full column from the start point down past the bottom of the visible area.
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 400.0,
        };
        let taken: BTreeMap<String, Pt> = (0..10)
            .map(|i| {
                (
                    format!("n{i}"),
                    Pt {
                        x: 300.0,
                        y: 150.0 + f64::from(i) * 40.0,
                    },
                )
            })
            .collect();
        let at = free_spot(
            &Flow::default(),
            &taken,
            Pt { x: 300.0, y: 150.0 },
            area,
            NODE_H,
        );
        assert!(!overlaps_any(&taken, at), "{at:?}");
        assert!(at.x >= area.x && at.x + NODE_W <= area.x + area.w, "{at:?}");
        assert!(at.y >= area.y && at.y + NODE_H <= area.y + area.h, "{at:?}");
    }

    #[test]
    fn free_spot_falls_back_to_the_start_when_full() {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 100.0,
        };
        let taken = BTreeMap::from([
            ("a".to_owned(), Pt { x: 0.0, y: 0.0 }),
            ("b".to_owned(), Pt { x: 0.0, y: 50.0 }),
        ]);
        let start = Pt { x: 20.0, y: 20.0 };
        assert_eq!(
            free_spot(&Flow::default(), &taken, start, area, NODE_H),
            start
        );
    }

    fn led() -> Flow {
        Flow::decode(include_str!("../tests/fixtures/living_led_motion.json").as_bytes()).unwrap()
    }

    fn intersects(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn inside(p: Pt, r: Rect) -> bool {
        p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h
    }

    fn lerp(a: Pt, b: Pt, t: f64) -> Pt {
        Pt {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
        }
    }

    /// `n` points along an SVG path of `M`, `C` and `L` commands, spread evenly over its
    /// segments (by parameter, not by length).
    fn route_samples(d: &str, n: usize) -> Vec<Pt> {
        let mut spaced = String::new();
        for ch in d.chars() {
            if ch.is_ascii_alphabetic() {
                spaced.push(' ');
                spaced.push(ch);
                spaced.push(' ');
            } else if ch == ',' {
                spaced.push(' ');
            } else {
                spaced.push(ch);
            }
        }
        let mut commands: Vec<(char, Vec<f64>)> = Vec::new();
        for token in spaced.split_whitespace() {
            match token.parse::<f64>() {
                Ok(v) => commands
                    .last_mut()
                    .expect("path starts with a command")
                    .1
                    .push(v),
                Err(_) => commands.push((token.chars().next().unwrap(), Vec::new())),
            }
        }
        let pt = |v: &[f64], i: usize| Pt {
            x: v[2 * i],
            y: v[2 * i + 1],
        };
        let mut segments: Vec<Vec<Pt>> = Vec::new();
        let mut cur = Pt { x: 0.0, y: 0.0 };
        for (c, v) in &commands {
            match c {
                'M' => cur = pt(v, 0),
                'L' => {
                    segments.push(vec![cur, pt(v, 0)]);
                    cur = pt(v, 0);
                }
                'C' => {
                    segments.push(vec![cur, pt(v, 0), pt(v, 1), pt(v, 2)]);
                    cur = pt(v, 2);
                }
                other => panic!("unexpected path command {other} in {d}"),
            }
        }
        (0..n)
            .map(|i| {
                let g = i as f64 / (n - 1) as f64 * segments.len() as f64;
                let s = (g.floor() as usize).min(segments.len() - 1);
                let t = g - s as f64;
                let p = &segments[s];
                if p.len() == 2 {
                    lerp(p[0], p[1], t)
                } else {
                    let a = lerp(lerp(p[0], p[1], t), lerp(p[1], p[2], t), t);
                    let b = lerp(lerp(p[1], p[2], t), lerp(p[2], p[3], t), t);
                    lerp(a, b, t)
                }
            })
            .collect()
    }

    #[test]
    fn led_layout_has_no_overlaps() {
        let pos = auto_layout(&led());
        assert_eq!(pos.len(), 23);
        let cards: Vec<_> = pos.iter().collect();
        for (i, (a, pa)) in cards.iter().enumerate() {
            assert!(pa.x >= PAD && pa.y >= PAD, "{a} at {pa:?}");
            for (b, pb) in &cards[i + 1..] {
                assert!(
                    !intersects(card(**pa, NODE_H), card(**pb, NODE_H)),
                    "{a} overlaps {b}"
                );
            }
        }
    }

    #[test]
    fn led_routes_avoid_cards() {
        let flow = led();
        let pos = auto_layout(&flow);
        let all = routes(&flow, &pos);
        assert_eq!(all.len(), flow.edges.len());
        for ((from, port, to), route) in &all {
            for p in route_samples(&route.d, 48) {
                for (id, at) in &pos {
                    if id == from || id == to {
                        continue;
                    }
                    assert!(
                        !inside(p, card(*at, NODE_H)),
                        "{from} -{port:?}-> {to} crosses {id} at {p:?}: {}",
                        route.d
                    );
                }
            }
        }
    }

    #[test]
    fn wires_end_on_the_input_pin() {
        // The canvas draws one input pin, halfway down the left edge (graph.rs), so every wire
        // into a card, however many there are, must end on it.
        let flow = led();
        let pos = auto_layout(&flow);
        let all = routes(&flow, &pos);
        let into_tv = all.iter().filter(|((_, _, to), _)| to == "tv_on").count();
        assert_eq!(into_tv, 5);
        for ((from, port, to), route) in &all {
            let end = *route_samples(&route.d, 48).last().unwrap();
            let pin = Pt {
                x: pos[to].x,
                y: pos[to].y + NODE_H / 2.0,
            };
            assert!(
                (end.x - pin.x).abs() < 1e-6 && (end.y - pin.y).abs() < 1e-6,
                "{from} -{port:?}-> {to} ends at {end:?}, pin at {pin:?}"
            );
        }
    }

    #[test]
    fn if_ports() {
        let flow = led();
        let kind = &flow.node("tv_on").unwrap().kind;
        let origin = Pt { x: 0.0, y: 0.0 };
        assert_eq!(out_anchor(kind, origin, Some("true")).y, NODE_H * 0.3);
        assert_eq!(out_anchor(kind, origin, Some("false")).y, NODE_H * 0.7);
        assert_eq!(out_anchor(kind, origin, Some("true")).x, NODE_W);
        let wait = &flow.node("wait").unwrap().kind;
        assert_eq!(
            out_anchor(wait, Pt { x: 10.0, y: 20.0 }, None),
            Pt {
                x: 10.0 + NODE_W,
                y: 20.0 + NODE_H / 2.0
            }
        );
    }

    fn switch_kind(rules: usize, otherwise: bool) -> NodeKind {
        let node: nidoo_core::flow::Node = serde_json::from_value(serde_json::json!({
            "id": "s", "type": "switch",
            "params": { "entity": "e", "otherwise": otherwise,
                "rules": (0..rules).map(|i| serde_json::json!({ "op": "gt", "value": i })).collect::<Vec<_>>() }
        }))
        .unwrap();
        node.kind
    }

    #[test]
    fn switch_card_grows_with_exits() {
        assert_eq!(node_height(&switch_kind(1, false)), NODE_H);
        assert_eq!(node_height(&switch_kind(1, true)), 54.0);
        assert_eq!(node_height(&switch_kind(5, true)), 134.0);
        let kind = switch_kind(5, true);
        let height = node_height(&kind);
        let origin = Pt { x: 0.0, y: 0.0 };
        let ports = kind.ports();
        let ys: Vec<f64> = ports
            .iter()
            .map(|p| out_anchor(&kind, origin, Some(p)).y)
            .collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
        assert!(ys.iter().all(|y| *y > 0.0 && *y < height), "{ys:?}");
        // Other cards keep the standard height.
        assert_eq!(node_height(&led().node("tv_on").unwrap().kind), NODE_H);
    }

    #[test]
    fn time_card_pins_sit_at_30_and_70_percent() {
        let node: nidoo_core::flow::Node = serde_json::from_value(serde_json::json!({
            "id": "t", "type": "time.between", "params": { "from": "07:00", "to": "22:00" }
        }))
        .unwrap();
        let o = Pt { x: 0.0, y: 0.0 };
        assert_eq!(out_anchor(&node.kind, o, Some("in")).y, NODE_H * 0.3);
        assert_eq!(out_anchor(&node.kind, o, Some("out")).y, NODE_H * 0.7);
    }

    #[test]
    fn switch_wires_end_on_their_pins() {
        let flow = Flow::decode(
            br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": "e" } },
            { "id": "s", "type": "switch", "params": { "entity": "e", "otherwise": true,
              "rules": [ { "op": "gt", "value": 1 }, { "op": "gt", "value": 2 }, { "op": "gt", "value": 3 } ] } },
            { "id": "a", "type": "delay", "params": { "for": "1m" } },
            { "id": "b", "type": "delay", "params": { "for": "1m" } },
            { "id": "c", "type": "delay", "params": { "for": "1m" } },
            { "id": "d", "type": "delay", "params": { "for": "1m" } }
          ],
          "edges": [
            { "from": "t", "to": "s" },
            { "from": "s", "port": "1", "to": "a" },
            { "from": "s", "port": "2", "to": "b" },
            { "from": "s", "port": "3", "to": "c" },
            { "from": "s", "port": "otherwise", "to": "d" }
          ]
        }"#,
        )
        .unwrap();
        let pos = auto_layout(&flow);
        let kind = &flow.node("s").unwrap().kind;
        assert_eq!(node_height(kind), 94.0);
        for ((from, port, to), route) in &routes(&flow, &pos) {
            let samples = route_samples(&route.d, 48);
            let (start, end) = (samples[0], *samples.last().unwrap());
            let pin_out = out_anchor(&flow.node(from).unwrap().kind, pos[from], port.as_deref());
            let pin_in = in_anchor(pos[to], height_of(&flow, to));
            // The path text keeps one decimal.
            let near = |a: Pt, b: Pt| (a.x - b.x).abs() < 0.06 && (a.y - b.y).abs() < 0.06;
            assert!(
                near(start, pin_out),
                "{from} {port:?}: {start:?} vs {pin_out:?}"
            );
            assert!(near(end, pin_in), "{to}: {end:?} vs {pin_in:?}");
        }
        // No two cards of the laid-out flow overlap, the tall switch included.
        let rects: Vec<Rect> = pos
            .iter()
            .map(|(id, at)| card(*at, height_of(&flow, id)))
            .collect();
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!intersects(*a, *b), "{a:?} {b:?}");
            }
        }
    }

    fn placed(flow: &mut Flow) {
        for (i, n) in flow.nodes.iter_mut().enumerate() {
            n.position = Some(Position {
                x: 10.0 * i as f64,
                y: 5.0,
            });
        }
    }

    #[test]
    fn stored_positions_win() {
        let mut flow = led();
        placed(&mut flow);
        let pos = positions(&flow);
        assert_eq!(pos.len(), 23);
        for (i, n) in flow.nodes.iter().enumerate() {
            assert_eq!(
                pos[&n.id],
                Pt {
                    x: 10.0 * i as f64,
                    y: 5.0
                }
            );
        }
    }

    #[test]
    fn partial_positions_autolayout() {
        let mut flow = led();
        placed(&mut flow);
        flow.nodes[3].position = None;
        assert_eq!(positions(&flow), auto_layout(&flow));
    }

    #[test]
    fn tidy_overwrites_positions() {
        let mut flow = led();
        placed(&mut flow);
        let auto = auto_layout(&flow);
        let tidied = tidy(&flow);
        for n in &tidied.nodes {
            let p = n.position.expect("tidy sets every position");
            assert_eq!(Pt { x: p.x, y: p.y }, auto[&n.id], "{}", n.id);
        }
    }

    #[test]
    fn resolve_positions_fills_only_the_missing_ones() {
        let mut flow = led();
        flow.nodes[0].position = Some(Position { x: 10.0, y: 20.0 });
        let auto = auto_layout(&flow);
        let resolved = with_resolved_positions(&flow);
        assert_eq!(
            resolved.nodes[0].position,
            Some(Position { x: 10.0, y: 20.0 })
        );
        for n in &resolved.nodes[1..] {
            let p = n.position.expect("filled");
            assert_eq!(Pt { x: p.x, y: p.y }, auto[&n.id], "{}", n.id);
        }
    }

    #[test]
    fn crossings_reduced() {
        let flow = Flow::decode(
            br#"{
          "id": "x", "name": "X",
          "nodes": [
            { "id": "a", "type": "trigger.state", "params": { "entity": "m" } },
            { "id": "b", "type": "trigger.state", "params": { "entity": "n" } },
            { "id": "c", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "d", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
          ],
          "edges": [ { "from": "a", "to": "d" }, { "from": "b", "to": "c" } ]
        }"#,
        )
        .unwrap();
        let pos = auto_layout(&flow);
        assert!(pos["a"].y < pos["b"].y);
        assert!(pos["d"].y < pos["c"].y, "{pos:?}");
        assert_eq!(pos["a"].x, PAD);
        assert_eq!(pos["c"].x, PAD + NODE_W + COL_GAP);
    }

    #[test]
    fn bounds_cover_every_card() {
        let pos = BTreeMap::from([
            ("a".to_owned(), Pt { x: 40.0, y: 100.0 }),
            ("b".to_owned(), Pt { x: 400.0, y: 10.0 }),
        ]);
        assert_eq!(
            bounds(&Flow::default(), &pos),
            Rect {
                x: 40.0,
                y: 10.0,
                w: 360.0 + NODE_W,
                h: 90.0 + NODE_H
            }
        );
        assert_eq!(
            bounds(&Flow::default(), &BTreeMap::new()),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0
            }
        );
    }

    #[test]
    fn clear_route_is_one_cubic_with_label_and_middle() {
        let r = route(Pt { x: 0.0, y: 0.0 }, Pt { x: 400.0, y: 0.0 }, &[]);
        assert_eq!(r.d, "M0.0 0.0 C200.0 0.0, 200.0 0.0, 400.0 0.0");
        assert_eq!(
            r.label_at,
            Pt {
                x: LABEL_DX,
                y: 0.0
            }
        );
        assert!((r.mid.x - 200.0).abs() < 1.0, "{:?}", r.mid);
    }

    #[test]
    fn if_labels_stay_apart_when_branches_bend_toward_each_other() {
        // `true` leaves the upper pin but goes down, `false` leaves the lower pin but goes up:
        // the two wires cross right after the card.
        let flow = Flow::decode(
            br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "check", "type": "if", "params": { "op": "eq", "value": true },
              "position": { "x": 0, "y": 200 } },
            { "id": "down", "type": "delay", "params": { "for": "1m" },
              "position": { "x": 328, "y": 320 } },
            { "id": "up", "type": "delay", "params": { "for": "1m" },
              "position": { "x": 328, "y": 80 } }
          ],
          "edges": [
            { "from": "check", "port": "true", "to": "down" },
            { "from": "check", "port": "false", "to": "up" }
          ]
        }"#,
        )
        .unwrap();
        let wires = routes(&flow, &positions(&flow));
        let label = |port: &str| {
            wires
                .iter()
                .find(|(k, _)| k.1.as_deref() == Some(port))
                .map(|(_, r)| r.label_at)
                .unwrap()
        };
        let (yes, no) = (label("true"), label("false"));
        assert!((no.y - yes.y).abs() >= 16.0, "yes {yes:?} no {no:?}");
        // Each label sits just right of its own pin.
        assert!(yes.x > NODE_W && yes.x < NODE_W + COL_GAP / 2.0, "{yes:?}");
    }

    #[test]
    fn blocked_route_takes_a_lane() {
        let block = Rect {
            x: 150.0,
            y: 74.0,
            w: NODE_W,
            h: NODE_H,
        };
        let from = Pt { x: 0.0, y: 100.0 };
        let to = Pt { x: 600.0, y: 100.0 };
        let r = route(from, to, &[block]);
        assert!(r.d.contains('L'), "{}", r.d);
        for p in route_samples(&r.d, 48) {
            assert!(!inside(p, block), "{p:?} in {}", r.d);
        }
        // Lane at the top, ROW_GAP/2 above the card (both lanes cost the same; top comes first).
        assert!(
            r.d.contains(&format!("{:.1}", block.y - ROW_GAP / 2.0)),
            "{}",
            r.d
        );
    }

    /// A trigger wired to a delay 700 px to its right, with a loose delay between them.
    fn wire_and_loose_card(loose_y: f64) -> Flow {
        Flow::decode(
            format!(
                r#"{{
          "id": "f", "name": "F",
          "nodes": [
            {{ "id": "t", "type": "trigger.state", "params": {{ "entity": "e" }},
              "position": {{ "x": 0, "y": 100 }} }},
            {{ "id": "d", "type": "delay", "params": {{ "for": "1m" }},
              "position": {{ "x": 700, "y": 100 }} }},
            {{ "id": "n", "type": "delay", "params": {{ "for": "1m" }},
              "position": {{ "x": 350, "y": {loose_y} }} }}
          ],
          "edges": [ {{ "from": "t", "to": "d" }} ]
        }}"#
            )
            .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn loose_card_leaves_the_wire_under_it() {
        let flow = wire_and_loose_card(90.0);
        let pos = positions(&flow);
        // Routed around it, the wire dodges the card; ignoring it, the wire runs straight under.
        assert_eq!(wire_under(&routes(&flow, &pos), pos["n"], NODE_H), None);
        let wires = routes_past(&flow, &pos, Some("n"));
        assert_eq!(
            wire_under(&wires, pos["n"], NODE_H),
            Some(("t".into(), None, "d".into()))
        );
    }

    #[test]
    fn card_away_from_the_wire_finds_none() {
        let flow = wire_and_loose_card(300.0);
        let pos = positions(&flow);
        assert_eq!(
            wire_under(&routes_past(&flow, &pos, Some("n")), pos["n"], NODE_H),
            None
        );
    }

    /// Three triggers into one `if` whose two exits go to two commands, then a plain chain.
    fn fan_flow() -> Flow {
        Flow::decode(
            br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "t1", "type": "trigger.state", "params": { "entity": "a" } },
            { "id": "t2", "type": "trigger.state", "params": { "entity": "b" } },
            { "id": "t3", "type": "trigger.state", "params": { "entity": "c" } },
            { "id": "check", "type": "if", "params": { "op": "eq", "value": true } },
            { "id": "yes", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "no", "type": "delay", "params": { "for": "1m" } },
            { "id": "off", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
          ],
          "edges": [
            { "from": "t1", "to": "check" }, { "from": "t2", "to": "check" }, { "from": "t3", "to": "check" },
            { "from": "check", "port": "true", "to": "yes" },
            { "from": "check", "port": "false", "to": "no" },
            { "from": "no", "to": "off" }
          ]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn fans_get_wider_gaps() {
        let pos = auto_layout(&fan_flow());
        // Fan-in: the triggers sit further apart, and so do their column and the if's.
        let mut ys: Vec<f64> = ["t1", "t2", "t3"].iter().map(|id| pos[*id].y).collect();
        ys.sort_by(f64::total_cmp);
        for w in ys.windows(2) {
            assert_eq!(w[1] - w[0], NODE_H + FAN_ROW_GAP, "{ys:?}");
        }
        assert_eq!(pos["check"].x - pos["t1"].x, NODE_W + FAN_COL_GAP);
        // Fan-out: the if's two exits sit further apart, one column further right.
        assert_eq!((pos["yes"].y - pos["no"].y).abs(), NODE_H + FAN_ROW_GAP);
        assert_eq!(pos["yes"].x - pos["check"].x, NODE_W + FAN_COL_GAP);
        // A plain one-to-one wire keeps the standard gap.
        assert_eq!(pos["off"].x - pos["no"].x, NODE_W + COL_GAP);
    }

    #[test]
    fn plain_column_keeps_the_standard_row_gap() {
        // Two independent chains: their cards share no neighbour and none fans.
        let flow = Flow::decode(
            br#"{
          "id": "p", "name": "P",
          "nodes": [
            { "id": "a", "type": "trigger.state", "params": { "entity": "a" } },
            { "id": "b", "type": "trigger.state", "params": { "entity": "b" } },
            { "id": "c", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "d", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
          ],
          "edges": [ { "from": "a", "to": "c" }, { "from": "b", "to": "d" } ]
        }"#,
        )
        .unwrap();
        let pos = auto_layout(&flow);
        assert_eq!((pos["b"].y - pos["a"].y).abs(), NODE_H + ROW_GAP);
        assert_eq!(pos["c"].x - pos["a"].x, NODE_W + COL_GAP);
    }

    #[test]
    fn fan_layout_has_no_overlaps_and_routes_avoid_cards() {
        let flow = fan_flow();
        let pos = auto_layout(&flow);
        for ((from, port, to), route) in &routes(&flow, &pos) {
            for p in route_samples(&route.d, 48) {
                for (id, at) in &pos {
                    if id != from && id != to {
                        assert!(
                            !inside(p, card(*at, NODE_H)),
                            "{from} -{port:?}-> {to} crosses {id}"
                        );
                    }
                }
            }
        }
    }
}
