//! The in-memory copy of the retained ambiance definitions.

use std::collections::BTreeMap;

use nidoo_api::{Ambiance, AmbianceView, validate_ambiance};

/// What `apply` did, so the caller knows whether to broadcast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreApplied {
    /// The stored definition (or its invalid state) changed.
    Changed,
    /// The payload is byte-identical to the last one for this id.
    Unchanged,
    /// An empty payload removed a known ambiance.
    Removed,
    /// An empty payload for an id the store never held.
    NotFound,
}

/// A retained payload, decoded: either a usable ambiance or the reason it is not.
enum Entry {
    Valid(Ambiance),
    Invalid { reason: String },
}

/// The decoded copy of the retained ambiances. Invalid payloads are kept, with the reason, so
/// they are not silently lost.
#[derive(Default)]
pub struct AmbianceStore {
    entries: BTreeMap<String, Entry>,
    last_payload: BTreeMap<String, Vec<u8>>,
}

impl AmbianceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies the retained payload of the topic `<prefix>/<id>`. An empty payload removes the
    /// ambiance; a payload identical to the last one for this id is `Unchanged`.
    pub fn apply(&mut self, id: &str, payload: &[u8]) -> StoreApplied {
        if payload.is_empty() {
            self.last_payload.remove(id);
            return match self.entries.remove(id) {
                Some(_) => StoreApplied::Removed,
                None => StoreApplied::NotFound,
            };
        }
        if self
            .last_payload
            .get(id)
            .is_some_and(|last| last == payload)
        {
            return StoreApplied::Unchanged;
        }
        self.last_payload.insert(id.to_owned(), payload.to_vec());
        self.entries
            .insert(id.to_owned(), Self::decode(id, payload));
        StoreApplied::Changed
    }

    fn decode(id: &str, payload: &[u8]) -> Entry {
        let decoded = serde_json::from_slice::<serde_json::Value>(payload)
            .map_err(|err| err.to_string())
            .and_then(|mut value| {
                // A payload without an id means "the topic's id".
                if let Some(object) = value.as_object_mut()
                    && object.get("id").is_none_or(|v| v.is_null())
                {
                    object.insert("id".to_owned(), serde_json::Value::String(id.to_owned()));
                }
                serde_json::from_value::<Ambiance>(value).map_err(|err| err.to_string())
            });
        let ambiance = match decoded {
            Ok(ambiance) => ambiance,
            Err(reason) => return Entry::Invalid { reason },
        };
        if ambiance.id != id {
            return Entry::Invalid {
                reason: format!("id `{}` does not match the topic", ambiance.id),
            };
        }
        match validate_ambiance(&ambiance) {
            Ok(()) => Entry::Valid(ambiance),
            Err(reason) => Entry::Invalid { reason },
        }
    }

    /// A valid ambiance by id.
    pub fn get(&self, id: &str) -> Option<&Ambiance> {
        match self.entries.get(id)? {
            Entry::Valid(ambiance) => Some(ambiance),
            Entry::Invalid { .. } => None,
        }
    }

    /// Whether the store holds this id, valid or not.
    pub fn knows(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// The view of one id, valid or not.
    pub fn view(&self, id: &str) -> Option<AmbianceView> {
        self.entries.get(id).map(|entry| view_of(id, entry))
    }

    /// Valid ambiances sorted by `(order, id)`, then the invalid ones by id.
    pub fn list(&self) -> Vec<AmbianceView> {
        let mut valid: Vec<&Ambiance> = Vec::new();
        let mut invalid: Vec<AmbianceView> = Vec::new();
        for (id, entry) in &self.entries {
            match entry {
                Entry::Valid(ambiance) => valid.push(ambiance),
                Entry::Invalid { .. } => invalid.push(view_of(id, entry)),
            }
        }
        valid.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        valid
            .into_iter()
            .map(|a| view_of(&a.id, &Entry::Valid(a.clone())))
            .chain(invalid)
            .collect()
    }
}

fn view_of(id: &str, entry: &Entry) -> AmbianceView {
    match entry {
        Entry::Valid(ambiance) => AmbianceView {
            id: id.to_owned(),
            ambiance: Some(ambiance.clone()),
            invalid: None,
        },
        Entry::Invalid { reason } => AmbianceView {
            id: id.to_owned(),
            ambiance: None,
            invalid: Some(reason.clone()),
        },
    }
}
