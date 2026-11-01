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
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_700_000_000;
    const SRC: &str = "frigate";

    fn n(payload: Value) -> Result<Alert, Rejected> {
        normalize(SRC, &payload, NOW)
    }
    fn ok(payload: Value) -> Alert {
        n(payload).expect("alert")
    }
    fn fcm_size(data: &BTreeMap<String, String>) -> usize {
        size(data)
    }
    fn act(id: &str, label: &str) -> Action {
        Action {
            id: id.into(),
            label: label.into(),
        }
    }
    fn alert(f: impl FnOnce(&mut Alert)) -> Alert {
        let mut a = Alert {
            id: "x".into(),
            title: "T".into(),
            ..Alert::default()
        };
        f(&mut a);
        a
    }

    // ---------- v1 ----------

    #[test]
    fn v1_every_field() {
        let a = ok(json!({
            "v": 1, "id": "frigate-front-1696", "channel": "camera", "severity": "critical",
            "title": "Front door", "body": "Person detected",
            "image_url": "http://192.168.1.20:5000/api/events/1696/snapshot.jpg",
            "video_url": "http://192.168.1.20:5000/api/events/1696/clip.mp4",
            "entity_id": "binary_sensor.front_door", "camera": "front_door",
            "actions": [{"id": "disarm", "label": "Disarm"}],
            "silent": true, "ttl": 600,
        }));
        assert_eq!(
            a,
            Alert {
                id: "frigate-front-1696".into(),
                channel: "camera".into(),
                severity: "critical".into(),
                title: "Front door".into(),
                body: "Person detected".into(),
                image_url: Some("http://192.168.1.20:5000/api/events/1696/snapshot.jpg".into()),
                video_url: Some("http://192.168.1.20:5000/api/events/1696/clip.mp4".into()),
                entity_id: Some("binary_sensor.front_door".into()),
                actions: vec![act("disarm", "Disarm")],
                silent: true,
                cancel: false,
                ttl: 600,
                sent_at: NOW,
                source: "frigate".into(),
                loc: None,
                camera: Some("front_door".into()),
            }
        );
    }

    #[test]
    fn v1_defaults() {
        let a = ok(json!({"v": 1, "id": "x", "title": "T"}));
        assert_eq!(
            (
                a.channel.as_str(),
                a.severity.as_str(),
                a.body.as_str(),
                a.actions.len(),
                a.silent,
                a.ttl
            ),
            ("misc", "info", "", 0, false, DEFAULT_TTL)
        );
        assert_eq!((a.image_url, a.video_url, a.entity_id), (None, None, None));
        assert_eq!(a.camera, None);
    }

    #[test]
    fn v1_unknown_channel_becomes_misc() {
        for channel in [json!("nope"), json!(null), json!(7), json!(["alarm"])] {
            assert_eq!(
                ok(json!({"v": 1, "id": "x", "title": "T", "channel": channel})).channel,
                "misc"
            );
        }
    }

    #[test]
    fn v1_all_seven_channels_accepted() {
        for channel in CHANNELS {
            assert_eq!(
                ok(json!({"v": 1, "id": "x", "title": "T", "channel": channel})).channel,
                channel
            );
        }
    }

    #[test]
    fn v1_unknown_severity_becomes_info() {
        for severity in [json!("urgent"), json!(null), json!(3)] {
            assert_eq!(
                ok(json!({"v": 1, "id": "x", "title": "T", "severity": severity})).severity,
                "info"
            );
        }
    }

    #[test]
    fn v1_bad_ttl_uses_default() {
        for ttl in [json!(0), json!(-5), json!("600"), json!(null), json!(true)] {
            assert_eq!(
                ok(json!({"v": 1, "id": "x", "title": "T", "ttl": ttl})).ttl,
                DEFAULT_TTL
            );
        }
    }

    #[test]
    fn v1_numeric_id_becomes_string() {
        assert_eq!(ok(json!({"v": 1, "id": 1696, "title": "T"})).id, "1696");
    }

    #[test]
    fn v1_invalid_actions_entries_are_dropped() {
        let a = ok(json!({"v": 1, "id": "x", "title": "T", "actions": [
            {"id": "a", "label": "A"}, {"id": "b"}, {"label": "C"}, "junk", {"id": "", "label": "D"}]}));
        assert_eq!(a.actions, vec![act("a", "A")]);
    }

    #[test]
    fn v1_cancel_needs_only_v_and_id() {
        let a = ok(json!({"v": 1, "id": "frigate-front-1696", "cancel": true}));
        assert_eq!(
            a,
            Alert {
                id: "frigate-front-1696".into(),
                cancel: true,
                sent_at: NOW,
                source: "frigate".into(),
                ..Alert::default()
            }
        );
    }

    #[test]
    fn v1_cancel_drops_other_content() {
        let a = ok(
            json!({"v": 1, "id": "x", "cancel": true, "title": "T", "body": "B",
                          "image_url": "http://x/y.jpg"}),
        );
        assert_eq!(
            (a.title.as_str(), a.body.as_str(), a.image_url, a.cancel),
            ("", "", None, true)
        );
    }

    // ---------- rejections ----------

    #[test]
    fn rejected_v1_payloads() {
        for payload in [
            json!({"v": 1, "title": "T"}),
            json!({"v": 1, "id": "", "title": "T"}),
            json!({"v": 1, "id": "x"}),
            json!({"v": 1, "id": "x", "title": "   "}),
            json!({"v": 1, "cancel": true}),
            json!({"v": 2, "id": "x", "title": "T"}),
            json!({"v": "1", "id": "x", "title": "T"}),
            json!({"v": true, "id": "x", "title": "T"}),
            json!({"v": null, "id": "x", "title": "T"}),
        ] {
            assert!(n(payload.clone()).is_err(), "{payload}");
        }
    }

    #[test]
    fn non_object_payloads_are_rejected() {
        for payload in [
            json!([1, 2]),
            json!("text"),
            json!(42),
            json!(null),
            json!(true),
        ] {
            let r = n(payload).unwrap_err();
            assert!(!r.reason.is_empty());
        }
    }

    // ---------- legacy ----------

    #[test]
    fn legacy_channel_mapping() {
        for (channel_id, expected) in [
            (json!("alarme"), "alarm"),
            (json!("nvr"), "camera"),
            (json!("pv"), "energy"),
            (json!("surveillance"), "climate"),
            (json!("Supervisor"), "system"),
            (json!("anything-else"), "misc"),
            (json!(null), "misc"),
            (json!(12), "misc"),
        ] {
            let a = ok(
                json!({"title": "T", "body": "B", "data": {"id": "1", "channelId": channel_id}}),
            );
            assert_eq!(a.channel, expected);
        }
    }

    #[test]
    fn legacy_full_example() {
        let payload = json!({
            "title": "Alarme", "body": "Intrusion salon", "priority": "high", "ttl": 900,
            "data": {
                "id": 1696, "channelId": "alarme", "image_url": "http://cam/snap.jpg",
                "video_url": "http://cam/clip.mp4", "silent": "true",
                "actions": "[{\"btn\":\"Désarmer\",\"action\":\"disarm\"},{\"btn\":\"Ignorer\",\"action\":\"ignore\"}]",
                "something_else": "dropped", "color": "red",
            },
        });
        assert_eq!(
            normalize("jeedom", &payload, NOW).unwrap(),
            Alert {
                id: "1696".into(),
                channel: "alarm".into(),
                severity: "warning".into(),
                title: "Alarme".into(),
                body: "Intrusion salon".into(),
                image_url: Some("http://cam/snap.jpg".into()),
                video_url: Some("http://cam/clip.mp4".into()),
                entity_id: None,
                actions: vec![act("disarm", "Désarmer"), act("ignore", "Ignorer")],
                silent: true,
                cancel: false,
                ttl: 900,
                sent_at: NOW,
                source: "jeedom".into(),
                loc: None,
                camera: None,
            }
        );
    }

    #[test]
    fn legacy_id_number_or_string() {
        for (raw, expected) in [
            (json!("abc"), "abc"),
            (json!(42), "42"),
            (json!(42.0), "42"),
        ] {
            assert_eq!(ok(json!({"title": "T", "data": {"id": raw}})).id, expected);
        }
    }

    #[test]
    fn legacy_missing_id_falls_back_to_source_and_sha1() {
        // sha1("Door openSalon")[:8]
        let expected = format!("frigate-{}", sha1_prefix("Door openSalon"));
        assert_eq!(expected.len(), "frigate-".len() + 8);
        for data in [
            json!({}),
            json!({"id": ""}),
            json!({"id": null}),
            json!(null),
            json!("junk"),
        ] {
            let a = ok(json!({"title": "Door open", "body": "Salon", "data": data}));
            assert_eq!(a.id, expected);
        }
    }

    #[test]
    fn sha1_prefix_matches_known_digest() {
        // sha1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
        assert_eq!(sha1_prefix("abc"), "a9993e36");
    }

    #[test]
    fn legacy_same_content_gives_same_fallback_id() {
        let p = json!({"title": "Door open", "body": "Salon"});
        assert_eq!(ok(p.clone()).id, ok(p).id);
    }

    #[test]
    fn legacy_empty_or_missing_title_is_cancel() {
        for title in [Some(""), None, Some("   ")] {
            let mut payload = json!({"body": "x", "data": {"id": "77"}});
            if let Some(t) = title {
                payload["title"] = json!(t);
            }
            let a = ok(payload);
            assert_eq!(
                (a.id.as_str(), a.cancel, a.title.as_str(), a.body.as_str()),
                ("77", true, "", "")
            );
        }
    }

    #[test]
    fn legacy_priority_maps_to_severity() {
        for (priority, severity) in [
            (Some("high"), "warning"),
            (Some("normal"), "info"),
            (None, "info"),
        ] {
            let mut payload = json!({"title": "T"});
            if let Some(p) = priority {
                payload["priority"] = json!(p);
            }
            assert_eq!(ok(payload).severity, severity);
        }
    }

    #[test]
    fn legacy_bad_actions_json_gives_no_actions() {
        assert!(
            ok(json!({"title": "T", "data": {"id": "1", "actions": "not json"}}))
                .actions
                .is_empty()
        );
    }

    #[test]
    fn legacy_actions_may_already_be_a_list() {
        let a = ok(
            json!({"title": "T", "data": {"id": "1", "actions": [{"btn": "Go", "action": "go"}, {"btn": "x"}]}}),
        );
        assert_eq!(a.actions, vec![act("go", "Go")]);
    }

    #[test]
    fn legacy_defaults() {
        let a = ok(json!({"title": "T"}));
        assert_eq!(
            (a.ttl, a.silent, a.entity_id, a.image_url),
            (DEFAULT_TTL, false, None, None)
        );
    }

    // ---------- to_fcm_data ----------

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn to_fcm_data_full_map_is_all_strings() {
        let a = Alert {
            id: "frigate-front-1696".into(),
            channel: "camera".into(),
            severity: "critical".into(),
            title: "Front door".into(),
            body: "Person detected".into(),
            image_url: Some("http://i/s.jpg".into()),
            video_url: Some("http://i/c.mp4".into()),
            entity_id: Some("binary_sensor.front_door".into()),
            actions: vec![act("disarm", "Disarm")],
            silent: false,
            sent_at: NOW,
            source: "frigate".into(),
            ..Alert::default()
        };
        assert_eq!(
            to_fcm_data(&a),
            map(&[
                ("v", "1"),
                ("id", "frigate-front-1696"),
                ("channel", "camera"),
                ("severity", "critical"),
                ("title", "Front door"),
                ("body", "Person detected"),
                ("image_url", "http://i/s.jpg"),
                ("video_url", "http://i/c.mp4"),
                ("entity_id", "binary_sensor.front_door"),
                ("actions", r#"[{"id":"disarm","label":"Disarm"}]"#),
                ("silent", "false"),
                ("sent_at", &NOW.to_string()),
                ("source", "frigate"),
            ])
        );
    }

    #[test]
    fn to_fcm_data_leaves_out_absent_optionals_and_has_empty_actions() {
        let data = to_fcm_data(&alert(|a| {
            a.sent_at = NOW;
            a.source = "s".into();
        }));
        assert!(
            !data.contains_key("image_url")
                && !data.contains_key("video_url")
                && !data.contains_key("entity_id")
        );
        assert!(!data.contains_key("cancel"));
        assert_eq!(data["actions"], "[]");
        assert_eq!(data["silent"], "false");
    }

    #[test]
    fn to_fcm_data_cancel_and_silent() {
        let data = to_fcm_data(&Alert {
            id: "x".into(),
            cancel: true,
            silent: true,
            sent_at: NOW,
            source: "s".into(),
            ..Alert::default()
        });
        assert_eq!(data["cancel"], "true");
        assert_eq!(data["silent"], "true");
        assert!(data["title"].is_empty() && data["body"].is_empty());
    }

    #[test]
    fn to_fcm_data_keeps_non_ascii_unescaped() {
        let data = to_fcm_data(&alert(|a| a.actions = vec![act("a", "Désarmer")]));
        assert!(data["actions"].contains("Désarmer"));
    }

    #[test]
    fn small_body_is_not_truncated() {
        let data = to_fcm_data(&alert(|a| a.body = "hello".into()));
        assert_eq!(data["body"], "hello");
    }

    #[test]
    fn truncation_ascii_body_fits_under_limit() {
        let data = to_fcm_data(&alert(|a| a.body = "a".repeat(10_000)));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
        assert!(data["body"].ends_with('…'));
        assert!(data["body"].starts_with(&"a".repeat(100)));
    }

    #[test]
    fn truncation_multibyte_body_stays_valid_utf8_and_under_limit() {
        for body in [
            "é".repeat(5000),
            "🔥".repeat(3000),
            format!("é{}{}", "a".repeat(5000), "漢".repeat(100)),
        ] {
            let data = to_fcm_data(&alert(|a| {
                a.title = "Titre é".into();
                a.body = body.clone();
            }));
            assert!(fcm_size(&data) <= FCM_MAX_BYTES);
            assert!(data["body"].ends_with('…'));
            assert!(body.starts_with(data["body"].trim_end_matches('…')));
        }
    }

    #[test]
    fn ttl_constants() {
        assert_eq!((DEFAULT_TTL, CANCEL_TTL, FCM_MAX_BYTES), (43200, 300, 4096));
        assert_eq!(MAX_TTL, 2_419_200);
    }

    #[test]
    fn fit_truncates_a_huge_title_after_the_body() {
        let data = to_fcm_data(&alert(|a| {
            a.title = "é".repeat(3000);
            a.body = "b".repeat(3000);
        }));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
        assert!(data["title"].ends_with('…') && data["body"].is_empty());
    }

    #[test]
    fn fit_truncates_a_title_alone_when_body_is_empty() {
        let data = to_fcm_data(&alert(|a| a.title = "é".repeat(3000)));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
        assert!(data["title"].ends_with('…'));
    }

    #[test]
    fn fit_drops_actions_then_optional_fields_as_last_resort() {
        let data = to_fcm_data(&alert(|a| {
            a.actions = (0..200)
                .map(|i| act(&i.to_string(), &"L".repeat(50)))
                .collect();
            a.entity_id = Some("e".repeat(3000));
            a.video_url = Some("v".repeat(3000));
            a.image_url = Some("i".repeat(3000));
        }));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
        assert_eq!(data["actions"], "[]");
        assert!(!data.contains_key("entity_id") && !data.contains_key("video_url"));
    }

    #[test]
    fn fit_guarantees_the_limit_with_everything_oversized() {
        let data = to_fcm_data(&alert(|a| {
            a.title = "é".repeat(3000);
            a.body = "é".repeat(3000);
            a.entity_id = Some("e".repeat(5000));
            a.video_url = Some("v".repeat(5000));
            a.image_url = Some("i".repeat(5000));
        }));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
    }

    #[test]
    fn ttl_is_clamped_to_fcm_max_four_weeks() {
        assert_eq!(
            ok(json!({"v": 1, "id": "x", "title": "T", "ttl": 1_000_000_000u64})).ttl,
            2_419_200
        );
    }

    #[test]
    fn non_finite_ttl_uses_default() {
        // JSON cannot carry inf/nan; serde_json turns them into null, which is the default too.
        for ttl in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(
                ok(json!({"v": 1, "id": "x", "title": "T", "ttl": ttl})).ttl,
                DEFAULT_TTL
            );
        }
    }

    // ---------- loc ----------

    fn loc_value() -> Value {
        json!({"key": "frigate.object", "args": {"label": "car", "zone": "Driveway"}})
    }

    fn v1(extra: (&str, Value)) -> Value {
        let mut p = json!({"v": 1, "id": "a1", "channel": "camera", "title": "Driveway", "body": "Car in Driveway"});
        p[extra.0] = extra.1;
        p
    }

    #[test]
    fn loc_is_kept_and_sent_as_compact_json() {
        let a = ok(v1(("loc", loc_value())));
        assert_eq!(a.loc, Some(loc_value()));
        assert_eq!(
            to_fcm_data(&a)["loc"],
            r#"{"key":"frigate.object","args":{"label":"car","zone":"Driveway"}}"#
        );
    }

    #[test]
    fn loc_args_may_be_missing() {
        assert_eq!(
            ok(v1(("loc", json!({"key": "x.y"})))).loc,
            Some(json!({"key": "x.y", "args": {}}))
        );
    }

    #[test]
    fn no_loc_means_no_fcm_key() {
        let a = ok(json!({"v": 1, "id": "a1", "title": "T"}));
        assert!(a.loc.is_none());
        assert!(!to_fcm_data(&a).contains_key("loc"));
    }

    #[test]
    fn loc_wrong_shapes_are_dropped_but_the_alert_is_delivered() {
        let many: Map<String, Value> = (0..9).map(|i| (format!("a{i}"), json!("x"))).collect();
        for loc in [
            json!("{\"key\":\"frigate.object\"}"),
            json!({"args": {"label": "car"}}),
            json!({"key": "Frigate.Object"}),
            json!({"key": "x".repeat(65)}),
            json!({"key": "k", "args": ["car"]}),
            json!({"key": "k", "args": {"label": 3}}),
            json!({"key": "k", "args": {"Label": "car"}}),
            json!({"key": "k", "args": {"v": "x".repeat(201)}}),
            json!({"key": "k", "args": many}),
        ] {
            let a = ok(v1(("loc", loc.clone())));
            assert!(a.loc.is_none(), "{loc}");
            assert_eq!(a.body, "Car in Driveway");
        }
    }

    #[test]
    fn legacy_payloads_never_carry_loc() {
        let a = normalize(
            "nvr",
            &json!({"title": "Cam", "body": "x", "loc": loc_value()}),
            NOW,
        )
        .unwrap();
        assert!(a.loc.is_none());
    }

    #[test]
    fn fit_drops_loc_before_emptying_the_body() {
        let args: Map<String, Value> = (0..8)
            .map(|i| (format!("a{i}"), json!("x".repeat(200))))
            .collect();
        let mut p = v1(("loc", json!({"key": "k", "args": args})));
        p["body"] = json!("b".repeat(3000));
        let data = to_fcm_data(&ok(p));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
        assert!(!data.contains_key("loc"));
        assert!(data["body"].starts_with(&"b".repeat(2000)));
    }

    // ---------- priority and ttl (from fcm_sender tests) ----------

    #[test]
    fn android_priority_table() {
        use Priority::{High, Normal};
        for (channel, severity, expected) in [
            ("misc", "warning", High),
            ("misc", "critical", High),
            ("energy", "info", Normal),
            ("climate", "info", Normal),
            ("system", "info", Normal),
            ("misc", "info", Normal),
            ("alarm", "info", High),
            ("security", "info", High),
            ("camera", "info", High),
            ("energy", "warning", High),
        ] {
            let a = alert(|a| {
                a.channel = channel.into();
                a.severity = severity.into();
            });
            assert_eq!(android_priority(&a), expected, "{channel}/{severity}");
        }
    }

    #[test]
    fn ttl_is_payload_ttl_with_default() {
        assert_eq!(ttl_seconds(&alert(|_| {})), 43200);
        assert_eq!(ttl_seconds(&alert(|a| a.ttl = 600)), 600);
    }

    #[test]
    fn cancel_ttl_is_300_whatever_the_payload_ttl() {
        assert_eq!(
            ttl_seconds(&alert(|a| {
                a.cancel = true;
                a.ttl = 43200;
            })),
            300
        );
    }
}
