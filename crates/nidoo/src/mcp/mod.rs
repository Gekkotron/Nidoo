//! The MCP server: forty-five tools and the `nidoo://snapshot` resource over the hub, served at
//! `/mcp` over streamable HTTP with stateful sessions, and the stdio proxy in front of it.
//!
//! A failed tool call is answered in-band (`isError: true` with the reason as text) rather
//! than as a JSON-RPC error, so the assistant reads why and retries with better arguments.

mod alarm_tools;
mod ambiance_tools;
mod climate_tools;
mod flow_tools;
mod params;
mod stdio_proxy;
mod tools;

use std::sync::Arc;

use nidoo_core::model::UiOverride;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::model::{
    CallToolResponse, CallToolResult, ContentBlock, ErrorData, Implementation, ListResourcesResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    Resource, ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, tool_handler};

use crate::hub::{Hub, HubError};

pub use stdio_proxy::run_stdio_proxy;

/// The resource carrying the whole house as one JSON document.
pub const SNAPSHOT_URI: &str = "nidoo://snapshot";

/// The MCP server over one hub; one instance serves one session.
pub struct NidooMcp {
    hub: Arc<Hub>,
    tool_router: ToolRouter<NidooMcp>,
}

impl NidooMcp {
    /// The setup context, or a tool error while the hub runs without one.
    fn setup_ctx(&self) -> Result<&Arc<crate::setup::SetupCtx>, ToolError> {
        self.hub
            .setup
            .get()
            .ok_or_else(|| ToolError("the settings are not available on this hub".into()))
    }

    pub fn new(hub: Arc<Hub>) -> Self {
        Self {
            hub,
            tool_router: Self::tool_router()
                + Self::flow_tool_router()
                + Self::ambiance_tool_router()
                + Self::climate_tool_router()
                + Self::alarm_tool_router(),
        }
    }
}

/// The streamable HTTP service to nest at `/mcp`, with stateful sessions kept in memory.
///
/// Any `Host` is accepted: rmcp's DNS-rebinding guard (loopback hosts only by default) is
/// switched off. That is acceptable in v1 only because the REST API on the same router is
/// equally open, with permissive CORS and no auth, and the hub is reached by LAN name or
/// address. Revisit with auth: prefer `with_allowed_hosts([...])` derived from the configured
/// bind address or hostname.
pub fn mcp_service(hub: Arc<Hub>) -> StreamableHttpService<NidooMcp, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    StreamableHttpService::new(
        move || Ok(NidooMcp::new(Arc::clone(&hub))),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for NidooMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("nidoo", env!("CARGO_PKG_VERSION")))
        .with_instructions(instructions())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let snapshot = Resource::new(SNAPSHOT_URI, "snapshot")
            .with_title("House snapshot")
            .with_description(
                "The whole house at this instant: connection, bridge, areas, devices, \
                 entities with their states, and the ignored discovery topics.",
            )
            .with_mime_type("application/json");
        Ok(ListResourcesResult::with_all_items(vec![snapshot]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != SNAPSHOT_URI {
            return Err(ErrorData::resource_not_found(
                format!("unknown resource `{}`", request.uri),
                None,
            ));
        }
        let text = serde_json::to_string(&self.hub.snapshot())
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        let contents =
            ResourceContents::text(text, SNAPSHOT_URI).with_mime_type("application/json");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

/// One paragraph telling the assistant what the house is and how the tools behave.
fn instructions() -> String {
    format!(
        "Nidoo is the hub of a Zigbee2MQTT home behind a Mosquitto broker: devices are the \
         Zigbee nodes, entities are their Home Assistant discovery entries (lights, switches, \
         covers, climates, sensors and so on) and areas are the rooms. `list_entities` hides \
         diagnostic, config, disabled-by-default and folded entities unless you pass \
         `include: \"all\"`; use `search` to find a device or an entity from a few words (a name, a room, a kind, French or English) and `get_entity` \
         to see an entity's MQTT bindings (topics, templates) and its raw override. \
         `set_override` writes the Android app's `nidoo/ui/<id>` override (fields: {fields}; \
         `null` clears a field) and `clear_override` removes it. `send_command` publishes to \
         the device and reports `accepted: true` once the next state echo confirms it, or \
         `accepted: false` with the reason; a generic entity (lock, fan, siren, scene, text and so on) \
         takes `{{\"type\":\"control\",\"control\":\"<name>\",\"value\":...}}` with a name from its \
         `capabilities.generic.controls`, and a control with no state to echo is accepted as soon \
         as it is published. The `nidoo://snapshot` resource is the whole house \
         as one JSON document. Automations are flows: `list_flows` and `get_flow` read them, \
         `set_flow` creates or replaces one (validated against the house, saved retained on \
         `nidoo/automation/<id>`, the prefix being configurable), `set_flow_enabled` \
         switches it and `delete_flow` removes it; `run_flow` starts a manual run. A flow's optional `area` (an area id; a room name is resolved) is the room the UI lists it under; it does not change what the flow does. `set_virtual` and `delete_virtual` manage virtual \
         entities (a `switch`, `number`, `button` or `select` the hub owns, as `virtual_<id>`) for flows \
         to read and write. `list_groups`, `set_group` and `delete_group` manage flexible \
         groups (hub-published entities as `group_<id>` whose definition — kind, members, \
         hideMembers — is retained at `nidoo/group/<id>/definition`); the hub fans group \
         commands out to member entities and republishes the combined state as members \
         change. `list_network_watches`, `set_network_watch` and `delete_network_watch` manage \
         network watches: a device the hub pings, published as the connectivity sensor \
         `network_<id>` (disconnected after three missed pings in a row); pause one with \
         `enabled: false`; a device has at most one watch, and its id is the device id. \
         `system_status` reads the hub's own health: CPU, memory, disk, database size, the \
         broker, Zigbee2MQTT and database services, and the restart log. \
         Ambiances are one-tap house states: `list_ambiances`, `set_ambiance` (create or replace, validated against the house, retained on `<ui_prefix>/ambiances/<id>`), `delete_ambiance` and `apply_ambiance` (runs the actions once, in order, and returns one report line per action: `ok`, `failed` or `skipped` with the entities and reasons). An action is `category` (`what`: `lights`, `plugs` or `covers`; `to`: `on`/`off` or `open`/`close`; optional `brightness` in %, `area`, `except`. The set is resolved at apply time, so new devices are included; only switches typed `showAs: plug` are plugs; an untyped switch is a plain switch and is left out), `device` (`entity` plus the same `command` JSON as `send_command`), `climate` (`zone` or null for every active zone, `setpoint` and/or `mode` for `hours`, or `clear: true`) or `flow` (`flow` id, `do`: `enable`, `disable` or `run`). \
         A flow is a \
         graph of nodes: `trigger.state` (an entity's state changes), `trigger.signal` (a remote press, an event or a tag scan: `entity` is a `device_automation.*`, `event.*` or `tag.*` id and `signal` optionally names one of its announced signals; the trigger value is the signal name, and `trigger.state` is refused on those entities), `trigger.schedule` (a \
         5-field cron in the house local time; it fires only while the flow is enabled and \
         the first run is at the next tick), `flow.enable` (sets another flow's `enabled`, or its own flow when `flow` is absent; a flow \
         that disables itself still finishes the current run), `if` (compares a value; edges \
         leaving it name the port `true` or `false`; `missing` is `fail` (the default), `true` \
         or `false` and picks that port when the value is unknown; `average` (`15m`, at most `24h`) compares the mean of the entity's readings over that window, the newest included, instead of its current value; `value` must be a number), `switch` (1 to 10 ordered `rules` of `op` `eq`, `ne`, `gt`, `lt`, `gte`, `lte` or `between` (`value` to `to`, both included) on `entity` or the triggering value, with optional `average` as for `if`; edges leaving it name the port `\"1\"` to `\"n\"` (the rule's position) or `\"otherwise\"`; `otherwise: true` adds that port, `all: true` takes every matching rule), `time.between` (`from` included, `to` excluded, `HH:MM` in the house local time; ports `in` and `out`), `delay` (`for`: `5m`, `1h30m`; \
         with `up_to` it waits a random time between the two), \
         `command` (the same JSON as `send_command`), `notify` (a phone push: `title`, optional `body`, `channel`, `severity`, `entity_id`, \
         `image_url`, `silent`, `id`, `lang`) and `speak` (says `text` on the speaker satellites of `room`, an area id or name, or of every room without one), `ask` (says the question `text` in `room` or the house and listens: `answers` is 1 to 6 `{{label, words}}`, the card leaves by port `1`..`n` for the answer heard or by `none` when nothing was understood or speech input is off; a number heard is kept under `name`, or `default` when none was said, for later cards). The `title` and `body` of `notify` and the `text` of `speak` and `ask` take placeholders filled at \
         run time: `{{entity_id}}` is the entity's value with its unit (`{{weather_temperature}}` \
         → `10.1 °C`), `{{trigger}}` the value that started the run, `:N` rounds to N decimals \
         (`{{weather_temperature:0}}`), and `+ - * /` with spaces around them plus parentheses do \
         maths (`{{weather_today_max - weather_today_min:1}}`). A `math` card computes one number \
         from its `inputs` (entity readings, optionally `average`) with a `formula` such as \
         `clamp(heater - 100 - grid, 0, 1650)` and a result `name`; `save_to` (a number entity) \
         receives it. Later `if` and `switch` cards compare it with `result`, a `command` sends it \
         with `value_from`, and a notify text prints it as `{{name}}`, all within the same run. Text and on/off values are \
         written in `lang` (`en`, the default, or `fr`: pass the language the user writes in). \
         `{{{{` and `}}}}` are literal braces; a value that is unknown at run time prints `?`. \
         `if`, `switch` and `trigger.state` take an optional `field` naming a value inside the \
         entity's state (vacuum `battery`, `fan_speed`; light `brightness` in %; cover \
         `position`; climate and water heater `current`, `target` …); `average` works on a \
         field only when it is the entity's recorded number (cover position, current \
         temperature). Diagnostic entities (battery, link quality) are ordinary entities: name \
         them in `entity`. \
         Entity ids come from `search` or `list_entities`. For \"N minutes after the last \
         motion\" use `mode: restart`, so \
         every new trigger restarts the wait; the default `single` ignores triggers while a \
         run is active. Sun events use `trigger.sun` with `which` one of `sunrise`, `sunset`, \
         `dusk` (civil), `dawn` (civil), and an optional `offset_minutes` in [-720, 720]. A \
         `trigger.manual` card (optional `value`: a bool, number or string the run carries as its \
         trigger value) starts a run only by hand: `run_flow` with its id as `node`. `if` \
         with `op: is_dark` is true between civil dusk and civil dawn at the configured point; its \
         optional `offset_minutes` in [-720, 720] moves dusk by that much and dawn by the \
         opposite (`-30`: dark 30 minutes earlier in the evening and until 30 minutes later in \
         the morning). \
         Off-peak: `trigger.off_peak` with `edge` `start` or `end` fires when off-peak starts \
         or ends (trigger value `\"start\"` / `\"end\"`), and `if` with `op: is_off_peak` (no \
         operand) is true during off-peak. Off-peak comes from the home settings' \
         `off_peak_device` (`{{entity, values}}`: the entity whose state, matched as `on`/`off`, \
         a plain number or its text, is one of `values`) when that entity has a usable \
         reading, else from the `off_peak` windows; with neither it is never off-peak. \
         `trigger.silent` (`entity`, `for` from `1m` to `168h`) fires once when the entity has \
         sent no report for `for`; it fires again only after the entity reports and goes silent \
         again. A report is any state message on the entity's state topic, changed or not, even \
         one that carries no value for it. The clocks start when the flow \
         is saved or enabled and when the broker link returns, are frozen while the link is \
         down (a link that reconnects more often than `for` keeps restarting them), and are \
         forgotten by a hub restart. It is refused on buttons and signal entities. The trigger value is the `for` text. \
         `limit` (`max` 1 to 100, `per` `1m` to `168h`) lets a run through at most `max` times \
         in any rolling `per` window and ends the run there once the limit is reached \
         (nothing after it runs); every run that reaches it counts, cancelled and manual runs \
         included, so place it right before the action; its counts live in memory and reset on \
         a hub restart. A typical alive check is `trigger.silent` then `limit` then `notify`. A `trigger.state` \
         with a `to` (and no `from`) also fires when the value was unknown, confirmed by an \
         earlier state message on its topic, and then becomes `to`; it never fires on the first value seen after boot. \
         Set the house location in the settings; the hub falls back to \
         Paris otherwise and reports a warning on save. `get_history` returns one slice of an entity's history — a chart \
         series (`what: series` with `range`), kWh per day (`daily` with `days` and \
         `kind`), kWh since midnight (`today` with `kind`), or `last_change` — when \
         `[history] url` is set in the hub's config. `send_notification` sends a phone push (`title` \
         required; `channel` alarm, security, camera, energy, climate, system or misc; `severity` \
         info, warning or critical; the same `id` replaces an earlier alert) and `list_alerts` \
         returns the recent ones, newest first. `speak` says a sentence on the speaker satellites \
         of one `room` or of every room (only the Allowed and Connected ones). Climate (when `[climate] enabled`): each area with a \
         climate device is a zone (`off`, `pending`, `active`). `climate_status` shows every \
         zone's reading, decision and why (`source`, `reason`), overrides and pending proposal. \
         The engine learns rules from history and proposes them; nothing applies until \
         `approve_climate_proposal` (or the push button). Safety (frost, overheat, stale \
         readings) always acts. A temperature set by hand holds until the zone's next scheduled \
         change, at most `manual_hold_max_min`. Eco: `set_climate_settings` holds `eco` (`heat`, \
         `cool`, `actions` such as `{{\"fan_mode\":\"low\"}}`), never above a zone's schedule; \
         `set_climate_eco` puts one zone in eco until a Unix time and `clear_climate_eco` ends it; \
         the hub's alarm (`nidoo_alarm_house`, listed in `presence` or not) or an alarm panel in \
         `presence` that is armed away means nobody home, whatever a tracker says and with no \
         `away_grace`, so every zone is in eco. `get_weather` returns the outdoor weather when \
         `[weather] enabled`. `get_solar` returns the Solcast solar forecast (today, tomorrow, the \
    next days, the request budget) when `[solcast] enabled`; its `solar_…` entities carry the same \
    numbers. `get_home_settings` and `set_home_settings` read and replace the whole \
    home settings document: `eco_setpoint` (°C), `mode_entity` (a select), `indoor_entity` (a \
    temperature sensor), `cold_below` and `hot_above` (°C, the web reads cold below 15 and warm \
    above 26 when unset), `energy_view` (`classic`, `flux` or `orbite`, the web Energy page's live \
    diagram). The native alarm is optional (`enabled` in its settings): \
    `alarm_status` reads the settings and the panel state, `set_alarm_settings` replaces the \
    whole document (read it first). Arm and disarm with `send_command` on `nidoo_alarm_house`, \
    `{{\"type\":\"arm\",\"mode\":\"away\"}}` or `{{\"type\":\"disarm\"}}`; there are no codes.",
        fields = UiOverride::FIELDS.join(", ")
    )
}

/// Why a tool call failed, reported in-band with the reason as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError(pub String);

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        Ok(CallToolResult::error(vec![ContentBlock::text(self.0)]).into())
    }
}

impl From<HubError> for ToolError {
    /// A rejected patch names the valid fields (and, through `PatchError`'s own message, the
    /// valid values of the rejected one); the other errors keep the hub's wording.
    fn from(err: HubError) -> Self {
        match err {
            HubError::Patch(patch) => ToolError(format!(
                "{patch}; the override fields are {}",
                UiOverride::FIELDS.join(", ")
            )),
            other => ToolError(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{ENERGY_ROLES, PatchError};

    #[test]
    fn patch_errors_list_the_fields_and_the_valid_values() {
        let unknown = ToolError::from(HubError::Patch(PatchError::UnknownField("nope".into())));
        assert!(
            unknown.0.starts_with("unknown override field `nope`"),
            "{}",
            unknown.0
        );
        for field in UiOverride::FIELDS {
            assert!(
                unknown.0.contains(field),
                "{field} missing from {}",
                unknown.0
            );
        }

        let invalid = ToolError::from(HubError::Patch(PatchError::InvalidValue {
            field: "energyRole".into(),
            expected: format!("one of {}", ENERGY_ROLES.join(", ")),
        }));
        assert!(invalid.0.contains("energyRole"), "{}", invalid.0);
        assert!(invalid.0.contains("grid_export"), "{}", invalid.0);
    }

    #[test]
    fn other_hub_errors_keep_their_wording() {
        assert_eq!(
            ToolError::from(HubError::UnknownEntity("x".into())).0,
            "unknown entity `x`"
        );
        assert_eq!(
            ToolError::from(HubError::Rejected("entity is offline".into())).0,
            "rejected: entity is offline"
        );
    }

    #[test]
    fn instructions_mention_the_tools_and_the_fields() {
        let text = instructions();
        for needle in [
            "list_entities",
            "set_override",
            "send_command",
            "search",
            "get_entity",
        ] {
            assert!(text.contains(needle), "{needle} missing");
        }
        assert!(text.contains("lightOnBrightness"));
        assert!(text.contains(SNAPSHOT_URI));
    }

    #[test]
    fn mcp_instructions_mention_off_peak_nodes() {
        let instr = instructions();
        assert!(instr.contains("trigger.off_peak"));
        assert!(instr.contains("is_off_peak"));
        assert!(instr.contains("off_peak_device"));
    }

    #[test]
    fn mcp_instructions_mention_silent_and_limit() {
        let instr = instructions();
        assert!(instr.contains("trigger.silent"));
        assert!(instr.contains("limit"));
        assert!(instr.contains("unknown"));
    }

    #[test]
    fn mcp_instructions_mention_trigger_sun_and_is_dark() {
        let instr = instructions();
        assert!(instr.contains("trigger.sun"));
        assert!(instr.contains("is_dark"));
        assert!(instr.contains("Paris"));
    }

    #[test]
    fn instructions_mention_flows() {
        let text = instructions();
        for needle in [
            "set_flow",
            "restart",
            "nidoo/automation",
            "run_flow",
            "`math`",
            "value_from",
            "{name}",
        ] {
            assert!(text.contains(needle), "{needle} missing");
        }
    }

    #[test]
    fn instructions_explain_notify_placeholders() {
        let text = instructions();
        for needle in ["{trigger}", "`lang`", ":N", "{{"] {
            assert!(text.contains(needle), "{needle} missing");
        }
    }
}
