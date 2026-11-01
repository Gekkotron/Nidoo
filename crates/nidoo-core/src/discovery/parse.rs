//! From a discovery payload to an [`Entity`].

use serde_json::{Map, Value};

use super::capabilities;
use super::expand::expand_object;
use super::naming;
use super::topic::DiscoveryTopic;
use crate::model::{
    Bindings, Entity, EntityCategory, IGNORED_COMPONENTS, Kind, UNASSIGNED_AREA, UiOverride,
};
use crate::types::GenericCaps;

/// Why a discovery payload could not be used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// The payload is not UTF-8, not a JSON object, or lacks a field the kind needs.
    #[error("invalid discovery payload: {0}")]
    Invalid(String),
}

/// What a discovery payload turned into.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Entity(Box<Entity>),
    /// An empty payload: the entity was removed.
    Removed,
    /// Recognised but not an entity (ignored component, or a device-based config to expand).
    Ignored {
        reason: String,
    },
    /// One `device_automation` action; the registry folds the actions of a device into one
    /// remote entity.
    DeviceAutomation(DeviceAutomationFragment),
}

/// One `device_automation` trigger config, before it is folded into its device's remote.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceAutomationFragment {
    /// The device's node id: the discovery topic's node (else the first device identifier),
    /// lowercased with every non-alphanumeric character turned into `_`.
    pub node: String,
    /// The object id of the discovery topic.
    pub object: String,
    /// The topic the action is published on.
    pub topic: String,
    /// The payload that identifies the action.
    pub payload: String,
    pub action_type: Option<String>,
    pub action_subtype: Option<String>,
    /// The shared `device` object.
    pub device: Value,
}

/// The reason given for a device-based (`device` component) config holding `components`.
pub const DEVICE_BASED_REASON: &str = "device-based discovery: expanded by caller";

/// Kinds whose state can only be read from `state_topic`.
const REQUIRES_STATE_TOPIC: &[Kind] = &[Kind::Sensor, Kind::BinarySensor];

/// Parses one discovery payload published on `topic_str` (classified as `topic`).
pub fn parse(
    topic: &DiscoveryTopic,
    payload: &[u8],
    topic_str: &str,
) -> Result<Parsed, DiscoveryError> {
    let text = utf8(payload)?;
    if text.trim().is_empty() {
        return Ok(Parsed::Removed);
    }
    if topic.component == "device" {
        let config = object(text)?;
        return if config.contains_key("components") || config.contains_key("cmps") {
            Ok(Parsed::Ignored {
                reason: DEVICE_BASED_REASON.to_owned(),
            })
        } else {
            Err(DiscoveryError::Invalid(
                "device-based discovery without `cmps`".to_owned(),
            ))
        };
    }
    if IGNORED_COMPONENTS.contains(&topic.component.as_str()) {
        return Ok(Parsed::Ignored {
            reason: format!("ignored component `{}`", topic.component),
        });
    }
    let kind = Kind::from_component(&topic.component).filter(|k| *k != Kind::Generic);
    let Some(kind) = kind else {
        return parse_generic(topic, text, topic_str);
    };
    let config = expand_object(object(text)?);
    if REQUIRES_STATE_TOPIC.contains(&kind)
        && !config.get("state_topic").is_some_and(Value::is_string)
    {
        return Err(DiscoveryError::Invalid(format!(
            "{} without state_topic",
            topic.component
        )));
    }
    Ok(Parsed::Entity(Box::new(build(
        kind, topic, config, topic_str,
    ))))
}

/// A component the type table describes becomes a [`Kind::Generic`] entity; any other
/// component is unsupported.
fn parse_generic(
    topic: &DiscoveryTopic,
    text: &str,
    topic_str: &str,
) -> Result<Parsed, DiscoveryError> {
    if topic.component == "device_automation" {
        return parse_device_automation(topic, text);
    }
    let Some(def) = crate::types::type_def(&topic.component) else {
        return Ok(Parsed::Ignored {
            reason: format!("unsupported component `{}`", topic.component),
        });
    };
    let config = expand_object(object(text)?);
    if def.requires_state
        && let Some(field) = def.primary.as_ref().and_then(|p| def.fields.get(p))
        && !config.get(&field.topic).is_some_and(Value::is_string)
    {
        return Err(DiscoveryError::Invalid(format!(
            "{} without {}",
            topic.component, field.topic
        )));
    }
    let mut entity = build(Kind::Generic, topic, config, topic_str);
    entity.capabilities.generic = Some(GenericCaps::from_config(def, &entity.bindings.config));
    Ok(Parsed::Entity(Box::new(entity)))
}

/// A `device_automation` config becomes a fragment for the registry to fold.
fn parse_device_automation(topic: &DiscoveryTopic, text: &str) -> Result<Parsed, DiscoveryError> {
    let config = expand_object(object(text)?);
    let (Some(action_topic), Some(payload)) = (
        string(&config, "topic"),
        config.get("payload").and_then(Value::as_str),
    ) else {
        return Err(DiscoveryError::Invalid(
            "device_automation without topic or payload".to_owned(),
        ));
    };
    let device = config
        .get("device")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let raw_node = topic
        .node_id
        .clone()
        .or_else(|| device.get("identifiers").and_then(first_string))
        .unwrap_or_else(|| topic.object_id.clone());
    let node = raw_node
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    Ok(Parsed::DeviceAutomation(DeviceAutomationFragment {
        node,
        object: topic.object_id.clone(),
        topic: action_topic,
        payload: payload.to_owned(),
        action_type: string(&config, "type"),
        action_subtype: string(&config, "subtype"),
        device,
    }))
}

/// Splits a device-based config (`device` component with `components`) into one per-entity
/// config per entry, merging the shared keys (`device`, `origin`, `availability`, `~`, ...);
/// entries that are only a `platform` (removal markers) or have none are skipped.
pub fn expand_device_based(
    topic: &DiscoveryTopic,
    payload: &[u8],
) -> Result<Vec<(DiscoveryTopic, Value)>, DiscoveryError> {
    let mut config = expand_object(object(utf8(payload)?)?);
    let Some(Value::Object(components)) = config.remove("components") else {
        return Err(DiscoveryError::Invalid(
            "device-based discovery without `cmps`".to_owned(),
        ));
    };
    let shared = config;
    let mut expanded = Vec::with_capacity(components.len());
    for (component_id, entry) in components {
        let Value::Object(mut entry) = entry else {
            continue;
        };
        let Some(Value::String(platform)) = entry.remove("platform") else {
            continue;
        };
        if entry.is_empty() {
            continue;
        }
        let mut merged = shared.clone();
        merged.extend(entry);
        let entity_topic = DiscoveryTopic {
            prefix: topic.prefix.clone(),
            component: platform,
            node_id: Some(topic.object_id.clone()),
            object_id: component_id,
        };
        expanded.push((entity_topic, Value::Object(merged)));
    }
    Ok(expanded)
}

fn build(
    kind: Kind,
    topic: &DiscoveryTopic,
    config: Map<String, Value>,
    topic_str: &str,
) -> Entity {
    let device = config.get("device").and_then(Value::as_object);
    let device_id = device
        .and_then(|d| d.get("identifiers"))
        .and_then(first_string);
    let suggested_area = device.and_then(|d| string(d, "suggested_area"));
    let raw_device_name = device
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .or(device_id.as_deref());
    let entity_name = config.get("name").and_then(Value::as_str);
    let (mut name, device_name, label, path_area) = naming::split(raw_device_name, entity_name);
    if name.is_empty() {
        name = topic.object_id.clone();
    }
    let unique_id = string(&config, "unique_id").unwrap_or_else(|| {
        [
            Some(topic.component.as_str()),
            topic.node_id.as_deref(),
            Some(topic.object_id.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(".")
    });
    let category = match config.get("entity_category").and_then(Value::as_str) {
        Some("diagnostic") => EntityCategory::Diagnostic,
        Some("config") => EntityCategory::Config,
        _ => EntityCategory::Primary,
    };
    let enabled_by_default = config
        .get("enabled_by_default")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let device_class = string(&config, "device_class");
    let unit = string(&config, "unit_of_measurement");
    let icon = string(&config, "icon");
    let value = Value::Object(config);
    let capabilities = capabilities::from_config(kind, &value);
    Entity {
        unique_id,
        device_id,
        name,
        device_name,
        label,
        kind,
        protocol_kind: kind,
        component: topic.component.clone(),
        device_class,
        unit,
        icon,
        category,
        enabled_by_default,
        capabilities,
        bindings: Bindings { config: value },
        suggested_area,
        path_area,
        area: UNASSIGNED_AREA.to_owned(),
        ui: UiOverride::default(),
        part_of: None,
        discovery_topic: topic_str.to_owned(),
    }
}

fn utf8(payload: &[u8]) -> Result<&str, DiscoveryError> {
    std::str::from_utf8(payload)
        .map_err(|_| DiscoveryError::Invalid("payload is not UTF-8".to_owned()))
}

fn object(text: &str) -> Result<Map<String, Value>, DiscoveryError> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(DiscoveryError::Invalid(
            "payload is not a JSON object".to_owned(),
        )),
        Err(e) => Err(DiscoveryError::Invalid(format!("payload is not JSON: {e}"))),
    }
}

fn string(config: &Map<String, Value>, key: &str) -> Option<String> {
    config.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A string value, or the first string of an array value.
fn first_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(Value::as_str).map(str::to_owned),
        _ => None,
    }
}
