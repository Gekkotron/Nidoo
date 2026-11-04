//! Replay protection for the voice packets (see `docs/reference/voice-protocol.md`).
//!
//! A board has no clock and its counters start again at each boot, so the hub hands it a
//! fresh nonce in every hello: a wake echoes the latest nonce with a session number that
//! rises during one boot. The hub's own counter rises across hub restarts (it follows the
//! wall clock), so a board, which only takes rising hub counters, keeps listening to it.

use std::collections::{HashMap, VecDeque};

/// How many of its latest hello nonces the hub still accepts in a wake.
pub const RECENT_NONCES: usize = 3;

/// One satellite's recent hello nonces and the sessions already opened under each.
#[derive(Debug, Default)]
pub struct ReplayGuard {
    nonces: VecDeque<u64>,
    sessions: HashMap<u64, u32>,
}

impl ReplayGuard {
    /// A hello with `nonce` went to the board.
    pub fn note_hello(&mut self, nonce: u64) {
        self.nonces.push_back(nonce);
        while self.nonces.len() > RECENT_NONCES {
            if let Some(old) = self.nonces.pop_front() {
                self.sessions.remove(&old);
            }
        }
    }

    /// Whether a wake with `nonce` and `session` may open a session: the nonce is recent and
    /// the session is above every session already opened under it.
    pub fn accept_wake(&mut self, nonce: u64, session: u32) -> bool {
        if !self.nonces.contains(&nonce) {
            return false;
        }
        // Sessions start at 1: 0 is never accepted.
        let last = self.sessions.entry(nonce).or_insert(0);
        if session <= *last {
            return false;
        }
        *last = session;
        true
    }
}

/// The hub's message counter: never twice the same, rising across restarts.
#[derive(Debug, Default)]
pub struct HubCounter {
    last: u64,
}

impl HubCounter {
    /// The next counter for a message sent at `unix_us`.
    pub fn next(&mut self, unix_us: u64) -> u64 {
        self.last = unix_us.max(self.last + 1);
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replayed_wake_is_refused() {
        let mut g = ReplayGuard::default();
        g.note_hello(7);
        assert!(g.accept_wake(7, 1));
        assert!(!g.accept_wake(7, 1), "the same wake again");
        assert!(!g.accept_wake(7, 0), "an older session");
        assert!(g.accept_wake(7, 2));
    }

    #[test]
    fn old_nonce_is_refused() {
        let mut g = ReplayGuard::default();
        for nonce in 1..=4 {
            g.note_hello(nonce);
        }
        assert!(!g.accept_wake(1, 1), "four hellos ago");
        assert!(g.accept_wake(2, 1));
        assert!(!g.accept_wake(99, 1), "never sent");
    }

    #[test]
    fn rebooted_board_with_a_fresh_nonce_starts_again_at_session_1() {
        let mut g = ReplayGuard::default();
        g.note_hello(1);
        assert!(g.accept_wake(1, 5));
        g.note_hello(2);
        assert!(g.accept_wake(2, 1));
        assert!(!g.accept_wake(1, 5));
    }

    #[test]
    fn hub_counter_rises_even_if_the_clock_does_not() {
        let mut c = HubCounter::default();
        assert_eq!(c.next(1_000), 1_000);
        assert_eq!(c.next(1_000), 1_001);
        assert_eq!(c.next(10), 1_002, "the clock went back");
        assert_eq!(c.next(5_000), 5_000);
    }
}
