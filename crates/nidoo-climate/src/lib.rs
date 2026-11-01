//! Climate engine documents and MQTT topics.
//!
//! Pure data: no I/O, builds for `wasm32-unknown-unknown`. The hub and the web UI share these types.

pub mod decide;
pub mod doc;
pub mod eco;
pub mod feedback;
pub mod learn;
pub mod ledger;
pub mod presence;
pub mod schedule;
pub mod topics;
pub mod view;
