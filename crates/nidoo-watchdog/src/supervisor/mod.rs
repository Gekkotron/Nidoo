//! Sensor supervisor: flags numeric sensors that look stuck, silent or physically impossible.
//!
//! Pure logic only. The hub gathers [`Reading`]s (registry, live report clocks, history) and
//! hands them in with the clock; [`Supervisor::step`] decides which problems open and close,
//! and [`alerts`] turns that into v1 push alerts.

mod alerts;
mod checks;
mod problems;

pub use alerts::alerts;
pub use checks::{
    Baseline, Check, Ctx, Evidence, Finding, GapStats, Reading, Sensitivity, baseline, evaluate,
    physical_range,
};
pub use problems::{Closed, Problem, Step, Supervisor};
