//! Pure normalisation of producer payloads (v1 and legacy Mqtt2FCMPush format).
//!
//! No MQTT, no FCM, no I/O: [`normalize`] turns an already-parsed JSON value into an
//! [`Alert`] (or a [`Rejected`] reason) and [`to_fcm_data`] turns an [`Alert`] into the
//! string-only FCM data map.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

pub const CHANNELS: [&str; 7] = [
    "alarm", "security", "camera", "energy", "climate", "system", "misc",
];
pub const SEVERITIES: [&str; 3] = ["info", "warning", "critical"];
pub const DEFAULT_TTL: u32 = 43200;
pub const CANCEL_TTL: u32 = 300;
pub const MAX_TTL: u32 = 2_419_200;
pub const FCM_MAX_BYTES: usize = 4096;
pub const ELLIPSIS: &str = "…";
pub const LOC_MAX_ARGS: usize = 8;
pub const LOC_MAX_VALUE: usize = 200;

const LEGACY_CHANNELS: [(&str, &str); 5] = [
    ("alarme", "alarm"),
    ("nvr", "camera"),
    ("pv", "energy"),
    ("surveillance", "climate"),
    ("Supervisor", "system"),
];
const HIGH_PRIORITY_CHANNELS: [&str; 3] = ["alarm", "security", "camera"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Action {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub id: String,
    pub channel: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub image_url: Option<String>,
    pub video_url: Option<String>,
    pub entity_id: Option<String>,
    pub actions: Vec<Action>,
    pub silent: bool,
    pub cancel: bool,
    pub ttl: u32,
    pub sent_at: u64,
    pub source: String,
    /// Localized-sentence hint `{"key", "args"}`; `None` when absent or invalid.
    pub loc: Option<Value>,
    /// The raw Frigate camera id (`front_door`) of a camera alert; v1 payloads only.
    pub camera: Option<String>,
}

impl Default for Alert {
    fn default() -> Self {
        Self {
            id: String::new(),
            channel: "misc".into(),
            severity: "info".into(),
            title: String::new(),
            body: String::new(),
            image_url: None,
            video_url: None,
            entity_id: None,
            actions: Vec::new(),
            silent: false,
            cancel: false,
            ttl: DEFAULT_TTL,
            sent_at: 0,
            source: String::new(),
            loc: None,
            camera: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    High,
    Normal,
}

/// `High` for warning/critical or the alarm/security/camera channels.
pub fn android_priority(alert: &Alert) -> Priority {
    if matches!(alert.severity.as_str(), "warning" | "critical")
        || HIGH_PRIORITY_CHANNELS.contains(&alert.channel.as_str())
    {
        Priority::High
    } else {
        Priority::Normal
    }
}

pub fn ttl_seconds(alert: &Alert) -> u32 {
    if alert.cancel { CANCEL_TTL } else { alert.ttl }
}

// ---------- small coercion helpers ----------

/// String or number -> string; anything else (null, bool, array, object) -> "".
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Id as a trimmed string. 42 and 42.0 both give "42".
fn id_text(value: Option<&Value>) -> String {
    if let Some(Value::Number(n)) = value
        && let Some(f) = n.as_f64()
        && n.is_f64()
        && f.is_finite()
        && f.fract() == 0.0
        && f.abs() < 9.0e18
    {
        return (f as i64).to_string();
    }
    text(value).trim().to_string()
}

fn opt(value: Option<&Value>) -> Option<String> {
    let t = text(value);
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn is_loc_key(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

fn is_loc_arg(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A valid `{"key", "args"}` hint, else `None`: a bad hint never costs the alert.
fn loc(raw: Option<&Value>) -> Option<Value> {
    let obj = raw?.as_object()?;
    let key = obj.get("key")?.as_str()?;
    if !is_loc_key(key) {
        return None;
    }
    let empty = Map::new();
    let args = match obj.get("args") {
        None => &empty,
        Some(v) => v.as_object()?,
    };
    if args.len() > LOC_MAX_ARGS {
        return None;
    }
    for (name, value) in args {
        if !is_loc_arg(name) {
            return None;
        }
        if value.as_str()?.chars().count() > LOC_MAX_VALUE {
            return None;
        }
    }
    let mut out = Map::new();
    out.insert("key".into(), Value::String(key.into()));
    out.insert("args".into(), Value::Object(args.clone()));
    Some(Value::Object(out))
}

/// Finite positive number clamped to FCM's 4-week maximum, else `DEFAULT_TTL`.
fn ttl(value: Option<&Value>) -> u32 {
    let Some(Value::Number(n)) = value else {
        return DEFAULT_TTL;
    };
    let secs = if let Some(u) = n.as_u64() {
        if u == 0 {
            return DEFAULT_TTL;
        }
        u
    } else if let Some(f) = n.as_f64() {
        if !(f.is_finite() && f > 0.0) {
            return DEFAULT_TTL;
        }
        f as u64 // truncates like Python's int()
    } else {
        return DEFAULT_TTL; // negative integer
    };
    secs.clamp(1, u64::from(MAX_TTL)) as u32
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn actions_from(raw: Option<&Value>, id_key: &str, label_key: &str) -> Vec<Action> {
    let Some(Value::Array(items)) = raw else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|item| {
            let id = id_text(item.get(id_key));
            let label = text(item.get(label_key)).trim().to_string();
            (!id.is_empty() && !label.is_empty()).then_some(Action { id, label })
        })
        .collect()
}

fn legacy_actions(raw: Option<&Value>) -> Vec<Action> {
    if let Some(Value::String(s)) = raw {
        return match serde_json::from_str::<Value>(s) {
            Ok(parsed) => actions_from(Some(&parsed), "action", "btn"),
            Err(_) => Vec::new(),
        };
    }
    actions_from(raw, "action", "btn")
}

// ---------- normalize ----------

/// Normalise a parsed producer payload received from `source` (the topic's last segment).
pub fn normalize(source: &str, payload: &Value, now: u64) -> Result<Alert, Rejected> {
    let Some(obj) = payload.as_object() else {
        return Err(Rejected {
            reason: "payload is not a JSON object".into(),
        });
    };
    if obj.contains_key("v") {
        normalize_v1(source, obj, now)
    } else {
        Ok(normalize_legacy(source, obj, now))
    }
}

fn normalize_v1(
    source: &str,
    payload: &Map<String, Value>,
    sent_at: u64,
) -> Result<Alert, Rejected> {
    let version = &payload["v"];
    let is_one = matches!(version, Value::Number(n) if n.as_i64() == Some(1));
    if !is_one {
        return Err(Rejected {
            reason: format!("unsupported v: {version}"),
        });
    }
    let id = id_text(payload.get("id"));
    if id.is_empty() {
        return Err(Rejected {
            reason: "v1 payload without id".into(),
        });
    }
    let channel = match payload.get("channel") {
        Some(Value::String(c)) if CHANNELS.contains(&c.as_str()) => c.clone(),
        _ => "misc".to_string(),
    };
    if payload.get("cancel") == Some(&Value::Bool(true)) {
        return Ok(Alert {
            id,
            channel,
            cancel: true,
            sent_at,
            source: source.into(),
            ..Alert::default()
        });
    }
    let title = text(payload.get("title")).trim().to_string();
    if title.is_empty() {
        return Err(Rejected {
            reason: "v1 payload without title".into(),
        });
    }
    let severity = match payload.get("severity") {
        Some(Value::String(s)) if SEVERITIES.contains(&s.as_str()) => s.clone(),
        _ => "info".to_string(),
    };
    Ok(Alert {
        id,
        channel,
        severity,
        title,
        body: text(payload.get("body")),
        image_url: opt(payload.get("image_url")),
        video_url: opt(payload.get("video_url")),
        entity_id: opt(payload.get("entity_id")),
        actions: actions_from(payload.get("actions"), "id", "label"),
        silent: payload.get("silent") == Some(&Value::Bool(true)),
        cancel: false,
        ttl: ttl(payload.get("ttl")),
        sent_at,
        source: source.into(),
        loc: loc(payload.get("loc")),
        camera: opt(payload.get("camera")),
    })
}

fn sha1_prefix(input: &str) -> String {
    Sha1::digest(input.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn normalize_legacy(source: &str, payload: &Map<String, Value>, sent_at: u64) -> Alert {
    let raw_title = text(payload.get("title"));
    let body = text(payload.get("body"));
    let empty = Map::new();
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut id = id_text(data.get("id"));
    if id.is_empty() {
        id = format!("{source}-{}", sha1_prefix(&format!("{raw_title}{body}")));
    }
    let channel = match data.get("channelId") {
        Some(Value::String(c)) => LEGACY_CHANNELS
            .iter()
            .find(|(k, _)| k == c)
            .map_or("misc", |(_, v)| v),
        _ => "misc",
    }
    .to_string();
    let severity = if payload.get("priority") == Some(&Value::String("high".into())) {
        "warning"
    } else {
        "info"
    }
    .to_string();
    if raw_title.trim().is_empty() {
        return Alert {
            id,
            channel,
            severity,
            cancel: true,
            sent_at,
            source: source.into(),
            ..Alert::default()
        };
    }
    Alert {
        id,
        channel,
        severity,
        title: raw_title.trim().to_string(),
        body,
        image_url: opt(data.get("image_url")),
        video_url: opt(data.get("video_url")),
        entity_id: None,
        actions: legacy_actions(data.get("actions")),
        silent: truthy(data.get("silent")),
        cancel: false,
        ttl: ttl(payload.get("ttl")),
        sent_at,
        source: source.into(),
        loc: None,
        camera: None,
    }
}

// ---------- FCM data map ----------

fn size(data: &BTreeMap<String, String>) -> usize {
    data.iter().map(|(k, v)| k.len() + v.len()).sum()
}

/// Build the string-only FCM data map, shrunk to fit `FCM_MAX_BYTES`.
pub fn to_fcm_data(alert: &Alert) -> BTreeMap<String, String> {
    let mut data = BTreeMap::new();
    let mut put = |k: &str, v: String| {
        data.insert(k.to_string(), v);
    };
    put("v", "1".into());
    put("id", alert.id.clone());
    put("channel", alert.channel.clone());
    put("severity", alert.severity.clone());
    put("title", alert.title.clone());
    put("body", alert.body.clone());
    put(
        "actions",
        serde_json::to_string(&alert.actions).unwrap_or_else(|_| "[]".into()),
    );
    put("silent", if alert.silent { "true" } else { "false" }.into());
    put("sent_at", alert.sent_at.to_string());
    put("source", alert.source.clone());
    for (key, value) in [
        ("image_url", &alert.image_url),
        ("video_url", &alert.video_url),
        ("entity_id", &alert.entity_id),
    ] {
        if let Some(v) = value {
            put(key, v.clone());
        }
    }
    if let Some(l) = &alert.loc {
        put("loc", loc_json(l));
    }
    if alert.cancel {
        put("cancel", "true".into());
    }
    fit(data)
}

/// Compact JSON with `key` before `args` (the map type would sort `args` first).
fn loc_json(loc: &Value) -> String {
    let key = serde_json::to_string(&loc["key"]).unwrap_or_default();
    let args = serde_json::to_string(&loc["args"]).unwrap_or_else(|_| "{}".into());
    format!("{{\"key\":{key},\"args\":{args}}}")
}

/// Shrink `data[key]` (UTF-8 safe, with an ellipsis) so the whole map fits, or empty it.
fn truncate_field(data: &mut BTreeMap<String, String>, key: &str) {
    let current = data
        .insert(key.to_string(), String::new())
        .unwrap_or_default();
    let fixed = size(data);
    let Some(budget) = FCM_MAX_BYTES.checked_sub(fixed + ELLIPSIS.len()) else {
        return;
    };
    let mut cut = budget.min(current.len());
    while !current.is_char_boundary(cut) {
        cut -= 1;
    }
    data.insert(key.to_string(), format!("{}{ELLIPSIS}", &current[..cut]));
}

/// Guarantee key+value UTF-8 bytes <= `FCM_MAX_BYTES`: drop `loc`, shrink `body` then `title`,
/// empty `actions`, then drop `entity_id`, `video_url` and `image_url`, stopping once it fits.
fn fit(mut out: BTreeMap<String, String>) -> BTreeMap<String, String> {
    if size(&out) <= FCM_MAX_BYTES {
        return out;
    }
    out.remove("loc");
    if size(&out) <= FCM_MAX_BYTES {
        return out;
    }
    for key in ["body", "title"] {
        truncate_field(&mut out, key);
        if size(&out) <= FCM_MAX_BYTES {
            return out;
        }
    }
    out.insert("actions".into(), "[]".into());
    for key in ["entity_id", "video_url", "image_url"] {
        if size(&out) <= FCM_MAX_BYTES {
            break;
        }
        out.remove(key);
    }
    out
}

#[cfg(test)]
mod tests;
