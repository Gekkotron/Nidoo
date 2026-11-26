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
