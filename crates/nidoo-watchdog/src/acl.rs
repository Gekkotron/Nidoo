//! Mosquitto drops an ACL-denied PUBLISH after acknowledging it, leaving only a log line. This
//! watchdog turns those `Denied PUBLISH/SUBSCRIBE/UNSUBSCRIBE` lines into alerts.
//!
//! Flood control: a `(client, topic, action)` key alerts once per cooldown window; denials
//! inside the window are counted, then named in the next alert's body.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

/// How many `(client, topic, action)` windows are remembered.
const SEEN_CAP: usize = 500;
const TTL: u32 = 86400;

#[derive(Debug, PartialEq, Eq)]
struct Denial {
    /// `publish`, `subscribe` or `unsubscribe`.
    action: String,
    client: String,
    topic: String,
    user: Option<String>,
}

/// The ASCII-case-insensitive `needle` positions in `line` (same byte offsets as `lower`).
fn find_all<'a>(lower: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    lower.match_indices(needle).map(|(i, _)| i)
}

/// Splits one whitespace-delimited token off `rest`, which must start with whitespace.
fn token(rest: &str) -> Option<(&str, &str)> {
    let trimmed = rest.trim_start();
    if trimmed.len() == rest.len() {
        return None;
    }
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    (end > 0).then(|| trimmed.split_at(end))
}

/// `Denied <ACTION> from <client> (... '<topic>' ...)`; the topic is the last quoted token.
fn denied(line: &str, lower: &str) -> Option<Denial> {
    find_all(lower, "denied").find_map(|at| {
        let rest = &line[at + "denied".len()..];
        let (action, rest) = token(rest)?;
        let action = action.to_ascii_lowercase();
        if !matches!(action.as_str(), "publish" | "subscribe" | "unsubscribe") {
            return None;
        }
        let (from, rest) = token(rest)?;
        if !from.eq_ignore_ascii_case("from") {
            return None;
        }
        let (client, rest) = token(rest)?;
        let inner = rest.trim_start().strip_prefix('(')?;
        let quotes: Vec<usize> = inner.match_indices('\'').map(|(i, _)| i).collect();
        let topic = quotes
            .windows(2)
            .rev()
            .find(|pair| pair[1] > pair[0] + 1)
            .map(|pair| &inner[pair[0] + 1..pair[1]])?;
        Some(Denial {
            action,
            client: client.to_owned(),
            topic: topic.to_owned(),
            user: None,
        })
    })
}

/// `Client <c> [is] not authori[sz]ed to <action> to <topic>`, the older wording.
fn not_authorised(line: &str, lower: &str) -> Option<Denial> {
    find_all(lower, "client").find_map(|at| {
        let rest = &line[at + "client".len()..];
        let (client, rest) = token(rest)?;
        let (mut word, mut rest) = token(rest)?;
        if word.eq_ignore_ascii_case("is") {
            (word, rest) = token(rest)?;
        }
        if !word.eq_ignore_ascii_case("not") {
            return None;
        }
        let (word, rest) = token(rest)?;
        if !word.eq_ignore_ascii_case("authorised") && !word.eq_ignore_ascii_case("authorized") {
            return None;
        }
        let (word, rest) = token(rest)?;
        if !word.eq_ignore_ascii_case("to") {
            return None;
        }
        let (action, rest) = token(rest)?;
        let action = action.to_ascii_lowercase();
        if !matches!(action.as_str(), "publish" | "subscribe" | "unsubscribe") {
            return None;
        }
        let (word, rest) = token(rest)?;
        if !word.eq_ignore_ascii_case("to") {
            return None;
        }
        let (topic, _) = token(rest)?;
        Some(Denial {
            action,
            client: client.to_owned(),
            topic: topic.to_owned(),
            user: None,
        })
    })
}

/// Mosquitto prints the username inside the parentheses: `(u'Suez', ...)`.
fn user_of(line: &str) -> Option<String> {
    line.match_indices("u'").find_map(|(i, _)| {
        let rest = &line[i + 2..];
        let end = rest.find('\'')?;
        (end > 0).then(|| rest[..end].to_owned())
    })
}

fn parse_denial(line: &str) -> Option<Denial> {
    let lower = line.to_ascii_lowercase();
    let mut denial = denied(line, &lower).or_else(|| not_authorised(line, &lower))?;
    denial.user = user_of(line);
    Some(denial)
}

type Key = (String, String, String);

#[derive(Debug)]
struct Window {
    first: u64,
    suppressed: u64,
    /// Recency stamp, to drop the stalest window past the cap.
    seq: u64,
}

#[derive(Debug)]
pub struct AclWatchdog {
    cooldown: u64,
    windows: HashMap<Key, Window>,
    seq: u64,
}

impl AclWatchdog {
    /// `cooldown` is the window, in seconds, during which repeats of one denial are held back.
    pub fn new(cooldown: u64) -> Self {
        Self {
            cooldown,
            windows: HashMap::new(),
            seq: 0,
        }
    }

    /// Turns one broker log line into an alert, or `None` when it is no denial or a repeat
    /// inside the cooldown. `now` is epoch seconds.
    pub fn on_line(&mut self, line: &str, now: u64) -> Option<Value> {
        let denial = parse_denial(line)?;
        let key = (
            denial.client.clone(),
            denial.topic.clone(),
            denial.action.clone(),
        );
        self.seq += 1;
        let seq = self.seq;
        let suppressed = match self.windows.get_mut(&key) {
            Some(w) if now.saturating_sub(w.first) < self.cooldown => {
                w.suppressed += 1;
                w.seq = seq;
                return None;
            }
            Some(w) => {
                let held = w.suppressed;
                *w = Window {
                    first: now,
                    suppressed: 0,
                    seq,
                };
                held
            }
            None => {
                self.windows.insert(
                    key,
                    Window {
                        first: now,
                        suppressed: 0,
                        seq,
                    },
                );
                0
            }
        };
        if self.windows.len() > SEEN_CAP
            && let Some(stalest) = self
                .windows
                .iter()
                .min_by_key(|(_, w)| w.seq)
                .map(|(k, _)| k.clone())
        {
            self.windows.remove(&stalest);
        }
        Some(self.alert(&denial, suppressed, now))
    }

    fn alert(&self, d: &Denial, suppressed: u64, window_start: u64) -> Value {
        let who = match &d.user {
            Some(user) => format!("{} (u={user})", d.client),
            None => d.client.clone(),
        };
        let tail = if suppressed > 0 {
            format!(" (+{suppressed} more in last {}s)", self.cooldown)
        } else {
            String::new()
        };
        let mut args = Map::new();
        args.insert("client".into(), json!(d.client));
        args.insert("action".into(), json!(d.action));
        args.insert("topic".into(), json!(d.topic));
        if let Some(user) = &d.user {
            args.insert("user".into(), json!(user));
        }
        if suppressed > 0 {
            args.insert("suppressed".into(), json!(suppressed.to_string()));
        }
        json!({
            "v": 1,
            "id": format!("mqtt-acl-{}-{}-{window_start}", d.client, d.action),
            "channel": "system",
            "severity": "warning",
            "title": "MQTT ACL denial",
            "body": format!("{who} denied {} on {}{tail}", d.action.to_uppercase(), d.topic),
            "silent": false,
            "ttl": TTL,
            "loc": {"key": "mqtt.acl_denied", "args": args},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATHENA_LINE: &str = "1791743566: Denied PUBLISH from Athena \
        (d0, q0, r1, m0, 'athena/heartbeat', ... (207 bytes))";
    const SUEZ_LINE: &str = "1791743580: Denied PUBLISH from suez-to-mqtt \
        (u'Suez', q=1, r=1, m=8, 'jeedom/data/water', ... (82 bytes))";
    const OLD_FORM: &str = "Client zigbee2mqtt is not authorised to subscribe to zigbee2mqtt/#";
    const NOT_A_DENIAL: &str = "1791743566: Received PUBLISH from suez-to-mqtt \
        (d0, q1, r1, m8, 'water/data', ... (420 bytes))";

    fn denial(action: &str, client: &str, topic: &str, user: Option<&str>) -> Denial {
        Denial {
            action: action.into(),
            client: client.into(),
            topic: topic.into(),
            user: user.map(str::to_owned),
        }
    }

    #[test]
    fn parse_recognises_the_current_mosquitto_shape() {
        assert_eq!(
            parse_denial(ATHENA_LINE),
            Some(denial("publish", "Athena", "athena/heartbeat", None))
        );
    }

    #[test]
    fn parse_captures_the_user_when_mosquitto_prints_it() {
        assert_eq!(
            parse_denial(SUEZ_LINE),
            Some(denial(
                "publish",
                "suez-to-mqtt",
                "jeedom/data/water",
                Some("Suez")
            ))
        );
    }

    #[test]
    fn parse_recognises_the_older_not_authorised_wording() {
        assert_eq!(
            parse_denial(OLD_FORM),
            Some(denial("subscribe", "zigbee2mqtt", "zigbee2mqtt/#", None))
        );
        assert_eq!(
            parse_denial("client X not authorized to unsubscribe to a/b"),
            Some(denial("unsubscribe", "X", "a/b", None))
        );
    }

    #[test]
    fn parse_returns_none_for_non_denial_lines() {
        assert_eq!(parse_denial(NOT_A_DENIAL), None);
        assert_eq!(parse_denial(""), None);
        assert_eq!(
            parse_denial("Denied PUBLISH from X (no quotes at all)"),
            None
        );
        assert_eq!(parse_denial("Denied CONNECT from X (d0, 'a/b')"), None);
    }

    #[test]
    fn parse_is_case_insensitive_on_the_keywords() {
        assert_eq!(
            parse_denial("denied publish FROM x (q0, 'a/b')"),
            Some(denial("publish", "x", "a/b", None))
        );
    }

    #[test]
    fn first_denial_produces_an_alert() {
        let alert = AclWatchdog::new(300)
            .on_line(ATHENA_LINE, 1000)
            .expect("alert");
        assert_eq!(alert["v"], 1);
        assert_eq!(alert["channel"], "system");
        assert_eq!(alert["severity"], "warning");
        assert_eq!(alert["title"], "MQTT ACL denial");
        assert_eq!(alert["body"], "Athena denied PUBLISH on athena/heartbeat");
        assert_eq!(alert["id"], "mqtt-acl-Athena-publish-1000");
        assert_eq!(alert["ttl"], 86400);
        assert_eq!(
            alert["loc"],
            json!({"key": "mqtt.acl_denied",
                   "args": {"client": "Athena", "action": "publish", "topic": "athena/heartbeat"}})
        );
    }

    #[test]
    fn body_includes_the_user_when_known() {
        let alert = AclWatchdog::new(300)
            .on_line(SUEZ_LINE, 1000)
            .expect("alert");
        assert_eq!(
            alert["body"],
            "suez-to-mqtt (u=Suez) denied PUBLISH on jeedom/data/water"
        );
        assert_eq!(alert["loc"]["args"]["user"], "Suez");
    }

    #[test]
    fn cooldown_suppresses_repeats_within_the_window() {
        let mut wd = AclWatchdog::new(300);
        assert!(wd.on_line(ATHENA_LINE, 1000).is_some());
        assert!(wd.on_line(ATHENA_LINE, 1010).is_none());
        assert!(wd.on_line(ATHENA_LINE, 1110).is_none());
    }

    #[test]
    fn alert_after_cooldown_names_the_suppressed_count() {
        let mut wd = AclWatchdog::new(60);
        assert!(wd.on_line(ATHENA_LINE, 1000).is_some());
        for t in 1001..=1003 {
            assert!(wd.on_line(ATHENA_LINE, t).is_none());
        }
        let alert = wd.on_line(ATHENA_LINE, 1063).expect("second window");
        assert!(
            alert["body"]
                .as_str()
                .expect("body")
                .ends_with("(+3 more in last 60s)")
        );
        assert_eq!(alert["loc"]["args"]["suppressed"], "3");
    }

    #[test]
    fn different_topics_and_actions_each_get_their_own_window() {
        let mut wd = AclWatchdog::new(300);
        let alerts: Vec<Value> = [ATHENA_LINE, SUEZ_LINE, OLD_FORM]
            .iter()
            .filter_map(|line| wd.on_line(line, 1000))
            .collect();
        assert_eq!(alerts.len(), 3);
        let ids: std::collections::HashSet<_> = alerts.iter().map(|a| a["id"].clone()).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn window_start_in_the_id_bumps_after_a_new_window() {
        let mut wd = AclWatchdog::new(60);
        let first = wd.on_line(ATHENA_LINE, 1000).expect("first");
        let second = wd.on_line(ATHENA_LINE, 1120).expect("second");
        assert_ne!(first["id"], second["id"]);
    }

    #[test]
    fn non_matching_log_lines_are_ignored() {
        let mut wd = AclWatchdog::new(300);
        assert!(wd.on_line(NOT_A_DENIAL, 1000).is_none());
        assert!(
            wd.on_line("not even utf-ascii-noise-\u{fffd}\u{fffd}", 1000)
                .is_none()
        );
    }

    #[test]
    fn the_window_table_stays_bounded() {
        let mut wd = AclWatchdog::new(300);
        for n in 0..(SEEN_CAP + 50) {
            let line = format!("Denied PUBLISH from c{n} (q0, 'a/b')");
            assert!(wd.on_line(&line, 1000).is_some());
        }
        assert_eq!(wd.windows.len(), SEEN_CAP);
    }
}
