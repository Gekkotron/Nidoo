//! Network presence: a device watched by ping becomes a connectivity binary sensor. Pure
//! helpers for the topic layout, the miss tracker, the discovery config and the stored
//! definitions; the hub does the pinging and publishing.

pub mod discovery;
mod hub;
pub mod probe;
mod runner;
pub mod store;
pub mod topics;
pub mod tracker;

pub use probe::{IcmpProber, Prober};
pub use store::NetworkStore;
pub use tracker::{MISSES, Probe, Tracker, Verdict};
