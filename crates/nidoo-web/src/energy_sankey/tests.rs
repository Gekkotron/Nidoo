use super::*;
use nidoo_core::energy::NodeKind;

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

fn solar(w: f64) -> EnergyNode {
    node("solar", NodeKind::Solar, w, Flow::In)
}
fn grid(w: f64, flow: Flow) -> EnergyNode {
    node("grid", NodeKind::Grid, w, flow)
}
fn battery(w: f64, flow: Flow) -> EnergyNode {
    node("battery", NodeKind::Battery, w, flow)
}
fn house(w: f64) -> EnergyNode {
    node("house", NodeKind::House, w, Flow::In)
}

fn link_w(s: &Sankey, from: &str, to: &str) -> Option<f64> {
    s.links
        .iter()
        .find(|l| l.from == from && l.to == to)
        .map(|l| l.watts)
}
fn find<'a>(s: &'a Sankey, id: &str) -> Option<&'a SankeyNode> {
    s.nodes.iter().find(|n| n.id == id)
}

#[test]
fn import_only() {
    let oven = consumer("Oven", 300.0);
    let lamp = consumer("Lamp", 5.0);
    let u = ui(
        &[grid(500.0, Flow::In), house(500.0)],
        &[oven.clone(), lamp],
    );
    let s = build(&u, FOLD_WIDE);
    assert_eq!(link_w(&s, "grid", "house"), Some(500.0));
    assert_eq!(link_w(&s, "house", &oven.id), Some(300.0));
    assert_eq!(link_w(&s, "house", "unitemised"), Some(200.0));
    assert!(s.nodes.iter().all(|n| n.id != "consumer:lamp"));
    assert_eq!(s.import_w, Some(500.0));
    assert_eq!(s.solar_share, None);
}

#[test]
fn solar_surplus_exports() {
    let u = ui(
        &[solar(3000.0), grid(1200.0, Flow::Out), house(1800.0)],
        &[consumer("Heater", 900.0)],
    );
    let s = build(&u, FOLD_WIDE);
    assert_eq!(link_w(&s, "solar", "house"), Some(1800.0));
    assert_eq!(link_w(&s, "solar", "export"), Some(1200.0));
    let e = find(&s, "export").unwrap();
    assert_eq!((e.column, e.size), (1, 1200.0));
    assert_eq!(find(&s, "unitemised").unwrap().watts, 900.0);
    assert_eq!(s.solar_share, Some(100));
}

#[test]
fn solar_share_does_not_lose_a_percent_to_rounding() {
    // 290 W of 1000 W: 290 / 1000 * 100 is 28.999…
    let u = ui(&[solar(290.0), grid(710.0, Flow::In), house(1000.0)], &[]);
    let s = build(&u, FOLD_WIDE);
    assert_eq!(link_w(&s, "solar", "house"), Some(290.0));
    assert_eq!(s.solar_share, Some(29));
}

#[test]
fn charge_from_solar_then_grid() {
    let u = ui(
        &[
            solar(300.0),
            grid(400.0, Flow::In),
            battery(500.0, Flow::Out),
            house(200.0),
        ],
        &[],
    );
    let s = build(&u, FOLD_WIDE);
    assert_eq!(link_w(&s, "solar", "battery-charge"), Some(300.0));
    assert_eq!(link_w(&s, "grid", "battery-charge"), Some(200.0));
    assert_eq!(link_w(&s, "grid", "house"), Some(200.0));
    assert_eq!(s.solar_share, Some(0));
}

#[test]
fn battery_feeds_export_without_solar() {
    let u = ui(
        &[
            battery(800.0, Flow::In),
            grid(300.0, Flow::Out),
            house(500.0),
        ],
        &[],
    );
    let s = build(&u, FOLD_WIDE);
    assert_eq!(link_w(&s, "battery", "export"), Some(300.0));
    assert_eq!(link_w(&s, "battery", "house"), Some(500.0));
}

#[test]
fn night_with_solar_offline() {
    let mut sol = solar(0.0);
    sol.value = None;
    sol.status = NodeStatus::Unavailable;
    let u = ui(&[sol, grid(250.0, Flow::In), house(250.0)], &[]);
    let s = build(&u, FOLD_WIDE);
    assert!(find(&s, "solar").is_none());
    assert_eq!(link_w(&s, "grid", "house"), Some(250.0));
    assert_eq!(s.solar_w, None);
}

#[test]
fn house_meter_lower_than_inflow() {
    let u = ui(
        &[grid(1000.0, Flow::In), house(900.0)],
        &[consumer("Oven", 950.0)],
    );
    let s = build(&u, FOLD_WIDE);
    let h = find(&s, "house").unwrap();
    assert_eq!((h.size, h.watts), (1000.0, 900.0));
    assert!(find(&s, "unitemised").is_none());
}

#[test]
fn wide_folds_after_eight() {
    let cs: Vec<EnergyNode> = (0..9)
        .map(|i| consumer(&format!("Load {i}"), 500.0 - 50.0 * i as f64))
        .collect();
    let u = ui(&[grid(5000.0, Flow::In), house(5000.0)], &cs);
    let s = build(&u, FOLD_WIDE);
    let n = s
        .nodes
        .iter()
        .filter(|n| n.role == SankeyRole::Consumer)
        .count();
    assert_eq!(n, 7);
    assert_eq!(find(&s, "others").unwrap().count, 2);
    assert!(s.folded);
}

#[test]
fn narrow_folds_at_four() {
    let cs: Vec<EnergyNode> = (0..12)
        .map(|i| consumer(&format!("Load {i}"), 100.0))
        .collect();
    let u = ui(&[grid(1500.0, Flow::In), house(1500.0)], &cs);
    let s = build(&u, FOLD_NARROW);
    let n = s
        .nodes
        .iter()
        .filter(|n| n.role == SankeyRole::Consumer)
        .count();
    assert_eq!(n, 3);
    let o = find(&s, "others").unwrap();
    assert_eq!((o.count, o.watts), (9, 900.0));
    assert_eq!(find(&s, "unitemised").unwrap().watts, 300.0);
}

#[test]
fn every_drawn_node_balances() {
    let fixtures = vec![
        ui(
            &[grid(500.0, Flow::In), house(500.0)],
            &[consumer("Oven", 300.0)],
        ),
        ui(
            &[solar(3000.0), grid(1200.0, Flow::Out), house(1800.0)],
            &[consumer("Heater", 900.0)],
        ),
        ui(
            &[
                solar(300.0),
                grid(400.0, Flow::In),
                battery(500.0, Flow::Out),
                house(200.0),
            ],
            &[],
        ),
        ui(
            &[
                battery(800.0, Flow::In),
                grid(300.0, Flow::Out),
                house(500.0),
            ],
            &[],
        ),
        ui(
            &[grid(1000.0, Flow::In), house(900.0)],
            &[consumer("Oven", 950.0)],
        ),
    ];
    for u in &fixtures {
        let s = build(u, FOLD_WIDE);
        for n in &s.nodes {
            let i: f64 = s
                .links
                .iter()
                .filter(|l| l.to == n.id)
                .map(|l| l.watts)
                .sum();
            let o: f64 = s
                .links
                .iter()
                .filter(|l| l.from == n.id)
                .map(|l| l.watts)
                .sum();
            match n.column {
                1 => assert!(i <= n.size + 1e-9 && o <= n.size + 1e-9, "{}", n.id),
                0 => assert!(o <= n.watts + 1e-9, "{}", n.id),
                _ => {}
            }
        }
    }
}

#[test]
fn unknown_grid_has_no_import() {
    let s = build(&ui(&[solar(800.0), house(800.0)], &[]), FOLD_WIDE);
    assert_eq!(s.import_w, None);
    let s = build(
        &ui(
            &[solar(3000.0), grid(1200.0, Flow::Out), house(1800.0)],
            &[],
        ),
        FOLD_WIDE,
    );
    assert_eq!(s.import_w, Some(0.0));
}

#[test]
fn nothing_known_is_empty() {
    let mut h = house(0.0);
    h.value = None;
    let s = build(&ui(&[h], &[]), FOLD_WIDE);
    assert!(s.is_empty());
    assert_eq!(s.house_w, None);
}
