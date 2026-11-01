//! The in-memory copy of the retained network watch definitions.

use std::collections::BTreeMap;

use nidoo_api::NetworkWatchSpec;

use super::topics::validate_spec;
use crate::groups::ApplyOutcome;

/// The retained definitions by id, and a revision that moves on every change.
#[derive(Debug, Default)]
pub struct NetworkStore {
    defs: BTreeMap<String, NetworkWatchSpec>,
    rev: u64,
}

impl NetworkStore {
    /// Applies the retained payload of watch `id`. An empty payload deletes the definition.
    pub fn apply(&mut self, id: &str, payload: &[u8]) -> ApplyOutcome {
        let outcome = if payload.is_empty() {
            match self.defs.remove(id) {
                Some(_) => ApplyOutcome::Deleted,
                None => ApplyOutcome::Unchanged,
            }
        } else {
            let spec: NetworkWatchSpec = match serde_json::from_slice(payload) {
                Ok(spec) => spec,
                Err(err) => return ApplyOutcome::Rejected(err.to_string()),
            };
            if let Err(reason) = validate_spec(&spec) {
                return ApplyOutcome::Rejected(reason);
            }
            match self.defs.insert(id.to_owned(), spec.clone()) {
                None => ApplyOutcome::Created,
                Some(old) if old == spec => ApplyOutcome::Unchanged,
                Some(_) => ApplyOutcome::Replaced,
            }
        };
        if !matches!(outcome, ApplyOutcome::Unchanged) {
            self.rev += 1;
        }
        outcome
    }

    /// The definition of watch `id`.
    pub fn get(&self, id: &str) -> Option<&NetworkWatchSpec> {
        self.defs.get(id)
    }

    /// Every definition, by id.
    pub fn all(&self) -> &BTreeMap<String, NetworkWatchSpec> {
        &self.defs
    }

    /// The revision, bumped on every change.
    pub fn rev(&self) -> u64 {
        self.rev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAS: &[u8] = br#"{"name":"NAS","host":"nas.local"}"#;
    const NAS_2: &[u8] = br#"{"name":"NAS","host":"192.168.1.42"}"#;

    #[test]
    fn apply_round_trip_and_delete() {
        let mut store = NetworkStore::default();
        assert_eq!(store.apply("nas", NAS), ApplyOutcome::Created);
        assert_eq!(store.get("nas").unwrap().host, "nas.local");
        assert_eq!(store.apply("nas", NAS), ApplyOutcome::Unchanged);
        assert_eq!(store.apply("nas", NAS_2), ApplyOutcome::Replaced);
        assert_eq!(store.all().len(), 1);
        assert_eq!(store.apply("nas", b""), ApplyOutcome::Deleted);
        assert!(store.get("nas").is_none());
        assert_eq!(store.apply("nas", b""), ApplyOutcome::Unchanged);
    }

    #[test]
    fn bad_json_rejected_state_unchanged() {
        let mut store = NetworkStore::default();
        store.apply("nas", NAS);
        let rev = store.rev();
        assert!(matches!(
            store.apply("nas", b"{nope"),
            ApplyOutcome::Rejected(_)
        ));
        assert_eq!(store.get("nas").unwrap().host, "nas.local");
        assert_eq!(store.rev(), rev);
    }

    #[test]
    fn invalid_definition_rejected_state_unchanged() {
        let mut store = NetworkStore::default();
        let zero = br#"{"name":"NAS","host":"nas.local","intervalS":0}"#;
        assert!(matches!(
            store.apply("nas", zero),
            ApplyOutcome::Rejected(_)
        ));
        assert!(store.get("nas").is_none());
        assert_eq!(store.rev(), 0);
        store.apply("nas", NAS);
        assert!(matches!(
            store.apply("nas", zero),
            ApplyOutcome::Rejected(_)
        ));
        assert_eq!(store.get("nas").unwrap().interval_s, 30);
    }

    #[test]
    fn rev_bumps_on_change_only() {
        let mut store = NetworkStore::default();
        assert_eq!(store.rev(), 0);
        store.apply("nas", NAS);
        assert_eq!(store.rev(), 1);
        store.apply("nas", NAS);
        assert_eq!(store.rev(), 1);
        store.apply("nas", b"");
        assert_eq!(store.rev(), 2);
    }
}
