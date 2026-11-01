---
title: Entity types
sidebar_position: 5
description: The kinds and the JSON type table that describe entities, the folding and masking rules, and every override field.
---

Every discovered component becomes an entity of a `Kind`. Twelve kinds are hand-written in Rust; every other component is described as data by a JSON type table, compiled into the hub. Components that are neither are reported under `GET /api/v1/ignored`. To add one, see [Adding a device](../contributors/adding-a-device.md).

## Kinds

| Kind | Home Assistant component |
| --- | --- |
| `light` | `light` |
| `switch` | `switch` |
| `cover` | `cover` |
| `climate` | `climate` |
| `sensor` | `sensor` |
| `binary_sensor` | `binary_sensor` |
| `alarm_panel` | `alarm_control_panel` |
| `number` | `number` |
| `select` | `select` |
| `button` | `button` |
| `vacuum` | `vacuum` |
| `water_heater` | `water_heater` |
| `generic` | any component the type table describes; the component name is in the entity's `component` |

Only `device` (device-based discovery) is folded by routing: it expands into one entity per entry. Any other component that the hand-written kinds and the table do not describe is reported as `ignored component <name>` under `GET /api/v1/ignored`.

## The type table

[`crates/oikos-core/src/types/table.json`](https://github.com/Gekkotron/nidoo/blob/main/crates/oikos-core/src/types/table.json) is shaped `{"version": 1, "types": {"<component>": {…}}}`. Each entry says where the entity shows, which field is its primary state, which controls it offers, what puts it in alert style and how its history is charted.

Sixteen components are covered:

| Component | Placement | Primary field | Controls | Notes |
| --- | --- | --- | --- | --- |
| `camera` | `card` | `picture` | none | Picture served by `/api/v1/entities/{id}/picture`. |
| `device_automation` | `signal` | `last` | none | Remotes; several configs of one device fold into one entity. |
| `device_tracker` | `card` | `state` | none | `state` required. History: `home` 1, `not_home` 0. |
| `event` | `signal` | `last` | none | `state` required. |
| `fan` | `card` | `state` | `power`, `percentage`, `preset`, `oscillation`, `direction` | Main: `power`. History: on 1, off 0. |
| `humidifier` | `card` | `state` | `power`, `target`, `mode` | Main: `power`. History charts `target`. |
| `image` | `card` | `picture` | none | Picture served like a camera. |
| `lawn_mower` | `card` | `activity` | `start`, `pause`, `dock` | Main: `start`. Alert style on `error`. |
| `lock` | `card` | `state` | `locked`, `open` | `state` required. Main: `locked`. Alert style on `unlocked`, `jammed`. History: locked 1, unlocked 0. |
| `notify` | `target` | none | `message` | A target flows can send to. |
| `scene` | `card` | none | `activate` | Main: `activate`. |
| `siren` | `card` | `state` | `power`, `tone` | `state` required. Main: `power`. Alert style when on. |
| `tag` | `signal` | `last` | none | Tag scanners. |
| `text` | `card` | `value` | `value` | Main: `value`. |
| `update` | `update` | `latest` | none | Fields `installed` and `latest` feed the firmware marks. |
| `valve` | `card` | `state` | `valve`, `stop` | `state` required. Main: `valve`. History: open 1, closed 0. |

### Entry keys

| Key | Meaning |
| --- | --- |
| `placement` | Where the entity shows: `card` (Home), `signal` (a trigger for flows), `target` (something flows can send to) or `update` (a firmware mark). |
| `icon` | A glyph name for the card. |
| `primary` | The field name shown on the card, or `null`. |
| `requires_state` | When `true`, discovery is rejected without the primary field's topic. |
| `fields` | A map of `field_name` to `{ shape, topic, template?, values?, options?, range?, on?, json_path? }`. `shape` is `on_off`, `number`, `choice`, `text`, `picture` or `version`. `topic` is the config key holding the state topic, `template` the config key holding its value template. `values` lists the allowed values, each with the config `key` that may override its payload and the Home Assistant `default`. `options` names a list in the config (such as `preset_modes`). `range` is `{min, max, step}` with defaults and optional config keys. `on` is the value that counts as on. `json_path`, for a `text` or `version` field, reads one top-level key when the rendered template produces a JSON object (how a Zigbee2MQTT `update` entity exposes `installed_version` and `latest_version` from one template). |
| `controls` | A map of `control_name` to `{ shape, topic, template?, field?, press?, payload?, payload_on?, payload_off?, json_key?, requires?, labels? }`. `topic` is the config key of the command topic. `press: true` makes a button, which has no state. `payload` is a word definition `{value, default}`, which may also carry a `key`. `field` ties the control to a field it changes. `json_key` sends `{json_key: value}` instead of the bare value. `requires` is an extra config key that must be present to announce the control. `labels` are the card's button words for an on/off control. |
| `main` | The default main control, in preference order. |
| `alerts` | `[{field, any_of}]`: field values that put the card in alert style. |
| `history` | `{field, map}`. `map` turns the field's text values into numbers, such as `{"on": 1.0, "off": 0.0}`. An empty `map` keeps a number as it is. |

### Publication

The hub publishes the whole table retained on `<ui_prefix>/_types` (default `oikos/ui/_types`) on every connection, next to `oikos/ui/_areas` and the per-entity `oikos/ui/<id>` overrides, so the Android app reads the same vocabulary. Its own echo is ignored.

## Behaviours

- **Secret text.** A `text` entity with `mode: password` is masked as `••••` on every state path out of the hub (REST, WebSocket, MCP) and is left out of history.
- **Remotes fold into one entity.** Several `device_automation` configs for one device fold into a single remote entity, whose `capabilities.generic.signals` lists their payloads. Zigbee2MQTT announces an action only after it first sees it, so the hub also seeds each Zigbee2MQTT remote with every value of the `action` enum its device exposes in `bridge/devices` (in that order, marked `"seeded": true`): a double press is a signal before anyone double-presses, and a remote that was never pressed is listed. An announced action replaces its seeded twin. `trigger.signal` in a flow fires on those remotes, on events and on tags.
- **Main control.** The override `mainControl` names one announced control, or `"none"` to hide the main control. Without it the card leads with the first announced entry of the type's `main` list.
- **Firmware marks.** `update` entities feed the firmware marks (`installed`, `latest`, `available`) that the device view shows. A device with several `update` entities shows the first that reports a version.
- **Pictures.** Camera and image entities carry their picture in the `picture` field.

### Folding

Some entities are folded into another entity's card (`part_of`). A folded entity is not primary, so `list_entities` hides it unless you pass `include: "all"`. The rules are recomputed after every change, so the result does not depend on arrival order:

- A device with exactly one light folds its `number` and `select` entities into that light (a strip's "Effect speed" belongs in the light's sheet). With two lights the owner is ambiguous: no fold.
- Zigbee2MQTT publishes a cover and a separate "Moving" sensor off the same state payload. A sensor folds into the cover of its device when both read the same `state_topic` and the sensor's `value_template` reads exactly one JSON field that the cover's template also reads. A binary sensor with `device_class: moving` on a device with exactly one cover folds too. The cover rule wins a conflict.

An entity is **primary** (shown by default) when it has the primary category, is enabled by default, is not folded and is not hidden by its override. `entity_category: diagnostic` or `config` and `enabled_by_default: false` keep it out.

## Overrides

An override is a retained JSON document at `<ui_prefix>/<entity id>` that changes how one entity appears. Set it from the web device sheet, `PATCH /api/v1/entities/{id}/override` or MCP `set_override`; `null` clears a field. Unknown fields and invalid values are refused and nothing is written. These twenty fields are the app's, in its order:

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Display name. |
| `area` | string | Room id (a raw room name is resolved). |
| `hidden` | bool | Hide the entity from the default lists. |
| `order` | integer | Sort position. |
| `cardStyle` | string | Card style of a sensor's card in the app. |
| `favorite` | bool | Mark as favourite. |
| `energyRole` | string | `solar`, `grid`, `grid_export`, `battery`, `house`, `consumer` or `water`: puts the entity on the Energy tab. |
| `energyInvert` | bool | Flip the sign of an energy reading whose role is signed (a direction). |
| `securityHidden` | bool | Take a sensor off the Security tab. |
| `energyIcon` | string | Icon of a `consumer` on the Energy diagram. |
| `energyMeter` | string | The energy counter paired with a power reading on the Energy tab. |
| `homePinned` | bool | Keep the entity on Home although Energy or Security already shows it. |
| `securityAction` | bool | Offer a `switch` or `button` as an action on the Security tab. |
| `securityAsk` | bool (default `true`) | Ask for confirmation before the Security action. |
| `ungrouped` | bool | Keep the entity out of the derived room groups used by the one-time group migration. |
| `showAs` | string | `light`, `door`, `siren`, `chime` or `plug`: what a switch drives. Only read on a switch. Absent is a plain switch. |
| `lightOnPreset` | string | `white`, `warm`, `red`, `green` or `blue`: the colour a light takes when switched on. |
| `lightOnBrightness` | integer | 1 to 255: the brightness a light takes when switched on. |
| `climatePresets` | string | Preset buttons of a climate entity's card in the app. |
| `mainControl` | string | A generic entity's main control: one of its announced controls, or `none`. |
| `invert` | bool | A switch wired the other way round (relay on = device off). The hub swaps on and off in the state it decodes and the commands it sends, so the web UI, flows, ambiances, history and MCP all see the device's real state. Only read on a switch. |

A newer app's unknown fields survive a read-modify-write. For compatibility with an app that only knows the retired `asLight` flag, the hub writes `asLight: true` next to `showAs: "light"`, and reads a stored `asLight` as `showAs: "light"`.

Home leaves out what Energy or Security already shows, unless `homePinned` is set: an entity with an `energyRole`, a security-class binary sensor or alarm panel (unless `securityHidden`), or an opted-in `securityAction` leaves Home.

Room priority for an entity: the override `area`, then the discovery `suggested_area`, then the room from the device name's `/` path, then `Unassigned`.
