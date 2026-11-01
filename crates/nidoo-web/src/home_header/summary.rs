//! The pure facts the home header shows: indoor temperature, lights, openings and the chips.

use std::collections::BTreeMap;

use nidoo_api::{AreaView, EntityView, HomeSettings};
use nidoo_core::model::{Availability, EntityCategory, Kind, StateValue, UNASSIGNED_AREA};

use crate::format::{entity_title, is_on, room_name};
use crate::i18n::Lang;

/// The device the hub publishes its weather entities under.
const WEATHER_DEVICE: &str = "oikos_weather";

/// An open door or window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    pub title: String,
    pub room: Option<String>,
}

/// What the heating chip says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeatingChip {
    Off,
    Eco,
    Cooling,
}

/// The house mode select: its id, current option and the options to pick from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeChip {
    pub entity_id: String,
    pub current: String,
    pub options: Vec<String>,
}

/// Everything the header reads from the house.
#[derive(Clone, Debug, PartialEq)]
pub struct HouseSummary {
    pub indoor: Option<f64>,
    pub lights_on: usize,
    pub openings: Vec<Opening>,
    pub unseen_alerts: usize,
    pub heating: Option<HeatingChip>,
    pub mode: Option<ModeChip>,
}

/// The numeric value of an entity that is not offline.
fn live_number(entity: &EntityView) -> Option<f64> {
    match entity.state.value {
        StateValue::Number(n) if entity.state.availability != Availability::Offline => Some(n),
        _ => None,
    }
}

/// The indoor temperature: the chosen sensor when it reads a number, else the median of the room
/// values, each room being the mean of its primary temperature sensors in °C (weather and
/// unassigned ones left out).
pub fn indoor_temperature(
    entities: &BTreeMap<String, EntityView>,
    settings: &HomeSettings,
) -> Option<f64> {
    if let Some(n) = settings
        .indoor_entity
        .as_deref()
        .and_then(|id| entities.get(id))
        .and_then(live_number)
    {
        return Some(n);
    }
    let mut by_room: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for e in entities.values().filter(|e| {
        e.kind == Kind::Sensor
            && e.is_primary()
            && e.device_class.as_deref() == Some("temperature")
            && e.unit.as_deref().is_none_or(|u| u == "°C")
            && e.area != UNASSIGNED_AREA
            && e.device_id.as_deref() != Some(WEATHER_DEVICE)
    }) {
        if let Some(n) = live_number(e) {
            by_room.entry(e.area.as_str()).or_default().push(n);
        }
    }
    let mut rooms: Vec<f64> = by_room
        .values()
        .map(|v| v.iter().sum::<f64>() / v.len() as f64)
        .collect();
    rooms.sort_by(f64::total_cmp);
    let mid = rooms.len() / 2;
    match rooms.len() {
        0 => None,
        n if n % 2 == 1 => Some(rooms[mid]),
        _ => Some((rooms[mid - 1] + rooms[mid]) / 2.0),
    }
}

/// How many lights are on: groups, hidden, diagnostic and config entities are not counted.
pub fn lights_on(entities: &BTreeMap<String, EntityView>) -> usize {
    entities
        .values()
        .filter(|e| {
            e.kind == Kind::Light
                && !e.id.starts_with("group_")
                && !e.ui.hidden
                && e.category == EntityCategory::Primary
                && e.state.availability != Availability::Offline
                && is_on(&e.state)
        })
        .count()
}

/// The open doors and windows, sorted by title.
pub fn openings(
    lang: Lang,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
) -> Vec<Opening> {
    let mut list: Vec<Opening> = entities
        .values()
        .filter(|e| {
            e.kind == Kind::BinarySensor
                && matches!(
                    e.device_class.as_deref(),
                    Some("door" | "window" | "garage_door" | "opening")
                )
                && e.is_primary()
                && e.state.availability != Availability::Offline
                && matches!(e.state.value, StateValue::OnOff(true))
        })
        .map(|e| Opening {
            title: entity_title(lang, e),
            room: room_name(areas, &e.area),
        })
        .collect();
    list.sort_by(|a, b| a.title.cmp(&b.title));
    list
}

/// The heating chip, from the primary climate units that can heat (see the spec, section 5).
pub fn heating_chip(
    entities: &BTreeMap<String, EntityView>,
    eco_setpoint: Option<f64>,
) -> Option<HeatingChip> {
    let mut any = false;
    let mut cooling = false;
    let mut heating_targets: Vec<Option<f64>> = Vec::new();
    for e in entities.values() {
        if e.kind != Kind::Climate
            || !e.is_primary()
            || e.state.availability == Availability::Offline
            || !e.capabilities.hvac_modes.iter().any(|m| m == "heat")
        {
            continue;
        }
        let StateValue::Climate(c) = &e.state.value else {
            continue;
        };
        let Some(mode) = c.mode.as_deref() else {
            continue;
        };
        any = true;
        match mode {
            "cool" => cooling = true,
            "heat" | "auto" | "heat_cool" => heating_targets.push(c.target),
            _ => {}
        }
    }
    if !any {
        None
    } else if cooling {
        Some(HeatingChip::Cooling)
    } else if heating_targets.is_empty() {
        Some(HeatingChip::Off)
    } else {
        let eco = eco_setpoint?;
        heating_targets
            .iter()
            .all(|t| t.is_some_and(|t| t <= eco))
            .then_some(HeatingChip::Eco)
    }
}

/// The house mode select, when one is chosen and present.
pub fn mode_chip(
    entities: &BTreeMap<String, EntityView>,
    mode_entity: Option<&str>,
) -> Option<ModeChip> {
    let e = entities.get(mode_entity?)?;
    let current = match &e.state.value {
        StateValue::Text(t) => t.clone(),
        _ => String::new(),
    };
    Some(ModeChip {
        entity_id: e.id.clone(),
        current,
        options: e.capabilities.options.clone(),
    })
}

/// The whole summary in one pass.
pub fn summarize(
    lang: Lang,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
    settings: &HomeSettings,
    unseen_alerts: usize,
) -> HouseSummary {
    HouseSummary {
        indoor: indoor_temperature(entities, settings),
        lights_on: lights_on(entities),
        openings: openings(lang, entities, areas),
        unseen_alerts,
        heating: heating_chip(entities, settings.eco_setpoint),
        mode: mode_chip(entities, settings.mode_entity.as_deref()),
    }
}

#[cfg(test)]
mod tests {
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
        weather.device_id = Some("oikos_weather".into());
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
}
