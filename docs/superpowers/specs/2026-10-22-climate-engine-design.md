# Climate engine v1

Status: design, 2026-10-22. Branch `sc-critical-meissner-823c`. Origin: `Athena-Hestia/docs/VISION.md` (a Python prototype, used here as prior art only).

## 1. Intent

The hub learns how each room is heated or cooled, proposes plain rules from that history, and once the user approves them, keeps every room on those rules. It never lets a room freeze or overheat because a sensor went quiet or a command got lost, and it can always say *why* a room is at the temperature it is.

It is an optional feature: `[climate] enabled = false` by default, switched on in `oikos.toml`, like `[groups] enabled`.

The project is generic ([feedback: project must be generic]): no room names, topic prefixes, vendor payload keys or house-specific rules in code. The user's house is the test bed, not the limit.

## 2. Goals, non-goals, anti-goals

**Goals**
- Every area that holds at least one `climate` entity becomes a **zone**. The engine keeps each managed zone on its rules, for heating and cooling.
- **Rules are learned from history, not written by hand.** On first enable, and every night after, the engine reads the hub's TimescaleDB history and drafts each zone's schedule, with the evidence behind every block ("20.5 °C weekdays 07:00–22:30: you had it there on 41 of 60 mornings").
- **The user approves every rule and every change.** A proposal waits until approved; the current rules keep running meanwhile. The user is told through the hub's push (`oikos-push`) and approves from the notification or from the Climate web screen, where proposals can also be edited.
- **Safety never waits for approval and nothing loosens it.** Frost protection, overheat protection and the stale-sensor fallback act on every zone that is not switched off, from the moment the feature is enabled, above manual overrides.
- **Hand changes are respected.** A temperature set by hand (the device itself, the app, the web, MCP or a flow) holds until the zone's next scheduled change, at most 4 hours by default.
- **House presence.** When nobody is home, zones drop to their away setpoint; a holiday switch drops them to the frost setpoint.
- **Explainable.** Every decision carries a `source` and a reason, shown on the web screen and over MCP.

**Non-goals (this design)**
- Shutters as a climate lever (design 2).
- The comfort ↔ cost slider and energy prices (design 3).
- Learned layers / ML: occupancy prediction, preheat timing (design 4, only if a replay over history shows they beat the rules).
- Heaters driven through a plain `switch` (smart plug, pilot wire) where the hub would run the thermostat loop itself. v1 drives only `climate` entities, which regulate themselves. (lifted for heating by `2026-11-26-climate-onoff-heaters-design.md`)
- `water_heater` entities.
- Per-room occupancy in decisions. Room motion and occupancy are only evidence for the learner.
- A flow node that sets an override with its own expiry. Flows that command a climate entity are treated as hand changes (§7).
- Climate screens in the Android app. The app only needs to show the proposal notification with its buttons (§10).
- Any coexistence with the Python prototype (no shadow comparison, no cut-over). The user chose "only Rust".

**Anti-goals (a failure even if it works)**
- A colder or less safe house: a zone held at a comfort setpoint on a reading that went stale, or a frost condition the engine did not act on.
- A second config language: no YAML, no zone files. Only the on/off switch and the topic prefix live in `oikos.toml`; everything else is retained JSON edited through MCP and the web.
- The engine fighting the user: undoing a hand change minutes later, or re-sending a command that keeps failing.
- Rules the user cannot read or check: every learned block must show its evidence.
- Notification spam: one pending proposal per zone, at most one proposal push per zone per day.
- The learner learning its own schedule back (the engine's own commands are not evidence, §6.2).
- Setup-specific code, French strings or vendor payload keys.

## 3. Decided with the user

- Name: **`climate`** everywhere (crate `oikos-climate`, config `[climate]`, docs and UI "Climate"). No "Hestia" in code or docs.
- `[climate] enabled` defaults to `false`.
- Rust only; the Python prototype is prior art.
- Rules from history, not ML. The user approves the first rules **and every change**, notified through `oikos-push`.
- This design is engine v1; shutters, pricing and ML come after, in that order, each in its own design.

## 4. Configuration

### 4.1 `oikos.toml`

```toml
# The climate engine: learns each room's heating/cooling rules from history, asks before
# applying them, and keeps managed rooms on them. Everything except this switch is edited
# through MCP and the web screen (retained JSON under `prefix`).
[climate]
enabled = false
prefix = "oikos/climate"
```

`ClimateConfig { enabled: bool, prefix: String }` with `#[serde(default)]`, like `GroupsConfig` (`crates/oikos-os/src/config.rs:452`). `validate()` refuses an empty `prefix` when enabled. `prefix` joins `STRING_ENV_KEYS`. With `enabled = false` the hub subscribes to nothing under the prefix, runs no climate task, and the climate HTTP routes and MCP tools answer "climate disabled" (HTTP 503), the same way `require_groups()` gates groups.

### 4.2 Retained documents

All under `<prefix>` (default `oikos/climate`), subscribed through `Hub::extra_filters()` only when enabled. Writes follow the hub's pattern: publish retained, then apply locally; an empty payload deletes. Invalid payloads are kept with their reason, like `FlowStore`, so the web can show and fix them.

| Topic | Who writes | Content |
|---|---|---|
| `<prefix>/house` | user (MCP, web) | house-wide settings (§4.3) |
| `<prefix>/zone/<area_id>/settings` | user; engine on approval | zone settings and approved rules (§4.4) |
| `<prefix>/zone/<area_id>/proposal` | engine | the pending proposal, if any (§6.3) |
| `<prefix>/zone/<area_id>/override` | engine, user | active overrides with their expiry (§7) |
| `<prefix>/zone/<area_id>/decision` | engine | the latest decision (§5.3) |

A zone id is its area id. A zone with no settings document yet is shown with defaults and state `pending`.

**No climate document ends in `/config`.** The default prefix sits under the discovery prefix `oikos`, and `classify()` (`crates/oikos-core/src/discovery/topic.rs`) reads any `oikos/climate/<a>/<b>/config` as climate discovery; flexible groups really do publish climate discovery at `oikos/climate/group/<id>/config`. So the climate module subscribes only to `<prefix>/house` and `<prefix>/zone/+/+`, never `<prefix>/#`, and a test pins that none of its topics classify as discovery.

### 4.3 Settings (defaults in brackets)

- `season`: `auto` | `heat` | `cool` | `off` [`auto`]. `auto` reads `outdoor_entity` (§5.2); without it, `auto` means `heat`.
- `outdoor_entity`: a temperature sensor id [none].
- `heat_below` [16 °C], `cool_above` [24 °C]: thresholds on the outdoor 24 h mean for `auto`.
- `presence`: entity ids that say someone is home: `device_tracker` (`home`), `binary_sensor` with class `presence` / `occupancy`, or a `virtual_*` switch (on = home) [none = always home].
- `away_grace` [15 min]: how long nobody must be home before away applies.
- `holiday_entity`: a switch or `virtual_*` switch; on = holiday [none].
- `frost` [7 °C], `overheat` [30 °C], `overcool` [16 °C]: safety limits.
- `stale_after` [60 min]: a reading older than this is stale.
- `learn_days` [28], `learn_min_days` [10], `learn_support` [0.6]: learner window, minimum days with data, and the share of days that must agree with a block (§6).
- `learn_at` ["03:30"]: local time of the nightly learning run.

### 4.4 Zone config

- `state`: `off` | `pending` | `active`. `off`: the engine does nothing in the zone, not even safety (the user chose to keep it out). `pending`: no approved rules yet; safety acts, everything else is only shown ("would set 20.5 °C"). `active`: the engine runs the zone. Approving the first proposal turns `pending` into `active`.
- `entities`: the zone's climate entities [all `climate` entities in the area, except `group_*` entities, so a group and its members are not commanded twice]. A device that does not advertise the current mode in `hvac_modes` is left alone for that mode (a cool-only air conditioner is never touched in winter).
- `sensor`: a temperature sensor that represents the room better than the devices' own reading [none: a thermometer of the zone's area, else the climate entities' `current`].
- `away_setpoint` per mode [heat 16 °C, cool 28 °C].
- `manual_hold_max` [4 h].
- `rules`: per mode (`heat`, `cool`), a `base` setpoint and a list of blocks `{ days: [mon…sun], from: "HH:MM", to: "HH:MM", setpoint }`. Outside every block the zone sits at `base`. `base` or a block's `setpoint` may be `"off"` (device mode `off`), for houses that switch heating off at night instead of lowering it. Each block keeps the evidence it was approved with.
- `approved_at`, `approved_proposal`.

Editing a zone's rules directly (web, MCP) counts as approving them.

## 5. The decision

### 5.1 Priority tree

Evaluated per zone, first match wins:

1. **Safety** (`source: safety`), even when the zone is `pending`:
   - room reading below `frost` → heat to `frost + 2 °C` if the device can heat;
   - in heat mode, reading above `overheat` → `base` of the heat rules, or the device's minimum if that is still above `overheat`;
   - in cool mode, reading below `overcool` → stop cooling (mode `off` if advertised, else the device's maximum setpoint);
   - reading stale (§5.4) → never keep a block setpoint on it: fall back to the lower of the current target and `base` in heat (the higher in cool), never below `frost` in heat (a `base` of `off` falls back to `frost`), and notify. In a `pending` zone there is no `base`: notify only.
   - **When a safety condition clears in a `pending` zone**, the engine puts back the target and mode it found on the device before it acted. In an `active` zone the tree simply moves on to the next row.
2. **Override** (`source: override`): a hand change or an explicit override, until its expiry (§7).
3. **Holiday** (`source: holiday`): `frost` setpoint in heat, `off` (or max setpoint) in cool.
4. **Away** (`source: away`): `away_setpoint` of the mode.
5. **Schedule** (`source: schedule`): the block covering now, else `base`.
6. **Idle** (`source: idle`): season is `off`, or the zone is `pending`, or the zone has no rules for the current mode: nothing is commanded beyond safety.

Every setpoint is clamped to the device's `min_temp`/`max_temp` and rounded to `temp_step`, and modes are limited to the device's advertised `hvac_modes`. A zone whose devices cannot do the current mode (no `cool` in `hvac_modes`) is idle for that mode.

In an `active` zone the engine sets each device's mode to the zone's mode (`heat` or `cool`, or `off` for an `"off"` setpoint), whatever mode it found, including the device's own `auto`. Turning a device off by hand is a hand change like any other (§7); keeping a room out for good is the zone's `off` state.

### 5.2 Mode (season)

`heat`, `cool` or `off` comes from `settings.season`. In `auto`: heat when the outdoor 24 h mean is below `heat_below`, cool above `cool_above`, otherwise keep the previous mode (hysteresis; `heat` at start). The 24 h mean comes from history when available, else from the live readings seen since boot.

### 5.3 Decision record

```json
{ "zone": "living_room", "mode": "heat", "setpoint": 20.5, "source": "schedule",
  "reason": "Weekdays 07:00–22:30 block (approved 2026-10-23)",
  "reading": 19.8, "reading_age_s": 120, "outdoor_mean": 9.4, "home": true,
  "applied": true, "at": "2026-10-23T07:00:04+02:00" }
```

The record also carries `entities: { "<id>": { "setpoint", "mode" } }`, what the engine last commanded on each device; being retained, it rebuilds the command ledger after a hub restart (§7). `reason` is English text built from the inputs, never free-form from config. `applied` is `false` when the zone is `pending`/`off` or the command failed. The latest record is retained on `…/decision`; the hub keeps the last 200 per zone in memory for the web log, and when history is on, writes every *changed* decision to a `climate_decision` table (§8.2).

### 5.4 Readings and staleness

The room reading is `sensor` if set, else a thermometer of the zone's area (a primary `temperature` sensor that is not on one of the zone's climate devices and is not `outdoor_entity`; the first by id), else the mean of the zone's climate entities' `current`. The read view names the sensor it used in `room_sensor`. A reading is stale when no state for it has been seen (`subscribe_state_seen`, which includes unchanged echoes) for `stale_after`. The room sensor stays fresh up to the zone's `feedback.quiet_limit_min` when that is longer than `stale_after` (thermometers that only report changes are silent for hours in a steady room). If the room sensor is older than that but the devices' own readings are fresh, the engine uses those and says so in the reason; only when every source is stale does the stale-sensor rule apply.

### 5.5 When it runs and when it commands

- The engine evaluates every zone every 60 s and on any state change of an entity it reads (debounced, 2 s).
- It commands only when the decision differs from the device's current `target`/`mode` (by more than half a `temp_step`), and at most once per entity per 60 s.
- It goes through `Hub::send_command` (QoS 1, waits for the echo). On `accepted: false` it retries at the next evaluation; after 3 consecutive failures on one entity it stops retrying that entity for 15 min and sends one push (§9).
- **Re-assert:** a device that drifts from what the engine set is put back at the next evaluation only when the drift is not a hand change. Drift is a change seen within 2 minutes after the device came back from unavailable (`state/availability.rs`): a rebooted device falling back to its default. Every other unexplained change is a hand change (§7).

## 6. Learning rules from history

### 6.1 What it needs

Learning needs `[history] url` set. Without history the engine still runs safety, overrides, presence and any rules entered by hand, the learner is off, and the web screen says why.

History already stores every state change as the full `StateValue` in `entity_state.value` (jsonb), so the climate target and mode are there. A new query reads a climate entity's `(time, target, mode)` over a range (none exists yet; `Db::samples` returns numbers only).

### 6.2 What counts as evidence

The evidence is the zone's **effective target timeline** over `learn_days`, split by mode: per 15-minute slot, the mean target of the zone's devices that are on (a device held `off` contributes `off`; if every device is off, the slot is `off`).

Where the engine set the target (recorded in the `climate_decision` table with `applied = true`), the slot is not a new opinion from the user, but it is not missing data either: a slot the engine held and the user left alone **counts as agreement with the rule in force**, and a hand change in it counts as disagreement with its new value. So:

- before a zone is `active`, everything in history is evidence, whoever set it;
- once it is `active`, a day the user never touched agrees with the approved rules, and a block is only proposed to change when hand changes over enough days (`learn_support`) point the same way. One hand change can never reach the threshold alone;
- `learn_min_days` counts days with any target data, engine-held or not.

The learner therefore takes the approved rules as an input alongside the samples.

### 6.3 What it proposes

For each zone and mode with at least `learn_min_days` days of data, the learner produces a full rule set (`base` + blocks), deterministic for the same input. The approach:

- cut each day into 15-minute slots and take the target held for most of each slot;
- group days into weekdays and weekend days, or per day when the days in a group disagree;
- take the median per slot across the group's days, merge neighbouring slots within 0.5 °C into blocks, drop blocks shorter than 30 min;
- `base` = the most common low value (or `off` when off dominates); blocks are the stretches above it (in cool: below it);
- keep a block only if at least `learn_support` of the group's days agree with it (within 0.5 °C, start/end within 30 min); its evidence is "N of M days";
- clamp to device limits and step, and never below `frost`.

The builder may refine the algorithm as long as it stays deterministic, uses only the evidence above, and every block carries its "N of M days" evidence.

A **proposal**:

```json
{ "id": "p-20261023-0330-living_room", "zone": "living_room", "kind": "initial",
  "created": "2026-10-23T03:30:00+02:00", "window": { "from": "…", "to": "…", "days": 28 },
  "rules": { "heat": { "base": 17.0, "blocks": [ { "days": ["mon","tue","wed","thu","fri"],
     "from": "07:00", "to": "22:30", "setpoint": 20.5, "evidence": { "agree": 17, "of": 20 } } ] } },
  "changes": [ "New: weekdays 07:00–22:30 at 20.5 °C (17 of 20 weekdays)" ] }
```

`kind` is `initial` (the zone has no approved rules) or `change`. A `change` is proposed only when the nightly run differs from the approved rules by a block starting or ending ≥ 30 min apart, a setpoint ≥ 0.5 °C apart, or a block added or gone. `changes` lists those differences in plain English.

### 6.4 Lifecycle

- **On enable** and every night at `learn_at`: learn every zone that is not `off`. Zones without enough data get no proposal; the web shows "Learning: 6 of 10 days so far".
- **One pending proposal per zone.** A newer one replaces it; the user is notified again at most once a day per zone.
- **Approve** (push button, web, MCP): the proposal's rules (as edited, if edited on the web) become the zone's `rules`, `pending` becomes `active`, the proposal topic is cleared.
- **Reject**: the proposal is cleared and its fingerprint (the rules it would have set) is remembered for 7 days, so the same proposal is not re-sent.
- **Relearn now**: MCP and web can start a learning run for one zone.

## 7. Hand changes and overrides

- A **hand change** is a climate entity's target or mode changing to something the engine did not command (beyond half a `temp_step`), outside the 30 s window after the engine's own command, and not drift (§5.5). The source does not matter: the device's buttons, the web, the app, MCP `send_command`, or a flow `command` node all look the same once they echo (no command carries its origin today). The engine keeps its own command ledger to tell them apart.
- **Hand changes are per device.** Turning up one of two radiators in a room holds that radiator only; the zone's other devices stay on the rules. (The user changed one device; copying it to the others would be the engine guessing.)
- A hand change becomes an override on that device: its target/mode holds **until the zone's next scheduled change**, but at most `manual_hold_max` (4 h default). In a `pending` zone nothing is overridden, since nothing is commanded.
- **After a hub restart** the ledger comes back from the retained decision (§5.3). A device whose target differs from what the engine last commanded changed while the hub was down: it is a hand change, held by the same rule, measured from boot.
- An **explicit override** (web, MCP, HTTP) sets a setpoint and/or mode for the whole zone (every device) with a duration or an end time, and can be cleared.
- Overrides are retained on `…/override` as a list (`[{ entity | null, setpoint, mode, until, origin: "hand" | "user" }]`, `entity: null` for a whole-zone override), so they survive a hub restart.
- Safety still wins over an override.

## 8. How it is built (Claude's calls)

### 8.1 Crates

- **`crates/oikos-climate`** (new, pure, no I/O): the document types and their validation (settings, zone config, rules, proposal, override, decision), the priority tree (`decide(inputs, now) -> Decision`), mode selection, staleness, the hand-change classifier, the learner (`learn(evidence, settings, caps) -> Option<Proposal>`) and the proposal diff. It may depend on `oikos-core` for model types. Every priority row and every safety path has a unit test.
- **`crates/oikos-os/src/climate/`** (new hub module): the retained stores, the evaluation task (60 s tick + debounced state changes), the command ledger, the learner run (history queries + nightly timer in the house zone, `Hub::zone()`), push integration, HTTP routes and MCP tools. Held on `Hub` as `Option<…>` built from config, like the push/frigate/watchdog services. The task is spawned in `main.rs` and aborted on shutdown.

### 8.2 History additions

- A query for a climate entity's `(time, target, mode)` over a range, reading `entity_state.value`.
- A `climate_decision` table (`time, zone, entity_id, mode, setpoint, source, reason, applied`), written on changed decisions only, created by the same migration path as `entity_state`. It feeds the evidence filter (§6.2) and the web's decision log beyond the in-memory 200.

### 8.3 Push

- Producer source `climate`, channel `climate` (already a valid channel).
- **Proposal ready:** "Climate: new rules for Living room" with action buttons `approve` ("Approve") and `reject` ("Reject"); alert id `climate-proposal-<proposal id>`. Approving or rejecting anywhere else resolves the alert (`cancel: true`). Reviewing and editing happen on the web Climate screen; the body says so.
- **Safety:** frost or overheat acted on (severity `critical`), a stale reading (`warning`), repeated command failures (`warning`). One alert per zone and cause until it clears; clearing resolves it.
- **Response routing (new):** the hub subscribes to `<push prefix>/responses` and routes a response whose alert id starts with `climate-` to the climate module. `approve` approves the proposal as it stands; `reject` rejects it. A response for a proposal that is no longer pending is ignored.

### 8.4 HTTP and MCP

- HTTP under `/api/v1/climate`: `GET /` (enabled, settings, zones with state, reading, decision, override, pending proposal, learning status), `PUT /house`, `PUT /zones/{id}`, `POST /zones/{id}/override`, `DELETE /zones/{id}/override`, `POST /zones/{id}/proposal/approve` (optional edited rules in the body), `POST /zones/{id}/proposal/reject`, `POST /zones/{id}/learn`, `GET /zones/{id}/decisions`. All answer 503 when disabled.
- MCP: a `climate_tools.rs` router added to the `+` chain in `mcp/mod.rs`: `climate_status`, `get_climate_zone` (decision, why, proposal, recent decisions), `set_climate_settings`, `set_climate_zone`, `approve_climate_proposal`, `reject_climate_proposal`, `set_climate_override`, `clear_climate_override`, `learn_climate_zone`. Tools are always registered and answer "climate disabled" when off. The MCP instructions text gets a short climate paragraph.

### 8.5 Web

A **Climate** screen (`/climate`), in the nav only when `GET /api/v1/climate` says enabled:
- one card per zone: room reading and its age, target, mode, state (`off`/`pending`/`active`), the decision's source and reason, an active override with its end time and a clear button, the zone's state switch;
- a proposal panel per zone with a pending proposal: the blocks drawn on a week timeline, each with its evidence, the plain-English changes, Approve / Reject, and edit-before-approve (move a block's times, change a setpoint, delete a block);
- the zone's approved rules on the same timeline, editable;
- the decision log (latest first);
- a settings sheet for §4.3.

It follows the existing web style and components (`cards/climate.rs` for the room's controls). The look is left to the builder.

## 9. Notifications summary

| Event | Severity | Buttons | Resolves when |
|---|---|---|---|
| Proposal ready | info | Approve, Reject | approved, rejected or replaced |
| Frost / overheat acted on | critical | — | reading back inside limits |
| Reading stale | warning | — | reading fresh again |
| Command failing (3×) | warning | — | next accepted command |

Override expiry sends nothing (noise).

## 10. Android app

No app change is needed: the app already shows push action buttons and sends the chosen action back on `<prefix>/responses` (`AlertNotifier.kt`, `ReplyWorker`). That is why the buttons are Approve / Reject rather than a "Review" that would have to open the web screen from the app. No climate screens in the app in v1.

## 11. Constraints

- Generic code: no room names, house values or vendor keys. Defaults above are the only numbers in code.
- Safety logic in the pure crate, unit-tested for every row of §5.1 and every staleness case of §5.4, including "configured sensor stale, device readings fresh" and "everything stale".
- The engine must keep running when history is off or the database is unreachable (learner paused, decisions still made).
- QoS 1 for every publish; retained for every document in §4.2.
- No user-facing French strings.

## 12. Left to the builder

- The exact learner algorithm within §6.3's rules.
- Module layout inside `oikos-climate` and the hub module.
- The web layout and timeline drawing.
- Exact English wording of reasons and notifications.
- Whether `climate_decision` lives in the existing history migration or its own.

## 13. Claude's calls (for the user to check)

- Hand change hold: until the next scheduled change, max 4 h, per zone.
- Zone states `off` / `pending` / `active`; `off` stops even safety; safety acts in `pending` zones from the moment the feature is enabled.
- Only `climate` entities are driven in v1 (no plug-driven heaters, no water heaters). (lifted for heating by `2026-11-26-climate-onoff-heaters-design.md`)
- House-level presence only; room motion is learner evidence, not a decision input.
- `auto` season from the outdoor 24 h mean with 16 °C / 24 °C thresholds, `heat` without an outdoor sensor.
- Safety defaults: frost 7 °C, overheat 30 °C, overcool 16 °C, stale after 60 min.
- Learning: 28-day window, 10 days minimum, 60 % agreement, nightly at 03:30.
- Flows commanding a climate entity count as hand changes; no flow node for overrides in v1.
- Climate documents live at `<prefix>/house` and `<prefix>/zone/<id>/settings|proposal|override|decision`, never ending in `/config`, so they cannot be mistaken for discovery (found by the review).
- Hand changes are held per device; explicit overrides apply to the whole zone.
- In an active zone the engine sets device modes too (switching `auto` or `off` devices to heat/cool); rules may say `off`.
- Once a zone is active, days the user left alone count as agreeing with its rules; only repeated hand changes propose a change.
- After a hub restart, a target changed while the hub was down is a hand change; a change right after a device comes back online is drift and is put back.
- In a `pending` zone, safety restores what it found once the condition clears; a stale reading there only notifies.
- Push buttons are Approve / Reject (no app change needed); review and edit on the web.
