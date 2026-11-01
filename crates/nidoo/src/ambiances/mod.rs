//! The hub's copy of the retained ambiance definitions.

mod apply;
pub(crate) mod hub;
pub mod store;

pub use store::{AmbianceStore, StoreApplied};

use nidoo_api::ClimateAction;
use nidoo_climate::view::OverrideRequest;

/// The climate crate's request for a hold action: one mapping (`hours` become `minutes`) for
/// the save-time check and for the apply, so they cannot disagree.
pub(crate) fn override_request(
    action: &ClimateAction,
) -> Result<OverrideRequest, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "setpoint": action.setpoint,
        "mode": action.mode,
        "minutes": action.hours.map(|h| u64::from(h) * 60),
    }))
}
