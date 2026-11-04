---
title: MCP tools
sidebar_position: 3
description: The 53 tools and the one resource of the hub's MCP server, with their arguments.
---

The MCP server is at `http://<host>:8081/mcp` (streamable HTTP, stateful sessions) and offers **fifty-three tools** and **one resource**. It has the same trust model as the REST API on the same port: LAN-only, no authentication in v1. The tools are defined in [`crates/nidoo/src/mcp`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo/src/mcp). For setup in Claude Code, Codex and Claude Desktop, see [Talking to the house (MCP)](../users/assistants-mcp.md).

A failed tool call answers in-band (`isError: true` with the reason as text) so the assistant can retry with better arguments. A refused publish (broker link down) is the same in-band error.

Claude Desktop only speaks stdio, so it launches `nidoo mcp-stdio --server http://<host>:8081`, a proxy that relays every request to the running hub. It exits with `cannot reach the Nidoo MCP server at …` when the hub is down; its logs go to stderr and stdout carries the protocol.

## Resource

| URI | Content |
| --- | --- |
| `nidoo://snapshot` | The whole house as one JSON document (`application/json`): connection, bridge, areas, devices, entities with their states, ignored discovery topics. |

## House

| Tool | What it does |
| --- | --- |
| `bridge_info` | The Zigbee2MQTT bridge (version, coordinator, permit join, online or not) and the state of the link to the broker. |
| `system_status` | The hub's System view, the same as `GET /api/v1/system` (uptime, restarts, server and hub figures, services, the notify switch). No arguments. Read-only. |
| `list_areas` | The rooms, each with its entity count and the count of entities shown by default (`primary_count`): declared areas first, then those only carried by entities, then `Unassigned`. |
| `set_areas` | Replaces the declared rooms (retained on `nidoo/ui/_areas`) and returns the resulting list. Args: `areas` (each with `id`, `name`, optional `icon`, `order`, `groupNames`, `groupFavorites`). A room kept by id keeps the fields you leave out. |
| `list_devices` | The Zigbee devices with availability, battery and link quality. Args: optional `area` (an area id), `query` (case-insensitive, matched against name, model and vendor). |
| `list_entities` | A page of entities with their state. By default only what the app shows (`include: "primary"`: no diagnostic, config, disabled-by-default, folded or hidden entity); `include: "all"` for everything. Args: `area`, `kind`, `device_id`, `query` (entity name and id), `changed_since` (unix seconds), `include`, `limit` (100 by default, 500 at most), `offset`. The reply has `total` (matches before paging) and `items`. |
| `get_entity` | One entity with its state, its MQTT `bindings` (the expanded discovery config) and its `raw_override`. Args: `id`. |
| `search` | Devices and entities found from a few words, French or English, case and accents ignored (`température salon`, `volet cuisine`). Articles and prepositions are dropped; every other word must match. An entity matches on its name, id, room (id or name), kind, device class (with their French words) or its device; a device on its name, model or vendor. Args: `query`, optional `limit` (20 by default, 500 at most). |
| `list_ignored` | The discovery topics that did not become an entity, with the reason. |

## Overrides

| Tool | What it does |
| --- | --- |
| `set_override` | Changes fields of the entity's `nidoo/ui/<id>` override and returns the entity. Args: `id`, `patch` (an object; `null` clears a field). Unknown fields and invalid values are refused and nothing is written. The fields are listed in [Entity types](entity-types.md#overrides). |
| `clear_override` | Removes the override entirely and returns the entity as discovered. Args: `id`. |

## Commands

| Tool | What it does |
| --- | --- |
| `send_command` | Publishes a command to an entity and reports `accepted: true` once the next state echo confirms it, or `accepted: false` with the reason. Args: `id`, `command`. An unknown or offline entity, or a command the entity cannot take, is refused before anything is published. |

The `command` is tagged by `type`: `turn_on` (optional `brightness` 0-255, `color` `{h, s}`, `effect`), `turn_off`, `toggle`, `set_position` `{position 0-100}`, `open`, `close`, `stop`, `set_temperature` `{temperature}`, `set_hvac_mode` `{mode}`, `set_fan_mode` `{mode}`, `set_number` `{value}`, `select_option` `{option}`, `press`, `arm` `{mode, code?}`, `disarm` `{code?}`, plus the vacuum and water-heater commands the entity's capabilities list. A generic entity (lock, fan, siren, scene, text and so on) takes `{"type": "control", "control": "<name>", "value": …}` with a name from its `capabilities.generic.controls`; `value` is a bool, number or string by the control's shape and absent for a press. A control with no state to echo is accepted as soon as it is published.

## Flows

See [Flow nodes](flow-nodes.md) for the document format.

| Tool | What it does |
| --- | --- |
| `list_flows` | The flows, one summary each: `id`, `name`, `enabled`, `area`, `invalid` (why it cannot run), `nodes` (count) and `last_run`. |
| `get_flow` | One flow in full: definition, why it is invalid, the run in progress and the recent runs (newest first) with each node's outcome. Args: `id`. |
| `set_flow` | Creates or replaces a flow, validated against the house and saved retained on `nidoo/automation/<id>`. Args: `flow` (the whole document). Returns `{flow, warning}`; `warning` is set when the flow reads the sun against the Paris fallback. |
| `delete_flow` | Removes a flow, valid or not. Args: `id`. |
| `set_flow_enabled` | Turns a flow on or off without touching its definition. Args: `id`, `enabled`. |
| `run_flow` | Starts a manual run and returns the run as it starts. Refused when the flow is disabled, invalid or already running, except that a card press (`node`) on a `restart` flow replaces the active run instead. Args: `id`, optional `node` (the id of a `trigger.manual` card to start from; the run carries its value, and a card that is not a manual trigger of the flow is refused). |

:::note
The argument schema of `set_flow` that the server advertises describes the node types `trigger.state`, `if`, `switch`, `time.between`, `delay`, `command` and `math`. The server accepts all sixteen; the server instructions it sends name the others (`trigger.schedule`, `trigger.sun`, `trigger.off_peak`, `trigger.signal`, `trigger.manual`, `trigger.silent`, `limit`, `flow.enable`, `notify`).
:::

## Groups

Groups need `groups.enabled` in the advanced settings ([Settings reference](configuration.md#groups)); otherwise these tools answer `groups disabled`. See [Groups and virtual entities](../users/groups-and-virtual-entities.md).

| Tool | What it does |
| --- | --- |
| `list_groups` | Every group the hub stores, sorted by id, with its members and the flows that use it. |
| `set_group` | Creates or replaces a group, the entity `group_<id>`. Args: `id` (1 to 48 characters of `a-z`, `0-9`, `_`), `spec` (`kind`, `name`, `members`, optional `hideMembers`, true by default). Members must be of the group's kind and cannot be groups. |
| `delete_group` | Removes a group and clears what the broker retained for it. The UI override is kept. Args: `id`. |

## Network presence

Network presence needs `network.enabled`, on by default; otherwise these tools answer `network presence disabled`.

| Tool | What it does |
| --- | --- |
| `list_network_watches` | Every network watch the hub stores, sorted by id, with its entity and live status. |
| `set_network_watch` | Creates or replaces a watch, the entity `network_<id>`. Args: `id` (1 to 48 characters of `a-z`, `0-9`, `_`; the device id for a watch on a device), `spec` (`name`, `host`, optional `intervalS` 10 to 600, `enabled`, `area`, `device`). A device has at most one watch. |
| `delete_network_watch` | Removes a watch and clears what the broker retained for it. Args: `id`. |

## Virtual entities

| Tool | What it does |
| --- | --- |
| `set_virtual` | Creates or replaces a virtual `switch`, `number`, `button` or `select` the hub owns, as `virtual_<id>`. Args: `id` (1 to 48 characters of `a-z`, `0-9`, `_`), `spec` (`kind`, `name`; for a number `min`, `max`, `step`, `unit`; for a select `options`; `initial`). |
| `delete_virtual` | Removes a virtual entity. Args: `id` (without the `virtual_` prefix). |

## Notifications

| Tool | What it does |
| --- | --- |
| `speak` | Says a sentence on the speaker satellites and returns `{players}`, how many were asked. Args: `text` (required), optional `room` (an area id or name; every room without it), `language`, `voice`. Only the satellites whose Allowed and Connected are on count. Fails when speech is not set up, the room is unknown, or no satellite can speak there. |
| `send_notification` | Sends a phone push and returns its alert id. Args: `title` (required), optional `body`, `channel` (`alarm`, `security`, `camera`, `energy`, `climate`, `system`, `misc`; default `misc`), `severity` (`info`, `warning`, `critical`; default `info`), `entity_id`, `image_url`, `silent`, `id`. Sending again with the same `id` replaces the earlier alert; without one the id is `mcp-<unix seconds>-<n>`. |
| `list_alerts` | The recent push alerts, newest first. Args: optional `limit` (20 by default), `channel`. |

## History

| Tool | What it does |
| --- | --- |
| `get_history` | One slice of an entity's history, available only when the database is connected (**Settings → Database**). Args: `entity`, `what`, and by `what`: `series` needs `range` (`1h`, `6h`, `24h`, `7d`, `30d`, `1y`); `daily` needs `days` (7, 30 or 365) and `kind` (`counter` or `power`); `today` needs `kind`; `last_change` needs nothing more. The reply is a one-field object named after `what`. Errors read `history_unavailable: …` or `not_recorded: …`. |
| `get_grid_day` | What crossed the grid meter on one house-local day, per half hour: kWh bought off-peak and peak, kWh sold, with the day's totals and off-peak bands (the shape of `GET /history/grid_day`). Args: `date` (`YYYY-MM-DD`, today when absent). Errors read `history_unavailable: …`, `no_grid_meter: …` or `bad_request: …`. |

## Weather

| Tool | What it does |
| --- | --- |
| `get_weather` | The outdoor weather from Open-Meteo: now, today, rain in the next hour, tonight's low and the next five days, in metric units, with `stale` set when the data is old. The same document as `GET /api/v1/weather`. Answers `weather unavailable` while weather is off in **Settings → Weather** or before the first fetch. See [Weather](../users/weather.md). |

## Solar

| Tool | What it does |
| --- | --- |
| `get_solar` | The Solcast solar forecast: today, tomorrow, the next seven days, the 30-minute periods and the request budget, with `stale` set when the data is old. The same document as `GET /api/v1/solar`. Answers `solar unavailable` while the solar forecast is off in **Settings → Solar forecast** or before the first fetch. See [Solar forecast](../users/solar-forecast.md). |

## Home settings

| Tool | What it does |
| --- | --- |
| `get_home_settings` | The home settings: `eco_setpoint` (°C), `mode_entity` (a select) and `indoor_entity` (a temperature sensor), `cold_below` and `hot_above` (°C); a field that is not set is `null`. `off_peak` lists the off-peak windows in force, `off_peak_history` the dated windows of past days. `energy_view` is `classic`, `flux` or `orbite`, the web Energy page's live diagram. `off_peak_device`, when set, is `{entity, values}`: the entity whose state says off-peak, the windows being its backup. |
| `set_home_settings` | Replaces the home settings (retained on `nidoo/ui/_home`) and returns them. Read them with `get_home_settings` first and send the whole document back. Args: `settings`. `eco_setpoint` must be from 5 to 30, `cold_below` and `hot_above` from 5 to 35 with `cold_below` lower (the web uses 15 and 26 when unset); an entity of the wrong kind is refused with the reason. `off_peak` takes `{start, end}` `HH:MM` windows (crossing midnight allowed, no overlap); a change applies from today and past days keep theirs; `off_peak_history` is kept by the hub and ignored when sent. `energy_view` is `classic` (the default), `flux` or `orbite`. `off_peak_device` (optional) is `{entity, values}`: an existing entity and at least one value. |
| `get_settings` | One section of the service settings: `weather`, `solcast`, `frigate`, `push`, `climate` or `advanced`, as the settings page shows it. Secrets are write-only: the Solcast key, the Frigate password and the push service account read as `{"set": true or false}`, never the value. Args: `section`. |
| `set_settings` | Replaces one section with `value`, checks it, saves it and restarts the hub (returns `{"restarting": true}`). Read the section with `get_settings` first and send the whole of it back: what is left out takes its default. A secret is set by a string, cleared by `null` and kept when left out (or sent back as `{"set": true}`). Changing a service's address (the Frigate URL) needs its secret sent again. Refused with a fixed reason, never containing a secret, in setup mode, while a restart is pending or when the database is unreachable. Args: `section`, `value`. |

## Climate

The climate engine is in beta. These nine tools answer `climate disabled` while the climate engine is off. See [Climate](../users/climate.md).

| Tool | What it does |
| --- | --- |
| `climate_status` | The house settings and every zone with its reading, decision and why (`source`, `reason`), overrides, pending proposal and learner state. |
| `get_climate_zone` | One zone as `climate_status` shows it, with its last 20 decisions. Args: `zone`. |
| `set_climate_settings` | Replaces the house settings. Read them with `climate_status` first and send the whole document back. Args: `settings`. |
| `set_climate_zone` | Replaces a zone's settings (`state` `off`, `pending` or `active`, targets, rules). New non-empty rules count as approved. A device another zone lists in `entities` is refused. Args: `zone`, `settings`. |
| `approve_climate_proposal` | Approves the pending proposal; its rules, or `rules` when you edit them, become the zone's schedule. Refused when there is none. Args: `zone`, optional `rules`. |
| `reject_climate_proposal` | Rejects the pending proposal; the same proposal is not made again for a week. Args: `zone`. |
| `set_climate_override` | Holds a temperature and/or a mode on a zone. Args: `zone`, `setpoint` and/or `mode`, and exactly one of `minutes` and `until` (unix seconds). Safety still acts. |
| `clear_climate_override` | Removes every override of a zone. Args: `zone`. |
| `set_climate_eco` | Puts a zone in eco until a time: the house eco setpoint (never above its schedule) and eco actions. Args: `zone`, `until` (unix seconds). Refused on an `off` zone or a past `until`. |
| `clear_climate_eco` | Ends a zone's eco. Args: `zone`. |
| `learn_climate_zone` | Learns a zone's rules from its history now and returns where the learner stands. Args: `zone`. |

## Ambiances

See [Ambiances](../users/ambiances.md) for the action types, the category rules and the report. The tools are the same as the REST routes, and the server instructions describe the action types.

| Tool | What it does |
| --- | --- |
| `list_ambiances` | Every stored ambiance with its actions, or the reason it cannot be read (`invalid`). |
| `set_ambiance` | Creates or replaces an ambiance, validated against the house and retained on `nidoo/ui/ambiances/<id>`. Args: `ambiance` (the whole document: `id`, `name`, optional `subtitle` and `icon`, `pinned`, `order` and `actions`). A refusal names the action by its index so you can fix it and retry. |
| `delete_ambiance` | Removes an ambiance, valid or not. Args: `id`. |
| `apply_ambiance` | Runs the actions once, in order, and returns one report line per action (`ok`, `failed` or `skipped`, with the entities and reasons). Args: `id`. Refused while the same ambiance is already running. |

## Alarm

The native alarm (optional, off by default). Its settings are a whole document and are off until `enabled` is true. Arming and disarming are not tools: they are `send_command` on the alarm entity `nidoo_alarm_house`, with `{"type":"arm","mode":"away"}` (or `home`, `night`) and `{"type":"disarm"}`. See [Alarm](../users/alarm.md).

| Tool | What it does |
| --- | --- |
| `alarm_status` | The settings (`enabled`, delays, sensors, sirens, remotes, presence), the panel state (`disarmed`, `arming`, `armed_away`, `armed_home`, `armed_night`, `pending`, `triggered`), its mode and source, `deadline`, `cause`, `open_at_arm`, any refused settings and the run-time warnings. |
| `set_alarm_settings` | Replaces the settings document. Read them with `alarm_status` first and send the whole document back. A refusal names the reason and the previous document is kept. Args: `settings`. |

## Count

Thirty tools live in `tools.rs` (house, system, overrides, command, groups, network, virtual, history, notifications, weather, solar, home settings), six in `flow_tools.rs`, eleven in `climate_tools.rs`, four in `ambiance_tools.rs` and two in `alarm_tools.rs`: 30 + 6 + 11 + 4 + 2 = 53.
