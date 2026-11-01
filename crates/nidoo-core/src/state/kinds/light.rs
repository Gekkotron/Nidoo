//! Light: the `json` schema reads one JSON object on `state_topic`; the default schema reads
//! one topic (+ `*_value_template`) per attribute. `brightness` is stored scaled to 0..=255.

use serde_json::{Map, Value};

use crate::config::brightness_scale;
use crate::model::{Entity, EntityState, Hs, LightState, StateValue};
use crate::state::color::{rgb_to_hs, xy_to_hs};
use crate::state::util::{self, Message, Read};
use crate::state::{Applied, StateError};
use crate::template::Templates;

pub(super) fn apply(
    entity: &Entity,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    templates: &Templates,
) -> Result<Applied, StateError> {
    let config = &entity.bindings.config;
    let message = Message {
        templates,
        config,
        topic,
        payload,
    };
    let light = match &state.value {
        StateValue::Light(l) => l.clone(),
        _ => LightState::default(),
    };
    let schema = config
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let (light, applied) = if schema == "json" {
        apply_json(&message, light)
    } else {
        apply_default(&message, light)?
    };
    if applied == Applied::Changed {
        state.value = StateValue::Light(light);
    }
    Ok(applied)
}

fn scale_brightness(raw: f64, scale: f64) -> Option<u8> {
    util::to_u8_255(raw * 255.0 / scale)
}

fn hs(h: f64, s: f64) -> Option<Hs> {
    (h.is_finite() && s.is_finite()).then(|| Hs {
        h: h.rem_euclid(360.0),
        s: s.clamp(0.0, 100.0),
    })
}

/// Mireds as published, rounded; negative or non-finite values are rejected.
fn color_temp(raw: f64) -> Option<u32> {
    (raw.is_finite() && raw >= 0.0).then(|| raw.round().min(f64::from(u32::MAX)) as u32)
}

fn apply_json(message: &Message<'_>, mut light: LightState) -> (LightState, Applied) {
    if !message.is_topic("state_topic") {
        return (light, Applied::NotForThisEntity);
    }
    let Some(object) = util::parse_object(message.payload) else {
        return (light, Applied::Unchanged);
    };
    let mut changed = false;
    if let Some(on) = object
        .get("state")
        .and_then(Value::as_str)
        .and_then(|s| util::on_off(s, "ON", "OFF"))
    {
        light.on = on;
        changed = true;
    }
    if let Some(brightness) = object
        .get("brightness")
        .and_then(Value::as_f64)
        .and_then(|b| scale_brightness(b, brightness_scale(message.config)))
    {
        light.brightness = Some(brightness);
        changed = true;
    }
    let mode = object.get("color_mode").and_then(Value::as_str);
    if let Some(color) = object.get("color").and_then(Value::as_object)
        && let Some(hs) = color_from_json(color, mode)
    {
        light.hs = Some(hs);
        changed = true;
    }
    if let Some(mireds) = object
        .get("color_temp")
        .and_then(Value::as_f64)
        .and_then(color_temp)
    {
        light.color_temp = Some(mireds);
        changed = true;
    }
    // Colour modes are exclusive: the inactive attribute is cleared, as Home Assistant does.
    match mode {
        Some("color_temp") => light.hs = None,
        Some("hs" | "xy" | "rgb" | "rgbw" | "rgbww") => light.color_temp = None,
        _ => {}
    }
    if let Some(effect) = object
        .get("effect")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
    {
        light.effect = Some(effect.to_owned());
        changed = true;
    }
    let applied = if changed {
        Applied::Changed
    } else {
        Applied::Unchanged
    };
    (light, applied)
}

/// The `color` object as hue/saturation, by `color_mode` when given, else whichever of
/// `h,s` / `x,y` / `r,g,b` it carries.
fn color_from_json(color: &Map<String, Value>, mode: Option<&str>) -> Option<Hs> {
    let number = |key: &str| color.get(key).and_then(Value::as_f64);
    let from_hs = || hs(number("h")?, number("s")?);
    let from_xy = || Some(xy_to_hs(number("x")?, number("y")?));
    let from_rgb = || {
        let channel = |key: &str| number(key).and_then(util::to_u8_255);
        Some(rgb_to_hs(channel("r")?, channel("g")?, channel("b")?))
    };
    match mode {
        Some("hs") => from_hs(),
        Some("xy") => from_xy(),
        Some("rgb" | "rgbw" | "rgbww") => from_rgb(),
        Some(_) => None,
        None => from_hs().or_else(from_xy).or_else(from_rgb),
    }
}

/// Exactly `N` comma-separated numbers (`"255,0,0"`, `"0.3,0.4"`).
fn numbers<const N: usize>(text: &str) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    let mut parts = text.split(',');
    for slot in &mut out {
        *slot = util::parse_number(parts.next()?)?;
    }
    parts.next().is_none().then_some(out)
}

/// The default-schema attributes, each on its own topic and template.
const FIELDS: &[(&str, &str)] = &[
    ("state_topic", "state_value_template"),
    ("brightness_state_topic", "brightness_value_template"),
    ("hs_state_topic", "hs_value_template"),
    ("rgb_state_topic", "rgb_value_template"),
    ("xy_state_topic", "xy_value_template"),
    ("color_temp_state_topic", "color_temp_value_template"),
    ("effect_state_topic", "effect_value_template"),
];

fn apply_default(
    message: &Message<'_>,
    mut light: LightState,
) -> Result<(LightState, Applied), StateError> {
    let config = message.config;
    let mut applied = Applied::NotForThisEntity;
    let on_word = util::config_string_or(config, "payload_on", "ON");
    let off_word = util::config_string_or(config, "payload_off", "OFF");
    let scale = brightness_scale(config);
    for (topic_key, template_key) in FIELDS {
        let text = match message.read(topic_key, &[template_key])? {
            Read::Other => continue,
            Read::Value(text) => text,
            Read::Missing | Read::None => {
                applied = util::strongest(applied, Applied::Unchanged);
                continue;
            }
        };
        let used = match *topic_key {
            "state_topic" => util::on_off(&text, &on_word, &off_word)
                .map(|on| light.on = on)
                .is_some(),
            "brightness_state_topic" => util::parse_number(&text)
                .and_then(|b| scale_brightness(b, scale))
                .map(|b| light.brightness = Some(b))
                .is_some(),
            "hs_state_topic" => numbers::<2>(&text)
                .and_then(|[h, s]| hs(h, s))
                .map(|color| light.hs = Some(color))
                .is_some(),
            "rgb_state_topic" => numbers::<3>(&text)
                .and_then(|[r, g, b]| {
                    Some(rgb_to_hs(
                        util::to_u8_255(r)?,
                        util::to_u8_255(g)?,
                        util::to_u8_255(b)?,
                    ))
                })
                .map(|color| light.hs = Some(color))
                .is_some(),
            "xy_state_topic" => numbers::<2>(&text)
                .map(|[x, y]| light.hs = Some(xy_to_hs(x, y)))
                .is_some(),
            "color_temp_state_topic" => util::parse_number(&text)
                .and_then(color_temp)
                .map(|mireds| light.color_temp = Some(mireds))
                .is_some(),
            "effect_state_topic" => {
                light.effect = Some(text);
                true
            }
            _ => false,
        };
        applied = if used {
            Applied::Changed
        } else {
            util::strongest(applied, Applied::Unchanged)
        };
    }
    Ok((light, applied))
}
