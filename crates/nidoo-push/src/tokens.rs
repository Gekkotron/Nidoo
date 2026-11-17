//! Registered FCM tokens, fed by the retained `tokens/<installId>` messages. Keyed by the
//! full topic, so two registrations of one install are two entries.

use serde_json::Value;

/// Installs registered at once. A house has a handful of phones; a full list refuses new
/// installs (an install already listed can still update its token).
pub const MAX_INSTALLS: usize = 32;
/// An FCM token is ~160 characters; anything much longer is not one.
pub const MAX_TOKEN_LEN: usize = 1024;
/// A registration is `{token, device_name, lang, updated_at}`: a few hundred bytes.
pub const MAX_PAYLOAD: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenStatus {
    Registered,
    Updated,
    Removed,
    Noop,
    Ignored,
    /// A new install while `MAX_INSTALLS` are registered: refused.
    Full,
}

#[derive(Debug, Default)]
pub struct Tokens {
    /// Registration order; an update keeps its slot.
    entries: Vec<(String, String)>,
}

impl Tokens {
    /// Apply one retained token message: an empty payload unregisters.
    pub fn handle(&mut self, topic: &str, payload: &[u8]) -> TokenStatus {
        if payload.len() > MAX_PAYLOAD {
            return TokenStatus::Ignored;
        }
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
            .filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN_LEN);
        let Some(token) = token else {
            return TokenStatus::Ignored;
        };
        let full = self.entries.len() >= MAX_INSTALLS;
        match self.entries.iter_mut().find(|(t, _)| t == topic) {
            Some(slot) => {
                slot.1 = token;
                TokenStatus::Updated
            }
            None if full => TokenStatus::Full,
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
        r.handle("nidoo/push/tokens/a", &reg("t1"));
        r.handle("other/push/tokens/a", &reg("t1"));
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
    fn the_list_is_capped_but_listed_installs_still_update() {
        let mut r = Tokens::default();
        for i in 0..MAX_INSTALLS {
            assert_eq!(
                r.handle(&format!("t/{i}"), &reg(&format!("k{i}"))),
                TokenStatus::Registered
            );
        }
        assert_eq!(r.handle("t/extra", &reg("x")), TokenStatus::Full);
        assert_eq!(r.len(), MAX_INSTALLS);
        assert!(!r.tokens().contains(&"x".to_string()));
        assert_eq!(r.handle("t/0", &reg("k0-new")), TokenStatus::Updated);
        // Unregistering frees a slot.
        assert_eq!(r.handle("t/1", b""), TokenStatus::Removed);
        assert_eq!(r.handle("t/extra", &reg("x")), TokenStatus::Registered);
    }

    #[test]
    fn oversized_tokens_and_payloads_are_ignored() {
        let mut r = Tokens::default();
        r.handle("t/a", &reg("keep"));
        let long = "k".repeat(MAX_TOKEN_LEN + 1);
        assert_eq!(r.handle("t/a", &reg(&long)), TokenStatus::Ignored);
        let padded = format!(
            "{{\"token\":\"ok\",\"device_name\":\"{}\"}}",
            "x".repeat(MAX_PAYLOAD)
        );
        assert_eq!(r.handle("t/a", padded.as_bytes()), TokenStatus::Ignored);
        let blank = " ".repeat(MAX_PAYLOAD + 1);
        assert_eq!(r.handle("t/a", blank.as_bytes()), TokenStatus::Ignored);
        assert_eq!(r.tokens(), ["keep"]);
        let max = "k".repeat(MAX_TOKEN_LEN);
        assert_eq!(r.handle("t/a", &reg(&max)), TokenStatus::Updated);
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
