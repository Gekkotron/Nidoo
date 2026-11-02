//! Virtual entities: switches, numbers, buttons and selects the hub owns. Pure helpers that build
//! their retained discovery configs and initial state; the hub publishes them.
//!
//! Topics, for the first discovery prefix `P`: the config is `P/<component>/virtual/<id>/config`
//! and the state (the command topic too, so a command is echoed back retained) is
//! `P/virtual/<id>`. The `virtual` sub-tree is reserved, so the state topic is never read as
//! discovery.

use nidoo_api::{VirtualKind, VirtualSpec};
use serde_json::{Value, json};

/// The longest id.
const MAX_ID_LEN: usize = 48;
/// The most options a select takes.
const MAX_OPTIONS: usize = 32;
/// The longest option, in characters after trimming.
const MAX_OPTION_LEN: usize = 64;
const DEFAULT_MIN: f64 = 0.0;
const DEFAULT_MAX: f64 = 100.0;
const DEFAULT_STEP: f64 = 1.0;

/// `[a-z0-9_]{1,48}`.
pub fn validate_id(id: &str) -> Result<(), String> {
    let valid = (1..=MAX_ID_LEN).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid virtual id `{id}`: use 1 to {MAX_ID_LEN} characters of a-z, 0-9 and _"
        ))
    }
}

/// The entity's unique id.
pub fn unique_id(id: &str) -> String {
    format!("virtual_{id}")
}

fn component(kind: VirtualKind) -> &'static str {
    match kind {
        VirtualKind::Switch => "switch",
        VirtualKind::Number => "number",
        VirtualKind::Button => "button",
        VirtualKind::Select => "select",
    }
}

/// The retained discovery config topic.
pub fn config_topic(prefix: &str, kind: VirtualKind, id: &str) -> String {
    format!("{prefix}/{}/virtual/{id}/config", component(kind))
}

/// The state topic of a switch or a number, and the command topic of all three.
pub fn state_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/virtual/{id}")
}

/// Whether `topic` is the config topic of virtual entity `id`, whatever its kind.
pub fn is_config_topic(prefix: &str, id: &str, topic: &str) -> bool {
    [
        VirtualKind::Switch,
        VirtualKind::Number,
        VirtualKind::Button,
        VirtualKind::Select,
    ]
    .into_iter()
    .any(|kind| config_topic(prefix, kind, id) == topic)
}

fn device() -> Value {
    json!({ "identifiers": ["nidoo_virtual"], "name": "Virtual" })
}

/// The number's bounds with their defaults.
fn bounds(spec: &VirtualSpec) -> (f64, f64) {
    (
        spec.min.unwrap_or(DEFAULT_MIN),
        spec.max.unwrap_or(DEFAULT_MAX),
    )
}

/// A switch echoing on its own topic.
pub fn switch_config(prefix: &str, id: &str, spec: &VirtualSpec) -> Value {
    let topic = state_topic(prefix, id);
    json!({
        "name": spec.name,
        "unique_id": unique_id(id),
        "command_topic": topic,
        "state_topic": topic,
        "payload_on": "ON",
        "payload_off": "OFF",
        "retain": true,
        "device": device(),
    })
}

/// A number echoing on its own topic.
pub fn number_config(prefix: &str, id: &str, spec: &VirtualSpec) -> Value {
    let topic = state_topic(prefix, id);
    let (min, max) = bounds(spec);
    let mut config = json!({
        "name": spec.name,
        "unique_id": unique_id(id),
        "command_topic": topic,
        "state_topic": topic,
        "min": min,
        "max": max,
        "step": spec.step.unwrap_or(DEFAULT_STEP),
        "retain": true,
        "device": device(),
    });
    if let Some(unit) = &spec.unit {
        config["unit_of_measurement"] = json!(unit);
    }
    config
}

/// A button: a command topic and nothing else, a press is an event.
pub fn button_config(prefix: &str, id: &str, spec: &VirtualSpec) -> Value {
    json!({
        "name": spec.name,
        "unique_id": unique_id(id),
        "command_topic": state_topic(prefix, id),
        "payload_press": "PRESS",
        "device": device(),
    })
}

/// A select echoing on its own topic.
pub fn select_config(prefix: &str, id: &str, spec: &VirtualSpec) -> Value {
    let topic = state_topic(prefix, id);
    json!({
        "name": spec.name,
        "unique_id": unique_id(id),
        "command_topic": topic,
        "state_topic": topic,
        "options": spec.options.clone().unwrap_or_default(),
        "retain": true,
        "device": device(),
    })
}

/// The discovery config for `spec`.
pub fn build_config(prefix: &str, id: &str, spec: &VirtualSpec) -> Value {
    match spec.kind {
        VirtualKind::Switch => switch_config(prefix, id, spec),
        VirtualKind::Number => number_config(prefix, id, spec),
        VirtualKind::Button => button_config(prefix, id, spec),
        VirtualKind::Select => select_config(prefix, id, spec),
    }
}

/// 1 to 32 distinct options of 1 to 64 characters after trimming.
fn validate_options(options: &[String]) -> Result<(), String> {
    if !(1..=MAX_OPTIONS).contains(&options.len()) {
        return Err(format!("a select has 1 to {MAX_OPTIONS} options"));
    }
    for (i, option) in options.iter().enumerate() {
        if !(1..=MAX_OPTION_LEN).contains(&option.trim().chars().count()) {
            return Err(format!(
                "an option is 1 to {MAX_OPTION_LEN} characters, not {option:?}"
            ));
        }
        if option.trim().eq_ignore_ascii_case("none") {
            return Err(format!("{option:?} is reserved: it means an unknown state"));
        }
        if options[..i].iter().any(|seen| seen.trim() == option.trim()) {
            return Err(format!("duplicate option {option:?}"));
        }
    }
    Ok(())
}

/// Checks the spec and returns the state to publish when the entity is first created:
/// `OFF` or `initial` for a switch, the lower bound or `initial` for a number, the first
/// option or `initial` for a select, none for a button. A number's `initial` must lie within the bounds (0 and 100 unless given).
pub fn initial_state(spec: &VirtualSpec) -> Result<Option<String>, String> {
    if spec.name.trim().is_empty() {
        return Err("a virtual entity needs a name".to_owned());
    }
    if spec.kind != VirtualKind::Select && spec.options.is_some() {
        return Err("`options` only applies to a select".to_owned());
    }
    match spec.kind {
        VirtualKind::Button => Ok(None),
        VirtualKind::Select => {
            let options = spec
                .options
                .as_deref()
                .ok_or_else(|| "a select needs `options`".to_owned())?;
            validate_options(options)?;
            match &spec.initial {
                None | Some(Value::Null) => Ok(Some(options[0].clone())),
                Some(Value::String(text)) if options.contains(text) => Ok(Some(text.clone())),
                Some(other) => Err(format!(
                    "a select's `initial` is one of its options, not {other}"
                )),
            }
        }
        VirtualKind::Switch => {
            let on = match &spec.initial {
                None | Some(Value::Null) => false,
                Some(Value::Bool(on)) => *on,
                Some(Value::String(text)) if text.eq_ignore_ascii_case("on") => true,
                Some(Value::String(text)) if text.eq_ignore_ascii_case("off") => false,
                Some(other) => {
                    return Err(format!(
                        "a switch's `initial` is true, false, \"ON\" or \"OFF\", not {other}"
                    ));
                }
            };
            Ok(Some(if on { "ON" } else { "OFF" }.to_owned()))
        }
        VirtualKind::Number => {
            let (min, max) = bounds(spec);
            if !(min.is_finite() && max.is_finite() && min <= max) {
                return Err(format!("`min` ({min}) must not exceed `max` ({max})"));
            }
            if spec
                .step
                .is_some_and(|step| !(step.is_finite() && step > 0.0))
            {
                return Err("`step` must be a positive number".to_owned());
            }
            let value = match &spec.initial {
                None | Some(Value::Null) => min,
                Some(initial) => initial
                    .as_f64()
                    .ok_or_else(|| format!("a number's `initial` is a number, not {initial}"))?,
            };
            if !(min..=max).contains(&value) {
                return Err(format!("`initial` {value} is outside {min}..{max}"));
            }
            Ok(Some(value.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: VirtualKind) -> VirtualSpec {
        VirtualSpec {
            kind,
            name: "Force heating".to_owned(),
            min: None,
            max: None,
            step: None,
            unit: None,
            options: None,
            initial: None,
        }
    }

    #[test]
    fn switch_config_echoes_on_its_own_topic() {
        let config = switch_config("nidoo", "force_heating", &spec(VirtualKind::Switch));
        assert_eq!(config["command_topic"], "nidoo/virtual/force_heating");
        assert_eq!(config["state_topic"], config["command_topic"]);
        assert_eq!(config["retain"], true);
        assert_eq!(config["unique_id"], "virtual_force_heating");
        assert_eq!(config["payload_on"], "ON");
        assert_eq!(config["payload_off"], "OFF");
        assert_eq!(config["name"], "Force heating");
        assert_eq!(
            config["device"],
            json!({ "identifiers": ["nidoo_virtual"], "name": "Virtual" })
        );
    }

    #[test]
    fn number_config_has_defaults_and_unit() {
        let config = number_config("nidoo", "t", &spec(VirtualKind::Number));
        assert_eq!(config["min"], 0.0);
        assert_eq!(config["max"], 100.0);
        assert_eq!(config["step"], 1.0);
        assert_eq!(config["retain"], true);
        assert!(config.get("unit_of_measurement").is_none());

        let mut custom = spec(VirtualKind::Number);
        custom.min = Some(5.0);
        custom.max = Some(25.0);
        custom.step = Some(0.5);
        custom.unit = Some("°C".to_owned());
        let config = number_config("nidoo", "t", &custom);
        assert_eq!(config["min"], 5.0);
        assert_eq!(config["max"], 25.0);
        assert_eq!(config["step"], 0.5);
        assert_eq!(config["unit_of_measurement"], "°C");
    }

    #[test]
    fn button_config_has_a_command_topic_only() {
        let config = button_config("nidoo", "boost", &spec(VirtualKind::Button));
        assert_eq!(config["command_topic"], "nidoo/virtual/boost");
        assert_eq!(config["payload_press"], "PRESS");
        assert!(config.get("state_topic").is_none());
        assert!(config.get("retain").is_none());
    }

    #[test]
    fn topics_follow_the_component() {
        assert_eq!(
            config_topic("nidoo", VirtualKind::Number, "t"),
            "nidoo/number/virtual/t/config"
        );
        assert!(is_config_topic(
            "nidoo",
            "t",
            "nidoo/button/virtual/t/config"
        ));
        assert!(!is_config_topic(
            "nidoo",
            "t",
            "nidoo/button/other/t/config"
        ));
    }

    #[test]
    fn select_config_lists_options_and_echoes() {
        let mut s = spec(VirtualKind::Select);
        s.options = Some(vec!["home".into(), "away".into(), "night".into()]);
        let c = select_config("nidoo", "house_mode", &s);
        assert_eq!(c["command_topic"], "nidoo/virtual/house_mode");
        assert_eq!(c["state_topic"], c["command_topic"]);
        assert_eq!(c["options"], json!(["home", "away", "night"]));
        assert_eq!(c["retain"], true);
        assert_eq!(
            config_topic("nidoo", VirtualKind::Select, "house_mode"),
            "nidoo/select/virtual/house_mode/config"
        );
        assert!(is_config_topic(
            "nidoo",
            "house_mode",
            "nidoo/select/virtual/house_mode/config"
        ));
    }

    #[test]
    fn select_initial_state_rules() {
        let mut s = spec(VirtualKind::Select);
        s.options = Some(vec!["home".into(), "away".into()]);
        assert_eq!(initial_state(&s), Ok(Some("home".to_owned())));
        s.initial = Some(json!("away"));
        assert_eq!(initial_state(&s), Ok(Some("away".to_owned())));
        s.initial = Some(json!("party"));
        assert!(initial_state(&s).is_err());
    }

    #[test]
    fn select_options_are_validated() {
        let mut s = spec(VirtualKind::Select);
        for bad in [
            None,
            Some(vec![]),
            Some(vec!["a".into(), "a".into()]),
            Some(vec!["a".into(), "a ".into()]),
            Some(vec!["  ".into()]),
            Some(vec!["None".into()]),
            Some(vec!["a".into(), " none ".into()]),
            Some(vec!["x".repeat(65)]),
            Some((0..33).map(|i| i.to_string()).collect()),
        ] {
            s.options = bad.clone();
            assert!(initial_state(&s).is_err(), "{bad:?}");
        }
        let mut sw = spec(VirtualKind::Switch);
        sw.options = Some(vec!["a".into()]);
        assert!(initial_state(&sw).is_err(), "options refused on a switch");
    }

    #[test]
    fn bad_ids_are_refused() {
        for id in ["Force", "", &"x".repeat(49), "a/b", "a-b", "a b", "é"] {
            assert!(validate_id(id).is_err(), "{id:?}");
        }
        for id in ["a", "force_heating", "x1_2", &"x".repeat(48)] {
            assert!(validate_id(id).is_ok(), "{id:?}");
        }
    }

    #[test]
    fn initial_state_per_kind() {
        assert_eq!(
            initial_state(&spec(VirtualKind::Switch)),
            Ok(Some("OFF".into()))
        );
        let mut on = spec(VirtualKind::Switch);
        on.initial = Some(json!(true));
        assert_eq!(initial_state(&on), Ok(Some("ON".into())));
        on.initial = Some(json!("off"));
        assert_eq!(initial_state(&on), Ok(Some("OFF".into())));
        on.initial = Some(json!(3));
        assert!(initial_state(&on).is_err());

        assert_eq!(initial_state(&spec(VirtualKind::Button)), Ok(None));

        let mut number = spec(VirtualKind::Number);
        number.min = Some(10.0);
        assert_eq!(initial_state(&number), Ok(Some("10".into())));
        number.initial = Some(json!(12.5));
        assert_eq!(initial_state(&number), Ok(Some("12.5".into())));
        number.initial = Some(json!(101));
        assert!(initial_state(&number).is_err());
        number.initial = Some(json!("x"));
        assert!(initial_state(&number).is_err());
        number.initial = None;
        number.max = Some(5.0);
        assert!(initial_state(&number).is_err());
    }
}
