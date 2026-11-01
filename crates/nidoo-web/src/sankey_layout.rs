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
mod tests {
    use super::*;
    use crate::energy_sankey::{SankeyLink, SankeyNode};

    fn node(id: &str, role: SankeyRole, column: u8, size: f64) -> SankeyNode {
        SankeyNode {
            id: id.into(),
            role,
            column,
            name: None,
            watts: size,
            size,
            sensor: None,
            count: 0,
            slot: 0,
        }
    }

    fn link(from: &str, to: &str, watts: f64) -> SankeyLink {
        SankeyLink {
            from: from.into(),
            to: to.into(),
            watts,
        }
    }

    /// Solar and grid feed the house, which feeds an export sink and two consumers.
    fn sample() -> Sankey {
        Sankey {
            nodes: vec![
                node("solar", SankeyRole::Solar, 0, 3000.0),
                node("grid", SankeyRole::Grid, 0, 1000.0),
                node("house", SankeyRole::House, 1, 3500.0),
                node("export", SankeyRole::Export, 1, 500.0),
                node("oven", SankeyRole::Consumer, 2, 2000.0),
                node("heat-pump", SankeyRole::Consumer, 2, 1500.0),
            ],
            links: vec![
                link("solar", "house", 2500.0),
                link("solar", "export", 500.0),
                link("grid", "house", 1000.0),
                link("house", "oven", 2000.0),
                link("house", "heat-pump", 1500.0),
            ],
            ..Sankey::default()
        }
    }

    fn bar<'a>(p: &'a Placed, id: &str) -> &'a Bar {
        p.bars.iter().find(|b| b.id == id).unwrap()
    }

    fn label<'a>(p: &'a Placed, id: &str) -> &'a Label {
        p.labels.iter().find(|b| b.id == id).unwrap()
    }

    #[test]
    fn paths_keep_one_shape() {
        for f in [WIDE, NARROW] {
            let p = place(&sample(), f);
            assert_eq!(p.bands.len(), 5);
            for b in &p.bands {
                assert!(b.path.starts_with("M "), "{}", b.path);
                assert!(b.path.ends_with(" Z"), "{}", b.path);
                assert_eq!(b.path.matches("C ").count(), 2, "{}", b.path);
                assert_eq!(b.path.matches("L ").count(), 1, "{}", b.path);
            }
        }
    }

    #[test]
    fn bars_fit_the_frame() {
        for f in [WIDE, NARROW] {
            let p = place(&sample(), f);
            assert_eq!(p.bars.len(), 6);
            for b in &p.bars {
                assert!(b.x >= 0.0 && b.y >= 0.0, "{b:?}");
                assert!(b.x + b.w <= f.w && b.y + b.h <= f.h, "{b:?}");
            }
        }
    }

    #[test]
    fn bands_never_outgrow_their_bar() {
        for f in [WIDE, NARROW] {
            let s = sample();
            let p = place(&s, f);
            // Recover each band's thickness from the path: the L command's far edge minus the
            // M command's start, along the cross axis.
            let thickness = |path: &str| -> f64 {
                let nums: Vec<f64> = path
                    .split(|c: char| c.is_alphabetic() || c == ' ')
                    .filter(|t| !t.is_empty())
                    .map(|t| t.parse().unwrap())
                    .collect();
                // M(0,1) C(2..7) L(8,9): cross index is 0 (vertical) or 1 (horizontal).
                let i = usize::from(!f.vertical);
                (nums[8 + i] - nums[2 + 2 + i]).abs()
            };
            for b in &p.bars {
                let len = if f.vertical { b.w } else { b.h };
                let out: f64 = p
                    .bands
                    .iter()
                    .filter(|x| x.from == b.id)
                    .map(|x| thickness(&x.path))
                    .sum();
                let inn: f64 = p
                    .bands
                    .iter()
                    .filter(|x| x.to == b.id)
                    .map(|x| thickness(&x.path))
                    .sum();
                assert!(out <= len + 0.5, "{} leaves {out} > {len}", b.id);
                assert!(inn <= len + 0.5, "{} enters {inn} > {len}", b.id);
            }
        }
    }

    #[test]
    fn house_mix_follows_the_bands_into_it() {
        for f in [WIDE, NARROW] {
            let p = place(&sample(), f);
            let mix: Vec<_> = p
                .house_mix
                .iter()
                .map(|m| (m.from.as_str(), m.start, m.end))
                .collect();
            // 2500 W of solar then 1000 W of grid fill the 3500 W house bar.
            assert_eq!(mix.len(), 2);
            assert_eq!((mix[0].0, mix[0].1), ("solar", 0.0));
            assert!((mix[0].2 - 2500.0 / 3500.0).abs() < 1e-9, "{mix:?}");
            assert_eq!(mix[1].0, "grid");
            assert!((mix[1].1 - mix[0].2).abs() < 1e-9, "{mix:?}");
            assert!((mix[1].2 - 1.0).abs() < 1e-9, "{mix:?}");
        }
    }

    #[test]
    fn house_mix_leaves_the_unsourced_rest_open() {
        let mut s = sample();
        // The house reads more than its sources feed it.
        s.nodes[2].size = 5000.0;
        let p = place(&s, WIDE);
        assert!((p.house_mix.last().unwrap().end - 3500.0 / 5000.0).abs() < 1e-9);
    }

    #[test]
    fn columns_sit_on_the_main_axis() {
        let p = place(&sample(), WIDE);
        assert_eq!(bar(&p, "solar").x, bar(&p, "grid").x);
        let p = place(&sample(), NARROW);
        assert_eq!(bar(&p, "solar").y, bar(&p, "grid").y);
        assert_eq!(bar(&p, "oven").y, bar(&p, "heat-pump").y);
    }

    #[test]
    fn vertical_swaps_axes() {
        let p = place(&sample(), NARROW);
        let h = bar(&p, "house");
        assert!(h.w > h.h, "{h:?}");
        let p = place(&sample(), WIDE);
        let h = bar(&p, "house");
        assert!(h.h > h.w, "{h:?}");
    }

    #[test]
    fn labels_follow_the_side_rules() {
        let p = place(&sample(), WIDE);
        let (solar, l) = (bar(&p, "solar"), label(&p, "solar"));
        assert_eq!(l.anchor, Anchor::End);
        assert!(l.x < solar.x);
        let (oven, l) = (bar(&p, "oven"), label(&p, "oven"));
        assert_eq!(l.anchor, Anchor::Start);
        assert!(l.x > oven.x + BAR);
        let (house, l) = (bar(&p, "house"), label(&p, "house"));
        assert_eq!(l.anchor, Anchor::Middle);
        assert!(l.y < house.y);
        let l = label(&p, "export");
        assert_eq!(l.anchor, Anchor::Start);

        let p = place(&sample(), NARROW);
        assert_eq!(label(&p, "oven").anchor, Anchor::Middle);
        assert_eq!(label(&p, "house").anchor, Anchor::Middle);
        let (oven, l) = (bar(&p, "oven"), label(&p, "oven"));
        assert!(l.y > oven.y + oven.h);
        assert!(label(&p, "house").y < bar(&p, "house").y);
        // Sources read above their bar, clear of the bands below it.
        for id in ["solar", "grid"] {
            assert!(label(&p, id).y + LINE < bar(&p, id).y, "{id}");
        }
        assert!(label(&p, "export").y > bar(&p, "export").y + BAR);
        assert!(label(&p, "solar").y > 0.0);
    }

    #[test]
    fn narrow_labels_stay_inside_the_frame() {
        // A tiny consumer first in its column, one last, a big one between: the column fills
        // the frame, so the tiny ones sit at its edges.
        let s = Sankey {
            nodes: vec![
                node("house", SankeyRole::House, 1, 1000.0),
                node("tiny", SankeyRole::Consumer, 2, 2.0),
                node("oven", SankeyRole::Consumer, 2, 1000.0),
                node("tiny2", SankeyRole::Consumer, 2, 2.0),
            ],
            links: vec![
                link("house", "tiny", 2.0),
                link("house", "oven", 1000.0),
                link("house", "tiny2", 2.0),
            ],
            ..Sankey::default()
        };
        let p = place(&s, NARROW);
        for l in &p.labels {
            let width = 2.0 * LABEL_HALF;
            let (from, to) = match l.anchor {
                Anchor::Start => (l.x, l.x + width),
                Anchor::Middle => (l.x - width / 2.0, l.x + width / 2.0),
                Anchor::End => (l.x - width, l.x),
            };
            assert!(from >= 0.0 && to <= NARROW.w, "{} {from}..{to}", l.id);
        }
        assert_eq!(label(&p, "tiny").anchor, Anchor::Start);
        assert_eq!(label(&p, "tiny2").anchor, Anchor::End);
    }

    #[test]
    fn narrow_labels_in_a_row_never_overlap() {
        // One big consumer and three thin ones: the thin bars sit closer than a label is wide.
        let s = Sankey {
            nodes: vec![
                node("grid", SankeyRole::Grid, 0, 3120.0),
                node("house", SankeyRole::House, 1, 3120.0),
                node("oven", SankeyRole::Consumer, 2, 3000.0),
                node("lamp", SankeyRole::Consumer, 2, 40.0),
                node("radio", SankeyRole::Consumer, 2, 40.0),
                node("fan", SankeyRole::Consumer, 2, 40.0),
            ],
            links: vec![
                link("grid", "house", 3120.0),
                link("house", "oven", 3000.0),
                link("house", "lamp", 40.0),
                link("house", "radio", 40.0),
                link("house", "fan", 40.0),
            ],
            ..Sankey::default()
        };
        let p = place(&s, NARROW);
        let span = |l: &Label| match l.anchor {
            Anchor::Start => (l.x, l.x + 2.0 * LABEL_HALF),
            Anchor::Middle => (l.x - LABEL_HALF, l.x + LABEL_HALF),
            Anchor::End => (l.x - 2.0 * LABEL_HALF, l.x),
        };
        for a in &p.labels {
            // Two lines from the baseline of the first; the frame holds both.
            assert!(
                a.y - LINE >= 0.0 && a.y + LINE <= NARROW.h,
                "{} {}",
                a.id,
                a.y
            );
            for b in &p.labels {
                if a.id >= b.id || (a.y - b.y).abs() >= 2.0 * LINE {
                    continue;
                }
                let ((a0, a1), (b0, b1)) = (span(a), span(b));
                assert!(a1 <= b0 || b1 <= a0, "{} overlaps {}", a.id, b.id);
            }
            // Still on its own side of the bar.
            assert!(a.y > bar(&p, &a.id).y || a.id == "house" || a.id == "grid");
        }
    }

    #[test]
    fn empty_model_places_nothing() {
        assert_eq!(place(&Sankey::default(), WIDE), Placed::default());
    }
}
