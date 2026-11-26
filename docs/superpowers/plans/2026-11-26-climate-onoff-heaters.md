# Climate on/off heaters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A climate zone can heat with on/off `switch` heaters (thermostat or hold drive, with a cool-down), each heat block can say which devices heat the room, and the climate units fill in while a chosen heater cannot run.

**Architecture:** `decide` stays the single decision owner. On/off heaters reach it as a separate input (`ZoneInputs::heaters`), and a pure post-pass (`heaters::apply`) picks the chosen set from the decision's source and the schedule, turns the unchosen climate units off, adds each switch's target (`mode: "heat"` = on, `"off"`), and writes the reason. The hub builds the heater inputs from `StateValue::OnOff`, keeps each heater's `off_since` / `on_since` (restored from the retained decision), sends `TurnOn` / `TurnOff`, and turns a hand flip into a per-device override.

**Tech Stack:** Rust workspace: `nidoo-climate` (pure, also wasm), `nidoo` (hub, tokio), `nidoo-web` (Leptos, wasm). serde retained JSON documents.

**Spec:** `docs/superpowers/specs/2026-11-26-climate-onoff-heaters-design.md`

## Global Constraints

- Every new document field is optional: `#[serde(default, skip_serializing_if = …)]`. A document without heaters serializes byte-for-byte as before, and `learn::fingerprint` of it is unchanged.
- The engine acts on on/off heaters only when the zone's decided mode is heat and the zone is `active` or `pending`. In cool mode, season `off`, or zone `off`, nothing is published to a heater, except the one last off of Task 7.
- Switching off is always allowed. No path switches a heater on before `off_since + min_off_min * 60`, safety frost included.
- Thermostat band: on at `reading <= target - 0.3`, off at `reading >= target`. Constant `THERMOSTAT_BAND: f64 = 0.3`.
- Limits: `min_off_min` 0..=240, `min_on_min` 0..=120.
- Thermometer correction never applies to an on/off heater.
- A zone with no `heaters` and no `heated_by` decides exactly as today: every existing test stays green without edits other than added struct fields.
- Web strings go through `crates/nidoo-web/src/i18n/climate.rs`, English and French, French with `\u{A0}` before `:`. Run `cargo test -p nidoo-web` (not only `--lib`): bare text literals fail it.
- `nidoo-climate` must keep building for wasm: `cargo check -p nidoo-web --target wasm32-unknown-unknown`.
- Never write the name of any commercial media player anywhere (CI brand check).
- Commits as `Gekkotron` (`-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`), conventional messages.
- Local tests: there is no Postgres on the dev machine, so DB tests skip; `cargo test --workspace` needs an empty `crates/nidoo-web/dist`; never run `wasm-pack test` (it links every web test binary).

## Review Focus

1. **A heater listed in the settings whose entity is gone or is not a switch any more** (device removed from Zigbee): the zone keeps deciding, the heater is ignored, the reason names it. Test in Task 4 (`unknown_heated_by_id_falls_back`) and Task 6 (`missing_heater_entity_is_skipped`).
2. **Hub restart while a hold heater is on or cooling down**: clocks come back from the retained decision; no off/on blip, no early switch-on. Test in Task 7 (`restart_inside_cooldown_does_not_switch_on`, `restart_keeps_running_heater_on`).
3. **The same switch used by a flow or by hand at the same minute as an engine command**: the echo of the engine's own command is not a hand change. Test in Task 7 (`engine_echo_is_not_a_hand_change`).
4. **A schedule whose every block names a heater that is disabled**: the room default applies, then the climate units; the room is never left unheated silently. Test in Task 4 (`disabled_heater_is_not_chosen`).
5. **Season flips to cool while the stove is on**: one off, then silence, including across a restart in cool. Test in Task 7 (`leaving_heat_sends_one_off_then_nothing`).

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/nidoo-climate/src/doc.rs` (modify) | `Heater`, `HeaterDrive`, `HeaterRun`; new fields on `ZoneSettings`, `Block`, `Decision`; validation |
| `crates/nidoo-climate/src/schedule.rs` (modify) | who is named at an instant, `next_change` counting `heated_by`, next start of a heater |
| `crates/nidoo-climate/src/heaters.rs` (create) | pure: chosen set, thermostat / hold drive, cool-down, fill-in, reason text |
| `crates/nidoo-climate/src/decide.rs` (modify) | `ZoneInputs::heaters`, call the post-pass |
| `crates/nidoo-climate/src/learn.rs` (modify) | carry `heated_by` over, fingerprint ignores it |
| `crates/nidoo/src/climate/inputs.rs`, `hub.rs` (modify) | heater inputs, claims, save-time validation |
| `crates/nidoo/src/climate/heater_io.rs` (create) | hub side of heaters: clocks, observe, hand flip, on/off command, last off |
| `crates/nidoo/src/climate/engine.rs` (modify) | call into `heater_io` from the pass and the observer |
| `crates/nidoo-web/src/climate/heaters_card.rs` (create) | the Heaters section of the room settings |
| `crates/nidoo-web/src/climate/timeline.rs`, `proposal.rs`, `room_settings.rs`, `i18n/climate.rs`, `entity_picker/needs.rs` (modify) | Heated by row, labels, wiring, strings, switch predicate |
| `docs/users/climate.md`, `docs/reference/*`, the v1 spec (modify) | user and reference docs |

---

### Task 1: Document model

**Files:**
- Modify: `crates/nidoo-climate/src/doc.rs`
- Modify: every `Block { … }` literal in the workspace (`grep -rn "Block {" crates --include=*.rs`), adding `heated_by: None`
- Test: unit tests in `crates/nidoo-climate/src/doc.rs`

**Interfaces:**
- Produces:
  ```rust
  #[serde(rename_all = "snake_case")] pub enum HeaterDrive { Thermostat, Hold }   // default Thermostat
  #[serde(default)] pub struct Heater { pub entity: String, pub drive: HeaterDrive,
      pub enabled: bool /* default true */, pub min_off_min: u32, pub min_on_min: u32 }
  pub struct HeaterRun { pub off_since: Option<u64>, pub on_since: Option<u64>, pub wait_until: Option<u64> }
  // ZoneSettings: pub heaters: Vec<Heater>, pub heated_by: Option<Vec<String>>, pub fill_in: bool /* default true */
  // Block:        pub heated_by: Option<Vec<String>>
  // Decision:     pub heaters: BTreeMap<String, HeaterRun>   (in same_outcome)
  impl ZoneSettings { pub fn heater(&self, id: &str) -> Option<&Heater>;        // enabled or not
                      pub fn validate_heated_by(&self, units: &[String]) -> Result<(), String>; }
  pub const THERMOSTAT_BAND: f64 = 0.3;
  ```
  Serde: `heaters` skipped when empty, `heated_by` when `None`, `fill_in` when `true`, each `HeaterRun` field when `None`, `Decision::heaters` when empty. All derive `schemars::JsonSchema` under the feature like their neighbours.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn old_zone_document_round_trips_unchanged() {
    let json = serde_json::to_string(&ZoneSettings::default()).unwrap();
    assert!(!json.contains("heaters") && !json.contains("heated_by") && !json.contains("fill_in"));
    let block: Block = serde_json::from_str(
        r#"{"days":["mon"],"from":"09:00","to":"10:00","setpoint":20.0}"#).unwrap();
    assert_eq!(block.heated_by, None);
    assert!(!serde_json::to_string(&block).unwrap().contains("heated_by"));
}

#[test]
fn heater_defaults() {
    let h: Heater = serde_json::from_str(r#"{"entity":"sw"}"#).unwrap();
    assert_eq!((h.drive, h.enabled, h.min_off_min, h.min_on_min), (HeaterDrive::Thermostat, true, 0, 0));
}

#[test]
fn heaters_validate() {
    let mut s = ZoneSettings::default();
    s.heaters = vec![heater("sw"), heater("sw")];
    assert!(s.validate().unwrap_err().contains("twice"));
    s.heaters = vec![Heater { min_off_min: 241, ..heater("sw") }];
    assert!(s.validate().unwrap_err().contains("min_off_min"));
    s.heaters = vec![Heater { min_on_min: 121, ..heater("sw") }];
    assert!(s.validate().unwrap_err().contains("min_on_min"));
    s.heaters = vec![heater("group_x")];
    assert!(s.validate().unwrap_err().contains("group"));
}

#[test]
fn heated_by_validates_against_heaters_and_units() {
    let mut s = ZoneSettings::default();
    s.heaters = vec![heater("stove")];
    s.heated_by = Some(vec!["stove".into(), "ac".into()]);
    assert!(s.validate_heated_by(&["ac".into()]).is_ok());
    s.heated_by = Some(vec![]);
    assert!(s.validate_heated_by(&["ac".into()]).unwrap_err().contains("empty"));
    s.heated_by = None;
    s.rules.heat = Some(rules_with_block_heated_by(&["gone"]));
    assert!(s.validate_heated_by(&["ac".into()]).unwrap_err().contains("heat block 0"));
    s.rules.heat = None;
    s.rules.cool = Some(rules_with_block_heated_by(&["stove"]));
    assert!(s.validate().unwrap_err().contains("cool block 0"));   // heated_by on a cool block
}
```
`heater(id)` and `rules_with_block_heated_by(ids)` are local test helpers.

- [ ] **Step 2: Run** `cargo test -p nidoo-climate doc::` — expected: does not compile (`Heater` unknown).

- [ ] **Step 3: Implement** the types, defaults, serde attributes and validation above. `ZoneSettings::validate` checks the heater list and rejects `heated_by` on cool blocks; `validate_heated_by(units)` checks the room default and every heat block against `heaters ∪ units` (errors: `"heated_by is empty"`, `"heat block {i}: unknown heater {id}"`). Add `heated_by: None` to every `Block` literal and `heaters: BTreeMap::new()` to `base_decision`.

- [ ] **Step 4: Run** `cargo test -p nidoo-climate` and `cargo check --workspace --tests` — expected: PASS, no warnings.

- [ ] **Step 5: Commit** `feat(climate): document model for on/off heaters`

---

### Task 2: Schedule lookups

**Files:**
- Modify: `crates/nidoo-climate/src/schedule.rs`
- Test: unit tests in the same file

**Interfaces:**
- Consumes: `Block::heated_by`.
- Produces:
  ```rust
  impl ModeRules {
      /// The block's `heated_by` at `local`, `None` for the base or a block that names nobody.
      pub fn heated_by_at(&self, local: NaiveDateTime) -> Option<&[String]>;
      /// The next instant strictly after `local`, within `within_min` minutes, where a block
      /// naming `id` starts. `default_names` says whether base time and unnamed blocks count.
      pub fn next_start_of(&self, id: &str, default_names: bool, local: NaiveDateTime,
                           within_min: u32) -> Option<NaiveDateTime>;
  }
  ```
  `next_change` now compares the pair `(setpoint, heated_by)`.

- [ ] **Step 1: Write the failing tests** (Monday 2026-11-23 as the reference day)

```rust
#[test]
fn next_change_counts_a_heated_by_edge() {
    // two touching blocks, same 20.0, 09:00-12:00 by ["stove"], 12:00-18:00 unnamed
    let r = rules(15.0, vec![named(9, 12, 20.0, &["stove"]), plain(12, 18, 20.0)]);
    assert_eq!(r.next_change(at(10, 0)), Some(at(12, 0)));
}

#[test]
fn heated_by_at_reads_the_block() {
    let r = rules(15.0, vec![named(9, 12, 20.0, &["stove"])]);
    assert_eq!(r.heated_by_at(at(10, 0)), Some(&["stove".to_string()][..]));
    assert_eq!(r.heated_by_at(at(13, 0)), None);
}

#[test]
fn next_start_of_finds_a_block_within_the_window() {
    let r = rules(15.0, vec![named(9, 12, 20.0, &["stove"]), named_hm((12, 10), (14, 0), 20.0, &["stove"])]);
    assert_eq!(r.next_start_of("stove", false, at(12, 0), 15), Some(at(12, 10)));
    assert_eq!(r.next_start_of("stove", false, at(12, 0), 5), None);
    assert_eq!(r.next_start_of("ac", false, at(12, 0), 15), None);
    // default_names: the base after 14:00 counts for a heater in the room default
    assert_eq!(r.next_start_of("ac", true, at(13, 55), 15), Some(at(14, 0)));
}
```
Write the helpers `rules`, `plain`, `named`, `named_hm`, `at`.

- [ ] **Step 2: Run** `cargo test -p nidoo-climate schedule::` — expected: FAIL to compile.
- [ ] **Step 3: Implement** the two functions and the `next_change` comparison. `next_start_of` walks the same block-edge candidates as `next_change`, limited to `local + within_min`.
- [ ] **Step 4: Run** `cargo test -p nidoo-climate` — expected: PASS.
- [ ] **Step 5: Commit** `feat(climate): schedule knows who heats each block`

---

### Task 3: Pure heater drive

**Files:**
- Create: `crates/nidoo-climate/src/heaters.rs`; add `pub mod heaters;` to `lib.rs`
- Test: unit tests in `heaters.rs`

**Interfaces:**
- Consumes: `Heater`, `HeaterDrive`, `HeaterRun`, `THERMOSTAT_BAND`.
- Produces:
  ```rust
  /// One on/off heater as the hub sees it now.
  #[derive(Clone, Debug, PartialEq)]
  pub struct HeaterDevice { pub id: String, pub on: Option<bool>, pub available: bool,
                            pub off_since: Option<u64>, pub on_since: Option<u64> }

  #[derive(Clone, Copy, Debug, PartialEq)]
  pub enum Want { On, Off, /// wanted on, not allowed before this Unix time
                  Wait(u64) }

  /// What one heater should do. `chosen`: it is in the chosen set with a temperature setpoint.
  /// `target`: the zone target. `reading`: the usable room reading. `bridge`: a hold heater
  /// will be chosen again within its cool-down.
  pub fn drive(h: &Heater, dev: &HeaterDevice, chosen: bool, target: Option<f64>,
               reading: Option<f64>, bridge: bool, now: u64) -> Want;
  ```

- [ ] **Step 1: Write the failing tests**

```rust
const NOW: u64 = 1_800_000_000;
fn th() -> Heater { Heater { entity: "rad".into(), ..Heater::default() } }
fn hold(min_off: u32) -> Heater { Heater { entity: "stove".into(), drive: HeaterDrive::Hold, min_off_min: min_off, ..Heater::default() } }
fn dev(on: bool) -> HeaterDevice { HeaterDevice { id: "x".into(), on: Some(on), available: true, off_since: None, on_since: None } }

#[test] fn thermostat_switches_on_below_the_band() {
    assert_eq!(drive(&th(), &dev(false), true, Some(20.0), Some(19.7), false, NOW), Want::On);
    assert_eq!(drive(&th(), &dev(false), true, Some(20.0), Some(19.8), false, NOW), Want::Off); // inside band: unchanged
    assert_eq!(drive(&th(), &dev(true),  true, Some(20.0), Some(19.8), false, NOW), Want::On);  // inside band: unchanged
    assert_eq!(drive(&th(), &dev(true),  true, Some(20.0), Some(20.0), false, NOW), Want::Off);
}
#[test] fn thermostat_without_a_reading_is_off_even_inside_min_on() {
    let h = Heater { min_on_min: 10, ..th() };
    let d = HeaterDevice { on_since: Some(NOW - 60), ..dev(true) };
    assert_eq!(drive(&h, &d, true, Some(20.0), None, false, NOW), Want::Off);
}
#[test] fn thermostat_min_on_delays_the_off() {
    let h = Heater { min_on_min: 10, ..th() };
    let d = HeaterDevice { on_since: Some(NOW - 60), ..dev(true) };
    assert_eq!(drive(&h, &d, true, Some(20.0), Some(20.5), false, NOW), Want::On);
    let d = HeaterDevice { on_since: Some(NOW - 601), ..dev(true) };
    assert_eq!(drive(&h, &d, true, Some(20.0), Some(20.5), false, NOW), Want::Off);
}
#[test] fn not_chosen_is_off_and_min_on_does_not_keep_it() {
    let h = Heater { min_on_min: 10, ..th() };
    let d = HeaterDevice { on_since: Some(NOW - 60), ..dev(true) };
    assert_eq!(drive(&h, &d, false, Some(20.0), Some(15.0), false, NOW), Want::Off);
}
#[test] fn hold_is_on_whatever_the_reading() {
    assert_eq!(drive(&hold(15), &dev(false), true, Some(20.0), Some(25.0), false, NOW), Want::On);
    assert_eq!(drive(&hold(15), &dev(false), true, Some(20.0), None, false, NOW), Want::On);
}
#[test] fn hold_bridges_a_short_gap_only_while_on() {
    assert_eq!(drive(&hold(15), &dev(true),  false, None, None, true, NOW), Want::On);
    assert_eq!(drive(&hold(15), &dev(false), false, None, None, true, NOW), Want::Off);
}
#[test] fn cool_down_makes_it_wait() {
    let d = HeaterDevice { off_since: Some(NOW - 300), ..dev(false) };
    assert_eq!(drive(&hold(15), &d, true, Some(20.0), None, false, NOW), Want::Wait(NOW - 300 + 900));
    let d = HeaterDevice { off_since: Some(NOW - 900), ..dev(false) };
    assert_eq!(drive(&hold(15), &d, true, Some(20.0), None, false, NOW), Want::On);
}
#[test] fn unknown_off_clock_means_no_cool_down_pending() {
    assert_eq!(drive(&hold(15), &dev(false), true, Some(20.0), None, false, NOW), Want::On); // see note
}
```
Note for the last test: `drive` treats `off_since: None` as "no cool-down pending"; the hub (Task 7) is what sets `off_since = now` for a heater first seen off, so the fresh-hub rule lives there.

- [ ] **Step 2: Run** `cargo test -p nidoo-climate heaters::` — expected: FAIL to compile.
- [ ] **Step 3: Implement `drive`.** Order: not chosen → `Off` unless hold + `bridge` + currently on; `target` `None` (setpoint off) → `Off`; thermostat: no reading → `Off`, then the band with "unchanged inside" (`on: None` counts as off), then `min_on_min` only against the band's own off; a wanted `On` on a device that is off with `off_since` passes through the cool-down → `Wait(off_since + min_off_min*60)`. A device already on never waits.
- [ ] **Step 4: Run** `cargo test -p nidoo-climate` — expected: PASS.
- [ ] **Step 5: Commit** `feat(climate): thermostat and hold drive for on/off heaters`

---

### Task 4: Who heats — the decision post-pass

**Files:**
- Modify: `crates/nidoo-climate/src/heaters.rs`, `crates/nidoo-climate/src/decide.rs`
- Modify: every `ZoneInputs { … }` literal (the `Case` builder in `decide.rs` tests, `crates/nidoo/src/climate/inputs.rs`), adding `heaters: &[]` / the real list
- Test: unit tests in `decide.rs`, using the `Case` builder

**Interfaces:**
- Consumes: Task 1-3.
- Produces:
  ```rust
  // decide.rs
  pub struct ZoneInputs<'a> { …, /// The zone's on/off heaters, enabled or not, reachable or not.
                              pub heaters: &'a [HeaterDevice] }
  // heaters.rs
  /// Rewrite `d` for a zone with on/off heaters: who heats, the switches' targets, the reason.
  /// No-op unless `d.mode == Some(Mode::Heat)`, the zone is not off, and the zone has an
  /// enabled heater or a `heated_by` anywhere.
  pub(crate) fn apply(inp: &ZoneInputs, d: &mut Decision);
  ```
  `decide` calls `heaters::apply(inp, &mut d)` last, before the "thermometer silent" suffix. `Case` gains `fn heater(self, h: Heater, dev: HeaterDevice) -> Self`.

  Rules `apply` implements (spec "Who heats", "Fill-in", "Edges"):
  - chosen ids by `d.source`: Schedule / Empty / Boost / Override → `heat.heated_by_at(local_now)`, else `settings.heated_by`, else all climate units; Eco / Away / Holiday → the room default, minus hold heaters; Safety+Frost → all climate units + thermostat heaters; Safety+Overheat, Safety+Stale, pending → no on/off heater (climate units untouched); Idle → untouched.
  - ids that are neither an enabled heater nor a unit in `inp.devices`' zone list are dropped and named in the reason (`"; ignoring unknown heater {id}"`); a list emptied that way falls to the room default, then to the climate units. A disabled heater is dropped the same way, without the reason suffix.
  - unchosen, unheld climate units get `device_target(Setpoint::Off, Mode::Heat, caps)`; chosen ones keep what `decide` gave them.
  - each enabled heater: per-device `Override` live for it wins (`setpoint: Some(Off)` → held off, `mode: Some(Heat)` → held on, both `held: true`, an on still passes the cool-down); else `drive(...)` with `target` = `d.setpoint` when it is a temperature, `reading` = `d.reading` when the reading is fresh, `bridge` = `next_start_of(id, …, min_off_min)`.
  - an unavailable heater gets no entry in `d.entities`.
  - `Want::On` → `DeviceTarget{setpoint: None, mode: "heat", held}`; `Off` → `mode: "off"`; `Wait(t)` → `mode: "off"` plus `d.heaters[id].wait_until = Some(t)`. `d.heaters[id]` always carries the device's `off_since` / `on_since`.
  - fill-in: `settings.fill_in`, a chosen heater is `Wait` or unavailable, no climate unit chosen → every unheld climate unit gets `device_target(d.setpoint, Heat, caps)`.
  - reason suffixes, exact: `"; {id} cooling down until {HH:MM}"`, `"; {id} unreachable"`, `"; the climate units heat meanwhile"`, `"; nothing heats the room"` (a chosen heater cannot run and nothing fills in), `"; {id} stays on until its next block"` (bridge). `{id}` is the entity id; the hub and the web replace ids by names when they show a reason (existing behaviour for device ids, see `zone_display_name` neighbours; if no such replacement exists for reasons, leave the id).

- [ ] **Step 1: Write the failing tests** (one per row; `stove()` = hold, `min_off_min: 15`; `rad()` = thermostat; the Case's `trv` is the climate unit)

```rust
#[test] fn zone_without_heaters_is_unchanged() { /* every field of d equals the pre-change decision for Case::new() */ }
#[test] fn block_names_the_stove_unit_goes_off() {
    let d = Case::new().heater(stove(), off()).heat_block_by(9, 18, 20.0, &["stove"]).at_hm(10, 0).run();
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
}
#[test] fn base_uses_the_room_default() { /* heated_by Some(["rad"]) at 20:00: rad driven, trv off */ }
#[test] fn no_default_means_the_units_heat_and_heaters_are_off() { /* rad present, no heated_by: trv 15.0 heat, rad off */ }
#[test] fn eco_drops_hold_heaters() { /* room default ["stove","trv"], away: stove off, trv eco target */ }
#[test] fn frost_turns_thermostat_heaters_on_not_hold() { /* reading 5.0: rad heat, stove off, trv frost target */ }
#[test] fn overheat_turns_every_heater_off() { /* reading 31.0, both on: both "off" */ }
#[test] fn stale_reading_turns_heaters_off() { /* Reading::Stale: rad off, stove off */ }
#[test] fn pending_zone_keeps_heaters_off() {}
#[test] fn cool_mode_leaves_heaters_out() { let d = …mode(Mode::Cool).run(); assert!(!d.entities.contains_key("stove")); assert!(d.heaters.is_empty()); }
#[test] fn zone_off_leaves_heaters_out() {}
#[test] fn boost_uses_what_the_schedule_would_choose() {}
#[test] fn cooling_down_stove_waits_and_the_unit_fills_in() {
    let d = Case::new().heater(stove(), off_since(NOW - 300)).heat_block_by(9, 18, 20.0, &["stove"]).at_hm(10, 0).run();
    assert_eq!(d.entities["stove"].mode, "off");
    assert_eq!(d.heaters["stove"].wait_until, Some(NOW + 600));
    assert_eq!(d.entities["trv"], target(Some(20.0), "heat", false));
    assert!(d.reason.contains("stove cooling down until") && d.reason.contains("the climate units heat meanwhile"));
}
#[test] fn fill_in_off_says_nothing_heats() { /* fill_in false: trv "off", reason contains "nothing heats the room" */ }
#[test] fn unreachable_heater_fills_in() { /* available false: no "stove" entry, trv heats, reason "stove unreachable" */ }
#[test] fn running_stove_never_fills_in() { /* stove on, reading 12.0: trv off */ }
#[test] fn cool_down_blocks_a_boost_and_frost() {}
#[test] fn hold_bridges_a_gap_shorter_than_its_cool_down() { /* blocks 9-12 and 12:10-14, at 12:05, stove on: stays "heat", reason "stays on until its next block" */ }
#[test] fn hand_off_holds_the_heater() { /* dev_ov("stove", Off): target(None,"off",true) */ }
#[test] fn hand_on_holds_but_respects_cool_down() {}
#[test] fn unknown_heated_by_id_falls_back() { /* block by ["gone"]: room default applies, reason "ignoring unknown heater gone" */ }
#[test] fn disabled_heater_is_not_chosen() { /* block by ["stove"], stove enabled:false: trv heats 20.0, no "stove" entry */ }
#[test] fn correction_never_touches_a_heater() { /* feedback offset -1.0: rad target has setpoint None */ }
```
Each body builds the case and asserts the exact `entities` entries named in its comment.

- [ ] **Step 2: Run** `cargo test -p nidoo-climate decide::` — expected: FAIL to compile.
- [ ] **Step 3: Implement** `apply` and the call in `decide`; extend `Case`.
- [ ] **Step 4: Run** `cargo test -p nidoo-climate` and `cargo check --workspace --tests` — expected: PASS.
- [ ] **Step 5: Commit** `feat(climate): heated-by per block, cool-down and fill-in in the decision`

---

### Task 5: Learner keeps `heated_by`

**Files:**
- Modify: `crates/nidoo-climate/src/learn.rs`
- Test: `crates/nidoo-climate/tests/learn_scenarios.rs`, unit tests in `learn.rs`

**Interfaces:**
- Produces: `fn inherit_heated_by(new: &mut [Block], approved: &[Block])` (private), called by `revise`; `fingerprint` clears `heated_by` as it clears `evidence`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn fingerprint_ignores_heated_by() {
    let (a, mut b) = (rules_9_18(), rules_9_18());
    b.heat.as_mut().unwrap().blocks[0].heated_by = Some(vec!["stove".into()]);
    assert_eq!(fingerprint(&a), fingerprint(&b));
}
#[test] fn revised_block_inherits_from_the_approved_block_it_overlaps_most() {
    // approved: 09:00-18:00 by ["stove"], 18:00-22:00 by ["rad"]; learned: 10:00-19:00
    // → inherits ["stove"] (8 h overlap vs 1 h)
}
#[test] fn revised_block_with_no_overlap_inherits_nothing() {}
#[test] fn rules_differing_only_by_heated_by_do_not_differ() { assert!(!differs(&a, &b)); }
```
- [ ] **Step 2: Run** `cargo test -p nidoo-climate learn` — expected: FAIL.
- [ ] **Step 3: Implement.** Overlap = minutes shared on shared days, summed over days; ties go to the earlier approved block.
- [ ] **Step 4: Run** `cargo test -p nidoo-climate` — expected: PASS.
- [ ] **Step 5: Commit** `feat(climate): the learner keeps who heats each block`

---

### Task 6: Hub inputs and save-time validation

**Files:**
- Modify: `crates/nidoo/src/climate/inputs.rs`, `crates/nidoo/src/climate/hub.rs`
- Test: `crates/nidoo/tests/climate_hub.rs` (add a switch discovery fixture from `crates/nidoo-core/tests/fixtures/discovery/switch__0x00124b0024bc601e__switch.json` and a `place()` for it)

**Interfaces:**
- Consumes: `HeaterDevice`, `ZoneSettings::validate_heated_by`.
- Produces:
  ```rust
  // inputs.rs
  /// The zone's on/off heaters as they are now; one whose entity is missing or not a switch is skipped.
  pub(crate) fn zone_heaters(hub: &Hub, settings: &ZoneSettings, clocks: &HeaterClocks) -> Vec<HeaterDevice>;
  // hub.rs
  /// entity id -> zone, for every heater of every zone.
  pub(crate) fn heater_claims(&self) -> BTreeMap<String, String>;
  ```
  `OwnedZoneInputs` gains `heaters: Vec<HeaterDevice>`. `HeaterClocks` is `BTreeMap<String, HeaterRun>` (Task 7 owns filling it; here it is read, empty is fine). `available` = entity present, state `OnOff`, availability not `Offline`.
  `Hub::set_climate_zone` additionally rejects: a heater that is not a `Kind::Switch` entity (`"{id} is not a switch"`), one that is a heater of another zone (`"{id} already heats {zone}"`), and `validate_heated_by(&zone_entity_ids)` errors.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(start_paused = true)] async fn saving_a_non_switch_heater_is_rejected() { /* heater = the Daikin id → Err contains "is not a switch" */ }
#[tokio::test(start_paused = true)] async fn a_heater_cannot_heat_two_zones() { /* → "already heats" */ }
#[tokio::test(start_paused = true)] async fn a_block_naming_an_unknown_heater_is_rejected() { /* → "heat block 0" */ }
#[tokio::test(start_paused = true)] async fn a_heater_from_another_area_is_accepted() { /* switch placed in "garage", zone "living" → Ok */ }
#[tokio::test(start_paused = true)] async fn missing_heater_entity_is_skipped() {
    // settings name a heater whose discovery is then cleared: the next pass still publishes a decision,
    // with no entry for it in `entities`
}
```
- [ ] **Step 2: Run** `cargo test -p nidoo --test climate_hub heater` — expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p nidoo --test climate_hub` — expected: PASS.
- [ ] **Step 5: Commit** `feat(climate): the hub reads and validates a zone's on/off heaters`

---

### Task 7: Hub engine — commands, clocks, hand flips, last off

**Files:**
- Create: `crates/nidoo/src/climate/heater_io.rs` (declare in `climate/mod.rs`)
- Modify: `crates/nidoo/src/climate/engine.rs` (`observe_climate` caller, `evaluate_zone`, `command_zone`)
- Test: `crates/nidoo/tests/climate_hub.rs` (extend `Sim` to echo `…/watering_stove/set` on the state topic)

**Interfaces:**
- Consumes: Task 4 decision shape, Task 6 `zone_heaters`.
- Produces (all `impl Hub`, in `heater_io.rs`):
  ```rust
  /// A switch that is some zone's heater changed state: update its clock, and record a hand
  /// change when the engine did not ask for it.
  pub fn observe_heater(&self, id: &str, on: bool, now: u64);
  /// Send on/off for the switch targets of `decision`; same pause, resend spacing and failure
  /// counting as climate units.
  pub(crate) async fn command_heaters(self: &Arc<Self>, zone: &str, decision: &Decision, now: u64);
  /// The zone stopped deciding in heat: one off to each heater the previous decision had on
  /// and unheld, that is still on.
  pub(crate) async fn release_heaters(self: &Arc<Self>, zone: &str, previous: &Decision, now: u64);
  ```
  Runtime: `heater_clocks: BTreeMap<String, HeaterRun>` and `heater_sent: BTreeMap<String, (bool /*on*/, u64 /*at*/)>` in the climate runtime.
  Rules:
  - clocks: an observed off sets `off_since = now` and clears `on_since`; an observed on does the reverse. On the first pass after start, a zone's clocks are seeded from its retained decision's `heaters`; a heater with no entry there, seen off, gets `off_since = now`.
  - hand change: the new state differs from `heater_sent[id].0`, or nothing was sent, **and** no command for it is in flight, **and** the last send is older than `UNCONFIRMED_S`. Only for an `active` zone in heat. It writes `Override{entity: Some(id), setpoint: Some(Setpoint::Off) | None, mode: None | Some(Mode::Heat), until: hand_hold_until(zone, now), origin: Hand}` through `update_overrides`.
  - commands: `Command::TurnOn{brightness: None, color: None, effect: None}` / `Command::TurnOff` when the target's mode differs from the switch state; never for a `held` target; never for a switch absent from `decision.entities`.
  - `release_heaters` runs in `evaluate_zone` when the previous decision's mode was heat with heaters and the new decision is not heat or the zone is off.

- [ ] **Step 1: Write the failing tests**

```rust
async fn stove_block_switches_the_plug_on_and_off() { /* 09:00 pass → "ON" published once; 18:00 pass → "OFF" */ }
async fn nothing_is_published_to_the_plug_in_cool_mode() { /* season cool, 3 passes → 0 publishes on …/watering_stove/set */ }
async fn leaving_heat_sends_one_off_then_nothing() { /* on in heat; season → cool; pass → one "OFF"; restart hub in cool; 3 passes → nothing more */ }
async fn leaving_heat_leaves_a_hand_held_heater_alone() {}
async fn disabling_a_heater_sends_nothing() {}
async fn restart_inside_cooldown_does_not_switch_on() { /* off at t0, restart at t0+5 min inside a stove block → no "ON" before t0+15 min, "ON" on the first pass after */ }
async fn restart_keeps_running_heater_on() { /* on, restart inside the block → no publish at all */ }
async fn fresh_hub_counts_an_off_heater_as_off_since_now() { /* no retained decision, block active → first "ON" 15 min after start */ }
async fn hand_off_becomes_a_hold_and_starts_the_cool_down() { /* publish OFF on the state topic from outside → overrides doc has the entry; engine does not re-send ON; after the hold ends, ON waits for off_since+15 min */ }
async fn engine_echo_is_not_a_hand_change() { /* after the engine's ON echo, overrides doc stays empty */ }
async fn failing_plug_commands_pause_like_a_unit() { /* no echo ×3 → paused, decision reason carries "unreachable", unit fills in */ }
```
All `#[tokio::test(start_paused = true)]`, on the living zone with the Daikin fixture plus the plug.

- [ ] **Step 2: Run** `cargo test -p nidoo --test climate_hub -- stove plug heater` — expected: FAIL.
- [ ] **Step 3: Implement** `heater_io.rs` and the three call sites. Where the hub ingests a state change and calls `observe_climate`, also call `observe_heater` for an `OnOff` state whose id is in `heater_claims()`.
- [ ] **Step 4: Run** `cargo test -p nidoo` — expected: PASS (DB tests skip without Postgres).
- [ ] **Step 5: Commit** `feat(climate): the engine switches on/off heaters`

---

### Task 8: Web — Heaters section in the room settings

**Files:**
- Create: `crates/nidoo-web/src/climate/heaters_card.rs` (declare in `climate/mod.rs`)
- Modify: `crates/nidoo-web/src/climate/room_settings.rs` (mount it above the thermometer field), `crates/nidoo-web/src/entity_picker/needs.rs`, `crates/nidoo-web/src/i18n/climate.rs`
- Test: `crates/nidoo-web/tests/climate_rooms.rs`, unit tests in `needs.rs` and `heaters_card.rs`

**Interfaces:**
- Consumes: `ZoneSettings::{heaters, heated_by, fill_in}`.
- Produces:
  ```rust
  // needs.rs
  pub fn is_switch(e: &EntityView) -> bool;          // Kind::Switch, primary category, not group_*
  // heaters_card.rs
  #[component] pub fn HeatersCard(settings: Signal<ZoneSettings>, units: Signal<Vec<(String, String)>> /* id, label */,
                                  on_change: Callback<ZoneSettings>) -> impl IntoView;
  /// Remove a heater and every reference to it (room default, heat blocks); a list emptied that way becomes None.
  pub fn remove_heater(s: &mut ZoneSettings, id: &str);
  /// id -> label for the room's units and heaters, units first.
  pub fn heater_labels(settings: &ZoneSettings, units: &[(String, String)], name_of: impl Fn(&str) -> String) -> Vec<(String, String)>;
  ```
  Layout (spec "Screens"): one read-only row per climate unit; one row per heater with a drive select (`Thermostat` / `Stays on`), a cool-down number input in minutes (0..=240), an enabled switch and ✕; "Add a heater…" opening the entity picker with `pool = keep(is_switch)`, all rooms; "Default heater" chips (none ticked = `heated_by: None`, shown as the climate units ticked); the fill-in checkbox. Saving goes through the existing whole-`ZoneSettings` `api::put_zone` path, and a `400` shows the hub's message as the other settings cards do.
  New `ClimateText` entries (en / fr): `Heaters` / `Chauffages`; `ClimateUnit` "climate unit" / "climatiseur"; `AddHeater` "Add a heater…" / "Ajouter un chauffage…"; `DriveThermostat` "Thermostat" / "Thermostat"; `DriveHold` "Stays on" / "Reste allumé"; `CoolDown` "Cool-down" / "Refroidissement"; `DefaultHeater` "Default heater" / "Chauffage par défaut"; `FillIn` "The climate units take over while a heater cannot run" / "Les climatiseurs prennent le relais quand un chauffage ne peut pas fonctionner"; `HeatedBy` "Heated by" / "Chauffé par"; `RoomDefault` "Room default" / "Défaut de la pièce". Add each to `ALL`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn is_switch_keeps_switches_only() { /* switch yes; light, binary_sensor, group_ switch no */ }
#[test] fn remove_heater_clears_every_reference() {
    // heaters [stove, rad], heated_by Some([stove]), block0 by [stove, rad], block1 by [stove]
    // after remove_heater("stove"): heaters [rad], heated_by None, block0 by [rad], block1 None
}
#[test] fn heater_labels_lists_units_first() {}
```
plus, in `tests/climate_rooms.rs`, a render test: a zone with one heater shows the "Heaters" heading and the heater's name; a zone with none shows the heading and "Add a heater…".

- [ ] **Step 2: Run** `cargo test -p nidoo-web heater` — expected: FAIL.
- [ ] **Step 3: Implement.** Brace closure attributes on self-closing tags (`on:click={move |_| …} />`), or `view!` misparses.
- [ ] **Step 4: Run** `cargo test -p nidoo-web` and `cargo check -p nidoo-web --target wasm32-unknown-unknown` — expected: PASS.
- [ ] **Step 5: Commit** `feat(web): add on/off heaters in the room settings`

---

### Task 9: Web — "Heated by" in the schedule

**Files:**
- Modify: `crates/nidoo-web/src/climate/timeline.rs` (`BlockEditor`, `WeekTimeline`, `add_block`), `crates/nidoo-web/src/climate/proposal.rs` (`RulesTimeline`), and their callers in `climate/mod.rs`
- Test: `crates/nidoo-web/tests/climate_timeline.rs`, unit tests in `timeline.rs`

**Interfaces:**
- Consumes: `heater_labels` (Task 8), `ClimateText::{HeatedBy, RoomDefault}`.
- Produces: a new prop on `RulesTimeline`, `WeekTimeline` and `BlockEditor`: `heaters: Signal<Vec<(String, String)>>` (id, label; empty = feature hidden) and
  ```rust
  /// Tick or untick `id` on block `i`; an emptied list becomes None (room default).
  pub fn toggle_heated_by(rules: &ModeRules, i: usize, id: &str) -> ModeRules;
  /// "Stove" / "Stove + Heat pump", or None for the room default.
  pub fn heated_by_label(block: &Block, heaters: &[(String, String)]) -> Option<String>;
  ```
  The row sits after the Off checkbox, only in heat rules and only when the zone has at least one on/off heater: a "Room default" chip (selected when `heated_by` is `None`) and one chip per entry of `heaters`. A block with a label shows it under its temperature in the week view. `add_block` creates `heated_by: None`. The proposal card passes the same list, so an edited proposal keeps and shows `heated_by`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn toggle_heated_by_adds_then_removes() {
    let r = toggle_heated_by(&one_block(), 0, "stove");
    assert_eq!(r.blocks[0].heated_by, Some(vec!["stove".to_string()]));
    assert_eq!(toggle_heated_by(&r, 0, "stove").blocks[0].heated_by, None);
}
#[test] fn heated_by_label_joins_names_and_skips_unknown_ids() {}
```
plus render tests in `tests/climate_timeline.rs`: the row is absent with an empty `heaters` list and in cool rules; present with one heater; a block by the stove shows "Stove" in the week view.

- [ ] **Step 2: Run** `cargo test -p nidoo-web --test climate_timeline` — expected: FAIL. (Run one test file at a time; never `wasm-pack test`.)
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p nidoo-web` and the wasm check — expected: PASS.
- [ ] **Step 5: Commit** `feat(web): choose who heats each schedule block`

---

### Task 10: Documentation

**Files:**
- Modify: `docs/users/climate.md` — new "Heaters" section after "Zones" (adding a heater, the two drives, the cool-down, Default heater, fill-in, hand changes, what happens outside the heating season, the "not detected: a stove that went out" limit); update "How a zone is decided" with who heats per source; one line under "The learner" (it never proposes Heated by and keeps yours); settings field names (`heaters`, `heated_by`, `fill_in`, block `heated_by`).
- Modify: `docs/reference/` — the pages that list zone settings and decision fields for REST and MCP (`grep -rln "manual_hold_max_min" docs/reference`).
- Modify: `docs/superpowers/specs/2026-10-22-climate-engine-design.md` — at the two "only `climate` entities" statements, add "(lifted for heating by `2026-11-26-climate-onoff-heaters-design.md`)".

- [ ] **Step 1:** Write the sections. User docs describe the screens and behaviour in plain words; no entity ids from a real house.
- [ ] **Step 2: Run** the website's docs check if the repo has one (`grep -n "\"build\"\|\"lint\"" website/package.json`) and `tools/brand-check.sh` if present — expected: PASS.
- [ ] **Step 3: Commit** `docs(climate): on/off heaters and heated by`

---

## Final verification

- [ ] `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace` (with an empty `crates/nidoo-web/dist`)
- [ ] `cargo check -p nidoo-web --target wasm32-unknown-unknown`
- [ ] `cd satellite/mac && cargo check` only if a `nidoo-satellite` API changed (it should not).
