//! One mapper per entity kind, selected by `protocol_kind`, with Home Assistant's payload
//! words and templates.

use serde_json::{Map, Value, json};

use super::{CommandError, Mapper, Publish, in_range, listed, listed_if_any};
use crate::config;
use crate::model::{AlarmMode, Command, Hs, Kind, LightState, StateValue};

/// Maps the command through the mapper of the entity's `protocol_kind`.
pub(super) fn map(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    match m.entity.protocol_kind {
        Kind::Light => light(m),
        Kind::Switch => switch(m),
        Kind::Cover => cover(m),
        Kind::Climate => thermostat(m, true),
        Kind::WaterHeater => thermostat(m, false),
        Kind::Number => number(m),
        Kind::Select => select(m),
        Kind::Button => button(m),
        Kind::AlarmPanel => alarm(m),
        Kind::Vacuum => vacuum(m),
        Kind::Sensor | Kind::BinarySensor => Err(m.unsupported()),
        Kind::Generic => match crate::types::type_def(&m.entity.component) {
            Some(def) => crate::types::command::map(m, def),
            None => Err(m.unsupported()),
        },
    }
}

/// `TurnOn` with nothing else: what `Toggle` sends to an entity that is off or unknown.
const PLAIN_ON: Command = Command::TurnOn {
    brightness: None,
    color: None,
    effect: None,
};

/// `Toggle` resolved against the current value and `TurnOn` at brightness 0 turned into
/// `TurnOff` (as Home Assistant does before the MQTT layer); any other command as it is.
fn resolve_on_off<'a>(m: &Mapper<'a>) -> &'a Command {
    match m.command {
        Command::Toggle => {
            let on = matches!(
                m.state.value,
                StateValue::OnOff(true) | StateValue::Light(LightState { on: true, .. })
            );
            if on { &Command::TurnOff } else { &PLAIN_ON }
        }
        Command::TurnOn {
            brightness: Some(0),
            ..
        } => &Command::TurnOff,
        other => other,
    }
}

/// An inverted switch (see `UiOverride::invert`) sends the off word to turn on.
fn switch(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let on = match resolve_on_off(m) {
        Command::TurnOn { .. } => true,
        Command::TurnOff => false,
        _ => return Err(m.unsupported()),
    };
    let word = if on != m.entity.ui.invert {
        m.word("payload_on", "ON")
    } else {
        m.word("payload_off", "OFF")
    };
    Ok(vec![m.publish(
        "command_topic",
        Some("command_template"),
        &word,
        &[],
    )?])
}

/// The hue normalised into 0..<360; the saturation must be a percentage.
fn checked_hs(color: Hs) -> Result<Hs, CommandError> {
    if !(color.h.is_finite() && color.s.is_finite()) || !(0.0..=100.0).contains(&color.s) {
        return Err(CommandError::OutOfRange(format!(
            "color h={} s={} is not a hue and a saturation in 0..=100",
            color.h, color.s
        )));
    }
    Ok(Hs {
        h: color.h.rem_euclid(360.0),
        s: color.s,
    })
}

/// The 0..=255 brightness on the device's `brightness_scale`, never below 1 (0 would be off).
fn device_brightness(brightness: u8, scale: f64) -> u64 {
    (f64::from(brightness) * scale / 255.0)
        .round()
        .clamp(1.0, scale.max(1.0)) as u64
}

fn light(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let json_schema = m.config.get("schema").and_then(Value::as_str) == Some("json");
    let (brightness, color, effect) = match resolve_on_off(m) {
        Command::TurnOn {
            brightness,
            color,
            effect,
        } => (*brightness, *color, effect.as_deref()),
        Command::TurnOff => {
            let publish = if json_schema {
                m.publish(
                    "command_topic",
                    None,
                    &json!({"state": "OFF"}).to_string(),
                    &[],
                )?
            } else {
                let off = m.word("payload_off", "OFF");
                m.publish("command_topic", None, &off, &[])?
            };
            return Ok(vec![publish]);
        }
        _ => return Err(m.unsupported()),
    };
    if let Some(effect) = effect {
        listed(effect, &m.entity.capabilities.effects, "effect")?;
    }
    let color = color.map(checked_hs).transpose()?;
    let scale = config::brightness_scale(m.config);
    let brightness = brightness.map(|b| device_brightness(b, scale));
    if json_schema {
        return light_json(m, brightness, color, effect);
    }
    light_default(m, brightness, color, effect, scale)
}

/// JSON schema: one object on `command_topic`.
fn light_json(
    m: &Mapper<'_>,
    brightness: Option<u64>,
    color: Option<Hs>,
    effect: Option<&str>,
) -> Result<Vec<Publish>, CommandError> {
    let mut object = Map::new();
    object.insert("state".to_owned(), Value::from("ON"));
    if let Some(brightness) = brightness {
        object.insert("brightness".to_owned(), Value::from(brightness));
    }
    if let Some(hs) = color {
        object.insert("color".to_owned(), json!({"h": hs.h, "s": hs.s}));
    }
    if let Some(effect) = effect {
        object.insert("effect".to_owned(), Value::from(effect));
    }
    let payload = Value::Object(object).to_string();
    Ok(vec![m.publish("command_topic", None, &payload, &[])?])
}

/// Default schema: one topic per attribute, with the ON word placed by `on_command_type`
/// (`last` by default, `first`, or `brightness` which stands in for ON).
fn light_default(
    m: &Mapper<'_>,
    brightness: Option<u64>,
    color: Option<Hs>,
    effect: Option<&str>,
    scale: f64,
) -> Result<Vec<Publish>, CommandError> {
    let on_type = m.word("on_command_type", "last");
    let on = |m: &Mapper<'_>| {
        let word = m.word("payload_on", "ON");
        m.publish("command_topic", None, &word, &[])
    };
    let mut out = Vec::new();
    let brightness = match (on_type.as_str(), brightness) {
        ("first", b) => {
            out.push(on(m)?);
            b
        }
        // ON is implied by a brightness: the given one, else the current one, else full.
        ("brightness", None) => {
            let current = match &m.state.value {
                StateValue::Light(light) => light.brightness.filter(|b| *b > 0),
                _ => None,
            };
            Some(device_brightness(current.unwrap_or(255), scale))
        }
        (_, b) => b,
    };
    if let Some(hs) = color {
        // `hs_command_template` sees `hue` and `sat` besides `value`, as Home Assistant documents.
        let payload = format!("{},{}", hs.h, hs.s);
        let extra = [("hue", json!(hs.h)), ("sat", json!(hs.s))];
        out.push(m.publish(
            "hs_command_topic",
            Some("hs_command_template"),
            &payload,
            &extra,
        )?);
    }
    if let Some(brightness) = brightness {
        out.push(m.publish(
            "brightness_command_topic",
            Some("brightness_command_template"),
            &brightness.to_string(),
            &[],
        )?);
    }
    if let Some(effect) = effect {
        out.push(m.publish(
            "effect_command_topic",
            Some("effect_command_template"),
            effect,
            &[],
        )?);
    }
    if !matches!(on_type.as_str(), "first" | "brightness") {
        out.push(on(m)?);
    }
    Ok(out)
}

fn cover(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let (key, default) = match m.command {
        Command::Open => ("payload_open", "OPEN"),
        Command::Close => ("payload_close", "CLOSE"),
        Command::Stop => ("payload_stop", "STOP"),
        Command::SetPosition { position } => return cover_position(m, *position),
        _ => return Err(m.unsupported()),
    };
    let word = m.word(key, default);
    Ok(vec![m.publish("command_topic", None, &word, &[])?])
}

/// The 0 (closed) ..= 100 (open) position mapped onto `position_closed..=position_open`
/// (defaults 0..=100), through `set_position_template` when present. As in Home Assistant,
/// the template sees the device value as `value` and the percentage as `position`.
fn cover_position(m: &Mapper<'_>, position: u8) -> Result<Vec<Publish>, CommandError> {
    in_range(f64::from(position), Some(0.0), Some(100.0), "position")?;
    let open = config::config_f64(m.config, "position_open").unwrap_or(100.0);
    let closed = config::config_f64(m.config, "position_closed").unwrap_or(0.0);
    let scaled = (closed + f64::from(position) / 100.0 * (open - closed)).round() as i64;
    let extra = [
        ("position", json!(position)),
        ("position_open", json!(open)),
        ("position_closed", json!(closed)),
    ];
    Ok(vec![m.publish(
        "set_position_topic",
        Some("set_position_template"),
        &scaled.to_string(),
        &extra,
    )?])
}

/// Climate (with a fan) and water heater: target temperature and modes, one topic each.
fn thermostat(m: &Mapper<'_>, with_fan: bool) -> Result<Vec<Publish>, CommandError> {
    let caps = &m.entity.capabilities;
    let publish = match m.command {
        Command::SetTemperature { temperature } => {
            in_range(*temperature, caps.min_temp, caps.max_temp, "temperature")?;
            m.publish(
                "temperature_command_topic",
                Some("temperature_command_template"),
                &temperature.to_string(),
                &[],
            )?
        }
        Command::SetHvacMode { mode } => {
            listed_if_any(mode, &caps.hvac_modes, "mode")?;
            m.publish(
                "mode_command_topic",
                Some("mode_command_template"),
                mode,
                &[],
            )?
        }
        Command::SetFanMode { mode } if with_fan => {
            listed_if_any(mode, &caps.fan_modes, "fan mode")?;
            m.publish(
                "fan_mode_command_topic",
                Some("fan_mode_command_template"),
                mode,
                &[],
            )?
        }
        _ => return Err(m.unsupported()),
    };
    Ok(vec![publish])
}

fn number(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let Command::SetNumber { value } = m.command else {
        return Err(m.unsupported());
    };
    let caps = &m.entity.capabilities;
    in_range(*value, caps.min, caps.max, "value")?;
    Ok(vec![m.publish(
        "command_topic",
        Some("command_template"),
        &value.to_string(),
        &[],
    )?])
}

fn select(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let Command::SelectOption { option } = m.command else {
        return Err(m.unsupported());
    };
    listed(option, &m.entity.capabilities.options, "option")?;
    Ok(vec![m.publish(
        "command_topic",
        Some("command_template"),
        option,
        &[],
    )?])
}

fn button(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    if *m.command != Command::Press {
        return Err(m.unsupported());
    }
    let word = m.word("payload_press", "PRESS");
    Ok(vec![m.publish(
        "command_topic",
        Some("command_template"),
        &word,
        &[],
    )?])
}

/// The config key and default word for an arming mode.
fn arm_word(mode: AlarmMode) -> (&'static str, &'static str) {
    match mode {
        AlarmMode::Home => ("payload_arm_home", "ARM_HOME"),
        AlarmMode::Away => ("payload_arm_away", "ARM_AWAY"),
        AlarmMode::Night => ("payload_arm_night", "ARM_NIGHT"),
        AlarmMode::Vacation => ("payload_arm_vacation", "ARM_VACATION"),
        AlarmMode::CustomBypass => ("payload_arm_custom_bypass", "ARM_CUSTOM_BYPASS"),
    }
}

/// The action word on `command_topic`, through `command_template` (context `action`, `code`)
/// when present. When the panel requires a code (`code_arm_required` /
/// `code_disarm_required`), one must be given, and it must match a literal configured `code`
/// (`REMOTE_CODE` / `REMOTE_CODE_TEXT` leave the check to the device), as Home Assistant's
/// `_validate_code` does.
fn alarm(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let caps = &m.entity.capabilities;
    let ((key, default), code, code_required) = match m.command {
        Command::Arm { mode, code } => {
            if !caps.alarm_modes.is_empty() && !caps.alarm_modes.contains(mode) {
                return Err(CommandError::OutOfRange(format!(
                    "mode {mode:?} is not one of {:?}",
                    caps.alarm_modes
                )));
            }
            (arm_word(*mode), code, caps.code_arm_required)
        }
        Command::Disarm { code } => (
            ("payload_disarm", "DISARM"),
            code,
            caps.code_disarm_required,
        ),
        _ => return Err(m.unsupported()),
    };
    if code_required {
        let Some(code) = code.as_deref() else {
            return Err(CommandError::MissingCode);
        };
        match m.config.get("code").and_then(Value::as_str) {
            None | Some("REMOTE_CODE" | "REMOTE_CODE_TEXT") => {}
            Some(configured) if configured == code => {}
            Some(_) => return Err(CommandError::WrongCode),
        }
    }
    let action = m.word(key, default);
    let extra = [
        ("action", Value::from(action.as_str())),
        ("code", Value::from(code.as_deref().unwrap_or(""))),
    ];
    Ok(vec![m.publish(
        "command_topic",
        Some("command_template"),
        &action,
        &extra,
    )?])
}

/// The config key and default word for a vacuum action.
fn vacuum_word(action: &str) -> Option<(&'static str, &'static str)> {
    Some(match action {
        "start" => ("payload_start", "start"),
        "pause" => ("payload_pause", "pause"),
        "stop" => ("payload_stop", "stop"),
        "return_to_base" => ("payload_return_to_base", "return_to_base"),
        "locate" => ("payload_locate", "locate"),
        _ => return None,
    })
}

fn vacuum(m: &Mapper<'_>) -> Result<Vec<Publish>, CommandError> {
    let caps = &m.entity.capabilities;
    let publish = match m.command {
        Command::VacuumAction { action } => {
            let Some((key, default)) = vacuum_word(action) else {
                return Err(CommandError::OutOfRange(format!(
                    "action {action:?} is not a vacuum action"
                )));
            };
            listed_if_any(action, &caps.vacuum_actions, "action")?;
            let word = m.word(key, default);
            m.publish("command_topic", None, &word, &[])?
        }
        Command::SetFanSpeed { speed } => {
            listed_if_any(speed, &caps.fan_speeds, "fan speed")?;
            m.publish("set_fan_speed_topic", None, speed, &[])?
        }
        _ => return Err(m.unsupported()),
    };
    Ok(vec![publish])
}
