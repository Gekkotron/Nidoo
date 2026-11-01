//! Catch-up buffer: recent normalized alerts, newest first. Persistence is the broker's
//! retained `history` topic, so this type only holds the list; the clock is a `now` argument.

use serde_json::{Map, Value};

use crate::normalize::Alert;

const DAY_SECONDS: u64 = 86_400;

/// Same keys as the FCM data map, but typed JSON values.
pub fn to_history_entry(alert: &Alert) -> Value {
    let mut entry = Map::new();
    entry.insert("v".into(), 1.into());
    entry.insert("id".into(), alert.id.clone().into());
    entry.insert("channel".into(), alert.channel.clone().into());
    entry.insert("severity".into(), alert.severity.clone().into());
    entry.insert("title".into(), alert.title.clone().into());
    entry.insert("body".into(), alert.body.clone().into());
    entry.insert(
        "actions".into(),
        serde_json::to_value(&alert.actions).unwrap_or_else(|_| Value::Array(Vec::new())),
    );
    entry.insert("silent".into(), alert.silent.into());
    entry.insert("sent_at".into(), alert.sent_at.into());
    entry.insert("source".into(), alert.source.clone().into());
    for (key, value) in [
        ("image_url", &alert.image_url),
        ("video_url", &alert.video_url),
        ("entity_id", &alert.entity_id),
        ("camera", &alert.camera),
    ] {
        if let Some(v) = value {
            entry.insert(key.into(), v.clone().into());
        }
    }
    if let Some(loc) = &alert.loc {
        entry.insert("loc".into(), loc.clone());
    }
    if alert.cancel {
        entry.insert("cancel".into(), true.into());
    }
    Value::Object(entry)
}

#[derive(Debug)]
pub struct History {
    size: usize,
    days: u64,
    entries: Vec<Value>,
}

impl History {
    pub fn new(size: usize, days: u64) -> Self {
        Self {
            size,
            days,
            entries: Vec::new(),
        }
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e["id"] == id)
    }

    pub fn apply(&mut self, alert: &Alert, now: u64) {
        let index = self.position(&alert.id);
        match (alert.cancel, index) {
            // A cancel resolves the alert in place.
            (true, Some(i)) => {
                if let Some(obj) = self.entries[i].as_object_mut() {
                    obj.insert("cancel".into(), true.into());
                    obj.insert("resolved_at".into(), alert.sent_at.into());
                }
            }
            (_, found) => {
                if let (false, Some(i)) = (alert.cancel, found) {
                    self.entries.remove(i);
                }
                self.entries.insert(0, to_history_entry(alert));
            }
        }
        self.trim(now);
    }

    /// Drop entries older than `days` (by `sent_at`), then cap the list at `size`.
    pub fn trim(&mut self, now: u64) {
        let cutoff = now.saturating_sub(self.days.saturating_mul(DAY_SECONDS)) as f64;
        self.entries
            .retain(|e| e["sent_at"].as_f64().is_some_and(|t| t >= cutoff));
        self.entries.truncate(self.size);
    }

    pub fn entries(&self) -> &[Value] {
        &self.entries
    }

    /// Compact JSON array; non-ASCII text stays unescaped.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.entries).unwrap_or_else(|_| "[]".into())
    }

    /// Replace the list from a retained payload, keeping only objects with a string `id`.
    /// Returns false (and keeps the list) when the payload is not a JSON array. The caller
    /// trims afterwards, since only it knows the time.
    pub fn replace_from_json(&mut self, payload: &[u8]) -> bool {
        let Ok(Value::Array(items)) = serde_json::from_slice::<Value>(payload) else {
            return false;
        };
        self.entries = items
            .into_iter()
            .filter(|e| e.is_object() && e["id"].is_string())
            .collect();
        true
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::{Action, Alert};
    use serde_json::json;

    const NOW: u64 = 1_700_000_000;
    const DAY: u64 = 86_400;

    fn mk(id: &str, sent_at: u64, f: impl FnOnce(&mut Alert)) -> Alert {
        let mut a = Alert {
            id: id.into(),
            channel: "camera".into(),
            body: "b".into(),
            title: format!("title-{id}"),
            sent_at,
            source: "src".into(),
            ..Alert::default()
        };
        f(&mut a);
        a
    }

    fn cancel(id: &str) -> Alert {
        Alert {
            id: id.into(),
            cancel: true,
            sent_at: NOW,
            source: "src".into(),
            ..Alert::default()
        }
    }

    fn ids(h: &History) -> Vec<String> {
        h.entries()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn filled(size: usize, days: u64, names: &str) -> History {
        let mut h = History::new(size, days);
        for n in names.chars() {
            h.apply(&mk(&n.to_string(), NOW, |_| {}), NOW);
        }
        h
    }

    #[test]
    fn new_alerts_are_prepended_newest_first() {
        assert_eq!(ids(&filled(100, 7, "abc")), ["c", "b", "a"]);
    }

    #[test]
    fn same_id_replaces_and_moves_to_front() {
        let mut h = filled(100, 7, "abc");
        h.apply(&mk("a", NOW, |a| a.title = "updated".into()), NOW);
        assert_eq!(ids(&h), ["a", "c", "b"]);
        assert_eq!(h.entries().len(), 3);
        assert_eq!(h.entries()[0]["title"], "updated");
    }

    #[test]
    fn cancel_marks_existing_alert_resolved() {
        let mut h = filled(100, 7, "a");
        h.apply(&cancel("a"), NOW);
        assert_eq!(h.entries().len(), 1);
        assert_eq!(h.entries()[0]["cancel"], true);
        assert_eq!(h.entries()[0]["resolved_at"], NOW);
        assert_eq!(h.entries()[0]["title"], "title-a");
    }

    #[test]
    fn cancel_for_unknown_id_is_stored_alone() {
        let mut h = filled(100, 7, "a");
        h.apply(&cancel("ghost"), NOW);
        assert_eq!(ids(&h), ["ghost", "a"]);
        let ghost = &h.entries()[0];
        assert_eq!(ghost["cancel"], true);
        assert_eq!(ghost["title"], "");
        assert_eq!(ghost["body"], "");
    }

    #[test]
    fn cancel_keeps_position_and_reposting_clears_resolved() {
        let mut h = filled(100, 7, "abc");
        h.apply(&cancel("a"), NOW);
        assert_eq!(ids(&h), ["c", "b", "a"]);
        assert_eq!(h.entries()[2]["cancel"], true);
        assert_eq!(h.entries()[2]["resolved_at"], NOW);
        h.apply(&mk("a", NOW, |a| a.title = "again".into()), NOW);
        assert_eq!(ids(&h), ["a", "c", "b"]);
        assert!(h.entries()[0].get("cancel").is_none());
        assert!(h.entries()[0].get("resolved_at").is_none());
        assert_eq!(h.entries()[0]["title"], "again");
    }

    #[test]
    fn size_cap_keeps_newest() {
        assert_eq!(ids(&filled(3, 7, "abcde")), ["e", "d", "c"]);
    }

    #[test]
    fn day_cap_drops_old_entries_by_sent_at() {
        let mut h = History::new(100, 7);
        h.apply(&mk("fresh", NOW - DAY, |_| {}), NOW);
        h.apply(&mk("edge", NOW - 7 * DAY, |_| {}), NOW);
        h.apply(&mk("old", NOW - 7 * DAY - 1, |_| {}), NOW);
        let mut got = ids(&h);
        got.sort();
        assert_eq!(got, ["edge", "fresh"]);
    }

    #[test]
    fn trim_drops_stale_entries_as_time_passes() {
        let mut h = filled(100, 7, "a");
        h.trim(NOW + 8 * DAY);
        assert!(h.is_empty());
    }

    #[test]
    fn replace_from_json_round_trips_filters_and_trim_cleans_up() {
        let h = filled(100, 7, "ab");
        let mut other = History::new(100, 7);
        assert!(other.replace_from_json(h.to_json().as_bytes()));
        assert_eq!(other.entries(), h.entries());

        let old = to_history_entry(&mk("old", NOW - 30 * DAY, |_| {}));
        let fresh = to_history_entry(&mk("fresh", NOW, |_| {}));
        let raw = serde_json::to_vec(&json!([fresh, old, 1, "x", {"id": 5}])).unwrap();
        assert!(other.replace_from_json(&raw));
        other.trim(NOW);
        assert_eq!(ids(&other), ["fresh"]);
    }

    #[test]
    fn replace_from_json_rejects_non_array_and_keeps_list() {
        let mut h = filled(100, 7, "ab");
        assert!(!h.replace_from_json(b"{\"a\": 1}"));
        assert!(!h.replace_from_json(b"{not json"));
        assert_eq!(ids(&h), ["b", "a"]);
    }

    #[test]
    fn entry_has_typed_json_values() {
        let entry = to_history_entry(&Alert {
            id: "a".into(),
            channel: "alarm".into(),
            severity: "critical".into(),
            title: "T".into(),
            body: "B".into(),
            image_url: Some("http://i".into()),
            video_url: Some("http://v".into()),
            entity_id: Some("e.x".into()),
            camera: Some("front_door".into()),
            actions: vec![Action {
                id: "go".into(),
                label: "Go".into(),
            }],
            silent: true,
            sent_at: NOW,
            source: "s".into(),
            ..Alert::default()
        });
        assert_eq!(
            entry,
            json!({
                "v": 1, "id": "a", "channel": "alarm", "severity": "critical",
                "title": "T", "body": "B", "image_url": "http://i",
                "video_url": "http://v", "entity_id": "e.x", "camera": "front_door",
                "actions": [{"id": "go", "label": "Go"}], "silent": true,
                "sent_at": NOW, "source": "s"
            })
        );
    }

    #[test]
    fn entry_omits_absent_optionals_and_flags_cancel() {
        let entry = to_history_entry(&Alert {
            id: "a".into(),
            sent_at: NOW,
            source: "s".into(),
            ..Alert::default()
        });
        for k in [
            "image_url",
            "video_url",
            "entity_id",
            "camera",
            "cancel",
            "loc",
        ] {
            assert!(entry.get(k).is_none(), "{k}");
        }
        assert_eq!(entry["actions"], json!([]));
        assert_eq!(entry["silent"], false);
        assert_eq!(to_history_entry(&cancel("a"))["cancel"], true);
    }

    #[test]
    fn entry_keeps_loc_as_an_object() {
        let loc = json!({"key": "frigate.object", "args": {"label": "car"}});
        let with = mk("a1", NOW, |a| a.loc = Some(loc.clone()));
        assert_eq!(to_history_entry(&with)["loc"], loc);
        assert!(
            to_history_entry(&mk("a2", NOW, |_| {}))
                .get("loc")
                .is_none()
        );
    }

    #[test]
    fn to_json_is_a_compact_array_with_unescaped_unicode() {
        let mut h = History::new(100, 7);
        h.apply(&mk("a", NOW, |a| a.title = "Désarmé".into()), NOW);
        let raw = h.to_json();
        assert!(raw.contains("Désarmé") && !raw.contains(", "));
        let back: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, h.entries());
    }
}
