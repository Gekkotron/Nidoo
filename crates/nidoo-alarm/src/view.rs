//! The alarm as one read document: what the REST, MCP and web layers show.

use serde::{Deserialize, Serialize};

use crate::doc::{AlarmSettings, ArmMode};
use crate::machine::Source;

/// The alarm's settings, state and run-time warnings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AlarmView {
    pub enabled: bool,
    /// The panel entity's id.
    pub entity_id: String,
    /// The published state name (`disarmed`, `arming`, `armed_away`, ...).
    pub state: String,
    pub mode: Option<ArmMode>,
    pub source: Option<Source>,
    /// Unix seconds the current state began.
    pub since: u64,
    /// Unix seconds the current phase ends by itself.
    pub deadline: Option<u64>,
    pub cause: Vec<String>,
    pub open_at_arm: Vec<String>,
    pub settings: AlarmSettings,
    /// Why the last settings document was refused; the previous one is kept.
    pub settings_invalid: Option<String>,
    /// Configured entities the house no longer has.
    pub warnings: Vec<String>,
}
