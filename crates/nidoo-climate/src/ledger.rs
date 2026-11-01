//! Command ledger: what the engine last commanded per entity, and how to tell an observed device
//! state apart from our own command (echo), a device catching up after an outage (drift) and a
//! hand change.

use std::collections::BTreeMap;

use crate::doc::{Decision, DeviceTarget};

/// A device may still be reporting the previous state this long after we commanded it.
pub const ECHO_WINDOW_S: u64 = 30;
/// After a device comes back online, differences within this window are drift, not hand changes.
pub const DRIFT_WINDOW_S: u64 = 120;

/// The last command sent to an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Commanded {
    pub setpoint: Option<f64>,
    pub mode: String,
    pub at: u64,
    /// The engine sent this itself. An entry that only acknowledges what a device holds (a hand
    /// change, or a device already at its target) has no echo window.
    pub ours: bool,
}

/// How an observed device state relates to the last command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observed {
    /// The entity was never commanded.
    Unknown,
    /// Same target (within half a step) and same mode.
    Matches,
    /// Differs, but we commanded it moments ago: the device has not caught up yet.
    Echo,
    /// Differs right after the device came back online.
    Drift,
    /// Differs for no known reason: someone changed it by hand.
    HandChange,
}

#[derive(Debug, Default)]
pub struct Ledger {
    last: BTreeMap<String, Commanded>,
}

impl Ledger {
    /// Seed from each zone's latest decision, applied or not, skipping held entities. The newest
    /// decision wins for an entity whatever the order they come in; on equal times the later one.
    pub fn from_decisions<'a>(decisions: impl IntoIterator<Item = &'a Decision>) -> Ledger {
        let mut ledger = Ledger::default();
        for d in decisions {
            for (entity, target) in d.entities.iter().filter(|(_, t)| !t.held) {
                if ledger.get(entity).is_some_and(|c| c.at > d.at) {
                    continue;
                }
                ledger.record(entity, target, d.at);
            }
        }
        ledger
    }

    /// The engine commanded `entity`.
    pub fn record(&mut self, entity: &str, target: &DeviceTarget, at: u64) {
        self.put(entity, target, at, true);
    }

    /// The engine saw `entity` in this state and takes it as the baseline (a hand change, or a
    /// device already at its target). Unlike [`Ledger::record`], a later difference is a hand
    /// change right away.
    pub fn acknowledge(&mut self, entity: &str, target: &DeviceTarget, at: u64) {
        self.put(entity, target, at, false);
    }

    fn put(&mut self, entity: &str, target: &DeviceTarget, at: u64, ours: bool) {
        self.last.insert(
            entity.to_string(),
            Commanded {
                setpoint: target.setpoint,
                mode: target.mode.clone(),
                at,
                ours,
            },
        );
    }

    pub fn forget(&mut self, entity: &str) {
        self.last.remove(entity);
    }

    pub fn get(&self, entity: &str) -> Option<&Commanded> {
        self.last.get(entity)
    }

    /// Classify an observation. `online_since` is when the device last came back online, if it did.
    pub fn classify(
        &self,
        entity: &str,
        observed_target: Option<f64>,
        observed_mode: Option<&str>,
        step: f64,
        at: u64,
        online_since: Option<u64>,
    ) -> Observed {
        let Some(c) = self.last.get(entity) else {
            return Observed::Unknown;
        };
        // A device that is off has no meaningful target: whatever it reports is the same. Modes
        // compare without case: a device may report `Heat` for the `heat` we sent.
        let same_target = c.mode.eq_ignore_ascii_case("off")
            || match (c.setpoint, observed_target) {
                (None, None) => true,
                (Some(a), Some(b)) => (a - b).abs() <= step / 2.0,
                _ => false,
            };
        if same_target && observed_mode.is_some_and(|m| m.eq_ignore_ascii_case(&c.mode)) {
            return Observed::Matches;
        }
        if c.ours && at.saturating_sub(c.at) <= ECHO_WINDOW_S {
            return Observed::Echo;
        }
        if online_since.is_some_and(|t| at.saturating_sub(t) <= DRIFT_WINDOW_S) {
            return Observed::Drift;
        }
        Observed::HandChange
    }
}

/// When a manual hold ends: the next scheduled change, but never later than `max_min` minutes out.
pub fn hold_until(now: u64, next_change_in_s: Option<u64>, max_min: u32) -> u64 {
    now + next_change_in_s
        .unwrap_or(u64::MAX)
        .min(u64::from(max_min) * 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Mode, Source};

    fn target(setpoint: Option<f64>, mode: &str, held: bool) -> DeviceTarget {
        DeviceTarget {
            setpoint,
            mode: mode.into(),
            held,
        }
    }

    fn ledger_with(setpoint: Option<f64>, mode: &str, at: u64) -> Ledger {
        let mut l = Ledger::default();
        l.record("climate.a", &target(setpoint, mode, false), at);
        l
    }

    fn decision(applied: bool, at: u64, entities: Vec<(&str, DeviceTarget)>) -> Decision {
        Decision {
            zone: "z".into(),
            mode: Some(Mode::Heat),
            setpoint: None,
            source: Source::Schedule,
            cause: None,
            reason: String::new(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            applied,
            at,
            entities: entities
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            eco_restore: BTreeMap::new(),
            adjust: None,
        }
    }

    #[test]
    fn unknown_when_never_commanded() {
        let l = Ledger::default();
        assert_eq!(
            l.classify("climate.a", Some(20.0), Some("heat"), 0.5, 100, None),
            Observed::Unknown
        );
    }

    #[test]
    fn matches_within_half_step() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        assert_eq!(
            l.classify("climate.a", Some(20.2), Some("heat"), 0.5, 5000, None),
            Observed::Matches
        );
    }

    #[test]
    fn none_against_some_is_a_difference() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        assert_eq!(
            l.classify("climate.a", None, Some("heat"), 0.5, 5000, None),
            Observed::HandChange
        );
    }

    #[test]
    fn echo_within_30s() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        assert_eq!(
            l.classify("climate.a", Some(19.0), Some("heat"), 0.5, 1010, None),
            Observed::Echo
        );
    }

    #[test]
    fn echo_window_ends_after_30s() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        let at = |t| l.classify("climate.a", Some(19.0), Some("heat"), 0.5, t, None);
        assert_eq!(at(1000 + ECHO_WINDOW_S), Observed::Echo);
        assert_eq!(at(1000 + ECHO_WINDOW_S + 1), Observed::HandChange);
    }

    #[test]
    fn drift_window_ends_after_120s() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        let at = 5000;
        let since = |s| l.classify("climate.a", Some(19.0), Some("heat"), 0.5, at, Some(s));
        assert_eq!(since(at - DRIFT_WINDOW_S), Observed::Drift);
        assert_eq!(since(at - DRIFT_WINDOW_S - 1), Observed::HandChange);
    }

    #[test]
    fn modes_compare_without_case() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        assert_eq!(
            l.classify("climate.a", Some(20.0), Some("Heat"), 0.5, 5000, None),
            Observed::Matches
        );
        let l = ledger_with(None, "OFF", 1000);
        assert_eq!(
            l.classify("climate.a", Some(21.0), Some("off"), 0.5, 5000, None),
            Observed::Matches
        );
    }

    #[test]
    fn drift_after_coming_back_online() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        let at = 1600;
        assert_eq!(
            l.classify(
                "climate.a",
                Some(19.0),
                Some("heat"),
                0.5,
                at,
                Some(at - 60)
            ),
            Observed::Drift
        );
    }

    #[test]
    fn hand_change_otherwise() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        assert_eq!(
            l.classify("climate.a", Some(19.0), Some("heat"), 0.5, 1600, None),
            Observed::HandChange
        );
        // Online long ago: not drift.
        assert_eq!(
            l.classify("climate.a", Some(19.0), Some("heat"), 0.5, 1600, Some(1000)),
            Observed::HandChange
        );
    }

    #[test]
    fn mode_change_is_a_change() {
        let l = ledger_with(Some(20.0), "heat", 1000);
        assert_eq!(
            l.classify("climate.a", Some(20.0), Some("off"), 0.5, 1600, None),
            Observed::HandChange
        );
    }

    #[test]
    fn ledger_rebuilt_from_every_latest_decision() {
        let ds = [
            decision(
                true,
                1000,
                vec![
                    ("climate.a", target(Some(20.0), "heat", false)),
                    ("climate.held", target(Some(18.0), "heat", true)),
                ],
            ),
            decision(
                false,
                2000,
                vec![("climate.b", target(Some(21.0), "heat", false))],
            ),
        ];
        let l = Ledger::from_decisions(&ds);
        let a = l.get("climate.a").unwrap();
        assert_eq!(
            (a.setpoint, a.mode.as_str(), a.at),
            (Some(20.0), "heat", 1000)
        );
        assert!(l.get("climate.held").is_none());
        let b = l.get("climate.b").unwrap();
        assert_eq!((b.setpoint, b.at), (Some(21.0), 2000));
    }

    #[test]
    fn a_later_decision_wins_for_the_same_entity() {
        let ds = [
            decision(
                true,
                1000,
                vec![("climate.a", target(Some(20.0), "heat", false))],
            ),
            decision(
                false,
                2000,
                vec![("climate.a", target(Some(18.0), "heat", false))],
            ),
        ];
        assert_eq!(
            Ledger::from_decisions(&ds)
                .get("climate.a")
                .unwrap()
                .setpoint,
            Some(18.0)
        );
    }

    #[test]
    fn an_older_decision_never_overwrites_a_newer_one() {
        let newer = decision(
            false,
            2000,
            vec![("climate.a", target(Some(18.0), "heat", false))],
        );
        let older = decision(
            true,
            1000,
            vec![("climate.a", target(Some(20.0), "heat", false))],
        );
        let a = Ledger::from_decisions([&newer, &older])
            .get("climate.a")
            .cloned()
            .unwrap();
        assert_eq!((a.setpoint, a.at), (Some(18.0), 2000));
    }

    #[test]
    fn off_ignores_the_setpoint() {
        let l = ledger_with(None, "off", 1000);
        assert_eq!(
            l.classify("climate.a", Some(21.0), Some("off"), 0.5, 5000, None),
            Observed::Matches
        );
        let l = ledger_with(Some(10.0), "off", 1000);
        assert_eq!(
            l.classify("climate.a", None, Some("off"), 0.5, 5000, None),
            Observed::Matches
        );
        // Another mode still differs.
        assert_eq!(
            l.classify("climate.a", Some(10.0), Some("heat"), 0.5, 5000, None),
            Observed::HandChange
        );
    }

    #[test]
    fn an_acknowledged_state_has_no_echo_window() {
        let mut l = Ledger::default();
        l.acknowledge("climate.a", &target(Some(22.0), "heat", false), 1000);
        assert_eq!(
            l.classify("climate.a", Some(23.0), Some("heat"), 0.5, 1005, None),
            Observed::HandChange
        );
        assert_eq!(
            l.classify("climate.a", Some(22.0), Some("heat"), 0.5, 1005, None),
            Observed::Matches
        );
    }

    #[test]
    fn forget_removes_the_entry() {
        let mut l = ledger_with(Some(20.0), "heat", 1000);
        l.forget("climate.a");
        assert!(l.get("climate.a").is_none());
    }

    #[test]
    fn restart_difference_is_hand_change() {
        let ds = [decision(
            true,
            1000,
            vec![("climate.a", target(Some(20.0), "heat", false))],
        )];
        let l = Ledger::from_decisions(&ds);
        assert_eq!(
            l.classify("climate.a", Some(17.0), Some("heat"), 0.5, 5000, None),
            Observed::HandChange
        );
    }

    #[test]
    fn hold_until_caps_at_max() {
        assert_eq!(hold_until(1000, Some(6 * 3600), 240), 1000 + 14400);
        assert_eq!(hold_until(1000, Some(3600), 240), 1000 + 3600);
        assert_eq!(hold_until(1000, None, 240), 1000 + 14400);
    }
}
