//! Pure, I/O-free rewrite rules for the switch-over.

use serde_json::Value;

/// One renamed string or object key inside a JSON payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// RFC 6901 pointer built from the original (pre-rename) keys.
    pub path: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retained {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Move {
        from: String,
        to: Retained,
        changes: Vec<Change>,
    },
    Drop {
        topic: String,
        reason: &'static str,
    },
    Refuse {
        topic: String,
        reason: String,
    },
}

/// Replaces `oikos`, `Oikos` and `OIKOS`; other casings are left alone.
pub fn rename_text(s: &str) -> String {
    s.replace("oikos", "nidoo")
        .replace("Oikos", "Nidoo")
        .replace("OIKOS", "NIDOO")
}

/// A leading `oikos/` and every segment starting with `oikos_` are renamed.
pub fn rename_topic(topic: &str) -> String {
    topic
        .split('/')
        .enumerate()
        .map(|(i, seg)| {
            if i == 0 && seg == "oikos" {
                "nidoo".to_string()
            } else if let Some(rest) = seg.strip_prefix("oikos_") {
                format!("nidoo_{rest}")
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Applies `rename_text` to every string value and object key.
pub fn rename_json(v: Value) -> (Value, Vec<Change>) {
    let mut changes = Vec::new();
    let out = walk(v, "", &mut changes);
    (out, changes)
}

fn walk(v: Value, path: &str, changes: &mut Vec<Change>) -> Value {
    match v {
        Value::String(s) => {
            let new = rename_text(&s);
            if new != s {
                changes.push(Change {
                    path: path.to_string(),
                    old: s,
                    new: new.clone(),
                });
            }
            Value::String(new)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(i, item)| walk(item, &format!("{path}/{i}"), changes))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, item) in map {
                let child = format!("{path}/{}", escape(&k));
                let new_key = rename_text(&k);
                if new_key != k {
                    changes.push(Change {
                        path: child.clone(),
                        old: k,
                        new: new_key.clone(),
                    });
                }
                out.insert(new_key, walk(item, &child, changes));
            }
            Value::Object(out)
        }
        other => other,
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn payload_mentions_oikos(p: &[u8]) -> bool {
    [&b"oikos"[..], b"Oikos", b"OIKOS"]
        .iter()
        .any(|n| contains(p, n))
}

fn topic_selected(topic: &str) -> bool {
    topic.starts_with("oikos/")
        || topic.starts_with("oikos-push/")
        || topic.split('/').any(|s| s.starts_with("oikos_"))
}

/// The backup entries `--restore` republishes: the old names' topics (the `oikos/` and
/// `oikos-push/` prefixes, `oikos_` segments) and the new `nidoo/` prefix. Everything else on
/// the broker (Zigbee2MQTT, cameras, discovery the hub does not own) is left alone, so a
/// rollback never replays it to live subscribers.
pub fn restorable(topic: &str) -> bool {
    topic_selected(topic) || topic.starts_with("nidoo/")
}

/// Longer values than this are never printed: a service-account key is one long string.
pub const SHOWN_MAX: usize = 200;

/// `s` quoted for printing, or `<redacted, N bytes>` when it may be a secret: it holds a
/// private key, or it is longer than [`SHOWN_MAX`] bytes.
pub fn shown(s: &str) -> String {
    if s.contains("PRIVATE KEY") || s.len() > SHOWN_MAX {
        format!("<redacted, {} bytes>", s.len())
    } else {
        format!("{s:?}")
    }
}

/// One change as the dry run prints it, redacted by [`shown`].
pub fn change_line(c: &Change) -> String {
    format!("{}: {} -> {}", c.path, shown(&c.old), shown(&c.new))
}

/// True when the move's target topic already holds other bytes than the ones it would get:
/// the move overwrites a newer copy (the new hub ran before the switch-over).
pub fn overwrites_existing(msgs: &[Retained], from: &str, to: &Retained) -> bool {
    from != to.topic
        && msgs
            .iter()
            .any(|m| m.topic == to.topic && m.payload != to.payload)
}

/// Decides what to do with each retained message that mentions `oikos`.
pub fn plan(retained: &[Retained]) -> Vec<Action> {
    let mut actions = Vec::new();
    for r in retained {
        if !(topic_selected(&r.topic) || payload_mentions_oikos(&r.payload)) {
            continue;
        }
        if r.topic.starts_with("oikos/push/tokens/") {
            actions.push(Action::Drop {
                topic: r.topic.clone(),
                reason: "old Firebase token",
            });
            continue;
        }
        if r.topic.starts_with("oikos-push/") {
            actions.push(Action::Drop {
                topic: r.topic.clone(),
                reason: "legacy push topic",
            });
            continue;
        }
        let (payload, changes) = match std::str::from_utf8(&r.payload) {
            Err(_) => {
                if payload_mentions_oikos(&r.payload) {
                    actions.push(Action::Refuse {
                        topic: r.topic.clone(),
                        reason: "non-UTF-8 payload contains oikos".to_string(),
                    });
                    continue;
                }
                (r.payload.clone(), Vec::new())
            }
            Ok(text) => match serde_json::from_slice::<Value>(&r.payload) {
                Ok(v) => {
                    let (new, changes) = rename_json(v);
                    if changes.is_empty() {
                        (r.payload.clone(), changes)
                    } else {
                        // Value serialisation to bytes cannot fail.
                        (serde_json::to_vec(&new).unwrap_or_default(), changes)
                    }
                }
                Err(_) => (rename_text(text).into_bytes(), Vec::new()),
            },
        };
        actions.push(Action::Move {
            from: r.topic.clone(),
            to: Retained {
                topic: rename_topic(&r.topic),
                payload,
                qos: r.qos,
            },
            changes,
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(topic: &str, payload: &[u8]) -> Retained {
        Retained {
            topic: topic.to_string(),
            payload: payload.to_vec(),
            qos: 1,
        }
    }

    #[test]
    fn rename_topic_prefix_and_segments() {
        assert_eq!(
            rename_topic("oikos/ui/oikos_alarm_house"),
            "nidoo/ui/nidoo_alarm_house"
        );
        assert_eq!(
            rename_topic("homeassistant/device_tracker/oikos_phone/config"),
            "homeassistant/device_tracker/nidoo_phone/config"
        );
        assert_eq!(rename_topic("zigbee2mqtt/lamp"), "zigbee2mqtt/lamp");
    }

    #[test]
    fn rewrites_case_variants_everywhere() {
        let v = json!({
            "name": "Oikos alarm",
            "unique_id": "oikos_alarm_house",
            "device": {"identifiers": ["oikos_alarm"]},
            "stat_t": "oikos/alarm/panel",
            "OIKOS_KEY": 1
        });
        let (out, changes) = rename_json(v);
        assert_eq!(
            out,
            json!({
                "name": "Nidoo alarm",
                "unique_id": "nidoo_alarm_house",
                "device": {"identifiers": ["nidoo_alarm"]},
                "stat_t": "nidoo/alarm/panel",
                "NIDOO_KEY": 1
            })
        );
        let mut paths: Vec<_> = changes.iter().map(|c| c.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            [
                "/OIKOS_KEY",
                "/device/identifiers/0",
                "/name",
                "/stat_t",
                "/unique_id"
            ]
        );
        let key = changes.iter().find(|c| c.path == "/OIKOS_KEY").unwrap();
        assert_eq!(
            (key.old.as_str(), key.new.as_str()),
            ("OIKOS_KEY", "NIDOO_KEY")
        );
    }

    #[test]
    fn plan_drops_tokens_and_legacy() {
        let actions = plan(&[
            msg("oikos/push/tokens/abc", b"{}"),
            msg("oikos-push/history", b"[]"),
        ]);
        assert_eq!(
            actions,
            vec![
                Action::Drop {
                    topic: "oikos/push/tokens/abc".into(),
                    reason: "old Firebase token"
                },
                Action::Drop {
                    topic: "oikos-push/history".into(),
                    reason: "legacy push topic"
                },
            ]
        );
    }

    #[test]
    fn plan_moves_flow_with_id_refs() {
        let payload = br#"{"id":"f1","nodes":[{"entity":"oikos_alarm_house"}]}"#;
        let actions = plan(&[msg("oikos/automation/f1", payload)]);
        assert_eq!(actions.len(), 1);
        let Action::Move { from, to, changes } = &actions[0] else {
            panic!("expected Move");
        };
        assert_eq!(from, "oikos/automation/f1");
        assert_eq!(to.topic, "nidoo/automation/f1");
        assert_eq!(to.qos, 1);
        let v: Value = serde_json::from_slice(&to.payload).unwrap();
        assert_eq!(v["nodes"][0]["entity"], "nidoo_alarm_house");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "/nodes/0/entity");
    }

    #[test]
    fn plan_rewrites_in_place_outside_namespace() {
        let topic = "homeassistant/alarm_control_panel/alarm/house/config";
        let payload = br#"{"command_topic":"oikos/alarm/set","state_topic":"oikos/alarm/state"}"#;
        let actions = plan(&[msg(topic, payload)]);
        assert_eq!(actions.len(), 1);
        let Action::Move { from, to, .. } = &actions[0] else {
            panic!("expected Move");
        };
        assert_eq!(from, topic);
        assert_eq!(to.topic, topic);
        let v: Value = serde_json::from_slice(&to.payload).unwrap();
        assert_eq!(v["command_topic"], "nidoo/alarm/set");
        assert_eq!(v["state_topic"], "nidoo/alarm/state");
    }

    #[test]
    fn plan_ignores_clean_messages() {
        assert!(plan(&[msg("zigbee2mqtt/lamp", br#"{"state":"ON"}"#)]).is_empty());
    }

    #[test]
    fn binary_payload_untouched() {
        let actions = plan(&[msg("oikos/x", &[0xff, 0x00])]);
        let Action::Move { to, changes, .. } = &actions[0] else {
            panic!("expected Move");
        };
        assert_eq!(to.topic, "nidoo/x");
        assert_eq!(to.payload, vec![0xff, 0x00]);
        assert!(changes.is_empty());
    }

    #[test]
    fn secrets_are_redacted_but_the_path_is_kept() {
        let key = "{\"project_id\":\"oikos-1\",\"private_key\":\"-----BEGIN PRIVATE KEY-----\\nMIIE\\n-----END PRIVATE KEY-----\\n\"}";
        let c = Change {
            path: "/service_account".into(),
            old: key.into(),
            new: rename_text(key),
        };
        let line = change_line(&c);
        assert_eq!(
            line,
            format!(
                "/service_account: <redacted, {0} bytes> -> <redacted, {0} bytes>",
                key.len()
            )
        );
        assert!(!line.contains("MIIE"));
        // A long value without a key marker is redacted too; a short one is shown.
        let long = "x".repeat(SHOWN_MAX + 1);
        assert_eq!(shown(&long), format!("<redacted, {} bytes>", SHOWN_MAX + 1));
        assert_eq!(
            shown(&"x".repeat(SHOWN_MAX)),
            format!("{:?}", "x".repeat(SHOWN_MAX))
        );
        assert_eq!(
            change_line(&Change {
                path: "/p".into(),
                old: "oikos/ui".into(),
                new: "nidoo/ui".into(),
            }),
            r#"/p: "oikos/ui" -> "nidoo/ui""#
        );
    }

    #[test]
    fn overwrite_of_a_different_target_is_flagged() {
        let to = msg("nidoo/ui/a", br#"{"name":"x"}"#);
        let same = [msg("nidoo/ui/a", br#"{"name":"x"}"#)];
        let other = [msg("nidoo/ui/a", br#"{"name":"newer"}"#)];
        assert!(!overwrites_existing(&[], "oikos/ui/a", &to));
        assert!(!overwrites_existing(&same, "oikos/ui/a", &to));
        assert!(overwrites_existing(&other, "oikos/ui/a", &to));
        // An in-place rewrite replaces its own topic by design.
        let in_place = msg("oikos/ui/a", b"new");
        assert!(!overwrites_existing(
            &[msg("oikos/ui/a", b"old")],
            "oikos/ui/a",
            &in_place
        ));
    }

    #[test]
    fn restore_keeps_to_the_old_and_new_names() {
        for t in [
            "oikos/ui/a",
            "oikos-push/history",
            "homeassistant/device_tracker/oikos_phone/config",
            "nidoo/ui/a",
        ] {
            assert!(restorable(t), "{t}");
        }
        for t in [
            "zigbee2mqtt/lamp",
            "frigate/events",
            "homeassistant/light/x/config",
        ] {
            assert!(!restorable(t), "{t}");
        }
    }

    #[test]
    fn binary_payload_with_oikos_refused() {
        let actions = plan(&[msg("oikos/x", b"\xffoikos")]);
        assert!(matches!(&actions[0], Action::Refuse { topic, .. } if topic == "oikos/x"));
    }
}
