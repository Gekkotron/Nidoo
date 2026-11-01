//! The four ambiance tools, in a router of their own like the flow tools: lists are wrapped in
//! objects, write tools go through `log_write`, and a refusal is answered in-band.

use nidoo_api::{AmbianceView, ApplyReport};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};

use super::NidooMcp;
use super::params::{AmbianceIdParams, AmbianceList, Deleted, SetAmbianceParams};
use super::tools::{ToolResult, log_write};

#[tool_router(router = ambiance_tool_router, vis = "pub(super)")]
impl NidooMcp {
    /// The ambiances (one-tap house states), each with its actions, or the reason it cannot be
    /// read.
    #[tool(name = "list_ambiances", annotations(read_only_hint = true))]
    async fn list_ambiances(&self) -> Json<AmbianceList> {
        Json(AmbianceList {
            ambiances: self.hub.ambiances(),
        })
    }

    /// Creates or replaces an ambiance: a name and an ordered list of actions (category,
    /// device, climate or flow). It is validated against the house and saved retained on
    /// `<ui_prefix>/ambiances/<id>`; the reason of a refusal is returned so you can fix it and
    /// retry.
    #[tool(name = "set_ambiance", annotations(idempotent_hint = true))]
    async fn set_ambiance(
        &self,
        Parameters(params): Parameters<SetAmbianceParams>,
    ) -> ToolResult<AmbianceView> {
        log_write("set_ambiance", &params);
        Ok(Json(self.hub.set_ambiance(params.ambiance).await?))
    }

    /// Removes an ambiance, valid or not, by clearing its retained definition.
    #[tool(name = "delete_ambiance", annotations(idempotent_hint = true))]
    async fn delete_ambiance(
        &self,
        Parameters(params): Parameters<AmbianceIdParams>,
    ) -> ToolResult<Deleted> {
        log_write("delete_ambiance", &params);
        self.hub.delete_ambiance(&params.id).await?;
        Ok(Json(Deleted { id: params.id }))
    }

    /// Runs an ambiance's actions once, in order, and returns one report line per action:
    /// `ok`, `failed` or `skipped`, with the entities and reasons. Refused when it is unknown,
    /// invalid or already running, or while the broker is offline.
    #[tool(name = "apply_ambiance")]
    async fn apply_ambiance(
        &self,
        Parameters(params): Parameters<AmbianceIdParams>,
    ) -> ToolResult<ApplyReport> {
        log_write("apply_ambiance", &params);
        Ok(Json(self.hub.apply_ambiance(&params.id).await?))
    }
}
