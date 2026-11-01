//! The verdict of one watch from its pings: connected on the first reply, disconnected after
//! `MISSES` unanswered pings in a row. Not being able to ping changes only availability.

use std::time::Duration;

use nidoo_api::{WatchState, WatchStatus};

/// Unanswered pings in a row before the device counts as disconnected.
pub const MISSES: u32 = 3;

/// The outcome of one ping.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    /// The device answered after this round trip.
    Reply(Duration),
    /// No answer in time.
    Miss,
    /// The hub could not send the ping, with the reason.
    CannotPing(String),
}

/// What changed and must be published; each field is `Some` only on a change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The new connected value.
    pub value: Option<bool>,
    /// The new availability.
    pub available: Option<bool>,
}

/// The running state of one watch.
#[derive(Debug, Default)]
pub struct Tracker {
    misses: u32,
    value: Option<bool>,
    available: Option<bool>,
    cannot_ping: bool,
    since: Option<u64>,
    last_reply: Option<u64>,
    rtt: Option<Duration>,
}

impl Tracker {
    pub fn new() -> Tracker {
        Tracker::default()
    }

    /// Folds one ping outcome in at Unix second `now`.
    pub fn on_probe(&mut self, probe: &Probe, now: u64) -> Verdict {
        let mut value = None;
        let mut available = None;
        match probe {
            Probe::Reply(rtt) => {
                self.cannot_ping = false;
                self.misses = 0;
                self.last_reply = Some(now);
                self.rtt = Some(*rtt);
                value = self.set_value(true, now);
            }
            Probe::Miss => {
                self.cannot_ping = false;
                self.misses += 1;
                if self.misses >= MISSES {
                    value = self.set_value(false, now);
                }
            }
            Probe::CannotPing(_) => {
                self.cannot_ping = true;
                self.misses = 0;
                if self.available != Some(false) {
                    self.available = Some(false);
                    available = Some(false);
                }
            }
        }
        if !self.cannot_ping && self.value.is_some() && self.available != Some(true) {
            self.available = Some(true);
            available = Some(true);
        }
        Verdict { value, available }
    }

    fn set_value(&mut self, value: bool, now: u64) -> Option<bool> {
        if self.value == Some(value) {
            return None;
        }
        self.value = Some(value);
        self.since = Some(now);
        Some(value)
    }

    /// The current connected value; `None` before the first verdict.
    pub fn value(&self) -> Option<bool> {
        self.value
    }

    /// The current availability; `None` before the first verdict.
    pub fn available(&self) -> Option<bool> {
        self.available
    }

    /// The watch's status; a paused watch is reported by the caller.
    pub fn status(&self) -> WatchStatus {
        let state = if self.cannot_ping {
            WatchState::CannotPing
        } else {
            match self.value {
                None => WatchState::Waiting,
                Some(true) => WatchState::Connected,
                Some(false) => WatchState::Disconnected,
            }
        };
        WatchStatus {
            state,
            since: self.since,
            last_reply: self.last_reply,
            rtt_ms: self.rtt.map(|rtt| rtt.as_secs_f64() * 1000.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: Probe = Probe::Reply(Duration::from_millis(4));

    #[test]
    fn waits_until_first_verdict() {
        assert_eq!(Tracker::new().status().state, WatchState::Waiting);
    }

    #[test]
    fn first_reply_connects() {
        let mut t = Tracker::new();
        let v = t.on_probe(&REPLY, 1);
        assert_eq!(v.value, Some(true));
        assert_eq!(v.available, Some(true));
        assert_eq!(t.status().state, WatchState::Connected);
    }

    #[test]
    fn two_misses_change_nothing_third_disconnects() {
        let mut t = Tracker::new();
        t.on_probe(&REPLY, 1);
        assert_eq!(t.on_probe(&Probe::Miss, 2).value, None);
        assert_eq!(t.on_probe(&Probe::Miss, 3).value, None);
        assert_eq!(t.on_probe(&Probe::Miss, 4).value, Some(false));
        assert_eq!(t.status().state, WatchState::Disconnected);
        assert_eq!(t.status().since, Some(4));
    }

    #[test]
    fn reply_resets_miss_count() {
        let mut t = Tracker::new();
        for probe in [Probe::Miss, Probe::Miss, REPLY, Probe::Miss, Probe::Miss] {
            assert_ne!(t.on_probe(&probe, 1).value, Some(false));
        }
    }

    #[test]
    fn cannot_ping_only_flips_availability() {
        let mut t = Tracker::new();
        t.on_probe(&REPLY, 1);
        let v = t.on_probe(&Probe::CannotPing("no ping".into()), 2);
        assert_eq!(
            v,
            Verdict {
                value: None,
                available: Some(false)
            }
        );
        assert_eq!(t.status().state, WatchState::CannotPing);
        let v = t.on_probe(&REPLY, 3);
        assert_eq!(
            v,
            Verdict {
                value: None,
                available: Some(true)
            }
        );
    }

    #[test]
    fn three_misses_from_start_publish_off() {
        let mut t = Tracker::new();
        assert_eq!(t.on_probe(&Probe::Miss, 1).value, None);
        assert_eq!(t.on_probe(&Probe::Miss, 2).value, None);
        let v = t.on_probe(&Probe::Miss, 3);
        assert_eq!(v.value, Some(false));
        assert_eq!(v.available, Some(true));
    }

    #[test]
    fn status_reports_rtt_and_last_reply() {
        let mut t = Tracker::new();
        t.on_probe(&Probe::Reply(Duration::from_millis(4)), 100);
        let status = t.status();
        assert_eq!(status.last_reply, Some(100));
        assert_eq!(status.rtt_ms, Some(4.0));
    }
}
