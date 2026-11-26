//! Ambiance wire types: one-tap house states made of actions. Shape validation lives here
//! (no house needed); checks against the house belong to the hub.

use crate::views::EntityView;
use nidoo_core::model::{
    Availability, Command, EntityCategory, Hs, Kind, StateValue, is_window_covering,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The icon names an ambiance may carry; the UI draws one glyph per name.
pub const AMBIANCE_ICONS: [&str; 12] = [
    "sofa", "moon", "door_out", "book", "sun", "sparkle", "film", "plate", "bath", "leaf", "flame",
    "house",
];

const MAX_ID: usize = 48;
const MAX_NAME: usize = 60;
const MAX_SUBTITLE: usize = 80;
const MAX_HOURS: u32 = 720;

/// A named set of actions the house applies in one tap. Unknown top-level fields are kept on
/// a round trip (`extra`) so a client can add display fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ambiance {
    /// Taken from the topic or the request path when a payload omits it.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// One of `AMBIANCE_ICONS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub actions: Vec<AmbianceAction>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One step of an ambiance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AmbianceAction {
    Category(CategoryAction),
    Device(DeviceAction),
    Climate(ClimateAction),
    Flow(FlowAction),
}

/// Sets every entity of a category, optionally limited to one area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CategoryAction {
    pub what: Category,
    pub to: Target,
    /// In percent, 1..=100; lights turned on only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
    /// An area id; `None` is the whole house.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// Entity ids left alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Lights,
    Plugs,
    Covers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    On,
    Off,
    Open,
    Close,
}

/// Sends one command to one entity (the same command `send_command` takes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceAction {
    pub entity: String,
    pub command: Command,
}

/// Overrides a climate zone for a while, or clears the override.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClimateAction {
    /// A zone id; `None` is every zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// A number or the string `"off"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setpoint: Option<Value>,
    /// `heat` or `cool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// How long the override lasts, 1..=720.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Enables, disables or runs a flow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowAction {
    pub flow: String,
    #[serde(rename = "do")]
    pub action: FlowDo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowDo {
    Enable,
    Disable,
    Run,
}

/// An ambiance as listed: valid, or kept visible with the reason it cannot be read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AmbianceView {
    pub id: String,
    pub ambiance: Option<Ambiance>,
    pub invalid: Option<String>,
}

/// What applying an ambiance did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyReport {
    pub ambiance: String,
    /// Unix seconds.
    pub started_at: u64,
    pub actions: Vec<ActionReport>,
}

/// One line of an apply report, for the action at `index`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionReport {
    pub index: usize,
    pub outcome: Outcome,
    /// Entities or zones targeted.
    pub total: usize,
    pub failed: Vec<Problem>,
    pub skipped: Vec<Problem>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    pub id: String,
    pub reason: String,
}

/// An id is 1 to 48 characters from `[a-z0-9_]`.
pub fn is_valid_ambiance_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Checks the shape of an ambiance, without a house: ids, lengths, icon and each action's
/// own rules. An action's error names its index.
pub fn validate_ambiance(a: &Ambiance) -> Result<(), String> {
    if !is_valid_ambiance_id(&a.id) {
        return Err(format!(
            "id must be 1 to {MAX_ID} characters from a-z, 0-9 and _"
        ));
    }
    if a.name.trim().is_empty() {
        return Err("name must not be empty".into());
    }
    if a.name.chars().count() > MAX_NAME {
        return Err(format!("name is longer than {MAX_NAME} characters"));
    }
    if a.subtitle
        .as_ref()
        .is_some_and(|s| s.chars().count() > MAX_SUBTITLE)
    {
        return Err(format!("subtitle is longer than {MAX_SUBTITLE} characters"));
    }
    if let Some(icon) = &a.icon
        && !AMBIANCE_ICONS.contains(&icon.as_str())
    {
        return Err(format!("unknown icon {icon:?}"));
    }
    for (i, action) in a.actions.iter().enumerate() {
        validate_action(action).map_err(|e| format!("action {i}: {e}"))?;
    }
    Ok(())
}

fn validate_action(action: &AmbianceAction) -> Result<(), String> {
    match action {
        AmbianceAction::Category(c) => validate_category(c),
        AmbianceAction::Climate(c) => validate_climate(c),
        AmbianceAction::Device(_) | AmbianceAction::Flow(_) => Ok(()),
    }
}

fn validate_category(c: &CategoryAction) -> Result<(), String> {
    match (c.what, c.to) {
        (Category::Lights | Category::Plugs, Target::On | Target::Off) => {}
        (Category::Lights | Category::Plugs, _) => {
            return Err("lights and plugs take on or off".into());
        }
        (Category::Covers, Target::Open | Target::Close) => {}
        (Category::Covers, _) => return Err("covers take open or close".into()),
    }
    if let Some(b) = c.brightness {
        if c.what != Category::Lights || c.to != Target::On {
            return Err("brightness only applies to lights turned on".into());
        }
        if !(1..=100).contains(&b) {
            return Err("brightness must be between 1 and 100".into());
        }
    }
    Ok(())
}

fn validate_climate(c: &ClimateAction) -> Result<(), String> {
    if let Some(mode) = &c.mode
        && mode != "heat"
        && mode != "cool"
    {
        return Err("mode must be heat or cool".into());
    }
    if let Some(setpoint) = &c.setpoint
        && !(setpoint.is_number() || setpoint.as_str() == Some("off"))
    {
        return Err("setpoint must be a number or \"off\"".into());
    }
    if c.clear {
        if c.setpoint.is_some() || c.mode.is_some() || c.hours.is_some() {
            return Err("clear cannot come with a setpoint, a mode or hours".into());
        }
        return Ok(());
    }
    if c.setpoint.is_none() && c.mode.is_none() {
        return Err("climate needs clear, a setpoint or a mode".into());
    }
    match c.hours {
        Some(h) if (1..=MAX_HOURS).contains(&h) => Ok(()),
        _ => Err(format!("hours must be between 1 and {MAX_HOURS}")),
    }
}

/// Whether an entity may be commanded as a member of a category or of an assistant order: a
/// primary, enabled entity that is not part of a device, a group (hub or Zigbee2MQTT), a
/// hub-owned virtual entity or the Zigbee2MQTT bridge. A hidden entity still belongs: hiding only
/// removes a card.
pub fn commandable(e: &EntityView) -> bool {
    e.category == EntityCategory::Primary
        && e.enabled_by_default
        && e.part_of.is_none()
        && !e.id.starts_with("group_")
        && !e.id.starts_with("virtual_")
        && !e.capabilities.z2m_group
        && !e
            .device_id
            .as_deref()
            .is_some_and(|d| d.starts_with("zigbee2mqtt_bridge_"))
}

/// The entities a category action targets, offline ones included (the UI counts them).
/// See [`commandable`] for which entities never belong.
pub fn category_members<'a>(
    action: &CategoryAction,
    entities: &'a [EntityView],
) -> Vec<&'a EntityView> {
    entities
        .iter()
        .filter(|e| {
            commandable(e)
                && !action.except.contains(&e.id)
                && action.area.as_ref().is_none_or(|a| &e.area == a)
                && in_category(action.what, e)
        })
        .collect()
}

fn in_category(what: Category, e: &EntityView) -> bool {
    match what {
        Category::Lights => e.kind == Kind::Light,
        Category::Plugs => {
            e.protocol_kind == Kind::Switch
                // An untyped switch is a plain switch, which could drive anything.
                && e.ui.show_as.as_deref() == Some("plug")
        }
        Category::Covers => {
            e.kind == Kind::Cover && is_window_covering(e.device_class.as_deref(), &e.ui)
        }
    }
}

/// What applying a category action does: commands to send, entities already in the
/// wanted state, and entities that cannot be reached.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CategoryPlan {
    pub send: Vec<(String, Command)>,
    pub already: Vec<String>,
    pub offline: Vec<String>,
}

pub fn plan_category(action: &CategoryAction, entities: &[EntityView]) -> CategoryPlan {
    let mut plan = CategoryPlan::default();
    for e in category_members(action, entities) {
        let Some(cmd) = member_command(action, e) else {
            continue;
        };
        if e.state.availability == Availability::Offline {
            plan.offline.push(e.id.clone());
        } else if already_there(e, &cmd) {
            plan.already.push(e.id.clone());
        } else {
            plan.send.push((e.id.clone(), cmd));
        }
    }
    plan
}

/// The command for one member; `None` when the target does not fit the category.
fn member_command(action: &CategoryAction, e: &EntityView) -> Option<Command> {
    let plain_on = Command::TurnOn {
        brightness: None,
        color: None,
        effect: None,
    };
    match (action.what, action.to) {
        (Category::Lights | Category::Plugs, Target::Off) => Some(Command::TurnOff),
        (Category::Lights, Target::On) => {
            let brightness = action
                .brightness
                .filter(|_| e.capabilities.brightness)
                .map(|pct| (f64::from(pct) * 255.0 / 100.0).round().min(255.0) as u8);
            Some(Command::TurnOn {
                brightness,
                color: None,
                effect: None,
            })
        }
        (Category::Plugs, Target::On) => Some(plain_on),
        (Category::Covers, Target::Open) => Some(Command::Open),
        (Category::Covers, Target::Close) => Some(Command::Close),
        _ => None,
    }
}

/// Two colours a few degrees and points apart; any hue is the same white.
fn same_colour(a: Hs, b: Hs) -> bool {
    let hue = (a.h - b.h).rem_euclid(360.0);
    (a.s - b.s).abs() <= 5.0 && (a.s.max(b.s) <= 5.0 || hue.min(360.0 - hue) <= 5.0)
}

/// True when the entity's known state already matches what `cmd` would set.
pub fn already_there(view: &EntityView, cmd: &Command) -> bool {
    let value = &view.state.value;
    let is_on = match value {
        StateValue::OnOff(on) => Some(*on),
        StateValue::Light(l) => Some(l.on),
        _ => None,
    };
    match cmd {
        Command::TurnOff => is_on == Some(false),
        Command::TurnOn {
            brightness: None,
            color: None,
            ..
        } => is_on == Some(true),
        // A brightness or a colour must match too: a lit lamp asked for red is not there yet.
        Command::TurnOn {
            brightness, color, ..
        } => match value {
            StateValue::Light(l) => {
                l.on && brightness
                    .is_none_or(|b| l.brightness.is_some_and(|cur| cur.abs_diff(b) <= 5))
                    && color.is_none_or(|c| l.hs.is_some_and(|cur| same_colour(cur, c)))
            }
            _ => false,
        },
        // A known position wins: a half-open shutter is "open" by its state word, yet an open
        // order still has to raise it fully.
        Command::Close => match value {
            StateValue::Cover(c) => match c.position {
                Some(p) => p == 0,
                None => c.state.as_deref() == Some("closed"),
            },
            _ => false,
        },
        Command::Open => match value {
            StateValue::Cover(c) => match c.position {
                Some(p) => p == 100,
                None => c.state.as_deref() == Some("open"),
            },
            _ => false,
        },
        Command::SetPosition { position } => {
            matches!(value, StateValue::Cover(c) if c.position == Some(*position))
        }
        Command::SelectOption { option } => matches!(value, StateValue::Text(t) if t == option),
        Command::SetTemperature { temperature } => {
            matches!(value, StateValue::Climate(c) if c.target.is_some_and(|t| (t - temperature).abs() < 0.05))
        }
        Command::SetHvacMode { mode } => {
            matches!(value, StateValue::Climate(c) if c.mode.as_deref() == Some(mode.as_str()))
        }
        // Playing already, or paused already; next, previous and a search always act.
        Command::MediaAction { action } if matches!(action.as_str(), "play" | "pause") => {
            let wanted = if action == "play" {
                "playing"
            } else {
                "paused"
            };
            matches!(value, StateValue::MediaPlayer(p) if p.state.as_deref() == Some(wanted))
        }
        // Cleaning already, docked or on its way home already, idle or paused already. A room
        // button's `Press` is never "already there".
        Command::VacuumAction { action } => {
            let StateValue::Vacuum(v) = value else {
                return false;
            };
            let state = v.state.as_deref();
            match action.as_str() {
                "start" => state == Some("cleaning"),
                "return_to_base" => matches!(state, Some("docked" | "returning")),
                "stop" => matches!(state, Some("docked" | "idle")),
                "pause" => state == Some("paused"),
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
