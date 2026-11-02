//! The six flow tools. They live beside the eleven house tools of `tools.rs` in a second router, combined
//! in [`NidooMcp::new`], and share their conventions: lists are wrapped in objects, write tools
//! go through `log_write`, and a refusal is answered in-band.

use nidoo_api::{FlowView, FlowWriteOutcome, RunView};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};

use super::params::{
    DeleteFlowParams, Deleted, FlowList, FlowSummary, GetFlowParams, LastRun, RunFlowParams,
    SetFlowEnabledParams, SetFlowParams,
};
use super::tools::{ToolResult, log_write};
use super::{NidooMcp, ToolError};

#[tool_router(router = flow_tool_router, vis = "pub(super)")]
impl NidooMcp {
    /// The automations (flows), each with its name, whether it is enabled, why it is invalid
    /// if it is, its node count and its latest run. `get_flow` returns one in full.
    #[tool(name = "list_flows", annotations(read_only_hint = true))]
    async fn list_flows(&self) -> Json<FlowList> {
        let flows = self.hub.flows().into_iter().map(summary).collect();
        Json(FlowList { flows })
    }

    /// One flow: its definition, why it is invalid if it is, the run in progress and the recent
    /// runs (newest first) with each node's outcome.
    #[tool(name = "get_flow", annotations(read_only_hint = true))]
    async fn get_flow(
        &self,
        Parameters(params): Parameters<GetFlowParams>,
    ) -> ToolResult<FlowView> {
        self.hub
            .flow(&params.id)
            .map(Json)
            .ok_or_else(|| ToolError(format!("unknown flow `{}`", params.id)))
    }

    /// Creates or replaces an automation: a trigger on an entity's state, optional conditions
    /// and delays, and commands. It is validated against the house (entities must exist and
    /// support the commands) and saved retained on `nidoo/automation/<id>`; the reason of a
    /// refusal is returned so you can fix the flow and retry.
    #[tool(name = "set_flow", annotations(idempotent_hint = true))]
    async fn set_flow(
        &self,
        Parameters(params): Parameters<SetFlowParams>,
    ) -> ToolResult<FlowWriteOutcome> {
        log_write("set_flow", &params);
        Ok(Json(self.hub.set_flow(params.flow).await?))
    }

    /// Removes a flow, valid or not, by clearing its retained definition.
    #[tool(name = "delete_flow", annotations(idempotent_hint = true))]
    async fn delete_flow(
        &self,
        Parameters(params): Parameters<DeleteFlowParams>,
    ) -> ToolResult<Deleted> {
        log_write("delete_flow", &params);
        self.hub.delete_flow(&params.id).await?;
        Ok(Json(Deleted { id: params.id }))
    }

    /// Turns a flow on or off without touching its definition. A disabled flow never runs.
    #[tool(name = "set_flow_enabled", annotations(idempotent_hint = true))]
    async fn set_flow_enabled(
        &self,
        Parameters(params): Parameters<SetFlowEnabledParams>,
    ) -> ToolResult<FlowView> {
        log_write("set_flow_enabled", &params);
        Ok(Json(
            self.hub
                .set_flow_enabled(&params.id, params.enabled)
                .await?,
        ))
    }

    /// Starts a manual run of a flow, without waiting for its trigger, and returns the run as
    /// it starts; read its outcome with `get_flow`. Refused when the flow is
    /// disabled, invalid or already running; a card press (`node`) on a `restart` flow replaces
    /// the active run instead. `node` names a `trigger.manual` card to start from (the run
    /// carries its value); refused when it is not one.
    #[tool(name = "run_flow")]
    async fn run_flow(&self, Parameters(params): Parameters<RunFlowParams>) -> ToolResult<RunView> {
        log_write("run_flow", &params);
        Ok(Json(
            self.hub.run_flow_from(&params.id, params.node.as_deref())?,
        ))
    }
}

fn summary(view: FlowView) -> FlowSummary {
    FlowSummary {
        id: view.definition.id,
        name: view.definition.name,
        enabled: view.definition.enabled,
        area: view.definition.area,
        invalid: view.invalid,
        nodes: view.definition.nodes.len(),
        last_run: view.runs.first().map(|run| LastRun {
            status: run.status,
            started_at: run.started_at,
        }),
    }
}
