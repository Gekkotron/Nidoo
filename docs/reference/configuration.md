---
title: Settings reference
sidebar_position: 1
description: Where the hub keeps its settings, every section and field with its default, how secrets are handled and the environment variables and flags that still apply.
---

The hub is configured in the browser: the setup wizard on a first start, then the **Settings** pages. There is no file to edit. This page is the reference behind those screens: where each value is stored, every field with its default, and the few things that still come from the command line or the environment. The structs are in [`crates/nidoo/src/config.rs`](https://github.com/Gekkotron/Nidoo/blob/main/crates/nidoo/src/config.rs) and the sections in [`crates/nidoo/src/setup/sections.rs`](https://github.com/Gekkotron/Nidoo/blob/main/crates/nidoo/src/setup/sections.rs).

Other pages write a field as `[topics] ui_prefix` or `watchdog.acl.enabled`. That is the field `ui_prefix` of `topics`, or `acl.enabled` of `watchdog`, inside the **advanced** section below.

## Where settings live

Two places, split by what the hub needs before it can reach anything:

| Where | What it holds |
| --- | --- |
| `data/hub.json` | The bootstrap settings: the MQTT connection, the database URL, the home location, the HTTP bind address, the log level, the wizard's progress and a "setup finished" flag. The hub writes it, one wizard step at a time, with mode `0600`. You do not edit it, except for the rare field with no screen (below). |
| The `settings` table in the database | Everything else, one JSON row per section: `weather`, `solcast`, `frigate`, `push`, `climate` and `advanced`. Secrets are stored in the same rows. |
| `data/settings-cache.json` | A copy of the settings the hub last read, mode `0600`. If the database is down when the hub starts, it starts from this copy with a warning, so the house keeps running. |
| `data/bundled.json` | Written by the compose `init` service (`nidoo init-bundled`): the credentials of the Mosquitto and TimescaleDB that come with Nidoo, `mqtt_user`, `mqtt_password`, `db_user`, `db_password` and `db_name`, mode `0600`. The wizard reads it to pre-fill the MQTT and Database steps. It is not used once those steps are saved. |

The documents the hub keeps on the broker (home settings, alarm, climate schedules, flows, groups, dashboards, ambiances) are not part of this: they are edited on their own screens and routes.

You read and change the section rows with **Settings**, with `GET`/`PUT /api/v1/settings/{section}` ([HTTP API](http-api.md#setup-and-settings)) or with the `get_settings` and `set_settings` MCP tools ([MCP tools](mcp-tools.md#home-settings)). A change is checked the way the hub checks its settings at start, then saved, and the hub restarts to apply it. A value it could not start with is refused with the reason, and nothing is saved.

The folder is `./data` by default. `nidoo --data <dir>` (or `NIDOO_DATA_DIR`) picks another one; in the Docker image it is `/data`, mounted from `./data`.

## Secrets

A secret is write-only. A read shows `{"set": true}` or `{"set": false}`, never the value. A write sets it with a string, clears it with `null`, and keeps it when the field is absent, `""` or sent back as `{"set": true}`. So a form that posts an empty password field never wipes the stored one.

- Changing the address a secret goes to (the MQTT or database host and port, the Frigate address) needs the secret sent again. Otherwise anyone on the LAN could point the hub at their own machine and collect the stored password.
- The secrets are the MQTT and database passwords, the Solcast API key, the Frigate password and the Firebase service-account key. No log line, error or API reply carries one; a URL that holds a password is masked when logged.
- A login written inside a Frigate address is refused (use the user and password fields).

See [Security](../users/security.md#settings-and-secrets) for what is and is not protected.

## Bootstrap settings (`hub.json`)

The location, the database and the MQTT connection have a screen in the wizard and in **Settings**. The HTTP and log settings do not: set them with the `NIDOO_*` variables below, or in `hub.json` while the hub is stopped.

### Home location

**Settings → Location**, or the first wizard step. It replaces the old `[flows]` and `[weather]` city fields.

| Field | Default | Description |
| --- | --- | --- |
| `city` | none | The place's name. Required. |
| `country` | none | Optional ISO 3166-1 alpha-2 code such as `FR`, exactly two letters (stored in upper case). |
| `latitude` | none | Degrees, north positive, -90 to 90. |
| `longitude` | none | Degrees, east positive, -180 to 180. |
| `timezone` | none | IANA zone name (`Europe/Paris`). `trigger.schedule` runs in it. |

The wizard looks the city up and fills the rest, which you can correct. Sun nodes, `is_dark`, the solar forecast and the weather all use these coordinates. With no location, sun nodes and `is_dark` use Paris (48.8566, 2.3522), saving a flow that reads the sun reports a warning, and schedules run in the system's zone (UTC if it cannot be read; an unknown zone falls back to UTC with a warning). See [Flow nodes](flow-nodes.md).

### Database and broker

**Settings → Database** and **Settings → Broker** (the MQTT connection).

| Field | Default | Description |
| --- | --- | --- |
| `mqtt.host` | `"localhost"` | Broker host name or address. |
| `mqtt.port` | `1883` | Broker port. |
| `mqtt.user` | none | Broker user. A password needs a user. |
| `mqtt.password` | none | Broker password (a secret). |
| `mqtt.client_id` | `"nidoo"` | MQTT client id and session key. Two hubs on one broker must not share one: the broker would bounce them off each other and hand one the other's session. |
| `z2m_base` | `"zigbee2mqtt"` | Zigbee2MQTT base topic; the hub reads `<z2m_base>/bridge/devices`, `/info` and `/state`. Under Advanced in the wizard. |
| `database_url` | none | The Postgres URL, built from the Database page's host, port (5432 for the bundled one), database, user and password. TimescaleDB 2.17+ on Postgres 17 with the `timescaledb` extension. Without it the hub stays in setup mode. |

The hub keeps a persistent MQTT session under `client_id`, so a write queued just before a connection blip is retransmitted on reconnect.

Two more MQTT fields have no screen. Set them in the `mqtt` object of `data/hub.json` while the hub is stopped, or with the environment variables below.

| Field | Default | Description |
| --- | --- | --- |
| `mqtt.send_ha_birth` | `false` | When `true`, publish `online` (not retained) on `ha_status_topic` each time the hub connects, so bridges re-publish their discovery. |
| `mqtt.ha_status_topic` | `"homeassistant/status"` | Topic of the birth message. |

### HTTP and log

No screen. Set with `NIDOO_HTTP__BIND`, `NIDOO_LOG__LEVEL` and `NIDOO_LOG__JSON`, or in the `http` and `log` objects of `data/hub.json` while the hub is stopped.

| Field | Default | Description |
| --- | --- | --- |
| `http.bind` | `"0.0.0.0:8081"` | Address the HTTP server listens on: the [HTTP API](http-api.md), the WebSocket, the MCP server and the web UI share it. 8081 sits next to Jeedom on 80 and clear of Zigbee2MQTT on 8080 on the same server. The Docker image keeps this default; change it with `NIDOO_HTTP__BIND` in `.env`. |
| `log.level` | `"info"` | A `tracing` filter: a level (`debug`) or a directive list (`nidoo=debug,rumqttc=warn`). `RUST_LOG` wins when it is set and valid; if it is unset or invalid, `log.level` applies, and if that is invalid too, the level is `info`. |
| `log.json` | `false` | `true` writes JSON lines; otherwise a human-readable format. Logs go to stderr. |

## Environment variables and flags

`NIDOO_*` variables override the **bootstrap** values only: the MQTT connection, the database URL, the HTTP bind address and the log settings. They are the only way to set them without the wizard, so an unattended install can script them. They win over `hub.json` at every start.

| Variable | Overrides |
| --- | --- |
| `NIDOO_MQTT__HOST`, `NIDOO_MQTT__PORT`, `NIDOO_MQTT__USER`, `NIDOO_MQTT__PASSWORD`, `NIDOO_MQTT__CLIENT_ID`, `NIDOO_MQTT__SEND_HA_BIRTH`, `NIDOO_MQTT__HA_STATUS_TOPIC` | `mqtt.*`. Setting any one of them makes the MQTT connection complete from the environment. |
| `NIDOO_HISTORY__URL` | `database_url`. |
| `NIDOO_HTTP__BIND` | `http.bind`. |
| `NIDOO_LOG__LEVEL`, `NIDOO_LOG__JSON` | `log.level`, `log.json`. |
| `NIDOO_DATA_DIR` | The data folder, like `--data`. |
| `RUST_LOG` | The log level, ahead of `log.level`. |

Two underscores separate the levels, and the names are upper case. String values keep the raw text of the variable, so `NIDOO_MQTT__PASSWORD=007` stays `"007"` and is not read as a number; numbers and booleans are parsed. In Docker, put them in a git-ignored `.env` next to `compose.yml`. Variables for any other section (`NIDOO_WEATHER__ENABLED` and the like) are ignored: those settings live in the database.

Command line:

| | |
| --- | --- |
| `nidoo` or `nidoo run` | Run the hub. |
| `nidoo mcp-stdio --server <url>` | Serve MCP over stdio, proxying to a running hub (for example `http://localhost:8081`). It reads the log level from `data/hub.json` when that exists. |
| `nidoo init-bundled` | Create the bundled broker and database credentials in `data/` and exit. The compose `init` service runs it; it changes nothing once `data/bundled.json` exists. |
| `--data <dir>` | The data folder (default `./data`, or `NIDOO_DATA_DIR`). Works with every command. |

## Weather

**Settings → Weather.** Outdoor weather from [Open-Meteo](https://open-meteo.com), a free service with no API key. Off by default: while `enabled` is `false` the hub makes no request at all. See [Weather](../users/weather.md).

| Field | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | Turn the weather on. Needs the home location. While `false`, `GET /api/v1/weather` answers 503 and `get_weather` answers `weather unavailable`. |
| `refresh_minutes` | `15` | Minutes between two fetches, with about 10 % random jitter. Between 10 and 1440 when enabled. |

The city, country and coordinates come from the home location. An enabled weather with a refresh below 10 or above 1440 is refused when you save, and so is enabling it with no home location.

## Solcast

**Settings → Solar forecast.** The Solcast rooftop solar forecast. Off by default: while `enabled` is `false` the hub makes no request at all. See [Solar forecast](../users/solar-forecast.md).

| Field | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | Turn the solar forecast on. While `false`, `GET /api/v1/solar` answers 503 and `get_solar` answers `solar unavailable`. |
| `sites` | `[]` | Rooftop resource ids from the Solcast Toolkit, one or more. Each id may hold only letters, digits, `-` and `_`. One request per site per refresh. |
| `daily_limit` | `10` | This hub's share of the account's requests per UTC day (the form calls it Calls per day). The hobbyist limit is 10. Refreshes per day are `(daily_limit - 2) / number of sites`, at least 1; 2 calls are kept in reserve. Must be at least the number of sites. |
| `api_key` | none | The Solcast API key (a secret). Required when enabled. |

When enabled, no site, an empty or invalid id, a `daily_limit` below the number of sites, or no key is refused when you save. The key is never written to a log line, an API response or MQTT.

## Frigate

**Settings → Cameras.** Frigate, for alert media and the event bridge. See [Cameras (Frigate)](../users/cameras.md).

| Field | Default | Description |
| --- | --- | --- |
| `url` | none | Frigate origin. With it set, media on that origin is served by the hub at `/api/v1/media`, so the browser never sees the login. Any other origin is refused. A blank string counts as unset. |
| `user` | none | Frigate login, for its login port (8971). Leave `user` and `password` out for the unauthenticated port (5000). |
| `password` | none | Frigate password (a secret). |
| `media_url` | none | Base URL the phone fetches snapshots and clips from; `url` when unset. Use the unauthenticated port (5000): the app sends no login. |
| `bridge` | `false` | Turn `<topic_prefix>/events` into `camera` push alerts. Needs `url` or `media_url`. The topic is subscribed only when the bridge is on. |
| `labels` | `[]` | Object labels to alert on. Empty: all. |
| `cameras` | `[]` | Cameras to alert on. Empty: all. |
| `live` | `false` | Serve live camera frames at `/api/v1/cameras/{id}/latest.jpg` and `/stream` (the dashboard Camera widget). Every device on the LAN can read them once it is on. Needs `url`. |
| `zones` | `[]` | No screen. Alert only once the object enters one of these zones. Empty: any. |
| `cooldown` | `0` | No screen. Seconds to ignore new events on a camera after an alert. |
| `severity` | `"warning"` | No screen. Severity of the alerts: `info`, `warning` or `critical`. |
| `ttl` | `43200` | No screen. Time to live of the alerts, in seconds. |
| `topic_prefix` | `"frigate"` | No screen. Frigate's MQTT `topic_prefix`. |

The hub also keeps a connection timeout (5000 ms) that no screen or route sets. The broker user needs read on `<topic_prefix>/events`. Frigate needs `snapshots.enabled: true`, and event clips retained (`record.enabled`) for the clip update.

## Push

**Settings → Notifications.** Push notifications, sent by the hub through Firebase Cloud Messaging. Without a key the hub keeps the alert history and sends nothing, with one warning at start. See [Push notifications](../users/push-notifications.md).

| Field | Default | Description |
| --- | --- | --- |
| `service_account` | none | The Firebase service-account key JSON (a secret). A key with no `project_id` is refused. A read also shows `project_id`. |
| `prefix` | `"nidoo/push"` | No screen. Topic prefix: `<prefix>/notification/<source>`, `<prefix>/tokens/<installId>`, `<prefix>/responses`, `<prefix>/history`. |
| `retry_interval` | `60` | No screen. Seconds between two passes of the retry queue. |
| `max_retries` | `10` | No screen. Attempts before a queued alert is given up. |
| `history_size` | `100` | No screen. Alerts kept in the history. |
| `history_days` | `7` | No screen. Days an alert stays in the history. |

## Climate

The climate engine (beta, still being tested). Switched on from the Climate card of the wizard's Services step. Everything except these two fields is edited through MCP, REST and the web Climate screen, as retained JSON under `prefix`. See [Climate](../users/climate.md).

| Field | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | Turn the engine on. While `false`, `GET /api/v1/climate` answers 503 and the climate MCP tools answer `climate disabled`. |
| `prefix` | `"nidoo/climate"` | No screen. Topic prefix of the climate documents. When enabled it must not be empty or only `/`, and must not contain `+` or `#`. |

## Advanced

The low-level knobs, with no screen. Read the section with `GET /api/v1/settings/advanced` (or `get_settings`), change what you need and send the whole document back with `PUT /api/v1/settings/advanced` (or `set_settings`): what you leave out takes its default.

### `topics`

Where the hub looks for discovery and where it keeps its own retained documents. See [MQTT topics](mqtt-topics.md) for what lives under each prefix.

| Field | Default | Description |
| --- | --- | --- |
| `discovery_prefixes` | `["nidoo"]` | Home Assistant discovery prefixes the hub subscribes to (`<prefix>/#`). The first one is also where the hub publishes the discovery of its own virtual entities and groups. |
| `ui_prefix` | `"nidoo/ui"` | Entity overrides (`<ui_prefix>/<id>`), rooms (`<ui_prefix>/_areas`), the type table (`<ui_prefix>/_types`) and dashboards (`<ui_prefix>/dashboards/<id>`). |
| `z2m_base` | `"zigbee2mqtt"` | Zigbee2MQTT base topic. The one in `hub.json` (the MQTT step's Advanced field) wins when it is set. |
| `automation_prefix` | `"nidoo/automation"` | Flow definitions: one retained JSON document per flow at `<automation_prefix>/<flow id>`. |

Until Zigbee2MQTT publishes under `nidoo`, list the Home Assistant prefix next to it so the discovery topics it still retains under `homeassistant/` are picked up too: `"discovery_prefixes": ["nidoo", "homeassistant"]`.

### `commands`

| Field | Default | Description |
| --- | --- | --- |
| `confirm_timeout_ms` | `5000` | How long a command waits for the entity's state echo before it reports `accepted: false`. Flow `command` nodes and `send_command` use it. |
| `subscription_debounce_ms` | `500` | How long the MQTT task waits after the set of state topics changes before it re-diffs its subscriptions. |

### `automation`

The flow engine.

| Field | Default | Description |
| --- | --- | --- |
| `runs_kept` | `20` | Recent runs remembered per flow, newest first. Runs live in memory only; a restart forgets them. A value of 0 is treated as 1. |

### `groups`

| Field | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | Kill switch for [flexible groups](../users/groups-and-virtual-entities.md). While `false`, `GET /api/v1/groups`, `PUT`/`DELETE /api/v1/groups/{id}` and the MCP group tools answer `groups disabled` (503 over HTTP), group commands are rejected, and the one-shot room-group migration is skipped. |

### `network`

| Field | Default | Description |
| --- | --- | --- |
| `enabled` | `true` | Kill switch for network presence watches. While `false`, the network watch routes and MCP tools answer `network presence disabled` (503 over HTTP) and no pings are sent. |

### `alarm`

The native alarm's topic prefix. The alarm itself is switched on in its settings document (**Settings → Alarm**, [MCP](mcp-tools.md#alarm), [HTTP](http-api.md#alarm)), so there is no `enabled` field here. See [Alarm](../users/alarm.md).

| Field | Default | Description |
| --- | --- | --- |
| `prefix` | `"nidoo/alarm"` | Topic prefix of the alarm's retained documents (`<prefix>/settings`, `<prefix>/state`) and of its panel topics. Must not be empty or only `/`, and must not contain `+` or `#`. |

### `watchdog`

Two opt-in watchdogs that raise `system` alerts, both off by default. See [Watchdogs](../users/watchdogs.md) for the alert wording and the broker permissions each source needs.

| Field | Default | Description |
| --- | --- | --- |
| `gateway.enabled` | `false` | Subscribe to `<prefix>/+`. |
| `gateway.prefix` | `"encryption-gateway/health"` | Topic prefix where the gateway publishes one retained state per component. Must not be empty when enabled. |
| `acl.enabled` | `false` | Alert on Mosquitto `Denied PUBLISH/SUBSCRIBE/UNSUBSCRIBE` log lines. |
| `acl.source` | `"topic"` | `"topic"` reads `log_topic` (needs `log_dest topic` in `mosquitto.conf`); `"file"` tails `log_file`. |
| `acl.log_topic` | `"$SYS/broker/log/#"` | Topic read when `source` is `"topic"`. Must not be empty in that case. |
| `acl.log_file` | none | Mosquitto log path, required when `source` is `"file"`. |
| `acl.cooldown` | `300` | Seconds during which repeats of one denial are held back; later denials are counted in the next alert. |

### `history_buffer_rows`

| Field | Default | Description |
| --- | --- | --- |
| `history_buffer_rows` | `100000` | States the history writer keeps while the database is unreachable, before it drops the oldest (one warning per outage). See [History](../users/history.md). |
