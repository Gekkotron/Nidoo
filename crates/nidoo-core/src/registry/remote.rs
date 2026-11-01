//! Folding the `device_automation` configs of one device into a single remote entity
//! (`device_automation.<node>`) whose `actions` are the per-config triggers.

use serde_json::{Map, Value, json};

use super::{Change, Registry, as_discovered};
use crate::discovery::{DeviceAutomationFragment, capabilities, naming};
use crate::model::{Bindings, Device, Entity, EntityCategory, Kind, UNASSIGNED_AREA, UiOverride};
use crate::types::GenericCaps;

const COMPONENT: &str = "device_automation";

impl Registry {
    /// Adds (or updates) one action on the remote of its device, creating the remote on the
    /// first action. `origin` is the discovery topic that carried the action.
    pub(super) fn upsert_device_automation(
        &mut self,
        fragment: DeviceAutomationFragment,
        origin: &str,
        changes: &mut Vec<Change>,
    ) {
        let remote_id = format!("{COMPONENT}.{}", fragment.node);
        // The topic now describes another device: take the action off the old remote first.
        if self
            .device_automation_actions
            .get(origin)
            .is_some_and(|(id, _)| *id != remote_id)
        {
            self.remove_device_automation(origin, changes);
        }
        let action = json!({
            "object": fragment.object,
            "topic": fragment.topic,
            "payload": fragment.payload,
            "type": fragment.action_type,
            "subtype": fragment.action_subtype,
        });
        let mut entity = match self.entities.get(&remote_id) {
            Some(existing) => existing.clone(),
            None => new_remote(&remote_id, &fragment.device, &fragment.node, origin),
        };
        let mut actions = actions_of(&entity);
        splice(&mut actions, action);
        let seeded = self.seeded_for(&remote_id);
        set_actions(&mut entity, merge(seeded, &actions));
        self.device_automation_actions.insert(
            origin.to_owned(),
            (remote_id.clone(), fragment.object.clone()),
        );
        self.store_remote(entity, changes);
    }

    /// Takes the action carried by `origin` off its remote; the last one removes the remote.
    /// Does nothing when `origin` carried no action.
    pub(super) fn remove_device_automation(&mut self, origin: &str, changes: &mut Vec<Change>) {
        let Some((remote_id, object)) = self.device_automation_actions.remove(origin) else {
            return;
        };
        let Some(mut entity) = self.entities.get(&remote_id).cloned() else {
            return;
        };
        let mut actions = actions_of(&entity);
        actions.retain(|a| a.get("object").and_then(Value::as_str) != Some(object.as_str()));
        let actions = merge(self.seeded_for(&remote_id), &actions);
        if actions.is_empty() {
            self.remove_entity(&remote_id, changes);
            return;
        }
        set_actions(&mut entity, actions);
        self.store_remote(entity, changes);
    }

    /// Seeds every remote from the `action` enum its device exposes in `bridge/devices`, so an
    /// action is a signal before it is first pressed: creates the missing remotes, merges the
    /// seeded actions with the discovered ones, and drops the seeded actions of a device that
    /// is gone or no longer exposes any (a remote left with none is removed).
    pub(super) fn sync_exposed_actions(&mut self, changes: &mut Vec<Change>) {
        let devices = self.devices.clone();
        for device in &devices {
            let seeded = self.seeded_actions(device);
            if seeded.is_empty() {
                continue;
            }
            let remote_id = format!("{COMPONENT}.{}", device.ieee);
            let mut entity = match self.entities.get(&remote_id) {
                Some(existing) => existing.clone(),
                None => new_remote(&remote_id, &synth_device(device), &device.ieee, ""),
            };
            let actions = merge(seeded, &actions_of(&entity));
            set_actions(&mut entity, actions);
            self.store_remote(entity, changes);
        }
        let remotes: Vec<String> = self
            .entities
            .values()
            .filter(|e| e.component == COMPONENT)
            .map(|e| e.unique_id.clone())
            .collect();
        for remote_id in remotes {
            if !self.seeded_for(&remote_id).is_empty() {
                continue;
            }
            let Some(mut entity) = self.entities.get(&remote_id).cloned() else {
                continue;
            };
            let all = actions_of(&entity);
            let kept = merge(Vec::new(), &all);
            if kept.len() == all.len() {
                continue;
            }
            if kept.is_empty() {
                self.remove_entity(&remote_id, changes);
            } else {
                set_actions(&mut entity, kept);
                self.store_remote(entity, changes);
            }
        }
    }

    /// The seeded actions of the device behind `remote_id`; none when the device is unknown.
    fn seeded_for(&self, remote_id: &str) -> Vec<Value> {
        let ieee = remote_id
            .strip_prefix(COMPONENT)
            .and_then(|r| r.strip_prefix('.'));
        match self.devices.iter().find(|d| Some(d.ieee.as_str()) == ieee) {
            Some(device) => self.seeded_actions(device),
            None => Vec::new(),
        }
    }

    /// One seeded action per value of the device's exposed `action` enum, shaped like a
    /// discovered action plus `"seeded": true`.
    fn seeded_actions(&self, device: &Device) -> Vec<Value> {
        let topic = format!("{}/{}/action", self.config.z2m_base, device.friendly_name);
        exposed_actions(device)
            .into_iter()
            .map(|p| {
                json!({
                    "object": format!("action_{p}"),
                    "topic": topic,
                    "payload": p,
                    "type": "action",
                    "subtype": p,
                    "seeded": true,
                })
            })
            .collect()
    }

    /// Stores the remote unless it is the same as discovered before.
    fn store_remote(&mut self, entity: Entity, changes: &mut Vec<Change>) {
        if let Some(previous) = self.entities.get(&entity.unique_id)
            && as_discovered(previous) == as_discovered(&entity)
        {
            return;
        }
        self.store(entity, changes);
    }
}

/// The values of the `action` enums a device exposes (top level and nested `features`), in
/// order, without duplicates or empty strings.
pub(super) fn exposed_actions(device: &Device) -> Vec<String> {
    fn walk(exposes: &Value, out: &mut Vec<String>) {
        let Some(items) = exposes.as_array() else {
            return;
        };
        for item in items {
            let is_action = ["property", "name"]
                .iter()
                .any(|k| item.get(k).and_then(Value::as_str) == Some("action"));
            if is_action && item.get("type").and_then(Value::as_str) == Some("enum") {
                for value in item
                    .get("values")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if !value.is_empty() && !out.iter().any(|v| v == value) {
                        out.push(value.to_owned());
                    }
                }
            }
            if let Some(features) = item.get("features") {
                walk(features, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&device.exposes, &mut out);
    out
}

/// The device as the discovery `device` block names it, for a remote made from `bridge/devices`.
fn synth_device(device: &Device) -> Value {
    json!({
        "identifiers": [device.id],
        "name": device.friendly_name,
        "manufacturer": device.vendor,
        "model": device.description,
        "model_id": device.model,
    })
}

/// The seeded actions in order, each replaced by the discovered action with the same payload,
/// then the discovered actions whose payload is not seeded, in their order.
fn merge(seeded: Vec<Value>, current: &[Value]) -> Vec<Value> {
    let payload = |a: &Value| a.get("payload").and_then(Value::as_str).map(str::to_owned);
    let discovered: Vec<&Value> = current
        .iter()
        .filter(|a| a.get("seeded").and_then(Value::as_bool) != Some(true))
        .collect();
    let mut out: Vec<Value> = seeded
        .into_iter()
        .map(|s| {
            discovered
                .iter()
                .find(|d| payload(d) == payload(&s))
                .map_or(s, |d| (*d).clone())
        })
        .collect();
    let seeded_payloads: Vec<Option<String>> = out.iter().map(payload).collect();
    out.extend(
        discovered
            .into_iter()
            .filter(|d| !seeded_payloads.contains(&payload(d)))
            .cloned(),
    );
    out
}

fn actions_of(entity: &Entity) -> Vec<Value> {
    entity
        .bindings
        .config
        .get("actions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Puts `action` in place of the one with the same object id or payload, else at the end,
/// then drops any other action with the same payload.
fn splice(actions: &mut Vec<Value>, action: Value) {
    let key = |a: &Value, k: &str| a.get(k).and_then(Value::as_str).map(str::to_owned);
    let (object, payload) = (key(&action, "object"), key(&action, "payload"));
    match actions
        .iter()
        .position(|a| key(a, "object") == object)
        .or_else(|| actions.iter().position(|a| key(a, "payload") == payload))
    {
        Some(at) => actions[at] = action,
        None => actions.push(action),
    }
    let mut seen = Vec::new();
    actions.retain(|a| {
        let p = key(a, "payload");
        if seen.contains(&p) {
            return false;
        }
        seen.push(p);
        true
    });
}

/// Writes `actions` into the config and refreshes the signals (the payloads, in order).
fn set_actions(entity: &mut Entity, actions: Vec<Value>) {
    let signals: Vec<String> = actions
        .iter()
        .filter_map(|a| a.get("payload").and_then(Value::as_str).map(str::to_owned))
        .collect();
    if let Some(config) = entity.bindings.config.as_object_mut() {
        config.insert("actions".to_owned(), Value::Array(actions));
    }
    let generic = entity
        .capabilities
        .generic
        .get_or_insert_with(GenericCaps::default);
    generic.signals = signals;
}

fn new_remote(id: &str, device_json: &Value, node: &str, origin: &str) -> Entity {
    let device = device_json.as_object();
    let device_id = device
        .and_then(|d| d.get("identifiers"))
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Array(items) => items.iter().find_map(Value::as_str).map(str::to_owned),
            _ => None,
        });
    let raw_name = device
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .or(device_id.as_deref());
    let (mut name, device_name, label, path_area) = naming::split(raw_name, None);
    if name.is_empty() {
        name = node.to_owned();
    }
    let suggested_area = device
        .and_then(|d| d.get("suggested_area"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut config = Map::new();
    config.insert("device".to_owned(), device_json.clone());
    config.insert("actions".to_owned(), Value::Array(Vec::new()));
    let value = Value::Object(config);
    let mut capabilities = capabilities::from_config(Kind::Generic, &value);
    capabilities.generic = Some(GenericCaps::default());
    Entity {
        unique_id: id.to_owned(),
        device_id,
        name,
        device_name,
        label,
        kind: Kind::Generic,
        protocol_kind: Kind::Generic,
        component: COMPONENT.to_owned(),
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        capabilities,
        bindings: Bindings { config: value },
        suggested_area,
        path_area,
        area: UNASSIGNED_AREA.to_owned(),
        ui: UiOverride::default(),
        part_of: None,
        discovery_topic: origin.to_owned(),
    }
}
