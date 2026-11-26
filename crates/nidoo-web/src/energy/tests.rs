use super::*;
use nidoo_core::energy::EnergyFlow;

#[test]
fn group_entities_never_reach_the_energy_list() {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
    let view = |id: &str| EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
        protocol_kind: Kind::Sensor,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "x".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    };
    let map: BTreeMap<String, EntityView> = ["power", "group_all"]
        .into_iter()
        .map(|id| (id.to_owned(), view(id)))
        .collect();
    let ids: Vec<_> = energy_entities(&map).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["power"]);
}

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

#[test]
fn level_is_log_scaled() {
    assert_eq!(level(Some(10.0)), 0.0);
    assert_eq!(level(Some(10_000.0)), 1.0);
    assert_eq!(level(Some(1_000_000.0)), 1.0);
    assert_eq!(level(Some(5.0)), 0.0);
    assert_eq!(level(None), 0.0);
}

#[test]
fn texts() {
    assert_eq!(power_text(Lang::En, Some(3200.0)), "3.2\u{a0}kW");
    assert_eq!(power_text(Lang::En, Some(253.3)), "253\u{a0}W");
    assert_eq!(power_text(Lang::En, None), "—");
    assert_eq!(
        energy_text(Lang::En, Some(6584.04)),
        "6\u{2009}584\u{a0}kWh"
    );
    assert_eq!(energy_text(Lang::En, Some(32.7)), "32.7\u{a0}kWh");
    assert_eq!(energy_text(Lang::En, None), "—");
    assert_eq!(amount_text(Lang::En, 12.5, Some("L")), "12.5\u{a0}L");
    assert_eq!(meter_text(Lang::En, 601.359, Some("m³")), "601.359\u{a0}m³");
}

#[test]
fn aria_sentence_follows_the_app() {
    // The expectation is written from EnergyDescription.kt, not from the spec's rough
    // example: role nodes in diagram order (solar, grid, battery, house), then every
    // consumer by power, kilowatts only from 1000 W and watts below.
    let es = vec![
        ent("s", "S", EnergyRole::Solar, "W", 3200.0),
        ent("g", "G", EnergyRole::Grid, "W", -400.0),
        ent("a", "Water heater", EnergyRole::Consumer, "W", 1700.0),
        ent("b", "AC", EnergyRole::Consumer, "W", 600.0),
    ];
    let ui = EnergyFlow::build(&es, 0, None);
    assert_eq!(
        aria_sentence(Lang::En, &ui),
        "Solar 3.2\u{a0}kilowatts to the house. Exporting 400\u{a0}watts to the grid. \
             House uses 2.8\u{a0}kilowatts. Water heater 1.7\u{a0}kilowatts. AC 600\u{a0}watts."
    );
}

#[test]
fn labels_and_values() {
    let es = vec![
        ent("g", "G", EnergyRole::Grid, "W", -400.0),
        ent("w", "Water", EnergyRole::Water, "L", 5.0),
    ];
    let ui = EnergyFlow::build(&es, 0, None);
    let grid = ui.node("grid").unwrap();
    assert_eq!(grid_text(Lang::En, grid), "↗ 400\u{a0}W");
    assert_eq!(node_label(Lang::En, grid), "Grid");
    assert_eq!(
        node_value_text(Lang::En, ui.node("water").unwrap()),
        "5\u{a0}L"
    );
}

#[test]
fn circle_texts_name_the_direction() {
    let build = |es: Vec<EnergyEntity>| EnergyFlow::build(&es, 0, None);
    let ui = build(vec![ent("g", "G", EnergyRole::Grid, "W", 253.0)]);
    assert_eq!(
        circle_texts(Lang::En, ui.node("grid").unwrap()),
        ("253".to_string(), "W \u{b7} \u{2198} import".to_string())
    );
    let ui = build(vec![ent("g", "G", EnergyRole::Grid, "W", -1610.0)]);
    assert_eq!(
        circle_texts(Lang::Fr, ui.node("grid").unwrap()),
        ("1,6".to_string(), "kW \u{b7} \u{2197} export".to_string())
    );
    let ui = build(vec![ent("s", "S", EnergyRole::Solar, "W", 3460.0)]);
    assert_eq!(
        circle_texts(Lang::En, ui.node("solar").unwrap()),
        ("3.5".to_string(), "kW \u{b7} production".to_string())
    );
    let ui = build(vec![ent("b", "B", EnergyRole::Battery, "W", -500.0)]);
    assert_eq!(
        circle_texts(Lang::Fr, ui.node("battery").unwrap()),
        ("500".to_string(), "W \u{b7} \u{2197} charge".to_string())
    );
    let ui = build(vec![ent("b", "B", EnergyRole::Battery, "W", 500.0)]);
    assert_eq!(
        circle_texts(Lang::En, ui.node("battery").unwrap()).1,
        "W \u{b7} \u{2198} discharging"
    );
    let mut e = ent("g", "G", EnergyRole::Grid, "W", 1.0);
    e.available = false;
    let ui = build(vec![e]);
    assert_eq!(
        circle_texts(Lang::En, ui.node("grid").unwrap()),
        ("Unavailable".to_string(), String::new())
    );
}

#[test]
fn big_power_reads_in_kilowatts() {
    assert_eq!(
        power_parts(Lang::En, Some(12_345.0)),
        ("12.3".to_string(), "kW")
    );
    assert_eq!(
        power_parts(Lang::En, Some(3200.0)),
        ("3.2".to_string(), "kW")
    );
    assert_eq!(power_parts(Lang::En, Some(253.3)), ("253".to_string(), "W"));
    assert_eq!(power_parts(Lang::En, None), ("\u{2014}".to_string(), ""));
    assert_eq!(power_text(Lang::En, Some(999.0)), "999\u{a0}W");
    assert_eq!(power_text(Lang::En, Some(1000.0)), "1.0\u{a0}kW");
}

#[test]
fn texts_in_french() {
    assert_eq!(power_text(Lang::Fr, Some(3200.0)), "3,2\u{a0}kW");
    assert_eq!(power_text(Lang::Fr, Some(253.3)), "253\u{a0}W");
    assert_eq!(energy_text(Lang::Fr, Some(1250.0)), "1\u{202f}250\u{a0}kWh");
    assert_eq!(energy_text(Lang::Fr, Some(32.7)), "32,7\u{a0}kWh");
    assert_eq!(meter_text(Lang::Fr, 601.359, Some("m³")), "601,359\u{a0}m³");
    assert_eq!(amount_text(Lang::Fr, 12.5, Some("L")), "12,5\u{a0}L");
}

#[test]
fn french_aria_sentence_and_labels() {
    let es = vec![
        ent("s", "S", EnergyRole::Solar, "W", 3200.0),
        ent("g", "G", EnergyRole::Grid, "W", -400.0),
        ent("a", "AC", EnergyRole::Consumer, "W", 600.0),
    ];
    let ui = EnergyFlow::build(&es, 0, None);
    assert_eq!(
        aria_sentence(Lang::Fr, &ui),
        "Solaire 3,2\u{a0}kilowatts vers la maison. Exportation de 400\u{a0}watts vers le réseau. \
             La maison consomme 2,8\u{a0}kilowatts. AC 600\u{a0}watts."
    );
    assert_eq!(node_label(Lang::Fr, ui.node("grid").unwrap()), "Réseau");
    assert_eq!(node_label(Lang::Fr, ui.node("house").unwrap()), "Maison");
}

#[test]
fn solar_glyph_lights_with_units() {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
    let view = |unit: &str, v: f64| EntityView {
        id: "s".into(),
        device_id: None,
        name: "s".into(),
        device_name: None,
        label: None,
        kind: Kind::Sensor,
        component: "sensor".to_string(),
        protocol_kind: Kind::Sensor,
        device_class: None,
        unit: Some(unit.into()),
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "a".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride {
            energy_role: Some("solar".into()),
            ..Default::default()
        },
        state: EntityState {
            value: StateValue::Number(v),
            ..Default::default()
        },
    };
    assert!(any_solar_active([view("kW", 0.5)].iter()));
    assert!(!any_solar_active([view("W", 9.0)].iter()));
    assert!(!any_solar_active([view("Wh", 900.0)].iter()));
}

#[test]
fn next_fire_delay_pauses_when_hidden() {
    assert_eq!(next_fire_delay(false, 300_000), Some(300_000));
    assert_eq!(next_fire_delay(true, 300_000), None);
}

#[test]
fn should_fire_mount_fires_on_loaded_edge_only() {
    assert!(should_fire_mount(None, true));
    assert!(!should_fire_mount(None, false));
    assert!(!should_fire_mount(Some(true), true));
    assert!(should_fire_mount(Some(false), true));
}

#[test]
fn derived_house_today_sums_plus_minus_minus() {
    let m = HouseTodayParts {
        plus: vec![Some(10.0), Some(4.0)],
        minus: vec![Some(1.5)],
    };
    assert_eq!(derived_house_today(&m), Some(12.5));
}

#[test]
fn derived_house_today_is_none_when_any_part_is_missing() {
    let partial = HouseTodayParts {
        plus: vec![Some(10.0), None],
        minus: vec![Some(1.0)],
    };
    assert_eq!(derived_house_today(&partial), None);
    let partial = HouseTodayParts {
        plus: vec![Some(10.0)],
        minus: vec![None],
    };
    assert_eq!(derived_house_today(&partial), None);
}

#[test]
fn derived_house_today_handles_empty_parts() {
    assert_eq!(derived_house_today(&HouseTodayParts::default()), Some(0.0));
}
