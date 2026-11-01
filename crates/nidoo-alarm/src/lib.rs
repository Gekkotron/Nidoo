//! Native alarm documents, state machine and MQTT topics.
//!
//! Pure data: no I/O, builds for `wasm32-unknown-unknown`. The hub and the web UI share these types.

pub mod doc;
pub mod machine;
pub mod topics;
pub mod view;
