//! The body of `PUT /api/v1/virtual/{id}`: a hub-owned switch, number, button or select.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a virtual entity is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VirtualKind {
    /// An on/off value flows can read and write.
    Switch,
    /// A number between `min` and `max`.
    Number,
    /// A press with no state: an event, not a value.
    Button,
    /// One value out of a fixed list of `options`.
    Select,
}

/// A virtual entity's definition. Only `kind` and `name` are required.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VirtualSpec {
    pub kind: VirtualKind,
    /// The display name.
    pub name: String,
    /// Number only: the lower bound, 0 by default.
    #[serde(default)]
    pub min: Option<f64>,
    /// Number only: the upper bound, 100 by default.
    #[serde(default)]
    pub max: Option<f64>,
    /// Number only: the step, 1 by default.
    #[serde(default)]
    pub step: Option<f64>,
    /// Number only: the unit of measurement.
    #[serde(default)]
    pub unit: Option<String>,
    /// Select only: the allowed values, 1 to 32 distinct strings of 1 to 64 characters.
    #[serde(default)]
    pub options: Option<Vec<String>>,
    /// The state published when the entity is first created: `true`/`false` or `"ON"`/`"OFF"`
    /// for a switch (default off), a number within the bounds for a number (default `min`),
    /// one of the options for a select (default the first).
    /// Ignored when the entity already exists.
    #[serde(default)]
    pub initial: Option<serde_json::Value>,
}
