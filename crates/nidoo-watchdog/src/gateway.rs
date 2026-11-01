//! Gateway health: republishes state transitions of `<prefix>/<component>` messages as alerts.
//!
//! Policy, per component (keyed by the full topic):
//! - steady `ok`, or the same non-ok status again: no alert;
//! - first message after start with a non-ok status: alert (news to us);
//! - first message after start with `ok`: no alert;
//! - `ok` to non-ok: "down"; non-ok to `ok`: "recovered"; one non-ok state to another: alert.
//!
//! The gateway publishes retained states, so a reconnect replays them: an unchanged state never
//! alerts.

use std::collections::HashMap;

use serde_json::{Value, json};

const TTL: u32 = 86400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Fail,
    ConfigError,
}

impl Status {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_lowercase().as_str() {
            "ok" => Some(Self::Ok),
            "fail" => Some(Self::Fail),
            "config_error" => Some(Self::ConfigError),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fail => "fail",
            Self::ConfigError => "config_error",
        }
    }
}

/// A parsed health message; `component` is the topic suffix.
struct Report {
    component: String,
    status: Status,
    exit_code: Option<i64>,
    host: Option<String>,
    timestamp: Option<i64>,
}

impl Report {
    fn parse(topic: &str, payload: &[u8], prefix: &str) -> Option<Self> {
        let component = topic.strip_prefix(prefix)?.strip_prefix('/')?;
        if component.is_empty() || component.contains('/') {
            return None;
        }
        let text = String::from_utf8_lossy(payload);
        let text = if text.is_empty() { "{}" } else { &text };
        let Value::Object(data) = serde_json::from_str(text).ok()? else {
            return None;
        };
        let status = Status::parse(data.get("status").and_then(Value::as_str).unwrap_or(""))?;
        Some(Self {
            component: component.to_owned(),
            status,
            exit_code: data.get("exit_code").and_then(Value::as_i64),
            host: data.get("host").and_then(Value::as_str).map(str::to_owned),
            timestamp: data.get("timestamp").and_then(Value::as_i64),
        })
    }
}

#[derive(Debug)]
pub struct GatewayWatchdog {
    prefix: String,
    last: HashMap<String, &'static str>,
}

impl GatewayWatchdog {
    /// Watches `<prefix>/<component>`; surrounding slashes of `prefix` are ignored.
    pub fn new(prefix: String) -> Self {
        Self {
            prefix: prefix.trim_matches('/').to_owned(),
            last: HashMap::new(),
        }
    }

    /// The MQTT filter to subscribe to.
    pub fn filter(&self) -> String {
        format!("{}/+", self.prefix)
    }

    /// Whether `topic` is a direct child of the prefix, so worth locking the watchdog for.
    pub fn wants(&self, topic: &str) -> bool {
        topic
            .strip_prefix(&self.prefix)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
    }

    /// Turns one health message into an alert, or `None` when the state did not change.
    /// `now` (epoch seconds) stamps the alert id when the message carries no timestamp.
    pub fn on_message(&mut self, topic: &str, payload: &[u8], now: u64) -> Option<Value> {
        let report = Report::parse(topic, payload, &self.prefix)?;
        let previous = self.last.insert(topic.to_owned(), report.status.as_str());
        if previous == Some(report.status.as_str()) {
            return None;
        }
        if previous.is_none() && report.status == Status::Ok {
            return None;
        }
        Some(alert(&report, previous, now))
    }
}

fn alert(r: &Report, previous: Option<&str>, now: u64) -> Value {
    let on_host = r
        .host
        .as_ref()
        .map(|h| format!(" on {h}"))
        .unwrap_or_default();
    let recovered = r.status == Status::Ok && previous.is_some_and(|p| p != "ok");
    let (severity, key, title, body) = if recovered {
        (
            "info",
            "gateway.health_recovered",
            format!("{} recovered", r.component),
            format!("{}: back to ok{on_host}", r.component),
        )
    } else if r.status == Status::ConfigError {
        (
            "warning",
            "gateway.health_config_error",
            format!("{} config error", r.component),
            format!("{}: configuration error{on_host}", r.component),
        )
    } else {
        (
            "warning",
            "gateway.health_down",
            format!("{} down", r.component),
            format!("{}: health check failed{on_host}", r.component),
        )
    };
    let ts = r.timestamp.unwrap_or(now as i64);
    let mut args = serde_json::Map::new();
    args.insert("component".into(), json!(r.component));
    args.insert("status".into(), json!(r.status.as_str()));
    if let Some(host) = &r.host {
        args.insert("host".into(), json!(host));
    }
    if let Some(previous) = previous {
        args.insert("previous".into(), json!(previous));
    }
    if let Some(code) = r.exit_code {
        args.insert("exit_code".into(), json!(code.to_string()));
    }
    json!({
        "v": 1,
        "id": format!("gw-health-{}-{}-{ts}", r.component, r.status.as_str()),
        "channel": "system",
        "severity": severity,
        "title": title,
        "body": body,
        "silent": false,
        "ttl": TTL,
        "loc": {"key": key, "args": args},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "encryption-gateway/health";
    const NOW: u64 = 1_700_000_000;

    fn msg(status: &str, ts: i64) -> Vec<u8> {
        json!({"component": "gateway", "status": status,
               "exit_code": if status == "ok" { 0 } else { 1 },
               "restart_attempted": false, "timestamp": ts, "host": "hub-host"})
        .to_string()
        .into_bytes()
    }

    fn feed(wd: &mut GatewayWatchdog, component: &str, status: &str, ts: i64) -> Option<Value> {
        wd.on_message(&format!("{PREFIX}/{component}"), &msg(status, ts), NOW)
    }

    fn wd() -> GatewayWatchdog {
        GatewayWatchdog::new(PREFIX.to_owned())
    }

    // --- parsing ---

    #[test]
    fn parse_extracts_component_from_topic_suffix() {
        let r =
            Report::parse(&format!("{PREFIX}/gateway"), &msg("fail", 1), PREFIX).expect("report");
        assert_eq!((r.component.as_str(), r.status), ("gateway", Status::Fail));
        assert_eq!(
            (r.host.as_deref(), r.exit_code),
            (Some("hub-host"), Some(1))
        );
    }

    #[test]
    fn parse_rejects_unknown_status() {
        let payload = br#"{"status":"pending"}"#;
        assert!(Report::parse(&format!("{PREFIX}/gateway"), payload, PREFIX).is_none());
    }

    #[test]
    fn parse_rejects_nested_topic() {
        assert!(Report::parse(&format!("{PREFIX}/sub/gateway"), &msg("ok", 1), PREFIX).is_none());
    }

    #[test]
    fn parse_rejects_non_json_or_non_object() {
        let topic = format!("{PREFIX}/gateway");
        assert!(Report::parse(&topic, b"not-json", PREFIX).is_none());
        assert!(Report::parse(&topic, b"[]", PREFIX).is_none());
    }

    #[test]
    fn parse_rejects_topic_outside_prefix() {
        assert!(Report::parse("other/health/gateway", &msg("ok", 1), PREFIX).is_none());
    }

    // --- transition policy ---

    #[test]
    fn first_seen_ok_does_not_alert() {
        assert!(feed(&mut wd(), "gateway", "ok", 1).is_none());
    }

    #[test]
    fn first_seen_fail_alerts_as_news() {
        let alert = feed(&mut wd(), "gateway", "fail", 1).expect("alert");
        assert_eq!(alert["v"], 1);
        assert_eq!(alert["channel"], "system");
        assert_eq!(alert["severity"], "warning");
        assert_eq!(alert["ttl"], 86400);
        assert_eq!(alert["silent"], false);
        assert_eq!(alert["title"], "gateway down");
        assert_eq!(alert["body"], "gateway: health check failed on hub-host");
        assert_eq!(alert["loc"]["key"], "gateway.health_down");
        assert_eq!(alert["loc"]["args"]["component"], "gateway");
        assert_eq!(alert["loc"]["args"]["exit_code"], "1");
        assert!(alert["loc"]["args"].get("previous").is_none());
    }

    #[test]
    fn steady_ok_does_not_alert() {
        let mut wd = wd();
        for _ in 0..3 {
            assert!(feed(&mut wd, "gateway", "ok", 1).is_none());
        }
    }

    #[test]
    fn steady_fail_only_alerts_on_first_transition() {
        let mut wd = wd();
        assert!(feed(&mut wd, "gateway", "ok", 1).is_none());
        let alert = feed(&mut wd, "gateway", "fail", 2).expect("down");
        assert_eq!(alert["loc"]["key"], "gateway.health_down");
        assert_eq!(alert["loc"]["args"]["previous"], "ok");
        assert!(feed(&mut wd, "gateway", "fail", 3).is_none());
    }

    #[test]
    fn replayed_retained_state_never_alerts_twice() {
        // A reconnect replays the retained message of every component, unchanged.
        let mut wd = wd();
        assert!(feed(&mut wd, "gateway", "fail", 5).is_some());
        assert!(feed(&mut wd, "gateway", "fail", 5).is_none());
        assert!(feed(&mut wd, "tailscale", "ok", 5).is_none());
        assert!(feed(&mut wd, "tailscale", "ok", 5).is_none());
    }

    #[test]
    fn recovery_fires_an_info_alert() {
        let mut wd = wd();
        feed(&mut wd, "gateway", "fail", 1).expect("down");
        let alert = feed(&mut wd, "gateway", "ok", 2).expect("recovered");
        assert_eq!(alert["severity"], "info");
        assert_eq!(alert["title"], "gateway recovered");
        assert_eq!(alert["body"], "gateway: back to ok on hub-host");
        assert_eq!(alert["loc"]["key"], "gateway.health_recovered");
        assert_eq!(alert["loc"]["args"]["previous"], "fail");
    }

    #[test]
    fn config_error_uses_its_own_loc_key() {
        let alert = feed(&mut wd(), "gateway", "config_error", 1).expect("alert");
        assert_eq!(alert["loc"]["key"], "gateway.health_config_error");
        assert_eq!(alert["title"], "gateway config error");
    }

    #[test]
    fn fail_then_config_error_still_alerts() {
        let mut wd = wd();
        feed(&mut wd, "gateway", "fail", 1).expect("down");
        let alert = feed(&mut wd, "gateway", "config_error", 2).expect("alert");
        assert_eq!(alert["loc"]["key"], "gateway.health_config_error");
        assert_eq!(alert["loc"]["args"]["previous"], "fail");
    }

    #[test]
    fn components_are_tracked_independently() {
        let mut wd = wd();
        assert!(feed(&mut wd, "gateway", "ok", 1).is_none());
        let down = feed(&mut wd, "tailscale", "fail", 2).expect("down");
        assert!(feed(&mut wd, "gateway", "ok", 3).is_none());
        let up = feed(&mut wd, "tailscale", "ok", 4).expect("recovered");
        assert_eq!(down["loc"]["args"]["component"], "tailscale");
        assert_eq!(up["loc"]["args"]["component"], "tailscale");
        assert_eq!(down["loc"]["key"], "gateway.health_down");
        assert_eq!(up["loc"]["key"], "gateway.health_recovered");
    }

    #[test]
    fn alert_id_includes_component_status_and_timestamp() {
        let alert = feed(&mut wd(), "gateway", "fail", 1_700_000_000).expect("alert");
        assert_eq!(alert["id"], "gw-health-gateway-fail-1700000000");
    }

    #[test]
    fn alert_id_falls_back_to_now_without_a_timestamp() {
        let alert = wd()
            .on_message(&format!("{PREFIX}/gateway"), br#"{"status":"fail"}"#, 42)
            .expect("alert");
        assert_eq!(alert["id"], "gw-health-gateway-fail-42");
        assert_eq!(alert["body"], "gateway: health check failed");
    }

    #[test]
    fn non_health_topic_is_ignored() {
        let mut wd = wd();
        assert!(
            wd.on_message("other/health/gateway", &msg("fail", 1), NOW)
                .is_none()
        );
        assert!(!wd.wants("other/health/gateway"));
        assert!(wd.wants(&format!("{PREFIX}/gateway")));
        assert!(!wd.wants(&format!("{PREFIX}/a/b")));
        assert_eq!(wd.filter(), "encryption-gateway/health/+");
    }
}
