//! Push alerts: the entries of OikosPush's retained history topic, as the web shows them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The seven notification channels of the push alerts contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertChannel {
    Alarm,
    Security,
    Camera,
    Energy,
    Climate,
    System,
    Misc,
}

impl AlertChannel {
    pub const ALL: [AlertChannel; 7] = [
        AlertChannel::Alarm,
        AlertChannel::Security,
        AlertChannel::Camera,
        AlertChannel::Energy,
        AlertChannel::Climate,
        AlertChannel::System,
        AlertChannel::Misc,
    ];

    /// The wire id (`camera`). An unknown id is `Misc`.
    pub fn id(self) -> &'static str {
        match self {
            AlertChannel::Alarm => "alarm",
            AlertChannel::Security => "security",
            AlertChannel::Camera => "camera",
            AlertChannel::Energy => "energy",
            AlertChannel::Climate => "climate",
            AlertChannel::System => "system",
            AlertChannel::Misc => "misc",
        }
    }

    /// The name shown to the user (`Camera`).
    pub fn name(self) -> &'static str {
        match self {
            AlertChannel::Alarm => "Alarm",
            AlertChannel::Security => "Security",
            AlertChannel::Camera => "Camera",
            AlertChannel::Energy => "Energy",
            AlertChannel::Climate => "Climate",
            AlertChannel::System => "System",
            AlertChannel::Misc => "Misc",
        }
    }

    fn from_id(id: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|c| Some(c.id()) == id)
            .unwrap_or(AlertChannel::Misc)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl AlertSeverity {
    fn from_id(id: Option<&str>) -> Self {
        match id {
            Some("warning") => AlertSeverity::Warning,
            Some("critical") => AlertSeverity::Critical,
            _ => AlertSeverity::Info,
        }
    }
}

/// A button on an alert: the `id` goes back to the producer, the `label` is shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AlertAction {
    pub id: String,
    pub label: String,
}

/// One normalized alert of the history, as the web shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AlertView {
    pub id: String,
    pub channel: AlertChannel,
    pub severity: AlertSeverity,
    pub title: String,
    pub body: String,
    pub image_url: Option<String>,
    pub video_url: Option<String>,
    pub entity_id: Option<String>,
    /// The raw Frigate camera id of a camera alert; older entries have none.
    pub camera: Option<String>,
    /// At most [`MAX_ACTIONS`].
    pub actions: Vec<AlertAction>,
    pub source: Option<String>,
    pub silent: bool,
    /// The entry's `cancel: true`.
    pub resolved: bool,
    /// Unix seconds the entry was cancelled; older entries have none.
    pub resolved_at: Option<u64>,
    /// Unix seconds.
    pub sent_at: u64,
}

/// The app shows at most three buttons; the rest are dropped.
pub const MAX_ACTIONS: usize = 3;

/// The retained history payload as alerts, in the order published (newest first).
/// `None` when the payload is not a JSON array; a bad entry is skipped, never fatal.
pub fn parse_history(payload: &[u8]) -> Option<Vec<AlertView>> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    let entries = value.as_array()?;
    Some(entries.iter().filter_map(parse_entry).collect())
}

fn parse_entry(entry: &Value) -> Option<AlertView> {
    let object = entry.as_object()?;
    let id = text(object.get("id"))?;
    if id.is_empty() {
        return None;
    }
    let resolved = object.get("cancel").and_then(Value::as_bool) == Some(true);
    let title = text(object.get("title")).unwrap_or_default();
    if resolved && title.trim().is_empty() {
        // A cancel for an alert the history never held: nothing to show.
        return None;
    }
    let actions = object
        .get("actions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(AlertAction {
                        id: text(item.get("id"))?,
                        label: text(item.get("label"))?,
                    })
                })
                .take(MAX_ACTIONS)
                .collect()
        })
        .unwrap_or_default();
    Some(AlertView {
        id,
        channel: AlertChannel::from_id(object.get("channel").and_then(Value::as_str)),
        severity: AlertSeverity::from_id(object.get("severity").and_then(Value::as_str)),
        title,
        body: text(object.get("body")).unwrap_or_default(),
        image_url: text(object.get("image_url")),
        video_url: text(object.get("video_url")),
        entity_id: text(object.get("entity_id")),
        camera: text(object.get("camera")),
        actions,
        source: text(object.get("source")),
        silent: object
            .get("silent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        resolved,
        resolved_at: object.get("resolved_at").and_then(Value::as_u64),
        sent_at: object.get("sent_at").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// A string field; anything else (absent, null, number, object) is `None`.
fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = r#"{"v":1,"id":"frigate-front-1696","channel":"camera","severity":"warning","title":"Front Door","body":"Person in Porch","image_url":"http://192.168.1.20:5000/api/events/1696/snapshot.jpg","video_url":"http://192.168.1.20:5000/api/events/1696/clip.mp4","entity_id":"binary_sensor.front_door","camera":"front_door","actions":[{"id":"disarm","label":"Disarm"}],"silent":false,"sent_at":1760880000,"source":"frigate","loc":{"key":"frigate.object","args":{"label":"person","zone":"Porch"}}}"#;

    fn history(entries: &[&str]) -> Vec<u8> {
        format!("[{}]", entries.join(",")).into_bytes()
    }

    fn with(entry: &str, patch: &str) -> String {
        let mut value: serde_json::Value = serde_json::from_str(entry).unwrap();
        let patch: serde_json::Value = serde_json::from_str(patch).unwrap();
        for (k, v) in patch.as_object().unwrap() {
            value[k] = v.clone();
        }
        value.to_string()
    }

    #[test]
    fn parses_a_full_entry() {
        let alerts = parse_history(&history(&[ENTRY])).unwrap();
        assert_eq!(
            alerts,
            vec![AlertView {
                id: "frigate-front-1696".into(),
                channel: AlertChannel::Camera,
                severity: AlertSeverity::Warning,
                title: "Front Door".into(),
                body: "Person in Porch".into(),
                image_url: Some("http://192.168.1.20:5000/api/events/1696/snapshot.jpg".into()),
                video_url: Some("http://192.168.1.20:5000/api/events/1696/clip.mp4".into()),
                entity_id: Some("binary_sensor.front_door".into()),
                camera: Some("front_door".into()),
                actions: vec![AlertAction {
                    id: "disarm".into(),
                    label: "Disarm".into()
                }],
                source: Some("frigate".into()),
                silent: false,
                resolved: false,
                resolved_at: None,
                sent_at: 1_760_880_000,
            }]
        );
    }

    #[test]
    fn an_entry_without_camera_has_none() {
        let entry = with(ENTRY, r#"{"camera":null}"#);
        let alerts = parse_history(&history(&[&entry])).unwrap();
        assert_eq!(alerts[0].camera, None);
    }

    #[test]
    fn unknown_channel_and_severity_fall_back() {
        let entry = with(ENTRY, r#"{"channel":"nvr","severity":"urgent"}"#);
        let alerts = parse_history(&history(&[&entry])).unwrap();
        assert_eq!(alerts[0].channel, AlertChannel::Misc);
        assert_eq!(alerts[0].severity, AlertSeverity::Info);
    }

    #[test]
    fn cancel_marks_resolved() {
        let entry = with(ENTRY, r#"{"cancel":true}"#);
        let alerts = parse_history(&history(&[&entry])).unwrap();
        assert!(alerts[0].resolved);
        assert_eq!(alerts[0].title, "Front Door");
    }

    #[test]
    fn cancel_stub_without_title_is_dropped() {
        let stub = r#"{"v":1,"id":"x","cancel":true,"sent_at":1,"title":"","body":""}"#;
        assert_eq!(parse_history(&history(&[stub])).unwrap(), vec![]);
    }

    #[test]
    fn bad_entries_are_skipped_not_fatal() {
        let payload = br#"[42, {"id":7}, {"id":"ok","title":"T","sent_at":1}]"#;
        let alerts = parse_history(payload).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].id, "ok");
        assert_eq!(alerts[0].title, "T");
        assert_eq!(alerts[0].channel, AlertChannel::Misc);
        assert!(!alerts[0].silent);
    }

    #[test]
    fn optional_fields_tolerate_wrong_types() {
        let entry = with(
            ENTRY,
            r#"{"image_url":5,"actions":"x","sent_at":"soon","source":null,"silent":"yes"}"#,
        );
        let alerts = parse_history(&history(&[&entry])).unwrap();
        assert_eq!(alerts[0].image_url, None);
        assert_eq!(alerts[0].actions, vec![]);
        assert_eq!(alerts[0].sent_at, 0);
        assert_eq!(alerts[0].source, None);
        assert!(!alerts[0].silent);
    }

    #[test]
    fn at_most_three_actions() {
        let entry = with(
            ENTRY,
            r#"{"actions":[{"id":"a","label":"A"},{"id":"b","label":"B"},{"id":"c","label":"C"},{"id":"d","label":"D"}]}"#,
        );
        let alerts = parse_history(&history(&[&entry])).unwrap();
        assert_eq!(alerts[0].actions.len(), 3);
        assert_eq!(alerts[0].actions[2].id, "c");
    }

    #[test]
    fn non_array_is_none() {
        assert_eq!(parse_history(b"{}"), None);
        assert_eq!(parse_history(b"nope"), None);
        assert_eq!(parse_history(b"[]"), Some(vec![]));
    }

    #[test]
    fn channel_ids_and_names() {
        assert_eq!(AlertChannel::ALL.len(), 7);
        assert_eq!(AlertChannel::Camera.id(), "camera");
        assert_eq!(AlertChannel::Camera.name(), "Camera");
        assert_eq!(
            serde_json::to_value(AlertChannel::Alarm).unwrap(),
            serde_json::json!("alarm")
        );
    }
}
