//! A discovered entity: what it is, what it can do and how it is wired to MQTT.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::command::AlarmMode;
use super::kind::Kind;
use super::ui_override::UiOverride;

/// Home Assistant `entity_category`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum EntityCategory {
    #[default]
    Primary,
    Diagnostic,
    Config,
}

/// Keypad (digits) or text field for an alarm code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CodeFormat {
    #[default]
    Number,
    Text,
}

/// What an entity can do, as the app's `Capabilities`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Capabilities {
    pub brightness: bool,
    pub color_modes: BTreeSet<String>,
    /// The light's `effect_list`, in published order; empty when it has no effects.
    pub effects: Vec<String>,
    pub position: bool,
    /// A cover with a `command_topic`: it can be told to open and close.
    pub open_close: bool,
    /// A cover that can be stopped: a `command_topic` and a `payload_stop` that is not null.
    pub stop: bool,
    pub hvac_modes: Vec<String>,
    /// The climate's `fan_modes`, in published order; empty when it has no fan control.
    pub fan_modes: Vec<String>,
    /// The vacuum's supported actions (start, pause, stop, return_to_base, locate), in that order.
    pub vacuum_actions: Vec<String>,
    /// The vacuum's `fan_speed_list`; empty when it cannot set a fan speed.
    pub fan_speeds: Vec<String>,
    pub min_temp: Option<f64>,
    pub max_temp: Option<f64>,
    pub temp_step: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub options: Vec<String>,
    pub alarm_modes: BTreeSet<AlarmMode>,
    pub code_arm_required: bool,
    pub code_disarm_required: bool,
    pub code_format: CodeFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic: Option<crate::types::GenericCaps>,
    /// A Zigbee2MQTT group (its config lists `group` members), not a device of its own.
    #[serde(default)]
    pub z2m_group: bool,
}

/// The expanded discovery config: topics, templates and payload constants.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bindings {
    pub config: Value,
}

/// Top-level `*_topic` keys that are written to, never subscribed to, besides `*command_topic`.
const WRITE_ONLY_TOPIC_KEYS: &[&str] = &[
    "set_position_topic",
    "set_fan_speed_topic",
    "send_command_topic",
];

impl Bindings {
    /// The topics to subscribe to for this entity: every top-level `*_topic` string that is not
    /// a command topic, plus every `availability[].topic`.
    pub fn topics(&self) -> BTreeSet<String> {
        let mut topics = self.data_topics(true);
        if let Some(availability) = self.config.get("availability").and_then(Value::as_array) {
            for topic in availability
                .iter()
                .filter_map(|entry| entry.get("topic").and_then(Value::as_str))
            {
                topics.insert(topic.to_owned());
            }
        }
        topics
    }

    /// The topics the entity's availability is read from: the `availability[]` list when
    /// present, else `availability_topic` (Home Assistant treats the two as exclusive).
    pub fn availability_topics(&self) -> BTreeSet<String> {
        let listed: BTreeSet<String> = self
            .config
            .get("availability")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.get("topic").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !listed.is_empty() {
            return listed;
        }
        self.config
            .get("availability_topic")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .into_iter()
            .collect()
    }

    /// True when `topic` carries only this entity's availability: a message there says
    /// nothing about its value or attributes.
    pub fn is_availability_only(&self, topic: &str) -> bool {
        self.availability_topics().contains(topic) && !self.data_topics(false).contains(topic)
    }

    /// The top-level `*_topic` strings that are read (not command or write-only topics),
    /// `availability_topic` included only when `with_availability`.
    fn data_topics(&self, with_availability: bool) -> BTreeSet<String> {
        let mut topics = BTreeSet::new();
        let Some(config) = self.config.as_object() else {
            return topics;
        };
        for (key, value) in config {
            let is_topic = key.ends_with("_topic")
                || key == "state_topic"
                || key == "json_attributes_topic"
                || key == "topic";
            let is_command =
                key.ends_with("command_topic") || WRITE_ONLY_TOPIC_KEYS.contains(&key.as_str());
            let is_availability = key == "availability_topic";
            if is_topic
                && !is_command
                && (with_availability || !is_availability)
                && let Some(topic) = value.as_str()
            {
                topics.insert(topic.to_owned());
            }
        }
        if let Some(actions) = config.get("actions").and_then(Value::as_array) {
            topics.extend(
                actions
                    .iter()
                    .filter_map(|action| action.get("topic").and_then(Value::as_str))
                    .map(str::to_owned),
            );
        }
        topics
    }
}

/// A discovered entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub unique_id: String,
    pub device_id: Option<String>,
    /// Display name: `"<device name> <label>"` when both exist.
    pub name: String,
    /// Short device name when `name` is that device name followed by `label`.
    pub device_name: Option<String>,
    /// The entity's own label as published, or `None` when `name` is not device + label.
    pub label: Option<String>,
    /// The kind shown to the user; differs from `protocol_kind` only for a switch shown as a light.
    pub kind: Kind,
    /// The kind as discovered, which picks the MQTT mapper.
    pub protocol_kind: Kind,
    /// The Home Assistant discovery component this entity was created from
    /// (`light`, `switch`, `lock`, `fan`, `device_automation`, ...).
    pub component: String,
    pub device_class: Option<String>,
    pub unit: Option<String>,
    pub icon: Option<String>,
    pub category: EntityCategory,
    pub enabled_by_default: bool,
    pub capabilities: Capabilities,
    pub bindings: Bindings,
    /// `device.suggested_area` from discovery.
    pub suggested_area: Option<String>,
    /// Room taken from a hierarchical device name (second-to-last segment).
    pub path_area: Option<String>,
    /// Effective area id: override → `suggested_area` → `path_area` → [`super::UNASSIGNED_AREA`].
    pub area: String,
    /// The override applied to this entity (defaults when none).
    pub ui: UiOverride,
    /// Unique id of the entity this one is folded into (e.g. a Z2M "Moving" sensor into its cover).
    pub part_of: Option<String>,
    /// The discovery topic this entity came from.
    pub discovery_topic: String,
}
