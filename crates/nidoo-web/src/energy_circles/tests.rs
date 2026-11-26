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
fn consumer_names_fit_beside_or_under_their_circle() {
    let wide = &CirclesLayout::WIDE;
    for at in wide.three.iter().chain(&wide.two).chain(&wide.one) {
        let room = label_room(wide, true, *at, wide.r_small);
        // Measured in Inter SemiBold 13 px: Climatisation 82, Pompe à chaleur 104.9.
        assert_eq!(
            fit_label("Climatisation".into(), room, 13.0),
            "Climatisation"
        );
        assert_eq!(
            fit_label("Pompe à chaleur".into(), room, 13.0),
            "Pompe à chaleur"
        );
    }
    let narrow = &CirclesLayout::NARROW;
    for at in &narrow.three {
        let room = label_room(narrow, false, *at, narrow.r_small);
        assert!(room <= 120.0, "{room}");
        // Measured at 12 px: Climatisation 75.7, Lave-vaisselle 83.6.
        assert_eq!(
            fit_label("Climatisation".into(), room, 12.0),
            "Climatisation"
        );
        assert_eq!(
            fit_label("Lave-vaisselle".into(), room, 12.0),
            "Lave-vaisselle"
        );
    }
}

#[test]
fn a_name_too_wide_is_ellipsised_within_its_room() {
    let cut = fit_label("Climatisation du salon".into(), 92.0, 13.0);
    assert!(
        cut.ends_with('…') && cut.starts_with("Climatisati"),
        "{cut}"
    );
    assert!(width_em(&cut) * 13.0 <= 92.0);
    assert_eq!(fit_label("WWWWWWWWWWWWW".into(), 60.0, 13.0), "WWW…");
    assert_eq!(fit_label("Four".into(), 60.0, 13.0), "Four");
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
        vec![(610.0, 80.0), (635.0, 214.0), (610.0, 348.0)]
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
