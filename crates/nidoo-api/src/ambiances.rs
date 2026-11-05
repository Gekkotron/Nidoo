//! Ambiance wire types: one-tap house states made of actions. Shape validation lives here
//! (no house needed); checks against the house belong to the hub.

use crate::views::EntityView;
use nidoo_core::model::{Availability, Command, EntityCategory, Kind, StateValue};
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

/// Cover classes that are window coverings; garages, gates, doors and dampers never belong.
const WINDOW_COVERINGS: [&str; 6] = ["shutter", "blind", "curtain", "shade", "awning", "window"];

/// The entities a category action targets, offline ones included (the UI counts them).
/// A hidden entity still belongs: hiding only removes a card. Groups (hub and Zigbee2MQTT),
/// hub-owned virtual entities and the Zigbee2MQTT bridge never belong.
pub fn category_members<'a>(
    action: &CategoryAction,
    entities: &'a [EntityView],
) -> Vec<&'a EntityView> {
    entities
        .iter()
        .filter(|e| {
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
            e.kind == Kind::Cover
                && e.device_class
                    .as_deref()
                    .is_none_or(|class| WINDOW_COVERINGS.contains(&class))
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
            brightness: None, ..
        } => is_on == Some(true),
        Command::TurnOn {
            brightness: Some(b),
            ..
        } => match value {
            StateValue::Light(l) => l.on && l.brightness.is_some_and(|cur| cur.abs_diff(*b) <= 5),
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
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{ClimateState, CoverState, EntityState, LightState};
    use serde_json::json;

    /// Spec Appendix A.2, without the comment. The command is the real `select_option`
    /// shape (the spec's `select` was a placeholder).
    fn spec_example() -> Value {
        json!({
          "id": "bonne_nuit",
          "name": "Bonne nuit",
          "subtitle": "La maison s'endort",
          "icon": "moon",
          "pinned": true,
          "order": 1,
          "actions": [
            { "type": "category", "what": "lights", "to": "off", "area": null, "except": [] },
            { "type": "category", "what": "lights", "to": "on", "brightness": 30, "area": "living_room" },
            { "type": "category", "what": "plugs", "to": "off" },
            { "type": "category", "what": "covers", "to": "close" },
            { "type": "device", "entity": "select.house_mode", "command": { "type": "select_option", "option": "night" } },
            { "type": "climate", "zone": null, "setpoint": 16.0, "mode": null, "hours": 12 },
            { "type": "climate", "zone": null, "clear": true },
            { "type": "flow", "flow": "away_lights", "do": "enable" }
          ]
        })
    }

    /// Null and absent are the same on the wire: optional fields are not serialized when
    /// empty, so the expected value drops its nulls and empty `except` lists.
    fn without_empty(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, v)| !v.is_null() && !(k.as_str() == "except" && *v == &json!([])))
                    .map(|(k, v)| (k.clone(), without_empty(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(without_empty).collect()),
            other => other.clone(),
        }
    }

    fn ambiance(actions: Value) -> Ambiance {
        serde_json::from_value(json!({"id": "a", "name": "A", "actions": actions})).unwrap()
    }

    #[test]
    fn spec_example_round_trips() {
        let value = spec_example();
        let a: Ambiance = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(a.actions.len(), 8);
        assert_eq!(serde_json::to_value(&a).unwrap(), without_empty(&value));
    }

    #[test]
    fn unknown_top_level_fields_survive() {
        let value = json!({"id": "a", "name": "A", "pinned": false, "order": 0, "actions": [], "color": "#fff"});
        let a: Ambiance = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(a.extra["color"], "#fff");
        assert_eq!(serde_json::to_value(&a).unwrap(), value);
    }

    #[test]
    fn minimal_ambiance_defaults() {
        let a: Ambiance = serde_json::from_value(json!({"id": "a", "name": "A"})).unwrap();
        assert!(!a.pinned);
        assert_eq!(a.order, 0);
        assert!(a.actions.is_empty());
        let no_id: Ambiance = serde_json::from_value(json!({"name": "A"})).unwrap();
        assert_eq!(no_id.id, "");
    }

    #[test]
    fn validate_accepts_spec_example() {
        let a: Ambiance = serde_json::from_value(spec_example()).unwrap();
        assert_eq!(validate_ambiance(&a), Ok(()));
    }

    #[test]
    fn validate_rejects() {
        let cat = |v: Value| ambiance(json!([v]));
        let climate = |v: Value| {
            let mut m = json!({"type": "climate"});
            m.as_object_mut()
                .unwrap()
                .extend(v.as_object().unwrap().clone());
            ambiance(json!([m]))
        };
        let base = || ambiance(json!([]));
        let mut cases: Vec<(&str, Ambiance)> = Vec::new();
        let mut bad = base();
        bad.id = "Bonne Nuit".into();
        cases.push(("id with caps and space", bad));
        let mut bad = base();
        bad.id = "a".repeat(49);
        cases.push(("id too long", bad));
        let mut bad = base();
        bad.name = "  ".into();
        cases.push(("blank name", bad));
        let mut bad = base();
        bad.name = "n".repeat(61);
        cases.push(("name too long", bad));
        let mut bad = base();
        bad.subtitle = Some("s".repeat(81));
        cases.push(("subtitle too long", bad));
        let mut bad = base();
        bad.icon = Some("rocket".into());
        cases.push(("unknown icon", bad));
        cases.push((
            "lights open",
            cat(json!({"type": "category", "what": "lights", "to": "open"})),
        ));
        cases.push((
            "covers off",
            cat(json!({"type": "category", "what": "covers", "to": "off"})),
        ));
        cases.push((
            "plugs brightness",
            cat(json!({"type": "category", "what": "plugs", "to": "on", "brightness": 50})),
        ));
        cases.push((
            "lights off brightness",
            cat(json!({"type": "category", "what": "lights", "to": "off", "brightness": 50})),
        ));
        for b in [0, 101] {
            cases.push((
                "brightness out of range",
                cat(json!({"type": "category", "what": "lights", "to": "on", "brightness": b})),
            ));
        }
        cases.push((
            "clear with setpoint",
            climate(json!({"clear": true, "setpoint": 16})),
        ));
        cases.push((
            "clear with hours",
            climate(json!({"clear": true, "hours": 2})),
        ));
        cases.push(("empty climate", climate(json!({"zone": "z"}))));
        cases.push(("setpoint without hours", climate(json!({"setpoint": 16}))));
        for h in [0, 721] {
            cases.push((
                "hours out of range",
                climate(json!({"setpoint": 16, "hours": h})),
            ));
        }
        cases.push((
            "bad mode",
            climate(json!({"mode": "dry", "setpoint": 16, "hours": 2})),
        ));
        cases.push((
            "bad setpoint",
            climate(json!({"setpoint": "warm", "hours": 2})),
        ));
        for (label, a) in &cases {
            assert!(validate_ambiance(a).is_err(), "{label} should be refused");
        }
    }

    #[test]
    fn action_errors_name_the_index() {
        let a = ambiance(json!([
            {"type": "category", "what": "plugs", "to": "off"},
            {"type": "category", "what": "covers", "to": "off"},
        ]));
        assert_eq!(
            validate_ambiance(&a),
            Err("action 1: covers take open or close".to_string())
        );
    }

    fn entity(id: &str, kind: Kind) -> EntityView {
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
            area: "living".into(),
            part_of: None,
            capabilities: Default::default(),
            ui: Default::default(),
            state: Default::default(),
        }
    }

    fn switch_as(id: &str, show_as: Option<&str>) -> EntityView {
        let mut e = entity(id, Kind::Switch);
        e.ui.show_as = show_as.map(Into::into);
        e
    }

    fn act(what: Category, to: Target) -> CategoryAction {
        CategoryAction {
            what,
            to,
            brightness: None,
            area: None,
            except: vec![],
        }
    }

    fn ids(v: Vec<&EntityView>) -> Vec<&str> {
        v.into_iter().map(|e| e.id.as_str()).collect()
    }

    fn light_state(on: bool, brightness: Option<u8>) -> EntityState {
        EntityState {
            value: StateValue::Light(LightState {
                on,
                brightness,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn with_state(mut e: EntityView, value: StateValue) -> EntityView {
        e.state.value = value;
        e
    }

    #[test]
    fn plugs_never_include_door_siren_chime() {
        let all = [
            switch_as("door", Some("door")),
            switch_as("siren", Some("siren")),
            switch_as("chime", Some("chime")),
            switch_as("plain", None),
            switch_as("plug", Some("plug")),
        ];
        let a = act(Category::Plugs, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["plug"]);
    }

    #[test]
    fn untyped_switch_is_a_switch_not_a_plug() {
        let all = [switch_as("relay", None), switch_as("plug", Some("plug"))];
        let a = act(Category::Plugs, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["plug"]);
    }

    #[test]
    fn switch_shown_as_light_is_a_light_not_a_plug() {
        let mut lamp = switch_as("lamp", Some("light"));
        lamp.kind = Kind::Light;
        let all = [lamp];
        assert!(category_members(&act(Category::Plugs, Target::Off), &all).is_empty());
        assert_eq!(
            ids(category_members(&act(Category::Lights, Target::Off), &all)),
            ["lamp"]
        );
    }

    #[test]
    fn groups_are_not_category_members() {
        let all = [entity("group_x", Kind::Light), entity("lamp", Kind::Light)];
        let a = act(Category::Lights, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["lamp"]);
    }

    #[test]
    fn virtual_entities_and_the_z2m_bridge_are_never_plugs() {
        let mut permit_join = switch_as(
            "bridge_0x00124b0022983b3e_permit_join_zigbee2mqtt",
            Some("plug"),
        );
        permit_join.device_id = Some("zigbee2mqtt_bridge_0x00124b0022983b3e".into());
        let all = [
            switch_as("virtual_guest_mode", Some("plug")),
            permit_join,
            switch_as("plug", Some("plug")),
        ];
        let a = act(Category::Plugs, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["plug"]);
    }

    #[test]
    fn z2m_groups_are_not_category_members() {
        let mut group = entity("1_light_zigbee2mqtt", Kind::Light);
        group.capabilities.z2m_group = true;
        let all = [group, entity("lamp", Kind::Light)];
        let a = act(Category::Lights, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["lamp"]);
    }

    #[test]
    fn covers_are_window_coverings_only() {
        let cover = |id: &str, class: Option<&str>| {
            let mut e = entity(id, Kind::Cover);
            e.device_class = class.map(Into::into);
            e
        };
        let all = [
            cover("plain", None),
            cover("shutter", Some("shutter")),
            cover("blind", Some("blind")),
            cover("curtain", Some("curtain")),
            cover("shade", Some("shade")),
            cover("awning", Some("awning")),
            cover("window", Some("window")),
            cover("garage", Some("garage")),
            cover("gate", Some("gate")),
            cover("door", Some("door")),
            cover("damper", Some("damper")),
        ];
        let a = act(Category::Covers, Target::Close);
        assert_eq!(
            ids(category_members(&a, &all)),
            [
                "plain", "shutter", "blind", "curtain", "shade", "awning", "window"
            ]
        );
    }

    #[test]
    fn unknown_show_as_is_not_a_plug() {
        let all = [
            switch_as("fan", Some("fan")),
            switch_as("plug", Some("plug")),
        ];
        let a = act(Category::Plugs, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["plug"]);
    }

    #[test]
    fn non_primary_and_folded_are_excluded_hidden_is_kept() {
        let mut diag = entity("diag", Kind::Light);
        diag.category = EntityCategory::Diagnostic;
        let mut off_by_default = entity("dis", Kind::Light);
        off_by_default.enabled_by_default = false;
        let mut folded = entity("folded", Kind::Light);
        folded.part_of = Some("other".into());
        let mut hidden = entity("hidden", Kind::Light);
        hidden.ui.hidden = true;
        let all = [diag, off_by_default, folded, hidden];
        let a = act(Category::Lights, Target::Off);
        assert_eq!(ids(category_members(&a, &all)), ["hidden"]);
    }

    #[test]
    fn area_and_except_filter() {
        let mut kitchen = entity("k", Kind::Light);
        kitchen.area = "kitchen".into();
        let all = [entity("a", Kind::Light), entity("b", Kind::Light), kitchen];
        let mut a = act(Category::Lights, Target::Off);
        a.area = Some("living".into());
        a.except = vec!["b".into()];
        assert_eq!(ids(category_members(&a, &all)), ["a"]);
    }

    #[test]
    fn brightness_pct_maps_to_255_scale_only_when_dimmable() {
        let mut dimmable = entity("dim", Kind::Light);
        dimmable.capabilities.brightness = true;
        let plain = entity("plain", Kind::Light);
        let mut a = act(Category::Lights, Target::On);
        a.brightness = Some(30);
        let plan = plan_category(&a, &[dimmable, plain]);
        let on = |brightness| Command::TurnOn {
            brightness,
            color: None,
            effect: None,
        };
        assert_eq!(
            plan.send,
            [("dim".into(), on(Some(77))), ("plain".into(), on(None))]
        );
    }

    #[test]
    fn plan_splits_offline_already_and_send() {
        let mut off = entity("off", Kind::Light);
        off.state = light_state(false, None);
        let mut on = entity("on", Kind::Light);
        on.state = light_state(true, Some(200));
        let mut gone = entity("gone", Kind::Light);
        gone.state = light_state(true, None);
        gone.state.availability = Availability::Offline;
        let plan = plan_category(&act(Category::Lights, Target::Off), &[off, on, gone]);
        assert_eq!(plan.send, [("on".to_string(), Command::TurnOff)]);
        assert_eq!(plan.already, ["off"]);
        assert_eq!(plan.offline, ["gone"]);
    }

    #[test]
    fn already_there_cases() {
        let on = |brightness| Command::TurnOn {
            brightness,
            color: None,
            effect: None,
        };
        let light = |on, b| {
            let mut e = entity("l", Kind::Light);
            e.state = light_state(on, b);
            e
        };
        let plug = |v| with_state(entity("p", Kind::Switch), StateValue::OnOff(v));
        let cover = |state: Option<&str>, position| {
            with_state(
                entity("c", Kind::Cover),
                StateValue::Cover(CoverState {
                    state: state.map(Into::into),
                    position,
                }),
            )
        };
        let climate = with_state(
            entity("t", Kind::Climate),
            StateValue::Climate(ClimateState {
                target: Some(20.0),
                mode: Some("heat".into()),
                ..Default::default()
            }),
        );

        assert!(already_there(&plug(false), &Command::TurnOff));
        assert!(!already_there(&plug(true), &Command::TurnOff));
        assert!(already_there(&light(false, None), &Command::TurnOff));
        assert!(already_there(&plug(true), &on(None)));
        assert!(already_there(&light(true, Some(10)), &on(None)));
        assert!(!already_there(&light(false, None), &on(None)));
        assert!(already_there(&light(true, Some(250)), &on(Some(255))));
        assert!(!already_there(&light(true, Some(240)), &on(Some(255))));
        assert!(!already_there(&light(false, Some(255)), &on(Some(255))));
        assert!(already_there(&cover(None, Some(0)), &Command::Close));
        assert!(already_there(&cover(Some("closed"), None), &Command::Close));
        assert!(!already_there(&cover(None, Some(50)), &Command::Close));
        assert!(already_there(&cover(None, Some(100)), &Command::Open));
        assert!(already_there(&cover(Some("open"), None), &Command::Open));
        // A known position wins over the open/closed word: half-open is neither.
        assert!(!already_there(&cover(Some("open"), Some(30)), &Command::Open));
        assert!(!already_there(&cover(Some("closed"), Some(30)), &Command::Close));
        assert!(already_there(&cover(Some("open"), Some(100)), &Command::Open));
        assert!(already_there(
            &cover(None, Some(40)),
            &Command::SetPosition { position: 40 }
        ));
        assert!(!already_there(
            &cover(None, Some(41)),
            &Command::SetPosition { position: 40 }
        ));
        let sel = with_state(entity("s", Kind::Select), StateValue::Text("night".into()));
        assert!(already_there(
            &sel,
            &Command::SelectOption {
                option: "night".into()
            }
        ));
        assert!(already_there(
            &climate,
            &Command::SetTemperature { temperature: 20.04 }
        ));
        assert!(!already_there(
            &climate,
            &Command::SetTemperature { temperature: 20.1 }
        ));
        assert!(already_there(
            &climate,
            &Command::SetHvacMode {
                mode: "heat".into()
            }
        ));
        assert!(!already_there(
            &climate,
            &Command::SetHvacMode {
                mode: "cool".into()
            }
        ));
        assert!(!already_there(&entity("u", Kind::Light), &Command::TurnOff));
        assert!(!already_there(&plug(true), &Command::Toggle));
    }
}
