//! Command mapping: from a client's [`Command`] to the MQTT publishes that carry it out,
//! following the Home Assistant MQTT field names the entity was discovered with.
//!
//! [`map_command`] validates the command against the entity's capabilities, then hands it to
//! the mapper of its `protocol_kind`. Every `Ok` carries at least one publish, retained only
//! when the entity's discovery config says `"retain": true` (default `false`); a command the
//! kind cannot take is [`CommandError::Unsupported`]. Nothing here panics on a hostile config:
//! a missing topic or a broken template is an error.

mod kinds;

use serde_json::Value;

use crate::config;
use crate::model::{Command, Entity, EntityState, Kind};
use crate::template::Templates;

/// One MQTT message to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publish {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
}

/// Why a command could not be mapped; nothing was published.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// The entity's `protocol_kind` has no mapping for this command.
    #[error("{command} is not supported by a {}", kind.component())]
    Unsupported { kind: Kind, command: String },
    /// The config lacks the topic (named by its config key) the command needs.
    #[error("the entity has no `{0}`")]
    MissingTopic(String),
    /// The value is outside what the entity accepts (its capabilities or the command's range).
    #[error("out of range: {0}")]
    OutOfRange(String),
    /// A command template of the entity failed to render.
    #[error("template: {0}")]
    Template(String),
    /// Arming (`code_arm_required`) or disarming (`code_disarm_required`) needs a code.
    #[error("a code is required")]
    MissingCode,
    /// The code differs from the panel's configured literal `code`.
    #[error("wrong code")]
    WrongCode,
}

/// Maps `cmd` for `entity` to the publishes that carry it out; `state` resolves
/// [`Command::Toggle`] and a default-schema light's `on_command_type: brightness`.
pub fn map_command(
    entity: &Entity,
    state: &EntityState,
    cmd: &Command,
    templates: &Templates,
) -> Result<Vec<Publish>, CommandError> {
    let mapper = Mapper {
        entity,
        state,
        config: &entity.bindings.config,
        templates,
        command: cmd,
    };
    let publishes = kinds::map(&mapper)?;
    if publishes.is_empty() {
        return Err(mapper.unsupported());
    }
    Ok(publishes)
}

/// The snake_case name of a command, as its JSON `type` tag.
pub fn command_name(command: &Command) -> &'static str {
    match command {
        Command::TurnOn { .. } => "turn_on",
        Command::TurnOff => "turn_off",
        Command::Toggle => "toggle",
        Command::SetPosition { .. } => "set_position",
        Command::Open => "open",
        Command::Close => "close",
        Command::Stop => "stop",
        Command::SetTemperature { .. } => "set_temperature",
        Command::SetHvacMode { .. } => "set_hvac_mode",
        Command::SetFanMode { .. } => "set_fan_mode",
        Command::SetNumber { .. } => "set_number",
        Command::SelectOption { .. } => "select_option",
        Command::Press => "press",
        Command::Arm { .. } => "arm",
        Command::Disarm { .. } => "disarm",
        Command::VacuumAction { .. } => "vacuum_action",
        Command::SetFanSpeed { .. } => "set_fan_speed",
        Command::Control { .. } => "control",
    }
}

/// One command seen through an entity's config.
#[derive(Clone, Copy)]
pub(crate) struct Mapper<'a> {
    pub(crate) entity: &'a Entity,
    pub(crate) state: &'a EntityState,
    pub(crate) config: &'a Value,
    pub(crate) templates: &'a Templates,
    pub(crate) command: &'a Command,
}

impl Mapper<'_> {
    pub(crate) fn unsupported(&self) -> CommandError {
        CommandError::Unsupported {
            kind: self.entity.protocol_kind,
            command: command_name(self.command).to_owned(),
        }
    }

    /// The topic named by `config[key]`.
    pub(crate) fn topic(&self, key: &str) -> Result<String, CommandError> {
        self.config
            .get(key)
            .and_then(Value::as_str)
            .filter(|topic| !topic.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| CommandError::MissingTopic(key.to_owned()))
    }

    /// A payload word from the config, with its Home Assistant default.
    pub(crate) fn word(&self, key: &str, default: &str) -> String {
        config::config_string_or(self.config, key, default)
    }

    /// `payload` through the template at `config[template_key]` when there is one, with
    /// `value` = `payload` plus `extra` in context; the bare payload otherwise.
    pub(crate) fn render(
        &self,
        template_key: &str,
        payload: &str,
        extra: &[(&str, Value)],
    ) -> Result<String, CommandError> {
        match self.config.get(template_key).and_then(Value::as_str) {
            Some(template) => self
                .templates
                .render_with(template, payload, extra)
                .map_err(|err| CommandError::Template(err.to_string())),
            None => Ok(payload.to_owned()),
        }
    }

    /// A publish on `config[topic_key]` carrying `payload`, rendered through
    /// `config[template_key]` when that key is given and present.
    pub(crate) fn publish(
        &self,
        topic_key: &str,
        template_key: Option<&str>,
        payload: &str,
        extra: &[(&str, Value)],
    ) -> Result<Publish, CommandError> {
        let topic = self.topic(topic_key)?;
        let payload = match template_key {
            Some(key) => self.render(key, payload, extra)?,
            None => payload.to_owned(),
        };
        let retain = self
            .config
            .get("retain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(Publish {
            topic,
            payload,
            retain,
        })
    }
}

/// `Ok` when `value` is one of `list`; `OutOfRange` otherwise, including for an empty list.
pub(crate) fn listed(value: &str, list: &[String], what: &str) -> Result<(), CommandError> {
    if list.iter().any(|item| item == value) {
        Ok(())
    } else {
        Err(CommandError::OutOfRange(format!(
            "{what} {value:?} is not one of {list:?}"
        )))
    }
}

/// Like [`listed`], but an empty list (nothing advertised) accepts anything.
fn listed_if_any(value: &str, list: &[String], what: &str) -> Result<(), CommandError> {
    if list.is_empty() {
        Ok(())
    } else {
        listed(value, list, what)
    }
}

/// `Ok` when `value` is finite and within the bounds that are given.
pub(crate) fn in_range(
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
    what: &str,
) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(CommandError::OutOfRange(format!(
            "{what} {value} is not a number"
        )));
    }
    if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
        return Err(CommandError::OutOfRange(format!(
            "{what} {value} is outside {min:?}..={max:?}"
        )));
    }
    Ok(())
}
