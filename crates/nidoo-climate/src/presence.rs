//! Room presence (spec §7.1): whether a room counts as empty, from when its sensors last
//! detected someone.
//!
//! Pure and deterministic: no I/O, no clock. `now` (Unix seconds) comes in through the input.

use crate::doc::PresenceSettings;

/// How close to the end of the "back" window a detection must be for the room to count as
/// occupied again: someone still there, not someone who passed through.
pub const CONFIRM_S: u64 = 120;

/// What presence remembers between two ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresenceState {
    /// The latest detection known, or when tracking (re)started, or the last time no sensor was
    /// usable (Unix seconds).
    pub last_detected: u64,
    /// When the room became empty; `None` while occupied.
    pub empty_since: Option<u64>,
    /// The first detection while empty, which opens the "back" window.
    pub arrival: Option<u64>,
}

impl PresenceState {
    pub fn is_empty(&self) -> bool {
        self.empty_since.is_some()
    }
}

/// What the room's sensors say this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensed {
    /// None configured, or none usable: the room counts as occupied.
    NoSensor,
    /// The latest time any usable sensor was detecting: `Some(now)` while one is on, `None` when
    /// none has been seen on since the hub started.
    Seen(Option<u64>),
}

/// The presence state after this tick.
pub fn track(
    previous: Option<&PresenceState>,
    settings: &PresenceSettings,
    sensed: Sensed,
    now: u64,
) -> PresenceState {
    // A restart, or no usable sensor: occupied, counting from now (comfort first).
    let (Some(previous), Sensed::Seen(seen)) = (previous, sensed) else {
        return PresenceState {
            last_detected: now,
            empty_since: None,
            arrival: None,
        };
    };
    let fresh = seen.filter(|at| *at > previous.last_detected);
    let last = previous.last_detected.max(seen.unwrap_or(0));
    let mut state = PresenceState {
        last_detected: last,
        ..*previous
    };
    if state.empty_since.is_none() {
        if now.saturating_sub(last) >= u64::from(settings.empty_after_min) * 60 {
            state.empty_since = Some(now);
            state.arrival = None;
        }
        return state;
    }
    if let Some(arrival) = state.arrival {
        let back = u64::from(settings.back_after_min) * 60;
        let end = arrival + back;
        if now >= end {
            state.arrival = None;
            // Someone was still detected near the end of the window: they stayed.
            if last >= end - CONFIRM_S.min(back) {
                state.empty_since = None;
                return state;
            }
        }
    }
    if state.arrival.is_none() {
        state.arrival = fresh;
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_000_000;
    /// When the room in the "empty" tests became empty.
    const E: u64 = T + 1800;

    fn settings() -> PresenceSettings {
        PresenceSettings::default()
    }

    fn step(previous: &PresenceState, seen: Option<u64>, now: u64) -> PresenceState {
        track(Some(previous), &settings(), Sensed::Seen(seen), now)
    }

    fn empty(last_detected: u64, arrival: Option<u64>) -> PresenceState {
        PresenceState {
            last_detected,
            empty_since: Some(E),
            arrival,
        }
    }

    #[test]
    fn boot_counts_as_occupied() {
        let s0 = track(None, &settings(), Sensed::Seen(None), T);
        assert!(!s0.is_empty());
        assert_eq!(s0.last_detected, T);
        let s1 = step(&s0, None, T + 1799);
        assert!(!s1.is_empty());
        let s2 = step(&s1, None, T + 1800);
        assert!(s2.is_empty());
        assert_eq!(s2.empty_since, Some(T + 1800));
    }

    #[test]
    fn detection_keeps_it_occupied() {
        let s0 = track(None, &settings(), Sensed::Seen(None), T);
        let s1 = step(&s0, Some(T + 1000), T + 2700);
        assert!(!s1.is_empty());
        let s2 = step(&s1, Some(T + 1000), T + 2800);
        assert!(s2.is_empty());
    }

    #[test]
    fn no_sensor_is_occupied() {
        let s = track(
            Some(&empty(T, None)),
            &settings(),
            Sensed::NoSensor,
            T + 5000,
        );
        assert!(!s.is_empty());
        assert_eq!(s.last_detected, T + 5000);
    }

    #[test]
    fn a_pass_does_not_end_it() {
        let s1 = step(&empty(T, None), Some(E + 10), E + 10);
        assert!(s1.is_empty());
        assert_eq!(s1.arrival, Some(E + 10));
        let s2 = step(&s1, Some(E + 10), E + 309);
        assert!(s2.is_empty());
        assert_eq!(s2.arrival, Some(E + 10));
        let s3 = step(&s2, Some(E + 10), E + 310);
        assert!(s3.is_empty());
        assert_eq!(s3.arrival, None);
        // A later detection opens a new window.
        let s4 = step(&s3, Some(E + 900), E + 900);
        assert!(s4.is_empty());
        assert_eq!(s4.arrival, Some(E + 900));
    }

    #[test]
    fn staying_ends_it_after_back_after() {
        let mut s = step(&empty(T, None), Some(E + 10), E + 10);
        for t in (E + 60..=E + 300).step_by(60) {
            s = step(&s, Some(t), t);
            assert!(s.is_empty(), "at {}", t - E);
        }
        let s = step(&s, Some(E + 310), E + 310);
        assert!(!s.is_empty());
        assert_eq!(s.arrival, None);
    }

    #[test]
    fn pulses_near_the_end_count() {
        let s = step(&empty(E + 200, Some(E + 10)), Some(E + 200), E + 310);
        assert!(!s.is_empty());
        let s = step(&empty(E + 150, Some(E + 10)), Some(E + 150), E + 310);
        assert!(s.is_empty());
        assert_eq!(s.arrival, None);
    }

    #[test]
    fn back_after_shorter_than_confirm() {
        let short = PresenceSettings {
            back_after_min: 1,
            ..settings()
        };
        let a = E + 10;
        let s = track(
            Some(&empty(a, Some(a))),
            &short,
            Sensed::Seen(Some(a)),
            a + 60,
        );
        assert!(!s.is_empty());
    }
}
