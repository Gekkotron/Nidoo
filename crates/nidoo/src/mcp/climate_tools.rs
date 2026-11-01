//! The eleven climate tools, in a third router combined in [`NidooMcp::new`]. While
//! `[climate] enabled` is false every one answers `climate disabled` in-band.

use nidoo_climate::view::{ClimateView, LearnStatus, RoomEcoRequest};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};

use super::params::{
    ApproveClimateProposalParams, ClimateAck, ClimateZoneDetail, ClimateZoneParams,
    SetClimateEcoParams, SetClimateOverrideParams, SetClimateSettingsParams, SetClimateZoneParams,
};
use super::tools::{ToolResult, log_write};
use super::{NidooMcp, ToolError};
use crate::clock;

/// How many decisions `get_climate_zone` returns.
const ZONE_DECISIONS: usize = 20;

#[tool_router(router = climate_tool_router, vis = "pub(super)")]
impl NidooMcp {
    /// The climate engine: the house settings and every zone with its live reading, its
    /// decision and why (`source`, `reason`), its overrides, its pending proposal and where
    /// the learner stands.
    #[tool(name = "climate_status", annotations(read_only_hint = true))]
    async fn climate_status(&self) -> ToolResult<ClimateView> {
        Ok(Json(self.hub.climate_view()?))
    }

    /// One zone as `climate_status` shows it, with its last 20 decisions, newest first.
    #[tool(name = "get_climate_zone", annotations(read_only_hint = true))]
    async fn get_climate_zone(
        &self,
        Parameters(params): Parameters<ClimateZoneParams>,
    ) -> ToolResult<ClimateZoneDetail> {
        let view = self.hub.climate_view()?;
        let zone = view
            .zones
            .into_iter()
            .find(|z| z.zone == params.zone)
            .ok_or_else(|| ToolError(format!("unknown zone `{}`", params.zone)))?;
        let decisions = self
            .hub
            .climate_decision_log(&params.zone, ZONE_DECISIONS)
            .await?
            .recent;
        Ok(Json(ClimateZoneDetail { zone, decisions }))
    }

    /// Replaces the house settings (season and outdoor sensor, presence and away grace,
    /// holiday, safety thresholds, stale-reading limit, learner). Read them with
    /// `climate_status` first and send the whole document back.
    #[tool(name = "set_climate_settings", annotations(idempotent_hint = true))]
    async fn set_climate_settings(
        &self,
        Parameters(params): Parameters<SetClimateSettingsParams>,
    ) -> ToolResult<ClimateView> {
        log_write("set_climate_settings", &params);
        self.hub.set_climate_house(params.settings)?;
        Ok(Json(self.hub.climate_view()?))
    }

    /// Replaces a zone's settings (`state` `off`, `pending` or `active`, targets, rules). New
    /// non-empty rules count as approved: a pending zone becomes active. A device another zone
    /// lists in `entities` is refused. The reason of a refusal is returned so you can fix the
    /// document and retry.
    #[tool(name = "set_climate_zone", annotations(idempotent_hint = true))]
    async fn set_climate_zone(
        &self,
        Parameters(params): Parameters<SetClimateZoneParams>,
    ) -> ToolResult<ClimateView> {
        log_write("set_climate_zone", &params);
        self.hub.set_climate_zone(&params.zone, params.settings)?;
        Ok(Json(self.hub.climate_view()?))
    }

    /// Approves the pending proposal of a zone: its rules, or `rules` when you edit them,
    /// become the zone's schedule. Refused when there is no proposal.
    #[tool(name = "approve_climate_proposal")]
    async fn approve_climate_proposal(
        &self,
        Parameters(params): Parameters<ApproveClimateProposalParams>,
    ) -> ToolResult<ClimateAck> {
        log_write("approve_climate_proposal", &params);
        self.hub
            .approve_climate_proposal(&params.zone, params.rules)
            .await?;
        Ok(Json(ClimateAck { zone: params.zone }))
    }

    /// Rejects the pending proposal of a zone; the same proposal is not made again for a week.
    #[tool(name = "reject_climate_proposal")]
    async fn reject_climate_proposal(
        &self,
        Parameters(params): Parameters<ClimateZoneParams>,
    ) -> ToolResult<ClimateAck> {
        log_write("reject_climate_proposal", &params);
        self.hub.reject_climate_proposal(&params.zone).await?;
        Ok(Json(ClimateAck { zone: params.zone }))
    }

    /// Holds a temperature and/or a mode on a zone, for `minutes` from now or `until` a Unix
    /// time (exactly one of the two). Safety still acts.
    #[tool(name = "set_climate_override")]
    async fn set_climate_override(
        &self,
        Parameters(params): Parameters<SetClimateOverrideParams>,
    ) -> ToolResult<ClimateAck> {
        log_write("set_climate_override", &params);
        let ov = params
            .request
            .into_override(clock::now())
            .map_err(|reason| ToolError(format!("rejected: {reason}")))?;
        self.hub.set_climate_override(&params.zone, ov).await?;
        Ok(Json(ClimateAck { zone: params.zone }))
    }

    /// Removes every override of a zone, so its schedule applies again.
    #[tool(name = "clear_climate_override", annotations(idempotent_hint = true))]
    async fn clear_climate_override(
        &self,
        Parameters(params): Parameters<ClimateZoneParams>,
    ) -> ToolResult<ClimateAck> {
        log_write("clear_climate_override", &params);
        self.hub.clear_climate_override(&params.zone).await?;
        Ok(Json(ClimateAck { zone: params.zone }))
    }

    /// Puts a zone in eco until a Unix time: it holds the house eco setpoint (never above its
    /// schedule) and the eco actions, then goes back to its schedule by itself.
    #[tool(name = "set_climate_eco")]
    async fn set_climate_eco(
        &self,
        Parameters(params): Parameters<SetClimateEcoParams>,
    ) -> ToolResult<ClimateAck> {
        log_write("set_climate_eco", &params);
        let eco = RoomEcoRequest {
            until: params.until,
        }
        .into_room_eco(clock::now())
        .map_err(|reason| ToolError(format!("rejected: {reason}")))?;
        self.hub.set_climate_eco(&params.zone, eco).await?;
        Ok(Json(ClimateAck { zone: params.zone }))
    }

    /// Ends a zone's eco, so its schedule applies again.
    #[tool(name = "clear_climate_eco", annotations(idempotent_hint = true))]
    async fn clear_climate_eco(
        &self,
        Parameters(params): Parameters<ClimateZoneParams>,
    ) -> ToolResult<ClimateAck> {
        log_write("clear_climate_eco", &params);
        self.hub.clear_climate_eco(&params.zone).await?;
        Ok(Json(ClimateAck { zone: params.zone }))
    }

    /// Learns a zone's rules from its history now and returns where the learner stands
    /// (`proposed` when there is a proposal to approve).
    #[tool(name = "learn_climate_zone")]
    async fn learn_climate_zone(
        &self,
        Parameters(params): Parameters<ClimateZoneParams>,
    ) -> ToolResult<LearnStatus> {
        log_write("learn_climate_zone", &params);
        Ok(Json(self.hub.learn_climate_zone(&params.zone).await?))
    }
}
