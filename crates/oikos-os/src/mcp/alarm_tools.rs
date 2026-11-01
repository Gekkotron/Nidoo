//! The two alarm tools, in a fourth router combined in [`NidooMcp::new`]. Arming and disarming
//! are not tools: they are `send_command` on the alarm entity.

use oikos_alarm::view::AlarmView;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};

use super::NidooMcp;
use super::params::SetAlarmSettingsParams;
use super::tools::{ToolResult, log_write};

#[tool_router(router = alarm_tool_router, vis = "pub(super)")]
impl NidooMcp {
    /// The native alarm: its settings (`enabled`, delays, sensors, sirens, remotes, presence),
    /// the panel state (`disarmed`, `arming`, `armed_away`, `armed_home`, `armed_night`, `pending`, `triggered`), why, and the
    /// run-time warnings.
    #[tool(name = "alarm_status", annotations(read_only_hint = true))]
    async fn alarm_status(&self) -> ToolResult<AlarmView> {
        Ok(Json(self.hub.alarm_view()))
    }

    /// Replaces the alarm settings. Read them with `alarm_status` first and send the whole
    /// document back. The reason of a refusal is returned so you can fix the document and retry.
    #[tool(name = "set_alarm_settings", annotations(idempotent_hint = true))]
    async fn set_alarm_settings(
        &self,
        Parameters(params): Parameters<SetAlarmSettingsParams>,
    ) -> ToolResult<AlarmView> {
        log_write("set_alarm_settings", &params);
        self.hub.set_alarm_settings(params.settings).await?;
        Ok(Json(self.hub.alarm_view()))
    }
}
