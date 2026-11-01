//! The hub's copy of the retained climate documents.
//!
//! A document that does not parse or validate never replaces the previous one: the reason is kept
//! next to it so the UI can say why an edit has no effect.

use std::collections::BTreeMap;

use nidoo_climate::doc::{Decision, HouseSettings, Override, Proposal, RoomEco, ZoneSettings};
use nidoo_climate::topics::{ClimateTopic, ZoneDoc};
use serde::de::DeserializeOwned;

/// What applying a payload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    Changed,
    Unchanged,
    /// The payload was refused, with the reason; the previous value is kept.
    Invalid(String),
}

/// The retained documents of the house and of each zone.
#[derive(Default)]
pub struct ClimateStore {
    pub house: HouseSettings,
    pub house_invalid: Option<String>,
    pub zones: BTreeMap<String, ZoneDocs>,
}

/// The retained documents of one zone.
#[derive(Default)]
pub struct ZoneDocs {
    pub settings: ZoneSettings,
    pub settings_invalid: Option<String>,
    pub proposal: Option<Proposal>,
    pub overrides: Vec<Override>,
    pub decision: Option<Decision>,
    pub eco: Option<RoomEco>,
}

impl ZoneDocs {
    fn is_empty(&self) -> bool {
        self.settings == ZoneSettings::default()
            && self.settings_invalid.is_none()
            && self.proposal.is_none()
            && self.overrides.is_empty()
            && self.decision.is_none()
            && self.eco.is_none()
    }
}

impl ClimateStore {
    /// Applies the retained `payload` of `topic`; an empty payload resets that document.
    pub fn apply(&mut self, topic: &ClimateTopic, payload: &[u8]) -> Applied {
        match topic {
            ClimateTopic::House => apply_validated(
                &mut self.house,
                &mut self.house_invalid,
                payload,
                HouseSettings::validate,
            ),
            ClimateTopic::Zone { zone, doc } => {
                let docs = self.zones.entry(zone.clone()).or_default();
                let applied = apply_zone(docs, *doc, zone, payload);
                if docs.is_empty() {
                    self.zones.remove(zone);
                }
                applied
            }
        }
    }
}

fn apply_zone(docs: &mut ZoneDocs, doc: ZoneDoc, zone: &str, payload: &[u8]) -> Applied {
    match doc {
        ZoneDoc::Settings => apply_validated(
            &mut docs.settings,
            &mut docs.settings_invalid,
            payload,
            ZoneSettings::validate,
        ),
        ZoneDoc::Proposal => replace(&mut docs.proposal, optional(payload, zone, "proposal")),
        ZoneDoc::Decision => replace(&mut docs.decision, optional(payload, zone, "decision")),
        ZoneDoc::Eco => replace(&mut docs.eco, optional(payload, zone, "eco")),
        ZoneDoc::Override => {
            let value = if payload.is_empty() {
                Some(Vec::new())
            } else {
                parse::<Vec<Override>>(payload, zone, "override")
            };
            match value {
                Some(list) if list != docs.overrides => {
                    docs.overrides = list;
                    Applied::Changed
                }
                _ => Applied::Unchanged,
            }
        }
    }
}

/// Applies a validated document. An empty payload resets to the default; a refused one keeps
/// `slot` and stores the reason in `invalid`.
pub(crate) fn apply_validated<T: DeserializeOwned + Default + PartialEq>(
    slot: &mut T,
    invalid: &mut Option<String>,
    payload: &[u8],
    validate: fn(&T) -> Result<(), String>,
) -> Applied {
    let parsed = if payload.is_empty() {
        Ok(T::default())
    } else {
        serde_json::from_slice::<T>(payload)
            .map_err(|err| err.to_string())
            .and_then(|value| validate(&value).map(|()| value))
    };
    match parsed {
        Ok(value) => {
            // A document that was invalid and is valid now has changed, even to the same value.
            let changed = *slot != value || invalid.is_some();
            *slot = value;
            *invalid = None;
            if changed {
                Applied::Changed
            } else {
                Applied::Unchanged
            }
        }
        Err(reason) => {
            *invalid = Some(reason.clone());
            Applied::Invalid(reason)
        }
    }
}

fn replace<T: PartialEq>(slot: &mut Option<T>, value: Option<Option<T>>) -> Applied {
    match value {
        Some(value) if *slot != value => {
            *slot = value;
            Applied::Changed
        }
        _ => Applied::Unchanged,
    }
}

/// `None` when the payload is malformed (logged), `Some(None)` when it is empty.
fn optional<T: DeserializeOwned>(payload: &[u8], zone: &str, doc: &str) -> Option<Option<T>> {
    if payload.is_empty() {
        return Some(None);
    }
    parse(payload, zone, doc).map(Some)
}

fn parse<T: DeserializeOwned>(payload: &[u8], zone: &str, doc: &str) -> Option<T> {
    match serde_json::from_slice(payload) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!(%zone, %doc, error = %err, "climate document ignored");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_climate::doc::ZoneState;

    fn zone(doc: ZoneDoc) -> ClimateTopic {
        ClimateTopic::Zone {
            zone: "living".to_owned(),
            doc,
        }
    }

    #[test]
    fn settings_apply_reset_and_keep_invalid() {
        let mut store = ClimateStore::default();
        let t = zone(ZoneDoc::Settings);
        assert_eq!(store.apply(&t, br#"{"state":"active"}"#), Applied::Changed);
        assert_eq!(store.zones["living"].settings.state, ZoneState::Active);
        assert_eq!(
            store.apply(&t, br#"{"state":"active"}"#),
            Applied::Unchanged
        );
        let bad = store.apply(&t, br#"{"state":"bogus"}"#);
        assert!(matches!(bad, Applied::Invalid(_)));
        let docs = &store.zones["living"];
        assert_eq!(docs.settings.state, ZoneState::Active);
        assert!(docs.settings_invalid.is_some());
        // A valid document clears the reason, even when the value is the same.
        assert_eq!(store.apply(&t, br#"{"state":"active"}"#), Applied::Changed);
        assert!(store.zones["living"].settings_invalid.is_none());
        assert_eq!(store.apply(&t, b""), Applied::Changed);
        assert!(store.zones.is_empty(), "an all-default zone is dropped");
    }

    #[test]
    fn house_invalid_keeps_previous() {
        let mut store = ClimateStore::default();
        store.apply(&ClimateTopic::House, br#"{"heat_below":18.0}"#);
        let bad = store.apply(&ClimateTopic::House, br#"{"frost":40.0}"#);
        assert!(matches!(bad, Applied::Invalid(_)));
        assert_eq!(store.house.heat_below, 18.0);
        assert!(store.house_invalid.is_some());
        store.apply(&ClimateTopic::House, b"");
        assert_eq!(store.house, HouseSettings::default());
        assert!(store.house_invalid.is_none());
    }

    #[test]
    fn malformed_proposal_decision_override_are_ignored() {
        let mut store = ClimateStore::default();
        for doc in [ZoneDoc::Proposal, ZoneDoc::Decision, ZoneDoc::Override] {
            assert_eq!(store.apply(&zone(doc), b"{nope"), Applied::Unchanged);
        }
        assert!(store.zones.is_empty());
        let list = br#"[{"entity":null,"setpoint":21.0,"mode":null,"until":10,"origin":"user"}]"#;
        assert_eq!(
            store.apply(&zone(ZoneDoc::Override), list),
            Applied::Changed
        );
        assert_eq!(store.zones["living"].overrides.len(), 1);
        assert_eq!(store.apply(&zone(ZoneDoc::Override), b""), Applied::Changed);
        assert!(store.zones.is_empty());
    }
}
