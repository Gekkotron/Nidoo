//! Holding the music of some satellites while someone talks to the house.
//!
//! The hub sends `{"id", "players", "for_ms"}` on [`HOLD_TOPIC`] when a spoken exchange starts,
//! refreshes it while the exchange lasts and sends `for_ms: 0` when it is over. A held satellite
//! gets silence instead of music; speech is still mixed in. Every hold has a deadline, so a hub
//! that dies mid-exchange never leaves a room silent, and holds are never saved: a restart of
//! this service brings the music back at once.

use std::collections::HashMap;

use serde::Deserialize;

/// The topic the hub sends holds on; not retained.
pub const HOLD_TOPIC: &str = "nidoo-sound/hold";

/// The longest a hold may last without a refresh, in milliseconds.
pub const MAX_HOLD_MS: u32 = 60_000;

/// One hold from the hub: `players` (short ids) are held for `for_ms`; `0` releases `id`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HoldRequest {
    pub id: String,
    #[serde(default)]
    pub players: Vec<String>,
    pub for_ms: u32,
}

/// The request in a [`HOLD_TOPIC`] payload; `None` for bad JSON or an empty id.
pub fn parse_hold(payload: &[u8]) -> Option<HoldRequest> {
    let mut req: HoldRequest = serde_json::from_slice(payload).ok()?;
    if req.id.is_empty() {
        return None;
    }
    req.for_ms = req.for_ms.min(MAX_HOLD_MS);
    Some(req)
}

/// The live holds, by id; times on the server clock, in µs.
#[derive(Debug, Default)]
pub struct Holds {
    by_id: HashMap<String, (Vec<String>, i64)>,
}

impl Holds {
    /// Replaces the hold `req.id` (players and deadline), or removes it for `for_ms: 0`.
    pub fn apply(&mut self, req: HoldRequest, now_us: i64) {
        if req.for_ms == 0 {
            self.by_id.remove(&req.id);
            return;
        }
        let until = now_us + i64::from(req.for_ms) * 1_000;
        self.by_id.insert(req.id, (req.players, until));
    }

    /// Whether a live hold names `sid`.
    pub fn is_held(&self, sid: &str, now_us: i64) -> bool {
        self.by_id
            .values()
            .any(|(players, until)| now_us < *until && players.iter().any(|p| p == sid))
    }

    /// Drops the holds past their deadline; the satellites no live hold names any more.
    pub fn expire(&mut self, now_us: i64) -> Vec<String> {
        let mut dropped: Vec<String> = Vec::new();
        self.by_id.retain(|_, (players, until)| {
            let live = now_us < *until;
            if !live {
                dropped.extend(players.iter().cloned());
            }
            live
        });
        dropped.sort();
        dropped.dedup();
        dropped.retain(|sid| !self.is_held(sid, now_us));
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(id: &str, players: &[&str], for_ms: u32) -> HoldRequest {
        HoldRequest {
            id: id.into(),
            players: players.iter().map(|p| (*p).to_owned()).collect(),
            for_ms,
        }
    }

    #[test]
    fn parse_hold_reads_the_hub_message() {
        assert_eq!(
            parse_hold(br#"{"id":"x","players":["a"],"for_ms":30000}"#),
            Some(hold("x", &["a"], 30_000))
        );
        assert_eq!(
            parse_hold(br#"{"id":"x","players":["a"],"for_ms":90000}"#),
            Some(hold("x", &["a"], 60_000))
        );
        assert_eq!(parse_hold(br#"{"id":"","players":[],"for_ms":1}"#), None);
        assert_eq!(parse_hold(b"nope"), None);
    }

    #[test]
    fn release_frees_only_players_no_other_hold_names() {
        let mut h = Holds::default();
        h.apply(hold("x", &["a", "b"], 30_000), 0);
        h.apply(hold("y", &["b"], 30_000), 0);
        h.apply(hold("x", &[], 0), 1);
        assert!(!h.is_held("a", 1));
        assert!(h.is_held("b", 1));
    }

    #[test]
    fn hold_expires_without_refresh() {
        let mut h = Holds::default();
        h.apply(hold("x", &["a"], 30_000), 0);
        assert!(h.is_held("a", 29_990_000));
        assert!(!h.is_held("a", 30_000_000));
        assert_eq!(h.expire(30_000_000), vec!["a".to_owned()]);
        assert!(h.expire(30_000_000).is_empty());
    }

    #[test]
    fn refresh_extends_the_deadline() {
        let mut h = Holds::default();
        h.apply(hold("x", &["a"], 30_000), 0);
        h.apply(hold("x", &["a"], 30_000), 20_000_000);
        assert!(h.is_held("a", 40_000_000));
        assert!(h.expire(40_000_000).is_empty());
    }
}
