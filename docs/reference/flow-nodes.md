---
title: Flow nodes
sidebar_position: 4
description: The flow document format and every node type with its parameters, outputs and rules.
---

A flow is an n8n-style automation: a small graph of nodes stored as one retained JSON document under `<automation_prefix>/<id>` (default `nidoo/automation/<id>`) on the broker. The hub subscribes to that prefix, validates every document it reads, and runs the valid, enabled ones. An invalid document is kept and shown with the reason, but never runs. For a guided introduction, see [Flows](../users/flows.md).

The model is in [`crates/nidoo-core/src/flow`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo-core/src/flow) (`model.rs`, `validate.rs`, `sun.rs`) and the engine in [`crates/nidoo/src/flows`](https://github.com/Gekkotron/Nidoo/tree/main/crates/nidoo/src/flows).

## Flow document

```json
{
  "id": "living_led_motion", "name": "Living LED on motion", "enabled": true, "mode": "restart",
  "nodes": [
    { "id": "motion", "type": "trigger.state", "params": { "entity": "0x00158d00044c4d4c_occupancy_zigbee2mqtt" } },
    { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
    { "id": "on", "type": "command", "params": { "entity": "<light id>", "command": { "type": "turn_on" } } },
    { "id": "wait", "type": "delay", "params": { "for": "5m" } },
    { "id": "off", "type": "command", "params": { "entity": "<light id>", "command": { "type": "turn_off" } } }
  ],
  "edges": [
    { "from": "motion", "to": "moving" },
    { "from": "moving", "port": "true", "to": "on" },
    { "from": "moving", "port": "false", "to": "wait" },
    { "from": "wait", "to": "off" }
  ]
}
```

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | 1 to 64 characters of `a-z`, `0-9`, `_` and `-`. It is the last topic segment. |
| `name` | string | Free text shown in the UI. |
| `description` | string, optional | Free text. |
| `enabled` | bool, default `true` | A disabled flow keeps its definition but never runs. |
| `mode` | `single` (default) or `restart` | What a trigger does while a run of the same flow is still active. `single` ignores the new trigger and records a skipped run. `restart` cancels the active run and starts a new one, which is what makes "5 minutes after the last motion" work: motion coming back cancels the pending `delay`. |
| `area` | string, optional | The room (an area id) the Flows page lists the flow under. It does not change what the flow does. The hub checks it on save: an unknown room is refused, a room name is resolved to its id, and `Unassigned` is not a room for a flow. A room id already stored and since deleted is kept as it is, so it never blocks a save. |
| `labels` | array of strings, optional | The label ids the flow carries (see [Flow labels](#flow-labels)). They group the flow on the Flows page and let a label switch turn it on or off; they do not change what the flow does. The hub sorts and de-duplicates them on save and refuses an id the catalog does not have ("unknown label `x`"), unless the stored flow already carries it: a label deleted later never blocks a save. Changing only `labels` does not restart a run in progress. |
| `nodes` | array | The steps. |
| `edges` | array | The connections, followed from a trigger onwards. |

Unknown keys, on the flow, on a node and in its params, are kept as they are, so a newer writer's fields survive. A `null` reads as absent.

Each node is `{"id", "type", "params", "name"?, "position"?}`:

- `id`: unique in the flow, 1 to 32 characters of `a-z`, `0-9`, `_` and `-`. Edges refer to it.
- `type`: one of the types below.
- `name`: an optional label shown on the canvas.
- `position`: optional `{x, y}` canvas coordinates. The hub ignores it.

Each edge is `{"from", "to", "port"?}`. An edge leaving a branching node must carry one of its ports: `"true"` or `"false"` for an `if`, `"1"` to `"10"` or `"otherwise"` for a `switch`, `"in"` or `"out"` for a `time.between`. Omit a port to do nothing there. An edge leaving any other node has no `port`.

A flow is refused when:

- an id is malformed or a node id is used twice;
- an edge names a missing node, a port that does not exist, or lacks the port a branching node (`if`, `switch`, `time.between`) needs;
- there is no trigger node;
- the graph has a cycle (the error names the path);
- a node's params are invalid (see each node below).

On save the hub also checks the house: every entity a node names must exist, a `command` must map for that entity, a `flow.enable` target must be an existing flow, every id in `labels` must be a label of the catalog, and a `flow.label` must name an existing label (one in a choice for `pick`).

### How a run goes

A run follows the graph from the trigger that fired, depth first. A failing node ends the run as `failed` at that step. A branching step (`if`, `switch`, `time.between`) that succeeds records the `ports` it followed, which can be none (a `switch` whose rules all missed); the web UI uses them to light the right wires. Other steps carry no `ports`. A non-trigger node with no incoming edge never runs. A run that is skipped (because of `single` mode, or because the broker link is offline when a schedule or sun trigger fires) is recorded as a skipped run.

The **scalar** is the one value of a state that triggers and `if` compare: the on/off bool, the number, the text, a light's `on`, a cover's `state`.

Runs live in memory only: a hub restart forgets them, the flow definitions stay on the broker. The `automation.runs_kept` setting (default 20, see the [Settings reference](configuration.md#automation)) sets how many recent runs are remembered per flow.

### The house position

Sun nodes need the position of the house. Set your home location in the setup wizard or **Settings → Location** (see the [Settings reference](configuration.md#home-location)). With none set, the hub uses Paris and saving a flow that has a `trigger.sun` or an `if` with `op: is_dark` returns a warning. Schedules use the home location's time zone.

## `trigger.state`

Starts a run when an entity's scalar changes to a different value.

| Param | Type | Description |
| --- | --- | --- |
| `entity` | string | The entity id (from `search` or `list_entities`). |
| `field` | string, optional | A value inside the entity's state (vacuum `battery`, `fan_speed`; light `brightness` (in %), `color_temp`, `effect`; cover `position`; climate `current`, `target`, `fan_mode`, `action`; water heater `current`, `target`; a type-table entity's other fields). Absent: the entity's headline value. The trigger fires when that value changes, `to` and `from` compare it, and the run carries it as the trigger value. A field the entity does not report refuses the save: "node `x`: `entity` reports no field `f`". |
| `to` | JSON value, optional | Fire only when the new scalar equals it (`true`, `"ON"`, `21.5`). |
| `from` | JSON value, optional | Fire only when the previous scalar equalled it. |

Outputs: one. The run carries the entity's new value.

- The replayed value after boot or reconnect arms the entity silently. It never fires on that replay, on an availability-only change, or on an equal-value report. A change needs a known previous value and a known new one.
- A **button** (a virtual or discovered `button`) has no value: a press is an event and fires the trigger with the value `PRESS`. A button trigger takes no `to` or `from`, and a retained message is a replay, never a press.
- The trigger is refused on entities that send signals (remotes, events, tags): use `trigger.signal`.
- **Unknown, then a value.** A trigger with a `to` and no `from` also fires when its value was unknown and then becomes `to`. The unknown must have been confirmed by a report the hub received while the flow was watching it (any state message on the entity's topic, even one that carried no value for it, such as a detector that sends `null` or omits its alarm field until it is set). The value vanishing again and returning fires again. It stays silent on the first value seen after boot or a reconnect, on a value restored from the recorded history (the hub never restores over an entity that has sent any message since boot), at flow save, when the value goes away, and for any trigger without `to` or with a `from`. A missing key counts as unknown, which assumes the bridge sends every field it knows in each message (Zigbee2MQTT's default `cache_state: true`); a bridge that publishes one field at a time would make the first report of a field fire.

## `trigger.signal`

Starts a run when a remote, an event entity or a tag scanner sends a signal. Like a press, a signal is an event the hub never replays from retained messages, and a repeated signal is a new one.

| Param | Type | Description |
| --- | --- | --- |
| `entity` | string | A `device_automation.*`, `event.*` or `tag.*` entity id. |
| `signal` | string, optional | Fire only for this signal name. Absent: any signal of the entity. |

Outputs: one. The trigger value is the signal name.

The entity must send signals (otherwise the hub says to use `trigger.state`), and a `signal` must be one the entity announces: its `capabilities.generic.signals`, or for an `event` the options of its last value.

## `trigger.schedule`

Starts a run at each wall-clock tick while the flow is enabled.

| Param | Type | Description |
| --- | --- | --- |
| `cron` | string | Five fields: minute, hour, day of month, month, day of week. Seconds and year forms are refused. |

Outputs: one. The cron runs in the home location's time zone (the system zone when none is set). The first run is at the next tick. Ticks missed while the hub was busy or down are never replayed. `GET /api/v1/schedule?cron=…` previews the next ticks.

An `if` or a `switch` reachable from a schedule trigger (or a sun trigger) must name an `entity`, because these triggers carry no value; the exceptions are an `if` with `op: is_dark` or `op: is_off_peak`. A `time.between` reads the clock and needs none.

## `trigger.sun`

Starts a run at a solar event.

| Param | Type | Description |
| --- | --- | --- |
| `which` | `sunrise`, `sunset`, `dusk` or `dawn` | The event. `sunrise` and `sunset` are the apparent horizon crossing (sun centre at -0.833 degrees). `dusk` and `dawn` are civil twilight (sun centre at -6 degrees). |
| `offset_minutes` | integer, optional | Minutes added to the event time; negative fires earlier. Between -720 and 720; 0 when absent. |

Outputs: one.

The hub arms the next event within the coming week, in the house zone. Events missed while the hub was busy are never replayed: the next one is searched from now. Where the sun never crosses the threshold (polar night, midnight sun) nothing is armed. While the broker link is offline when the event is due, the run is recorded as skipped.

## `trigger.off_peak`

Starts a run when off-peak starts or ends.

| Param | Type | Description |
| --- | --- | --- |
| `edge` | `start` or `end` | `start` fires when peak turns to off-peak, `end` when off-peak turns to peak. |

Outputs: one. The trigger value is `"start"` or `"end"`; the trigger entity is the off-peak device when its reading decided, none when the windows did.

Off-peak comes from the home settings (**Settings → Home**, `off_peak_device` and `off_peak` in the [HTTP API](http-api.md)): the device's reading when it is usable, else the windows in force that day, else never. A reading is not usable while the entity is offline, has no value, or reads empty, `unknown` or `unavailable`. With a device and no windows, no usable reading means peak.

- The hub re-reads the answer when the device reports and at each window start or end. It fires only on a change it saw happen: never at boot, on the broker's replay after a reconnect, when a flow is saved, or when the home settings change (those only take the new answer as the starting point).
- A device going offline while its windows disagree is a change, and fires.
- While the broker link is offline at a window boundary, the run is recorded as skipped.

## `trigger.silent`

Starts a run when an entity has sent no report for a while: a sensor whose battery died, one out of range, one unpaired.

| Param | Type | Description |
| --- | --- | --- |
| `entity` | string | The entity id (from `search` or `list_entities`). It must exist. |
| `for` | string | A duration in the `delay` format (`50m`, `1h30m`). At least `1m`, at most `168h`. |

Outputs: one. The trigger value is the `for` text (`50m`), so `{trigger}` in a notify prints it; the trigger entity is the silent entity. The step detail reads `<entity> silent for <for>`.

- A report is any state message the hub receives on one of the entity's state topics, changed or not, and even when it carries no value for the entity (a `null` field, a missing key). A message on an availability-only topic is not a report. A value restored from the recorded history at boot is not a report.
- The trigger fires once per silence. It fires again only after the entity reports and then stays silent for `for` again.
- Each clock starts when the node is armed: when the hub starts, when the flow is saved or enabled, and when the broker link comes back. Saving a flow without changing the node's `entity` or `for` keeps its clock; changing either restarts it. Nothing fires at boot, on a save or on enable before `for` has elapsed.
- While the broker link is down nothing can be reported, so the clocks are frozen: nothing fires, and every clock restarts when the link returns. If the hub misses reports because it is overloaded, every clock restarts too. A broker link that reconnects more often than `for` therefore keeps restarting the clocks, and nothing ever fires.
- With `mode: single`, a due clock whose flow still has a run active does not fire and is not recorded as skipped: it fires once that run ends, unless the entity reports first.
- The trigger is refused on entities that never report state: buttons and entities that send signals (remotes, events, tags).
- Clocks live in memory and start again after a hub restart.

```json
{ "id": "silent", "type": "trigger.silent", "params": { "entity": "binary_sensor.smoke", "for": "50m" } }
```

## `limit`

Lets a run through at most a given number of times in a rolling window; past that the run ends at this node.

| Param | Type | Description |
| --- | --- | --- |
| `max` | integer | How many runs may pass, from 1 to 100. |
| `per` | string | The window, in the `delay` format. At least `1m`, at most `168h`. |

Outputs: one, with no port.

- A run reaching the node passes when fewer than `max` runs passed it in the last `per`. The window is rolling, not reset at midnight. A pass is counted when a run reaches the node, whatever happens after it: a run cancelled later, or a manual run, still used up a pass. Put the node right before the action it protects.
- Over the limit, the run ends at this node with status `done`, and nothing after it runs. The step has outcome `ok` and the detail `limited: 4 in the last 24h`; a pass reads `passed (2 of 4 in the last 24h)`.
- The count is kept per flow and node, in memory: a hub restart forgets it. Saving the flow keeps it while the node exists with the same `max` and `per`; removing or changing the node, or deleting the flow, drops it. Disabling a flow keeps it.

```json
{ "id": "cap", "type": "limit", "params": { "max": 4, "per": "24h" } }
```

## `trigger.manual`

Starts a run when someone presses its button on the card, from the web UI, `POST /api/v1/flows/{id}/run` or the `run_flow` MCP tool. It never fires on its own.

| Param | Type | Description |
| --- | --- | --- |
| `value` | boolean, number or string, optional | The value the run carries. |

Outputs: one. No input.

- Without a `value`, it is a valueless trigger like `trigger.schedule`: an `if` or `switch` reachable from it must name an `entity` (`op: is_dark` excepted), or the flow is refused with "node `x`: an if after a schedule, sun or manual trigger without a value must name an entity" (a `switch` reads "a switch after ...").
- With a `value`, an `if` or `switch` without an `entity` compares that value, and `{trigger}` in a notify text prints it. Such an `if` or `switch` cannot use `average`: "node `x`: an average after a manual trigger must name an entity".
- The run's step detail reads `manual`, or `manual: <json value>` when the card has a value.

## `if`

Branches on a comparison.

| Param | Type | Description |
| --- | --- | --- |
| `op` | `eq`, `ne`, `gt`, `lt`, `gte`, `lte`, `is_dark` or `is_off_peak` | The comparison. |
| `value` | bool, number or string | The constant to compare with. Required for every op except `is_dark` and `is_off_peak`, where it is refused. |
| `entity` | string, optional | The entity whose current scalar is read, live. Absent: the entity that triggered the run. Refused with `is_dark` and `is_off_peak`. |
| `field` | string, optional | A value inside `entity`'s state, as for [`trigger.state`](#triggerstate); a brightness is compared in %. Needs `entity` ("node `x`: a field needs an entity"). With `average`, only the entity's recorded number: cover `position`, climate and water heater `current` ("node `x`: `f` of `entity` has no history to average"). Without `entity`, the field the trigger read is compared, and `average` is refused after a trigger on a field with no history. |
| `missing` | `fail`, `true` or `false`, optional | What to do when the value is unknown. Refused with `is_dark` and `is_off_peak`. |
| `average` | duration, optional | A window such as `15m` or `1h`, at most `24h`. Compare the mean of the entity's readings over that window instead of its current value. `value` must then be a number. Refused with `is_dark` and `is_off_peak`. |
| `offset_minutes` | integer in [-720, 720], optional | With `is_dark` only. Moves civil dusk by this many minutes and civil dawn by the opposite. Absent means 0. Refused with every other op. |
| `result` | string, optional | The `name` of a [`math`](#math) card. Its result of this run is compared instead of an entity's value, as a number. Refused together with `entity`, `average` or `missing`; `value` must then be a number. A result missing from the run fails the node (`no result x in this run`); `missing` does not apply to it. |

Outputs: two ports, `true` and `false`. Every edge leaving an `if` names one.

- `eq` and `ne` are typed: values of different types are never equal, and an unknown value equals nothing, so `ne` holds.
- `gt`, `lt`, `gte` and `lte` need both sides numeric; two known values that are not both numbers fail the node. An unknown value is `false`.
- `missing` set to `true` or `false` takes that port when the value is unknown, before any comparison.

:::caution
`missing: "fail"` (and leaving `missing` out) does not fail the node today: an unknown value goes through the comparison above, so `eq` and the ordering ops take `false` and `ne` takes `true`. Set `missing` to `true` or `false` when the branch matters.
:::
- `average` is the plain mean of the readings received in the window ending now, the newest included. While a flow uses it the hub keeps every reading of that entity, repeats included; for the part of the window before that (after a hub restart, or right after the flow is saved) it reads history, which holds one row per change, so a repeated reading there counts once. With no reading in the window the current value is the mean. The value is unknown, so `missing` applies, when the entity is unknown, unavailable or not a number. History is optional: without it the average starts from the readings received since the flow was loaded. The run detail shows what was compared, for example `average 30m = 18.6 → true`.
- `is_dark` is true between civil dusk and civil dawn (sun centre 6 degrees or more below the horizon) at the house position, now. It reads the sun, not an entity. With `offset_minutes: -30` it is true from 30 minutes before civil dusk until 30 minutes after civil dawn, which suits indoor spaces that get dark before the street does; a positive offset shortens the dark at both ends.
- `is_off_peak` is true during off-peak, now, with the same rule as [`trigger.off_peak`](#triggeroff_peak): the off-peak device's usable reading, else the windows. It takes no operand. The run detail reads `is_off_peak → true` or `false`.

Example: a temperature sensor averaging 19 or less over 30 minutes takes `true`, so a single noisy reading does not trigger the branch.

```json
{ "id": "low", "type": "if", "params": { "entity": "sensor_temperature", "op": "lte", "value": 19, "average": "30m", "missing": "false" } }
```

## `switch`

Branches on several values at once: one output per rule, tried in order. It is an `if` with many comparisons.

| Param | Type | Description |
| --- | --- | --- |
| `rules` | array of rules | 1 to 10 rules. Required. See below. |
| `entity` | string, optional | The entity whose current scalar is read, live. Absent: the entity that triggered the run. |
| `field` | string, optional | A value inside `entity`'s state, with the same rules as the [`if`](#if) `field`. |
| `average` | duration, optional | A window such as `15m` or `1h`, at most `24h`. Compare the mean of the entity's readings over that window, as for the [`if`](#if) `average`. Every rule `value` (and `to`) must then be a number. |
| `result` | string, optional | The `name` of a [`math`](#math) card. Its result of this run is switched on instead of an entity's value. Refused together with `entity` or `average`; every rule's `value` must then be a number. A result missing from the run fails the node. |
| `otherwise` | bool, default `false` | Adds an `otherwise` port, taken when no rule matches. |
| `all` | bool, default `false` | Take every matching rule instead of the first one only. |

A rule is `{"op", "value", "to"?}`:

| Field | Type | Description |
| --- | --- | --- |
| `op` | `eq`, `ne`, `gt`, `lt`, `gte`, `lte` or `between` | The comparison. |
| `value` | bool, number or string | The constant compared with. With `between` it is the lower bound. |
| `to` | number | The upper bound. Required with `between`, refused with every other op. |

Outputs: one port per rule, named by its 1-based position (`1`, `2`, ...), plus `otherwise` when set. Every edge leaving a `switch` names one.

- Rules are tried from the first. Without `all`, the first match is the only port taken. With `all`, every matching rule's port is taken, in rule order.
- `between` includes both bounds: `value <= x <= to`. It needs numbers, and `to` must be at least `value`.
- The operators compare as in an `if`: `eq` and `ne` are typed, the ordering ops and `between` need numbers.
- An unknown value (unknown or unavailable entity, or an `average` with a non-numeric reading) matches nothing except an `ne` rule, so `otherwise` is taken when no `ne` rule matches. A known value that is not a number, tested with an ordering op or `between`, fails the node.
- When nothing matches and `otherwise` is off, no port is taken and the branch ends there.
- The step detail shows the value and the ports taken, for example `20 → 2`, `unknown → otherwise`, `20 → none`, or `average 30m = 18.6 → 1`.

The ports follow the rule positions: when you add or remove a rule in the middle, the later rules change port number, and the edges leaving them have to follow. The web editor renumbers them for you.

Example: three bands of temperature, with a fallback.

```json
{ "id": "band", "type": "switch", "params": {
    "entity": "sensor_temperature",
    "rules": [
      { "op": "lt", "value": 18 },
      { "op": "between", "value": 18, "to": 22 },
      { "op": "gt", "value": 22 }
    ],
    "otherwise": true } }
```

Edges leave it with `"port": "1"`, `"2"`, `"3"` or `"otherwise"`. A reading of 22 takes port `2`; a reading of 22.5 takes port `3`; an unknown reading takes `otherwise`.

A `switch` refuses to save when `rules` is empty or has more than 10 entries, a rule has a `value` that is not a bool, number or string, `to` is missing with `between` or present with another op, `between` is given non-numbers or `to` is less than `value`, or `average` is badly formed, over `24h`, or used with a non-numeric rule value.

## `math`

Computes one number from entity readings and from the results of earlier `math` cards, and can save it to a number entity. The result is kept for the rest of the run, so a later `command` (`value_from`), `if` or `switch` (`result`) or `notify` text (`{name}`) can use it.

| Param | Type | Description |
| --- | --- | --- |
| `inputs` | list, 0 to 10 | Each `{name, entity, average?}`. `name` is how the formula refers to the entity's value: `[a-z][a-z0-9_]{0,31}`, unique within the card. `average` (optional) is a window such as `15m` or `1h`, at most `24h`, that reads the mean of the entity's readings instead of its current value, on the same terms as an `if`'s `average`. |
| `formula` | string, 1 to 500 characters | The expression, see the grammar below. |
| `name` | string | The result's name, used by later cards. The same form as an input name, and not a function name or one of `and`, `or`, `not`, `trigger`. |
| `save_to` | string, optional | A `number` entity (a device, a `virtual_*` or a `group_*` number) the result is written to with `set_number`. |

A formula can read:

- its own inputs, by name;
- the result of any **other** `math` card of the same flow, by that card's `name`;
- `trigger`: the run's trigger value, when it is a number (a bool reads as 1 or 0). A run with no number in its trigger (a schedule, a sun event, a Manual card without a value) fails the step with `trigger has no number`.

An input reads a number as itself and an on/off value as 1 or 0. An unknown or unavailable entity, a text value, or an `average` with no numeric reading fails the step (`grid is unknown`, or `grid is not a number` for text).

### Formula grammar

- Numbers (`12`, `0.5`, `-3`), names and parentheses.
- Arithmetic `+ - * / %` and unary `-`, with the usual precedence, left-associative.
- Comparisons `< <= > >= == !=` give 1 or 0. `and`, `or` and `not` treat a non-zero value as true and give 1 or 0.
- Whitespace is optional: names are plain identifiers, so `heater-100-grid` is a subtraction. Write spaces for readability.

| Function | Result |
| --- | --- |
| `min(a, b, …)`, `max(a, b, …)` | The least or the greatest argument. One or more arguments. |
| `abs(x)`, `floor(x)`, `ceil(x)` | The absolute value, the value rounded down, the value rounded up. |
| `round(x)`, `round(x, n)` | Rounds to `n` decimals, 0 to 6 (0 when absent), halves away from zero. |
| `clamp(x, low, high)` | `x` kept between `low` and `high`. Refused on save when both bounds are numbers and `low` is above `high`. |
| `if(condition, then, else)` | `then` when the condition is non-zero, else `else`. Only the taken branch is evaluated, so `if(b != 0, a / b, 0)` never divides by zero. |
| `interp(x, [x0, x1, …], [y0, y1, …])` | Linear interpolation in a lookup table: below `x0` gives `y0`, above the last `x` gives the last `y`. The two lists are literals of the same length, 2 to 50 points, with `x` strictly increasing. |

A formula is at most 500 characters, nested at most 32 levels deep, and a card has at most 10 inputs.

### Saving and refusing

A flow is refused on save when:

- a `name` or an input `name` is not a valid name, or an input name is used twice;
- an `average` is badly formed or over `24h`;
- the formula does not parse, uses a function or a list it does not accept, or names something that is neither an input, `trigger` nor another math card's `name` (`unknown name x`); a card cannot read its own `name`;
- an `interp` table is malformed, or a `clamp` has literal bounds with `low` above `high`;
- an input `entity` or `save_to` is not in the house (`unknown entity`), or `save_to` is not a number entity;
- the card's `name` is also an entity id of the house, so a `{name}` placeholder is never ambiguous.

### Behaviour in a run

- The step runs its inputs, evaluates the formula and keeps the result under `name`. A later card of the same run reads it. Results do not outlive the run.
- A card that runs again in the same run (in a loop path) overwrites its earlier result. Two cards may share a `name` when they sit on different branches and compute the same result. A card after a fork sees the results of sibling branches that ran before it, in edge order, so compute a value on the path that needs it.
- A step fails, and the run ends as `failed`, on: division or `%` by zero, a result that is not a finite number, a `clamp` whose bounds have crossed at run time, `round` with `n` outside 0 to 6, an input the formula reads that is unknown or not a number (an input in the untaken branch of `if`, or one the formula never names, does not fail the step), a `trigger` with no number, or a name whose math card did not run on this path (`no result desired in this run`).
- With `save_to`, the card then sends `set_number` to that entity. The value is rounded to the entity's `step` and kept inside its `min` and `max` when the entity announces them; the step detail adds `(kept in range)` when it was clamped. The step succeeds only when the command is accepted, like a `command` card, and fails with the entity's refusal reason otherwise. The result kept for the run is the exact computed value, before rounding.
- Saving to a virtual number and reading it back as an input on the next run is how a flow remembers a value (for example, to smooth a target from one run to the next).

The step detail shows the inputs read, the result and the save, for example `grid=-412, heater=820 → desired = 1132 → virtual_heater_target = 1132`.

```json
{ "id": "target", "type": "math", "params": {
    "inputs": [
      { "name": "grid", "entity": "sensor_grid_power" },
      { "name": "heater", "entity": "sensor_heater_power", "average": "1m" }
    ],
    "formula": "clamp(heater - 100 - grid, 0, 1650)",
    "name": "desired",
    "save_to": "virtual_heater_target" } }
```

## `time.between`

Branches on the time of day: in or out of a daily range.

| Param | Type | Description |
| --- | --- | --- |
| `from` | string | Start of the range, `HH:MM`, 24-hour (`00:00` to `23:59`). Included. |
| `to` | string | End of the range, same format. Excluded. |

Outputs: two ports, `in` and `out`. Every edge leaving a `time.between` names one.

- The range is `from <= time < to`. A `to` earlier than `from` crosses midnight: `22:00` to `06:00` is in from 22:00 up to 05:59.
- `from` and `to` must differ. Times need two digits each (`07:00`, not `7:00`).
- It reads the clock of the house zone (the home location's time zone, the system zone when none is set), by wall-clock time: on a day the clocks change, `07:00` still means 07:00 on the wall.
- It needs no entity and no trigger value, so it works after any trigger, schedule and sun triggers included.
- The step detail shows the time read and the port, for example `22:40 → out`.

```json
{ "id": "daytime", "type": "time.between", "params": { "from": "07:00", "to": "22:30" } }
```

A `time.between` refuses to save when `from` or `to` is not a `HH:MM` time (the reason reads ``from: `7:00` is not a HH:MM time``), or when `from` and `to` are equal (`from and to must differ`).

## `delay`

Pauses the run. A run that is cancelled (by `restart` mode, by disabling the flow, or by a hub restart) never reaches the node after it.

| Param | Type | Description |
| --- | --- | --- |
| `for` | string | A duration: whole seconds, minutes and hours, alone or combined, without spaces (`90s`, `5m`, `1h30m`). Every number needs a unit (`5` is refused) and the total must not be zero. |
| `up_to` | string, optional | Same format. When set, each run waits a random whole number of seconds between `for` and `up_to`, both included. It must not be shorter than `for`; equal to `for` is a fixed wait. |

Outputs: one. The step detail records the wait (`waited 5m`, or `waited 7m12s (5m to 10m)` with `up_to`).

```json
{ "id": "wait", "type": "delay", "params": { "for": "5m", "up_to": "10m" } }
```

## `command`

Sends a command to an entity.

| Param | Type | Description |
| --- | --- | --- |
| `entity` | string | The entity id. |
| `command` | object | The same JSON as the API's `Command`, for example `{"type": "turn_on", "brightness": 200}`. See [MCP tools](mcp-tools.md#commands). |
| `value_from` | string, optional | The `name` of a [`math`](#math) card. Its result replaces the command's number: `set_number`'s `value`, `set_temperature`'s `temperature`, `set_position`'s `position` (rounded, kept between 0 and 100) or `turn_on`'s `brightness` (rounded, kept between 0 and 255). Refused on save with any other command type, and with a name that is no math card of the flow. |

With `value_from`, the number in `command` stays in the JSON (the web editor shows it when the option is switched off) and is ignored. A result that is missing from the run fails the node (`no result x in this run`). On a `set_number` command the number is fitted to the target entity like a math card's `save_to`: rounded to its `step` and kept inside its `min` and `max`, and the step detail reads `accepted (kept in range)` when it was clamped.

Outputs: one. The command goes through the hub's command path, confirmed by the entity's state echo within the `commands.confirm_timeout_ms` setting (default 5000), exactly like `send_command`. A button press, or a control with no state to echo, succeeds once it is published. No echo, a rejection or an offline entity fails the node.

On save the hub refuses a command the entity cannot take. It also refuses `arm` and `disarm` with a `code`: flows are retained on the broker in clear, so alarm codes are not allowed. Arm and disarm from a flow only on a panel that needs none.

## `notify`

Sends a phone push notification.

| Param | Type | Description |
| --- | --- | --- |
| `title` | string | Required, not empty. Takes `{entity_id}` placeholders, see [Values in a notification](../users/flows.md#values-in-a-notification). |
| `body` | string, optional | The alert body. Takes placeholders like `title`. |
| `channel` | string, optional | `alarm`, `security`, `camera`, `energy`, `climate`, `system` or `misc` (default). |
| `severity` | string, optional | `info` (default), `warning` or `critical`. |
| `entity_id` | string, optional | The entity the alert is about. |
| `image_url` | string, optional | An image to show with the alert. |
| `silent` | bool, optional | Deliver without sound. |
| `id` | string, optional | A fixed alert id. Absent: `flow-<flow id>-<run id>`, one alert per run. A fixed `id` makes each run replace the previous alert. |
| `lang` | string, optional | The language placeholder values are written in: `en` (default) or `fr`. The web editor sets it on save. |

Outputs: one. The step fails if the push service rejects the alert.

A placeholder named after a [`math`](#math) card (`{name}`, or `{name:2}` to round) prints that card's result from this run; a result the run did not compute prints `?`. A math result wins over an entity of the same name, and the hub refuses a math `name` that is an entity id, so the two never clash.

## `speak`

Says a sentence out loud on the speaker satellites (see [Sound](../users/sound.md)).

| Param | Type | Description |
| --- | --- | --- |
| `text` | string | Required, not empty. Takes `{entity_id}` placeholders like a `notify` title; values are written in French when the house speech language is French, in English otherwise. |
| `room` | string, optional | The room whose satellites speak: an area id, or a room name that the hub turns into its id when the flow is saved. Absent: every room. An unknown room is refused at save. |

Only satellites whose Allowed and Connected are both on speak; a muted one counts and stays silent. The sentence plays over the music, which is turned down while it plays.

Outputs: one. The step detail reads `spoke on <n> satellite(s): <text>`. The step fails, with the reason, when speech is not set up on the hub, when no satellite can speak in the room, or when the broker is offline.

## `flow.enable`

Enables or disables a flow.

| Param | Type | Description |
| --- | --- | --- |
| `flow` | string, optional | The target flow id. Absent: the flow the node belongs to. |
| `enabled` | bool | `true` enables, `false` disables. |

Outputs: one. A flow that disables itself still finishes the current run. A target must be a valid flow id and, when it is another flow, an existing one.

## `flow.label`

Switches a label, like the label's buttons on the Flows page.

| Param | Type | Description |
| --- | --- | --- |
| `label` | string | The label id. |
| `action` | `pick` (default), `on` or `off` | `pick` turns on the flows carrying the label and turns off the flows carrying the other labels of its choice, then makes it the choice's current label; a flow carrying both is turned on. Only for a label in a choice of at least two labels. `on` / `off` turn every flow carrying the label on or off. |

Outputs: one. Only flows not already in the target state are written; invalid flows are skipped and listed in the step detail. The step detail reads `picked ete: 2 on, 1 off`, `on vacances: 3 on`, `off vacances: 3 off` or `picked ete: nothing to change`, with `, N skipped (Name: reason, …)` when some flows could not be switched. A step that turns its own flow off still finishes the current run.

On save, an empty `label` is refused ("node `x`: choose a label"), and so is an unknown label ("unknown label `x` in node `n`") or a `pick` of a label in no choice ("label `x` is no longer in a choice in node `n`"). A node left as it was stored passes, so a flow whose label was deleted can still be saved to fix another node; at run time the step then fails with the same reason, and a `pick` in a choice with fewer than two labels fails too.

## `backup.run`

Backs the hub up to the FTP server set in **Settings → Backup** and waits until the backup ends. See [Backup and restore](../users/backup.md).

| Param | Type | Description |
| --- | --- | --- |
| `destination` | `ftp` (default) | Where the backup goes. `ftp` is the only value. |
| `timeout_minutes` | integer, 1 to 1440 (default 120) | The time limit. When the step starts the backup, it is the backup's own limit. When the step joins a backup that is already running, it is how long the step waits. |

Outputs: two, `ok` and `failed`. Both record the step as `Ok` and the run goes on along the matching port, so a failed backup does not stop the flow. Wire both to the step that shuts the NAS down.

The step detail on `ok` reads `Backed up <size> to <host> as <file>`. On `failed` it is the reason: `FTP not configured`, `no backup password set`, `can't reach the FTP server (…)`, `login refused`, `FTP folder: …`, `time limit reached`, `cancelled`, or the database or file error that stopped it. A step that joins a running backup that ends badly takes `failed` with that backup's reason.

When `timeout_minutes` passes while the step waits on a backup that was already running, the step takes `failed` with `time limit reached` and the backup keeps running. A run cancelled while it waits (the flow disabled, or restarted in restart mode) does not cancel the backup.

On save, a `timeout_minutes` outside 1 to 1440 is refused. A flow is saved even when FTP or the backup password is not set yet: the step fails at run time instead.

## Flow labels

The catalog of labels and choices is one retained document on `nidoo/flow_labels` (the `[topics] flow_labels` key, see the [Settings reference](configuration.md)), written by the hub only:

```json
{
  "labels": [
    {"id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison"},
    {"id": "ete", "name": "Été", "color": "amber", "choice": "saison"},
    {"id": "vacances", "name": "Vacances", "color": "green"}
  ],
  "choices": [{"id": "saison", "name": "Saison", "current": "hiver"}]
}
```

- A label has an `id` (made from its name on creation, `[a-z0-9_-]`, never changed by a rename), a `name`, a `color` (`blue`, `amber`, `green`, `red`, `purple`, `teal`, `pink` or `slate`) and an optional `choice` (a choice id).
- A choice groups labels of which one is current: `current` is the label picked last, absent until the first pick.
- Unknown keys are kept. Edit the catalog with `POST /api/v1/flow-labels` ([HTTP API](http-api.md#flows)) or the `edit_flow_labels` MCP tool; deleting a label removes it from every flow carrying it, deleting a choice keeps its labels as standalone labels.
