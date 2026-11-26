use super::super::testkit::e;
use super::*;

fn p(id: &str, role: EnergyRole, v: f64) -> EnergyEntity {
    EnergyEntity {
        role: Some(role),
        value: Some(v),
        ..e(id)
    }
}

fn consumer(id: &str, name: &str, v: f64) -> EnergyEntity {
    EnergyEntity {
        name: name.into(),
        ..p(id, EnergyRole::Consumer, v)
    }
}

fn build(all: &[EnergyEntity]) -> EnergyUi {
    EnergyFlow::build(all, 0, None)
}

/// Signed grid, solar, two consumers, derived house.
fn house() -> Vec<EnergyEntity> {
    vec![
        p("grid", EnergyRole::Grid, -400.0),
        p("solar", EnergyRole::Solar, 3200.0),
        consumer("ac", "AC", 600.0),
        consumer("heater", "Water heater", 1700.0),
    ]
}

fn set(all: &mut [EnergyEntity], id: &str, v: f64) {
    all.iter_mut().find(|x| x.id == id).unwrap().value = Some(v);
}

fn kinds_ids(ui: &EnergyUi) -> Vec<&str> {
    ui.diagram.iter().map(|n| n.id.as_str()).collect()
}

#[test]
fn empty_and_loading() {
    let ui = build(&[]);
    assert!(ui.loading && !ui.empty);
    let ui = build(&[e("t")]);
    assert!(!ui.loading && ui.empty);
    assert!(ui.diagram.is_empty());
}

#[test]
fn derived_house_sums_inputs() {
    let ui = build(&house());
    assert_eq!(ui.node("house").unwrap().value, Some(2800.0));
    assert_eq!(ui.node("grid").unwrap().value, Some(400.0));
    assert_eq!(ui.node("grid").unwrap().flow, Flow::Out);
    assert_eq!(ui.node("solar").unwrap().flow, Flow::In);
    assert_eq!(ui.self_consumption, Some(87));
    assert_eq!(ui.hint, None);
    assert_eq!(
        kinds_ids(&ui),
        ["solar", "grid", "house", "consumer:heater", "consumer:ac"]
    );
}

#[test]
fn measured_house_wins() {
    let mut all = house();
    all.push(p("load", EnergyRole::House, 2500.0));
    let ui = build(&all);
    assert_eq!(ui.node("house").unwrap().value, Some(2500.0));
    assert_eq!(ui.node("house").unwrap().sensor_ids, ["load"]);
    assert!(build(&house()).node("house").unwrap().sensor_ids.is_empty());
}

#[test]
fn inverts_signed_roles_only() {
    let grid = EnergyEntity {
        invert: true,
        ..p("grid", EnergyRole::Grid, 400.0)
    };
    assert_eq!(build(&[grid]).node("grid").unwrap().flow, Flow::Out);
    let solar = EnergyEntity {
        invert: true,
        ..p("solar", EnergyRole::Solar, 1000.0)
    };
    assert_eq!(build(&[solar]).node("solar").unwrap().value, Some(1000.0));
}

#[test]
fn grid_minus_export() {
    let ui = build(&[
        p("imp", EnergyRole::Grid, 0.0),
        p("exp", EnergyRole::GridExport, 700.0),
        p("solar", EnergyRole::Solar, 2000.0),
    ]);
    assert_eq!(ui.node("grid").unwrap().value, Some(700.0));
    assert_eq!(ui.node("grid").unwrap().flow, Flow::Out);
    assert_eq!(ui.node("house").unwrap().value, Some(1300.0));
    assert_eq!(ui.node("grid").unwrap().sensor_ids, ["exp", "imp"]);
}

#[test]
fn export_alone_is_a_negative_grid() {
    let ui = build(&[p("exp", EnergyRole::GridExport, 700.0)]);
    assert_eq!(ui.node("grid").unwrap().flow, Flow::Out);
}

#[test]
fn battery_directions() {
    let grid = p("grid", EnergyRole::Grid, 100.0);
    let ui = build(&[p("bat", EnergyRole::Battery, 300.0), grid.clone()]);
    assert_eq!(ui.node("battery").unwrap().flow, Flow::In);
    assert_eq!(ui.node("house").unwrap().value, Some(400.0));
    let ui = build(&[p("bat", EnergyRole::Battery, -300.0), grid.clone()]);
    assert_eq!(ui.node("battery").unwrap().flow, Flow::Out);
    assert_eq!(ui.node("battery").unwrap().value, Some(300.0));
    let ui = build(&[p("bat", EnergyRole::Battery, 5.0), grid]);
    assert_eq!(ui.node("battery").unwrap().flow, Flow::Idle);
}

#[test]
fn units_are_converted_to_watts() {
    let solar = EnergyEntity {
        unit: Some("kW".into()),
        ..p("solar", EnergyRole::Solar, 3.2)
    };
    assert_eq!(build(&[solar]).node("solar").unwrap().value, Some(3200.0));
}

#[test]
fn bad_unit_is_unknown() {
    let grid = EnergyEntity {
        unit: Some("kWh".into()),
        ..p("grid", EnergyRole::Grid, 6584.0)
    };
    let ui = build(&[grid]);
    let n = ui.node("grid").unwrap();
    assert_eq!(n.value, None);
    assert_eq!(n.flow, Flow::Idle);
    let solar = EnergyEntity {
        unit: Some("kWh".into()),
        ..p("solar", EnergyRole::Solar, 3.2)
    };
    let ui = build(&[solar, p("grid", EnergyRole::Grid, 100.0)]);
    assert_eq!(ui.node("solar").unwrap().value, None);
    assert_eq!(ui.node("house").unwrap().value, None);
}

#[test]
fn summed_roles_add_up() {
    let ui = build(&[
        p("s1", EnergyRole::Solar, 1000.0),
        p("s2", EnergyRole::Solar, 500.0),
    ]);
    assert_eq!(ui.node("solar").unwrap().value, Some(1500.0));
    assert_eq!(
        ui.diagram
            .iter()
            .filter(|n| n.kind == NodeKind::Solar)
            .count(),
        1
    );
}

#[test]
fn negative_house_is_zero_with_hint() {
    let ui = build(&[
        p("grid", EnergyRole::Grid, -3000.0),
        p("solar", EnergyRole::Solar, 1000.0),
    ]);
    assert_eq!(ui.node("house").unwrap().value, Some(0.0));
    assert_eq!(ui.hint, Some(EnergyHint::CheckSign));
}

#[test]
fn unknown_input_makes_house_unknown() {
    let mut all = house();
    all[1].value = None;
    let ui = build(&all);
    assert_eq!(ui.node("house").unwrap().value, None);
    assert_eq!(ui.hint, None);
    assert_eq!(ui.self_consumption, None);
    assert!(
        ui.diagram
            .iter()
            .filter(|n| n.kind == NodeKind::Solar)
            .all(|n| n.flow == Flow::Idle)
    );
}

#[test]
fn unavailable_input_leaves_house_unknown_not_unavailable() {
    let mut all = house();
    set(&mut all, "grid", 900.0);
    set(&mut all, "solar", 0.0);
    all[1].available = false;
    let ui = build(&all);
    assert_eq!(ui.node("solar").unwrap().status, NodeStatus::Unavailable);
    assert_eq!(ui.node("house").unwrap().value, None);
    assert_eq!(ui.node("house").unwrap().status, NodeStatus::Live);
}

#[test]
fn unavailable_consumer_is_unknown_and_flagged() {
    let mut all = house();
    all[2].available = false;
    let ui = build(&all);
    let n = ui.node("consumer:ac").unwrap();
    assert_eq!((n.value, n.status), (None, NodeStatus::Unavailable));
}

#[test]
fn self_consumption_hidden_under_50w() {
    let mut all = house();
    set(&mut all, "grid", 300.0);
    set(&mut all, "solar", 49.0);
    assert_eq!(build(&all).self_consumption, None);
    set(&mut all, "solar", 50.0);
    assert_eq!(build(&all).self_consumption, Some(100));
    set(&mut all, "solar", 0.0);
    assert_eq!(build(&all).self_consumption, None);
}

#[test]
fn edges_below_10w_are_idle_and_negative_consumers_clamp() {
    let mut all = house();
    set(&mut all, "grid", 5.0);
    set(&mut all, "ac", 9.0);
    let ui = build(&all);
    assert_eq!(ui.node("grid").unwrap().flow, Flow::Idle);
    assert_eq!(ui.node("consumer:ac").unwrap().flow, Flow::Idle);
    assert_eq!(ui.node("consumer:heater").unwrap().flow, Flow::Out);
    set(&mut all, "ac", -3.0);
    let n = build(&all);
    assert_eq!(n.node("consumer:ac").unwrap().value, Some(0.0));
}

#[test]
fn folds_four_consumers_into_more() {
    let mut all = house();
    all.push(consumer("ev", "EV", 3000.0));
    all.push(consumer("pump", "Pump", 50.0));
    let ui = build(&all);
    let arc: Vec<&str> = ui
        .diagram
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Consumer | NodeKind::More))
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(arc, ["consumer:ev", "consumer:heater", "more"]);
    let more = ui.node("more");
    // `more` is a diagram node only; `node` finds it there.
    let more = more.unwrap();
    assert_eq!(more.count, 2);
    assert_eq!(more.value, Some(650.0));
    assert_eq!(more.sensor_ids, ["ac", "pump"]);
    let order: Vec<&str> = ui.consumers.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        order,
        [
            "consumer:ev",
            "consumer:heater",
            "consumer:ac",
            "consumer:pump"
        ]
    );
    assert_eq!(
        ui.cards
            .iter()
            .filter(|n| n.kind == NodeKind::Consumer)
            .count(),
        4
    );
}

#[test]
fn override_order_comes_before_power_order() {
    let ac = EnergyEntity {
        order: Some(1),
        ..consumer("ac", "AC", 10.0)
    };
    let ui = build(&[ac, consumer("heater", "Heater", 2000.0)]);
    let ids: Vec<&str> = ui.consumers.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["consumer:ac", "consumer:heater"]);
}

#[test]
fn ties_and_unknowns_sort_by_name_with_unknown_last() {
    let mut unknown = consumer("a", "A", 0.0);
    unknown.value = None;
    let ui = build(&[
        unknown,
        consumer("c", "C", 100.0),
        consumer("b", "B", 100.0),
    ]);
    let ids: Vec<&str> = ui.consumers.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["consumer:b", "consumer:c", "consumer:a"]);
}

#[test]
fn order_is_throttled() {
    let mut all = house();
    let first = EnergyFlow::build(&all, 0, None);
    let ids =
        |ui: &EnergyUi| -> Vec<String> { ui.consumers.iter().map(|n| n.id.clone()).collect() };
    assert_eq!(ids(&first), ["consumer:heater", "consumer:ac"]);
    set(&mut all, "ac", 2500.0);
    let kept = EnergyFlow::build(&all, 2_000, first.order.as_ref());
    assert_eq!(ids(&kept), ["consumer:heater", "consumer:ac"]);
    assert_eq!(kept.order.as_ref().unwrap().at_ms, 0);
    let kept = EnergyFlow::build(&all, 4_999, first.order.as_ref());
    assert_eq!(ids(&kept), ["consumer:heater", "consumer:ac"]);
    let fresh = EnergyFlow::build(&all, 6_000, first.order.as_ref());
    assert_eq!(ids(&fresh), ["consumer:ac", "consumer:heater"]);
    assert_eq!(fresh.order.as_ref().unwrap().at_ms, 6_000);
    let at_limit = EnergyFlow::build(&all, 5_000, first.order.as_ref());
    assert_eq!(ids(&at_limit), ["consumer:ac", "consumer:heater"]);
}

#[test]
fn order_is_recomputed_when_the_set_changes() {
    let all = house();
    let first = EnergyFlow::build(&all, 0, None);
    let fewer: Vec<EnergyEntity> = all.into_iter().filter(|x| x.id != "heater").collect();
    let ui = EnergyFlow::build(&fewer, 1_000, first.order.as_ref());
    assert_eq!(ui.consumers.len(), 1);
    assert_eq!(ui.order.unwrap().at_ms, 1_000);
}

#[test]
fn consumer_icon_and_name_reach_the_node() {
    let oven = EnergyEntity {
        icon: Some(ConsumerIcon::Oven),
        ..consumer("oven", "Oven", 900.0)
    };
    let ui = build(&[oven]);
    let n = ui.node("consumer:oven").unwrap();
    assert_eq!(n.icon, Some(ConsumerIcon::Oven));
    assert_eq!(n.name.as_deref(), Some("Oven"));
}

#[test]
fn consumers_alone_still_show_an_unknown_house() {
    let ui = build(&[consumer("ac", "AC", 300.0)]);
    assert_eq!(ui.node("house").unwrap().value, None);
    assert!(ui.diagram.iter().any(|n| n.id == "consumer:ac"));
}

#[test]
fn a_consumer_named_like_a_role_does_not_collide() {
    let ui = build(&[
        p("grid", EnergyRole::Grid, 500.0),
        consumer("house", "House plug", 200.0),
    ]);
    let ids: BTreeSet<&str> = ui.cards.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids.len(), ui.cards.len());
    assert_eq!(ui.node("consumer:house").unwrap().value, Some(200.0));
}

fn water_entity(id: &str, unit: &str, v: f64, device: Option<&str>) -> EnergyEntity {
    EnergyEntity {
        unit: Some(unit.into()),
        device_id: device.map(String::from),
        ..p(id, EnergyRole::Water, v)
    }
}

#[test]
fn water_takes_bottom_left_only_without_battery() {
    let mut all = house();
    all.push(water_entity("water", "L", 312.0, None));
    let ui = build(&all);
    let w = ui.node("water").unwrap();
    assert_eq!((w.value, w.unit.as_deref()), (Some(312.0), Some("L")));
    assert!(ui.diagram.iter().any(|n| n.id == "water"));
    all.push(p("bat", EnergyRole::Battery, 0.0));
    let ui = build(&all);
    assert!(ui.diagram.iter().all(|n| n.id != "water"));
    assert!(ui.cards.iter().any(|n| n.id == "water"));
}

#[test]
fn a_water_card_carries_the_m3_meter_of_its_device() {
    let all = vec![
        water_entity("daily", "L", 172.0, Some("suez")),
        EnergyEntity {
            device_id: Some("suez".into()),
            unit: Some("m³".into()),
            value: Some(601.359),
            ..e("index")
        },
    ];
    let w = build(&all).node("water").cloned().unwrap();
    assert_eq!(w.value, Some(172.0));
    assert_eq!(w.water_total, Some(601.359));
    assert_eq!(w.water_total_unit.as_deref(), Some("m³"));
}

#[test]
fn a_water_card_on_the_meter_itself_or_alone_has_no_total() {
    let all = vec![
        water_entity("index", "m³", 601.0, Some("suez")),
        EnergyEntity {
            device_id: Some("suez".into()),
            unit: Some("m3".into()),
            value: Some(1.0),
            ..e("other")
        },
    ];
    assert_eq!(build(&all).node("water").unwrap().water_total, None);
    let alone = [water_entity("daily", "L", 172.0, Some("suez"))];
    assert_eq!(build(&alone).node("water").unwrap().water_total_unit, None);
}

#[test]
fn an_unavailable_water_meter_has_no_total_but_keeps_its_unit() {
    let all = vec![
        water_entity("daily", "L", 172.0, Some("suez")),
        EnergyEntity {
            device_id: Some("suez".into()),
            unit: Some("m³".into()),
            value: Some(601.0),
            available: false,
            ..e("index")
        },
    ];
    let w = build(&all).node("water").cloned().unwrap();
    assert_eq!(w.water_total, None);
    assert_eq!(w.water_total_unit.as_deref(), Some("m³"));
}

/// The live house: grid on `power_a`, solar on `power_b` of one clamp, with its counters.
fn clamp(extra: impl Fn(&str) -> Option<f64>) -> Vec<EnergyEntity> {
    let dev = "zigbee2mqtt_0xa4c13898428fae3a";
    let mk = |kind: &str, unit: &str, v: f64, role: Option<EnergyRole>| EnergyEntity {
        device_id: Some(dev.into()),
        label: Some(kind.into()),
        unit: Some(unit.into()),
        value: extra(kind).or(Some(v)),
        role,
        ..e(&format!("0xa4c13898428fae3a_{kind}_zigbee2mqtt"))
    };
    vec![
        mk("power_a", "W", 300.0, Some(EnergyRole::Grid)),
        mk("power_b", "W", 500.0, Some(EnergyRole::Solar)),
        mk("energy_a", "kWh", 100.0, None),
        mk("energy_b", "kWh", 40.0, None),
        mk("energy_produced_a", "kWh", 30.0, None),
        mk("energy_produced_b", "kWh", 200.0, None),
    ]
}

#[test]
fn lifetime_total_from_paired_meters() {
    let ui = build(&clamp(|_| None));
    let grid = ui.node("grid").unwrap();
    assert_eq!((grid.total_kwh, grid.has_meter), (Some(100.0), true));
    assert_eq!(ui.node("solar").unwrap().total_kwh, Some(200.0));
    let house = ui.node("house").unwrap();
    assert!(house.has_meter);
    assert_eq!(house.total_kwh, Some(100.0 - 30.0 + 200.0 - 40.0));
    assert_eq!(house.total_status, NodeStatus::Live);
}

#[test]
fn derived_house_total_is_clamped_at_zero() {
    let ui = build(&clamp(|k| (k == "energy_produced_a").then_some(900.0)));
    assert_eq!(ui.node("house").unwrap().total_kwh, Some(0.0));
}

#[test]
fn a_house_without_an_export_counter_has_no_total() {
    let all: Vec<EnergyEntity> = clamp(|_| None)
        .into_iter()
        .filter(|x| !x.id.contains("produced_a"))
        .collect();
    assert!(!build(&all).node("house").unwrap().has_meter);
}

#[test]
fn a_derived_house_follows_its_inputs_status() {
    let mut all = clamp(|_| None);
    all.iter_mut()
        .find(|x| x.id.contains("energy_produced_a"))
        .unwrap()
        .available = false;
    let house = build(&all).node("house").cloned().unwrap();
    assert_eq!(house.total_kwh, None);
    assert_eq!(house.total_status, NodeStatus::Unavailable);
}

#[test]
fn a_meter_in_wh_is_converted_and_an_unavailable_one_gives_no_total() {
    let dev = Some("d".to_string());
    let heater = EnergyEntity {
        device_id: dev.clone(),
        ..consumer("heater", "Heater", 100.0)
    };
    let meter = |unit: &str, v: f64, available: bool| EnergyEntity {
        device_id: dev.clone(),
        unit: Some(unit.into()),
        value: Some(v),
        available,
        ..e("m")
    };
    let ui = build(&[heater.clone(), meter("Wh", 2500.0, true)]);
    assert_eq!(ui.node("consumer:heater").unwrap().total_kwh, Some(2.5));
    let ui = build(&[heater.clone(), meter("kWh", 1.0, false)]);
    let n = ui.node("consumer:heater").unwrap();
    assert!(n.has_meter);
    assert_eq!(n.total_kwh, None);
    assert_eq!(n.total_status, NodeStatus::Unavailable);
    let mut unknown = meter("kWh", 1.0, true);
    unknown.value = None;
    let ui = build(&[heater, unknown]);
    assert_eq!(ui.node("consumer:heater").unwrap().total_kwh, None);
}

#[test]
fn grid_totals_ignore_the_export_counter() {
    let mk = |id: &str, role: Option<EnergyRole>, unit: &str, v: f64, dev: &str| EnergyEntity {
        device_id: Some(dev.into()),
        unit: Some(unit.into()),
        value: Some(v),
        role,
        ..e(id)
    };
    let all = vec![
        mk("grid", Some(EnergyRole::Grid), "W", 100.0, "g"),
        mk("export", Some(EnergyRole::GridExport), "W", 50.0, "x"),
        mk("grid_kwh", None, "kWh", 10.0, "g"),
        mk("export_kwh", None, "kWh", 99.0, "x"),
    ];
    assert_eq!(build(&all).node("grid").unwrap().total_kwh, Some(10.0));
    let export_only = vec![all[1].clone(), all[3].clone()];
    assert!(!build(&export_only).node("grid").unwrap().has_meter);
}

#[test]
fn solar_sums_its_inverters_meters() {
    let mk = |id: &str, role: Option<EnergyRole>, unit: &str, v: f64, dev: &str| EnergyEntity {
        device_id: Some(dev.into()),
        unit: Some(unit.into()),
        value: Some(v),
        role,
        ..e(id)
    };
    let all = vec![
        mk("s1", Some(EnergyRole::Solar), "W", 1.0, "a"),
        mk("s2", Some(EnergyRole::Solar), "W", 1.0, "b"),
        mk("k1", None, "kWh", 10.0, "a"),
        mk("k2", None, "kWh", 5.0, "b"),
    ];
    assert_eq!(build(&all).node("solar").unwrap().total_kwh, Some(15.0));
}

#[test]
fn a_derived_house_has_no_meter_without_solar_or_grid_counters() {
    let mut all = house();
    all.push(EnergyEntity {
        device_id: Some("other".into()),
        unit: Some("kWh".into()),
        ..e("x")
    });
    assert!(!build(&all).node("house").unwrap().has_meter);
}
