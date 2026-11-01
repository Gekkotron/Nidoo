//! The in-memory copy of the retained group definitions and the migration marker.

use std::collections::BTreeMap;

use nidoo_api::GroupSpec;

use super::topics::component_for;

/// The id the migration marker is applied under.
const MARKER_ID: &str = "_migrated";

/// What `apply` did, so the caller knows whether to republish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// A definition for an id the store did not hold.
    Created,
    /// A different definition for a known id.
    Replaced,
    /// An empty payload removed a known definition.
    Deleted,
    /// The migration marker was set or cleared.
    Marker,
    /// Nothing changed.
    Unchanged,
    /// The payload was refused, with the reason; the state is untouched.
    Rejected(String),
}

/// The retained group definitions by id, and whether the room groups were migrated.
#[derive(Debug, Default)]
pub struct GroupStore {
    defs: BTreeMap<String, GroupSpec>,
    migrated: bool,
}

impl GroupStore {
    /// Applies the retained payload of group `id`, or of the marker when `id` is `_migrated`.
    /// An empty payload deletes the definition (or clears the marker).
    pub fn apply(&mut self, id: &str, payload: &[u8]) -> ApplyOutcome {
        if id == MARKER_ID {
            self.migrated = !payload.is_empty();
            return ApplyOutcome::Marker;
        }
        if payload.is_empty() {
            return match self.defs.remove(id) {
                Some(_) => ApplyOutcome::Deleted,
                None => ApplyOutcome::Unchanged,
            };
        }
        let spec: GroupSpec = match serde_json::from_slice(payload) {
            Ok(spec) => spec,
            Err(err) => return ApplyOutcome::Rejected(err.to_string()),
        };
        if let Err(reason) = component_for(spec.kind) {
            return ApplyOutcome::Rejected(reason);
        }
        match self.defs.insert(id.to_owned(), spec.clone()) {
            None => ApplyOutcome::Created,
            Some(old) if old == spec => ApplyOutcome::Unchanged,
            Some(_) => ApplyOutcome::Replaced,
        }
    }

    /// The definition of group `id`.
    pub fn get(&self, id: &str) -> Option<&GroupSpec> {
        self.defs.get(id)
    }

    /// Every definition, by id.
    pub fn all(&self) -> &BTreeMap<String, GroupSpec> {
        &self.defs
    }

    /// Whether the room groups were migrated.
    pub fn migrated(&self) -> bool {
        self.migrated
    }

    /// Records the migration locally; the caller retains the marker on the broker.
    pub fn mark_migrated(&mut self) {
        self.migrated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVING: &[u8] = br#"{"name":"Living","kind":"light","members":["a"]}"#;
    const LIVING_2: &[u8] = br#"{"name":"Living","kind":"light","members":["a","b"]}"#;

    #[test]
    fn create_unchanged_replace_delete() {
        let mut store = GroupStore::default();
        assert_eq!(store.apply("living", LIVING), ApplyOutcome::Created);
        assert!(store.get("living").is_some());
        assert_eq!(store.apply("living", LIVING), ApplyOutcome::Unchanged);
        assert_eq!(store.apply("living", LIVING_2), ApplyOutcome::Replaced);
        assert_eq!(store.get("living").unwrap().members.len(), 2);
        assert_eq!(store.apply("living", b""), ApplyOutcome::Deleted);
        assert!(store.get("living").is_none());
        assert_eq!(store.apply("living", b""), ApplyOutcome::Unchanged);
    }

    #[test]
    fn marker_sets_and_clears() {
        let mut store = GroupStore::default();
        assert!(!store.migrated());
        assert_eq!(store.apply("_migrated", b"1"), ApplyOutcome::Marker);
        assert!(store.migrated());
        assert_eq!(store.apply("_migrated", b""), ApplyOutcome::Marker);
        assert!(!store.migrated());
        store.mark_migrated();
        assert!(store.migrated());
    }

    #[test]
    fn garbage_is_rejected_and_state_kept() {
        let mut store = GroupStore::default();
        store.apply("x", LIVING);
        assert!(matches!(
            store.apply("x", b"not json"),
            ApplyOutcome::Rejected(_)
        ));
        assert_eq!(store.get("x").unwrap().members, vec!["a"]);
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn out_of_scope_kind_is_rejected() {
        let mut store = GroupStore::default();
        let payload = br#"{"name":"S","kind":"sensor","members":[]}"#;
        match store.apply("x", payload) {
            ApplyOutcome::Rejected(reason) => assert!(reason.contains("sensor"), "{reason}"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert!(store.get("x").is_none());
    }
}
