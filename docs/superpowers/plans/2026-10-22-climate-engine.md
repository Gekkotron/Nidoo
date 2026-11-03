# Climate Engine v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An optional hub feature (`[climate] enabled = false` by default) that learns each room's heating/cooling rules from TimescaleDB history, asks the user to approve them over push and the web, keeps approved rooms on them, and never lets a room freeze or overheat.

**Architecture:** A new pure crate `oikos-climate` (document types, topics, schedule, decision tree, hand-change ledger, learner; no I/O, wasm-safe so the web can use its types) and a new hub module `crates/oikos-os/src/climate/` that wires it to retained MQTT documents, the registry, `send_command`, history and push. The web gets a Climate screen. Everything except the on/off switch and the topic prefix is retained JSON under `<prefix>` (default `oikos/climate`).

**Tech Stack:** Rust 2024 workspace, serde/serde_json, chrono (no clock feature in the pure crate) + chrono-tz in the hub, tokio, rumqttc, axum, rmcp, sqlx + TimescaleDB, Leptos 0.8 (CSR/wasm).

**Spec:** `docs/superpowers/specs/2026-10-22-climate-engine-design.md` (approved 2026-10-22). Read it before any task; section numbers below (§n) refer to it.

## Global Constraints

- Naming: `climate` everywhere — crate `oikos-climate`, config `[climate]`, UI "Climate". Never "Hestia" in code, docs or UI.
- `[climate] enabled` defaults to `false`; `prefix` defaults to `"oikos/climate"`. Nothing else climate-related goes in `oikos.toml`.
- Generic code: no room names, house values, vendor payload keys or French strings. The defaults of §4.3/§4.4 are the only numbers in code: `heat_below` 16, `cool_above` 24, `away_grace` 15 min, `frost` 7, `overheat` 30, `overcool` 16, `stale_after` 60 min, `learn_days` 28, `learn_min_days` 10, `learn_support` 0.6, `learn_at` 03:30, away setpoints heat 16 / cool 28, `manual_hold_max` 240 min, frost heat target `frost + 2`.
- No climate topic ends in `/config`; the hub subscribes only to `<prefix>/house` and `<prefix>/zone/+/+`, never `<prefix>/#`.
- Every publish QoS 1 (the hub's default); every §4.2 document retained. An empty retained payload deletes.
- Disabled = no subscriptions, no task, HTTP 503 and MCP "climate disabled" (same wording/shape as `require_groups()` in `crates/oikos-os/src/hub.rs:1359`).
- The engine keeps running when history is off or the database is unreachable (learner paused, decisions still made).
- `oikos-climate` depends only on `serde`, `serde_json`, `chrono` (workspace, no `clock`) and optionally `schemars` (feature `schemars`, like `oikos-core`). It must build for `wasm32-unknown-unknown`.
- Timestamps in climate documents are Unix seconds (`u64`), like the hub's `clock::now()`; the spec's RFC 3339 examples are illustrative. Local wall-clock math uses `chrono::NaiveDateTime` produced by the hub from `Hub::zone()`.
- Commits: identity `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`, message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and the task's tests before each commit. Cargo builds land in the shared `$CARGO_TARGET_DIR`; do not add per-repo cache config.

## Review Focus

1. **A climate device that disappears or is renamed while a zone is active** (entity removed from the registry, or moved to another area): the zone keeps working with the remaining devices, the ledger/overrides for the gone id are dropped, no panic, no command to an unknown id. → test `zone_drops_removed_entity` in Task 8.
2. **A retained zone document for an area that no longer exists, or invalid JSON on any climate topic**: kept as invalid with its reason, shown in `GET /api/v1/climate`, never crashes ingest. → tests `store_keeps_invalid_with_reason` (Task 6) and `zone_without_area_is_listed_orphan` (Task 6).
3. **DST transitions and blocks ending at `24:00`** (schedule lookup on the spring-forward/fall-back day, `next_change` across midnight and across the week end): no panic, no skipped block. → tests `at_handles_24_00_end`, `next_change_wraps_week` (Task 2) and `hold_until_across_dst` (Task 8, hub side where the zone is applied).
4. **The push reply arrives twice, late, or for a replaced proposal** (user taps Approve on an old notification): only a reply whose proposal id matches the pending one acts; others are ignored without error. → test `stale_reply_is_ignored` (Task 9).
5. **The broker link is down when the engine wants to command** (`Hub::publish` fails fast): counted as a failure, no ledger entry, no hand-change false positive when the device later echoes its old value. → test `failed_publish_is_not_recorded` (Task 8).

---

## File map

**New crate `crates/oikos-climate/`**
- `Cargo.toml`, `src/lib.rs` — re-exports.
- `src/doc.rs` — document types + `validate()` (Task 1).
- `src/topics.rs` — topic builders/parser (Task 1).
- `src/schedule.rs` — `ModeRules::at`, `next_change`, `season_mode`, `room_reading` (Task 2).
- `src/decide.rs` — `decide()`, `fit()`, `SafetyMemory` (Task 3).
- `src/ledger.rs` — `Ledger`, `Observed`, `hold_until` (Task 4).
- `src/learn.rs` — timeline, `learn()`, `differs()`, `diff()`, `fingerprint()` (Task 5).
- `src/view.rs` — `ClimateView`, `ZoneView`, `LearnStatus` (Task 6).

**Hub `crates/oikos-os/`**
- `src/config.rs` — `ClimateConfig` (Task 6).
- `src/climate/mod.rs`, `store.rs`, `hub.rs` (Task 6); `inputs.rs`, `engine.rs`, `alerts.rs` (Task 8); `learning.rs`, `proposals.rs` (Task 9).
- `src/hub.rs` — field, construction, `extra_filters`, `ingest_message` routing, `require_climate` (Task 6).
- `migrations/0002_climate.sql`, `src/history/db.rs`, `src/history/climate.rs` (Task 7).
- `src/main.rs` — spawn/abort the climate task (Task 8).
- `src/http/api.rs`, `src/mcp/climate_tools.rs`, `src/mcp/params.rs`, `src/mcp/mod.rs` (Task 10).
- Tests: `tests/climate_hub.rs` (Tasks 6, 8, 9), `tests/history_db.rs` (Task 7), `tests/http.rs`, `tests/mcp.rs` (Task 10).

**API/Web**
- `crates/oikos-api/src/events.rs` — `WsEvent::ClimateChanged` (Task 6).
- `crates/oikos-web/src/api/climate.rs`, `src/screens/climate.rs`, `src/climate/{mod.rs,timeline.rs,proposal.rs,zone_card.rs,settings_sheet.rs}`, `src/app.rs` (Tasks 11–12).

**Docs:** `oikos.example.toml` (Task 6), `README.md` feature list + `TODOS.md` (Task 10).

---

### Task 1: `oikos-climate` crate — documents and topics

**Files:**
- Create: `crates/oikos-climate/Cargo.toml`, `src/lib.rs`, `src/doc.rs`, `src/topics.rs`
- Test: inline `#[cfg(test)]` modules in `doc.rs`, `topics.rs`; `crates/oikos-climate/tests/not_discovery.rs`

**Interfaces:**
- Produces (all `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]`, `#[serde(rename_all = "snake_case")]`, `#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]`; structs `#[serde(default)]` where they have defaults):
  - `enum Mode { Heat, Cool }`; `enum Season { Auto, Heat, Cool, Off }` (default `Auto`); `enum ZoneState { Off, Pending, Active }` (default `Pending`); `enum Day { Mon, Tue, Wed, Thu, Fri, Sat, Sun }` with `Day::from_chrono(chrono::Weekday)`.
  - `enum Setpoint { Temp(f64), Off }` — serializes as a JSON number or the string `"off"` (custom impl or `#[serde(untagged)]` with a unit-string helper).
  - `struct HhMm(u16)` — minutes since midnight, `0..=1440`; serde as `"HH:MM"`, `"24:00"` allowed. `HhMm::parse(&str) -> Option<HhMm>`.
  - `struct Evidence { agree: u32, of: u32 }`.
  - `struct Block { days: Vec<Day>, from: HhMm, to: HhMm, setpoint: Setpoint, #[serde(skip_serializing_if = "Option::is_none")] evidence: Option<Evidence> }`.
  - `struct ModeRules { base: Setpoint, blocks: Vec<Block> }`; `struct Rules { heat: Option<ModeRules>, cool: Option<ModeRules> }` with `Rules::get(Mode) -> Option<&ModeRules>`, `Rules::is_empty()`.
  - `struct HouseSettings { season: Season, outdoor_entity: Option<String>, heat_below: f64, cool_above: f64, presence: Vec<String>, away_grace_min: u32, holiday_entity: Option<String>, frost: f64, overheat: f64, overcool: f64, stale_after_min: u32, learn_days: u32, learn_min_days: u32, learn_support: f64, learn_at: HhMm }` — `Default` = §4.3 values.
  - `struct AwaySetpoints { heat: f64, cool: f64 }` (16/28); `struct Rejection { fingerprint: String, until: u64 }`.
  - `struct ZoneSettings { state: ZoneState, entities: Option<Vec<String>>, sensor: Option<String>, away_setpoint: AwaySetpoints, manual_hold_max_min: u32, rules: Rules, approved_at: Option<u64>, approved_proposal: Option<String>, rejected: Vec<Rejection> }` — `Default`: `Pending`, `None`, `None`, 16/28, 240, empty, `None`, `None`, empty.
  - `enum OverrideOrigin { Hand, User }`; `struct Override { entity: Option<String>, setpoint: Option<Setpoint>, mode: Option<Mode>, until: u64, origin: OverrideOrigin }`. The override document is `Vec<Override>`.
  - `enum Source { Safety, Override, Holiday, Away, Schedule, Idle }`.
  - `struct DeviceTarget { setpoint: Option<f64>, mode: String, held: bool }` (`mode` is the device's hvac mode string, e.g. `"heat"`, `"off"`).
  - `struct Decision { zone: String, mode: Option<Mode>, setpoint: Option<Setpoint>, source: Source, reason: String, reading: Option<f64>, reading_age_s: Option<u64>, outdoor_mean: Option<f64>, home: bool, applied: bool, at: u64, entities: BTreeMap<String, DeviceTarget> }` with `Decision::same_outcome(&self, other) -> bool` (equal ignoring `at`, `reading`, `reading_age_s`, `outdoor_mean`).
  - `enum ProposalKind { Initial, Change }`; `struct Window { from: u64, to: u64, days: u32 }`; `struct Proposal { id: String, zone: String, kind: ProposalKind, created: u64, window: Window, rules: Rules, changes: Vec<String>, fingerprint: String }`.
  - `fn HouseSettings::validate(&self) -> Result<(), String>`, `fn ZoneSettings::validate(&self) -> Result<(), String>`, `fn Rules::validate(&self) -> Result<(), String>`.
  - `topics.rs`: `enum ZoneDoc { Settings, Proposal, Override, Decision }` (segments `settings`, `proposal`, `override`, `decision`); `enum ClimateTopic { House, Zone { zone: String, doc: ZoneDoc } }`; `fn house_topic(prefix: &str) -> String`; `fn zone_topic(prefix: &str, zone: &str, doc: ZoneDoc) -> String`; `fn filters(prefix: &str) -> [String; 2]` (`"<p>/house"`, `"<p>/zone/+/+"`); `fn parse(prefix: &str, topic: &str) -> Option<ClimateTopic>`. Prefix is used with trailing `/` trimmed.

- [ ] **Step 1: Create the crate.** `Cargo.toml` uses the workspace package fields (copy `crates/oikos-frigate/Cargo.toml`), deps `serde`, `serde_json`, `chrono` (workspace), `schemars` optional, feature `schemars = ["dep:schemars"]`; dev-dependency `oikos-core = { path = "../oikos-core" }` (for the discovery test only).

- [ ] **Step 2: Write the failing tests** in `doc.rs`:

```rust
#[test] fn setpoint_serde() {
    assert_eq!(serde_json::to_string(&Setpoint::Off).unwrap(), "\"off\"");
    assert_eq!(serde_json::from_str::<Setpoint>("20.5").unwrap(), Setpoint::Temp(20.5));
}
#[test] fn hhmm_serde_and_bounds() {
    assert_eq!(serde_json::from_str::<HhMm>("\"07:30\"").unwrap(), HhMm(450));
    assert_eq!(serde_json::from_str::<HhMm>("\"24:00\"").unwrap(), HhMm(1440));
    assert!(serde_json::from_str::<HhMm>("\"24:01\"").is_err());
    assert!(serde_json::from_str::<HhMm>("\"7:5\"").is_err());
}
#[test] fn house_defaults_match_spec() {
    let h: HouseSettings = serde_json::from_str("{}").unwrap();
    assert_eq!((h.frost, h.overheat, h.overcool, h.stale_after_min), (7.0, 30.0, 16.0, 60));
    assert_eq!((h.heat_below, h.cool_above, h.away_grace_min), (16.0, 24.0, 15));
    assert_eq!((h.learn_days, h.learn_min_days, h.learn_support, h.learn_at), (28, 10, 0.6, HhMm(210)));
    assert_eq!(h.season, Season::Auto);
}
#[test] fn zone_defaults_match_spec() {
    let z: ZoneSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(z.state, ZoneState::Pending);
    assert_eq!((z.away_setpoint.heat, z.away_setpoint.cool, z.manual_hold_max_min), (16.0, 28.0, 240));
}
#[test] fn rules_validate_rejects_bad_blocks() {
    // from >= to, empty days, overlap on a shared day, setpoint outside 0..=40
    // each case: Rules { heat: Some(ModeRules{..}), cool: None }.validate().is_err()
}
#[test] fn rules_validate_accepts_block_to_24_00() { /* {from 22:00, to 24:00} ok */ }
#[test] fn house_validate_rejects() {
    // frost >= overheat; overcool >= cool_above is fine; learn_support outside (0,1]; learn_min_days > learn_days; stale_after_min == 0
}
```

`topics.rs` tests:

```rust
#[test] fn builds_and_parses() {
    assert_eq!(house_topic("oikos/climate"), "oikos/climate/house");
    assert_eq!(zone_topic("oikos/climate/", "living", ZoneDoc::Decision), "oikos/climate/zone/living/decision");
    assert_eq!(parse("oikos/climate", "oikos/climate/zone/living/override"),
        Some(ClimateTopic::Zone { zone: "living".into(), doc: ZoneDoc::Override }));
    assert_eq!(parse("oikos/climate", "oikos/climate/group/g1/config"), None);
    assert_eq!(parse("oikos/climate", "oikos/climate/zone/living/config"), None);
    assert_eq!(filters("oikos/climate"), ["oikos/climate/house".to_owned(), "oikos/climate/zone/+/+".to_owned()]);
}
```

`tests/not_discovery.rs`:

```rust
#[test] fn no_climate_topic_classifies_as_discovery() {
    use oikos_core::discovery::{classify, TopicClass};
    let prefixes = vec!["oikos".to_owned()];
    let mut topics = vec![oikos_climate::topics::house_topic("oikos/climate")];
    for doc in [ZoneDoc::Settings, ZoneDoc::Proposal, ZoneDoc::Override, ZoneDoc::Decision] {
        topics.push(oikos_climate::topics::zone_topic("oikos/climate", "living", doc));
    }
    for t in topics { assert_eq!(classify(&t, &prefixes, &[]), TopicClass::Other, "{t}"); }
}
```

- [ ] **Step 3: Run** `cargo test -p oikos-climate` — expect compile failures.
- [ ] **Step 4: Implement** the types, serde, defaults, `validate()` and topics. Validation rules: setpoints in `0.0..=40.0`; `from < to`; `days` non-empty; no two blocks of one `ModeRules` overlap on a shared day; `frost < overheat`; `0 < learn_support <= 1`; `learn_min_days <= learn_days`; `stale_after_min > 0`; `ZoneSettings::validate` calls `rules.validate()` and requires `manual_hold_max_min > 0`.
- [ ] **Step 5: Run** `cargo test -p oikos-climate` and `cargo build -p oikos-climate --target wasm32-unknown-unknown` — PASS.
- [ ] **Step 6: Commit** `feat(climate): oikos-climate crate with documents and topics`.

---

### Task 2: Schedule, season and readings

**Files:**
- Create: `crates/oikos-climate/src/schedule.rs`
- Test: inline

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `impl ModeRules { pub fn at(&self, local: NaiveDateTime) -> (Setpoint, Option<usize>) }` — the block covering `local` (index into `blocks`) or `(base, None)`. A block covers `[from, to)` on each of its `days`.
  - `impl ModeRules { pub fn next_change(&self, local: NaiveDateTime) -> Option<NaiveDateTime> }` — the next instant strictly after `local` where `at()` changes value, searching 8 days; `None` if the schedule is constant.
  - `pub fn season_mode(house: &HouseSettings, outdoor_mean: Option<f64>, previous: Option<Mode>) -> Option<Mode>` — `None` = off. `Heat`/`Cool` seasons return that mode; `Off` → `None`; `Auto`: no outdoor → `Some(Heat)`; `< heat_below` → Heat; `> cool_above` → Cool; between → `previous.or(Some(Heat))`.
  - `pub struct ReadingSource { pub value: f64, pub age_s: u64 }`
  - `pub enum Reading { Fresh { value: f64, age_s: u64, from_sensor: bool, sensor_stale: bool }, Stale { last: Option<f64> }, Missing }`
  - `pub fn room_reading(sensor: Option<ReadingSource>, devices: &[ReadingSource], stale_after_s: u64) -> Reading` — §5.4: fresh sensor wins; else mean of fresh device readings (`sensor_stale = sensor.is_some()`); else `Stale { last }` (last = sensor value if any, else mean of device values) if anything was given; else `Missing`.

- [ ] **Step 1: Write the failing tests:**

```rust
fn dt(s: &str) -> NaiveDateTime { NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap() }
fn weekday_rules() -> ModeRules { /* base 17, block Mon–Fri 07:00–22:30 at 20.5 */ }
#[test] fn at_inside_and_outside_block() {
    assert_eq!(weekday_rules().at(dt("2026-10-21 07:00")), (Setpoint::Temp(20.5), Some(0))); // Wed
    assert_eq!(weekday_rules().at(dt("2026-10-21 22:30")), (Setpoint::Temp(17.0), None));
    assert_eq!(weekday_rules().at(dt("2026-10-24 10:00")), (Setpoint::Temp(17.0), None)); // Sat
}
#[test] fn at_handles_24_00_end() { /* block Fri 22:00–24:00: Fri 23:59 inside, Sat 00:00 outside */ }
#[test] fn next_change_same_day_and_wraps_week() {
    assert_eq!(weekday_rules().next_change(dt("2026-10-21 08:00")), Some(dt("2026-10-21 22:30")));
    assert_eq!(weekday_rules().next_change(dt("2026-10-23 23:00")), Some(dt("2026-10-26 07:00"))); // Fri → Mon
}
#[test] fn next_change_wraps_week() { /* only block Sun 10:00–11:00; from Sun 12:00 → next Sun 10:00 */ }
#[test] fn next_change_constant_is_none() { /* no blocks */ }
#[test] fn season_auto_hysteresis() {
    let h = HouseSettings::default();
    assert_eq!(season_mode(&h, Some(10.0), None), Some(Mode::Heat));
    assert_eq!(season_mode(&h, Some(26.0), Some(Mode::Heat)), Some(Mode::Cool));
    assert_eq!(season_mode(&h, Some(20.0), Some(Mode::Cool)), Some(Mode::Cool));
    assert_eq!(season_mode(&h, None, None), Some(Mode::Heat));
}
#[test] fn season_off_is_none() { /* Season::Off → None */ }
#[test] fn reading_prefers_fresh_sensor() { /* sensor age 60, devices fresh → from_sensor true */ }
#[test] fn reading_falls_back_to_devices_when_sensor_stale() { /* sensor age 4000, stale_after 3600, devices 19,21 fresh → Fresh{20.0, from_sensor:false, sensor_stale:true} */ }
#[test] fn reading_all_stale() { /* → Stale{last: Some(sensor value)} */ }
#[test] fn reading_missing() { /* nothing → Missing */ }
```

- [ ] **Step 2: Run** `cargo test -p oikos-climate schedule` — FAIL.
- [ ] **Step 3: Implement.** `next_change`: walk candidate instants (every block's `from` and `to` on each of the next 8 days) in order and return the first where `at()` differs from `at(local)`.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(climate): schedule lookup, season mode and room reading`.

---

### Task 3: The decision tree

**Files:**
- Create: `crates/oikos-climate/src/decide.rs`
- Test: inline (one test per priority row and per safety path, as §11 requires)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces:
  - `pub struct DeviceCaps { pub hvac_modes: Vec<String>, pub min_temp: Option<f64>, pub max_temp: Option<f64>, pub temp_step: Option<f64> }`
  - `pub struct Device { pub id: String, pub caps: DeviceCaps, pub target: Option<f64>, pub mode: Option<String> }`
  - `pub struct SafetyMemory { pub found: BTreeMap<String, DeviceTarget> }` with `SafetyMemory::capture(devices: &[Device]) -> SafetyMemory` (records each device's current target/mode; `held: false`).
  - `pub struct ZoneInputs<'a> { pub zone: &'a str, pub settings: &'a ZoneSettings, pub house: &'a HouseSettings, pub mode: Option<Mode>, pub reading: Reading, pub outdoor_mean: Option<f64>, pub home: bool, pub holiday: bool, pub overrides: &'a [Override], pub devices: &'a [Device], pub local_now: NaiveDateTime, pub now: u64, pub safety_memory: Option<&'a SafetyMemory> }`
  - `pub fn decide(inp: &ZoneInputs) -> Decision` — `applied` is always `false` here (the hub sets it); `at = inp.now`.
  - `pub fn fit(t: f64, caps: &DeviceCaps) -> f64` — clamp to min/max, round to `temp_step` (default 0.5).
  - `pub fn device_target(setpoint: Setpoint, mode: Mode, caps: &DeviceCaps) -> Option<DeviceTarget>` — `Temp(t)`: `Some{fit(t), mode}` only if `caps.hvac_modes` contains the mode string; `Off`: `"off"` if advertised, else heat → `min_temp`, cool → `max_temp` in that mode; `None` when the device cannot do the mode.

Rules `decide()` must implement (spec §5.1, §5.4, §7):
- `settings.state == Off` → `Source::Idle`, no entities, reason "Zone is off".
- **Safety** (any state but Off), first match, using `reading` (`Fresh` value):
  - value `< frost` → heat devices to `frost + 2`, reason `"Frost protection: 6.5 °C < 7 °C"`.
  - mode Heat and value `> overheat` → heat rules' `base` if it is `Temp(b)` with `b <= overheat`, else each device's `min_temp` (else `frost`).
  - mode Cool and value `< overcool` → `device_target(Off, Cool, …)`.
  - `Stale`/`Missing`: Active zone with a mode → heat: `min(current target, base)` floored at `frost` (`base == Off` → `frost`); cool: `max(current target, base)`; Pending zone → `Source::Safety`, no entities, reason says the reading is stale (notify only).
- **Pending zone, no safety:** if `safety_memory` is `Some` → `Source::Idle`, entities = `memory.found`, reason "Safety cleared: restoring previous settings"; else `Source::Idle` with no entities, reason "Waiting for approved rules", plus a would-do note: `setpoint` = what the schedule rows below would choose (no rules → `None`).
- **Override:** a zone override (`entity: None`, `until > now`) → `Source::Override` for all devices. Device overrides (`entity: Some(id)`) mark that device `held: true` with the override's setpoint/mode, and the decision's source comes from the remaining rows; reason appends `"; <id> held until HH:MM"`.
- **Holiday** → heat: `frost`; cool: `Off`. **Away** (`!home`) → `away_setpoint` of the mode. **Schedule** → `rules.get(mode).at(local_now)`; no rules for the mode or `mode == None` → `Source::Idle`, no entities.
- Devices that cannot do the mode are left out of `entities`.

- [ ] **Step 1: Write the failing tests** (names fixed; each builds `ZoneInputs` with one heat-capable device `trv` with caps `{["off","heat"], 5, 30, 0.5}` unless stated):

```rust
#[test] fn off_zone_does_nothing()
#[test] fn frost_beats_override_in_active_zone()      // reading 6.5, zone override 22 → Safety, trv 9.0 heat
#[test] fn frost_acts_in_pending_zone()
#[test] fn overheat_uses_base_or_device_min()        // base 17 → 17; base 31 → 5.0 (min_temp)
#[test] fn overcool_stops_cooling()                  // cool-capable device, mode Cool, reading 15 → mode "off"
#[test] fn overcool_without_off_uses_max_temp()
#[test] fn stale_active_heat_takes_lower_of_target_and_base() // target 21, base 17 → 17
#[test] fn stale_active_heat_base_off_falls_back_to_frost()   // → 7.0
#[test] fn stale_active_cool_takes_higher()
#[test] fn stale_pending_only_notifies()             // Safety, entities empty
#[test] fn pending_restores_after_safety()           // safety_memory {trv: 19 heat} → Idle with that entity
#[test] fn pending_shows_would_do()                  // rules give 20.5 → setpoint Some(Temp(20.5)), entities empty
#[test] fn zone_override_wins_over_schedule()
#[test] fn expired_override_is_ignored()             // until == now → schedule
#[test] fn device_override_holds_only_that_device()  // two devices, override on one → held:true on it, other on schedule
#[test] fn holiday_heat_is_frost()
#[test] fn away_uses_away_setpoint()
#[test] fn schedule_block_and_base()
#[test] fn rules_off_setpoint_turns_device_off()
#[test] fn device_without_mode_is_left_out()         // cool-only AC in Heat mode → absent from entities
#[test] fn fit_clamps_and_rounds()                   // max 30: 30.3 → 30.0; step 0.5: 20.26 → 20.5, 20.24 → 20.0; no step → 0.5
#[test] fn no_mode_is_idle()                         // season off
```

- [ ] **Step 2: Run** `cargo test -p oikos-climate decide` — FAIL.
- [ ] **Step 3: Implement `decide`** as a sequence of private fns (`safety`, `pending`, `overrides`, `presence`, `schedule`) each returning `Option<Decision>`. Reason strings are English and built from inputs only; temperatures formatted `{:.1} °C`.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(climate): decision tree with safety, overrides, presence and schedule`.

---

### Task 4: Command ledger and hand-change classifier

**Files:**
- Create: `crates/oikos-climate/src/ledger.rs`
- Test: inline

**Interfaces:**
- Consumes: Task 1 (`DeviceTarget`, `Decision`).
- Produces:
  - `pub const ECHO_WINDOW_S: u64 = 30; pub const DRIFT_WINDOW_S: u64 = 120;`
  - `pub struct Commanded { pub setpoint: Option<f64>, pub mode: String, pub at: u64 }`
  - `#[derive(Default)] pub struct Ledger { last: BTreeMap<String, Commanded> }` with
    - `pub fn from_decisions<'a>(decisions: impl IntoIterator<Item = &'a Decision>) -> Ledger` — seeds from each decision's `entities` (skipping `held`), `at = decision.at`; only decisions with `applied == true`.
    - `pub fn record(&mut self, entity: &str, target: &DeviceTarget, at: u64)`
    - `pub fn forget(&mut self, entity: &str)`
    - `pub fn get(&self, entity: &str) -> Option<&Commanded>`
    - `pub fn classify(&self, entity: &str, observed_target: Option<f64>, observed_mode: Option<&str>, step: f64, at: u64, online_since: Option<u64>) -> Observed`
  - `pub enum Observed { Unknown, Matches, Echo, Drift, HandChange }` — `Unknown`: never commanded; `Matches`: target within `step/2` and same mode; else `Echo` if `at - commanded.at <= ECHO_WINDOW_S`; else `Drift` if `online_since` is `Some(t)` and `at - t <= DRIFT_WINDOW_S`; else `HandChange`.
  - `pub fn hold_until(now: u64, next_change_in_s: Option<u64>, max_min: u32) -> u64` — `now + min(next_change_in_s.unwrap_or(MAX), max_min * 60)`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn unknown_when_never_commanded()
#[test] fn matches_within_half_step()        // commanded 20.0, observed 20.2 step 0.5 → Matches
#[test] fn echo_within_30s()                 // observed 19 at +10 s → Echo
#[test] fn drift_after_coming_back_online()  // +600 s, online_since = at-60 → Drift
#[test] fn hand_change_otherwise()           // +600 s, online_since None → HandChange
#[test] fn mode_change_is_a_change()         // same target, mode "off" vs "heat" at +600 → HandChange
#[test] fn ledger_rebuilt_from_applied_decisions_only()
#[test] fn restart_difference_is_hand_change() // from_decisions(at=1000), classify at 5000 with other target → HandChange
#[test] fn hold_until_caps_at_max()          // next 6 h, max 240 → now+14400; next 1 h → now+3600; None → now+14400
```

- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(climate): command ledger and hand-change classifier`.

---

### Task 5: Learner, diff and fingerprint

**Files:**
- Create: `crates/oikos-climate/src/learn.rs`
- Test: inline + `crates/oikos-climate/tests/learn_scenarios.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `pub const SLOTS: usize = 96;` (15 min)
  - `pub struct TargetSample { pub at: NaiveDateTime, pub target: Option<f64>, pub mode: Option<String> }` (local time; the first sample may predate the window and carries the value in force at its start)
  - `pub struct DeviceHistory { pub entity: String, pub samples: Vec<TargetSample> }` (sorted)
  - `pub struct EngineCommand { pub entity: String, pub at: NaiveDateTime, pub setpoint: Option<f64>, pub mode: String, pub source: Source }`
  - `pub struct LearnInput<'a> { pub mode: Mode, pub devices: &'a [DeviceHistory], pub engine: &'a [EngineCommand], pub approved: Option<&'a ModeRules>, pub first_day: NaiveDate, pub days: u32, pub min_days: u32, pub support: f64, pub frost: f64, pub caps: &'a DeviceCaps }`
  - `pub enum Slot { None, Off, Temp(f64) }`
  - `pub fn timeline(input: &LearnInput) -> Vec<(NaiveDate, [Slot; SLOTS])>` — §6.2: per slot, the value held for most of the slot per device; a device in a mode other than `input.mode` or `"off"` contributes nothing; `"off"` → `Off`; slot = mean of devices' `Temp`, or `Off` if every contributing device is off. A device value equal (within 0.25 °C, same mode) to the last `EngineCommand` for that device is: kept as is when that command's `source == Schedule` (agreement with the rule in force), `Slot::None` when the source is Safety/Away/Holiday/Override.
  - `pub enum LearnOutcome { NotEnough { days: u32, needed: u32 }, Rules(ModeRules) }`
  - `pub fn learn(input: &LearnInput) -> LearnOutcome` — a day counts when ≥ 48 slots are not `None`; fewer than `min_days` such days → `NotEnough`. Otherwise §6.3: groups Mon–Fri and Sat–Sun; a group splits into single days when any of its counted days differs from the group's per-slot median by > 0.5 °C (or Off vs Temp) on > 25 % of slots; per group: per-slot median (`Off` counts as below every temperature in heat, above in cool), merge neighbours within 0.5 °C, drop runs < 2 slots, `base` = most frequent run value (`Off` if Off dominates), blocks = runs above base (heat) / below base (cool); a block is kept if ≥ `support` of the group's counted days agree (≥ 80 % of the block's slots within 0.5 °C of its setpoint), `evidence = {agree, of}`; setpoints through `decide::fit` with `caps`, heat setpoints floored at `frost`; output sorted by (first day, from). Deterministic.
  - `pub fn differs(old: &ModeRules, new: &ModeRules) -> bool` — §6.3: base differs ≥ 0.5 °C (or Off vs Temp); a block with no counterpart (same days, overlapping) in the other; a counterpart whose `from` or `to` differs ≥ 30 min or setpoint ≥ 0.5 °C.
  - `pub fn diff(old: Option<&Rules>, new: &Rules) -> Vec<String>` — plain English lines, e.g. `"New: weekdays 07:00–22:30 at 20.5 °C (17 of 20 days)"`, `"Changed: weekdays start 07:00 → 06:30"`, `"Removed: Sat 09:00–12:00 at 21.0 °C"`, `"Base: 17.0 °C → 16.5 °C"`. Day sets print as `weekdays`, `weekends`, `every day`, or `Mon, Wed`.
  - `pub fn fingerprint(rules: &Rules) -> String` — FNV-1a 64-bit over `serde_json::to_string(rules)` with `evidence` stripped, lowercase hex (16 chars). No new dependency.

- [ ] **Step 1: Write the failing tests** (`tests/learn_scenarios.rs` builds histories with a helper `day_of(date, &[(“HH:MM”, target)])`):

```rust
#[test] fn not_enough_days()                 // 6 days of data, min 10 → NotEnough{days:6, needed:10}
#[test] fn learns_weekday_block()            // 20 weekdays: 17 → 20.5 at 07:00 → 17 at 22:30; 8 weekend days flat 17 → base 17, one block Mon–Fri 07:00–22:30 20.5, evidence {20,20}
#[test] fn noisy_day_lowers_evidence_not_block() // 3 of 20 weekdays flat 17 → evidence {17,20}, block kept (0.85 ≥ 0.6)
#[test] fn weak_pattern_dropped()            // block present on 10 of 20 weekdays → no block
#[test] fn off_at_night_learns_off_base()    // mode "off" 23:00–06:00 daily, 20 otherwise → base Off, block every day 06:00–23:00 at 20
#[test] fn cool_mode_blocks_below_base()
#[test] fn engine_schedule_slots_count_as_agreement() // active: engine commands from approved rules, no hand changes → learn == approved (differs false)
#[test] fn engine_away_slots_are_no_data()   // away commands in the evening → those slots None, not learned as a lower block
#[test] fn single_hand_change_does_not_change_rules() // 1 day with 22 °C instead of 20.5 → differs(approved, learned) == false
#[test] fn repeated_hand_changes_propose_change()    // 15 of 20 weekdays start 06:30 → differs true, diff contains "07:00 → 06:30"
#[test] fn heat_floor_at_frost()
#[test] fn deterministic()                   // learn twice → equal
#[test] fn fingerprint_ignores_evidence()
#[test] fn diff_lines()                      // New / Changed / Removed / Base wording
```

- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p oikos-climate` — PASS.
- [ ] **Step 5: Commit** `feat(climate): learn rules from target history, diff and fingerprint`.

---

### Task 6: Hub config, retained store, gating and view

**Files:**
- Create: `crates/oikos-climate/src/view.rs`, `crates/oikos-os/src/climate/mod.rs`, `climate/store.rs`, `climate/hub.rs`, `crates/oikos-os/tests/climate_hub.rs`
- Modify: `crates/oikos-os/Cargo.toml` (dep `oikos-climate = { path = "../oikos-climate", features = ["schemars"] }`), `crates/oikos-os/src/config.rs` (struct + `Config.climate` + `validate` + `STRING_ENV_KEYS` += `"climate.prefix"`), `crates/oikos-os/src/main.rs:48-53` (call `cfg.climate.validate()`), `crates/oikos-os/src/lib.rs` (`pub mod climate;`), `crates/oikos-os/src/hub.rs` (field, `new`, `extra_filters`, `ingest_message`, `require_climate`), `crates/oikos-api/src/events.rs` (`ClimateChanged`), `oikos.example.toml`

**Interfaces:**
- Consumes: Task 1 types and topics.
- Produces:
  - `config.rs`: `#[derive(Clone, Debug, Deserialize, Serialize)] #[serde(default)] pub struct ClimateConfig { pub enabled: bool, pub prefix: String }`, `Default` = `{false, "oikos/climate"}`, `fn validate(&self) -> Result<(), String>` → `Err("[climate] is enabled but prefix is empty")` when enabled and the trimmed prefix is empty.
  - `view.rs` (in `oikos-climate`, serde + optional schemars): `pub enum LearnStatus { Off, HistoryOff, NotEnough { days: u32, needed: u32 }, UpToDate, Proposed, Rejected }`; `pub struct ZoneView { pub zone: String, pub name: String, pub orphan: bool, pub settings: ZoneSettings, pub settings_invalid: Option<String>, pub entities: Vec<String>, pub reading: Option<f64>, pub reading_age_s: Option<u64>, pub decision: Option<Decision>, pub overrides: Vec<Override>, pub proposal: Option<Proposal>, pub learn: LearnStatus }`; `pub struct ClimateView { pub enabled: bool, pub house: HouseSettings, pub house_invalid: Option<String>, pub history: bool, pub zones: Vec<ZoneView> }`.
  - `store.rs`: `pub enum Applied { Changed, Unchanged, Invalid(String) }`; `#[derive(Default)] pub struct ClimateStore { pub house: HouseSettings, pub house_invalid: Option<String>, pub zones: BTreeMap<String, ZoneDocs> }`; `#[derive(Default)] pub struct ZoneDocs { pub settings: ZoneSettings, pub settings_invalid: Option<String>, pub proposal: Option<Proposal>, pub overrides: Vec<Override>, pub decision: Option<Decision> }`; `impl ClimateStore { pub fn apply(&mut self, topic: &ClimateTopic, payload: &[u8]) -> Applied }` — empty payload resets that document to default/`None`; JSON that fails to parse or `validate()` keeps the previous value and stores the reason in `*_invalid` (proposal/override/decision parse errors are logged and ignored).
  - `climate/hub.rs`: `impl Hub { pub fn climate_enabled(&self) -> bool; pub fn climate_view(&self) -> Result<ClimateView, HubError>; pub fn set_climate_house(&self, house: HouseSettings) -> Result<(), HubError>; pub fn set_climate_zone(&self, zone: &str, settings: ZoneSettings) -> Result<(), HubError>; pub(crate) fn climate_store(&self) -> MutexGuard<'_, ClimateStore>; pub(crate) fn climate_prefix(&self) -> Option<&str>; pub(crate) fn publish_climate(&self, topic: String, value: Option<&impl Serialize>) -> Result<(), HubError> }` (retained publish, then local `apply`, then broadcast `ClimateChanged`; `None` publishes the empty payload). Setters validate first (`HubError::Rejected(reason)`); `set_climate_zone` requires a known area (`HubError::UnknownArea`).
  - Zones in the view: every area with at least one `Kind::Climate` entity whose id does not start with `group_`, plus stored zones without such an area (`orphan: true`); `entities` = `settings.entities` or that default list. `name` from the area.
  - `hub.rs`: field `climate: Option<ClimateHub>` where `pub struct ClimateHub { prefix: String, store: Mutex<ClimateStore> }` (Task 8 adds runtime state); built in `Hub::new` only when `cfg.climate.enabled`. `extra_filters()` appends `topics::filters(prefix)` when enabled. `ingest_message` routes `topics::parse(prefix, topic)` hits to the store **before** the registry and returns `Vec::new()` for them. `fn require_climate(&self) -> Result<&ClimateHub, HubError>` → `HubError::Publish("climate disabled".into())` when off (HTTP maps it to 503 like groups).
  - `oikos-api`: `WsEvent::ClimateChanged { zone: Option<String> }` (`None` = house settings).

- [ ] **Step 1: Write the failing tests:**

`config.rs` (beside `groups_enabled_true_from_toml`):

```rust
#[test] fn climate_off_by_default() {
    let cfg = Config::default();
    assert!(!cfg.climate.enabled);
    assert_eq!(cfg.climate.prefix, "oikos/climate");
}
#[test] fn climate_enabled_from_toml() { /* "[climate]\nenabled = true\n" → enabled */ }
#[test] fn climate_empty_prefix_rejected_when_enabled() { /* validate() Err contains "prefix" */ }
```

`tests/climate_hub.rs` (helpers: `hub(enabled: bool)` like `tests/hub.rs:hub_with_timeout` with `cfg.climate.enabled = enabled`; `feed_daikin(&hub, n, area)` ingests `climate__daikin_192_0_2_15{n}__climate.json` from `oikos-core/tests/fixtures/discovery` and sets the area with `set_areas`/override as `tests/hub.rs` does):

```rust
#[tokio::test] async fn disabled_has_no_filters_and_refuses() {
    let (hub, _rx) = hub(false);
    assert!(!hub.extra_filters().iter().any(|f| f.contains("climate")));
    assert!(matches!(hub.climate_view(), Err(HubError::Publish(m)) if m == "climate disabled"));
}
#[tokio::test] async fn enabled_subscribes_house_and_zones_only() {
    let (hub, _rx) = hub(true);
    let f = hub.extra_filters();
    assert!(f.contains(&"oikos/climate/house".to_owned()) && f.contains(&"oikos/climate/zone/+/+".to_owned()));
    assert!(!f.iter().any(|t| t.ends_with('#')));
}
#[tokio::test] async fn zone_settings_doc_is_not_a_device() {
    let (hub, _rx) = hub(true);
    hub.ingest_message("oikos/climate/zone/living/settings", br#"{"state":"active"}"#, true);
    assert!(hub.snapshot().entities.iter().all(|e| !e.id.contains("living")));
}
#[tokio::test] async fn climate_group_discovery_still_reaches_registry() { /* ingest a climate group discovery at oikos/climate/group/g1/config (use groups test fixture/shape) → entity group_g1 exists */ }
#[tokio::test] async fn store_keeps_invalid_with_reason() {
    // valid settings then `{"state":"bogus"}` → view keeps state Active and settings_invalid is Some
}
#[tokio::test] async fn view_lists_area_zones_and_skips_groups() { /* two daikins in "living" + a group_* climate in living → one zone, entities = the two daikins */ }
#[tokio::test] async fn zone_without_area_is_listed_orphan() { /* settings for "attic", no area → orphan true */ }
#[tokio::test] async fn set_zone_publishes_retained_and_applies() {
    // set_climate_zone("living", ZoneSettings{state: Active, ..}) → publisher receives topic oikos/climate/zone/living/settings, retain true; view shows Active before any echo
}
#[tokio::test] async fn set_zone_rejects_invalid_rules() { /* overlapping blocks → HubError::Rejected */ }
```

- [ ] **Step 2: Run** `cargo test -p oikos-os --test climate_hub` and `cargo test -p oikos-os config::` — FAIL.
- [ ] **Step 3: Implement** config, view types, store, hub wiring and events. Add to `oikos.example.toml` after `[groups]`/`[history]`:

```toml
# The climate engine: learns each room's heating/cooling rules from history (needs [history]),
# asks before applying them (push + web), and keeps managed rooms on them. Everything except
# this switch is edited through MCP and the web Climate screen, as retained JSON under `prefix`.
[climate]
enabled = false
# prefix = "oikos/climate"
```

- [ ] **Step 4: Run** the tests above plus `cargo test -p oikos-os` — PASS (no regressions in groups/discovery tests).
- [ ] **Step 5: Commit** `feat(climate): [climate] switch, retained documents and hub view`.

---

### Task 7: History additions

**Files:**
- Create: `crates/oikos-os/migrations/0002_climate.sql`, `crates/oikos-os/src/history/climate.rs`
- Modify: `crates/oikos-os/src/history/mod.rs` (`pub mod climate;`), `crates/oikos-os/src/history/db.rs` (only if a helper must be shared)
- Test: `crates/oikos-os/tests/history_db.rs` (same `TEST_DATABASE_URL` skip pattern as the file's existing tests)

**Interfaces:**
- Consumes: Task 1 (`Source`, `DeviceTarget`).
- Produces (in `history/climate.rs`, methods on `Db` via `impl Db` in this file):
  - Migration:

```sql
CREATE TABLE climate_decision (
    time      timestamptz      NOT NULL,
    seq       bigserial,
    zone      text             NOT NULL,
    entity_id text             NOT NULL,
    mode      text             NOT NULL,
    setpoint  double precision,
    source    text             NOT NULL,
    reason    text             NOT NULL,
    applied   boolean          NOT NULL
);
SELECT create_hypertable('climate_decision', by_range('time', INTERVAL '30 days'));
CREATE INDEX climate_decision_zone_time ON climate_decision (zone, time DESC, seq DESC);
CREATE INDEX climate_decision_entity_time ON climate_decision (entity_id, time DESC, seq DESC);
```

  - `pub struct ClimateDecisionRow { pub at: Micros, pub zone: String, pub entity_id: String, pub mode: String, pub setpoint: Option<f64>, pub source: String, pub reason: String, pub applied: bool }` — one row per device of a changed decision (`entity_id = ""` with `mode = ""` for a decision with no devices).
  - `pub async fn insert_climate_decisions(&self, rows: &[ClimateDecisionRow]) -> Result<(), sqlx::Error>`
  - `pub async fn climate_decisions(&self, zone: &str, limit: i64) -> Result<Vec<ClimateDecisionRow>, sqlx::Error>` — newest first.
  - `pub async fn climate_commands(&self, entities: &[String], from: Micros, to: Micros) -> Result<Vec<ClimateDecisionRow>, sqlx::Error>` — `applied = true` rows for those entities, oldest first.
  - `pub async fn climate_targets(&self, entity: &str, from: Micros, to: Micros) -> Result<Vec<(Micros, Option<f64>, Option<String>)>, sqlx::Error>` — from `entity_state` where `value->>'type' = 'climate'`, reading `(value->'value'->>'target')::float8` and `value->'value'->>'mode'`; includes the last row before `from` (its time clamped to `from`), oldest first.
  - `pub async fn mean_since(&self, entity: &str, from: Micros, to: Micros) -> Result<Option<f64>, sqlx::Error>` — average `num` (for the outdoor 24 h mean).
  - `History` gets thin wrappers or callers use `History::db()` — follow whichever `query.rs` uses.

- [ ] **Step 1: Write the failing tests** (`history_db.rs`): `climate_decisions_round_trip` (insert 3 rows, read newest first), `climate_commands_filters_applied_and_entities`, `climate_targets_reads_jsonb_and_carries_prior_value` (insert climate states at t-2h, t-30m, t+10m for window [t-1h, t+1h] → first row value = t-2h's target at time t-1h), `mean_since_averages_num`.
- [ ] **Step 2: Run** `TEST_DATABASE_URL=… cargo test -p oikos-os --test history_db climate` — FAIL (without a DB the tests print `skipping` and pass; run with a local Timescale as the memory note says the Mac has one).
- [ ] **Step 3: Implement** the migration and queries.
- [ ] **Step 4: Run** — PASS with a DB.
- [ ] **Step 5: Commit** `feat(history): climate decision table and climate target queries`.

---

### Task 8: The evaluation loop, commands, hand changes and safety alerts

**Files:**
- Create: `crates/oikos-os/src/climate/inputs.rs`, `climate/engine.rs`, `climate/alerts.rs`
- Modify: `climate/mod.rs` (`ClimateHub` runtime state), `crates/oikos-os/src/main.rs` (spawn + abort)
- Test: `crates/oikos-os/tests/climate_hub.rs`

**Interfaces:**
- Consumes: Tasks 1–7; `Hub::read()`, `Hub::subscribe()`, `Hub::subscribe_state_seen()`, `Hub::send_command(&str, Command)`, `Hub::notify(&str, &Value)`, `Hub::zone()`, `Hub::history()`, `clock::now()`.
- Produces:
  - Runtime state on `ClimateHub` (behind one `Mutex<ClimateRuntime>`): `ledger: Ledger`, `seen_at: HashMap<String, u64>`, `online_since: HashMap<String, u64>`, `safety_memory: HashMap<String, SafetyMemory>` (zone → memory), `failures: HashMap<String, (u32, Option<u64>)>` (entity → consecutive failures, paused-until), `last_sent: HashMap<String, u64>`, `decisions: HashMap<String, VecDeque<Decision>>` (cap 200), `last_home_at: Option<u64>`, `previous_mode: Option<Mode>`, `outdoor_cache: Option<(u64, Option<f64>)>`, `outdoor_live: VecDeque<(u64, f64)>` (24 h of live readings), `active_alerts: HashSet<String>`.
  - `pub fn start_climate(self: &Arc<Hub>) -> Option<JoinHandle<()>>` (in `engine.rs`; `None` when disabled). `main.rs` spawns it after `start_engine()` and aborts it on shutdown.
  - `pub async fn evaluate_climate(self: &Arc<Hub>, now: u64)` — one full pass over all zones (also used by tests and by Task 9 after an approval).
  - `inputs.rs`: `pub(crate) fn zone_inputs_owned(hub: &Hub, zone: &str, now: u64) -> Option<OwnedZoneInputs>` collecting devices (`Device` from entity caps + `ClimateState`), reading (`room_reading` with ages from `seen_at`), presence (`home` with `away_grace`: home if any presence entity is home now, or was within `away_grace_min`), holiday, outdoor mean (history `mean_since` cached 10 min, else `outdoor_live` mean), `local_now` from `Hub::zone()`. Presence entity is home when: `device_tracker` state text `"home"`, a binary sensor `true`, a switch/virtual switch `true`.
  - `alerts.rs`: `pub(crate) fn raise(hub: &Hub, id: &str, title: &str, body: &str, severity: &str)` / `pub(crate) fn resolve(hub: &Hub, id: &str)` — payload `{v:1, id, title, body, channel:"climate", severity, ttl: 43200}` / `{v:1, id, cancel:true}` via `Hub::notify("climate", …)`; ids `climate-safety-<zone>-frost|overheat|overcool|stale` and `climate-command-<entity>`; `active_alerts` makes raise/resolve idempotent.

Behaviour (spec §5.5, §7, §9):
- Loop: `tokio::select!` over a 60 s interval, `StateChanged` events (2 s debounce on climate-relevant ids: zone devices, sensors, presence, holiday, outdoor), `StateSeen` (updates `seen_at` only), availability transitions (an entity going `Online` sets `online_since[id] = now`).
- Per zone: build inputs → `decide` → for `Active` zones (and `Safety`/restore decisions in `Pending` zones) command each non-held device whose current target/mode differs (> `step/2`, or mode differs): `SetHvacMode` first if needed, then `SetTemperature`; skip if `last_sent` < 60 s ago or paused. On `Ok(accepted: true)` → `ledger.record`; on refusal or `Err` → failure count; at 3 → pause 15 min + `raise(climate-command-<entity>)`; next acceptance resolves it.
- Pending safety: when a `Safety` decision first commands a pending zone, `SafetyMemory::capture` first; after the restore decision is applied, drop the memory.
- Hand changes: on a `StateChanged` for a device of an `Active` zone, `ledger.classify(...)`; `HandChange` → push `Override { entity: Some(id), setpoint, mode, until: hold_until(now, next_change_in_s, manual_hold_max_min), origin: Hand }` into the zone's override doc (replacing any for that entity), dropping expired ones; `Drift` → nothing (next pass re-asserts); `Unknown`/`Matches`/`Echo` → nothing. `next_change_in_s` uses `ModeRules::next_change(local_now)` converted with `Hub::zone()` (handle ambiguous/nonexistent local times with `earliest()` / skip forward).
- Boot: once the retained decisions have arrived (first pass after start), seed `ledger` with `Ledger::from_decisions` over stored decisions; devices then differing classify as `HandChange` measured from boot.
- Decision output: set `applied` (true when every intended command was accepted or nothing needed sending in an `Active` zone); if `!same_outcome(previous)`: publish retained `…/decision`, push to the 200-deque, write rows via `insert_climate_decisions` (spawned, errors logged), broadcast `ClimateChanged{zone}`.
- Safety alerts: `raise` on Safety decisions (frost/overheat → `"critical"`, overcool/stale → `"warning"`), `resolve` when the zone's decision is no longer that safety cause.
- Entities that vanish from the registry: `ledger.forget`, drop their overrides, never command them.

- [ ] **Step 1: Write the failing tests** (`climate_hub.rs`; drive with `evaluate_climate(&hub, now)`, inspect the publisher channel; set device state by ingesting the daikin state topic JSON):

```rust
#[tokio::test] async fn pending_zone_commands_nothing()
#[tokio::test] async fn active_zone_commands_schedule_target()      // rules base 17 → SetTemperature 17 published on the daikin temperature_command_topic
#[tokio::test] async fn frost_commands_pending_zone_then_restores() // reading 5 → frost+2; reading back 18 → previous target restored; alert raised then cancelled
#[tokio::test] async fn no_command_when_already_at_target()
#[tokio::test] async fn rate_limited_to_one_per_minute()
#[tokio::test] async fn hand_change_creates_device_override()       // ledger says 20, device echoes 22 at +10 min → override doc with entity, origin hand, until = next change or +240 min
#[tokio::test] async fn echo_within_30s_is_not_hand_change()
#[tokio::test] async fn drift_after_reconnect_is_reasserted()       // availability offline→online, target resets within 2 min → next pass re-sends
#[tokio::test] async fn restart_seeds_ledger_from_retained_decision() // retained decision says 20 applied, device at 22 → HandChange override
#[tokio::test] async fn three_failures_pause_and_alert()            // confirm timeout → 3 refusals → no 4th send for 15 min, climate-command-* alert
#[tokio::test] async fn failed_publish_is_not_recorded()            // ConnectionStatus::Offline → failure counted, ledger unchanged; later echo of old value is not HandChange
#[tokio::test] async fn decision_published_only_when_outcome_changes()
#[tokio::test] async fn zone_drops_removed_entity()
#[tokio::test] async fn hold_until_across_dst()                     // zone Europe/Paris, rules change at 02:30 on 2026-03-29 → override until is a valid instant, no panic
#[tokio::test] async fn away_after_grace()                          // presence switch off for 10 min → schedule; 16 min → away setpoint
```

- [ ] **Step 2: Run** `cargo test -p oikos-os --test climate_hub` — FAIL.
- [ ] **Step 3: Implement** inputs, engine loop, alerts, `main.rs` wiring.
- [ ] **Step 4: Run** the tests + `cargo test -p oikos-os` — PASS.
- [ ] **Step 5: Commit** `feat(climate): evaluation loop, commands, hand changes and safety alerts`.

---

### Task 9: Learning runs, proposals and push approval

**Files:**
- Create: `crates/oikos-os/src/climate/learning.rs`, `climate/proposals.rs`
- Modify: `climate/engine.rs` (nightly timer + first run), `crates/oikos-os/src/hub.rs` (`extra_filters` adds `push().responses_topic()` when climate is enabled; `ingest_message` routes non-retained responses to `on_climate_reply`)
- Test: `crates/oikos-os/tests/climate_hub.rs` (learning tests that need a DB live in `tests/history_db.rs` with the skip pattern)

**Interfaces:**
- Consumes: Tasks 5, 7, 8.
- Produces:
  - `impl Hub { pub async fn learn_climate_zone(self: &Arc<Self>, zone: &str) -> Result<LearnStatus, HubError>; pub async fn approve_climate_proposal(self: &Arc<Self>, zone: &str, edited: Option<Rules>) -> Result<(), HubError>; pub async fn reject_climate_proposal(self: &Arc<Self>, zone: &str) -> Result<(), HubError>; pub async fn set_climate_override(self: &Arc<Self>, zone: &str, ov: Override) -> Result<(), HubError>; pub async fn clear_climate_override(self: &Arc<Self>, zone: &str) -> Result<(), HubError>; pub(crate) fn on_climate_reply(self: &Arc<Self>, payload: &[u8]) }`
  - `learn_status: HashMap<String, LearnStatus>` and `last_proposal_push: HashMap<String, u64>` on the runtime.
  - `learning.rs`: `pub(crate) async fn learn_zone(hub: &Arc<Hub>, zone: &str, now: u64) -> LearnStatus` — `HistoryOff` without history; for each `Mode` some zone device supports: `climate_targets` per device over `learn_days` (+ prior value) and `climate_commands`, converted to local `NaiveDateTime` with `Hub::zone()`; `oikos_climate::learn::learn`. Merge learned modes into a candidate `Rules` (unlearned modes keep the approved ones). `Initial` when `settings.rules.is_empty()`; otherwise propose only if `differs()` for some learned mode. Skip when `fingerprint` matches a `rejected` entry with `until > now`. Publish via `proposals::publish`.
  - `proposals.rs`: `pub(crate) fn publish(hub, zone, proposal, now)` — retained `…/proposal`, and a push `{v:1, id:"climate-proposal-<proposal.id>", title:"Climate: new rules for <area name>" | "Climate: rule changes for <area name>", body: first two `changes` lines + "Review on the Climate screen.", channel:"climate", severity:"info", actions:[{id:"approve",label:"Approve"},{id:"reject",label:"Reject"}]}` unless a proposal push for that zone went out < 24 h ago; replacing a pending proposal resolves the old alert. Proposal id: `p-<YYYYMMDD-HHMM local>-<zone>`.
  - Approve: rules = `edited` (validated) or the proposal's; `settings.rules = rules`, `state: Pending → Active`, `approved_at = now`, `approved_proposal = id`; publish settings; clear the proposal (empty retained); resolve the alert; `evaluate_climate` for that zone. Reject: append `Rejection { fingerprint, until: now + 7*86400 }` (pruning expired), publish settings, clear proposal, resolve alert. Both `HubError::Rejected("no pending proposal")` when none.
  - `on_climate_reply`: parse `{id, action}`; ignore ids not starting `climate-proposal-`; find the zone whose pending proposal id matches the suffix — none → ignore (stale/duplicate); `approve`/`reject` → the methods above (spawned).
  - Schedule: the climate loop runs `learn_zone` for every non-`Off` zone on its first pass after start (the "on enable" run) and daily at `house.learn_at` local; `learn_climate_zone` runs one zone now.

- [ ] **Step 1: Write the failing tests:**

```rust
#[tokio::test] async fn history_off_reports_status()                   // learn_climate_zone → LearnStatus::HistoryOff, no proposal
#[tokio::test] async fn approve_activates_zone_and_clears_proposal()  // seed a proposal doc by ingest; approve → settings Active with rules, proposal topic published empty, alert cancel published
#[tokio::test] async fn approve_with_edited_rules_validates()          // overlapping edited blocks → Rejected
#[tokio::test] async fn reject_remembers_fingerprint_for_7_days()
#[tokio::test] async fn push_reply_approves()                          // ingest {"id":"climate-proposal-<id>","action":"approve"} on oikos/push/responses → zone Active
#[tokio::test] async fn stale_reply_is_ignored()                       // reply for an older proposal id → nothing changes, no error
#[tokio::test] async fn proposal_push_has_buttons_and_daily_limit()    // two publishes for one zone within 24 h → one notification
#[tokio::test] async fn responses_filter_only_when_enabled()
```

and in `history_db.rs` (DB-gated): `learn_zone_proposes_initial_rules_from_history` (insert 14 weekdays of daikin target history → proposal with a weekday block), `learn_zone_skips_rejected_fingerprint`.

- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p oikos-os` (+ DB tests with `TEST_DATABASE_URL`) — PASS.
- [ ] **Step 5: Commit** `feat(climate): learning runs, proposals and approval over push`.

---

### Task 10: HTTP routes, MCP tools and docs

**Files:**
- Create: `crates/oikos-os/src/mcp/climate_tools.rs`
- Modify: `crates/oikos-os/src/http/api.rs` (routes + 503 mapping already covers `HubError::Publish("climate disabled")` — verify), `crates/oikos-os/src/mcp/mod.rs:44` (`+ Self::climate_tool_router()`) and the instructions text at `mcp/mod.rs:113`, `crates/oikos-os/src/mcp/params.rs`, `README.md`, `TODOS.md`
- Test: `crates/oikos-os/tests/http.rs`, `crates/oikos-os/tests/mcp.rs`

**Interfaces:**
- Consumes: Tasks 6, 8, 9 hub methods.
- Produces:
  - HTTP under `/api/v1/climate`: `GET /` → `ClimateView`; `PUT /house` (body `HouseSettings`) → 204; `PUT /zones/{id}` (body `ZoneSettings`) → 204; `POST /zones/{id}/override` (body `{ setpoint?: Setpoint, mode?: Mode, minutes?: u32, until?: u64 }`, exactly one of `minutes`/`until`) → 204; `DELETE /zones/{id}/override` → 204; `POST /zones/{id}/proposal/approve` (optional body `Rules`) → 204; `POST /zones/{id}/proposal/reject` → 204; `POST /zones/{id}/learn` → 200 `LearnStatus`; `GET /zones/{id}/decisions?limit=` → `Vec<Decision>`-like rows (in-memory 200, then `climate_decisions` when history is on). Disabled → 503; `Rejected` → 400; `UnknownArea` → 404.
  - MCP tools (params in `params.rs` with `schemars`): `climate_status` (→ `ClimateView`), `get_climate_zone {zone}` (→ `ZoneView` + last 20 decisions), `set_climate_settings {settings: HouseSettings}`, `set_climate_zone {zone, settings: ZoneSettings}`, `approve_climate_proposal {zone, rules?: Rules}`, `reject_climate_proposal {zone}`, `set_climate_override {zone, setpoint?, mode?, minutes?, until?}`, `clear_climate_override {zone}`, `learn_climate_zone {zone}`. Disabled → tool error text `"climate disabled"`.
  - Instructions paragraph (append to `mcp/mod.rs` instructions): "Climate (when `[climate] enabled`): each area with a climate device is a zone (`off`, `pending`, `active`). `climate_status` shows every zone's reading, decision and why (`source`, `reason`), overrides and pending proposal. The engine learns rules from history and proposes them; nothing applies until `approve_climate_proposal` (or the push button). Safety (frost, overheat, stale readings) always acts. A temperature set by hand holds until the zone's next scheduled change, at most `manual_hold_max_min`."
  - README: a "Climate" entry in the feature list (one paragraph, no "Hestia"); TODOS.md: replace the climate brainstorm section with "Climate engine v1 landed" + follow-ups (shutters, pricing, ML designs).

- [ ] **Step 1: Write the failing tests:** `http.rs`: `climate_disabled_is_503`, `climate_get_view`, `climate_put_zone_400_on_invalid`, `climate_override_minutes_and_until_exclusive`, `climate_approve_without_proposal_400`. `mcp.rs`: `climate_tools_listed`, `climate_status_disabled_error`, `climate_status_enabled_returns_zones`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p oikos-os` — PASS.
- [ ] **Step 5: Commit** `feat(climate): HTTP routes, MCP tools and docs`.

---

### Task 11: Web Climate screen — zones, decisions, overrides, settings

**Files:**
- Create: `crates/oikos-web/src/api/climate.rs`, `src/screens/climate.rs`, `src/climate/mod.rs`, `src/climate/zone_card.rs`, `src/climate/settings_sheet.rs`
- Modify: `crates/oikos-web/Cargo.toml` (dep `oikos-climate = { path = "../oikos-climate" }`), `src/api/mod.rs`, `src/screens/mod.rs`, `src/lib.rs`, `src/app.rs` (route `/climate` + nav link shown only when enabled), `src/house_state.rs` (on `WsEvent::ClimateChanged` bump a `climate_rev` signal)
- Test: wasm-bindgen tests beside existing web tests (`wasm-pack test --headless --chrome crates/oikos-web` or the repo's documented command)

**Interfaces:**
- Consumes: Task 10 HTTP; `oikos_climate::{view::*, doc::*}`.
- Produces:
  - `api/climate.rs`: `get_climate() -> Result<ClimateView, ApiError>`, `put_house(&HouseSettings)`, `put_zone(&str, &ZoneSettings)`, `set_override(&str, OverrideBody)`, `clear_override(&str)`, `approve(&str, Option<&Rules>)`, `reject(&str)`, `learn(&str) -> Result<LearnStatus, ApiError>`, `decisions(&str, u32) -> Result<Vec<Decision>, ApiError>` — 503 maps to a distinct `ApiError` variant or status the nav uses to hide the link.
  - `screens/climate.rs`: `#[component] pub fn ClimateScreen() -> impl IntoView` — refetches on `climate_rev`; renders one `ZoneCard` per zone; empty state "No room has a climate device"; history-off banner.
  - `ZoneCard`: area name, reading + age, target/mode, state switch (`off`/`pending`/`active` segmented control → `put_zone`), `source` chip + `reason`, overrides with end time and a clear button, a quick override (setpoint + 1 h / 2 h / until next change), learn status text (`"Learning: 6 of 10 days so far"`, `"History is off"`), the last 10 decisions (expandable).
  - `SettingsSheet`: edits §4.3 fields with entity pickers for `outdoor_entity`, `presence`, `holiday_entity` (reuse the existing entity picker component used by flows/groups).
  - App nav: "Climate" link between "Security" and "Alerts", rendered only when `get_climate()` succeeded at start-up (and after `ClimateChanged`).

- [ ] **Step 1: Write the failing tests:** `learn_status_text` (pure fn `fn learn_status_text(&LearnStatus) -> String`: NotEnough{6,10} → "Learning: 6 of 10 days so far", HistoryOff → "History is off: rules can't be learned", Proposed → "New rules to review", UpToDate → "Rules up to date"); `reading_age_text` (`fn age_text(secs: u64) -> String`: 45 → "just now", 600 → "10 min ago", 7200 → "2 h ago"); a mount test `climate_screen_renders_zone` with a stubbed view if the repo's web tests support fetch stubs, else skip the mount test and keep the pure-fn tests.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement** following the existing screens' structure and CSS conventions (`cards/climate.rs` for room controls; class names `climate-*`). **Step 4: Run** web tests + `cargo build -p oikos-web --target wasm32-unknown-unknown` — PASS. Check the screen in a headless browser (reference memory: headless Chrome CDP screenshots, window ≥ 500 px) against a hub with `[climate] enabled = true`.
- [ ] **Step 5: Commit** `feat(web): Climate screen with zones, decisions, overrides and settings`.

---

### Task 12: Web proposal review and rule editor

**Files:**
- Create: `crates/oikos-web/src/climate/timeline.rs`, `src/climate/proposal.rs`
- Modify: `src/climate/zone_card.rs` (embed both)
- Test: wasm-bindgen / native unit tests for the pure editing functions

**Interfaces:**
- Consumes: Task 11; `oikos_climate::doc::{Rules, ModeRules, Block, HhMm, Setpoint, Day}`, `oikos_climate::learn::diff`.
- Produces:
  - `timeline.rs`: `#[component] pub fn WeekTimeline(rules: Signal<ModeRules>, editable: bool, on_change: Callback<ModeRules>) -> impl IntoView` — 7 rows (Mon–Sun) × 24 h, `base` as the background tint, blocks as bars labelled with setpoint and evidence ("17/20"); in edit mode a block's ends drag in 15-min steps and a click opens setpoint / delete.
  - Pure helpers (unit-tested, no DOM): `pub fn move_edge(rules: &ModeRules, block: usize, edge: Edge, to: HhMm) -> ModeRules` (snaps to 15 min, keeps `from < to`, refuses overlaps by clamping to the neighbour), `pub fn set_setpoint(rules: &ModeRules, block: usize, sp: Setpoint) -> ModeRules`, `pub fn delete_block(rules: &ModeRules, block: usize) -> ModeRules`; `pub enum Edge { From, To }`.
  - `proposal.rs`: `#[component] pub fn ProposalPanel(zone: String, proposal: Proposal, approved: Rules) -> impl IntoView` — title by kind, the plain-English `changes`, a mode tab (Heat/Cool) when both exist, `WeekTimeline` of the proposed rules (editable after "Edit"), live `diff(Some(&approved), &edited)` under it while editing, buttons Approve (sends edited rules when edited) / Reject.
  - The zone's approved rules also render with `WeekTimeline`; "Edit rules" saves through `put_zone` (direct edit = approval, §4.4).

- [ ] **Step 1: Write the failing tests:** `move_edge_snaps_to_15_min`, `move_edge_clamps_to_neighbour`, `move_edge_keeps_from_before_to`, `set_setpoint_off`, `delete_block_keeps_others`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** tests + wasm build — PASS; screenshot the proposal panel (pending zone with a seeded proposal) in headless Chrome.
- [ ] **Step 5: Commit** `feat(web): review, edit and approve climate proposals`.

---

## Self-review notes

- Spec coverage: §4.1 → T6; §4.2/§4.3/§4.4 → T1, T6; §5.1–§5.4 → T2, T3; §5.5 → T8; §6 → T5, T7, T9; §7 → T4, T8, T9 (explicit overrides); §8.1 → T1–T9; §8.2 → T7; §8.3/§9 → T8 (safety), T9 (proposals); §8.4 → T10; §8.5 → T11, T12; §10 (no app change) → T9 buttons Approve/Reject; §11 → Global Constraints + T3 tests.
- Plan-level calls beyond the spec: Unix-second timestamps in documents; a learning "day" needs ≥ 48 of 96 slots; group split rule (> 25 % of slots off by > 0.5 °C); block agreement = ≥ 80 % of its slots; one `climate_decision` row per device; outdoor live fallback ring of 24 h; proposal id format `p-<YYYYMMDD-HHMM>-<zone>`.
