---
title: Architecture
sidebar_position: 2
description: The crates of the workspace and how a message travels from the broker to the API, the web UI and the flow engine.
---

Nidoo is one Cargo workspace (`crates/*`) with one binary, `nidoo`. The hub keeps no per-device code: devices reach it as Home Assistant MQTT discovery on the broker, and everything else is built on top of the registry that discovery fills. See [Adding a device](adding-a-device.md) for the contract.

## Crates

| Crate | Role |
| --- | --- |
| [`nidoo-core`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-core) | The pure domain: discovery parsing, the registry, state and command mapping, the entity type table, energy, history bucketing, dashboards and the flow model, validation, schedule and sun maths. No I/O, no async. Compiles to native and `wasm32-unknown-unknown`. |
| [`nidoo-api`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-api) | The wire types shared by the hub and the web UI: views over the registry, the snapshot, WebSocket events and REST bodies. Pure data. Also builds for wasm. |
| [`nidoo`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo) | The hub binary and library: config, the MQTT task, the `Hub`, the HTTP/WebSocket/MCP server, the flow engine, groups, virtual entities, dashboards, history, the climate engine's hub side, the Frigate media proxy. |
| [`nidoo-web`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-web) | The browser UI, a Leptos app compiled to wasm with Trunk. The hub embeds its build output (`dist/`) and serves it. |
| [`nidoo-push`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-push) | Push notifications: payload normalisation, the token map, the retry queue, the alert history and the FCM transport. Pure at the bottom; the service takes the clock and a publisher as arguments. |
| [`nidoo-frigate`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-frigate) | The Frigate event bridge: a pure translator from `<frigate>/events` messages to v1 push alerts, with per-event and per-camera memory. |
| [`nidoo-watchdog`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-watchdog) | The gateway-health and broker-ACL watchdogs as pure translators to `system` push alerts, plus a Mosquitto log tailer. |
| [`nidoo-climate`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-climate) | The climate engine's documents, decision tree, schedule learning and topics. Pure data and logic, no I/O; the hub and the web UI share it. |
| [`nidoo-assist`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-assist) | The assistant: turns a short French sentence and a snapshot of the house into a plan (actions, an answer, a question back, or "not understood") with the safety checks for heating setpoints. Pure data and logic, no I/O; the hub executes the plans and the web UI shares the wire types. |

`nidoo-core` depends on no other crate. `nidoo-api`, `nidoo-push` and `nidoo-climate` depend on it; `nidoo-frigate` and `nidoo-watchdog` depend on no workspace crate. `nidoo-assist` depends on `nidoo-core`, `nidoo-api` and `nidoo-climate`. `nidoo` (native) depends on all of them, and `nidoo-web` (wasm) on `nidoo-core`, `nidoo-api`, `nidoo-climate` and `nidoo-assist`.

## Data flow

```text
 Zigbee2MQTT / bridges / firmware          phone app, producers
            |  discovery, state                  |  tokens, notifications
            v                                    v
        +-------------------- MQTT broker (Mosquitto) --------------------+
        |   retained: discovery, nidoo/ui/*, nidoo/automation/*, ...       |
        +-----------------------------+------------------------------------+
                                      ^ subscribe / publish
                               [ mqtt task ]  (nidoo)
                                      |  Hub::ingest
                                      v
   registry (nidoo-core)  ->  entity states  ->  event broadcast
        |                                          |        |        |
        |                                          v        v        v
        |                                    HTTP / WS    MCP     flow engine
        |                                    (REST, UI)  tools    climate engine
        |                                                          history writer
        +--<-- commands, overrides, areas, flows, groups --<-- publish queue
```

### Discovery to state

1. The MQTT task subscribes to `<discovery prefix>/#`, `<z2m_base>/bridge/#`, `<ui_prefix>/#`, `<automation_prefix>/#` and the push, climate, Frigate and watchdog topics that are on (see [MQTT topics](../reference/mqtt-topics.md)).
2. Every message goes to `Hub::ingest`. The registry classifies it: a discovery config upserts or removes entities, an `nidoo/ui/<id>` message is an override, `_areas` is the room list, and a topic an entity listed as its state topic is a state message.
3. The registry applies discovery configs (expanding Home Assistant abbreviations and device-based configs), computes the folding of entities into each other, and maps each state payload through the entity's templates into a typed state.
4. The hub turns the registry's changes into events (`entity_upserted`, `state_changed`, `areas_changed`, and so on) on a broadcast channel, and feeds changed states to the history writer.

After a restart the registry only knows what the broker replays as retained, so a sensor that does not retain its state stays unknown until its next message. Once the link is first up and the replay has settled (10 seconds), the hub restores the last recorded value from the history for each entity that has not reported yet, stamped with the time it was recorded. It does so once per process, and the restore is broadcast like a state change but is neither a report nor a new history row.

### Reads: HTTP, WebSocket, MCP

`/api/v1/snapshot` and the `nidoo://snapshot` MCP resource build the same `Snapshot` from the registry. `/ws` sends that snapshot, then every broadcast event. The web UI loads the snapshot, applies the events, and reads and writes through the same REST routes the assistants use. The MCP tools call the same `Hub` methods as the REST handlers, so both surfaces share validation and error mapping. A write refused because the broker link is down is a `503` over HTTP and an in-band tool error over MCP. See [HTTP API](../reference/http-api.md) and [MCP tools](../reference/mcp-tools.md).

### Commands

1. A client sends a command (`POST /api/v1/entities/{id}/command`, `send_command`, a flow `command` node or a group fan-out).
2. The hub maps it to one or more MQTT publishes using the entity's discovery config, and refuses it before publishing if the entity is unknown, offline or cannot take it.
3. The publishes go to the MQTT task through a bounded queue.
4. The hub waits up to the `commands.confirm_timeout_ms` setting for the entity's value to change. The reply is `accepted: true` with the new state once the echo arrives, or `accepted: false` with the reason.

Overrides, areas, flows, groups, virtual entities and dashboards follow a shorter path: the hub publishes the retained document and applies it locally at once, so the reply already reflects it. The broker's echo is then a no-op.

### Flow engine

The flow store keeps the decoded definitions read from `<automation_prefix>/#` and the recent runs. The engine task watches the broadcast events: a state change fires the `trigger.state` nodes watching that entity, a button press or a signal fires the matching nodes, and timers fire `trigger.schedule` and `trigger.sun` nodes. A run executes the graph depth first; `command` nodes go through the same command path, `notify` nodes go to the push service, and `delay` nodes sleep. Each step is recorded and broadcast as `flow_run_changed`. See [Flow nodes](../reference/flow-nodes.md).

### Push

`nidoo-push` normalises an alert (from a producer on `nidoo/push/notification/<source>`, or raised in process by the Frigate bridge, the watchdogs, flows or MCP), adds it to the history, and sends it through FCM to every registered install. Failed sends are retried from memory. The history is the retained `<prefix>/history` topic itself. The hub waits for the subscription acknowledgement and the retained tokens before it sends after a connect.

### History writer

The ingest path offers each changed state to a bounded channel with `try_send`, so a slow database never blocks the house. The writer task batches rows into TimescaleDB, retrying with backoff while the database is down and dropping the oldest pending rows past the `history_buffer_rows` setting (advanced section). A rollup task keeps an hourly table for the chart queries behind `/api/v1/history/*`. Secret entities are never offered to it.

### Climate engine

With the climate engine on, the hub holds the retained documents under the climate `prefix` and runs an evaluation loop. Per zone, `nidoo-climate` decides from a priority tree (safety, override, holiday, away, schedule); the hub commands the devices through the normal command path, tells a hand change on a device from the engine's own echo, and raises safety alerts through push. A learner reads history, proposes a schedule, and nothing applies until it is approved. See [Climate](../users/climate.md).
