# Heating That Follows the Wake-Up Alarm: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build three flow pieces and one option:
- `trigger.wake_up`, which fires N minutes before a wake-up alarm;
- `climate.hold` and `climate.eco` flow steps;
- an opt-in "catch up if missed" option on the schedule, sun and wake-up triggers.

**Architecture:**
- Pure model, validation and time math live in `nidoo-core::flow` (wasm-clean, shared with the
  web).
- The engine in `crates/nidoo/src/flows/` arms wake-up timers the way it arms sun timers. It
  runs climate steps through the existing `Hub::set_climate_override` and
  `Hub::set_climate_eco`.
- Catch-up keeps a retained "last fired" marker per trigger node under the automation prefix.
  It checks once the broker link has been online for a settle delay.
- The web canvas gets cards and sheets for the new nodes.

**Tech Stack:** Rust 2024 workspace, tokio, chrono/chrono-tz, serde, schemars, Leptos (web,
wasm).

**Spec:** `docs/superpowers/specs/2026-11-25-wake-up-climate-flow-design.md`. Read it first;
this plan argues from it.

## Global Constraints

- Attribution: commits use `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, and each message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Wire names are fixed:
  - node types: `trigger.wake_up`, `climate.hold`, `climate.eco`;
  - params: `before_minutes`, `alarms`, `catch_up_minutes`, `zones`, `setpoint`, `mode`,
    `minutes`, `until`.
- Ranges are fixed:
  - `before_minutes` `0..=240`;
  - `catch_up_minutes` `0..=720` (absent or 0 means off);
  - step `minutes` `1..=1440`;
  - `until` is `"HH:MM"`, from 00:00 to 23:59.
- Every hold and every eco needs exactly one of `minutes` or `until`. There is no "forever".
- With `catch_up_minutes` absent, every existing flow behaves exactly as before.
- `nidoo-core` stays wasm-clean: no tokio, no clocks. Every clock and zone lives in
  `crates/nidoo`.
- Web strings go through `src/i18n` (fr + en). Run `cargo test -p nidoo-web` for the
  bare-text test. Never `wasm-pack test`: it links every web test and fills the disk.
- Never name the music server anywhere (`tools/brand-check.sh`).
- The engine uses the shared cargo target. A "missing file" build error means a concurrent
  clean, so retry.

## Review Focus

1. **Second restart inside the window.** The hub restarts at 06:20, catches up the 06:15 fire,
   then restarts again at 06:25. Expect no second run: the marker must arrive (retained
   replay) before the settle delay ends. Test in Task 5: `second_restart_does_not_refire`.
2. **Alarm disabled, deleted or moved while a wake-up timer is armed.** Expect the timer to
   disarm or move on the next reload, with no fire at the old time. Test in Task 3:
   `disabled_alarm_disarms`, `moved_alarm_rearms`.
3. **`until` earlier than now.** A step with `until: "06:00"` running at 06:30 holds until
   06:00 tomorrow. Expect that, not a refusal. Test in Task 2:
   `until_past_rolls_to_tomorrow`.
4. **Climate engine off, or a zone deleted after the flow was saved.** Expect the step to fail
   with the hub's reason per zone. Expect no panic and no stop to sibling branches. Test in
   Task 4: `hold_unknown_zone_fails_with_reason`.
5. **A catch-up trigger node renamed or removed in an edit.** Expect its old marker to be
   cleared and the new node to seed without firing. Test in Task 5:
   `renamed_node_seeds_without_firing`.

---

### Task 1: Model and validation in `nidoo-core`

**Files:**
- Modify: `crates/nidoo-core/src/flow/model.rs`. Add the `NodeKind` variants, param structs,
  `type_name`, `is_trigger` and `ports` (no named ports).
- Modify: `crates/nidoo-core/src/flow/validate.rs`. Add `FlowError` variants, the per-node
  checks, and `trigger.wake_up` in the "no value" trigger list at about line 647.
- Modify: `crates/nidoo-core/src/flow/mod.rs`. Add the re-exports.
- Test: unit tests in `model.rs` and `validate.rs` (existing `mod tests`).

**Interfaces:**
- Produces:
  - `NodeKind::TriggerWakeUp(TriggerWakeUpParams)`, wire `trigger.wake_up`.
  - `NodeKind::ClimateHold(ClimateHoldParams)`, wire `climate.hold`.
  - `NodeKind::ClimateEco(ClimateEcoParams)`, wire `climate.eco`.
  - The structs:

    ```rust
    pub struct TriggerWakeUpParams { pub before_minutes: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")] pub alarms: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] pub catch_up_minutes: Option<u32>,
        #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")] pub extra: BTreeMap<String, Value> }
    /// A number in °C or the string "off" (same JSON as nidoo_climate::doc::Setpoint).
    pub enum ClimateSetpoint { Temp(f64), Off }
    pub struct ClimateHoldParams { pub zones: Vec<String>, pub setpoint: ClimateSetpoint,
        pub mode: Option<String> /* "heat" | "cool" */, pub minutes: Option<u32>, pub until: Option<String>, extra }
    pub struct ClimateEcoParams { pub zones: Vec<String>, pub minutes: Option<u32>, pub until: Option<String>, extra }
    ```

  - Add `catch_up_minutes: Option<u32>` (serde default, skip if none) to
    `TriggerScheduleParams` and `TriggerSunParams`.
  - `impl NodeKind { pub fn catch_up_minutes(&self) -> u32 }`. It returns the value for the
    three time triggers and 0 for every other kind and when absent.
  - New `FlowError` variants:
    - `BadWakeUpBefore { node, minutes: u32 }`;
    - `BadCatchUp { node, minutes: u32 }`;
    - `BadClimateStep { node, reason: String }`.

- [ ] **Step 1: Write the failing tests.**
  - `wake_up_trigger_round_trips`: parse
    `{"id":"t","type":"trigger.wake_up","params":{"before_minutes":45,"alarms":["wake-up-1"],"catch_up_minutes":60}}`.
    Assert `is_trigger()`, `type_name() == "trigger.wake_up"` and `catch_up_minutes() == 60`.
    Re-encode it and compare equal.
  - `climate_hold_off_parses`: `"setpoint":"off"` gives `ClimateSetpoint::Off`. `21.5` gives
    `Temp(21.5)`. `"warm"` fails to parse.
  - `catch_up_absent_is_zero`: an existing schedule JSON with no `catch_up_minutes` gives
    `catch_up_minutes() == 0` and re-encodes byte-identical.
  - In `validate.rs`:
    - `before_minutes = 241` gives `BadWakeUpBefore`, and 240 is ok.
    - `catch_up_minutes = 721` on schedule, sun and wake_up gives `BadCatchUp`, and 720 is ok.
    - A climate step with empty `zones`, with both or neither of `minutes` and `until`, with
      `minutes = 0`, with `minutes = 1441`, with `until = "24:00"`, or with `mode = "fan"`
      gives `BadClimateStep`.
    - An `if` with no `entity` reachable from a `trigger.wake_up` is refused, the same as from
      a schedule trigger.

- [ ] **Step 2: Run and see them fail.** Run `cargo test -p nidoo-core flow::`. The expected
  result is compile errors or failures on the new names.
- [ ] **Step 3: Implement the variants, structs, serde for `ClimateSetpoint`, and the checks.**
  `ClimateSetpoint` serde mirrors `nidoo_climate::doc::Setpoint` (a number or `"off"`). Give
  it a hand-written `schemars` impl like `Setpoint`'s. Fix every exhaustive `match` on
  `NodeKind` in `nidoo-core` that the compiler flags.
- [ ] **Step 4: Run.** Run `cargo test -p nidoo-core`. Expect PASS.
- [ ] **Step 5: Run `cargo check --workspace`.** It will flag exhaustive matches in `nidoo`
  and `nidoo-web`. Add placeholder arms only where the compiler forces it, with
  `// Task N` comments naming the task that fills them in:
  - engine `run_node`: return `failed("not implemented")` (filled by Task 4);
  - web labels: the generic step text (filled by Task 6).

  Commit:
  `feat(flows): wake-up trigger, climate steps and catch-up in the flow model`.

### Task 2: Pure time math in `nidoo-core`

**Files:**
- Modify: `crates/nidoo-core/src/flow/schedule.rs`. Add `Schedule::latest_in`.
- Modify: `crates/nidoo-core/src/flow/wake_up.rs`. Add the alarm lookup and pre-ring math.
- Create: `crates/nidoo-core/src/flow/climate.rs`, for `until` resolution. Register it in
  `mod.rs`.
- Test: unit tests in each file.

**Interfaces:**
- Consumes: `TriggerWakeUpParams` (Task 1), plus `wake_up::{read, is_managed, cron}` and
  `Schedule::{parse, next_after}`.
- Produces:
  - `Schedule::latest_in<Tz: TimeZone>(&self, from: &DateTime<Tz>, to: &DateTime<Tz>) -> Option<DateTime<Tz>>`.
    The last tick `t` with `from <= t <= to`.
  - `wake_up::alarm_schedules(flows: &[Flow], only: &[String]) -> Vec<(String, String, Schedule)>`.
    Each entry is `(flow id, cron, schedule)`. It covers enabled flows that are managed and
    readable. When `only` is non-empty it keeps only those ids. It is sorted by id.
  - `wake_up::next_pre_ring<Tz: TimeZone>(alarms: &[Schedule], before_minutes: u32, after: &DateTime<Tz>) -> Option<DateTime<Tz>>`.
    The minimum over the alarms of `s.next_after(after + before) - before`.
  - `wake_up::latest_pre_ring_in<Tz: TimeZone>(alarms: &[Schedule], before_minutes: u32, from: &DateTime<Tz>, to: &DateTime<Tz>) -> Option<DateTime<Tz>>`.
    The maximum over the alarms of `s.latest_in(from + before, to + before) - before`.
  - `climate::until_after<Tz: TimeZone>(now: &DateTime<Tz>, hhmm: &str) -> Option<DateTime<Tz>>`.
    The next local `HH:MM` strictly after `now`; today if it is still ahead, else tomorrow.
    On a DST gap, it returns the first valid instant after the gap.

- [ ] **Step 1: Write the failing tests**, all with `chrono_tz::Europe::Paris` instants:
  - `latest_in_picks_last_tick`: cron `0 * * * *`, from 05:10 to 07:30. Expect 07:00.
    An empty window gives `None`.
  - `pre_ring_crosses_midnight`: alarm `15 0 * * 1` (Monday 00:15), `before_minutes` 45,
    after Sunday 22:00. Expect Sunday 23:30.
  - `pre_ring_two_alarms_same_time_once`: two schedules `0 7 * * *`, before 30, after 06:00.
    Expect 06:30. Also `latest_pre_ring_in` between 06:00 and 07:00 gives 06:30.
  - `pre_ring_dst_day`: alarm `0 7 * * *` on 2026-03-29 (spring forward), before 45.
    Expect 06:15 local, exactly 45 min before the 07:00 ring.
  - `alarm_schedules_skips_disabled_and_unknown`: given one enabled wake-up flow, one
    disabled wake-up flow and a normal flow, expect only the enabled wake-up flow.
    `only = ["nope"]` gives an empty list.
  - `until_past_rolls_to_tomorrow`: now 06:30, `"06:00"`. Expect tomorrow 06:00.
    `"07:00"` gives today 07:00. `"06:30"` gives tomorrow 06:30 (strictly after).

- [ ] **Step 2: Run and see them fail.** Run `cargo test -p nidoo-core flow::`.
- [ ] **Step 3: Implement.** `latest_in` walks `next_after` from `from - 1s` while the tick is
  `<= to`. Stop after 10 000 ticks as a guard for `* * * * *` over 12 h.
- [ ] **Step 4: Run.** Run `cargo test -p nidoo-core`. Expect PASS. Commit:
  `feat(flows): pre-ring and catch-up time math`.

### Task 3: Engine — arm and fire `trigger.wake_up`

**Files:**
- Create: `crates/nidoo/src/flows/wake_up_timer.rs`, for `ArmedWakeUp` (modelled on
  `sun.rs`'s `ArmedSun`). Register it in `flows/mod.rs`.
- Modify: `crates/nidoo/src/flows/engine.rs`:
  - the `Engine` field `wake_timers: BTreeMap<String, ArmedWakeUp>`;
  - `arm_wake_ups()`, called from `reload` next to `arm_suns`;
  - `tick_wake_ups()` in the timer select branch;
  - the chain in `next_deadline`;
  - the trigger detail in `run_node`: `wake-up -{before}m`.
- Test: create `crates/nidoo/tests/engine_wake_up.rs`, with a harness copied from
  `tests/engine_sun.rs` (paused tokio clock, `Hub::new`, `start_engine`).

**Interfaces:**
- Consumes: `wake_up::alarm_schedules`, `wake_up::next_pre_ring` (Task 2).
- Produces:
  - `ArmedWakeUp { node: String, at: DateTime<Tz>, fingerprint: String }`.
  - The fingerprint is `before_minutes`, then `alarms`, then each matched `(id, cron)` pair,
    joined by `|`.
  - On reload, the engine keeps an armed entry only if its fingerprint is unchanged. Otherwise
    it re-arms from now.
  - Firing follows `tick_suns`: re-arm from `max(at, now)`, no catch-up. While offline, it
    calls `add_skipped_run_because(…, "the broker link is offline")`.

- [ ] **Step 1: Write the failing tests** in `engine_wake_up.rs`:
  - `fires_before_the_alarm`: alarm flow from `wake_up::to_flow` at now+60 min, every day.
    Heating flow `trigger.wake_up{before_minutes:45}` → a `command` on the test LED. Advance
    14 min: no run. Advance 2 more min: one `RunView` with status ok.
  - `moved_alarm_rearms`: after arming, re-save the alarm 30 min later. Advance past the old
    time: no run. Advance past the new time: one run.
  - `disabled_alarm_disarms`: disable the alarm flow and advance past the pre-ring. Expect no
    run.
  - `alarms_filter`: two alarms, the trigger lists only the second. Only the second's
    pre-ring fires.
  - `no_alarm_never_fires`: a trigger with no wake-up flow advances 25 h with no run and no
    panic.

- [ ] **Step 2: Run and see them fail.** Run `cargo test -p nidoo --test engine_wake_up`.
- [ ] **Step 3: Implement `arm_wake_ups` and `tick_wake_ups`**, mirroring `arm_suns` and
  `tick_suns` (`engine.rs` around lines 563–700).
- [ ] **Step 4: Run.** Run `cargo test -p nidoo --test engine_wake_up` and
  `cargo test -p nidoo --test engine_sun`. Expect PASS. Commit:
  `feat(flows): trigger.wake_up fires before a wake-up alarm`.

### Task 4: Engine — `climate.hold` and `climate.eco` steps, and save-time warnings

**Files:**
- Modify: `crates/nidoo/src/flows/engine.rs`.
  - Change `run_node`'s first parameter to `hub: &Arc<Hub>`. The climate methods take
    `self: &Arc<Self>`. The callers in `execute` and the tests already hold an `Arc`.
  - Add the two arms, plus `fn climate_detail(...)` helpers.
- Modify: `crates/nidoo/src/flows/hub.rs`, in `set_flow`:
  - `check_house` gains a `check_setpoint` range check on hold setpoints. It refuses with
    `HubError::InvalidFlow`, the same message shape as the climate page.
  - New `fn flow_warnings(&self, flow: &Flow) -> Vec<String>`, joined with `" "` into the
    existing `warning`. It covers the Paris fallback (existing), unknown climate zone ids,
    unknown ids in `alarms`, and "no wake-up alarm exists" for a `trigger.wake_up`.
- Test: extend `crates/nidoo/tests/flows.rs`, or create `tests/flow_climate.rs` with the
  zone setup copied from `tests/climate_hub.rs`.

**Interfaces:**
- Consumes:
  - `ClimateHoldParams` and `ClimateEcoParams` (Task 1), and `climate::until_after` (Task 2).
  - `Hub::set_climate_override(&Arc<Self>, zone, Override)` and
    `Hub::set_climate_eco(&Arc<Self>, zone, RoomEco)`.
  - `nidoo_climate::view::{OverrideRequest, RoomEcoRequest}` to build the documents (the same
    path as `ambiances/apply.rs::apply_climate`).
- Produces:
  - Behaviour per spec: zones are applied in list order, and successes stay applied.
  - Success detail: `living 21 °C until 10:00` (`off` for `Off`), with zones joined by `, `.
  - Failure detail: `bedroom: <hub reason>` per failed zone, joined by `; `, and
    `StepOutcome::Failed`.
  - `until` resolves with `until_after(now_in(now, hub.zone()), hhmm)` to Unix seconds.
    `minutes` becomes `OverrideRequest.minutes` or `RoomEcoRequest.until = now + 60*m`.

- [ ] **Step 1: Write the failing tests:**
  - `hold_sets_override_until`: an active zone `living`. Run a manual flow with a
    `climate.hold{zones:["living"],setpoint:21,minutes:180}` step. Assert the zone's override
    is `setpoint 21`, `origin User`, `until = now + 10800`, and the step detail starts with
    `living 21 °C until`.
  - `eco_sets_room_eco_until_time`: `climate.eco{zones:["bedroom"],until:"18:00"}`. The
    zone's eco `until` equals `until_after(now, "18:00")`.
  - `hold_unknown_zone_fails_with_reason`: zones `["living","gone"]`. `living` is applied,
    the step is failed, and the detail contains `gone:`. A sibling branch still runs.
  - `hold_ends_eco`: a zone in eco, then a temperature hold step. Eco is cleared (existing hub
    side effect).
  - `save_warns_unknown_zone_and_alarm`: `set_flow` with an unknown zone and an unknown alarm
    id returns `Ok` with a `warning` naming both. A setpoint of 99 is refused.

- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Run `cargo test -p nidoo --test flow_climate` and
  `cargo test -p nidoo --lib flows::`. Expect PASS. Commit:
  `feat(flows): climate.hold and climate.eco steps`.

### Task 5: Catch-up if missed

**Files:**
- Modify: `crates/nidoo/src/hub.rs`.
  - `fn fired_of<'a>(&self, topic: &'a str) -> Option<(&'a str, &'a str)>` matches
    `<automation_prefix>/<flow>/fired/<node>`. Route it in `ingest` before `flow_id_of`.
  - A `fired: Mutex<BTreeMap<(String, String), i64>>` store.
- Modify: `crates/nidoo/src/flows/hub.rs`:
  - `pub(crate) fn fired_due(&self, flow: &str, node: &str) -> Option<i64>`;
  - `pub(crate) fn record_fired(&self, flow: &str, node: &str, due: DateTime<Utc>)`. It
    publishes retained `{"due": <unix s>}` and stores it locally.
  - `fn clear_fired(&self, flow: &str, keep: &BTreeSet<String>)`. It publishes an empty
    retained payload for each marker of `flow` whose node is not in `keep`. Call it from
    `delete_flow` with an empty keep set. Call it from `set_flow` with the ids of the saved
    flow's trigger nodes that have `catch_up_minutes > 0`.
- Modify: `crates/nidoo/src/flows/engine.rs`:
  - `const CATCH_UP_SETTLE: Duration = 10 s`;
  - fields `catch_up_settle: Option<DateTime<Tz>>`;
  - on `ConnectionChanged(Online)`, and at start when already online, set
    `catch_up_settle = now + CATCH_UP_SETTLE`;
  - chain it in `next_deadline`;
  - in the timer branch, `tick_catch_up()`. It runs `catch_up()` once when due and online,
    then sets `catch_up_settle = None`.
  - Also: the normal `tick`, `tick_suns` and `tick_wake_ups` call
    `record_fired(flow, node, armed.at)` when they fire a node with `catch_up_minutes > 0`.
- Modify: `run_node` trigger arms for the three time triggers. When the `TriggerInfo.value`
  is a string, use it as the detail (`caught up: due 06:15`).
- Test: create `crates/nidoo/tests/engine_catch_up.rs`.

**Interfaces:**
- Consumes:
  - `Schedule::latest_in` and `wake_up::latest_pre_ring_in` (Task 2);
  - for sun, a new `pub(super) fn latest_sun_fire_in(lat, lon, zone, which, offset, from, to) -> Option<DateTime<Tz>>`
    in `flows/sun.rs`, built from the same event search as `next_sun_fire`;
  - `NodeKind::catch_up_minutes()` (Task 1).
- Produces `catch_up()`. For each enabled flow and each trigger node with
  `w = catch_up_minutes() > 0`:
  - Compute `d = latest due in [now - w min, now]`.
  - No marker: `record_fired(d.unwrap_or(now))`, and do not fire.
  - Marker earlier than `d`: `record_fired(d)`, then `fire(flow, node, Some(TriggerInfo { entity: None, field: None, value: "caught up: due HH:MM".into(), previous: Value::Null }))`.
  - Otherwise do nothing.
  - It never runs from `reload`.

- [ ] **Step 1: Write the failing tests.** Simulate a restart by building a fresh hub. Before
  `start_engine`, ingest the retained flow and marker payloads the old hub published (capture
  them from its publisher channel).
  - `restart_inside_window_fires_once`: schedule `15 6 * * *`, `catch_up_minutes` 60. The
    marker is yesterday 06:15. Restart at 06:40. After the 10 s settle: one run, with trigger
    detail `caught up: due 06:15`. The new marker equals today 06:15.
  - `restart_outside_window_does_not_fire`: the same, restarted at 07:30. Expect no run.
  - `second_restart_does_not_refire`: restart again at 06:45 with the marker from the first
    catch-up. Expect no run.
  - `missed_ticks_fire_once`: cron `*/5 * * * *`, window 60, marker 50 min old. Expect exactly
    one run.
  - `link_back_catches_up_skipped_tick`: go offline before a tick, which is skipped, then
    online. After the settle: one run.
  - `no_marker_seeds_without_firing`: catch-up on, no marker. After the settle, no run and a
    marker is published.
  - `renamed_node_seeds_without_firing`: save the flow with its trigger node id changed. The
    old marker topic gets an empty retained publish. The next check seeds and does not fire.
  - `edit_never_catches_up`: a reload at 06:40 with a missed 06:15 and no link change. Expect
    no run.
  - `absent_option_keeps_old_behaviour`: the same as the first test without
    `catch_up_minutes`. Expect no run and no marker published.
  - `wake_up_and_sun_catch_up`: one case each for `trigger.wake_up` and `trigger.sun`.

- [ ] **Step 2: Run and see them fail.** Run `cargo test -p nidoo --test engine_catch_up`.
- [ ] **Step 3: Implement the hub store, ingest routing, the markers and `catch_up()`.**
- [ ] **Step 4: Run** `cargo test -p nidoo --test engine_catch_up --test engine_sun --test engine_wake_up --test flows`.
  Expect PASS. Commit: `feat(flows): opt-in catch-up for missed time triggers`.

### Task 6: Web canvas — new cards, sheets and catch-up field

**Files:**
- Modify the exhaustive-match sites found in Task 1, Step 5:
  - `crates/nidoo-web/src/flow_edit.rs`, `flow_layout.rs`, `flows.rs` (titles and summaries),
    `flow_text.rs` (sentences), `flow_list.rs` (`TriggerKind::WakeUp`), `icons.rs`,
    `if_value.rs`, `screens/flow_canvas/graph.rs`.
  - `screens/flow_canvas/settings.rs`: the sheet dispatch.
- Modify: `crates/nidoo-web/src/flow_picker.rs`. Add generic cards:
  - `Group::When`: "Before a wake-up alarm" (`before_minutes: 45`,
    `catch_up_minutes: Some(60)`).
  - `Group::Then`: "Heating hold" (`setpoint: 20`, `minutes: 120`).
  - `Group::Then`: "Room eco" (`until: "18:00"`).
  - Use whichever existing group holds steps.
- Create: `crates/nidoo-web/src/screens/flow_canvas/wake_up.rs`. The sheet has:
  - a minutes field;
  - an alarm multi-select from `House::flows` filtered by `wake_up::is_managed`, shown by
    name, where empty means all alarms;
  - a next-fire line computed client-side with `wake_up::next_pre_ring` and `house_place` from
    `sun.rs`.
- Create: `crates/nidoo-web/src/screens/flow_canvas/climate_step.rs`. One sheet serves both
  steps:
  - a zone multi-select fed by `api::climate::get_climate()` (as `ambiances/editor.rs:140`
    does), shown by name;
  - a setpoint stepper with an "off" toggle (hold only);
  - a duration choice: "for N minutes" or "until HH:MM".
- Create: `crates/nidoo-web/src/screens/flow_canvas/catch_up.rs`. A `CatchUpField` component
  (toggle + minutes, `0..=720`) used by the schedule, sun and wake-up sheets.
- Modify: `crates/nidoo-web/src/i18n/flows.rs`, adding `FlowText` variants with fr and en.
- Test: unit tests beside each new module (native `cargo test`), plus the bare-text test.

**Interfaces:**
- Consumes: the Task 1 types, plus Task 2's `wake_up::next_pre_ring` and
  `climate::until_after`.
- Produces UI only. Sentence texts (en):
  - "45 min before a wake-up alarm" / "45 min before Weekdays";
  - "Hold Living room at 21 °C for 3 h" / "… until 10:00";
  - "Eco in Bedroom, Office until 18:00";
  - catch-up suffix: ", catches up within 60 min".
  - French wording follows the existing `FlowText` style.

- [ ] **Step 1: Write the failing tests:**
  - `wake_up_sentence_names_alarm_or_all`;
  - `hold_sentence_off_and_until`;
  - `eco_sentence_lists_zone_names` (unknown ids fall back to the raw id);
  - `catch_up_suffix_only_when_on`;
  - `picker_offers_three_new_cards`.
- [ ] **Step 2: Run and see them fail.** Run `cargo test -p nidoo-web`.
- [ ] **Step 3: Implement the sheets, cards, texts and i18n.** Brace closure attributes on
  self-closing tags (Leptos `view!` misparses `/>` otherwise).
- [ ] **Step 4: Run** `cargo test -p nidoo-web` (bare-text test included) and
  `cargo check -p nidoo-web --target wasm32-unknown-unknown`. Expect PASS. Commit:
  `feat(web): wake-up trigger, climate steps and catch-up in the flow canvas`.

### Task 7: Docs and MCP

**Files:**
- Modify: `docs/reference/flow-nodes.md`:
  - a `trigger.wake_up` section after `trigger.sun`;
  - `climate.hold` and `climate.eco` sections after `notify`;
  - a "Catch up if missed" subsection under "How a run goes", naming the three triggers, the
    window, the marker topic `<automation_prefix>/<flow>/fired/<node>`, and when checks happen;
  - the "Morning heating" example flow JSON from the spec.
- Modify: `docs/reference/mqtt-topics.md`, adding the marker topic.
- Modify: `crates/nidoo/src/mcp/mod.rs`. In the server instructions, name the three node types
  and `catch_up_minutes`. Add test assertions that `instr` contains `trigger.wake_up` and
  `climate.hold`.

- [ ] **Step 1:** Add the MCP test assertions. Run `cargo test -p nidoo --lib mcp::` and see
  them fail.
- [ ] **Step 2:** Write the docs and instructions.
- [ ] **Step 3:** Run `cargo test -p nidoo --lib mcp::` and `tools/brand-check.sh`. Expect
  PASS. Commit: `docs(flows): wake-up trigger, climate steps and catch-up`.

### Final verification

- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`. Database tests skip without Postgres, and that is expected. All
  green.
- [ ] Run `cargo check` in `satellite/mac`. It is its own workspace and only matters if
  `nidoo-core` types it uses changed.
