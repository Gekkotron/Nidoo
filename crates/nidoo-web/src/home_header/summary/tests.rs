use super::*;
use nidoo_core::model::{Capabilities, ClimateState, EntityState, LightState};

fn ent(id: &str, kind: Kind) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: Default::default(),
        state: EntityState::default(),
    }
}

fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
    list.into_iter().map(|e| (e.id.clone(), e)).collect()
}

fn temp(id: &str, value: f64, area: &str) -> EntityView {
    let mut e = ent(id, Kind::Sensor);
    e.device_class = Some("temperature".into());
    e.area = area.into();
    e.state.value = StateValue::Number(value);
    e
}

fn settings_with(indoor: Option<&str>) -> HomeSettings {
    HomeSettings {
        indoor_entity: indoor.map(str::to_owned),
        ..Default::default()
    }
}

#[test]
fn indoor_prefers_setting() {
    let h = house(vec![
        temp("chosen", 21.0, "salon"),
        temp("a", 18.0, "salon"),
        temp("b", 20.0, "chambre"),
    ]);
    assert_eq!(
        indoor_temperature(&h, &settings_with(Some("chosen"))),
        Some(21.0)
    );
}

#[test]
fn indoor_falls_back_when_setting_offline() {
    let mut chosen = temp("chosen", 21.0, "salon");
    chosen.state.availability = Availability::Offline;
    let h = house(vec![
        chosen,
        temp("a", 18.0, "salon"),
        temp("b", 20.0, "chambre"),
    ]);
    let got = indoor_temperature(&h, &settings_with(Some("chosen")));
    assert_eq!(got, Some(19.0));
}

#[test]
fn indoor_mean_skips_unassigned_offline_and_weather() {
    let mut offline = temp("off", 30.0, "salon");
    offline.state.availability = Availability::Offline;
    let mut weather = temp("w", 9.0, "salon");
    weather.device_id = Some("nidoo_weather".into());
    let h = house(vec![
        temp("a", 18.0, "salon"),
        temp("b", 20.0, "chambre"),
        temp("u", 5.0, UNASSIGNED_AREA),
        offline,
        weather,
    ]);
    assert_eq!(indoor_temperature(&h, &settings_with(None)), Some(19.0));
}

#[test]
fn indoor_mean_skips_diagnostic_and_hidden() {
    let mut diagnostic = temp("diag", 40.0, "salon");
    diagnostic.category = EntityCategory::Diagnostic;
    let mut hidden = temp("hid", 30.0, "salon");
    hidden.ui.hidden = true;
    let h = house(vec![temp("a", 20.0, "salon"), diagnostic, hidden]);
    assert_eq!(indoor_temperature(&h, &settings_with(None)), Some(20.0));
}

fn rooms(values: &[f64]) -> BTreeMap<String, EntityView> {
    house(
        values
            .iter()
            .enumerate()
            .map(|(i, v)| temp(&format!("t{i}"), *v, &format!("room {i}")))
            .collect(),
    )
}

#[test]
fn indoor_median_of_one_to_four_rooms() {
    let s = settings_with(None);
    assert_eq!(indoor_temperature(&rooms(&[18.0]), &s), Some(18.0));
    assert_eq!(indoor_temperature(&rooms(&[18.0, 20.0]), &s), Some(19.0));
    assert_eq!(
        indoor_temperature(&rooms(&[22.0, 18.0, 19.0]), &s),
        Some(19.0)
    );
    assert_eq!(
        indoor_temperature(&rooms(&[18.0, 24.0, 19.0, 21.0]), &s),
        Some(20.0)
    );
}

#[test]
fn indoor_median_ignores_an_outdoor_outlier() {
    let got = indoor_temperature(
        &rooms(&[19.0, 18.5, 19.5, 20.0, 18.0, 57.0]),
        &settings_with(None),
    )
    .unwrap();
    assert!((got - 19.25).abs() < 1.0, "got {got}");
}

#[test]
fn indoor_room_with_three_sensors_counts_once() {
    let h = house(vec![
        temp("a1", 10.0, "room a"),
        temp("a2", 10.0, "room a"),
        temp("a3", 10.0, "room a"),
        temp("b", 20.0, "room b"),
    ]);
    assert_eq!(indoor_temperature(&h, &settings_with(None)), Some(15.0));
}

#[test]
fn indoor_ignores_fahrenheit_sensors() {
    let mut f = temp("f", 70.0, "room b");
    f.unit = Some("°F".into());
    let h = house(vec![temp("a", 20.0, "room a"), f]);
    assert_eq!(indoor_temperature(&h, &settings_with(None)), Some(20.0));
}

#[test]
fn indoor_none_without_sensors() {
    let h = house(vec![ent("x", Kind::Switch)]);
    assert_eq!(indoor_temperature(&h, &settings_with(None)), None);
}

fn light(id: &str, on: bool) -> EntityView {
    let mut e = ent(id, Kind::Light);
    e.state.value = StateValue::Light(LightState {
        on,
        ..Default::default()
    });
    e
}

#[test]
fn lights_count_skips_groups_hidden_and_off() {
    let mut hidden = light("hidden", true);
    hidden.ui.hidden = true;
    let mut shown_switch = ent("sw", Kind::Light);
    shown_switch.protocol_kind = Kind::Switch;
    shown_switch.state.value = StateValue::OnOff(true);
    let h = house(vec![
        light("on", true),
        light("off", false),
        light("group_x", true),
        hidden,
        shown_switch,
    ]);
    assert_eq!(lights_on(&h), 2);
}

#[test]
fn lights_count_skips_offline() {
    let mut gone = light("gone", true);
    gone.state.availability = Availability::Offline;
    let mut unknown = light("unknown", true);
    unknown.state.availability = Availability::Unknown;
    assert_eq!(lights_on(&house(vec![gone, unknown])), 1);
}

#[test]
fn openings_skip_offline() {
    let mut gone = opening("gone", "door", true, Kind::BinarySensor);
    gone.state.availability = Availability::Offline;
    let live = opening("live", "door", true, Kind::BinarySensor);
    assert_eq!(openings(Lang::En, &house(vec![gone, live]), &[]).len(), 1);
}

fn opening(id: &str, class: &str, open: bool, kind: Kind) -> EntityView {
    let mut e = ent(id, kind);
    e.device_class = Some(class.into());
    e.state.value = StateValue::OnOff(open);
    e
}

#[test]
fn openings_lists_open_doors_and_windows_with_room() {
    let mut window = opening("window", "window", true, Kind::BinarySensor);
    window.area = "cuisine".into();
    let areas = vec![AreaView {
        id: "cuisine".into(),
        name: "Cuisine".into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }];
    let h = house(vec![
        window.clone(),
        opening("door", "door", false, Kind::BinarySensor),
        opening("motion", "motion", true, Kind::BinarySensor),
    ]);
    assert_eq!(
        openings(Lang::En, &h, &areas),
        vec![Opening {
            title: entity_title(Lang::En, &window),
            room: Some("Cuisine".into())
        }]
    );
}

fn unit(id: &str, modes: &[&str], mode: &str, target: Option<f64>) -> EntityView {
    let mut e = ent(id, Kind::Climate);
    e.capabilities.hvac_modes = modes.iter().map(|m| (*m).to_owned()).collect();
    e.state.value = StateValue::Climate(ClimateState {
        mode: Some(mode.into()),
        target,
        ..Default::default()
    });
    e
}

const HEAT: &[&str] = &["off", "heat", "auto", "dry"];

fn heating(units: Vec<EntityView>, eco: Option<f64>) -> Option<HeatingChip> {
    heating_chip(&house(units), eco)
}

#[test]
fn heating_off_when_all_off() {
    let got = heating(
        vec![
            unit("a", HEAT, "off", None),
            unit("b", HEAT, "dry", Some(20.0)),
        ],
        None,
    );
    assert_eq!(got, Some(HeatingChip::Off));
}

#[test]
fn heating_eco_at_or_below_setpoint() {
    let got = heating(
        vec![
            unit("a", HEAT, "heat", Some(15.0)),
            unit("b", HEAT, "auto", Some(14.0)),
        ],
        Some(15.0),
    );
    assert_eq!(got, Some(HeatingChip::Eco));
}

#[test]
fn heating_hidden_above_setpoint() {
    let got = heating(vec![unit("a", HEAT, "heat", Some(20.0))], Some(15.0));
    assert_eq!(got, None);
}

#[test]
fn heating_hidden_without_target() {
    let got = heating(vec![unit("a", HEAT, "heat", None)], Some(15.0));
    assert_eq!(got, None);
}

#[test]
fn heating_eco_needs_setpoint() {
    let got = heating(vec![unit("a", HEAT, "heat", Some(14.0))], None);
    assert_eq!(got, None);
}

#[test]
fn cooling_wins() {
    let got = heating(
        vec![
            unit("a", &["off", "heat", "cool"], "cool", Some(24.0)),
            unit("b", HEAT, "heat", Some(20.0)),
        ],
        Some(15.0),
    );
    assert_eq!(got, Some(HeatingChip::Cooling));
}

#[test]
fn cool_only_unit_ignored() {
    let got = heating(vec![unit("a", &["off", "cool"], "off", None)], Some(15.0));
    assert_eq!(got, None);
}

#[test]
fn heating_ignores_offline_units() {
    let mut gone = unit("gone", HEAT, "cool", None);
    gone.state.availability = Availability::Offline;
    assert_eq!(heating(vec![gone.clone()], None), None);
    let off = unit("o", HEAT, "off", None);
    assert_eq!(heating(vec![gone, off], None), Some(HeatingChip::Off));
}

#[test]
fn heating_ignores_unknown_state() {
    let mut unknown = unit("u", HEAT, "off", None);
    unknown.state.value = StateValue::Unknown;
    let off = unit("o", HEAT, "off", None);
    assert_eq!(
        heating(vec![unknown.clone(), off], None),
        Some(HeatingChip::Off)
    );
    assert_eq!(heating(vec![unknown], None), None);
}

fn mode_select() -> EntityView {
    let mut e = ent("virtual_mode", Kind::Select);
    e.capabilities.options = vec!["Home".into(), "Away".into(), "Night".into()];
    e.state.value = StateValue::Text("Away".into());
    e
}

#[test]
fn mode_chip_reads_select() {
    let h = house(vec![mode_select()]);
    assert_eq!(
        mode_chip(&h, Some("virtual_mode")),
        Some(ModeChip {
            entity_id: "virtual_mode".into(),
            current: "Away".into(),
            options: vec!["Home".into(), "Away".into(), "Night".into()],
        })
    );
}

#[test]
fn mode_chip_missing_entity() {
    let h = house(vec![mode_select()]);
    assert_eq!(mode_chip(&h, Some("virtual_gone")), None);
}

#[test]
fn mode_chip_none_when_unset() {
    let h = house(vec![mode_select()]);
    assert_eq!(mode_chip(&h, None), None);
}
