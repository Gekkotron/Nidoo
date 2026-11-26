use super::super::testkit::e;
use super::*;

const DEV_G: &str = "zigbee2mqtt_0xa4c13898428fae3a";
const DEV_C: &str = "zigbee2mqtt_0xa4c13870d8ba6a60";

/// One PJ-1203A clamp: power_a/power_b plus the four counters, ids as Z2M publishes them.
fn clamp(hex: &str, device: &str) -> Vec<EnergyEntity> {
    let id = |kind: &str| format!("{hex}_{kind}_zigbee2mqtt");
    let mk = |kind: &str, unit: &str| EnergyEntity {
        device_id: Some(device.to_string()),
        label: Some(kind.to_string()),
        unit: Some(unit.to_string()),
        ..e(&id(kind))
    };
    vec![
        mk("power_a", "W"),
        mk("power_b", "W"),
        mk("energy_a", "kWh"),
        mk("energy_b", "kWh"),
        mk("energy_produced_a", "kWh"),
        mk("energy_produced_b", "kWh"),
    ]
}

fn house() -> Vec<EnergyEntity> {
    let mut all = clamp("0xa4c13898428fae3a", DEV_G);
    all[0].role = Some(EnergyRole::Grid);
    all[1].role = Some(EnergyRole::Solar);
    all.extend(clamp("0xa4c13870d8ba6a60", DEV_C));
    all[6].role = Some(EnergyRole::Consumer);
    all[7].role = Some(EnergyRole::Consumer);
    all
}

fn id(x: Option<&EnergyEntity>) -> Option<&str> {
    x.map(|e| e.id.as_str())
}

fn by_id<'a>(all: &'a [EnergyEntity], id: &str) -> &'a EnergyEntity {
    all.iter().find(|e| e.id == id).unwrap()
}

#[test]
fn grid_counters_sum_phases_once() {
    let mut all = clamp("0x1111", "dev_one");
    all[0].role = Some(EnergyRole::Grid);
    let mut two = clamp("0x2222", "dev_two");
    two[0].role = Some(EnergyRole::Grid);
    // An export sensor pointed at a counter the grid already counts as sold.
    two[1].role = Some(EnergyRole::GridExport);
    two[1].meter = Some("0x2222_energy_produced_a_zigbee2mqtt".to_string());
    all.extend(two);
    let c = EnergyMeters::grid_counters(&all);
    assert_eq!(
        c.bought,
        vec!["0x1111_energy_a_zigbee2mqtt", "0x2222_energy_a_zigbee2mqtt"]
    );
    assert_eq!(
        c.sold,
        vec![
            "0x1111_energy_produced_a_zigbee2mqtt",
            "0x2222_energy_produced_a_zigbee2mqtt"
        ]
    );
}

#[test]
fn grid_counters_carry_unit_factor() {
    let mut all = clamp("0x1111", "dev_one");
    all[0].role = Some(EnergyRole::Grid);
    all[2].unit = Some("Wh".to_string()); // energy_a
    let c = EnergyMeters::grid_counters(&all);
    assert_eq!(c.factor("0x1111_energy_a_zigbee2mqtt"), 0.001);
    assert_eq!(c.factor("0x1111_energy_produced_a_zigbee2mqtt"), 1.0);
}

#[test]
fn grid_counters_without_export() {
    let mut all: Vec<EnergyEntity> = clamp("0x1111", "dev_one")
        .into_iter()
        .filter(|e| !e.id.contains("produced"))
        .collect();
    all[0].role = Some(EnergyRole::Grid);
    let c = EnergyMeters::grid_counters(&all);
    assert_eq!(c.bought, vec!["0x1111_energy_a_zigbee2mqtt"]);
    assert!(c.sold.is_empty());
    assert_eq!(EnergyMeters::grid_counters(&[]), GridCounters::default());
}

#[test]
fn grid_pairs_with_energy_a() {
    let all = house();
    let grid = &all[0];
    assert_eq!(
        id(EnergyMeters::meter_for(grid, &all)),
        Some("0xa4c13898428fae3a_energy_a_zigbee2mqtt")
    );
}

#[test]
fn consumers_on_the_other_device_pair_by_label() {
    let all = house();
    assert_eq!(
        id(EnergyMeters::meter_for(&all[6], &all)),
        Some("0xa4c13870d8ba6a60_energy_a_zigbee2mqtt")
    );
    assert_eq!(
        id(EnergyMeters::meter_for(&all[7], &all)),
        Some("0xa4c13870d8ba6a60_energy_b_zigbee2mqtt")
    );
}

#[test]
fn solar_pairs_with_its_production_twin_and_draws_from_energy_b() {
    let all = house();
    let solar = &all[1];
    assert_eq!(
        id(EnergyMeters::meter_for(solar, &all)),
        Some("0xa4c13898428fae3a_energy_produced_b_zigbee2mqtt")
    );
    assert_eq!(
        id(EnergyMeters::reverse_for(solar, &all)),
        Some("0xa4c13898428fae3a_energy_b_zigbee2mqtt")
    );
}

#[test]
fn grid_reverse_is_its_produced_twin() {
    let all = house();
    assert_eq!(
        id(EnergyMeters::reverse_for(&all[0], &all)),
        Some("0xa4c13898428fae3a_energy_produced_a_zigbee2mqtt")
    );
    assert_eq!(EnergyMeters::reverse_for(&all[6], &all), None);
}

#[test]
fn solar_reverse_needs_the_production_pairing() {
    let mut all = house();
    all[1].meter = Some("0xa4c13898428fae3a_energy_a_zigbee2mqtt".into());
    let solar = all[1].clone();
    assert_eq!(EnergyMeters::reverse_for(&solar, &all), None);
}

#[test]
fn explicit_meter_wins() {
    let mut all = house();
    all[0].meter = Some("0xa4c13870d8ba6a60_energy_b_zigbee2mqtt".into());
    let grid = all[0].clone();
    assert_eq!(
        id(EnergyMeters::meter_for(&grid, &all)),
        Some("0xa4c13870d8ba6a60_energy_b_zigbee2mqtt")
    );
    assert_eq!(
        id(EnergyMeters::auto(&grid, &all)),
        Some("0xa4c13898428fae3a_energy_a_zigbee2mqtt")
    );
}

#[test]
fn dangling_or_non_counter_explicit_meter_pairs_nothing() {
    let mut all = house();
    all[0].meter = Some("gone".into());
    let grid = all[0].clone();
    assert_eq!(EnergyMeters::meter_for(&grid, &all), None);
    let mut power_pointing_at_power = grid.clone();
    power_pointing_at_power.meter = Some(all[1].id.clone());
    assert_eq!(
        EnergyMeters::meter_for(&power_pointing_at_power, &all),
        None
    );
}

#[test]
fn lone_power_and_lone_counter_pair_whatever_their_labels() {
    let all = vec![
        EnergyEntity {
            device_id: Some("d".into()),
            ..e("p")
        },
        EnergyEntity {
            device_id: Some("d".into()),
            unit: Some("kWh".into()),
            ..e("c")
        },
    ];
    assert_eq!(id(EnergyMeters::meter_for(&all[0], &all)), Some("c"));
}

#[test]
fn two_unrelated_powers_pair_nothing_and_other_devices_are_ignored() {
    let mk = |i: &str, unit: &str, dev: &str| EnergyEntity {
        device_id: Some(dev.into()),
        unit: Some(unit.into()),
        ..e(i)
    };
    let all = vec![
        mk("a", "W", "d"),
        mk("b", "W", "d"),
        mk("c1", "kWh", "d"),
        mk("c2", "kWh", "d"),
        mk("far", "kWh", "x"),
    ];
    assert_eq!(EnergyMeters::meter_for(&all[0], &all), None);
    let no_device = EnergyEntity {
        device_id: None,
        ..e("n")
    };
    assert_eq!(EnergyMeters::meter_for(&no_device, &all), None);
}

#[test]
fn label_beats_name() {
    let mk = |i: &str, name: &str, label: Option<&str>, unit: &str| EnergyEntity {
        device_id: Some("d".into()),
        name: name.into(),
        label: label.map(String::from),
        unit: Some(unit.into()),
        ..e(i)
    };
    let all = vec![
        mk("p", "Plug Power", Some("Power"), "W"),
        mk("e", "Plug Energy", Some("Energy"), "kWh"),
        mk("x", "Other", None, "kWh"),
    ];
    assert_eq!(id(EnergyMeters::meter_for(&all[0], &all)), Some("e"));
}

#[test]
fn twin_token_is_a_word_not_a_substring() {
    assert_eq!(replace_power_token("a_power_b", "energy"), "a_energy_b");
    assert_eq!(replace_power_token("Power_a", "x"), "x_a");
    assert_eq!(replace_power_token("powers_a", "x"), "powers_a");
    assert_eq!(replace_power_token("mypower_a", "x"), "mypower_a");
    assert_eq!(replace_power_token("p_power", "x"), "p_x");
    assert_eq!(replace_power_token("power2_power", "x"), "x2_x");
}

#[test]
fn counter_power_pairs_back() {
    let all = house();
    let counter = by_id(&all, "0xa4c13898428fae3a_energy_a_zigbee2mqtt");
    assert_eq!(
        id(EnergyMeters::counter_power(counter, &all)),
        Some("0xa4c13898428fae3a_power_a_zigbee2mqtt")
    );
}

#[test]
fn counter_power_is_none_when_no_power_pairs() {
    let all = vec![EnergyEntity {
        unit: Some("kWh".into()),
        ..e("c")
    }];
    assert_eq!(EnergyMeters::counter_power(&all[0], &all), None);
}

#[test]
fn counter_power_first_registered_wins_on_ties() {
    let mk = |i: &str, unit: &str, dev: &str| EnergyEntity {
        device_id: Some(dev.into()),
        unit: Some(unit.into()),
        ..e(i)
    };
    let all = vec![mk("p1", "W", "d"), mk("p2", "W", "d"), mk("c", "kWh", "d")];
    // Two powers + one counter: auto-pairing refuses the ambiguity, so neither power maps to c
    // and counter_power(c) is None.
    assert_eq!(EnergyMeters::counter_power(&all[2], &all), None);

    // Labelled match for p1 only; counter_power must then find p1 (first-registered wins).
    let mut all = all;
    all[0].label = Some("Power".into());
    all[2].label = Some("Energy".into());
    assert_eq!(id(EnergyMeters::counter_power(&all[2], &all)), Some("p1"));
}

fn water(id: &str, unit: &str, device: Option<&str>) -> EnergyEntity {
    EnergyEntity {
        device_id: device.map(String::from),
        unit: Some(unit.into()),
        role: Some(EnergyRole::Water),
        ..e(id)
    }
}

#[test]
fn water_auto_pairs_a_meter_scale_sensor_on_its_device() {
    for unit in ["m³", "m3", "ft³", "CCF"] {
        let all = vec![
            water("daily", "L", Some("d")),
            water("index", unit, Some("d")),
            water("far", unit, Some("x")),
        ];
        assert_eq!(
            id(EnergyMeters::water_meter_for(&all[0], &all)),
            Some("index")
        );
    }
}

#[test]
fn water_meter_pairs_nothing_for_meters_same_units_or_no_device() {
    let index = water("index", "m³", Some("d"));
    let all = vec![index.clone(), water("other", "m³", Some("d"))];
    assert_eq!(EnergyMeters::water_meter_for(&index, &all), None);
    let gal = water("daily", "gal", Some("d"));
    let all = vec![gal.clone(), water("total", "gal", Some("d"))];
    assert_eq!(EnergyMeters::water_meter_for(&gal, &all), None);
    let loose = water("daily", "L", None);
    let all = vec![loose.clone(), water("index", "m³", None)];
    assert_eq!(EnergyMeters::water_meter_for(&loose, &all), None);
}

#[test]
fn water_auto_takes_the_smallest_id_among_meters() {
    let all = vec![
        water("daily", "L", Some("d")),
        water("b_index", "m³", Some("d")),
        water("a_index", "m³", Some("d")),
    ];
    assert_eq!(id(EnergyMeters::water_auto(&all[0], &all)), Some("a_index"));
}

#[test]
fn water_explicit_choice_takes_any_volume_but_not_itself_or_a_non_volume() {
    let mut w = water("daily", "gal", None);
    w.meter = Some("utility".into());
    let all = vec![w.clone(), water("utility", "gal", None)];
    assert_eq!(id(EnergyMeters::water_meter_for(&w, &all)), Some("utility"));
    let mut own = water("daily", "L", Some("d"));
    own.meter = Some("daily".into());
    assert_eq!(
        EnergyMeters::water_meter_for(&own, std::slice::from_ref(&own)),
        None
    );
    let mut kwh = water("daily", "L", Some("d"));
    kwh.meter = Some("e".into());
    let all = vec![
        kwh.clone(),
        EnergyEntity {
            unit: Some("kWh".into()),
            ..e("e")
        },
    ];
    assert_eq!(EnergyMeters::water_meter_for(&kwh, &all), None);
}

#[test]
fn counters_and_volumes_filter_by_unit() {
    let all = house();
    assert_eq!(EnergyMeters::counters(&all).len(), 8);
    assert!(EnergyMeters::volumes(&all).is_empty());
    let w = vec![water("v", "L", None), water("m", "m³", None), e("p")];
    assert_eq!(EnergyMeters::volumes(&w).len(), 2);
}

#[test]
fn node_meters_count_each_counter_once_and_skip_water() {
    let all = house();
    let sensors = vec![&all[6], &all[6], &all[7]];
    let ids: Vec<&str> = EnergyMeters::node_meters(&sensors, &all)
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "0xa4c13870d8ba6a60_energy_a_zigbee2mqtt",
            "0xa4c13870d8ba6a60_energy_b_zigbee2mqtt"
        ]
    );
    let w = vec![water("v", "L", Some("d")), water("m", "m³", Some("d"))];
    assert!(EnergyMeters::node_meters(&[&w[0]], &w).is_empty());
}

fn roles(all: &[EnergyEntity]) -> BTreeMap<EnergyRole, Vec<&EnergyEntity>> {
    let mut m: BTreeMap<EnergyRole, Vec<&EnergyEntity>> = BTreeMap::new();
    for x in all {
        if let Some(r) = x.role {
            m.entry(r).or_default().push(x);
        }
    }
    m
}

fn solar_grid() -> Vec<EnergyEntity> {
    let mut all = clamp("0xa4c13898428fae3a", DEV_G);
    all[0].role = Some(EnergyRole::Grid);
    all[1].role = Some(EnergyRole::Solar);
    all
}

#[test]
fn house_meters_without_a_house() {
    let all = solar_grid();
    let m = EnergyMeters::house_meters(&roles(&all), &all).unwrap();
    let ids = |v: &[&EnergyEntity]| v.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
    // solar first, then grid, as the app lists `solar + grid`.
    assert_eq!(
        ids(&m.plus),
        [
            "0xa4c13898428fae3a_energy_produced_b_zigbee2mqtt",
            "0xa4c13898428fae3a_energy_a_zigbee2mqtt"
        ]
    );
    assert_eq!(
        ids(&m.minus),
        [
            "0xa4c13898428fae3a_energy_produced_a_zigbee2mqtt",
            "0xa4c13898428fae3a_energy_b_zigbee2mqtt"
        ]
    );
}

#[test]
fn house_meters_give_no_total_with_a_battery_no_export_or_nothing() {
    let all = solar_grid();
    let mut with_battery = roles(&all);
    let bat = EnergyEntity {
        role: Some(EnergyRole::Battery),
        ..e("bat")
    };
    with_battery.insert(EnergyRole::Battery, vec![&bat]);
    assert_eq!(EnergyMeters::house_meters(&with_battery, &all), None);

    let no_export: Vec<EnergyEntity> = all
        .iter()
        .filter(|x| !x.id.contains("produced_a"))
        .cloned()
        .collect();
    assert_eq!(
        EnergyMeters::house_meters(&roles(&no_export), &no_export),
        None
    );

    let only_consumers = house();
    let mut m = BTreeMap::new();
    m.insert(
        EnergyRole::Consumer,
        vec![by_id(&only_consumers, &only_consumers[6].id)],
    );
    assert_eq!(EnergyMeters::house_meters(&m, &only_consumers), None);
}

#[test]
fn house_meters_need_a_counter_for_every_input() {
    let mut all = solar_grid();
    all.retain(|x| x.id != "0xa4c13898428fae3a_energy_a_zigbee2mqtt");
    assert_eq!(EnergyMeters::house_meters(&roles(&all), &all), None);
}
