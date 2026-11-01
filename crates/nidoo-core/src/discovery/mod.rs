//! Home Assistant MQTT discovery: topic classification, abbreviation expansion and the
//! conversion of a discovery config into an [`Entity`](crate::model::Entity).
//!
//! The registry resolves areas and overrides later: entities leave here with
//! [`UNASSIGNED_AREA`](crate::model::UNASSIGNED_AREA), a default override and no `part_of`.

mod abbreviations;
pub mod capabilities;
mod expand;
pub mod naming;
mod parse;
mod topic;

pub use expand::expand;
pub use parse::{
    DEVICE_BASED_REASON, DeviceAutomationFragment, DiscoveryError, Parsed, expand_device_based,
    parse,
};
pub use topic::{DiscoveryTopic, TopicClass, classify};
