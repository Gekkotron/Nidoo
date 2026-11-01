//! What the engine reads for one zone: devices, room reading, presence, holiday and outdoor
//! temperature, gathered from the registry, the retained documents and the runtime state.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use chrono_tz::Tz;
use nidoo_climate::decide::{Device, DeviceCaps, SafetyMemory, ZoneInputs};
use nidoo_climate::doc::{HouseSettings, Mode, Override, RoomEco, SafetyCause, ZoneSettings};
use nidoo_climate::feedback::{FeedbackState, UNKNOWN_AGE_S};
use nidoo_climate::presence::{PresenceState, Sensed, track};
use nidoo_climate::schedule::{Reading, ReadingSource, room_reading, season_mode};
use nidoo_climate::view::ThermometerReading;
use nidoo_core::model::{Availability, Entity, EntityCategory, EntityState, Kind, StateValue};

use super::hub::zone_devices;
use super::{ClimateRuntime, GROUP_PREFIX};
use crate::alarm::ALARM_ENTITY_ID;
use crate::hub::Hub;

/// Step assumed for a device that does not advertise one; the same as the decision tree's.
const DEFAULT_STEP: f64 = 0.5;
/// How long the history mean of the outdoor sensor is reused.
pub(crate) const OUTDOOR_CACHE_S: u64 = 600;

/// [`ZoneInputs`] with everything owned, so the registry lock is long gone when `decide` runs.
pub(crate) struct OwnedZoneInputs {
    pub zone: String,
    pub settings: ZoneSettings,
    pub house: HouseSettings,
    pub mode: Option<Mode>,
    pub reading: Reading,
    pub outdoor_mean: Option<f64>,
    pub home: bool,
    pub holiday: bool,
    pub overrides: Vec<Override>,
    /// The devices the engine may command: reachable, with a known state.
    pub devices: Vec<Device>,
    /// Every climate entity the zone lists, reachable or not.
    pub entity_ids: Vec<String>,
    pub local_now: NaiveDateTime,
    pub now: u64,
    pub safety_memory: Option<SafetyMemory>,
    /// The cause of the zone's previous safety decision, set by the caller.
    pub previous_cause: Option<SafetyCause>,
    /// The zone's own eco, if set.
    pub room_eco: Option<RoomEco>,
    /// The room thermometer's own reading, whatever `reading` uses.
    pub thermometer: Option<ReadingSource>,
    /// The thermometer correction's state from the previous tick.
    pub feedback: Option<FeedbackState>,
    /// The room presence state after this tick; `None` while presence is off.
    pub presence: Option<PresenceState>,
}

impl OwnedZoneInputs {
    pub(crate) fn borrowed(&self) -> ZoneInputs<'_> {
        ZoneInputs {
            zone: &self.zone,
            settings: &self.settings,
            house: &self.house,
            mode: self.mode,
            reading: self.reading,
            outdoor_mean: self.outdoor_mean,
            home: self.home,
            holiday: self.holiday,
            overrides: &self.overrides,
            devices: &self.devices,
            local_now: self.local_now,
            now: self.now,
            safety_memory: self.safety_memory.as_ref(),
            previous_cause: self.previous_cause,
            room_eco: self.room_eco.as_ref(),
            thermometer: self.thermometer,
            feedback: self.feedback.as_ref(),
            room_empty: self.presence.is_some_and(|p| p.is_empty()),
        }
    }
}

/// The wall-clock time of the house at `now`.
pub(crate) fn local_time(tz: Tz, now: u64) -> NaiveDateTime {
    i64::try_from(now)
        .ok()
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        .with_timezone(&tz)
        .naive_local()
}

/// The temperature step a device advertises, or the default.
pub(crate) fn step_from(advertised: Option<f64>) -> f64 {
    advertised.filter(|s| *s > 0.0).unwrap_or(DEFAULT_STEP)
}

/// The device's temperature step, or the default.
pub(crate) fn step_of(caps: &DeviceCaps) -> f64 {
    step_from(caps.temp_step)
}

/// The climate entities of `zone`: `settings.entities`, else the area's that no other zone
/// lists, never a group.
pub(crate) fn zone_entity_ids(hub: &Hub, zone: &str, settings: &ZoneSettings) -> Vec<String> {
    let claims = hub.climate_claims();
    match &settings.entities {
        Some(_) => {
            let listed = zone_devices(zone, settings, &[], &claims);
            let reg = hub.read();
            listed
                .into_iter()
                .filter(|id| {
                    !id.starts_with(GROUP_PREFIX)
                        && reg.entity(id).is_some_and(|e| e.kind == Kind::Climate)
                })
                .collect()
        }
        None => {
            let area = hub
                .climate_areas()
                .remove(zone)
                .map(|(_, devices)| devices)
                .unwrap_or_default();
            zone_devices(zone, settings, &area, &claims)
        }
    }
}

/// The numeric value of a sensor-like state: a number, a numeric text, or a climate's reading.
fn number_of(state: &EntityState) -> Option<f64> {
    if state.availability == Availability::Offline {
        return None;
    }
    match &state.value {
        StateValue::Number(n) => Some(*n),
        StateValue::Text(text) => text.trim().parse().ok(),
        StateValue::Climate(climate) => climate.current,
        _ => None,
    }
}

/// How old the last report of `id` is.
fn age_of(seen: &HashMap<String, u64>, id: &str, state: &EntityState, now: u64) -> u64 {
    let last = seen.get(id).copied().max(state.last_update);
    last.map_or(UNKNOWN_AGE_S, |at| now.saturating_sub(at))
}

/// Whether an entity says someone is home: `Some(false)` is a definite no, `None` is unknown
/// (no value yet, or unreachable).
fn home_of(state: &EntityState) -> Option<bool> {
    if state.availability == Availability::Offline {
        return None;
    }
    match &state.value {
        StateValue::Text(text) => match text.trim() {
            "" | "unknown" | "unavailable" => None,
            other => Some(other == "home"),
        },
        StateValue::OnOff(on) => Some(*on),
        StateValue::Alarm(state) => alarm_home(state),
        _ => None,
    }
}

/// What an alarm panel's state says about who is home: armed away (or vacation) is nobody,
/// disarmed or armed while inside is someone, and a passing state (arming, pending,
/// triggered, …) says nothing.
fn alarm_home(state: &str) -> Option<bool> {
    match state.trim() {
        "armed_away" | "armed_vacation" => Some(false),
        "disarmed" | "armed_home" | "armed_night" | "armed_custom_bypass" => Some(true),
        _ => None,
    }
}

/// Who is home right now, as the presence entities say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeNow {
    Home,
    /// Every presence entity with an answer says away: the house waits `away_grace` first.
    Away,
    /// An alarm panel is armed away: a definite answer, eco right away.
    AlarmAway,
}

/// Who is home right now. An alarm panel armed away (the hub's own, or one in the presence
/// list) is nobody home, whatever a tracker says. Otherwise some presence entity says home, or
/// none says anything definite (an unknown tracker never sends the house away); no presence
/// entity means home. An entity that says nothing now keeps its last definite answer, so a
/// triggered alarm or an entry delay does not bring the heating back. The hub's own alarm, when
/// not in the list, only ever says away: disarmed, it leaves the trackers in charge.
pub(crate) fn presence_home_now(hub: &Hub, house: &HouseSettings) -> HomeNow {
    let native_unlisted = !house.presence.iter().any(|id| id == ALARM_ENTITY_ID);
    let now: Vec<(String, Option<bool>, bool)> = {
        let reg = hub.read();
        house
            .presence
            .iter()
            .map(String::as_str)
            .chain(native_unlisted.then_some(ALARM_ENTITY_ID))
            .map(|id| {
                (
                    id.to_owned(),
                    reg.state(id).and_then(home_of),
                    reg.entity(id).is_some_and(|e| e.kind == Kind::AlarmPanel),
                )
            })
            .collect()
    };
    let mut rt = hub.climate_runtime();
    let mut alarm_away = false;
    let mut listed = Vec::new();
    for (id, answer, alarm) in now {
        if let Some(home) = answer {
            rt.presence_last.insert(id.clone(), home);
        }
        let Some(home) = answer.or_else(|| rt.presence_last.get(&id).copied()) else {
            continue;
        };
        alarm_away |= alarm && !home;
        if !(native_unlisted && id == ALARM_ENTITY_ID) {
            listed.push(home);
        }
    }
    if alarm_away {
        HomeNow::AlarmAway
    } else if listed.is_empty() || listed.contains(&true) {
        HomeNow::Home
    } else {
        HomeNow::Away
    }
}

/// Whether a presence sensor detects someone: `None` when it is unreachable or has no on/off
/// value.
pub(crate) fn detecting(state: &EntityState) -> Option<bool> {
    if state.availability == Availability::Offline {
        return None;
    }
    match &state.value {
        StateValue::OnOff(on) => Some(*on),
        _ => None,
    }
}

/// Notes what a presence sensor reports at `now`: on, or turning off (it was on until now), is
/// a detection; an idle sensor reporting off again is not.
pub(crate) fn note_detection(rt: &mut ClimateRuntime, id: &str, on: Option<bool>, now: u64) {
    let Some(on) = on else {
        return;
    };
    let was_on = rt.sensor_on.insert(id.to_owned(), on).unwrap_or(false);
    if on || was_on {
        rt.detected_at.insert(id.to_owned(), now);
    }
}

/// The room presence of `zone` after this tick, from its sensors as they are now.
fn presence_now(hub: &Hub, zone: &str, settings: &ZoneSettings, now: u64) -> Option<PresenceState> {
    let p = &settings.presence;
    if !p.enabled {
        return None;
    }
    let readings: Vec<(&str, Option<bool>)> = {
        let reg = hub.read();
        p.sensors
            .iter()
            .map(|id| (id.as_str(), reg.state(id).and_then(detecting)))
            .collect()
    };
    let mut rt = hub.climate_runtime();
    let mut usable = false;
    let mut latest = None;
    for (id, on) in readings {
        note_detection(&mut rt, id, on, now);
        if on.is_some() {
            usable = true;
            latest = latest.max(rt.detected_at.get(id).copied());
        }
    }
    let sensed = if usable {
        Sensed::Seen(latest)
    } else {
        Sensed::NoSensor
    };
    Some(track(rt.presence.get(zone), p, sensed, now))
}

fn holiday_now(hub: &Hub, house: &HouseSettings) -> bool {
    house
        .holiday_entity
        .as_ref()
        .is_some_and(|id| hub.read().state(id).and_then(home_of).unwrap_or(false))
}

/// Whether `entity` is a thermometer of the room `zone`: a primary temperature sensor of its
/// area that is not the outdoor sensor and not part of one of the zone's climate devices (whose
/// own sensors read the air at the unit). A diagnostic temperature is a device's chip.
fn is_room_thermometer(
    entity: &Entity,
    zone: &str,
    climate_devices: &HashSet<&str>,
    outdoor: Option<&str>,
) -> bool {
    entity.kind == Kind::Sensor
        && entity.area == zone
        && entity.device_class.as_deref() == Some("temperature")
        && entity.category == EntityCategory::Primary
        && !entity.unique_id.starts_with(GROUP_PREFIX)
        && outdoor != Some(entity.unique_id.as_str())
        && entity
            .device_id
            .as_deref()
            .is_none_or(|d| !climate_devices.contains(d))
}

/// The sensor that reads the room of `zone`: `settings.sensor`, else a thermometer of its area
/// (the first by id when there are several), else none and the devices' own readings are used.
pub(crate) fn room_sensor(
    hub: &Hub,
    zone: &str,
    settings: &ZoneSettings,
    entity_ids: &[String],
    outdoor: Option<&str>,
) -> Option<String> {
    if settings.sensor.is_some() {
        return settings.sensor.clone();
    }
    let reg = hub.read();
    let climate_devices: HashSet<&str> = entity_ids
        .iter()
        .filter_map(|id| reg.entity(id)?.device_id.as_deref())
        .collect();
    reg.entities()
        .filter(|e| is_room_thermometer(e, zone, &climate_devices, outdoor))
        .map(|e| e.unique_id.clone())
        .min()
}

/// The room reading sources of a zone: its sensor, and its devices' own readings.
fn reading_sources(
    hub: &Hub,
    sensor: Option<&str>,
    entity_ids: &[String],
    seen: &HashMap<String, u64>,
    now: u64,
) -> (Option<ReadingSource>, Vec<ReadingSource>) {
    let reg = hub.read();
    let source = |id: &str| {
        let state = reg.state(id)?;
        Some(ReadingSource {
            value: number_of(state)?,
            age_s: age_of(seen, id, state, now),
        })
    };
    let sensor = sensor.and_then(source);
    let devices = entity_ids.iter().filter_map(|id| source(id)).collect();
    (sensor, devices)
}

/// The reading and its age for the read view, recomputed from what is seen now.
pub(crate) fn view_reading(
    hub: &Hub,
    sensor: Option<&str>,
    entity_ids: &[String],
    stale_after_s: u64,
    quiet_s: u64,
    now: u64,
    seen: &HashMap<String, u64>,
) -> (Option<f64>, Option<u64>) {
    let (sensor, devices) = reading_sources(hub, sensor, entity_ids, seen, now);
    match room_reading(sensor, &devices, stale_after_s, quiet_s) {
        Reading::Fresh { value, age_s, .. } => (Some(value), Some(age_s)),
        Reading::Stale { last } => {
            let youngest = sensor
                .iter()
                .chain(&devices)
                .map(|source| source.age_s)
                .min()
                .filter(|age| *age < UNKNOWN_AGE_S);
            (last, youngest)
        }
        Reading::Missing => (None, None),
    }
}

/// The room thermometer's own value and age for the read view, whatever its age.
pub(crate) fn view_thermometer(
    hub: &Hub,
    sensor: Option<&str>,
    now: u64,
    seen: &HashMap<String, u64>,
) -> Option<ThermometerReading> {
    let id = sensor?;
    let (reading, _) = reading_sources(hub, Some(id), &[], seen, now);
    reading
        .filter(|source| source.age_s < UNKNOWN_AGE_S)
        .map(|source| ThermometerReading {
            entity: id.to_owned(),
            value: source.value,
            age_s: source.age_s,
        })
}

/// The mean outdoor temperature: the history's, while its cache is fresh, else the mean of the
/// live readings seen since boot.
pub(crate) fn outdoor_mean(hub: &Hub, now: u64) -> Option<f64> {
    let rt = hub.climate_runtime();
    if let Some((fetched, Some(mean))) = rt.outdoor_cache
        && now.saturating_sub(fetched) < OUTDOOR_CACHE_S
    {
        return Some(mean);
    }
    let (sum, n) = rt
        .outdoor_live
        .iter()
        .fold((0.0, 0u32), |(sum, n), (_, v)| (sum + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

/// The current reading of the outdoor sensor, when fresh.
pub(crate) fn outdoor_reading(hub: &Hub, house: &HouseSettings, now: u64) -> Option<f64> {
    let id = house.outdoor_entity.as_deref()?;
    let seen = hub.climate_runtime().seen_at.clone();
    let reg = hub.read();
    let state = reg.state(id)?;
    let age = age_of(&seen, id, state, now);
    (age <= u64::from(house.stale_after_min) * 60)
        .then(|| number_of(state))
        .flatten()
}

/// Everything `decide` needs for `zone`, or `None` while the engine is off.
pub(crate) fn zone_inputs_owned(hub: &Hub, zone: &str, now: u64) -> Option<OwnedZoneInputs> {
    hub.climate.as_ref()?;
    let (house, settings, overrides, room_eco) = {
        let store = hub.climate_store();
        let docs = store.zones.get(zone);
        (
            store.house.clone(),
            docs.map(|d| d.settings.clone()).unwrap_or_default(),
            docs.map(|d| d.overrides.clone()).unwrap_or_default(),
            docs.and_then(|d| d.eco.clone()),
        )
    };
    let entity_ids = zone_entity_ids(hub, zone, &settings);
    let seen = hub.climate_runtime().seen_at.clone();
    let devices: Vec<Device> = {
        let reg = hub.read();
        entity_ids
            .iter()
            .filter_map(|id| {
                let entity = reg.entity(id)?;
                let state = reg.state(id)?;
                let StateValue::Climate(climate) = &state.value else {
                    return None;
                };
                (state.availability != Availability::Offline).then(|| Device {
                    id: id.clone(),
                    caps: DeviceCaps {
                        hvac_modes: entity.capabilities.hvac_modes.clone(),
                        fan_modes: entity.capabilities.fan_modes.clone(),
                        min_temp: entity.capabilities.min_temp,
                        max_temp: entity.capabilities.max_temp,
                        temp_step: entity.capabilities.temp_step,
                    },
                    target: climate.target,
                    mode: climate.mode.clone(),
                    fan_mode: climate.fan_mode.clone(),
                })
            })
            .collect()
    };
    let stale_after_s = u64::from(house.stale_after_min) * 60;
    let sensor = room_sensor(
        hub,
        zone,
        &settings,
        &entity_ids,
        house.outdoor_entity.as_deref(),
    );
    let (sensor, device_readings) =
        reading_sources(hub, sensor.as_deref(), &entity_ids, &seen, now);
    let quiet_s = u64::from(settings.feedback.quiet_limit_min) * 60;
    let reading = room_reading(sensor, &device_readings, stale_after_s, quiet_s);
    let thermometer = sensor;
    let outdoor_mean = outdoor_mean(hub, now);
    let home_now = presence_home_now(hub, &house);
    let holiday = holiday_now(hub, &house);
    let presence = presence_now(hub, zone, &settings, now);
    let (mode, home, safety_memory, feedback) = {
        let rt = hub.climate_runtime();
        let grace_s = u64::from(house.away_grace_min) * 60;
        let recently_home = rt
            .last_home_at
            .is_some_and(|at| now.saturating_sub(at) <= grace_s);
        (
            season_mode(&house, outdoor_mean, rt.previous_mode),
            match home_now {
                HomeNow::Home => true,
                HomeNow::Away => recently_home,
                HomeNow::AlarmAway => false,
            },
            rt.safety_memory.get(zone).cloned(),
            rt.feedback.get(zone).copied(),
        )
    };
    Some(OwnedZoneInputs {
        zone: zone.to_owned(),
        settings,
        house,
        mode,
        reading,
        outdoor_mean,
        home,
        holiday,
        overrides,
        devices,
        entity_ids,
        local_now: local_time(hub.zone(), now),
        now,
        safety_memory,
        previous_cause: None,
        room_eco,
        thermometer,
        feedback,
        presence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarm_states_say_who_is_home() {
        for away in ["armed_away", "armed_vacation"] {
            assert_eq!(alarm_home(away), Some(false), "{away}");
        }
        for home in [
            "disarmed",
            "armed_home",
            "armed_night",
            "armed_custom_bypass",
        ] {
            assert_eq!(alarm_home(home), Some(true), "{home}");
        }
        for unsure in ["arming", "disarming", "pending", "triggered", "unknown", ""] {
            assert_eq!(alarm_home(unsure), None, "{unsure}");
        }
    }
}
