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
mod tests;
