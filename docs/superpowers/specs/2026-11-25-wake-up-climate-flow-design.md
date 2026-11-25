# Heating that follows the wake-up alarm

Author: Gekkotron
Date: 2026-11-25
Status: approved in conversation (approach B), awaiting builder check

## Why

The user sets a wake-up alarm (say 7:00) and wants the house to follow it: the living room warm
when they get up, the other rooms in eco. It must stay generic: no room is hard-coded, and the
same building blocks must work for any morning routine.

Today nothing links the two. A wake-up alarm is a generated flow (`wake-up-<n>`,
`crates/nidoo-core/src/flow/wake_up.rs`) holding a cron trigger, speaker steps and a playlist.
Its nodes are rebuilt from `extra["wake_up"]` every time the alarm page saves, so steps added by
hand are lost. The climate engine has a weekly schedule per zone plus two temporary layers, a
zone hold (`Override`, origin `User`) and a zone eco (`RoomEco`), but no flow step can set
them. A `command` to a thermostat is read as a hand change and undone by the next schedule
change.

The user's words: "I like temporary hold and eco to be controlled by flow", then picked
approach B (a trigger that fires before a wake-up alarm).

## Goals

- `trigger.wake_up`: fires `before_minutes` before a wake-up alarm rings. It covers every
  enabled alarm, or only the alarms listed. It follows the alarms: moving an alarm, changing its
  days, or disabling it moves or stops the trigger with no edit to the heating flow.
- Opt-in catch-up on `trigger.schedule`, `trigger.sun` and `trigger.wake_up`: when the hub
  starts, or the broker link comes back, a trigger whose time was missed within its window
  fires once.
- `climate.hold` step: holds one or more zones at a temperature (or `off`) for a duration. This
  is the same hold the climate page and the assistant set.
- `climate.eco` step: puts one or more zones in eco for a duration, the same eco the climate
  page sets.
- Web flow canvas: the new trigger and the two steps in the node library, with friendly editors
  (alarm picker, zone picker, duration). Text for all three goes in the flow description
  (`flow_text.rs`).
- Docs: `docs/reference/flow-nodes.md`, plus the MCP `set_flow` schema and instructions, so the
  assistant and Claude can write these flows.

Example the user wants to build in the editor, called "Morning heating":

```
[before a wake-up alarm · 45 min] ─┬─> [climate.hold · living room · 21 °C · 3 h]
                                   └─> [climate.eco  · bedroom, office · until 18:00]
```

## Non-goals

- Rewriting the weekly climate schedule from alarms (approach A was the other option and is
  not wanted).
- A heating section on the wake-up alarm page. The alarm page and its generator do not change.
- Learning the pre-heat lead from how fast a room warms. The lead is the user's
  `before_minutes`.
- Steps that end a hold or eco early (`climate.clear`). That can come later.
- Per-device holds (`Override.entity`) from flows. Holds are whole-zone only.
- Catch-up by default. Every time trigger keeps today's behaviour (a missed time is not
  replayed) unless its catch-up option is turned on (see "Catch-up if missed").
- Replaying more than the latest missed time, or catching up after a flow edit (only hub
  start and the broker link coming back count).
- A "skip the next alarm" feature. No such feature exists today.

## Anti-goals

- The alarm failing to ring because the heating failed. The heating lives in a separate flow,
  so a climate error ends only that flow's branch.
- A hold or eco that never ends. Every step needs a duration, with no "forever" option.
- A heating flow that silently does nothing. Unknown alarm ids or zone ids, or no alarm at all,
  give a save-time warning. A zone that refuses (not active, off, or climate disabled) makes
  the step fail with the hub's reason in the run log.
- A trigger firing on reload because the computed time "just passed". Reload arms only the
  next future fire. Only the opt-in catch-up below may fire a past time, and only at hub start
  or when the broker link comes back.
- A catch-up that replays a stale morning, such as heating at 22:00 for a 7:00 alarm. The
  window bounds it.
- A catch-up firing twice for the same time, for example after a second restart. A marker
  that survives a restart prevents it.

## Approaches considered

- **A. Heating section on the alarm page**, generated into the alarm flow. Rejected: the alarm
  page grows a climate form, and another routine would need generator changes again.
- **B. `trigger.wake_up` plus generic climate steps** (picked). Most generic. The heating
  follows the alarm with no extra edit, and the trigger can later drive lights or shutters.
  Cost: one more timer kind in the flow engine.
- **C. Climate steps only**, with the user's own cron. Rejected: the heating drifts as soon as
  the alarm moves.

## Design

### `trigger.wake_up` (nidoo-core model + nidoo engine)

Params:

```json
{ "type": "trigger.wake_up",
  "params": { "before_minutes": 45, "alarms": ["wake-up-1"] } }
```

- `before_minutes`: integer `0..=240`. 0 fires at the ring itself.
- `alarms`: optional list of alarm flow ids. Absent or empty means every alarm.
- An "alarm" is a flow that `wake_up::is_managed` accepts, whose `read` parses, and that is
  **enabled**. The engine's `self.flows` already holds only enabled flows, so a disabled
  alarm drops out on its own.
- Next fire after `now`: for each matching alarm, take its schedule from `wake_up::cron(&w)`
  (the same expression the alarm's own trigger uses) and compute
  `schedule.next_after(now + before) - before`. Take the minimum over all alarms. Subtracting
  in absolute time handles a pre-alarm time that falls on the previous day. A DST jump is
  covered as well, because the instant is exactly `before` ahead of the ring.
- If two matched alarms give the same instant, the trigger fires once.
- Optional `catch_up_minutes`: see "Catch-up if missed".
- Arming follows `arm_suns`. On every `reload`, the engine rebuilds the wanted set, keyed
  `flow/node`. Each entry holds the trigger's params plus a fingerprint of the matched alarms'
  crons. An armed entry whose fingerprint changed is re-armed from now, so editing an alarm
  moves the trigger. Editing an alarm saves a flow, and saving a flow already triggers a
  reload. Fire and re-arm work like `tick_suns`: no catch-up, and a fire while the broker link
  is offline counts as a skipped run.
- The trigger carries no value, like `trigger.schedule`. Validation rules that apply to
  schedule and sun triggers ("an `if` reachable from it must name an entity") apply here too.
- Save-time warnings, not errors: an id in `alarms` that is not a wake-up flow, and no
  wake-up alarm existing at all.

### Catch-up if missed (`trigger.schedule`, `trigger.sun`, `trigger.wake_up`)

The user asked that the hub check, when it starts, which flows should have run. This is an
opt-in option on every time trigger.

- New optional param on all three triggers: `catch_up_minutes`, an integer `0..=720`. Absent
  or 0 means no catch-up, which is today's behaviour, so existing flows are unchanged.
- **Marker.** Each time such a trigger fires, the hub records the instant it was *due*. This
  covers a normal fire and a catch-up fire. The record is a small retained broker document
  per `flow/node`, kept next to the flows' own retained documents (the builder picks the
  topic). It survives a restart, unlike the in-memory run list. Removing a flow, or a node,
  removes its markers.
- **When it checks:**
  - once at hub start, after the retained documents (flows, markers, wake-up alarms) have
    been loaded;
  - each time the broker link comes back online. That also covers today's "skipped: the
    broker link is offline".
  - It never checks on a flow reload or an edit.
- **What it checks, for each enabled flow with a trigger where `catch_up_minutes > 0`:**
  1. Find the *latest* instant `d` at which this trigger should have fired with
     `now - catch_up_minutes <= d <= now`. That is the cron tick, the sun event plus its
     offset, or the alarm ring minus `before_minutes`.
  2. If there is one and the marker is older than `d`, fire the flow once now and write `d`
     into the marker.
  3. Earlier missed instants are never replayed: a trigger fires at most once per check.
- **No marker yet** (catch-up just turned on, or a new flow): treat it as caught up. Write
  the latest `d`, or now, without firing. A first-ever fire missed before any marker existed
  is not replayed.
- A catch-up run goes through the normal run path (`single` and `restart` modes apply). The
  run log names it, for example `caught up: due 06:15`.
- `trigger.wake_up` reads the alarms at the time of the check. An alarm disabled since the
  miss is no longer caught up.
- In the editor, each of the three triggers gets a "Catch up if missed, within N minutes"
  toggle and field. A new "Before a wake-up alarm" node starts with it on at 60 min. A new
  schedule or sun node starts with it off.

### `climate.hold` step

```json
{ "type": "climate.hold",
  "params": { "zones": ["living"], "setpoint": 21.0, "minutes": 180 } }
```

- The duration counts from when the step runs, not from the ring. With `before_minutes` 45
  and `minutes` 180, the hold ends 2 h 15 after the alarm.
- `zones`: one or more climate zone ids (non-empty).
- `setpoint`: a number in °C, or `"off"`. Range-checked with `check_setpoint` at save time.
- Duration: exactly one of `minutes` (> 0) or `until` (`"HH:MM"`: the next time the house
  clock reaches it, strictly after now).
- Optional `mode` (`heat` or `cool`), as in `OverrideRequest`.
- The step goes through the same hub call as the climate page's hold
  (`Hub::set_climate_override`). That means the same checks (zone active, range) and the same
  side effect: a whole-zone temperature hold ends that room's eco.

### `climate.eco` step

```json
{ "type": "climate.eco",
  "params": { "zones": ["bedroom", "office"], "until": "18:00" } }
```

- `zones` and the duration (`minutes` or `until`) work exactly as for `climate.hold`.
- The step goes through `Hub::set_climate_eco` and replaces any eco the zone had.

### Running a climate step with several zones

- The step applies every zone in list order. Zones that succeed stay applied even if a later
  zone fails.
- The step succeeds if every zone succeeded. Otherwise it fails, and its detail names each
  failed zone with the hub's reason, for example `bedroom: zone is off`. Its successors then do
  not run (the normal branch rule).
- The detail on success is short, for example `living 21 °C until 10:00`.

### Web canvas

- Node library: "Before a wake-up alarm" in the triggers group; "Heating hold" and "Room eco"
  in the steps group.
- Trigger editor:
  - a minutes field;
  - an alarm multi-select listing the existing wake-up flows by name, with "all alarms" as the
    empty choice;
  - a read-only line showing the next fire time.
- Step editors:
  - a zone multi-select fed by the climate zones view (names, not ids);
  - a setpoint field (hold only);
  - a duration choice: "for N minutes" or "until HH:MM".
- Every new string goes through `src/i18n` (see the bare-text test).

### Docs and MCP

- `flow-nodes.md`: a section per new node, plus the "Morning heating" example.
- The MCP `set_flow` schema (schemars) and its tool description name the new types.

## Left to the builder

- Exact Rust names, the module split (for example a `wake_up_trigger.rs` beside `sun.rs` in
  `crates/nidoo/src/flows/`), and where the fingerprint lives.
- Exact UI wording, icons, and how the next-fire line is fetched.
- Test layout. Must cover:
  - pre-alarm across midnight (alarm 00:15, 45 min before);
  - a DST day;
  - two alarms at the same time firing once;
  - an alarm edited and disabled while armed;
  - a multi-zone step with one zone off;
  - `until` already past for today, which rolls to tomorrow.
  - Catch-up:
    - a restart inside the window fires once;
    - a restart outside the window does not fire;
    - a second restart does not fire again (marker);
    - several missed ticks fire once;
    - the broker link coming back catches up a skipped tick;
    - no marker seeds without firing;
    - an edit never catches up;
    - `catch_up_minutes` absent keeps today's behaviour, across all three trigger types.
