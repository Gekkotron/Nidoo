//! The parameter and result types of the MCP tools: documented, JSON-schema'd wire shapes.
//! Results are objects (the MCP `structuredContent` must be one), so lists are wrapped.

use nidoo_api::{
    Ambiance, AmbianceView, AreaView, BridgeView, ConnectionStatus, DeviceView, EnergyKind,
    EntityView, GroupSpec, GroupView, HistoryDaily, HistoryRange, HistorySeries, HistoryToday,
    IgnoredView, LastChangeItem, NetworkWatchSpec, NetworkWatchView, OverridePatch, RunStatus,
    VirtualSpec,
};
use nidoo_core::flow::Flow;
use nidoo_core::model::{Area, Command, Kind, UiOverride};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which entities `list_entities` returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Include {
    /// What the app shows: no diagnostic or config entity, none disabled by default, none
    /// folded into another entity, none hidden by its override.
    #[default]
    Primary,
    /// Every entity.
    All,
}

/// Default page size of `list_entities`.
pub const DEFAULT_LIMIT: usize = 100;
/// Largest page `list_entities` returns.
pub const MAX_LIMIT: usize = 500;
/// Default number of devices and of entities `search` returns.
pub const DEFAULT_SEARCH_LIMIT: usize = 20;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListDevicesParams {
    /// Only devices with at least one entity in this area (an area id from `list_areas`).
    pub area: Option<String>,
    /// Case-insensitive substring matched against the friendly name, the model and the vendor.
    pub query: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListEntitiesParams {
    /// Only entities in this area (an area id from `list_areas`).
    pub area: Option<String>,
    /// Only entities shown as this kind, in snake_case (`light`, `switch`, `cover`, `climate`,
    /// `sensor`, `binary_sensor`, `alarm_panel`, `number`, `select`, `button`, `vacuum`,
    /// `water_heater`, `media_player`); a switch overridden as a light counts as a light.
    pub kind: Option<Kind>,
    /// Only the entities of this device (a device id from `list_devices`).
    pub device_id: Option<String>,
    /// Case-insensitive substring matched against the entity name and id only; device names,
    /// models and vendors are searched by `list_devices` and `search`.
    pub query: Option<String>,
    /// Only entities whose last state message is at or after this unix time, in seconds.
    pub changed_since: Option<u64>,
    /// `primary` (the default) hides diagnostic, config, disabled-by-default, folded and hidden
    /// entities; `all` lists every entity.
    pub include: Option<Include>,
    /// Page size: 100 by default, 500 at most.
    pub limit: Option<usize>,
    /// Entities to skip before the page, 0 by default. `total` counts the matches before paging.
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEntityParams {
    /// The entity id, as `list_entities` returns it.
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetOverrideParams {
    /// The entity id, as `list_entities` returns it.
    pub id: String,
    /// The fields to change; the others keep their value and `null` clears one. Fields: `name`,
    /// `area`, `hidden`, `order`, `cardStyle`, `favorite`, `energyRole` (solar, grid,
    /// grid_export, battery, house, consumer, water), `energyInvert`, `securityHidden`,
    /// `energyIcon`, `energyMeter`, `homePinned`, `securityAction`, `securityAsk`, `ungrouped`,
    /// `showAs` (light, door, siren, chime, plug, tv), `lightOnPreset` (white, warm, red, green,
    /// blue), `lightOnBrightness` (1 to
    /// 255), `climatePresets`, `mainControl` (one of the entity's announced controls, or `none`
    /// to hide it), `invert` (a switch wired the other way round: the hub swaps on and off in
    /// its state and its commands).
    pub patch: OverridePatch,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClearOverrideParams {
    /// The entity id, as `list_entities` returns it.
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SendCommandParams {
    /// The entity id, as `list_entities` returns it.
    pub id: String,
    /// The command, tagged by `type`: `turn_on` (optional `brightness` 0-255, `color` {h, s},
    /// `effect`), `turn_off`, `toggle`, `set_position` {position 0-100}, `open`, `close`,
    /// `stop`, `set_temperature` {temperature}, `set_hvac_mode` {mode}, `set_fan_mode` {mode},
    /// `set_number` {value}, `select_option` {option}, `press`, `arm` {mode, code?}, `disarm`
    /// {code?}, `{"type":"control","control":"<name>","value":…}` for a generic entity (control
    /// names come from its `capabilities.generic.controls`; `value` is a bool, number or
    /// string by the control's shape, absent for a press), a media player's
    /// `{"type":"media_action","action":"play|pause|next|previous"}` and
    /// `{"type":"play_media","query":"<words>"}` (only the ones its
    /// `capabilities.media_features` list; `play` on an idle player plays something), and the
    /// vacuum and water-heater commands the entity's capabilities list.
    pub command: Command,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetAreasParams {
    /// The whole list of rooms; it replaces the declared areas. Each has an `id`, a `name` and
    /// optionally an `icon`, an `order` and the app's `groupNames` and `groupFavorites`.
    pub areas: Vec<Area>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// A few words, French or English, case and accents ignored ("température salon"); every
    /// word that is not an article or a preposition must match.
    pub query: String,
    /// At most this many devices and this many entities: 20 by default, 500 at most.
    pub limit: Option<usize>,
}

/// The bridge and the broker link.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BridgeStatus {
    pub bridge: BridgeView,
    pub connection: ConnectionStatus,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AreaList {
    pub areas: Vec<AreaView>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeviceList {
    pub devices: Vec<DeviceView>,
}

/// One page of entities and the number of matches before paging.
#[derive(Debug, Serialize, JsonSchema)]
pub struct EntityPage {
    pub total: usize,
    pub items: Vec<EntityView>,
}

/// An entity view with what the view leaves out (the wire shape of the hub's `EntityDetail`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct EntityDetailView {
    #[serde(flatten)]
    pub entity: EntityView,
    /// The expanded discovery config: topics, templates and payload constants.
    pub bindings: Value,
    /// The override retained for the entity, verbatim; `null` when there is none.
    pub raw_override: Option<UiOverride>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResult {
    pub devices: Vec<DeviceView>,
    pub entities: Vec<EntityView>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct IgnoredList {
    pub ignored: Vec<IgnoredView>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetFlowParams {
    /// The flow id, as `list_flows` returns it.
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetFlowParams {
    /// The whole flow; it replaces any flow with the same `id` (retained on
    /// `nidoo/automation/<id>`) and is checked before it is saved, so a refusal names what to
    /// fix. Fields: `id` (1 to 64 characters of `a-z`, `0-9`, `_` and `-`), `name` (free text),
    /// `description` (optional), `enabled` (default `true`; a disabled flow is kept but never
    /// runs), `mode`, `nodes` and `edges`.
    ///
    /// `mode` is `single` (the default: a trigger that fires while a run is active is ignored)
    /// or `restart` (it cancels the active run and starts over). Use `restart` for "N minutes
    /// after the last ..." (motion, door, button): each new trigger restarts the wait, so the
    /// final action only happens once the events stop.
    ///
    /// Each node is `{"id": "<unique in the flow>", "type": "<type>", "params": {...}}` with an
    /// optional `name` label. The types described here:
    ///
    /// - `trigger.state` starts a run when an entity's state changes. params: `entity` (an id
    ///   from `search` or `list_entities`), optional `to` (only when the new value equals it)
    ///   and `from` (only when the previous value equalled it), as JSON values such as `true`,
    ///   `"ON"` or `21.5`. A flow needs at least one trigger, and the run carries that entity's
    ///   new value. With a `to` and no `from`, it also fires when the value was unknown (a
    ///   state message arrived without it) and then becomes `to`.
    /// - `trigger.silent` starts a run when an entity sends no report for a while. params:
    ///   `entity` and `for` (a duration, `1m` to `168h`, such as `50m`). It fires once per
    ///   silence; the clock restarts at every state message of the entity (even one without a
    ///   value for it), at a flow save or enable and when the broker link returns, and is frozen
    ///   while the link is down (a link that reconnects more often than `for` keeps restarting
    ///   it). Refused on buttons and signal entities. The trigger value is `for`.
    /// - `limit` lets a run through at most `max` times (1 to 100) in any rolling `per` window
    ///   (a duration, `1m` to `168h`); beyond that the run ends at this node. Every run that
    ///   reaches it counts, cancelled and manual runs included, so place it right before the
    ///   action. Counts are kept in memory and reset when the hub restarts.
    /// - `if` branches on a comparison. params: `op` (`eq`, `ne`, `gt`, `lt`, `gte`, `lte`),
    ///   `value` (the constant to compare with) and optional `entity`. Without `entity` it
    ///   compares the triggering entity's live value, the one that fired the run; with it, that
    ///   entity's current value. It has two output ports, `true` and `false`. Instead of `entity`,
    ///   `result` names a math card whose result of this run is compared (not with `entity`,
    ///   `average` or `missing`).
    /// - `switch` branches on one value with 1 to 10 ordered rules. params: `rules`, a list of
    ///   `{"op": "eq"|"ne"|"gt"|"lt"|"gte"|"lte"|"between", "value": ..., "to": ...}` (`to` only
    ///   for `between`, which includes both ends), optional `entity` and `average` as for `if`,
    ///   or `result` (a math card's name, as for `if`), `otherwise` (`true` adds an `otherwise`
    ///   port taken when no rule matches or the value is unknown) and `all` (`true` takes every
    ///   matching rule, not only the first). Its output ports are `"1"` to `"n"` (the rule's
    ///   1-based position) and `"otherwise"`.
    /// - `time.between` branches on the house-local time of day. params: `from` (included) and
    ///   `to` (excluded), both `HH:MM` 24-hour and different; `to` earlier than `from` crosses
    ///   midnight. Its output ports are `in` and `out`.
    /// - `delay` waits. params: `for`, a duration such as `90s`, `5m` or `1h30m` (units `s`, `m`,
    ///   `h`, combined without spaces), and optional `up_to` (same format, not shorter than
    ///   `for`): each run then waits a random duration between `for` and `up_to`.
    /// - `command` sends a command to an entity. params: `entity` (an id) and `command`, the same
    ///   JSON as `send_command`'s, e.g. `{"type": "turn_on", "brightness": 200}` or
    ///   `{"type": "turn_off"}`. The entity must exist and support the command. Optional
    ///   `value_from` (a math card's name) replaces the number of `set_number`, `set_temperature`,
    ///   `set_position` or `turn_on`'s brightness with that card's result; the literal number
    ///   stays in the JSON and is ignored then. Other commands refuse `value_from`.
    /// - `math` computes one number and can save it. params: `inputs`, 0 to 10 of
    ///   `{"name", "entity", "average"?}`: `name` is how the formula refers to the entity's value
    ///   (`[a-z][a-z0-9_]{0,31}`, unique in the card), `average` (`15m`, at most `24h`) reads the
    ///   mean of its readings instead of the current value, as for `if`. `formula` (1 to 500
    ///   characters) is an expression over the inputs, the `trigger` value (when it is a number)
    ///   and the results of other math cards by their names: numbers, `+ - * / %`, unary `-`,
    ///   parentheses, comparisons (`< <= > >= == !=`, giving 1 or 0), `and`, `or`, `not`, and the
    ///   functions `min(a, b, ...)`, `max(a, b, ...)`, `abs(x)`, `floor(x)`, `ceil(x)`,
    ///   `round(x)` or `round(x, n)` (n from 0 to 6), `clamp(x, low, high)`, `if(c, a, b)` (only
    ///   the taken branch is computed) and `interp(x, [x0, x1, ...], [y0, y1, ...])` (linear,
    ///   2 to 50 points with increasing x). `name` (the result's name, the same form as an input
    ///   name, not a function or keyword) is how later cards read it. Optional `save_to` (a
    ///   number entity: device, `virtual_*` or `group_*`) receives the result with `set_number`,
    ///   rounded to its step and kept in its range; the step succeeds only when the command is
    ///   accepted. Division by zero, a result that is not finite, an unknown input or a result
    ///   not computed in this run fails the run. Results last for one run only. It has no ports:
    ///   its edge carries no `port`.
    ///
    /// Math results also feed `if` and `switch` (`result`, above) and `notify` text: `{name}`
    /// prints a result, `{name:2}` rounds it, as for entity values.
    ///
    /// `edges` is a list of `{"from": "<node id>", "to": "<node id>"}` followed from a trigger.
    /// An edge leaving an `if` must also carry `"port": "true"` or `"port": "false"`, one leaving
    /// a `switch` a port `"1"` to `"n"` or `"otherwise"`, one leaving a `time.between` `"in"` or
    /// `"out"` (the branch it belongs to; omit a branch to do nothing there); edges leaving any
    /// other node have no `port`.
    ///
    /// Example, LED on at motion and off five minutes after the last motion: `mode: "restart"`,
    /// nodes `motion` (`trigger.state`, entity the sensor), `moving` (`if`, `op: "eq"`,
    /// `value: true`), `on` (`command`, `turn_on`), `wait` (`delay`, `for: "5m"`) and `off`
    /// (`command`, `turn_off`); edges `motion` to `moving`, `moving` port `true` to `on`,
    /// `moving` port `false` to `wait`, `wait` to `off`.
    pub flow: Flow,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeleteFlowParams {
    /// The flow id, as `list_flows` returns it.
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetFlowEnabledParams {
    /// The flow id, as `list_flows` returns it.
    pub id: String,
    /// `true` lets the flow run; `false` keeps it saved but stops it from running.
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RunFlowParams {
    /// The flow id, as `list_flows` returns it.
    pub id: String,
    /// The id of a `trigger.manual` card to start from; the run carries its value. Absent: start
    /// at the first trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

/// The flows, one summary each; `get_flow` has the definition and the run history.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FlowList {
    pub flows: Vec<FlowSummary>,
}

/// One flow in a list.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FlowSummary {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// The room the flow is listed under (an area id), if any.
    pub area: Option<String>,
    /// Why the flow cannot run, when it cannot (a stored flow that no longer validates).
    pub invalid: Option<String>,
    /// How many nodes the flow has.
    pub nodes: usize,
    /// The most recent run; `None` when the flow never ran.
    pub last_run: Option<LastRun>,
}

/// How the latest run of a flow went.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LastRun {
    pub status: RunStatus,
    /// Unix seconds.
    pub started_at: u64,
}

/// A phone push notification to send.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SendNotificationParams {
    /// The alert title; required, not empty.
    pub title: String,
    /// The alert body.
    pub body: Option<String>,
    /// One of `alarm`, `security`, `camera`, `energy`, `climate`, `system`, `misc` (the default).
    pub channel: Option<String>,
    /// One of `info` (the default), `warning`, `critical`.
    pub severity: Option<String>,
    /// The entity the alert is about.
    pub entity_id: Option<String>,
    /// An image to show with the alert.
    pub image_url: Option<String>,
    /// Deliver without sound.
    pub silent: Option<bool>,
    /// A fixed alert id: sending again with it replaces the earlier alert. Absent means a new
    /// `mcp-<unix seconds>-<n>` id.
    pub id: Option<String>,
}

/// A sentence to say on the speaker satellites.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SpeakParams {
    /// The sentence to say; required, not empty.
    pub text: String,
    /// A room (area id or name); every room when left out.
    pub room: Option<String>,
    /// The language code to speak in; the house language when left out.
    pub language: Option<String>,
    /// The voice to use; the language's chosen voice when left out.
    pub voice: Option<String>,
}

/// How many satellites `speak` sent the sentence to.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Spoken {
    pub players: usize,
}

/// The id of the alert `send_notification` raised.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Sent {
    pub id: String,
}

/// Default number of alerts `list_alerts` returns.
pub const DEFAULT_ALERTS_LIMIT: usize = 20;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListAlertsParams {
    /// How many alerts to return, newest first: 20 by default.
    pub limit: Option<usize>,
    /// Only alerts of this channel (`alarm`, `security`, `camera`, `energy`, `climate`,
    /// `system`, `misc`).
    pub channel: Option<String>,
}

/// The push alerts, newest first.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AlertList {
    pub alerts: Vec<nidoo_api::AlertView>,
}

/// The id of a flow that `delete_flow` removed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Deleted {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{ENERGY_ROLES, LIGHT_ON_PRESETS, SHOW_AS};

    /// The `patch` documentation is written by hand: it must name every app field and value.
    #[test]
    fn set_override_patch_doc_names_every_field_and_value() {
        let schema = schemars::schema_for!(SetOverrideParams);
        let description = schema.as_value()["properties"]["patch"]["description"]
            .as_str()
            .expect("patch is documented")
            .to_owned();
        for name in UiOverride::FIELDS
            .iter()
            .chain(ENERGY_ROLES)
            .chain(LIGHT_ON_PRESETS)
            .chain(SHOW_AS)
        {
            assert!(description.contains(name), "`{name}` is missing");
        }
    }

    #[test]
    fn include_defaults_to_primary_and_reads_lowercase() {
        assert_eq!(Include::default(), Include::Primary);
        assert_eq!(
            serde_json::from_str::<Include>("\"all\"").expect("all"),
            Include::All
        );
        assert!(serde_json::from_str::<Include>("\"All\"").is_err());
    }

    /// The `flow` documentation is written by hand: it must name every node type, both `if`
    /// ports and both modes, since it is all an assistant reads before writing a flow.
    #[test]
    fn set_flow_doc_names_every_node_type_and_mode() {
        let schema = schemars::schema_for!(SetFlowParams);
        let description = schema.as_value()["properties"]["flow"]["description"]
            .as_str()
            .expect("flow is documented")
            .to_owned();
        for needle in [
            "trigger.state",
            "if",
            "delay",
            "command",
            "restart",
            "single",
            "true",
            "false",
            "port",
            "for",
            "to",
            "from",
            "entity",
            "op",
            "mode",
            "math",
            "value_from",
            "result",
            "save_to",
        ] {
            assert!(description.contains(needle), "`{needle}` is missing");
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetVirtualParams {
    /// The id: 1 to 48 characters of `a-z`, `0-9` and `_`. The entity becomes `virtual_<id>`.
    pub id: String,
    /// The definition: `kind` (`switch`, `number` or `button`), `name`, and for a number
    /// `min` (0), `max` (100), `step` (1) and `unit`. `initial` is the state published when
    /// the entity is first created (a switch starts off, a number at `min`); it is ignored
    /// when the entity already exists.
    pub spec: VirtualSpec,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeleteVirtualParams {
    /// The id given to `set_virtual` (without the `virtual_` prefix).
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetGroupParams {
    /// The id: 1 to 48 characters of `a-z`, `0-9` and `_`. The entity becomes `group_<id>`.
    pub id: String,
    /// The definition: `kind` (the kind the group presents itself as), `name`, `members` (the
    /// entity ids it controls, not other groups) and `hideMembers` (true by default).
    pub spec: GroupSpec,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteGroupParams {
    /// The id given to `set_group` (without the `group_` prefix).
    pub id: String,
}

/// The groups, one entry each with its members and the flows that use it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GroupList {
    pub groups: Vec<GroupView>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetNetworkWatchParams {
    /// The id: 1 to 48 characters of `a-z`, `0-9` and `_`. The entity becomes `network_<id>`. A
    /// watch on a device must use the device id as its id.
    pub id: String,
    /// The definition: `name`, `host` (name or IP to ping), `intervalS` (10 to 600, default 30),
    /// `enabled` (false pauses it), optional `area` and `device`.
    pub spec: NetworkWatchSpec,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteNetworkWatchParams {
    /// The id given to `set_network_watch` (without the `network_` prefix).
    pub id: String,
}

/// The network watches, one entry each with its entity and live status.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NetworkWatchList {
    pub watches: Vec<NetworkWatchView>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAmbianceParams {
    /// The ambiance: `id` (1 to 48 characters of `a-z`, `0-9` and `_`), `name`, optional
    /// `subtitle`, `icon`, `pinned` and `order`, and `actions`.
    pub ambiance: Ambiance,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmbianceIdParams {
    /// The ambiance id.
    pub id: String,
}

/// The ambiances, each valid or with the reason it is not.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AmbianceList {
    pub ambiances: Vec<AmbianceView>,
}

/// Which history slice to return.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryWhat {
    /// A chart series over a named range (`range` required).
    Series,
    /// kWh per calendar day for the last `days` days (`days` and `kind` required).
    Daily,
    /// kWh since local midnight (`kind` required).
    Today,
    /// The time of the last state change for the entity.
    LastChange,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetGridDayParams {
    /// The house-local day, `YYYY-MM-DD`; today when absent.
    pub date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetHistoryParams {
    /// The entity id, as `list_entities` returns it.
    pub entity: String,
    /// What to return: `series`, `daily`, `today` or `last_change`.
    pub what: HistoryWhat,
    /// `1h`, `6h`, `24h`, `7d`, `30d` or `1y`. Required for `what = series`.
    pub range: Option<HistoryRange>,
    /// 7, 30 or 365. Required for `what = daily`.
    pub days: Option<u32>,
    /// `counter` for a meter reading or `power` for a watt reading. Required for `daily`
    /// and `today`.
    pub kind: Option<EnergyKind>,
}

/// A single-entry object: at most one of `series`, `daily`, `today` or `last_change` is
/// present, matching the `what` of the request.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct HistoryAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<HistorySeries>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily: Option<HistoryDaily>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today: Option<HistoryToday>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_change: Option<LastChangeItem>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClimateZoneParams {
    /// The zone id: the id of an area with climate devices (see `climate_status`).
    pub zone: String,
}

/// A zone with its latest decisions.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClimateZoneDetail {
    pub zone: nidoo_climate::view::ZoneView,
    /// The last 20 decisions, newest first, each with its `source` and `reason`.
    pub decisions: Vec<nidoo_climate::doc::Decision>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAlarmSettingsParams {
    /// The whole alarm settings document; it replaces the current one.
    pub settings: nidoo_alarm::doc::AlarmSettings,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetClimateSettingsParams {
    /// The whole house document; it replaces the current one.
    pub settings: nidoo_climate::doc::HouseSettings,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetClimateZoneParams {
    /// The zone id: the id of an area with climate devices.
    pub zone: String,
    /// The whole zone document (`state` `off`, `pending` or `active`, rules, targets); it
    /// replaces the current one.
    pub settings: nidoo_climate::doc::ZoneSettings,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveClimateProposalParams {
    /// The zone id.
    pub zone: String,
    /// Edited rules to apply instead of the proposal's own.
    #[serde(default)]
    pub rules: Option<nidoo_climate::doc::Rules>,
}

/// No `deny_unknown_fields` here: serde does not support it beside `flatten`; the flattened
/// request denies unknown fields itself.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetClimateOverrideParams {
    /// The zone id.
    pub zone: String,
    /// What to hold, and for how long: `setpoint` and/or `mode`, and exactly one of `minutes`
    /// and `until` (Unix seconds).
    #[serde(flatten)]
    pub request: nidoo_climate::view::OverrideRequest,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetClimateEcoParams {
    /// The zone id.
    pub zone: String,
    /// Eco ends at this Unix time, in seconds.
    pub until: u64,
}

/// The zone a climate write was applied to.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClimateAck {
    pub zone: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetHomeSettingsParams {
    /// The whole home settings document; it replaces the current one. `eco_setpoint` is in
    /// 5..=30 °C, `mode_entity` a select entity id, `indoor_entity` a temperature sensor id,
    /// `cold_below` and `hot_above` 5..=35 °C with `cold_below` lower; `null` or absent clears
    /// a field. `energy_view` is `classic` (the default), `flux` or `orbite`.
    pub settings: nidoo_api::HomeSettings,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSettingsParams {
    /// The section: `weather`, `solcast`, `frigate`, `push`, `climate` or `advanced`.
    pub section: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSettingsParams {
    /// The section: `weather`, `solcast`, `frigate`, `push`, `climate` or `advanced`.
    pub section: String,
    /// The whole section, as `get_settings` shows it; it replaces the stored one. A secret
    /// field is a string to set it, `null` to clear it, absent, `""` or `{"set": true}` to keep it.
    pub value: serde_json::Value,
}

// A secret may sit in `value`, so the parameters never print it.
impl std::fmt::Debug for SetSettingsParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetSettingsParams")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}
