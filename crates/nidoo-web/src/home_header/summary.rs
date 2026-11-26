//! The pure facts the home header shows: indoor temperature, lights, openings and the chips.

use std::collections::BTreeMap;

use nidoo_api::{AreaView, EntityView, HomeSettings};
use nidoo_core::model::{Availability, EntityCategory, Kind, StateValue, UNASSIGNED_AREA};

use crate::format::{entity_title, is_on, room_name};
use crate::i18n::Lang;

/// The device the hub publishes its weather entities under.
const WEATHER_DEVICE: &str = "nidoo_weather";

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
mod tests;
