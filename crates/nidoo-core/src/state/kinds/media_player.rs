//! Media player: `state_topic` carries the whole state as JSON on every change (the media
//! player contract). A message replaces the state: a missing field is null, and an idle player
//! has nothing but its `state`.

use serde_json::{Map, Value};

use super::set_value;
use crate::model::{Entity, EntityState, MediaPlayerState, StateValue};
use crate::state::util::{self, Message};
use crate::state::{Applied, StateError};
use crate::template::Templates;

/// The states a media player reports.
pub(crate) const STATES: &[&str] = &["playing", "paused", "idle"];

pub(super) fn apply(
    entity: &Entity,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    templates: &Templates,
) -> Result<Applied, StateError> {
    let message = Message {
        templates,
        config: &entity.bindings.config,
        topic,
        payload,
    };
    if !message.is_topic("state_topic") {
        return Ok(Applied::NotForThisEntity);
    }
    let Some(object) = util::parse_object(payload) else {
        return Ok(Applied::Unchanged);
    };
    let Some(word) = object
        .get("state")
        .and_then(Value::as_str)
        .and_then(|word| STATES.iter().find(|s| word.eq_ignore_ascii_case(s)))
    else {
        return Ok(Applied::Unchanged);
    };
    let player = if *word == "idle" {
        MediaPlayerState {
            state: Some((*word).to_owned()),
            ..MediaPlayerState::default()
        }
    } else {
        read(&object, word)
    };
    Ok(set_value(state, StateValue::MediaPlayer(player)))
}

fn read(object: &Map<String, Value>, word: &str) -> MediaPlayerState {
    let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
    let seconds = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_f64)
            .filter(|s| s.is_finite() && *s >= 0.0)
    };
    MediaPlayerState {
        state: Some(word.to_owned()),
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        image_url: text("image_url").filter(|url| {
            let lower = url.to_ascii_lowercase();
            lower.starts_with("https://") || lower.starts_with("http://")
        }),
        color: text("color").and_then(|c| hex_color(&c)),
        duration_s: seconds("duration_s"),
        position_s: seconds("position_s"),
        position_at_ms: text("position_at")
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
            .map(|at| at.timestamp_millis()),
    }
}

/// `#rrggbb` in lowercase, or `None` for anything else (the web puts it in a style).
pub fn hex_color(text: &str) -> Option<String> {
    let digits = text.strip_prefix('#')?;
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", digits.to_ascii_lowercase()))
}
