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
mod tests;
