//! Flows on the wire: a flow with its run history, as the web shows it and as the hub pushes it.

use nidoo_core::flow::Flow;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One flow with what the engine knows about it right now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowView {
    /// The flow itself. For an invalid flow this is a stub (id, name = id, no nodes) so the
    /// UI still has something to list.
    pub definition: Flow,
    /// Why the flow cannot run, when it cannot.
    pub invalid: Option<String>,
    /// The id of the run in progress, if any.
    pub active_run: Option<String>,
    /// The recent runs, newest first.
    pub runs: Vec<RunView>,
}

/// The answer to a flow write: the stored flow and, when the flow reads the sun against the
/// Paris fallback, a warning. `warning` is always on the wire (`null` when there is none).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowWriteOutcome {
    pub flow: FlowView,
    pub warning: Option<String>,
}

/// The `[flows]` settings the UI needs: the house point as configured, the time zone and
/// whether sun flows run against the Paris fallback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowsConfigView {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// The house time zone (an IANA name).
    pub timezone: String,
    pub fallback_active: bool,
}

/// The next ticks of a cron expression, for the schedule editor's preview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchedulePreview {
    /// The house time zone the expression is read in (an IANA name).
    pub timezone: String,
    /// The next ticks, soonest first, as RFC 3339 with the zone's offset.
    pub next: Vec<String>,
}

/// One execution of a flow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunView {
    pub id: String,
    pub flow_id: String,
    pub status: RunStatus,
    /// Unix seconds.
    pub started_at: u64,
    /// Unix seconds; `None` while the run is in progress.
    pub ended_at: Option<u64>,
    /// The entity change, press or signal that fired the run; `None` for a manual, schedule or
    /// sun run.
    pub trigger: Option<TriggerInfo>,
    /// Started by hand (the flow's Run button or `run_flow`) rather than by a trigger.
    #[serde(default)]
    pub manual: bool,
    pub steps: Vec<StepView>,
}

/// Where a run is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Done,
    Failed,
    Cancelled,
    Skipped,
}

/// The state change that fired a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TriggerInfo {
    /// `None` for a run started from a Manual card that carries a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// The value inside the entity's state that changed (`battery` …); absent for its headline value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub value: Value,
    pub previous: Value,
}

/// One node's turn within a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepView {
    pub node: String,
    /// Unix seconds.
    pub at: u64,
    pub outcome: StepOutcome,
    pub detail: String,
    /// The output ports the run followed, for `if`, `switch` and `time.between` steps that
    /// succeeded (possibly none); absent for every other node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<String>>,
}

/// How a step ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Ok,
    Failed,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WsEvent;
    use serde_json::json;

    #[test]
    fn run_view_round_trips() {
        let empty = WsEvent::FlowsChanged { flows: vec![] };
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            json!({"type": "flows_changed", "flows": []})
        );

        let run = RunView {
            id: "r1".into(),
            flow_id: "f".into(),
            status: RunStatus::Running,
            started_at: 10,
            ended_at: None,
            trigger: Some(TriggerInfo {
                entity: Some("motion".into()),
                field: None,
                value: json!(true),
                previous: json!(false),
            }),
            manual: false,
            steps: vec![StepView {
                node: "n1".into(),
                at: 11,
                outcome: StepOutcome::Ok,
                detail: "sent".into(),
                ports: None,
            }],
        };
        let event = WsEvent::FlowRunChanged {
            flow_id: "f".into(),
            run,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "flow_run_changed");
        assert_eq!(value["run"]["status"], "running");
        assert_eq!(value["run"]["steps"][0]["outcome"], "ok");
        let back: WsEvent = serde_json::from_value(value).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn trigger_info_entity_is_optional() {
        let with: TriggerInfo =
            serde_json::from_value(json!({"entity": "x", "value": true, "previous": false}))
                .unwrap();
        assert_eq!(with.entity.as_deref(), Some("x"));
        let without: TriggerInfo =
            serde_json::from_value(json!({"value": true, "previous": null})).unwrap();
        assert_eq!(without.entity, None);
        let out = serde_json::to_value(&without).unwrap();
        assert!(out.get("entity").is_none());
    }
}
