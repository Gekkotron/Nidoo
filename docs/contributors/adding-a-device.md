---
title: Adding a device
sidebar_position: 3
description: How a new device reaches the hub as MQTT discovery, and when a type-table entry or a Rust kind is needed.
---

The hub speaks Home Assistant MQTT discovery. It holds no per-device code. Each device reaches the hub as discovery configs on the MQTT broker, and each discovered component becomes an entity of a `Kind`. MQTT discovery is the plugin API, so adding a device usually means changing whatever publishes its discovery, not the hub.

## Which path?

| Situation | Path | Where the change goes |
|---|---|---|
| A Zigbee device Zigbee2MQTT does not know | [Fix it in Zigbee2MQTT](#zigbee-devices-unknown-to-zigbee2mqtt) | External converter, then upstream PR. Not a Nidoo change. |
| The device is not Zigbee (cloud API, Wi-Fi, vendor protocol) | [Write a bridge](#writing-a-bridge) | Separate process, any language, own repo. It publishes discovery to the broker. |
| The device publishes discovery for a component Nidoo ignores (`GET /api/v1/ignored` reports `ignored component <name>`) | [Add a type or a kind](#adding-a-component-kind-to-nidoo) | Usually a JSON entry in `crates/nidoo-core/src/types/table.json`. A Rust kind only when the table cannot describe it. PR to Nidoo, one per component, never per device. |
| The device is a type Nidoo describes but reports no state for the primary field | [Type table](#a-type-table-entry-json-no-rust) | Bridge or firmware: publish the state topic of the primary field, retained. A `requires_state` type is rejected without it. |
| The device works but looks wrong (wrong room, wrong name, a relay that is really a heater or light) | [Override](#overrides) | No code. Web device sheet, `PATCH /api/v1/entities/{id}/override`, or MCP `set_override`. |

Some firmwares already publish discovery when their MQTT discovery option is on, Tasmota, ESPHome and Valetudo among them. Those devices need no bridge.

## Zigbee devices unknown to Zigbee2MQTT

Zigbee2MQTT (Z2M) holds the per-device code for thousands of Zigbee devices. When Z2M does not know a device:

1. Write an external converter for local use. It is a JavaScript file referenced in Z2M's configuration.
2. Contribute it upstream to `Koenkk/zigbee-herdsman-converters` so every user gets it.

Z2M publishes discovery under the `oikos` prefix. Once Z2M exposes the device, Nidoo picks it up with no change. See [Zigbee2MQTT](../users/zigbee2mqtt.md).

## Writing a bridge

A bridge is a separate process. It reads a non-Zigbee device and publishes that device's discovery and state to the MQTT broker. The bridge can be written in any language and lives in its own repository. The hub needs no change.

### The contract

Discovery topic:

```
<prefix>/<component>/[<node_id>/]<object_id>/config
```

Between the component and `config` there is one or two segments. None may be empty. The default prefix is `oikos`.

A bridge that publishes under `homeassistant` works once the hub lists that prefix: add `"homeassistant"` to `topics.discovery_prefixes` in the `advanced` section (`["oikos", "homeassistant"]`), with `PUT /api/v1/settings/advanced` or the `set_settings` MCP tool. See the [Settings reference](../reference/configuration.md#advanced).

Rules:

- Publish the config **retained**, as a UTF-8 JSON object.
- The hub expands Home Assistant abbreviations (`stat_t`, `cmd_t`, `dev`, `~`, and so on).
- Device-based discovery is supported: component `device` with a `cmps` or `components` map, one entry per entity, each with a `platform`.
- Remove an entity by publishing an empty retained payload on its config topic.
- `ui`, `automation`, `virtual` and `push` directly under the prefix are reserved. `oikos/ui/<id>` holds overrides, `oikos/automation/<id>` holds flows, `oikos/virtual/<id>` the state of virtual entities and `oikos/push/...` the push topics. Never publish discovery there. See [MQTT topics](../reference/mqtt-topics.md).

Hand-written kinds (each becomes an entity):

`light`, `switch`, `cover`, `climate`, `sensor`, `binary_sensor`, `alarm_control_panel`, `number`, `select`, `button`, `vacuum`, `water_heater`.

Type-table components (each becomes an entity from its table entry, no Rust needed):

`camera`, `device_automation`, `device_tracker`, `event`, `fan`, `humidifier`, `image`, `lawn_mower`, `lock`, `notify`, `scene`, `siren`, `tag`, `text`, `update`, `valve`.

Any other component is listed under `GET /api/v1/ignored` as `ignored component <name>`.

### Fields the hub reads

| Field | What the hub does with it |
|---|---|
| `unique_id` | The entity id. Set it and keep it stable. Without it the hub uses `<component>.<node_id>.<object_id>`. |
| `device.identifiers` | The first string is the entity's `device_id`. Entities sharing it form one device. |
| `device.name` | The device part of the display name. |
| `name` | The entity label. `null` makes the entity the device itself, named after the device. |
| `device.suggested_area` | The room. |
| `state_topic` | Required for `sensor` and `binary_sensor`. The config is rejected without it. Publish state retained so the hub has a value after a restart. |
| `availability_topic` | Online and offline payloads come from `payload_available` and `payload_not_available` (defaults `online` and `offline`). |
| `availability` | A list of topics, combined by `availability_mode` (`all`, `any` or `latest`). |
| `entity_category` | `diagnostic` or `config` hides the entity from the default entity list. |
| `enabled_by_default` | `false` hides the entity from the default entity list. |
| `device_class`, `unit_of_measurement`, `icon` | Passed through to the UI. |

Display name: `<device name> <label>`. A `device.name` with three or more `/` segments, such as `rooms/kitchen/kettle`, gives the short name `kettle` and the room `kitchen`.

Room priority: user override, then `suggested_area`, then the room from the `/` path, then Unassigned.

Without an availability topic, availability stays `unknown`. Pair `availability_topic` with an MQTT last will so the hub shows the device offline when the bridge dies.

### Worked example: Roborock2Mqtt

Roborock2Mqtt (https://github.com/Gekkotron/Roborock2Mqtt) talks to an S5 vacuum and publishes this config (as the hub stores it):

```json
{
  "unique_id": "roborock_40313ca8b964",
  "object_id": "roborock_40313ca8b964",
  "name": null,
  "device": {
    "identifiers": ["roborock_40313ca8b964"],
    "manufacturer": "Roborock",
    "model": "roborock.vacuum.s5",
    "name": "Roborock",
    "sw_version": "3.5.8_002034"
  },
  "availability_topic": "roborock/alive",
  "payload_available": "1",
  "payload_not_available": "0",
  "state_topic": "roborock/ha/state",
  "command_topic": "roborock/ha/command",
  "set_fan_speed_topic": "roborock/ha/fan_speed/set",
  "fan_speed_list": ["Silent", "Standard", "Medium", "Turbo", "Gentle", "Auto"],
  "supported_features": ["start", "pause", "stop", "return_home", "locate", "fan_speed", "battery", "status"]
}
```

- `name: null` makes the entity "Roborock".
- `supported_features` and `fan_speed_list` tell the hub which actions and fan speeds exist.
- `roborock/alive` with `1` and `0` drives online and offline.
- The hub has no Roborock code.

Vacuum wire format:

- `state_topic` carries JSON `{"state", "battery_level", "fan_speed"}`. `state` is one of `cleaning`, `docked`, `paused`, `idle`, `returning`, `error`.
- Commands are published on `command_topic` as `start`, `pause`, `stop`, `return_to_base` or `locate`. Each payload can be overridden (`payload_start` and so on).
- A fan speed is published as the plain speed name on `set_fan_speed_topic`.

A second example is Suez2Mqtt (https://github.com/Gekkotron/Suez2Mqtt). It reads a water meter from the Suez cloud and publishes `sensor` entities with `device_class: water`. It publishes no availability topic, so its availability shows `unknown`.

A device whose firmware already speaks MQTT but publishes no discovery needs no bridge at all: publish its discovery once, retained, by hand. WaterHeaterController (https://github.com/Gekkotron/WaterHeaterController) works this way; [WaterHeaterController](../users/water-heater-controller.md) holds its one-line `mosquitto_pub`. Its `water_heater` entity derives everything from the firmware's `water-heater/data` JSON with templates: the tank temperature, a mode of `heat` or `off`, and the card attributes below.

Water heater card attributes, read from `json_attributes_topic` (after `json_attributes_template`) when present:

- `power`: the heating power, 0 to 100 %. The card shows "Heating · N %" when it is above 0 and "Idle" otherwise.
- `inlet_temperature`: the cold-water inlet in °C, shown under the tank temperature.

Without them the card falls back to the mode. The target stepper appears only with a target temperature, and the mode picker only with `modes`.

### Restarts

With `mqtt.send_ha_birth` on (off by default; set it in the `mqtt` object of `data/hub.json` while the hub is stopped, or with `NIDOO_MQTT__SEND_HA_BIRTH=true`), the hub publishes `online` (not retained) on `mqtt.ha_status_topic` (default `homeassistant/status`) each time it connects. A bridge subscribes to that topic and re-publishes its discovery and state when it sees `online`.

Retained configs cover most restarts. The birth message covers a broker that lost its retained messages.

### Checking it

- `GET /api/v1/ignored`: every discovery topic that did not become an entity, with the reason (bad JSON, `sensor without state_topic`, ignored component, and so on).
- `GET /api/v1/snapshot`: the entity under `entities`, with state and availability. The snapshot's `devices` list holds Zigbee2MQTT devices only. A bridge's device appears through its entities' `device_id`.
- MCP `get_entity`: the entity with its `bindings` (the expanded discovery config). Use it to confirm abbreviations expanded as expected.

Security: bindings are readable by anything on the LAN. Never put credentials in topics or templates.

## Adding a component kind to Nidoo

Two routes. Most components need only a type-table entry. Write a Rust kind only when the component needs behaviour the table cannot express: a new state model, a command mapping of its own, or energy reporting.

### A type-table entry (JSON, no Rust)

Find or add the entry in `crates/nidoo-core/src/types/table.json` (the covered components are listed in [Entity types](../reference/entity-types.md#the-type-table)). Entries sit under the top-level `types` object, keyed by component name, inside a file shaped `{"version": 1, "types": {"<component>": {...}}}`:

| Key | Meaning |
|---|---|
| `placement` | Where the entity shows: `card` (Home), `signal` (a trigger for flows), `target` (something flows can send to) or `update` (a firmware mark). |
| `icon` | A glyph name for the card. |
| `primary` | The field name shown on the card, or `null`. |
| `requires_state` | When `true`, discovery is rejected without the primary field's topic. |
| `fields` | A map of `field_name` to `{ shape, topic, template?, values?, options?, range?, on? }`. `shape` is `on_off`, `number`, `choice`, `text`, `picture` or `version`. `topic` is the config key holding the state topic, `template` the config key holding its value template. `values` lists the allowed values, each with the config `key` that may override its payload and the Home Assistant `default`. `options` names a list in the config (such as `preset_modes`). `range` is `{min, max, step}`. `on` is the value that counts as on. `json_path`, for a `text` or `version` shape, reads one top-level key when the rendered template produces a JSON object. |
| `controls` | A map of `control_name` to `{ shape, topic, template?, field?, press?, payload?, payload_on?, payload_off?, json_key?, requires?, labels? }`. `topic` is the config key of the command topic. `press: true` makes a button, which has no state. `payload` is a WordDef `{value, default}`, which may also carry an optional `key` as the other WordDefs do. `field` ties the control to a field it changes. |
| `main` | The default main control, in preference order. |
| `alerts` | `[{field, any_of}]`: field values that put the card in alert style. |
| `history` | `{field, map}`. `map` turns the field's text values into numbers, such as `{"on": 1.0, "off": 0.0}`. An empty `map` keeps a number as it is. |

A short example, the `device_tracker` entry:

```json
"device_tracker": {
  "placement": "card",
  "icon": "tracker",
  "primary": "state",
  "requires_state": true,
  "fields": {
    "state": {
      "shape": "choice",
      "topic": "state_topic",
      "template": "value_template",
      "values": [
        {"value": "home", "key": "payload_home", "default": "home"},
        {"value": "not_home", "key": "payload_not_home", "default": "not_home"}
      ]
    }
  },
  "history": {"field": "state", "map": {"home": 1.0, "not_home": 0.0}}
}
```

The hub publishes the whole table retained on `oikos/ui/_types` (`{ui_prefix}/_types`) on every connection, so the Android app reads the same vocabulary. Its own echo is ignored. Camera and image entities carry their picture in the `picture` field. `GET /api/v1/entities/{id}/picture` serves the stored bytes, or the bytes of its URL fetched by the hub now (10 s timeout, 5 MiB cap, no redirects followed).

Check it with `GET /api/v1/ignored` (the component must no longer appear there) and `GET /api/v1/snapshot` (the entity must show its primary field). A type-table entry needs no other change to appear in the flow editor: `nidoo_core::vocabulary` turns its announced controls into actions and its primary field into the values an `if` card offers.

### A hand-written kind (Rust)

Base the PR on how `water_heater` was added. A kind touches these files:

| File | Change |
|---|---|
| `crates/nidoo-core/src/model/kind.rs` | Add the `Kind` variant, its component name and `Kind::ALL`. Remove the name from `IGNORED_COMPONENTS`. |
| `crates/nidoo-core/src/discovery/capabilities.rs` | What the kind can do, read from the config. |
| `crates/nidoo-core/src/model/state.rs` | State model for the kind. |
| `crates/nidoo-core/src/state/kinds/<kind>.rs`, `state/kinds/mod.rs` | Reading the kind's state. |
| `crates/nidoo-core/src/command/kinds.rs` | Mapping commands to publishes. |
| `crates/nidoo-core/src/flow/scalar.rs` | The kind's scalar for flows. |
| `crates/nidoo-core/src/vocabulary.rs` | What the kind accepts (`actions`) and reports (`reports`): the flow editor's Action and Value lists and the ambiance device action read them. |
| `crates/nidoo-core/src/energy/units.rs` | Only if the kind reports energy. |
| `crates/nidoo-web/src/cards/<kind>.rs`, `icons.rs`, `format.rs`, `screens/home.rs` | Web card. |

The Android app needs the matching card.

Tests run against recorded broker messages in `crates/nidoo-core/tests/fixtures`. Add cases to `crates/nidoo-core/tests/state.rs`. `crates/nidoo-core/tests/vocabulary.rs` runs every action `vocabulary::actions` lists through the command mapper, so the editors never offer a command the hub refuses; add a hand-written config there when the recording has no entity of the new kind.

Before opening the PR, run the commands in [Development](development.md#checks): `cargo test`, `clippy -D warnings`, `fmt --check`, and the wasm32 checks for `nidoo-core`, `nidoo-api` and `nidoo-web`.

## Overrides

The override has twenty-one fields (`name`, `area`, `hidden`, `order`, `cardStyle`, `favorite`, `energyRole`, `energyInvert`, `securityHidden`, `energyIcon`, `energyMeter`, `homePinned`, `securityAction`, `securityAsk`, `ungrouped`, `showAs`, `lightOnPreset`, `lightOnBrightness`, `climatePresets`, `mainControl`, `invert`). `showAs` makes a switch show as a light, door, siren, chime or plug; `mainControl` is the name of an announced control, or `"none"` to hide the main control; `invert` swaps on and off for a switch wired the other way round. [Entity types](../reference/entity-types.md#overrides) lists every field with its values.

Overrides are retained messages under `oikos/ui/<entity id>`. Set them from the web device sheet, `PATCH /api/v1/entities/{id}/override`, or MCP `set_override`.

## Virtual entities

A virtual entity (see also [Groups and virtual entities](../users/groups-and-virtual-entities.md)) is a switch, a number or a button the hub owns, with no device behind it: a "force heating" flag, a setpoint, a manual trigger for a flow. Create one with `PUT /api/v1/virtual/{id}` (body `{"kind": "switch" | "number" | "button", "name": "…"}`, plus `min`, `max`, `step`, `unit` for a number and `initial` for either state) or MCP `set_virtual`; remove it with `DELETE /api/v1/virtual/{id}` or `delete_virtual`. The id is 1 to 48 characters of `a-z`, `0-9` and `_`, and the entity becomes `virtual_<id>`, on the device `Virtual`.

Under the first discovery prefix `P` (normally `oikos`), the hub publishes, retained:

| What | Topic |
|---|---|
| Config | `P/<switch\|number\|button>/virtual/<id>/config` |
| State and command (switch, number) | `P/virtual/<id>` |
| Command (button) | `P/virtual/<id>` |

A switch (`ON` / `OFF`) and a number echo on their own topic and are retained (`retain: true` in the config), so a command is its own state and survives a restart. A number takes `min`, `max` and `step` (0, 100 and 1 by default). A button has no state and is not retained: a press is an event, not a value.

The hub publishes the initial state, retained, only the first time the entity is created: `OFF` for a switch, `min` for a number, or `initial`. A flow's trigger needs a known previous value to see a change, so a first tap would otherwise go unnoticed. Calling `PUT` again on an existing entity republishes its config (a rename, other bounds) and keeps its state. `virtual` is a reserved sub-tree: its topics never show in `GET /api/v1/ignored`.

A virtual entity is an ordinary entity otherwise: commands go through `send_command`, and it moves to a room with the usual override (`PATCH /api/v1/entities/virtual_<id>/override`, or the web device sheet).

## Bridge catalogue

| Bridge | Devices | Repo |
|---|---|---|
| Suez2Mqtt | Suez water meter (cloud) | https://github.com/Gekkotron/Suez2Mqtt |
| Roborock2Mqtt | Roborock vacuums (S5 tested) | https://github.com/Gekkotron/Roborock2Mqtt |

To list a bridge, open a PR that adds one row. The bridge's code stays in its own repository.
