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
