//! Reading an entity's discovery config: strings, numbers and the light's brightness scale,
//! shared by the state and command mappers.

use serde_json::Value;

/// A config entry as text: strings as they are, booleans and numbers as JSON writes them
/// (Zigbee2MQTT publishes `payload_on: false` as a boolean).
pub(crate) fn config_string(config: &Value, key: &str) -> Option<String> {
    match config.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// [`config_string`] with a default.
pub(crate) fn config_string_or(config: &Value, key: &str, default: &str) -> String {
    config_string(config, key).unwrap_or_else(|| default.to_owned())
}

/// A numeric config entry (a number, or a numeric string).
pub(crate) fn config_f64(config: &Value, key: &str) -> Option<f64> {
    match config.get(key)? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => parse_number(s),
        _ => None,
    }
}

/// A finite number, trimmed.
pub(crate) fn parse_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|f| f.is_finite())
}

/// A light's `brightness_scale` (default 255), guarded against zero and negatives.
pub(crate) fn brightness_scale(config: &Value) -> f64 {
    config_f64(config, "brightness_scale")
        .filter(|scale| *scale > 0.0)
        .unwrap_or(255.0)
}
