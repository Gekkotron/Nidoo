//! Hand-made groups: one entity that fans commands out to its members. Pure helpers for the
//! topic layout, the discovery config and the combined state (Appendix A of the design);
//! the hub publishes what they build.

pub mod discovery;
pub mod fanout;
pub mod migrate;
pub mod state;
pub mod store;
pub mod topics;

pub use store::{ApplyOutcome, GroupStore};
