//! The weather side of the hub: the stored view, its retained entities, and the clearing of a
//! device that was switched off.

use std::collections::BTreeMap;
use std::sync::MutexGuard;

use nidoo_api::{WeatherView, WsEvent};
use nidoo_core::command::Publish;

use super::{entities, is_stale};
use crate::clock;
use crate::hub::{Hub, HubError};

/// The payload that makes a sensor's state unknown (the registry reads `None` as unknown; an
/// empty payload would leave the old value in place).
const UNKNOWN: &str = "None";

/// What the hub knows of the weather. `published` only records payloads the broker link took,
/// so a failed publish is retried by the next [`Hub::apply_weather`].
#[derive(Default)]
pub(crate) struct WeatherSlot {
    view: Option<WeatherView>,
    published: BTreeMap<&'static str, String>,
    /// The availability last published (`publish_weather_configs` sets `Some(false)`); `None`
    /// until the first success.
    online: Option<bool>,
    /// Whether the last `publish_weather_configs` got every config and the availability out.
    configs_ok: bool,
}

impl Hub {
    fn weather_slot(&self) -> MutexGuard<'_, WeatherSlot> {
        self.weather
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether `[weather] enabled` is on.
    pub fn weather_enabled(&self) -> bool {
        self.weather_enabled
    }

    /// The configured city, trimmed.
    pub fn weather_city(&self) -> String {
        self.weather_city.clone()
    }

    /// The configured coordinates, when both latitude and longitude are set.
    pub fn weather_coordinates(&self) -> Option<(f64, f64)> {
        self.weather_coordinates
    }

    /// The configured country filter.
    pub fn weather_country(&self) -> Option<String> {
        self.weather_country
            .as_ref()
            .map(|c| c.to_ascii_uppercase())
    }

    /// Minutes between two fetches.
    pub fn weather_refresh_minutes(&self) -> u64 {
        self.weather_refresh_minutes
    }

    /// The last fetched weather, with `stale` computed now.
    pub fn weather_view(&self) -> Result<WeatherView, HubError> {
        let unavailable = || HubError::Publish("weather unavailable".to_owned());
        if !self.weather_enabled {
            return Err(unavailable());
        }
        let mut view = self.weather_slot().view.clone().ok_or_else(unavailable)?;
        view.stale = is_stale(view.fetched_at, clock::now(), self.weather_refresh_minutes);
        Ok(view)
    }

    /// Publishes `payload` retained and echoes it into the registry. Whether the broker took it.
    fn publish_weather(&self, topic: String, payload: String) -> bool {
        let published = self
            .publish(Publish {
                topic: topic.clone(),
                payload: payload.clone(),
                retain: true,
            })
            .is_ok();
        if published {
            self.ingest(&topic, payload.as_bytes());
        }
        published
    }

    fn publish_availability(&self, online: bool) -> bool {
        let topic = entities::availability_topic(&self.discovery_prefix);
        self.publish_weather(topic, if online { "online" } else { "offline" }.to_owned())
    }

    /// Stores a fresh view, publishes the entities that changed, and tells the web when the data
    /// did. A payload the broker refused is sent again by the next call.
    pub fn apply_weather(&self, mut view: WeatherView) {
        view.stale = false;
        // Self-healing: configs that never went out, or that something removed from the
        // registry, are published again (which also re-seeds every state for republishing).
        // An empty retained config is deliberately not reacted to: two hubs would ping-pong.
        if !self.weather_slot().configs_ok || self.entity_view("weather_temperature").is_none() {
            self.publish_weather_configs();
        }
        let (changed, data_changed) = {
            let mut slot = self.weather_slot();
            let data_changed = slot.view.as_ref().is_none_or(|old| {
                let mut old = old.clone();
                old.fetched_at = view.fetched_at;
                old != view
            });
            slot.view = Some(view.clone());
            let states = entities::states(&view);
            let mut changed: Vec<(&'static str, String)> = states
                .iter()
                .filter(|(key, payload)| slot.published.get(key) != Some(payload))
                .cloned()
                .collect();
            // A value that went missing is published as unknown, never left frozen.
            changed.extend(
                slot.published
                    .keys()
                    .filter(|key| !states.iter().any(|(k, _)| k == *key))
                    .map(|key| (*key, UNKNOWN.to_owned())),
            );
            (changed, data_changed)
        };
        if self.weather_slot().online != Some(true) && self.publish_availability(true) {
            self.weather_slot().online = Some(true);
        }
        for (key, payload) in changed {
            let topic = entities::state_topic(&self.discovery_prefix, key);
            if self.publish_weather(topic, payload.clone()) {
                let mut slot = self.weather_slot();
                if payload == UNKNOWN {
                    slot.published.remove(key);
                } else {
                    slot.published.insert(key, payload);
                }
            }
        }
        if data_changed {
            self.broadcast(WsEvent::WeatherChanged);
        }
    }

    /// Marks the entities unavailable, once per transition.
    pub fn set_weather_offline(&self) {
        if self.weather_slot().online != Some(false) && self.publish_availability(false) {
            self.weather_slot().online = Some(false);
        }
    }

    /// Publishes every entity's retained config, and `offline` until the first fetch lands.
    pub fn publish_weather_configs(&self) {
        // Every key is seeded with a payload no state can equal, so the first apply after a
        // (re)start republishes each present value and sends `None` for each absent one: what a
        // persistent broker kept from the previous run never stays frozen.
        self.weather_slot().published = entities::KEYS
            .iter()
            .map(|key| (*key, String::new()))
            .collect();
        let mut failed = 0;
        for (topic, config) in entities::configs(&self.discovery_prefix) {
            if !self.publish_weather(topic, config.to_string()) {
                failed += 1;
            }
        }
        let availability_ok = self.publish_availability(false);
        if availability_ok {
            self.weather_slot().online = Some(false);
        }
        if failed > 0 || !availability_ok {
            tracing::warn!(
                failed,
                availability_ok,
                "weather: some entity configs were not published; retrying on the next update"
            );
        }
        self.weather_slot().configs_ok = failed == 0 && availability_ok;
    }

    /// While `[weather]` is off, clears what an earlier run left retained: the states under
    /// `P/weather/` and the configs of the weather device. Whether `topic` was one of them.
    /// A publish error is ignored: the next connect replays the retained message.
    pub(crate) fn clear_disabled_weather(&self, topic: &str, payload: &[u8], retain: bool) -> bool {
        if self.weather_enabled || !retain || payload.is_empty() {
            return false;
        }
        let prefix = self.discovery_prefix.as_str();
        let is_state = topic
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix("/weather/"))
            .is_some_and(|key| !key.is_empty());
        let is_own_config = || {
            let segments: Vec<&str> = topic
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(|rest| rest.split('/').collect())
                .unwrap_or_default();
            matches!(segments[..], [_, "weather", _, "config"]) && entities::is_own_config(payload)
        };
        if !(is_state || is_own_config()) {
            return false;
        }
        let _ = self.publish(Publish {
            topic: topic.to_owned(),
            payload: String::new(),
            retain: true,
        });
        true
    }
}
