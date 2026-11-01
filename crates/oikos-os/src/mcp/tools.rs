//! The twenty-two tools of the house (the six flow tools are in `flow_tools.rs`). Read tools work on one snapshot (one read lock, released before any
//! await); write tools log their arguments and go through the hub's writers.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use oikos_api::{
    CommandResponse, DeviceView, EntityView, GridDay, GroupView, HomeSettings, NetworkWatchView,
    WeatherView,
};
use oikos_core::flow::{notify_channel_error, notify_severity_error};
use oikos_core::model::{AreasDocument, SwitchType};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use serde::Serialize;
use serde_json::Value;

use super::params::{
    AlertList, AreaList, BridgeStatus, ClearOverrideParams, DEFAULT_ALERTS_LIMIT, DEFAULT_LIMIT,
    DEFAULT_SEARCH_LIMIT, DeleteGroupParams, DeleteNetworkWatchParams, DeleteVirtualParams,
    Deleted, DeviceList, EntityDetailView, EntityPage, GetEntityParams, GetGridDayParams,
    GetHistoryParams, GetSettingsParams, GroupList, HistoryAnswer, HistoryWhat, IgnoredList,
    Include, ListAlertsParams, ListDevicesParams, ListEntitiesParams, MAX_LIMIT, NetworkWatchList,
    SearchParams, SearchResult, SendCommandParams, SendNotificationParams, Sent, SetAreasParams,
    SetGroupParams, SetHomeSettingsParams, SetNetworkWatchParams, SetOverrideParams,
    SetSettingsParams, SetVirtualParams,
};
use super::{OikosMcp, ToolError};
use crate::history::query as history_query;
use crate::setup::settings::{ApiError, read_section, write_section};

pub(super) type ToolResult<T> = Result<Json<T>, ToolError>;

fn api_error(e: ApiError) -> ToolError {
    ToolError(e.message)
}

#[tool_router(vis = "pub(super)")]
impl OikosMcp {
    /// The outdoor weather from Open-Meteo: now, today, rain in the next hour, tonight's low and
    /// the next five days with their full figures (metric units; `stale` is true when the data is old). Answers
    /// `weather unavailable` while `[weather]` is off or before the first fetch.
    #[tool(name = "get_weather", annotations(read_only_hint = true))]
    async fn get_weather(&self) -> ToolResult<WeatherView> {
        Ok(Json(self.hub.weather_view()?))
    }

    /// The Solcast solar forecast: today, tomorrow, the next 7 days, 30-minute periods and the
    /// request budget; `solar unavailable` while `[solcast]` is off or before the first fetch.
    #[tool(name = "get_solar", annotations(read_only_hint = true))]
    async fn get_solar(&self) -> ToolResult<oikos_api::SolarView> {
        Ok(Json(self.hub.solar_view_with_history().await?))
    }

    /// The home settings: the eco setpoint (°C), the house mode select and the indoor
    /// temperature sensor the home header uses. Empty fields are unset.
    #[tool(name = "get_home_settings", annotations(read_only_hint = true))]
    async fn get_home_settings(&self) -> Json<HomeSettings> {
        Json(self.hub.home_settings())
    }

    /// Replaces the home settings (`eco_setpoint` 5 to 30 °C, `mode_entity` a select entity,
    /// `indoor_entity` a temperature sensor, `cold_below` and `hot_above` 5 to 35 °C with
    /// `cold_below` lower; the web reads cold below 15 and warm above 26 when unset; `off_peak`
    /// a list of `{"start": "HH:MM", "end": "HH:MM"}` local off-peak windows, `end` before
    /// `start` crossing midnight, none overlapping, empty for a single tariff; a change applies
    /// from the start of today and past days keep their windows in `off_peak_history`, which the
    /// hub keeps and ignores when sent; `energy_view` `classic`, `flux` or `orbite`, how the web Energy
    /// page draws the live power). Read them with `get_home_settings` first and send
    /// the whole document back. The reason of a refusal is returned so you can fix it.
    #[tool(name = "set_home_settings", annotations(idempotent_hint = true))]
    async fn set_home_settings(
        &self,
        Parameters(params): Parameters<SetHomeSettingsParams>,
    ) -> ToolResult<HomeSettings> {
        log_write("set_home_settings", &params);
        Ok(Json(self.hub.set_home_settings(params.settings)?))
    }

    /// One section of the hub's service settings (`weather`, `solcast`, `frigate`, `push`,
    /// `climate` or `advanced`), as the settings page shows it. Secrets (the Solcast key, the
    /// Frigate password, the push service account) are write-only: they read as `{"set": bool}`.
    #[tool(name = "get_settings", annotations(read_only_hint = true))]
    async fn get_settings(
        &self,
        Parameters(params): Parameters<GetSettingsParams>,
    ) -> ToolResult<serde_json::Value> {
        let ctx = self.setup_ctx()?;
        Ok(Json(
            read_section(ctx, &params.section)
                .await
                .map_err(api_error)?,
        ))
    }

    /// Replaces one section of the service settings with `value`, checks it, saves it and
    /// restarts the hub so every service picks it up (returns `{"restarting": true}`).
    /// Read the section with `get_settings` first and send the whole of it back: what is left
    /// out takes its default. Secrets are write-only: a string sets one, `null` clears it, and
    /// leaving it out (or sending `""` or back `{"set": true}`) keeps the stored value. Changing a
    /// service's address (the Frigate URL) needs its secret sent again. The reason of a
    /// refusal is returned, and never contains a secret.
    #[tool(name = "set_settings", annotations(idempotent_hint = true))]
    async fn set_settings(
        &self,
        Parameters(params): Parameters<SetSettingsParams>,
    ) -> ToolResult<serde_json::Value> {
        // Not `log_write`: `value` may hold a secret, so only the section is logged.
        tracing::info!(tool = "set_settings", section = %params.section, "mcp write");
        let ctx = self.setup_ctx()?;
        Ok(Json(
            write_section(ctx, &params.section, params.value)
                .await
                .map_err(api_error)?,
        ))
    }

    /// The Zigbee2MQTT bridge (version, coordinator, permit join, online or not) and the
    /// state of the link to the MQTT broker.
    #[tool(name = "bridge_info", annotations(read_only_hint = true))]
    async fn bridge_info(&self) -> Json<BridgeStatus> {
        let snapshot = self.hub.snapshot();
        Json(BridgeStatus {
            bridge: snapshot.bridge,
            connection: snapshot.connection,
        })
    }

    /// The rooms, each with the number of entities in it: the declared areas first, then the
    /// ones only carried by entities, then `Unassigned` when any entity has no room.
    #[tool(name = "list_areas", annotations(read_only_hint = true))]
    async fn list_areas(&self) -> Json<AreaList> {
        Json(AreaList {
            areas: self.hub.snapshot().areas,
        })
    }

    /// Replaces the declared rooms (retained on `oikos/ui/_areas`) and returns the resulting
    /// list. A room kept by id keeps the `icon`, group names, group favourites and other
    /// fields you leave out.
    #[tool(name = "set_areas", annotations(idempotent_hint = true))]
    async fn set_areas(
        &self,
        Parameters(params): Parameters<SetAreasParams>,
    ) -> ToolResult<AreaList> {
        log_write("set_areas", &params);
        let current = self.hub.areas_document();
        let areas = params
            .areas
            .into_iter()
            .map(|mut area| {
                adopt_listed_fields(&mut area);
                if let Some(old) = current.areas.iter().find(|a| a.id == area.id) {
                    area.icon = area.icon.or_else(|| old.icon.clone());
                    if area.group_names.is_empty() {
                        area.group_names = old.group_names.clone();
                    }
                    if area.group_favorites.is_empty() {
                        area.group_favorites = old.group_favorites.clone();
                    }
                    // Key by key: what the caller sends wins, the rest is kept.
                    let mut extra = old.extra.clone();
                    extra.append(&mut area.extra);
                    area.extra = extra;
                }
                area
            })
            .collect();
        let doc = AreasDocument { areas };
        let areas = self.hub.set_areas(doc).await?;
        Ok(Json(AreaList { areas }))
    }

    /// The Zigbee devices with what their diagnostic entities report (availability, battery,
    /// link quality), optionally only those in an area or matching a name, model or vendor.
    #[tool(name = "list_devices", annotations(read_only_hint = true))]
    async fn list_devices(
        &self,
        Parameters(params): Parameters<ListDevicesParams>,
    ) -> Json<DeviceList> {
        let snapshot = self.hub.snapshot();
        let in_area: Option<BTreeSet<&str>> = params.area.as_deref().map(|area| {
            snapshot
                .entities
                .iter()
                .filter(|entity| entity.area == area)
                .filter_map(|entity| entity.device_id.as_deref())
                .collect()
        });
        let query = params.query.as_deref().map(str::to_lowercase);
        let devices = snapshot
            .devices
            .iter()
            .filter(|device| {
                in_area
                    .as_ref()
                    .is_none_or(|ids| ids.contains(device.id.as_str()))
            })
            .filter(|device| query.as_deref().is_none_or(|q| device_matches(device, q)))
            .cloned()
            .collect();
        Json(DeviceList { devices })
    }

    /// One page of entities with their current state. By default only what the app shows
    /// (`include: "primary"`): no diagnostic or config entity, none disabled by default, none
    /// folded into another entity, none hidden by its override. Filters combine.
    #[tool(name = "list_entities", annotations(read_only_hint = true))]
    async fn list_entities(
        &self,
        Parameters(params): Parameters<ListEntitiesParams>,
    ) -> Json<EntityPage> {
        let include = params.include.unwrap_or_default();
        let query = params.query.as_deref().map(str::to_lowercase);
        let matches: Vec<EntityView> = self
            .hub
            .snapshot()
            .entities
            .into_iter()
            .filter(|entity| include == Include::All || entity.is_primary())
            .filter(|entity| params.area.as_deref().is_none_or(|a| entity.area == a))
            .filter(|entity| params.kind.is_none_or(|k| entity.kind == k))
            .filter(|entity| {
                params
                    .device_id
                    .as_deref()
                    .is_none_or(|d| entity.device_id.as_deref() == Some(d))
            })
            .filter(|entity| query.as_deref().is_none_or(|q| entity_matches(entity, q)))
            .filter(|entity| {
                params
                    .changed_since
                    .is_none_or(|since| entity.state.last_update.is_some_and(|at| at >= since))
            })
            .collect();
        let total = matches.len();
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let items = matches
            .into_iter()
            .skip(params.offset.unwrap_or(0))
            .take(limit)
            .collect();
        Json(EntityPage { total, items })
    }

    /// One entity with its state, its MQTT bindings (the expanded discovery config: topics,
    /// templates, payload constants) and the override retained for it, verbatim.
    #[tool(name = "get_entity", annotations(read_only_hint = true))]
    async fn get_entity(
        &self,
        Parameters(params): Parameters<GetEntityParams>,
    ) -> ToolResult<EntityDetailView> {
        let detail = self
            .hub
            .entity_detail(&params.id)
            .ok_or_else(|| ToolError(format!("unknown entity `{}`", params.id)))?;
        Ok(Json(EntityDetailView {
            entity: detail.view,
            bindings: detail.bindings,
            raw_override: detail.raw_override,
        }))
    }

    /// Changes fields of the entity's override (the Android app's `oikos/ui/<id>` document,
    /// retained) and returns the entity as it now appears. Unknown fields and invalid values
    /// are refused and nothing is written.
    #[tool(name = "set_override", annotations(idempotent_hint = true))]
    async fn set_override(
        &self,
        Parameters(params): Parameters<SetOverrideParams>,
    ) -> ToolResult<EntityView> {
        log_write("set_override", &params);
        let view = self.hub.set_override(&params.id, &params.patch).await?;
        Ok(Json(view))
    }

    /// Removes the entity's override entirely and returns the entity as discovered.
    #[tool(name = "clear_override", annotations(idempotent_hint = true))]
    async fn clear_override(
        &self,
        Parameters(params): Parameters<ClearOverrideParams>,
    ) -> ToolResult<EntityView> {
        log_write("clear_override", &params);
        let view = self.hub.clear_override(&params.id).await?;
        Ok(Json(view))
    }

    /// Sends a command to an entity: it is published to the device and `accepted` is true once
    /// the next state echo confirms it (the state is returned), false with the reason when
    /// nothing came back in time. An unknown or offline entity, or a command the entity cannot
    /// take, is refused before anything is published.
    #[tool(name = "send_command")]
    async fn send_command(
        &self,
        Parameters(params): Parameters<SendCommandParams>,
    ) -> ToolResult<CommandResponse> {
        log_write("send_command", &params);
        let response = self.hub.send_command(&params.id, params.command).await?;
        Ok(Json(response))
    }

    /// Finds devices (by friendly name, model or vendor) and entities (by name or id, or
    /// belonging to a matching device) whose text contains the query, case-insensitively.
    #[tool(name = "search", annotations(read_only_hint = true))]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> Json<SearchResult> {
        let query = params.query.to_lowercase();
        let limit = params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).min(MAX_LIMIT);
        let snapshot = self.hub.snapshot();
        let matched_devices: BTreeSet<String> = snapshot
            .devices
            .iter()
            .filter(|device| device_matches(device, &query))
            .map(|device| device.id.clone())
            .collect();
        let entities = snapshot
            .entities
            .into_iter()
            .filter(|entity| {
                entity_matches(entity, &query)
                    || entity
                        .device_id
                        .as_deref()
                        .is_some_and(|id| matched_devices.contains(id))
            })
            .take(limit)
            .collect();
        let devices = snapshot
            .devices
            .into_iter()
            .filter(|device| matched_devices.contains(&device.id))
            .take(limit)
            .collect();
        Json(SearchResult { devices, entities })
    }

    /// Creates or replaces a virtual entity the hub owns: a `switch`, a `number`, a `button`
    /// or a `select` (give its `options`), for flows to read and write (a flag, a setpoint, a
    /// manual trigger, a mode). It is saved retained and returned as an entity with the id
    /// `virtual_<id>`; a switch, number or select is commanded with `send_command` like any
    /// other, and moved to a room with `set_override`. Redefining one keeps its state.
    #[tool(name = "set_virtual", annotations(idempotent_hint = true))]
    async fn set_virtual(
        &self,
        Parameters(params): Parameters<SetVirtualParams>,
    ) -> ToolResult<EntityView> {
        log_write("set_virtual", &params);
        Ok(Json(self.hub.set_virtual(&params.id, params.spec).await?))
    }

    /// Removes a virtual entity and clears what the broker retained for it.
    #[tool(name = "delete_virtual", annotations(idempotent_hint = true))]
    async fn delete_virtual(
        &self,
        Parameters(params): Parameters<DeleteVirtualParams>,
    ) -> ToolResult<Deleted> {
        log_write("delete_virtual", &params);
        self.hub.delete_virtual(&params.id).await?;
        Ok(Json(Deleted { id: params.id }))
    }

    /// Creates or replaces a group: one entity the hub owns, as `group_<id>`, that fans commands
    /// out to its members and presents their combined state. Its definition is retained on
    /// `oikos/group/<id>/definition`; a member of another kind than the group's is flagged.
    #[tool(name = "set_group", annotations(idempotent_hint = true))]
    async fn set_group(
        &self,
        Parameters(params): Parameters<SetGroupParams>,
    ) -> ToolResult<GroupView> {
        log_write("set_group", &params);
        Ok(Json(self.hub.set_group(&params.id, params.spec).await?))
    }

    /// Every group the hub stores, with its members and the flows that use it, sorted by id.
    #[tool(name = "list_groups", annotations(idempotent_hint = true))]
    async fn list_groups(&self) -> ToolResult<GroupList> {
        Ok(Json(GroupList {
            groups: self.hub.list_groups()?,
        }))
    }

    /// Removes a group and clears what the broker retained for it. The UI override is kept.
    #[tool(name = "delete_group", annotations(idempotent_hint = true))]
    async fn delete_group(
        &self,
        Parameters(params): Parameters<DeleteGroupParams>,
    ) -> ToolResult<Deleted> {
        log_write("delete_group", &params);
        self.hub.delete_group(&params.id).await?;
        Ok(Json(Deleted { id: params.id }))
    }

    /// Creates or replaces a network watch: a device pinged by the hub, presented as the
    /// `binary_sensor` `network_<id>` (connected after one reply, disconnected after three missed
    /// pings). Its definition is retained on `oikos/network/<id>/definition`.
    #[tool(name = "set_network_watch", annotations(idempotent_hint = true))]
    async fn set_network_watch(
        &self,
        Parameters(params): Parameters<SetNetworkWatchParams>,
    ) -> ToolResult<NetworkWatchView> {
        log_write("set_network_watch", &params);
        Ok(Json(
            self.hub.set_network_watch(&params.id, params.spec).await?,
        ))
    }

    /// Every network watch the hub stores, with its entity and live status, sorted by id.
    #[tool(name = "list_network_watches", annotations(read_only_hint = true))]
    async fn list_network_watches(&self) -> ToolResult<NetworkWatchList> {
        Ok(Json(NetworkWatchList {
            watches: self.hub.list_network_watches()?,
        }))
    }

    /// Removes a network watch and clears what the broker retained for it.
    #[tool(name = "delete_network_watch", annotations(idempotent_hint = true))]
    async fn delete_network_watch(
        &self,
        Parameters(params): Parameters<DeleteNetworkWatchParams>,
    ) -> ToolResult<Deleted> {
        log_write("delete_network_watch", &params);
        self.hub.delete_network_watch(&params.id).await?;
        Ok(Json(Deleted { id: params.id }))
    }

    /// The discovery topics that did not become an entity, each with the reason.
    #[tool(name = "list_ignored", annotations(read_only_hint = true))]
    async fn list_ignored(&self) -> Json<IgnoredList> {
        Json(IgnoredList {
            ignored: self.hub.ignored(),
        })
    }

    /// Returns one slice of an entity's history: `series` (chart over `range`), `daily`
    /// (kWh per day over `days` with `kind`), `today` (kWh since local midnight with
    /// `kind`), or `last_change` (when the entity last changed). The reply is always a
    /// one-field object: the field matches `what`. 503 `history_unavailable` when
    /// `[history] url` is not set; 404 `not_recorded` when the entity has no rows.
    #[tool(name = "get_history", annotations(read_only_hint = true))]
    async fn get_history(
        &self,
        Parameters(params): Parameters<GetHistoryParams>,
    ) -> ToolResult<HistoryAnswer> {
        let history = self
            .hub
            .history()
            .ok_or_else(|| ToolError("history_unavailable: `[history] url` is not set".into()))?;
        let now = crate::clock::now_micros();
        match params.what {
            HistoryWhat::Series => {
                let range = params
                    .range
                    .ok_or_else(|| ToolError("series requires `range`".into()))?;
                let series = history_query::series(history, &params.entity, range, now)
                    .await
                    .map_err(|e| ToolError(format!("{}: {e}", e.code())))?;
                Ok(Json(HistoryAnswer {
                    series: Some(series),
                    ..HistoryAnswer::default()
                }))
            }
            HistoryWhat::Daily => {
                let days = params
                    .days
                    .ok_or_else(|| ToolError("daily requires `days`".into()))?;
                let kind = params
                    .kind
                    .ok_or_else(|| ToolError("daily requires `kind`".into()))?;
                let daily = history_query::daily(history, &params.entity, days, kind, now)
                    .await
                    .map_err(|e| ToolError(format!("{}: {e}", e.code())))?;
                Ok(Json(HistoryAnswer {
                    daily: Some(daily),
                    ..HistoryAnswer::default()
                }))
            }
            HistoryWhat::Today => {
                let kind = params
                    .kind
                    .ok_or_else(|| ToolError("today requires `kind`".into()))?;
                let today = history_query::today(history, &params.entity, kind, now)
                    .await
                    .map_err(|e| ToolError(format!("{}: {e}", e.code())))?;
                Ok(Json(HistoryAnswer {
                    today: Some(today),
                    ..HistoryAnswer::default()
                }))
            }
            HistoryWhat::LastChange => {
                let ids = std::slice::from_ref(&params.entity);
                let mut items = history_query::last_change(history, ids)
                    .await
                    .map_err(|e| ToolError(format!("{}: {e}", e.code())))?;
                Ok(Json(HistoryAnswer {
                    last_change: items.items.pop(),
                    ..HistoryAnswer::default()
                }))
            }
        }
    }

    /// What crossed the grid meter on one house-local day, per half hour: kWh bought off-peak
    /// and peak, kWh sold. `date` is YYYY-MM-DD, today when absent. The split follows the
    /// off-peak windows in force that day (`get_home_settings`); the grid meter is the entity
    /// whose energy role is `grid`.
    #[tool(name = "get_grid_day", annotations(read_only_hint = true))]
    async fn get_grid_day(
        &self,
        Parameters(params): Parameters<GetGridDayParams>,
    ) -> ToolResult<GridDay> {
        let history = self
            .hub
            .history()
            .ok_or_else(|| ToolError("history_unavailable: `[history] url` is not set".into()))?;
        let date = params
            .date
            .as_deref()
            .map(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d"))
            .transpose()
            .map_err(|_| ToolError("bad_request: `date` must be YYYY-MM-DD".into()))?;
        let day = history_query::grid_day(
            history,
            &self.hub.grid_counters(),
            &self.hub.off_peak_history(),
            self.hub.home_settings().off_peak_device.as_ref(),
            date,
            crate::clock::now_micros(),
        )
        .await
        .map_err(|e| ToolError(format!("{}: {e}", e.code())))?;
        Ok(Json(day))
    }

    /// Sends a phone push notification and returns its alert id. Only `title` is required;
    /// `channel` is one of alarm, security, camera, energy, climate, system, misc (default
    /// misc) and `severity` one of info, warning, critical (default info). Sending again with
    /// the same `id` replaces the earlier alert.
    #[tool(name = "send_notification")]
    async fn send_notification(
        &self,
        Parameters(params): Parameters<SendNotificationParams>,
    ) -> ToolResult<Sent> {
        log_write("send_notification", &params);
        if let Some(reason) = params.channel.as_deref().and_then(notify_channel_error) {
            return Err(ToolError(reason));
        }
        if let Some(reason) = params.severity.as_deref().and_then(notify_severity_error) {
            return Err(ToolError(reason));
        }
        let id = params.id.clone().unwrap_or_else(generated_alert_id);
        let mut alert = serde_json::json!({
            "v": 1,
            "id": id,
            "title": params.title,
            "channel": params.channel.as_deref().unwrap_or("misc"),
            "severity": params.severity.as_deref().unwrap_or("info"),
        });
        for (key, value) in [
            ("body", params.body.as_ref()),
            ("entity_id", params.entity_id.as_ref()),
            ("image_url", params.image_url.as_ref()),
        ] {
            if let Some(value) = value {
                alert[key] = Value::String(value.clone());
            }
        }
        if let Some(silent) = params.silent {
            alert["silent"] = Value::Bool(silent);
        }
        self.hub
            .notify("mcp", &alert)
            .map(|id| Json(Sent { id }))
            .map_err(|rejected| ToolError(rejected.reason))
    }

    /// The push alerts the phone received, newest first, with their channel, severity, title,
    /// body and whether they are resolved. `limit` (20 by default) caps the list and
    /// `channel` keeps one channel.
    #[tool(name = "list_alerts", annotations(read_only_hint = true))]
    async fn list_alerts(
        &self,
        Parameters(params): Parameters<ListAlertsParams>,
    ) -> Json<AlertList> {
        let limit = params.limit.unwrap_or(DEFAULT_ALERTS_LIMIT);
        let alerts = self
            .hub
            .alerts()
            .into_iter()
            .filter(|alert| {
                params
                    .channel
                    .as_deref()
                    .is_none_or(|channel| alert.channel.id() == channel)
            })
            .take(limit)
            .collect();
        Json(AlertList { alerts })
    }
}

/// A fresh alert id for a notification sent without one: `mcp-<unix seconds>-<n>`, `n` counting
/// the notifications of this process so two in the same second differ.
fn generated_alert_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("mcp-{secs}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Every write tool leaves a trace of what was asked, minus any alarm code.
pub(super) fn log_write<P: Serialize>(tool: &'static str, params: &P) {
    tracing::info!(tool, args = %logged_args(params), "mcp write");
}

/// The arguments as JSON, with `command.code` (an alarm panel code, a secret) replaced by
/// `***` when present, at the top level or in a flow's `command` nodes, so the rest of the call stays readable in the log.
fn logged_args<P: Serialize>(params: &P) -> String {
    match serde_json::to_value(params) {
        Ok(mut value) => {
            if let Some(code) = value.get_mut("command").and_then(|c| c.get_mut("code")) {
                *code = Value::from("***");
            }
            let nodes = value
                .get_mut("flow")
                .and_then(|flow| flow.get_mut("nodes"))
                .and_then(Value::as_array_mut);
            for node in nodes.into_iter().flatten() {
                let code = node
                    .get_mut("params")
                    .and_then(|params| params.get_mut("command"))
                    .and_then(|command| command.get_mut("code"));
                if let Some(code) = code {
                    *code = Value::from("***");
                }
            }
            value.to_string()
        }
        Err(err) => format!("<unserializable: {err}>"),
    }
}

/// `query` is already lowercase.
fn device_matches(device: &DeviceView, query: &str) -> bool {
    contains_ci(&device.friendly_name, query)
        || device
            .model
            .as_deref()
            .is_some_and(|m| contains_ci(m, query))
        || device
            .vendor
            .as_deref()
            .is_some_and(|v| contains_ci(v, query))
}

/// `query` is already lowercase.
/// By name, id, or the word of the type a switch is shown as ("siren", "door opener").
fn entity_matches(entity: &EntityView, query: &str) -> bool {
    let type_word = SwitchType::of(entity.protocol_kind, &entity.ui)
        .filter(|t| *t != SwitchType::Switch)
        .map(SwitchType::label);
    contains_ci(&entity.name, query)
        || contains_ci(&entity.id, query)
        || type_word.is_some_and(|w| contains_ci(w, query))
}

fn contains_ci(haystack: &str, lowercase_needle: &str) -> bool {
    haystack.to_lowercase().contains(lowercase_needle)
}

/// `list_areas` speaks snake_case (`group_names`, `group_favorites`) and adds counts; a client
/// sending that output back lands those keys in the area's `extra`. Takes the counts out and
/// moves the two group fields to their real place unless the camelCase field says otherwise.
/// Not a serde alias on `Area`, which is the app's wire format and encodes camelCase only.
fn adopt_listed_fields(area: &mut oikos_core::model::Area) {
    area.extra.remove("entity_count");
    area.extra.remove("primary_count");
    if let Some(names) = area.extra.remove("group_names")
        && area.group_names.is_empty()
        && let Ok(names) = serde_json::from_value(names)
    {
        area.group_names = names;
    }
    if let Some(favorites) = area.extra.remove("group_favorites")
        && area.group_favorites.is_empty()
        && let Ok(favorites) = serde_json::from_value(favorites)
    {
        area.group_favorites = favorites;
    }
}

#[cfg(test)]
mod tests {
    use oikos_core::model::{AlarmMode, Command};

    use super::super::params::{SendCommandParams, SetFlowParams};
    use super::logged_args;

    #[test]
    fn send_command_log_hides_the_alarm_code() {
        for command in [
            Command::Arm {
                mode: AlarmMode::Away,
                code: Some("1234".to_owned()),
            },
            Command::Disarm {
                code: Some("1234".to_owned()),
            },
        ] {
            let logged = logged_args(&SendCommandParams {
                id: "panel_1".to_owned(),
                command,
            });
            assert!(!logged.contains("1234"), "{logged}");
            assert!(logged.contains("\"code\":\"***\""), "{logged}");
            assert!(logged.contains("\"id\":\"panel_1\""), "{logged}");
            assert!(logged.contains("\"type\":\""), "{logged}");
        }
    }

    #[test]
    fn set_flow_log_hides_the_alarm_code_of_command_nodes() {
        let flow = oikos_core::flow::Flow::decode(
            br#"{"id":"night","name":"Night","nodes":[{"id":"arm","type":"command",
                "params":{"entity":"panel","command":{"type":"arm","mode":"away","code":"1234"}}}]}"#,
        )
        .expect("flow");
        let logged = logged_args(&SetFlowParams { flow });
        assert!(!logged.contains("1234"), "{logged}");
        assert!(logged.contains("\"code\":\"***\""), "{logged}");
        assert!(logged.contains("\"id\":\"night\""), "{logged}");
    }

    #[test]
    fn send_command_log_keeps_a_command_without_code() {
        let logged = logged_args(&SendCommandParams {
            id: "lamp".to_owned(),
            command: Command::TurnOn {
                brightness: Some(42),
                color: None,
                effect: None,
            },
        });
        assert_eq!(
            logged,
            r#"{"command":{"brightness":42,"type":"turn_on"},"id":"lamp"}"#
        );
    }
}
