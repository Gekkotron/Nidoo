---
title: HTTP API
sidebar_position: 2
description: Every route of the hub's REST API and WebSocket, with bodies, replies and status codes.
---

The hub listens on the HTTP bind address (default `0.0.0.0:8081`, see the [Settings reference](configuration.md#bootstrap-settings-hubjson)). It answers JSON, with CORS open to any origin and no authentication in v1: keep it on the LAN. The router is in [`crates/nidoo/src/http`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo/src/http) and the wire types in [`crates/nidoo-api`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-api).

Besides the JSON API, the same port serves:

- `GET /ws`, the [WebSocket](#websocket);
- `/mcp`, the MCP server (see [MCP tools](mcp-tools.md));
- the web UI, for every other path that is not under `/api`. An unknown path under `/api` answers a JSON `404`.

Ids in paths are the entity, flow, group or dashboard ids the API returns. Bodies are JSON; send `Content-Type: application/json`.

## Health and snapshot

| Route | What it does |
| --- | --- |
| `GET /health` | `{status, connection, entities}`: `status` is `"ok"`, `connection` the broker link (`{"type": "connecting" \| "online"}` or `{"type": "offline", "reason": "…"}`), `entities` the entity count. |
| `GET /api/v1/snapshot` | The whole house: connection, bridge, areas, devices, entities with their state, ignored topics, alerts, flows and `generated_at` (unix seconds). The same document is the MCP resource `nidoo://snapshot`. |
| `GET /api/v1/ignored` | The discovery topics that did not become an entity, each `{topic, reason}`. |

## Setup and settings

The routes the setup wizard and the Settings pages use. They answer in both modes: a hub that has not finished its MQTT and database setup is in **setup mode** and serves only these routes, `/health` and the web UI; once both are saved it is **running** and serves everything. Like the rest of the API they have no login. Secrets never appear in a reply: a stored secret reads `{"set": true}` (or `false`). To send one, give a string; to clear it, send `null`; leave it out (or send `""` or `{"set": true}` back) to keep it. See the [Settings reference](configuration.md) for what each section holds.

| Route | What it does |
| --- | --- |
| `GET /api/v1/setup` | `{mode, step, finished, bundled, location, mqtt, database, restarting}` (`200`). `mode` is `setup` or `running`; `step` is `welcome`, `mqtt`, `database`, `devices`, `services` or `done`; `bundled` says `data/bundled.json` exists; `mqtt` shows the connection (`host`, `port`, `user`, `client_id`, `z2m_base`, `password: {"set": bool}`) and `database` its `host`, `port`, `database`, `user` and `password: {"set": bool}`, pre-filled from the bundled stack while nothing is saved. |
| `GET /api/v1/setup/geocode?city=…&country=…` | Up to five places `{name, admin, country, latitude, longitude, timezone}` for the city (`country` is an optional two-letter filter). An empty city gives `[]`. `502` when the lookup fails. |
| `PUT /api/v1/setup/location` | Saves the home location `{city, country, latitude, longitude, timezone}` and returns the setup status. `422` for an empty city, coordinates out of range, an unknown time zone or a country that is not two letters. Restarts a running hub. |
| `POST /api/v1/setup/test/mqtt` | Body `{host, port, user, password, client_id, z2m_base}`: connects with it and returns `{ok, message, detail}` (`200`, also when `ok` is `false`). `422` when the user is missing for a password, or the password is not sent again for a new address. |
| `PUT /api/v1/setup/mqtt` | The same body: tests, then saves it and returns the setup status. `422` (with the test result as the body) when the test fails. Restarts a running hub, and a hub that has just become complete. |
| `POST /api/v1/setup/test/database` | Body `{host, port, database, user, password}`: connects, checks the `timescaledb` extension and prepares the tables; `{ok, message, detail}` as above. `detail.history_since` says since when the database already holds history (or `null`). `422` as for MQTT. |
| `PUT /api/v1/setup/database` | The same body: tests, then saves it as the database URL. Same statuses as `PUT /api/v1/setup/mqtt`. |
| `PUT /api/v1/setup/step` | `{"step": "services"}`: records where the wizard stands and returns the status. Never restarts. |
| `POST /api/v1/setup/finish` | Marks the setup finished and returns the status. `409` while MQTT and the database are not both saved. |
| `GET /api/v1/settings/{section}` | One section as the Settings page shows it (`200`): `weather`, `solcast`, `frigate`, `push`, `climate`, `advanced` or `mobile`. `404` for another name. |
| `PUT /api/v1/settings/mobile` | `{"gateway_url": string or null, "gateway_key": string}`: the remote gateway the phone setup code carries. Saved **without a restart**; returns the section (`200`, the key as `{"set": bool}`, plus `gateway_ready`). The key keeps its stored value when left out, `""` or `{"set": true}`, and `null` clears it. `409` in setup mode, `422` for an address that is not `https://` with a host or a key that is not 64 hex characters, `503` while the database cannot save it. |
| `GET /api/v1/setup/mobile-code` | `{"payload": "<compact JSON>"}`: the `v:1` text the Android app's **Scan setup code** reads. It carries the broker login, the gateway (when both its address and key are set), the push token topic and the Frigate login, so treat it as a password. A broker on `localhost` or a Docker name is replaced by the host the page was opened on. Never logged. `409` in setup mode. |
| `PUT /api/v1/settings/{section}` | Replaces the section with the body, checks it the way the hub checks its settings at start, saves it and restarts the hub. Returns `{"restarting": true}` (`200`). What the body leaves out takes its default, so read the section first and send the whole of it back. `404` unknown section, `409` in setup mode, `422` for a body that does not validate (the reason is fixed text, never a secret), `500` when the settings cache cannot be written, `503` while a restart is pending or the database is unreachable. |
| `POST /api/v1/setup/test/solcast` | Body as for `PUT /api/v1/settings/solcast`: checks the key with one Solcast call (`{ok, message, detail}`, `200`; `detail.sites` lists the rooftop sites). Without a key anywhere it answers `ok: false`. |
| `POST /api/v1/setup/test/frigate` | Body as for `PUT /api/v1/settings/frigate`: logs into Frigate at the address (`{ok, message, detail}`, `200`; `detail.cameras` lists the enabled cameras). Without an address it answers `ok: false`. `422` for an address that is not valid or carries a login. |

The setup `PUT`s and `POST /api/v1/setup/finish` also answer `500` `{"error": "cannot save hub.json"}` when the file cannot be written, and the location, MQTT and database `PUT`s answer `422` with the reason, on a running hub, when the change would leave settings the hub cannot start with (the same check as `PUT /api/v1/settings/{section}`); nothing is saved then. Every write that restarts the hub answers first; the hub is back a few seconds later (poll `GET /health`). Writes answer `503` `{"error": "restarting"}` meanwhile. The same two settings operations exist over MCP as `get_settings` and `set_settings` ([MCP tools](mcp-tools.md#home-settings)).

## Entities

| Route | What it does |
| --- | --- |
| `POST /api/v1/entities/{id}/command` | Sends a command (`{"type": "turn_on", …}`) and reports `{accepted, reason, state}`: `accepted` is `true` once the entity's state echo confirms it within the `commands.confirm_timeout_ms` setting (default 5000), `false` with a `reason` when nothing came back in time. A button `press`, or a control with no state to echo, is accepted as soon as it is published, with `state: null`. `404` for an unknown entity, `422` for a command the entity cannot take or an offline entity, `503` while the broker link is down. |
| `PATCH /api/v1/entities/{id}/override` | Changes fields of the entity's `nidoo/ui/<id>` override and returns the entity as it now appears. The body is a partial override; `null` clears a field. `400` for an unknown field or a bad value (the error names the valid ones), `404`, `503`. The fields are listed in [Entity types](entity-types.md#overrides). |
| `DELETE /api/v1/entities/{id}/override` | Removes the override (`204`). |
| `GET /api/v1/entities/{id}/picture` | The picture of a camera or image entity: the stored bytes, or the bytes of its URL fetched by the hub now (10 s timeout, 5 MiB cap, no redirects followed). The reply is served as an image or as an opaque download, with `X-Content-Type-Options: nosniff`. `404` when the entity has no picture, `502` when the URL cannot be fetched. |

## Areas

| Route | What it does |
| --- | --- |
| `PUT /api/v1/areas` | Replaces the declared rooms (`nidoo/ui/_areas`) and returns the resulting list. |
| `PATCH /api/v1/areas/{id}` | Renames a room: body `{"name": "…"}`; the room's other fields are kept. Returns the area list. `422` for a blank name or for `Unassigned`, `404` for an unknown area, `503` while the broker link is down. |

## Home settings

| Route | What it does |
| --- | --- |
| `GET /api/v1/home/settings` | The home settings document `{eco_setpoint, mode_entity, indoor_entity, cold_below, hot_above, off_peak, off_peak_history, energy_view}`, plus `off_peak_device` once one is set (numbers and ids `null`, lists empty and `energy_view` `classic` until saved). |
| `PUT /api/v1/home/settings` | Replaces the document and returns it as stored (retained on `nidoo/ui/_home`). `eco_setpoint` is a number from 5 to 30 (°C), `mode_entity` the id of a `select` entity, `indoor_entity` the id of a temperature sensor; `cold_below` and `hot_above` are numbers from 5 to 35 (°C) with `cold_below` lower than `hot_above` (the web reads cold below 15 and warm above 26 when unset); `null` clears a field. `off_peak` is a list of `{start, end}` local `HH:MM` windows (`end` before `start` crosses midnight; none empty or overlapping; empty for a single tariff). `off_peak_history` (`[{since: "YYYY-MM-DD", windows}, …]`) is kept by the hub: a change of `off_peak` adds an entry dated today, and a value sent by the client is ignored. `energy_view` is `classic`, `flux` or `orbite`: how the web Energy page draws the live power, the same in every browser. `off_peak_device` (optional, `{entity, values}`) is the entity whose state says off-peak and the values that mean it (matched as `on`/`off`, a plain number or the text); the windows are its backup, and either may be left out. The entity must exist and `values` must not be empty. `assist_min_c` and `assist_max_c` (optional, 5 to 35 °C, `assist_min_c` lower than `assist_max_c`; 16 and 23 when unset) are the setpoints the [assistant](../users/assistant.md) applies without asking; they appear only once set. `422` for a value that does not validate (the error names the field), `503` while the broker link is down. |

## Groups

Groups need `groups.enabled` in the advanced settings ([Settings reference](configuration.md#groups)); while it is off every route here answers `503` with `groups disabled`. See [Groups and virtual entities](../users/groups-and-virtual-entities.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/groups` | The stored groups, sorted by id, each with its members, its entity and the names of the flows that use it. |
| `PUT /api/v1/groups/{id}` | Creates or replaces a group: body `{"kind", "name", "members": […], "hideMembers"?}`. Returns the group. `422` for a bad id, an unsupported kind, a member that is itself a group or a member of another kind than the group's; `503` while groups are off or the broker link is down. |
| `DELETE /api/v1/groups/{id}` | Removes the group and clears what the broker retained for it (`204`); `404` for an id that is not a group. |

## Virtual entities

| Route | What it does |
| --- | --- |
| `PUT /api/v1/virtual/{id}` | Creates or replaces a virtual switch, number, button or select (`{"kind", "name", …}`) and returns the entity `virtual_<id>`; the initial state is published only when it is new, except that redefining a select whose current state is no longer among its options resets it to the first option. `422` for a bad id or spec, `503` while the broker link is down. |
| `DELETE /api/v1/virtual/{id}` | Removes a virtual entity and clears its retained topics (`204`), or `404`. |

The body takes `kind` (`switch`, `number`, `button` or `select`) and `name`; for a number also `min` (0), `max` (100), `step` (1) and `unit`; for a select `options` (1 to 32 distinct strings of up to 64 characters, required); `initial` is the state published when the entity is first created. The id is 1 to 48 characters of `a-z`, `0-9` and `_`.

## Network presence

Network presence needs `network.enabled`, on by default; while it is off every route here answers `503` with `network presence disabled`.

| Route | What it does |
| --- | --- |
| `GET /api/v1/network` | The stored watches, sorted by id, each as `{id, spec, entity, status}` with the live status (`waiting`, `connected`, `disconnected`, `paused` or `cannot_ping`). |
| `PUT /api/v1/network/{id}` | Creates or replaces a watch: body `{"name", "host", "intervalS"?, "enabled"?, "area"?, "device"?}`. Returns the watch with its entity `network_<id>`. `422` for a bad id, an `intervalS` outside 10 to 600, an unknown device, an id other than the device id for a watch on a device, or a second watch for the same device; `503` while network presence is off or the broker link is down. |
| `DELETE /api/v1/network/{id}` | Removes the watch and clears what the broker retained for it (`204`); `404` for an id that is not a watch. |

The id is 1 to 48 characters of `a-z`, `0-9` and `_`. `intervalS` defaults to 30 and `enabled` to true; a paused watch keeps its definition and its entity reads offline.

## System health

The figures are read from memory, so these answer even while the broker or the database is down. The view is the one the Settings → System page shows; see [System health](../users/system-health.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/system` | The System view, camelCase: `startedAt` (unix seconds the hub started), `uptimePct` (0 to 100 over the last 30 days), `restarts` (newest first, last 30 days: `{startedAt, downSecs, stop}`, `downSecs` absent when unknown, `stop` one of `first`, `clean`, `unexpected`), `cpu` (`{serverPct, hubPct}`), `ram` (`{usedBytes, totalBytes, hubBytes}`), `disk` (`{usedBytes, totalBytes, dbBytes}`), `temperatureC`, `devices`, `entities`, `retained`, `commandsToday`, `services` (`broker`, `zigbee2mqtt` and `database`, each `{service, state, since}` with `state` `up`, `down` or `unknown`) and `notify`. A figure that cannot be measured is `null` (or `unknown` for a service), never `0`. |
| `PUT /api/v1/system/notify` | Switches the outage and restart pushes: body `{"notify": true}` or `{"notify": false}`. Returns `{"notify": …}` with the saved value. Only the switch is saved; the hub is not restarted. `422` for a bad body or an unknown field, `503` while the database cannot save the setting (or its settings cache cannot be written, in which case nothing changes). |

## Dashboards

| Route | What it does |
| --- | --- |
| `GET /api/v1/dashboards` | The dashboards as `{dashboard, synthesized}`: `house` first (synthesized by the hub), then the authored ones sorted by `order`. |
| `GET /api/v1/dashboards/{id}` | One dashboard; `404` for an unknown id. |
| `PUT /api/v1/dashboards/{id}` | Creates or replaces a dashboard (`{"id", "name", "order", "widgets": […]}`) and returns it. The body `id` must equal the path id. An `order` of 0 or one already taken is replaced by the next free one. `403` for the reserved id `house`, `422 id mismatch` when the ids differ, `422` when a widget does not validate, `503` while the broker link is down. |
| `DELETE /api/v1/dashboards/{id}` | Removes a dashboard (`204`); `403` for `house`, `404` for an unknown id. |
| `POST /api/v1/dashboards/order` | `{"order": ["<id>", …]}` reassigns contiguous orders by position (`204`). The list must name every authored dashboard exactly once (`house` is ignored); otherwise `422`, and nothing changes. |

A widget `kind` is one of `light`, `cover`, `climate`, `lock`, `sensor`, `room-group`, `flow-button`, `energy-stat`, `history-chart`, `camera` or `water-heater`, with a `span` from the kind's allowed set (`1x1`, `2x1`, `2x2`, `1x2`, plus `4x2` and `6x3` for `camera`) and an optional `pos` (`{"row": 1, "col": 0}`, 0-based) for its top-left cell on the 6-column desktop grid. A save is refused when a widget runs past column 6 or two widgets cover the same cell; widgets without `pos` flow into the first free cells in list order. A dashboard id is 1 to 64 characters of letters, digits, `_` and `-`, and not `house`.

## Ambiances

See [Ambiances](../users/ambiances.md) for the action types and the report. An ambiance id is 1 to 48 characters of `a-z`, `0-9` and `_`. The body of `PUT` is `{"id", "name", "subtitle"?, "icon"?, "pinned", "order", "actions": […]}`; an `id` in the body may be left out (the path id is used) and must equal the path id otherwise. Unknown top-level fields are kept.

| Route | What it does |
| --- | --- |
| `GET /api/v1/ambiances` | Every stored ambiance as `{id, ambiance, invalid}`: valid ones first, by `order`, then the invalid ones with the reason in `invalid`. |
| `PUT /api/v1/ambiances/{id}` | Creates or replaces an ambiance and returns its view. The shape is checked, then the house: unknown entities, areas, flows, zones and `except` ids are refused, and a climate action is refused while the climate engine is off. An action equal to one of the stored version is not checked again. `422` for a body or a reference the house refuses (the error names the action by its index, e.g. `action 2: …`), `503` while the broker link is down. |
| `DELETE /api/v1/ambiances/{id}` | Removes an ambiance (`204`), valid or not. `404` for an id that is not an ambiance; `503` while the broker link is down. |
| `POST /api/v1/ambiances/{id}/apply` | Runs the actions once, in order, and returns the [apply report](../users/ambiances.md#reading-the-report) (`200`). `404` for an unknown id; `422` for an invalid ambiance or one that is already running; `503` while the broker link is down. The run finishes and broadcasts `ambiance_applied` even if the request is dropped. Categories leave out group entities (the hub's and Zigbee2MQTT groups), virtual entities, the Zigbee2MQTT bridge, covers whose device class is not a window covering, and switches whose `showAs` is not `plug` (an untyped switch is a plain switch, not a plug). A flow `run` action is `failed` when the flow is disabled, invalid or already running; an action whose every target is offline is `skipped` with the reason `every target is offline`. |

## Flows

See [Flow nodes](flow-nodes.md) for the document format.

| Route | What it does |
| --- | --- |
| `GET /api/v1/flows` | Every flow with its engine state: definition, why it is invalid, the active run and the recent runs. Each step of a run has `node`, `at`, `outcome`, `detail` and, for a succeeded `if`, `switch` or `time.between` step, `ports`, the output ports it followed. |
| `GET /api/v1/flows/{id}` | One flow; `404` for an unknown id. |
| `PUT /api/v1/flows/{id}` | Creates or replaces a flow (`nidoo/automation/<id>`). The path id wins: a body without `id` takes it, another one is `422 id mismatch`. Returns `{flow, warning}`; `warning` is `null`, or a message when the flow reads the sun while the house point is the Paris fallback. `422` for a flow that does not validate against the house, `503` while the broker link is down. |
| `DELETE /api/v1/flows/{id}` | Removes the flow (`204`), or `404`. |
| `POST /api/v1/flows/{id}/enabled` | `{"enabled": bool}`; returns the flow. |
| `POST /api/v1/flows/{id}/run` | Starts a run by hand; `202` with the run. Optional JSON body `{"node": "<card id>"}` starts from that `trigger.manual` card and the run carries its value. `422` when the flow is disabled, invalid or already running (a card press on a `restart` flow replaces the active run instead), or when `<card id>` is not a manual trigger of this flow. |
| `POST /api/v1/flows/{id}/area` | `{"area": "<id>" \| null}` sets the room the flow is listed under (`null` clears it; the key is required); returns the flow. `422` for an unknown room, a body without `area` or an invalid flow, `404` for an unknown flow. |
| `POST /api/v1/flows/{id}/labels` | `{"labels": ["<label id>", …]}` sets the labels the flow carries (sorted and de-duplicated; the key is required); returns the flow. `422` for a label that is not in the catalog (a label already on the flow is kept even after it was deleted), a body without `labels` or an invalid flow, `404` for an unknown flow, `503` while the broker link is down. |
| `GET /api/v1/flow-labels` | The label catalog and how many flows carry each label: `{catalog: {labels: [{id, name, color, choice?}], choices: [{id, name, current?}]}, counts: {"<label id>": n}}`. A label no flow carries is absent from `counts`. |
| `POST /api/v1/flow-labels` | One catalog edit, `{"op": …}`: `create_label` (`name`, optional `choice`), `create_choice` (`name`), `rename_label` (`id`, `name`), `recolor_label` (`id`, `color`), `move_label` (`id`, optional `choice`), `rename_choice` (`id`, `name`), `delete_label` (`id`; removed from every flow carrying it) or `delete_choice` (`id`; its labels stay). Ids are made from the name and never change. No edit switches flows. Returns the catalog as `GET` does. `422` for an unknown id or an empty name, `503` while the broker link is down or until the hub has the retained catalog (a few seconds after it connects). |
| `POST /api/v1/flow-labels/{id}/switch` | `{"action": "pick" \| "on" \| "off"}`. `pick` (a label in a choice of at least two labels) turns its flows on and the flows of the choice's other labels off, then makes it the choice's current label; a flow in both is turned on. `on` / `off` switch every flow carrying the label. Returns `{label, action, enabled: [flow id], disabled: [flow id], skipped: [{id, name, reason}]}`: only the flows that changed, and the ones it could not switch (`invalid`, or the write error). `422` for an unknown label or a `pick` the label does not allow, `503` while the broker link is down. |
| `GET /api/v1/schedule?cron=…&count=…` | Previews a 5-field cron in the house time zone: `{timezone, next: […]}` with the next `count` ticks as RFC 3339 (default 3, at most 10). `422` for a bad cron. |
| `GET /api/v1/config/flows` | The home location as the UI needs it: `{latitude, longitude, timezone, fallback_active}`. |

## Alerts and push

| Route | What it does |
| --- | --- |
| `GET /api/v1/alerts` | The push alerts the hub keeps in its retained history (`nidoo/push/history`), newest first. |
| `POST /api/v1/alerts/{id}/actions/{action}` | Publishes the reply the app publishes when a button is tapped (`{"id","action","install_id":"nidoo-web","ts"}` on `nidoo/push/responses`); `202`, or `404` for an unknown alert or action, `503` while the broker link is down. |
| `GET /api/v1/push` | Push status: `{"state","installs","queued"}`. `state` is `off` (no usable Firebase key saved in **Settings → Notifications**), `syncing` (rebuilding the token map after a connect) or `ready`; `installs` counts the registered app installs, `queued` the alerts waiting in the retry queue. |

## Media

| Route | What it does |
| --- | --- |
| `GET /api/v1/media?url=…` | A Frigate snapshot or clip fetched by the hub with the Frigate login (**Settings → Cameras**). `Range` is passed through so clips seek. `400` without a valid absolute `url`, `403` off the Frigate origin or without a Frigate address, `502` when Frigate is unreachable or refuses the login. |
| `GET /api/v1/cameras` | `{ "live": bool, "cameras": [{ "id", "name" }] }`: Frigate's enabled cameras (from its `/api/config`, cached about a minute) and the live view switch. An empty list without a Frigate address or when Frigate is unreachable. |
| `GET /api/v1/cameras/{id}/latest.jpg?h=` | One live frame of camera `id`, `h` pixels high (default 360, at most 1080), with `cache-control: no-store`. `403` while `live` is off, `404` for an id Frigate does not list, `502` when Frigate is unreachable. |
| `GET /api/v1/cameras/{id}/stream?h=` | The camera's MJPEG stream (`multipart/x-mixed-replace`), with no total timeout; the hub drops it when the browser disconnects. Same answers as `latest.jpg`. |

When alerts point at Frigate's login port (8971) the browser cannot load them, so media on that origin is rewritten to `/api/v1/media?url=…`. The hub logs in on demand, and once more on a `401`.

## History

Available when the database is connected (**Settings → Database**). See [History](../users/history.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/history/series?entity=&range=` | A chart series over `1h`, `6h`, `24h`, `7d`, `30d` or `1y`: `{window, points: [[t, mean, min, max], …]}`, times in unix seconds. |
| `GET /api/v1/history/daily?entity=&days=&kind=` | kWh per calendar day for the last `days` days (7, 30 or 365), with `kind` (`counter` or `power`): `{days: [[t, kwh], …]}`. |
| `GET /api/v1/history/today?entity=&kind=` | kWh since local midnight: `{kwh, since}`. |
| `GET /api/v1/history/last_change?entity=&entity=…` | Last change time of 1 to 50 entities, in request order: `{items: [{changed_at, recorded}, …]}`. |
| `GET /api/v1/history/grid_day?date=YYYY-MM-DD` | What crossed the grid meter on one house-local day (today when `date` is absent): `{date, today, first_date, day_slots, slots: [{start, local, off_peak, peak, export}, …], bands: [[from, to], …], totals: {off_peak, peak, export}, has_export, tariffs}`. Slots are half hours of real time from local midnight (46, 48 or 50 a day, `day_slots`; today stops at the current one), `start` in unix seconds and `local` as `HH:MM`; kWh are counter increases, bought split by the off-peak device's recorded readings when one is set, else (and wherever it had no usable reading) by the off-peak windows in force that day (`bands`, unix seconds). The counters are those paired with the `grid` role (bought) and their export twin or `grid_export` (sold). `400 bad_request` for a bad or future date, `404 no_grid_meter` without a grid counter, `503 history_unavailable`. |

Errors are `400 bad_request` (a missing entity, an unknown range, `days` other than 7, 30 or 365, an unknown `kind`, or no `entity` at all), `404 not_recorded` (the entity has no rows) and `503 history_unavailable` (the database is not connected, or is down).

## Climate

Available when the climate engine is on; otherwise these routes answer `503`. See [Climate](../users/climate.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/climate` | The climate engine: house settings (with `eco`) and every zone with its reading, decision and why, overrides, its eco (`eco: {until, set_at}` or `null`), pending proposal and learner state. |
| `PUT /api/v1/climate/house` | Replaces the house settings (`204`); `422` when they do not validate. |
| `PUT /api/v1/climate/zones/{id}` | Replaces a zone's settings (`204`); `404` for an area that is not a zone, `422` when they do not validate. |
| `POST /api/v1/climate/zones/{id}/override` | `{setpoint?, mode?, minutes?, until?}`: holds a target and/or mode, with exactly one of `minutes` and `until` (`204`, else `422`). |
| `DELETE /api/v1/climate/zones/{id}/override` | Clears the zone's overrides (`204`). |
| `PUT /api/v1/climate/zones/{id}/eco` | `{until}` (unix seconds): holds the zone in eco until then (`204`; a past `until` or an `off` zone is `422`). |
| `DELETE /api/v1/climate/zones/{id}/eco` | Ends the zone's eco (`204`). |
| `POST /api/v1/climate/zones/{id}/proposal/approve` | Approves the pending proposal, optionally with edited `Rules` as the body (`204`); `422` when there is none or the rules have the wrong shape, `400` for a body that is not JSON. |
| `POST /api/v1/climate/zones/{id}/proposal/reject` | Drops the pending proposal (`204`); `422` when there is none. |
| `POST /api/v1/climate/zones/{id}/learn` | Learns the zone now; `200` with the learner state. |
| `GET /api/v1/climate/zones/{id}/decisions?limit=` | `{recent, older}`: the latest decisions from memory, then older rows from history when it is on (`limit` 50 by default, 500 at most; `400` for a `limit` that is not a number). |

## Alarm

The native alarm is off until its settings have `enabled` true; the routes answer as usual meanwhile. See [Alarm](../users/alarm.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/alarm` | The alarm as one document (`200`): the settings, the panel state, mode, source, `deadline`, `cause`, `open_at_arm`, any refused settings and the run-time warnings. |
| `PUT /api/v1/alarm/settings` | Replaces the settings document (`204`); `422` when it does not validate or names an entity the house does not have, `503` while the broker link is down. |
| `POST /api/v1/alarm/test-chirp` | `{"siren": <index>}`, the index into `settings.sirens`: sounds that siren's chirp (`204`). `422` when the index is out of range, that siren has no chirp, or the alarm is ringing; `503` while the broker link is down. |

## Weather

Available when weather is on in **Settings → Weather**. See [Weather](../users/weather.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/weather` | The outdoor weather as one document (below). `503` while weather is off, the city was not found, or no fetch has succeeded yet. |

```json
{
  "place": { "name": "Paris", "admin": "Île-de-France", "country": "FR" },
  "fetched_at": 1792746902,
  "stale": false,
  "current": {
    "temperature": 13.4, "apparent_temperature": 11.9, "humidity": 81,
    "condition": "cloudy", "daylight": true, "cloud_cover": 92,
    "precipitation": 0.0, "wind_speed": 14.0, "wind_gusts": 31.0,
    "wind_direction": 230, "pressure": 1012.3, "uv_index": 1.2
  },
  "today": {
    "min": 9.1, "max": 15.0, "precipitation": 2.4,
    "precipitation_probability": 70, "wind_gusts_max": 45.0
  },
  "rain_next_hour": 0.3,
  "tonight_min": 7.8,
  "days": [
    {
      "date": "2026-10-24", "condition": "rain", "min": 8.0, "max": 13.5,
      "precipitation_probability": 85, "apparent_max": 12.0, "precipitation": 4.6,
      "wind_speed_max": 25.4, "wind_gusts_max": 41.0, "wind_direction": 250,
      "humidity": 78.3, "pressure": 1005.8, "cloud_cover": 90.5, "uv_index_max": 1.8
    }
  ]
}
```

Units are metric: °C, km/h, mm, hPa and %. `fetched_at` is unix seconds. Any number may be `null` when Open-Meteo has no value. `days` holds the next five days, starting tomorrow; each has `date` (`YYYY-MM-DD`), `condition`, `min`, `max`, `apparent_max` (highest feels-like), `precipitation` (mm, day total), `precipitation_probability`, `wind_speed_max`, `wind_gusts_max`, `wind_direction` (the dominant direction, degrees), `humidity`, `pressure` and `cloud_cover` (means of the day's hours, one decimal) and `uv_index_max`. There are no coordinates in the response. `stale` is `true` once the last fetch is older than twice `refresh_minutes`.

## Solar

Available when the solar forecast is on in **Settings → Solar forecast**. See [Solar forecast](../users/solar-forecast.md).

| Route | What it does |
| --- | --- |
| `GET /api/v1/solar` | The solar forecast as one document (below). `503` with `solar unavailable` while the solar forecast is off or before the first fetch. |

```json
{
  "fetched_at": 1792746902, "stale": false, "next_refresh_at": 1792754100,
  "calls_today": 6, "daily_limit": 8,
  "now": { "power": 1.6, "remaining_today": 5.2 },
  "today": {
    "date": "2026-10-23", "estimate": 12.4, "low": 9.8, "high": 14.1,
    "peak": { "power": 2.4, "at": 1792758600 },
    "best_hours": { "from": 1792749600, "to": 1792764000 }
  },
  "days": [ { "date": "2026-10-24", "estimate": 8.1, "low": 5.0, "high": 11.0 } ],
  "sites": [ { "index": 0, "today": 5.1 }, { "index": 1, "today": 7.3 } ],
  "periods": [ { "end": 1792746000, "minutes": 30, "estimate": 0.0, "low": 0.0, "high": 0.0 } ],
  "history": {
    "forecasts": [
      { "date": "2026-10-22", "estimate": 11.6, "low": 8.9, "high": 13.0, "made_at": 1792680000 }
    ]
  }
}
```

Energies are kWh (the day figures), powers kW (the period values, the means over each period), times Unix seconds and dates `YYYY-MM-DD` in the house's time zone. Any number may be `null`. `next_refresh_at` is `null` when no call is planned (budget spent, key refused). `days` holds the next seven days, starting tomorrow. `periods` are the 30-minute forecast periods from local midnight today to the end of tomorrow, each with its mean power in kW (`estimate`, `low`, `high`) and its `minutes` length. `sites` never carries the resource id; `index` is its position in the solar forecast's site list. `best_hours` is the longest run today where the estimate is at least 60 % of the peak, or `null` on a day with no production. `history.forecasts` lists the morning forecast of each of the 7 local days before today, oldest first, and is empty when history is off. `stale` is `true` when a planned refresh has failed for over an hour or the last success is older than 18 hours.

## WebSocket

`GET /ws` upgrades to a WebSocket. The hub sends a `snapshot` frame first, then every hub event as a JSON text frame tagged by `type`. It subscribes before it takes the snapshot, so no event falls between the two. A client that falls behind gets a fresh `snapshot` instead of being dropped. Incoming frames are ignored.

| `type` | Payload |
| --- | --- |
| `snapshot` | The whole [snapshot](#health-and-snapshot). |
| `entity_upserted` | An entity view. |
| `entity_removed` | `{id}`. |
| `state_changed` | `{id, state}`. |
| `areas_changed` | `{areas}`. |
| `devices_changed` | `{devices}`. |
| `alerts_changed` | `{alerts}`. |
| `flows_changed` | `{flows}`. |
| `flow_labels_changed` | `{labels}`: the label catalog (the same shape as `GET /api/v1/flow-labels`) after a catalog edit or a pick. Its `counts` is as of that change; count a flow's labels from the flow list as flows change. |
| `flow_run_changed` | `{flow_id, run}`. |
| `bridge_changed` | The Zigbee2MQTT bridge view. |
| `connection_changed` | The broker link status. |
| `command_failed` | `{id, reason}`. |
| `button_pressed` | `{id}`: a virtual or discovered button was pressed. An event, not a state. |
| `signal_received` | `{id, signal}`: a remote, event entity or tag sent a signal. An event, not a state. |
| `weather_changed` | No payload: the weather was refreshed with new data, refetch `GET /api/v1/weather`. |
| `solar_changed` | No payload: the solar forecast was refreshed or its current figures moved, refetch `GET /api/v1/solar`. |
| `home_settings_changed` | `{settings}`: the home settings document changed. |
| `climate_changed` | `{zone}`: one zone's document changed, or the house settings when `zone` is `null`. |
| `alarm_changed` | No payload: the alarm's settings, state or warnings changed, refetch `GET /api/v1/alarm`. The entity's own state arrives as `state_changed`. |
| `ambiances_changed` | `{ambiances}`: the stored ambiances (the same list as `GET /api/v1/ambiances`) after one was saved, deleted or arrived from the broker. |
| `ambiance_applied` | `{report}`: the [apply report](../users/ambiances.md#reading-the-report) of a run, whichever client started it. |
| `assist_turn_added` | `{id}`: the assistant answered a turn, whoever spoke (a page, a satellite, a flow); refetch `GET /api/v1/assist/turns`. |

## Assist

The French and English [assistant](../users/assistant.md). Types are camelCase. A turn is `{id, at, client, text, simulate, outcome, reply, question, trace, wrong}`: `outcome` is `acted`, `answered`, `asked`, `confirming`, `not_understood` or `failed`; `question` is `null` or `{pending, kind: "choice" | "confirm", choices: […]}`; `trace` is `{words, intent, room, roomHow, entities, value, gate, actions: [{target, what, result}], elapsedUs}`.

| Route | What it does |
| --- | --- |
| `POST /api/v1/assist` | `{client, text?, reply?, simulate?, lang?}`; returns the turn (`200`). `lang` is `fr` (the default) or `en`: the language the sentence is read in and the reply is written in. `client` is an id the page generates once; each client has at most one pending question, open 60 s. `text` is a sentence; it answers the pending question when it is a yes / no or names one choice, and otherwise drops it. `reply` answers by tap: `{"type": "choice", "pending", "choice": <index>}`, `{"type": "yes", "pending"}` or `{"type": "no", "pending"}`. With `simulate: true` nothing is sent to the house and the reply starts with `Simulation :` (`Simulation:` in English). `422` without `text` or `reply`, or for a `pending` id that is unknown or expired (`question expired`). |
| `GET /api/v1/assist/turns?limit=…` | `{turns, kept}`: the recent turns, newest first (`limit` 50 by default, at most 200). `kept` is `true` when the history database keeps them (90 days); `false` means only the hub's memory, the last 100 turns, lost on restart. |
| `POST /api/v1/assist/turns/{id}/wrong` | Marks a turn as wrongly understood (`204`); `404` for an unknown id. |

## Speech

Text-to-speech for the [assistant's voice](../users/assistant.md#voice). Needs a hub built with the `tts` feature (the default); without it the management routes answer normally, `GET /api/v1/assist/tts/status` has `speech_built: false`, and only speaking answers `409`. Field names are snake_case.

The hub speaks an ordered list of languages, each with its own model and voice; the first is the **house language**, used when a speech request names none. A language code is two or three letters with optional `-` parts (`fr`, `pt-br`), trimmed and lower-cased; anything else is `422`. Every model a language uses is loaded; a model no language uses is unloaded.

Engine state is `{state: "no_model"}`, `{state: "loading", model}`, `{state: "ready", model}` or `{state: "error", model, message}`. A language is `{code, model, voice, engine}`, where `model` is `null` when it has none and `engine` is its model's state (`no_model` without one). A model is `{id, repo, revision, weights, size, licence, languages, source: "download" | "upload", installed_at, used_by}`, where `languages` are the codes it speaks (empty when its files list none, and then it may serve any language) and `used_by` the languages that speak with it, in list order. A download is `{model, state: "running" | "done" | "failed" | "cancelled", done, total, file, error}` (bytes).

| Route | What it does |
| --- | --- |
| `GET /api/v1/assist/tts/status` | `{languages, token_set, download, speech_built}`, the house language first. The token itself is never returned. |
| `GET /api/v1/assist/tts/models` | The installed models, a list. |
| `DELETE /api/v1/assist/tts/models/{id}` | Removes a model (`204`); `404` for an unknown id. The languages using it stay in the list with `model: null`, and it unloads. |
| `GET /api/v1/assist/tts/languages/available` | The codes the installed models speak that are not in the list yet, each with the models that speak it: `[{code, models}]`. A model whose files list no languages is added under every code (and, alone, offers none: any valid code can then be `PUT`). |
| `PUT /api/v1/assist/tts/languages/{code}` | `{model, voice?}`: adds the language at the end, or changes its model or voice; returns the status. A new model loads in the background (state `loading`). Changing the model resets the voice to `default` unless one is given; a catalog voice is downloaded first (about 256 KB). `404` for an unknown model; `422` when the model does not speak `code`, for a bad code or for an unknown voice. |
| `DELETE /api/v1/assist/tts/languages/{code}` | Stops speaking the language and returns the status (`200`); its model unloads if no other language uses it. `404` for a language not in the list. |
| `PUT /api/v1/assist/tts/languages/house` | `{code}`: moves that language first, making it the house language; returns the status. `404` for a language not in the list. |
| `POST /api/v1/assist/tts/inspect` | `{url}`: a Hugging Face address (`https://huggingface.co/owner/repo`, `hf://owner/repo[@rev]` or `owner/repo`). Returns `{repo, revision, gated, licence, weights: [{name, size}], smallBytes}`. `422` for another host, a bad shape, or a repository that is not a Phonon model; `404` for a repository that does not exist; `502` when Hugging Face cannot be reached. |
| `POST /api/v1/assist/tts/downloads` | `{repo, revision, weights}`; starts the background job and returns its progress (`202`). `409` while another download runs; `422` for a weight file name that is not a known Phonon weight name. A known name the repository does not hold answers `202` and the job then fails with the reason in `error`. A refusal for disk space, a size over the cap, or a gated repository without an accepted token comes back as a `failed` job whose `error` says why, not as an HTTP error. |
| `GET /api/v1/assist/tts/downloads/current` | The running job, or the last one until the next attempt; `404` when there is none. Poll it about once a second. |
| `DELETE /api/v1/assist/tts/downloads/current` | Cancels the job and deletes its temporary folder (`204`); `404` when there is no job. |
| `POST /api/v1/assist/tts/uploads` | Opens an upload session; returns `{id}`. An idle session already open is replaced and its files are discarded; `409` only while a file is being written. |
| `PUT /api/v1/assist/tts/uploads/{id}/files/{name}` | The raw file as the body, one file per request, streamed to disk (`204`). Only the file names a Phonon model holds are accepted, at most one weight file (200 MiB cap) and 8 MiB for each other file. `422` for a name that is not allowed or a second weight file, `409` for a file name sent twice, `413` for a file over the cap (which also ends the session), `404` for an unknown session. |
| `POST /api/v1/assist/tts/uploads/{id}/finish` | Installs the files as a model and returns it. The model is not load-tested here: one that cannot load shows as an engine `error` when activated. `409` while a file is still arriving (the session stays open), `404` for an unknown session. Every other outcome ends the session: `422` (a required file or the weight file is missing, or a file's size does not match what was received) discards the uploaded files. |
| `DELETE /api/v1/assist/tts/uploads/{id}` | Abandons the session and deletes its files (`204`). |
| `GET /api/v1/assist/tts/voices?language={code}` | The voices of that language: its model's own and the catalog voices in that language, `{voices, chosen, fell_back, catalog_offline}`, where `chosen` is the language's voice. Without `language`, the house language. A voice is `{name, language, gender, age, description, source: "model" | "catalog", on_disk}`. `422` for a language the hub does not speak. |
| `PUT /api/v1/assist/tts/token` | `{token}` follows the secret rules of the settings: left out keeps the stored token, `null` clears it, a string sets it (`204`). It does not restart the hub. |
| `POST /api/v1/assist/speech` | `{text, language?, voice?}`; returns `{id, url}` (`201`), where `url` is `/api/v1/assist/speech/{id}.wav`. The text is at most 500 characters and not empty (`422`), and is read (numbers, units) in the request's language. `language` defaults to the house language; a code the hub does not speak is `422`. `voice` defaults to the language's voice (the default stands in when that one is not usable). A named voice speaks without being chosen: a catalog voice in that language is downloaded first, and an unknown one, or one of another language, is `422`. `409` when no language is set up, the language has no model, or its engine is not ready, with the reason; `503` when the queue (four requests) is full. |
| `GET /api/v1/assist/speech/{id}.wav` | The speech as `audio/wav`: 24 kHz, 16-bit, mono, with a header of unknown length followed by the audio as it is produced. Valid for 60 seconds after the `POST`, so a client can retry once; `404` for an unknown or expired id. The text is never in the address. |

Speech management errors use the [common statuses](#errors) and also: `413` for an upload file over its cap, and `502` when Hugging Face cannot be reached while listing a repository. Download refusals (not enough disk space, a file over its size cap, a token the account has not accepted the terms for, a wrong token) are not HTTP errors: the `POST` answers `202` and the job then shows as `failed` with the reason in `error`.

## Speech input

Talking to a [satellite](../users/sound.md): the recognition model, its settings and the satellites' shared key (the UDP messages are in the [voice protocol](voice-protocol.md)). Field names are camelCase. Every route answers `409` when the hub has no speech input.

| Route | What it does |
|-------|--------------|
| `GET /api/v1/assist/stt/status` | `{built, installed, engine, download, settings, hasKey, portInUse}`: `built` is false for a hub built without the `stt` feature; `engine` is the recognizer's state as for speech (`no_model`, `loading`, `ready`, `error`); `download` the running or last model download; `portInUse` the UDP port bound when the hub started. |
| `PUT /api/v1/assist/stt/settings` | `{threads, gain, port, listenMarginMs}` (threads 1–16, default 4; gain 1–16, default 4; port 1024–65535, default 4960; margin 0–2000 ms, default 300); returns the status. `422` out of range. A new port applies when the hub starts again. |
| `POST /api/v1/assist/stt/download` | Fetches the pinned Parakeet archive (about 490 MB, about 670 MB once unpacked) and the Silero VAD from the sherpa-onnx release, checking every file's size and SHA-256; returns the job (`202`). `409` while a download runs or once the model is installed. A refusal for disk space or a file that does not match shows as a `failed` job with the reason in `error`. |
| `DELETE /api/v1/assist/stt/download` | Cancels the download and deletes its temporary files (`204`); `404` when none runs. |
| `DELETE /api/v1/assist/stt/model` | Removes the model files (`204`); `409` while a download runs. |
| `POST /api/v1/assist/stt/key` | Makes a new key and returns it as `{key}`, 64 hex characters (`voice_key` in each board's `secrets.yaml`); satellites with the old key are no longer heard. This is the only time the key is returned: there is no route to read it back, since anyone on the network can call the API. |

## Sound

The speaker satellites of the `nidoo-sound` service (see [Sound](../users/sound.md)).

| Route | Behaviour |
| --- | --- |
| `POST /api/v1/sound/speak` | `{text, room?, satellite?, language?, voice?}`; says the sentence on the satellites of one room (`room`, an area id or name), on one satellite (`satellite`, its short id: the device id without `nidoo_sound_`), or on every satellite when both are left out. Returns `{players}` (`202`), the number of satellites it was sent to. Only satellites whose Allowed and Connected are both on, while the `nidoo-sound` service runs, are counted; a muted one is counted and stays silent. The speech is made as for `POST /api/v1/assist/speech`, then sent on `nidoo-sound/speak`. `422` when `room` and `satellite` are both set, for an unknown room, and for a text the speech refuses; `409` when speech is not set up, has no language ready, or no satellite can speak there. |
| `DELETE /api/v1/sound/satellites/{id}` | Removes satellite `id` (its short id: the device id without `nidoo_sound_`): publishes `nidoo-sound/<id>/remove/set`, so `nidoo-sound` forgets its saved settings and clears its retained discovery and state messages (the hub drops its entities as they go), and clears the overrides (room, name) of its entities. `202`. A board still on the network comes back as a new satellite waiting for approval. `404` for an unknown satellite; `409` while the `nidoo-sound` service is offline (the command is not retained); `503` while the broker link is down. |

## Errors

Errors are `{"error": "…"}` with one of these statuses:

| Status | When |
| --- | --- |
| `400` | A bad override patch, a bad query parameter, or a body that is not valid JSON. |
| `403` | The reserved `house` dashboard id, or media off the Frigate origin. |
| `404` | An unknown entity, alert, action, flow, area, dashboard, group, virtual entity, ambiance or route. |
| `422` | A body of the wrong shape, a PUT body whose `id` differs from the path (`id mismatch`, on flows, dashboards and ambiances), a command refused, a bad virtual or group id or spec, a flow that is invalid, disabled or already running, an unknown room, a dashboard that does not validate, or an ambiance that does not validate, names something the house does not know, or is already running. |
| `502` | A picture URL or Frigate media that cannot be fetched. |
| `503` | The broker link is down, so the write was refused and nothing was applied; or the feature is off (groups, climate, history). The same publish refusal is an in-band tool error over MCP. |

History routes use short codes as the message: `bad_request`, `not_recorded` and `history_unavailable`.
