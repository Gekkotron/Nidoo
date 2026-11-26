# Climate: on/off heaters and "heated by" per schedule block — design

Date: 2026-11-26

## Why

A room can have more than one way to heat. The house this is built for has, in every bedroom,
a climate unit (a heat pump) and an electric radiator on a Zigbee relay, and in the living room
a climate unit and a stove powered through a smart plug. Today the climate engine drives only
`climate` entities (units with a mode and a target), so the radiators and the stove are outside
the schedule: nothing switches them, nothing protects them, and the schedule cannot say "the
stove heats the living room during the day, the heat pump otherwise".

The engine's v1 spec lists plug-driven heaters as a non-goal
(`2026-10-22-climate-engine-design.md`, "Heaters driven through a plain `switch`"). This design
lifts that non-goal for heating.

## Goals

- A room's settings list its **on/off heaters**: any `switch` entity, from any area (the stove
  plug sits in the garage and heats the living room).
- Each on/off heater has a **drive**:
  - `thermostat`: on below the room target, off above it. For a radiator.
  - `hold`: on for the whole time it is chosen, whatever the temperature. For a stove.
- Each on/off heater has a **cool-down**: once off, the engine does not switch it on again
  before `min_off_min` minutes. 15 for the stove, 0 by default.
- Each schedule block (heat rules) can say which devices heat the room during it
  (**Heated by**); the room has a default for the base temperature and for blocks that do not
  choose.
- While a chosen on/off heater cannot heat (cooling down, unreachable), the room's climate
  units **fill in** at the same target, when the room allows it.
- Generic: nothing names a stove, a radiator or a room.
- Every decision still says why, including "cooling down until 14:20".

## Non-goals

- Cooling. Cool rules and the cool season are untouched; on/off devices are never coolers.
- A zone for a room with no climate unit. Zones still come from areas holding a climate
  entity. (Every room concerned has one.)
- Learning on/off heaters. The learner keeps learning temperatures from the climate units'
  target history; it never proposes a "Heated by".
- Knowing whether a heater really heats (power metering, flame). The engine knows the switch's
  on/off state and availability, nothing more.
- Pilot-wire modes, PWM / time-proportional control, modulating a heater.
- The Android app.

## Anti-goals (a failure even if it works)

- The engine fights the user: a switch flipped by hand is flipped back on the next pass.
- A `hold` heater is cut and relit needlessly (between two touching blocks, on a setback, on a
  jittering reading).
- An on/off heater is switched on during its cool-down, by any path (schedule, boost, hand
  hold expiry, restart of the hub).
- The engine touches an on/off heater outside the heat mode. The stove's plug drives the
  garden watering in summer: in cool mode, season `off`, or a zone `off`, the engine sends it
  nothing at all, not even "off".
- Safety is loosened: an overheating room leaves a plug on, or a room with no usable reading
  keeps a `thermostat` radiator on.
- Existing zone documents change behaviour or fingerprint after the upgrade.

## Constraints (from the code today)

- `decide` (`crates/nidoo-climate/src/decide.rs`) is pure and gives every capable device the
  same target (`targets()`); a `Device` has hvac modes and a target, no kind.
- The hub filters on `Kind::Climate` / `StateValue::Climate` when it resolves a zone's
  entities (`inputs.rs`), detects hand changes (`engine.rs` `observe_climate`), and reads a
  command back (`send_target`).
- An unreachable device is already dropped from `devices` before `decide`.
- `Block` has no `Default`; about 30 literals build one. Climate documents ignore unknown
  fields and are retained JSON.
- `ModeRules::next_change` compares setpoints only.
- The learner rebuilds blocks from scratch (`learn`, `revise`) and replaces a mode's rules
  wholesale on approval; only `evidence` is carried over.
- The thermometer correction offsets every unheld device's setpoint.
- Engine pass every 60 s; at most one command per entity per 60 s.

## Approaches

1. **The engine drives on/off heaters itself (chosen).** One decision owner: which devices
   heat, the cool-down, the fill-in and the reason all come from `decide`. Safety has one
   owner. Cost: the climate-only assumptions listed above must each learn about switches.
2. **A hub-owned virtual thermostat** wrapping a switch and a sensor, so the engine keeps
   seeing only climate entities. Cheaper for the thermostat loop and free for the learner and
   hand changes, but "Heated by" and the fill-in need engine work anyway, the cool-down would
   be invisible to the decision ("Schedule 20 °C" while nothing heats), each heater would show
   twice in the app, and two loops would read one thermometer. Rejected.
3. **Flows** switching the plugs at set hours. Not in the schedule, no safety, no cool-down.
   Rejected.

The user handed the choice of approach to the builder; 1 is the builder's call.

## Data model

All new fields are optional with serde defaults and `skip_serializing_if`, so an existing
document serializes byte-for-byte as before (fingerprints unchanged).

`ZoneSettings` gains:

```jsonc
"heaters": [                      // on/off heaters of the room, default []
  { "entity": "0x…_switch_zigbee2mqtt",
    "drive": "thermostat",        // "thermostat" | "hold"
    "enabled": true,              // false: the engine leaves it alone entirely
    "min_off_min": 0,             // cool-down, 0..=240
    "min_on_min": 0 }             // thermostat only: minimum run, 0..=120
],
"heated_by": null,                // room default: null = the climate units (today's behaviour)
"fill_in": true                   // climate units heat while a chosen on/off heater cannot
```

`Block` (heat rules only) gains `"heated_by": ["<entity id>", …]`, absent = the room default.
A `heated_by` list names on/off heaters of the room and/or climate units of the zone. An empty
list is invalid. `heated_by` on a cool block is invalid.

Zone-wide thermostat band: a `thermostat` heater switches on at `target − 0.3 °C` and off at
`target` (constant, not a setting, in v1).

Validation (`ZoneSettings::validate` and `Hub::set_climate_zone`):

- a heater entity appears once; it is a `switch`; it is not a `group_*`; it is not a heater of
  another zone (same claim rule as `entities`);
- every id in a `heated_by` is a heater of this zone or one of its climate units. A block
  naming an id that later disappears (heater removed in the same save) is rejected with a
  message naming the block;
- `drive: hold` ignores `min_on_min`.

Runtime state, per on/off heater, carried in the retained `Decision` so it survives a restart
(as the correction offset does): `off_since` (Unix seconds of the last switch-off seen, by the
engine or by hand) and `on_since`. `DeviceTarget` for a switch uses `mode: "heat"` (on) or
`"off"`, `setpoint: null`, plus an optional `wait_until` when it is wanted on but cooling down.

## Behaviour

The engine acts on on/off heaters only when the zone's decided mode is **heat** and the zone is
`active` or `pending`. Otherwise they are not in the decision at all.

### Who heats

For a decision in heat with a temperature or off setpoint, the **chosen set** is:

| Source | Chosen set |
|---|---|
| schedule, block | the block's `heated_by`, else the room default |
| schedule, base | the room default |
| empty (room presence setback) | as the schedule block it lowers |
| boost / zone override | what the schedule would choose right now |
| eco, away, holiday | the room default, **without `hold` heaters** |
| safety frost | every climate unit and every `thermostat` heater |
| safety overheat | nobody: every on/off heater off |
| safety stale reading | climate units as today; every on/off heater off |
| pending | on/off heaters off (safety only) |

Devices of the room outside the chosen set are **off**: a climate unit gets what `Setpoint::Off`
gives it today (mode `off`, else its minimum), an on/off heater is switched off. Disabled
heaters are skipped everywhere.

Why `hold` heaters drop out under eco, away and holiday: a `hold` heater ignores the
temperature, so it cannot "save energy at 14 °C"; nobody is there, so it is off. This is the
builder's call.

### Driving a `thermostat` heater

Chosen, room reading known: on when reading ≤ target − 0.3, off when reading ≥ target, otherwise
unchanged. It respects `min_on_min` before going off (safety ignores it) and `min_off_min`
before going on. Setpoint `off` → off. Uses the same room reading as the decision.

### Driving a `hold` heater

Chosen with a temperature setpoint → on, whatever the reading. It goes off when it stops being
chosen, with one exception that avoids a needless cut: if it will be chosen again within
`min_off_min` minutes (the next block that names it starts that soon), it stays on through the
gap, and the reason says so. A setpoint of `off` → off.

### Cool-down

Switching **off** is always allowed. Switching **on** waits until `off_since + min_off_min`.
While waiting, the heater's target carries `wait_until` and the decision's reason reads, for
example, "Stove cooling down until 14:20". An unknown `off_since` after a first start counts as
"off since now" for a heater seen off, so a fresh hub never lights a stove early.

### Fill-in

With `fill_in` on: when a chosen on/off heater cannot heat (cooling down, unavailable, or its
command failing as the engine already tracks), and no climate unit is in the chosen set, the
zone's climate units heat to the same target until it can. They go back to off on the pass
where the heater is on again. The reason names it: "Stove cooling down until 14:20: the heat
pump heats meanwhile". A `hold` heater that is on never triggers fill-in, however cold the room
(the engine cannot know it has gone out; non-goal).

### Hand changes

A switch of the room flipped by someone else than the engine becomes a per-device hold, like a
target changed by hand on a climate unit: it holds until the zone's next schedule change, at
most `manual_hold_max_min`. A hand **off** starts the cool-down. When a hand hold ends, the
engine's wish applies again, cool-down included. `next_change` must count a block edge where
only `heated_by` changes.

A hand flip is a switch state the engine did not ask for: it differs from the last value the
engine commanded, and no engine command to that value is in flight or was sent within the
command confirmation window. Every observed switch-off, the engine's or a hand's, sets
`off_since`.

### Edges (builder's calls)

- **Leaving heat.** When a zone stops deciding in heat (season change, zone set to `off`), the
  engine sends one last **off** to each on/off heater it switched on itself and that is still
  on, then nothing more. A heater it did not switch on, a heater held by hand, and a disabled
  heater get nothing. Disabling a heater in the settings sends nothing either: the user takes
  over.
- **Cool-down beats everything, safety frost included.** No path switches a heater on before
  `off_since + min_off_min`. Under frost the climate units heat anyway.
- **No reading.** A `thermostat` heater with no usable room reading is off, whatever the
  source, and `min_on_min` does not delay it. A `hold` heater does not need a reading.
- **Stale ids.** A save is strict (see Validation). At decision time the engine is tolerant: an
  id in a `heated_by` that is no longer a heater or a climate unit of the zone is ignored; if
  none is left, the room default applies; if the default has none left either, the climate
  units heat. The reason says which name was ignored.
- **Removing a heater** drops its `off_since` / `on_since`. Added again and seen off, it counts
  as off since now.
- **Nobody can heat.** A chosen heater unreachable with `fill_in` off, or with the climate
  units unreachable too: nothing is commanded and the reason says so ("Stove unreachable:
  nothing heats the room"). The engine's existing command-failure alert covers the rest.

### What stays as it is

- Thermometer correction applies to climate units only; on/off heaters never get an offset and
  do not count in its step.
- Eco fan actions, the learner's inputs, decision history for climate units.
- A zone with no `heaters` and no `heated_by` decides exactly as today.

### Learner

Proposals never contain a new `heated_by`. When the learner rebuilds a mode's blocks from an
approved schedule, each new block inherits the `heated_by` of the approved block that overlaps
it most (same day set intersection, longest overlap in minutes); none overlapping → absent.
`differs` / `diff` ignore `heated_by`, so a schedule that differs only by it is not a change.

## Screens (web)

Room settings popup gains a **Heaters** section, above the thermometer:

```
Heaters
  Heat pump (Living room)                     climate unit
  Stove            Stays on ▾   Cool-down 15 min   [on]  ✕
  + Add a heater…                 (picker: switches, any room)
  Default heater    [Heat pump ✓] [Stove]
  [x] The heat pump takes over while a heater cannot run
```

Block editor gains a row after the Off checkbox, shown only in heat rules and only when the
room has an on/off heater:

```
Heated by   ( Room default )  [Heat pump]  [Stove ✓]
```

The week timeline marks a block that names its heaters with their short names under the
temperature. The room card's reason line carries the engine's sentence (cool-down, fill-in).
All strings go through `i18n/climate.rs` in English and French.

## Surfaces

- REST and MCP take the new fields through `ZoneSettings` and `Rules` as they are (shared
  types). `get_climate_zone` / `climate_status` show the heaters in the decision's `entities`.
- Docs: `docs/users/climate.md` (new "Heaters" section, the decision table, the cool-down),
  the MCP and HTTP references where they list zone fields, and a note in the v1 engine spec
  that this design lifts its switch non-goal.

## Testing

- `decide` unit tests, one per row of the "Who heats" table, plus: thermostat hysteresis,
  `min_on`/`min_off`, hold bridging a short gap, cool-down blocking a boost, fill-in start and
  end, heaters absent in cool mode, a zone without heaters unchanged.
- Hub tests (`crates/nidoo/tests/climate_hub.rs`): a switch fixture and simulator; on/off
  commands sent and confirmed; hand flip becomes a hold; restart inside a cool-down does not
  switch on; nothing is published to the switch in cool mode.
- Learner: `heated_by` survives a revise + approve; fingerprints of old documents unchanged.
- Web: block editor row, settings section, i18n completeness.

## Left to the builder

- How a switch is presented to `decide` (a kind on `Device`, or an adapter) and how the ledger
  records on/off.
- Exact wording of reasons and labels, in both languages.
- Whether the Heaters section is its own component file.
- Rollout order across crates.
