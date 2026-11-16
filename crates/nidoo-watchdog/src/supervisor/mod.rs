//! Sensor supervisor: flags numeric sensors that look stuck, silent or physically impossible.
//!
//! Pure logic only. The hub gathers [`Reading`]s (registry, live report clocks, history) and
//! hands them in with the clock; this module decides what is wrong.

mod checks;

pub use checks::{
    Baseline, Check, Ctx, Evidence, Finding, GapStats, Reading, Sensitivity, baseline, evaluate,
    physical_range,
};
