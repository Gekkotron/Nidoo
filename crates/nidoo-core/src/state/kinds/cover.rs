//! Cover: `state_topic` (+ `value_template`) matched against the `state_*` words, and
//! `position_topic` (+ `position_template`) normalised to 0 (closed) ..= 100 (open).

use serde_json::Value;

use crate::model::{CoverState, Entity, EntityState, StateValue};
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
    let mut cover = match &state.value {
        StateValue::Cover(c) => c.clone(),
        _ => CoverState::default(),
    };
    let mut applied = Applied::NotForThisEntity;

    // Position first, so a "stopped" state resolves against the position of this message.
    match message.read("position_topic", &["position_template"])? {
        Read::Other => {}
        Read::Value(text) => match util::parse_number(&text).and_then(|p| normalise(config, p)) {
            Some(position) => {
                cover.position = Some(position);
                applied = Applied::Changed;
                if config.get("state_topic").is_none() {
                    cover.state = Some(open_or_closed(position).to_owned());
                }
            }
            None => applied = util::strongest(applied, Applied::Unchanged),
        },
        Read::Missing | Read::None => applied = util::strongest(applied, Applied::Unchanged),
    }

    match message.read("state_topic", &["value_template"])? {
        Read::Other => {}
        Read::Value(text) => match resolve_state(config, &text, &cover) {
            Some(word) => {
                cover.state = Some(word.to_owned());
                applied = Applied::Changed;
            }
            None => applied = util::strongest(applied, Applied::Unchanged),
        },
        Read::None => {
            cover.state = None;
            applied = Applied::Changed;
        }
        Read::Missing => applied = util::strongest(applied, Applied::Unchanged),
    }

    if applied == Applied::Changed {
        state.value = StateValue::Cover(cover);
    }
    Ok(applied)
}

/// The raw position mapped from `position_closed..=position_open` (defaults 0..=100) onto 0..=100.
fn normalise(config: &Value, raw: f64) -> Option<u8> {
    let open = util::config_f64(config, "position_open").unwrap_or(100.0);
    let closed = util::config_f64(config, "position_closed").unwrap_or(0.0);
    let range = open - closed;
    if range.abs() <= f64::EPSILON {
        return None;
    }
    util::to_u8_100((raw - closed) / range * 100.0)
}

/// The states a cover is kept in: a received `stopped` resolves to `open` or `closed`.
pub(crate) const STATES: &[&str] = &["open", "closed", "opening", "closing"];

fn open_or_closed(position: u8) -> &'static str {
    if position == 0 { "closed" } else { "open" }
}

/// The canonical state for a received word, following Home Assistant: `stopped` becomes
/// `closed` when the known position is 0 (with a position topic) or the cover was closed or
/// closing (without one), else `open`.
fn resolve_state(config: &Value, word: &str, cover: &CoverState) -> Option<&'static str> {
    let matches = |key: &str, default: &str| {
        word.eq_ignore_ascii_case(&util::config_string_or(config, key, default))
    };
    if matches("state_stopped", "stopped") {
        let closed = if config.get("position_topic").is_some() {
            cover.position == Some(0)
        } else {
            matches!(cover.state.as_deref(), Some("closed" | "closing"))
        };
        return Some(if closed { "closed" } else { "open" });
    }
    if matches("state_opening", "opening") {
        return Some("opening");
    }
    if matches("state_closing", "closing") {
        return Some("closing");
    }
    if matches("state_open", "open") {
        return Some("open");
    }
    if matches("state_closed", "closed") {
        return Some("closed");
    }
    None
}
