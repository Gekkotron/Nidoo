//! Registered FCM tokens, fed by the retained `tokens/<installId>` messages. Keyed by the
//! full topic, so a legacy and a new registration of one install are two entries.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenStatus {
    Registered,
    Updated,
    Removed,
    Noop,
    Ignored,
}

#[derive(Debug, Default)]
pub struct Tokens {
    /// Registration order; an update keeps its slot.
    entries: Vec<(String, String)>,
}

impl Tokens {
    /// Apply one retained token message: an empty payload unregisters.
    pub fn handle(&mut self, topic: &str, payload: &[u8]) -> TokenStatus {
        let text = String::from_utf8_lossy(payload);
        if text.trim().is_empty() {
            return if self.remove(topic) {
                TokenStatus::Removed
            } else {
                TokenStatus::Noop
            };
        }
        let token = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("token")?.as_str().map(|t| t.trim().to_string()))
            .filter(|t| !t.is_empty());
        let Some(token) = token else {
            return TokenStatus::Ignored;
        };
        match self.entries.iter_mut().find(|(t, _)| t == topic) {
            Some(slot) => {
                slot.1 = token;
                TokenStatus::Updated
            }
            None => {
                self.entries.push((topic.to_string(), token));
                TokenStatus::Registered
            }
        }
    }

    pub fn remove(&mut self, topic: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(t, _)| t != topic);
        self.entries.len() != before
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Unique tokens in registration order (two registrations sharing a token are sent to once).
    pub fn tokens(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, token) in &self.entries {
            if !out.contains(token) {
                out.push(token.clone());
            }
        }
        out
    }

    pub fn topics_for_token(&self, token: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, t)| t == token)
            .map(|(topic, _)| topic.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(token: &str) -> Vec<u8> {
        format!("{{\"token\":\"{token}\"}}").into_bytes()
    }

    #[test]
    fn register_new_install() {
        let mut r = Tokens::default();
        let payload =
            br#"{"token":"tok-a","device_name":"Pixel 8","lang":"fr","updated_at":1700000000}"#;
        assert_eq!(r.handle("t/a", payload), TokenStatus::Registered);
        assert_eq!(r.tokens(), ["tok-a"]);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn update_replaces_token_for_same_topic() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("old"));
        assert_eq!(r.handle("t/a", &reg("new")), TokenStatus::Updated);
        assert_eq!(r.tokens(), ["new"]);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn only_token_is_required_and_unknown_fields_are_ignored() {
        let mut r = Tokens::default();
        let p = br#"{"token":"t","whatever":[1,2],"other":{"x":1}}"#;
        assert_eq!(r.handle("t/a", p), TokenStatus::Registered);
    }

    #[test]
    fn token_is_trimmed() {
        let mut r = Tokens::default();
        r.handle("t/a", br#"{"token":"  t  "}"#);
        assert_eq!(r.tokens(), ["t"]);
    }

    #[test]
    fn empty_payload_unregisters() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("t"));
        assert_eq!(r.handle("t/a", b""), TokenStatus::Removed);
        assert!(r.tokens().is_empty());
    }

    #[test]
    fn empty_payload_for_unknown_topic_is_noop() {
        let mut r = Tokens::default();
        assert_eq!(r.handle("ghost", b""), TokenStatus::Noop);
        assert_eq!(r.handle("ghost", b"   "), TokenStatus::Noop);
    }

    #[test]
    fn bad_inputs_are_ignored_and_keep_existing_entry() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("keep"));
        let bad: [&[u8]; 10] = [
            b"not json",
            b"\xff\xfe",
            b"[1]",
            b"42",
            b"null",
            br#"{"device_name":"x"}"#,
            br#"{"token": ""}"#,
            br#"{"token": "  "}"#,
            br#"{"token": 5}"#,
            br#"{"token": null}"#,
        ];
        for b in bad {
            assert_eq!(r.handle("t/a", b), TokenStatus::Ignored, "{b:?}");
        }
        assert_eq!(r.tokens(), ["keep"]);
    }

    #[test]
    fn duplicate_tokens_are_listed_once() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("same"));
        r.handle("t/b", &reg("same"));
        r.handle("t/c", &reg("other"));
        assert_eq!(r.tokens(), ["same", "other"]);
        assert_eq!(r.topics_for_token("same"), ["t/a", "t/b"]);
        assert!(r.topics_for_token("nope").is_empty());
    }

    #[test]
    fn tokens_on_two_topics_with_one_token_yield_it_once() {
        let mut r = Tokens::default();
        r.handle("oikos/push/tokens/a", &reg("t1"));
        r.handle("oikos-push/tokens/a", &reg("t1"));
        assert_eq!(r.tokens(), ["t1"]);
        assert_eq!(r.topics_for_token("t1").len(), 2);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn registration_order_survives_an_update() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("a"));
        r.handle("t/b", &reg("b"));
        r.handle("t/a", &reg("a2"));
        assert_eq!(r.tokens(), ["a2", "b"]);
    }

    #[test]
    fn remove_and_clear() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("a"));
        r.handle("t/b", &reg("b"));
        assert!(r.remove("t/a"));
        assert!(!r.remove("t/a"));
        r.clear();
        assert_eq!(r.len(), 0);
        assert!(r.tokens().is_empty());
    }
}
