//! Frigate event bridge: turns `<frigate>/events` messages into v1 push alerts.
//!
//! A pure translator: it holds the per-event and per-camera memory, takes the clock as an
//! argument and returns the v1 JSON; the caller hands it to the push service.
//!
//! - the first time an event qualifies (usually `new`): an audible alert with the snapshot;
//! - `end` with a clip: the same alert id again with `video_url`, sent silently, so the app
//!   replaces the notification instead of ringing again.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap, VecDeque};

use serde_json::{Map, Value, json};

/// How many announced event ids the bridge remembers.
const SEEN_CAP: usize = 500;

/// What the bridge needs to know; an empty filter set means "all" (labels, cameras) or "any"
/// (zones).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrigateSettings {
    /// Base URL the phone uses to fetch media, without a trailing slash.
    pub media_url: String,
    pub labels: BTreeSet<String>,
    pub cameras: BTreeSet<String>,
    pub zones: BTreeSet<String>,
    /// Seconds to ignore new events on a camera after an alert; `0` is off.
    pub cooldown: u64,
    pub severity: String,
    pub ttl: u32,
}

impl FrigateSettings {
    /// The Python defaults around `media_url`.
    pub fn new(media_url: impl Into<String>) -> Self {
        Self {
            media_url: media_url.into().trim_end_matches('/').to_owned(),
            labels: BTreeSet::new(),
            cameras: BTreeSet::new(),
            zones: BTreeSet::new(),
            cooldown: 0,
            severity: "warning".to_owned(),
            ttl: 43200,
        }
    }
}

#[derive(Debug)]
pub struct FrigateBridge {
    settings: FrigateSettings,
    /// Events whose audible alert went out, oldest first.
    sent: VecDeque<String>,
    last_by_camera: HashMap<String, u64>,
}

/// `front_door` -> `Front Door`, like Python's `str.title()` after `_` and `-` become spaces.
pub fn pretty(name: &str) -> String {
    let spaced = name.replace(['_', '-'], " ");
    let mut out = String::new();
    let mut in_word = false;
    for c in spaced.trim().chars() {
        if c.is_alphabetic() {
            if in_word {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            in_word = true;
        } else {
            out.push(c);
            in_word = false;
        }
    }
    out
}

/// A non-empty string field.
fn text<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// Frigate sends either `"Name"` or `["Name", score]`.
fn sub_label(event: &Value) -> Option<String> {
    let raw = match event.get("sub_label")? {
        Value::Array(items) => items.first()?,
        other => other,
    };
    let trimmed = raw.as_str()?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn zones(event: &Value) -> Vec<&str> {
    ["entered_zones", "current_zones"]
        .iter()
        .filter_map(|key| event.get(key)?.as_array())
        .find(|list| !list.is_empty())
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn flag(event: &Value, key: &str) -> bool {
    event.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The event id as text; a missing, empty or zero id is no id.
fn event_id(event: &Value) -> Option<String> {
    match event.get("id")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) if n.as_f64() != Some(0.0) => Some(n.to_string()),
        _ => None,
    }
}

impl FrigateBridge {
    pub fn new(settings: FrigateSettings) -> Self {
        Self {
            settings,
            sent: VecDeque::new(),
            last_by_camera: HashMap::new(),
        }
    }

    /// One `<frigate>/events` payload at `now` (Unix seconds): the v1 alert to raise, if any.
    pub fn on_event(&mut self, payload: &[u8], now: u64) -> Option<Value> {
        let message: Value = serde_json::from_slice(payload).ok()?;
        self.handle(&message, now)
    }

    fn qualifies(&self, event: &Value) -> bool {
        let s = &self.settings;
        if flag(event, "false_positive") {
            return false;
        }
        let listed = |set: &BTreeSet<String>, key: &str| {
            set.is_empty() || text(event, key).is_some_and(|v| set.contains(v))
        };
        if !listed(&s.labels, "label") || !listed(&s.cameras, "camera") {
            return false;
        }
        s.zones.is_empty() || zones(event).iter().any(|z| s.zones.contains(*z))
    }

    fn alert(&self, event: &Value, id: &str, with_clip: bool, silent: bool) -> Value {
        let s = &self.settings;
        let base = format!("{}/api/events/{id}", s.media_url);
        let label = pretty(text(event, "label").unwrap_or("object"));
        let who = sub_label(event);
        let shown: Vec<&str> = zones(event)
            .into_iter()
            .filter(|z| s.zones.is_empty() || s.zones.contains(*z))
            .collect();
        let mut body = match &who {
            Some(who) => format!("{who} ({})", label.to_lowercase()),
            None => label,
        };
        match shown.first() {
            Some(zone) => body.push_str(&format!(" in {}", pretty(zone))),
            None => body.push_str(" detected"),
        }
        let mut payload = Map::new();
        payload.insert("v".into(), json!(1));
        payload.insert("id".into(), json!(format!("frigate-{id}")));
        payload.insert("channel".into(), json!("camera"));
        payload.insert("severity".into(), json!(s.severity));
        payload.insert(
            "title".into(),
            json!(pretty(text(event, "camera").unwrap_or("Camera"))),
        );
        payload.insert("body".into(), json!(body));
        if let Some(camera) = text(event, "camera") {
            payload.insert("camera".into(), json!(camera));
        }
        payload.insert("image_url".into(), json!(format!("{base}/snapshot.jpg")));
        payload.insert("silent".into(), json!(silent));
        payload.insert("ttl".into(), json!(s.ttl));
        if let Some(raw) = text(event, "label")
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            let mut args = Map::new();
            args.insert("label".into(), json!(raw));
            if let Some(zone) = shown.first() {
                args.insert("zone".into(), json!(pretty(zone)));
            }
            if let Some(who) = &who {
                args.insert("who".into(), json!(who));
            }
            // The app writes the sentence in the phone's language; title/body stay the English
            // fallback.
            payload.insert("loc".into(), json!({"key": "frigate.object", "args": args}));
        }
        if with_clip {
            payload.insert("video_url".into(), json!(format!("{base}/clip.mp4")));
        }
        Value::Object(payload)
    }

    fn remember(&mut self, id: &str) {
        self.sent.retain(|seen| seen != id);
        self.sent.push_back(id.to_owned());
        while self.sent.len() > SEEN_CAP {
            self.sent.pop_front();
        }
    }

    fn cooling_down(&self, camera: &str, now: u64) -> bool {
        self.settings.cooldown > 0
            && self
                .last_by_camera
                .get(camera)
                .is_some_and(|last| now.saturating_sub(*last) < self.settings.cooldown)
    }

    fn handle(&mut self, message: &Value, now: u64) -> Option<Value> {
        let kind = message.get("type")?.as_str()?;
        let event = message.get("after").filter(|e| e.is_object())?;
        if !matches!(kind, "new" | "update" | "end") {
            return None;
        }
        let id = event_id(event)?;
        let ended = kind == "end";
        let has_clip = flag(event, "has_clip");

        if self.sent.contains(&id) {
            if !ended || !has_clip {
                return None;
            }
            self.sent.retain(|seen| *seen != id);
            return Some(self.alert(event, &id, true, true));
        }

        if !self.qualifies(event) {
            return None;
        }
        let camera = text(event, "camera").unwrap_or("").to_owned();
        if self.cooling_down(&camera, now) {
            return None;
        }
        self.last_by_camera.insert(camera, now);
        if ended {
            // Qualified only at the end (late zone entry, short event): one audible alert,
            // clip included.
            return Some(self.alert(event, &id, has_clip, false));
        }
        self.remember(&id);
        Some(self.alert(event, &id, false, false))
    }
}

#[cfg(test)]
mod tests;
