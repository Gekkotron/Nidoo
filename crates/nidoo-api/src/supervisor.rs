//! Sensor supervisor: its settings and what it currently sees. Times are Unix seconds.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How quickly a quiet or flat sensor is flagged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorSensitivity {
    /// Waits three times the sensor's longest usual gap.
    Low,
    /// Waits twice the sensor's longest usual gap.
    #[default]
    Normal,
    /// Waits one and a half times the sensor's longest usual gap.
    High,
}

/// The supervisor's settings, saved live (no hub restart).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SupervisorSettings {
    /// Run the checks at all.
    pub enabled: bool,
    /// Send a push when a sensor looks wrong.
    pub notify: bool,
    /// Send a push when it is back to normal.
    pub notify_recovered: bool,
    /// How quickly a quiet or flat sensor is flagged.
    pub sensitivity: SupervisorSensitivity,
    /// Entity ids never checked.
    pub ignored: Vec<String>,
}

impl Default for SupervisorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            notify: true,
            notify_recovered: true,
            sensitivity: SupervisorSensitivity::Normal,
            ignored: Vec::new(),
        }
    }
}

/// One open problem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProblemView {
    pub entity_id: String,
    /// The sensor's display name.
    pub name: String,
    /// `stuck`, `silent` or `impossible`.
    pub check: String,
    /// When the problem was first seen.
    pub opened_at: u64,
    /// What the check saw: `value`, `unit`, `for_secs`, `usual_secs`.
    pub evidence: Value,
}

/// The supervisor page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SupervisorView {
    /// False without a database: the supervisor does nothing.
    pub available: bool,
    pub settings: SupervisorSettings,
    /// Open problems.
    pub problems: Vec<ProblemView>,
    /// Supervised sensors still learning (less than 7 days or 20 changes of history).
    pub learning: u32,
}
