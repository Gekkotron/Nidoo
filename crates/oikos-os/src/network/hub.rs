//! The hub side of network watches: set, delete and list.

use oikos_api::{NetworkWatchSpec, NetworkWatchView, OverridePatch, WatchState, WatchStatus};
use oikos_core::command::Publish;
use serde_json::{Value, json};

use super::discovery::{build_config, standalone_device};
use super::topics::{
    availability_topic, config_topic, definition_topic, id_for_device, state_topic, unique_id,
    validate_spec,
};
use crate::hub::{Hub, HubError};
use crate::virtual_entities;

impl Hub {
    /// Fails with a `Publish` error (503 over HTTP) while `[network] enabled` is false.
    fn require_network(&self) -> Result<(), HubError> {
        if self.network_enabled {
            Ok(())
        } else {
            Err(HubError::Publish("network presence disabled".to_owned()))
        }
    }

    /// Creates or replaces watch `id`: validates everything first, then publishes the
    /// definition, discovery config and availability retained. The state is left to the first
    /// ping verdict. A watch on a device joins that device; a standalone watch gets its own.
    pub async fn set_network_watch(
        &self,
        id: &str,
        spec: NetworkWatchSpec,
    ) -> Result<NetworkWatchView, HubError> {
        self.require_network()?;
        virtual_entities::validate_id(id).map_err(HubError::Rejected)?;
        validate_spec(&spec).map_err(HubError::Rejected)?;
        let device_block = match spec.device.as_deref() {
            Some(device) => {
                let Some(identifiers) = self.device_identifiers(device) else {
                    return Err(HubError::Rejected(format!("unknown device: {device}")));
                };
                if id_for_device(device).as_deref() != Some(id) {
                    return Err(HubError::Rejected(format!(
                        "the watch id of device {device} must be {:?}",
                        id_for_device(device).unwrap_or_default()
                    )));
                }
                let taken = self
                    .network_store()
                    .all()
                    .iter()
                    .any(|(other, s)| other != id && s.device.as_deref() == Some(device));
                if taken {
                    return Err(HubError::Rejected(format!(
                        "device {device} already has a watch"
                    )));
                }
                json!({ "identifiers": identifiers })
            }
            None => standalone_device(id, &spec.name),
        };

        let prefix = &self.discovery_prefix;
        let config_topic = config_topic(prefix, id);
        let config = build_config(prefix, id, &spec, device_block).to_string();
        let definition = serde_json::to_string(&spec)
            .map_err(|err| HubError::Rejected(format!("cannot encode the watch: {err}")))?;
        let definition_topic = definition_topic(prefix, id);
        self.publish(Publish {
            topic: config_topic.clone(),
            payload: config.clone(),
            retain: true,
        })?;
        self.publish(Publish {
            topic: availability_topic(prefix, id),
            payload: if spec.enabled { "online" } else { "offline" }.to_owned(),
            retain: true,
        })?;
        self.publish(Publish {
            topic: definition_topic.clone(),
            payload: definition.clone(),
            retain: true,
        })?;
        self.ingest(&definition_topic, definition.as_bytes());
        self.ingest(&config_topic, config.as_bytes());

        if spec.device.is_none()
            && let Some(area) = spec.area.as_deref()
        {
            let mut patch = OverridePatch::new();
            patch.insert("area".to_owned(), Value::from(area));
            self.set_override(&unique_id(id), &patch).await?;
        }
        Ok(self.network_view(id, spec))
    }

    /// Removes watch `id`: empty retained payloads on its definition, config, state and
    /// availability topics. The UI override is left in place.
    pub async fn delete_network_watch(&self, id: &str) -> Result<(), HubError> {
        self.require_network()?;
        if self.network_store().get(id).is_none() {
            return Err(HubError::UnknownEntity(unique_id(id)));
        }
        let prefix = &self.discovery_prefix;
        let config_topic = config_topic(prefix, id);
        let definition_topic = definition_topic(prefix, id);
        for topic in [
            definition_topic.clone(),
            config_topic.clone(),
            state_topic(prefix, id),
            availability_topic(prefix, id),
        ] {
            self.publish(Publish {
                topic,
                payload: String::new(),
                retain: true,
            })?;
        }
        self.ingest(&definition_topic, b"");
        self.ingest(&config_topic, b"");
        Ok(())
    }

    /// Every stored watch with its status, sorted by id.
    pub fn list_network_watches(&self) -> Result<Vec<NetworkWatchView>, HubError> {
        self.require_network()?;
        let stored: Vec<(String, NetworkWatchSpec)> = self
            .network_store()
            .all()
            .iter()
            .map(|(id, spec)| (id.clone(), spec.clone()))
            .collect();
        Ok(stored
            .into_iter()
            .map(|(id, spec)| self.network_view(&id, spec))
            .collect())
    }

    fn network_view(&self, id: &str, spec: NetworkWatchSpec) -> NetworkWatchView {
        let status = if !spec.enabled {
            WatchStatus {
                state: WatchState::Paused,
                since: None,
                last_reply: None,
                rtt_ms: None,
            }
        } else {
            self.network_status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(id)
                .cloned()
                .unwrap_or(WatchStatus {
                    state: WatchState::Waiting,
                    since: None,
                    last_reply: None,
                    rtt_ms: None,
                })
        };
        NetworkWatchView {
            id: id.to_owned(),
            spec,
            entity: unique_id(id),
            status,
        }
    }

    /// The discovery `device.identifiers` of device `device_id`, taken from the first of its
    /// entities that carries them; `None` when no entity belongs to the device.
    fn device_identifiers(&self, device_id: &str) -> Option<Value> {
        let reg = self.read();
        let mut entities = reg
            .entities()
            .filter(|e| e.device_id.as_deref() == Some(device_id))
            .peekable();
        entities.peek()?;
        let found = entities.find_map(|e| {
            e.bindings
                .config
                .get("device")
                .and_then(|d| d.get("identifiers"))
                .cloned()
        });
        Some(match found {
            Some(Value::String(one)) => json!([one]),
            Some(list) => list,
            None => json!([device_id]),
        })
    }
}
