//! Every reading of the entities an `if` averages, kept for the longest window that reads them.
//!
//! History keeps one row per change, so a reading that repeats the last value is not there;
//! an average over readings needs them all, the newest included.

use std::collections::{BTreeMap, VecDeque};

use nidoo_core::history::Micros;
use nidoo_core::history::buckets::Sample;

#[derive(Debug, Default)]
pub struct RecentReadings {
    /// Per entity: how far back to keep, and the readings in arrival order.
    entities: BTreeMap<String, (Micros, VecDeque<Sample>)>,
}

impl RecentReadings {
    /// Keeps readings of exactly the entities in `windows`, each for its window. Readings
    /// already kept for an entity that stays are kept.
    pub fn set_windows(&mut self, windows: BTreeMap<String, Micros>) {
        self.entities.retain(|id, _| windows.contains_key(id));
        for (id, window) in windows {
            self.entities.entry(id).or_default().0 = window;
        }
    }

    /// Whether readings of `id` are kept.
    pub fn wants(&self, id: &str) -> bool {
        self.entities.contains_key(id)
    }

    /// Records a reading of `id` at `at`, dropping the ones older than its window.
    pub fn push(&mut self, id: &str, at: Micros, num: f64) {
        let Some((window, readings)) = self.entities.get_mut(id) else {
            return;
        };
        readings.push_back(Sample { at, num });
        while readings.front().is_some_and(|r| r.at < at - *window) {
            readings.pop_front();
        }
    }

    /// The kept readings of `id` at or after `from`, oldest first.
    pub fn since(&self, id: &str, from: Micros) -> Vec<Sample> {
        self.entities
            .get(id)
            .map(|(_, readings)| readings.iter().filter(|r| r.at >= from).copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Micros = 60_000_000;

    #[test]
    fn keeps_only_wanted_entities_within_their_window() {
        let mut r = RecentReadings::default();
        r.push("t", 0, 1.0);
        assert!(r.since("t", 0).is_empty());
        r.set_windows(BTreeMap::from([("t".to_owned(), 15 * M)]));
        r.push("t", 0, 1.0);
        r.push("t", 10 * M, 2.0);
        r.push("t", 10 * M, 2.0);
        r.push("t", 20 * M, 3.0);
        let nums: Vec<f64> = r.since("t", 0).iter().map(|s| s.num).collect();
        assert_eq!(nums, vec![2.0, 2.0, 3.0]);
        assert_eq!(r.since("t", 15 * M).len(), 1);
    }

    #[test]
    fn set_windows_drops_entities_no_longer_averaged_and_keeps_the_rest() {
        let mut r = RecentReadings::default();
        r.set_windows(BTreeMap::from([("a".to_owned(), M), ("b".to_owned(), M)]));
        r.push("a", 0, 1.0);
        r.push("b", 0, 2.0);
        r.set_windows(BTreeMap::from([("a".to_owned(), 2 * M)]));
        assert!(r.wants("a") && !r.wants("b"));
        assert_eq!(r.since("a", 0).len(), 1);
        assert!(r.since("b", 0).is_empty());
    }
}
