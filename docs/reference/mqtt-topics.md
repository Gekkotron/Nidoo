---
title: MQTT topics
sidebar_position: 6
description: Every MQTT topic the hub reads or writes, with its direction, retention, payload and the setting that moves it.
---

The table lists every topic the hub reads or writes, with its default prefix. "In" means the hub subscribes; "out" means the hub publishes. A prefix is shown in `<angle brackets>` or as its default, and the last column names the setting that moves it (see the [Settings reference](configuration.md)). Settings in the last column are written `[section] field`: `[topics]`, `[watchdog.gateway]` and `[watchdog.acl]` are in the **advanced** settings, `[groups]` and `[alarm]` too (see the [Settings reference](configuration.md#advanced)); `[weather]` is **Settings → Weather**, `[solcast]` **Settings → Solar forecast**, `[frigate]` **Settings → Cameras**, `[push]` **Settings → Notifications**, `[climate]` the Climate card of the setup wizard; `[mqtt] send_ha_birth` and `ha_status_topic` are in the `mqtt` object of `data/hub.json`. The broker user (the MQTT user of **Settings → Broker**) needs read and/or write on each topic accordingly.

| Topic | Direction | Retained | Payload | Moved by |
| --- | --- | --- | --- | --- |
| `<discovery prefix>/<component>/[<node_id>/]<object_id>/config`, default `nidoo/…` | in | yes (by the publisher) | Home Assistant discovery JSON; an empty payload removes the entity | `[topics] discovery_prefixes` |
| Device state topics named by the discovery configs | in | by the device | Device state; subscribed on demand | discovery |
| Device command topics named by the discovery configs | out | only when the config says `"retain": true` | The device's command payload | discovery |
| `<z2m_base>/bridge/devices`, `/info`, `/state` | in | yes (by Zigbee2MQTT) | Zigbee2MQTT device list, bridge info and state | `[topics] z2m_base` |
| `<ha_status_topic>`, default `homeassistant/status` | out | no | `online`, once per connect | `[mqtt] send_ha_birth`, `[mqtt] ha_status_topic` |
| `nidoo/ui/<entity id>` | in, out | yes | The entity's [override](entity-types.md#overrides) JSON; empty clears it | `[topics] ui_prefix` |
| `nidoo/ui/_areas` | in, out | yes | The declared rooms (`{"areas": […]}`) | `[topics] ui_prefix` |
| `nidoo/ui/_home` | in, out | yes | The home settings (`{"eco_setpoint", "mode_entity", "indoor_entity"}`); an empty payload means defaults | `[topics] ui_prefix` |
| `nidoo/ui/_types` | out | yes | The whole [type table](entity-types.md#the-type-table) | `[topics] ui_prefix` |
| `nidoo/ui/dashboards/<id>` | in, out | yes | A dashboard definition; empty removes it | `[topics] ui_prefix` |
| `nidoo/ui/ambiances/<id>` | in, out | yes | An ambiance definition; empty deletes it | `[topics] ui_prefix` |
| `nidoo/automation/<flow id>` | in, out | yes | A [flow](flow-nodes.md) document; empty removes it | `[topics] automation_prefix` |
| `<first discovery prefix>/<switch\|number\|button\|select>/virtual/<id>/config` | out | yes | Discovery of a virtual entity | `[topics] discovery_prefixes` (first entry) |
| `<first discovery prefix>/virtual/<id>` | in, out | yes for a switch, number or select; no for a button | `ON` / `OFF`, a number, an option name, or `PRESS`; the state and the command topic | `[topics] discovery_prefixes` (first entry) |
| `<first discovery prefix>/<sensor\|binary_sensor>/weather/<key>/config` | out | yes | Discovery of a weather entity (19 keys, one device) | `[topics] discovery_prefixes` (first entry), `[weather]` |
| `<first discovery prefix>/weather/<key>` | out | yes | A weather value: a number, an enum name or `ON` / `OFF`; `None` (unknown) once the source stops reporting a value that was published | `[topics] discovery_prefixes` (first entry), `[weather]` |
| `<first discovery prefix>/weather/availability` | out | yes | `online` once weather was fetched, `offline` before the first fetch and after an hour without one | `[topics] discovery_prefixes` (first entry), `[weather]` |
| `<first discovery prefix>/sensor/solar/<key>/config` | out | yes | Discovery of a solar forecast entity (10 keys, one device) | `[topics] discovery_prefixes` (first entry), `[solcast]` |
| `<first discovery prefix>/solar/<key>` | out | yes | A solar forecast value: a number in kWh or kW, or an RFC 3339 UTC time for the peak time | `[topics] discovery_prefixes` (first entry), `[solcast]` |
| `<first discovery prefix>/solar/availability` | out | yes | `online` once a forecast covers now, `offline` otherwise (see [Solar forecast](../users/solar-forecast.md#freshness-and-offline)) | `[topics] discovery_prefixes` (first entry), `[solcast]` |
| `<first discovery prefix>/solar/cache` | out, in | yes | The retained forecast and call count, so a restart does not spend calls; never the API key | `[topics] discovery_prefixes` (first entry), `[solcast]` |
| `<first discovery prefix>/<component>/group/<id>/config` | out | yes | Discovery of a group entity | `[groups] enabled`, `[topics] discovery_prefixes` |
| `<first discovery prefix>/group/<id>` | in, out | yes (state) | The combined state; commands arrive here as JSON | `[groups] enabled` |
| `<first discovery prefix>/group/<id>/definition` | in, out | yes | `{kind, name, members, hideMembers}` | `[groups] enabled` |
| `<first discovery prefix>/group/<id>/avail` | out | yes | `online` | `[groups] enabled` |
| `<first discovery prefix>/group/_migrated` | in, out | yes | `1`, the marker that the one-time room-group migration ran | `[groups] enabled` |
| `<first discovery prefix>/binary_sensor/network/<id>/config` | out | yes | Discovery of a network watch entity | `[network] enabled`, `[topics] discovery_prefixes` |
| `<first discovery prefix>/network/<id>` | out | yes | `ON` (connected) or `OFF` (disconnected); nothing until the first ping verdict | `[network] enabled` |
| `<first discovery prefix>/network/<id>/definition` | in, out | yes | `{name, host, intervalS, enabled, area, device}` | `[network] enabled` |
| `<first discovery prefix>/network/<id>/avail` | out | yes | `online`, or `offline` while the watch is paused or the hub cannot ping | `[network] enabled` |
| `nidoo/push/notification/<source>` | in | no | An alert, v1 or legacy payload | `[push] prefix` |
| `nidoo/push/tokens/<installId>` | in; out to clear | yes | `{token, device_name, lang, updated_at}`; empty unregisters. The hub publishes an empty payload to clear a dead token | `[push] prefix` |
| `nidoo/push/responses` | in (climate and alarm), out | no | `{id, action, install_id, ts}`, a tapped alert button | `[push] prefix` |
| `nidoo/push/history` | in, out | yes | JSON array of the normalised alerts, newest first | `[push] prefix` |
| `nidoo/climate/house` | in, out | yes | The house settings document | `[climate] prefix` |
| `nidoo/climate/zone/<zone>/<doc>`, `<doc>` being `settings`, `proposal`, `override`, `decision` or `eco` (`{until, set_at}`, the zone's eco until a date; the hub deletes it once ended) | in, out | yes | A zone document | `[climate] prefix` |
| `<prefix>/settings` (alarm) | in, out | yes | The alarm settings document; `enabled` defaults to false. Default prefix `nidoo/alarm` | `[alarm] prefix` |
| `<prefix>/state` (alarm) | in, out | yes | The alarm's runtime: state, mode, source, timers, motion hits, open sensors and the open alert | `[alarm] prefix` |
| `<prefix>/panel` (alarm) | out | yes | The panel entity's state: `disarmed`, `arming`, `armed_away`, `armed_home`, `armed_night`, `pending` or `triggered` | `[alarm] prefix` |
| `<prefix>/panel/attributes` (alarm) | out | yes | The panel's JSON attributes: `source`, `since`, `deadline`, `cause`, `open_at_arm` | `[alarm] prefix` |
| `<prefix>/panel/set` (alarm) | in | no | The panel's commands: `ARM_AWAY`, `ARM_HOME`, `ARM_NIGHT` or `DISARM`. A retained replay is never acted on | `[alarm] prefix` |
| `<first discovery prefix>/alarm_control_panel/alarm/house/config` | out | yes | Discovery of the alarm panel, entity `nidoo_alarm_house`. Published while the alarm is on; an empty payload removes it | `[topics] discovery_prefixes` (first entry), `[alarm] prefix` |
| `nidoo-sound/speak` | out | no | `{"id", "url", "players"}`: a sentence for the `nidoo-sound` service to fetch (`url`, `/api/v1/assist/speech/<id>.wav` on the hub) and say on the satellites listed by short id | — |
| `nidoo-sound/hold` | out | no | `{"id", "players", "for_ms"}`: while someone talks to a satellite, the music of the listed satellites (short ids) is replaced by silence for `for_ms` (at most 60 000) unless refreshed; speech is still heard. `for_ms: 0` releases that `id`. Never saved: a restart of `nidoo-sound` brings the music back | — |
| `nidoo-sound/spoken` | in | no | `{"id", "played"}`: the sentence with clip `id` finished playing on its satellites (`true`, the instant its last sample plays) or was dropped (`false`) | — |
| `nidoo-sound/<sid>/address` | in | yes | The satellite's IPv4 address while it is connected (empty otherwise); its discovery entity is a diagnostic sensor folded into the satellite's Volume | — |
| `<topic_prefix>/events`, default `frigate/events` | in | no | Frigate event JSON; subscribed only with the bridge on | `[frigate] bridge`, `[frigate] topic_prefix` |
| `<prefix>/+`, default `encryption-gateway/health/+` | in | yes (by the gateway) | `{"status": "ok" \| "fail" \| "config_error", …}` per component | `[watchdog.gateway] enabled`, `prefix` |
| `$SYS/broker/log/#` | in | no | Mosquitto log lines | `[watchdog.acl] source = "topic"`, `log_topic` |

## Notes

- **Reserved sub-trees.** Under a discovery prefix, the segments `ui`, `automation`, `virtual`, `push`, `weather` and `solar` are reserved: never publish discovery there. A topic in them is never read as discovery and never shows under `GET /api/v1/ignored`.
- **Climate documents never end in `/config`.** They cannot be read as discovery. The hub subscribes to `<prefix>/house` and `<prefix>/zone/+/+`, never `<prefix>/#`. `nidoo/push/responses` is always subscribed: a reply whose id starts with `alarm-` goes to the alarm, and the others (a tapped proposal button, for example) go to the climate engine when it is on.
- **Overlapping subscriptions.** The hub subscribes to `<discovery prefix>/#`, `<z2m_base>/bridge/#`, `<ui_prefix>/#` and `<automation_prefix>/#`. A push filter that falls inside one of those wildcards (the default `nidoo/push/…` under `nidoo/#`) is left out, so the broker does not deliver a notification twice.
- **Alerts raised inside the hub** (the Frigate bridge, watchdogs, flows, MCP) go to the push service in process, without the broker.
- **Retained history.** `nidoo/push/history` is the history itself: the hub adopts it at its first sync.
- **After each (re)connect** the hub waits for its subscription to be acknowledged, then 2 s more for the retained tokens, before it sends; alerts arriving meanwhile wait in the queue.
- **Flows and dashboards read as written.** An invalid flow or dashboard document is kept and shown with its reason, never run.
- **Broker permissions.** Give the hub user read and write on `nidoo/#` (or the narrower sets above), write on `nidoo-sound/speak` and `nidoo-sound/hold` (satellite speech and listening), read on `<topic_prefix>/events` and `<gateway prefix>/+`, and for the ACL watchdog read on `$SYS/broker/log/#`. Mosquitto denies silently.
