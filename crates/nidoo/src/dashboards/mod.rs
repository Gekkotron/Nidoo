//! The hub's copy of the retained dashboard definitions.

pub mod house;
pub(crate) mod hub;
pub mod store;

pub use house::synthesize_house;
pub use store::{DashboardStore, StoreApplied, validate_widget};
