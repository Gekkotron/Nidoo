//! Geometry of the Sankey model: where each bar, band and label sits in an SVG frame, horizontal
//! on wide screens and vertical on phones. Pure; `screens::energy_flux` draws it.

use std::collections::HashMap;

use crate::energy_sankey::{Sankey, SankeyRole};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub w: f64,
    pub h: f64,
    /// Columns are rows, bands flow downward.
    pub vertical: bool,
}

pub const WIDE: Frame = Frame {
    w: 980.0,
    h: 460.0,
    vertical: false,
};
pub const NARROW: Frame = Frame {
    w: 360.0,
    h: 620.0,
    vertical: true,
};
/// Bar thickness across the main axis.
pub const BAR: f64 = 16.0;
/// Line height of a two-line narrow label (value, then name).
pub const LINE: f64 = 14.0;
/// Half the widest narrow label (12 characters at ~7 px), so one centred on a bar stays inside.
pub const LABEL_HALF: f64 = 44.0;
/// How far a narrow label steps away from its bar to clear the one beside it.
const ROW_STEP: f64 = 2.0 * LINE + 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    /// `"{from}>{to}"`.
    pub id: String,
    pub from: String,
    pub to: String,
    /// Always the same five commands so CSS can interpolate it between frames.
    pub path: String,
    /// Gradient axis: centre line from the source bar's edge to the target bar's edge.
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub anchor: Anchor,
}

/// The stretch of the house bar a source's band lands on, as shares of the bar's length.
#[derive(Clone, Debug, PartialEq)]
pub struct MixStop {
    pub from: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Placed {
    pub bars: Vec<Bar>,
    pub bands: Vec<Band>,
    pub labels: Vec<Label>,
    /// The house bar painted in what flows into it, from its top (left) end.
    pub house_mix: Vec<MixStop>,
}

/// Where a column sits along the main axis, as a share of it.
fn column_share(column: u8, vertical: bool) -> f64 {
    let shares = if vertical {
        // Room above the sources for a second row of labels.
        [0.14, 0.46, 0.78]
    } else {
        [0.17, 0.49, 0.79]
    };
    shares[(column as usize).min(2)]
}

/// A coordinate for an SVG path: one decimal at most, no trailing ".0", never "-0".
fn n(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    format!("{}", if r == 0.0 { 0.0 } else { r })
}

/// A bar's position on the cross axis and its length.
struct Slot {
    cross: f64,
    len: f64,
    main: f64,
}

pub fn place(s: &Sankey, frame: Frame) -> Placed {
    let v = frame.vertical;
    let (main_size, cross_size) = if v {
        (frame.h, frame.w)
    } else {
        (frame.w, frame.h)
    };
    let (margin, gap) = if v { (16.0, 12.0) } else { (30.0, 20.0) };
    let space = cross_size - 2.0 * margin;

    // One scale for every column so equal watts look equal.
    let mut k = f64::INFINITY;
    for c in 0..3u8 {
        let col: Vec<_> = s.nodes.iter().filter(|nd| nd.column == c).collect();
        let sum: f64 = col.iter().map(|nd| nd.size).sum();
        if col.is_empty() || sum <= 0.0 {
            continue;
        }
        k = k.min((space - gap * (col.len() as f64 - 1.0)) / sum);
    }
    if !k.is_finite() || k <= 0.0 {
        return Placed::default();
    }

    let mut slots: HashMap<&str, Slot> = HashMap::new();
    for c in 0..3u8 {
        let col: Vec<_> = s.nodes.iter().filter(|nd| nd.column == c).collect();
        if col.is_empty() {
            continue;
        }
        let lens: Vec<f64> = col.iter().map(|nd| (nd.size * k).max(3.0)).collect();
        let total = lens.iter().sum::<f64>() + gap * (col.len() as f64 - 1.0);
        let mut cross = margin + (space - total) / 2.0;
        let main = column_share(c, v) * main_size;
        for (nd, len) in col.iter().zip(lens) {
            slots.insert(nd.id.as_str(), Slot { cross, len, main });
            cross += len + gap;
        }
    }

    let mut out = Placed::default();
    for nd in &s.nodes {
        let Some(sl) = slots.get(nd.id.as_str()) else {
            continue;
        };
        let (x, y, w, h) = if v {
            (sl.cross, sl.main, sl.len, BAR)
        } else {
            (sl.main, sl.cross, BAR, sl.len)
        };
        out.bars.push(Bar {
            id: nd.id.clone(),
            x,
            y,
            w,
            h,
        });

        let house = nd.role == SankeyRole::House;
        let (lx, ly, anchor) = if v {
            // Two lines. Sources and the house read above their bar, sinks below.
            let cx = x + w / 2.0;
            let ly = if house || nd.column == 0 {
                y - 8.0 - LINE
            } else {
                y + h + 16.0
            };
            // Near an edge the label hangs inward rather than being clipped.
            if cx - LABEL_HALF < 0.0 {
                (x, ly, Anchor::Start)
            } else if cx + LABEL_HALF > frame.w {
                (x + w, ly, Anchor::End)
            } else {
                (cx, ly, Anchor::Middle)
            }
        } else if house {
            (x + w / 2.0, y - 12.0, Anchor::Middle)
        } else if nd.column == 0 {
            (x - 14.0, y + h / 2.0, Anchor::End)
        } else {
            (x + w + 14.0, y + h / 2.0, Anchor::Start)
        };
        out.labels.push(Label {
            id: nd.id.clone(),
            x: lx,
            y: ly,
            anchor,
        });
    }

    if v {
        stagger(&mut out);
    }

    // Bands stack from the top (left) of each bar in link order.
    let mut leaving: HashMap<&str, f64> = HashMap::new();
    let mut entering: HashMap<&str, f64> = HashMap::new();
    for l in &s.links {
        let (Some(a), Some(b)) = (slots.get(l.from.as_str()), slots.get(l.to.as_str())) else {
            continue;
        };
        let t = (l.watts * k).max(1.5);
        let out_off = leaving.entry(l.from.as_str()).or_insert(0.0);
        let c0 = a.cross + *out_off;
        *out_off += t;
        let in_off = entering.entry(l.to.as_str()).or_insert(0.0);
        let c1 = b.cross + *in_off;
        *in_off += t;
        if s.nodes
            .iter()
            .any(|nd| nd.id == l.to && nd.role == SankeyRole::House)
        {
            let share = |c: f64| ((c - b.cross) / b.len).clamp(0.0, 1.0);
            out.house_mix.push(MixStop {
                from: l.from.clone(),
                start: share(c1),
                end: share(c1 + t),
            });
        }
        let (m0, m1) = (a.main + BAR, b.main);
        let mm = (m0 + m1) / 2.0;
        // Horizontal: (main, cross) = (x, y). Vertical swaps them.
        let pt = |m: f64, c: f64| {
            if v {
                format!("{} {}", n(c), n(m))
            } else {
                format!("{} {}", n(m), n(c))
            }
        };
        let path = format!(
            "M {} C {} {} {} L {} C {} {} {} Z",
            pt(m0, c0),
            pt(mm, c0),
            pt(mm, c1),
            pt(m1, c1),
            pt(m1, c1 + t),
            pt(mm, c1 + t),
            pt(mm, c0 + t),
            pt(m0, c0 + t),
        );
        let (x0, y0, x1, y1) = if v {
            (c0 + t / 2.0, m0, c1 + t / 2.0, m1)
        } else {
            (m0, c0 + t / 2.0, m1, c1 + t / 2.0)
        };
        out.bands.push(Band {
            id: format!("{}>{}", l.from, l.to),
            from: l.from.clone(),
            to: l.to.clone(),
            path,
            x0,
            y0,
            x1,
            y1,
        });
    }
    out
}

/// The x range a narrow label may cover.
fn span(l: &Label) -> (f64, f64) {
    match l.anchor {
        Anchor::Start => (l.x, l.x + 2.0 * LABEL_HALF),
        Anchor::Middle => (l.x - LABEL_HALF, l.x + LABEL_HALF),
        Anchor::End => (l.x - 2.0 * LABEL_HALF, l.x),
    }
}

/// Narrow labels side by side would overlap over thin bars: each one that would steps a row
/// further from its bar, so a run of thin bars reads as a zigzag. Three rows fit below a bar and
/// two above (the frame's top margin). Labels and bars share an index.
fn stagger(p: &mut Placed) {
    let mut order: Vec<usize> = (0..p.labels.len()).collect();
    order.sort_by(|&a, &b| {
        let (la, lb) = (&p.labels[a], &p.labels[b]);
        la.y.total_cmp(&lb.y)
            .then(span(la).0.total_cmp(&span(lb).0))
    });
    // Per row of labels (keyed by its first baseline), where each step ends on the x axis.
    let mut rows: Vec<(f64, Vec<f64>)> = Vec::new();
    for i in order {
        let (y, (from, to)) = (p.labels[i].y, span(&p.labels[i]));
        let above = y < p.bars[i].y;
        let row = match rows.iter().position(|r| r.0 == y) {
            Some(r) => r,
            None => {
                let steps = if above { 2 } else { 3 };
                rows.push((y, vec![f64::NEG_INFINITY; steps]));
                rows.len() - 1
            }
        };
        let ends = &mut rows[row].1;
        // The first step with room, else the one that ends earliest.
        let step = ends.iter().position(|&e| from >= e).unwrap_or_else(|| {
            (0..ends.len())
                .min_by(|&a, &b| ends[a].total_cmp(&ends[b]))
                .unwrap_or(0)
        });
        ends[step] = to;
        let shift = step as f64 * ROW_STEP;
        p.labels[i].y += if above { -shift } else { shift };
    }
}

#[cfg(test)]
mod tests;
