//! The hub's flow engine: the store of decoded definitions and their run history.

pub mod engine;
mod hub;
pub(crate) use hub::LimitCounts;
pub mod readings;
pub mod schedule;
mod store;
mod sun;

pub use schedule::house_zone;
pub use store::{FlowStore, StoreApplied};

/// What the hub asks of the flow engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineRequest {
    /// The definitions changed: re-read them and re-arm.
    Reload,
    /// Execute a run the hub already created (a manual run), from the Manual card `node` or,
    /// without one, from the flow's first trigger.
    Run {
        flow_id: String,
        run_id: String,
        node: Option<String>,
    },
    /// The run is about to disable its own flow (a `flow.enable` node): let it survive the
    /// reload that follows and finish its remaining steps.
    KeepRun { flow_id: String, run_id: String },
}
