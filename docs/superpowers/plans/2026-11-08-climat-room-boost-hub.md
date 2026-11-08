# Climat room boost (hub) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A whole-zone override with a temperature becomes a *boost*: a temporary room target reached through the room-thermometer correction. Leaving the house ends a boost set before the leave, and boosts can be set and cleared over a new MQTT command topic.

**Architecture:** The pure crate `nidoo-climate` gains a `Source::Boost` decision row that runs the existing correction, a `Decision.holiday` flag, a pure `left_house` transition test, and the hold command topic helpers. The `nidoo` hub drops boosts on the home → away / holiday-start transition in `evaluate_zone`, and handles `<prefix>/zone/<zone>/hold/set` in `ingest_message`.

**Tech Stack:** Rust workspace, serde, tokio. Tests: in-file unit tests plus `crates/nidoo/tests/climate_hub.rs` (part of the `integration` test binary).

**Spec:** `~/superconductor/projects/Nidoo-app/docs/superpowers/specs/2026-11-08-climat-room-boost-design.md` §5 (Nidoo-app repo, commit `0f322422`).

## Global Constraints

- Work on a new branch `feat/climat-room-boost` from `master` (the repo is currently on `ci/faster-tests`). Commit as `Gekkotron` (`-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`).
- Green before each commit: `cargo test -p nidoo-climate`, `cargo test -p nidoo`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`.
- Retained doc changes stay backward compatible: new fields are `#[serde(default)]`, so old retained JSON still parses.
- Safety still beats everything. A `pending` zone ignores overrides. Per-device hand holds are unchanged.
- The boost duration is exactly what the request says (`minutes`/`until`). Never cap it by `manual_hold_max_min` or the next schedule change.
- Mode-only and `"off"` whole-zone overrides keep today's behavior (`Source::Override`, no correction).

## Review Focus

1. **Boost while the thermometer is stale or missing:** the safety stale row already wins, so the boost must not push its raw target. Test it in Task 1.
2. **Boost with correction disabled for the zone:** the boost target goes to the devices unchanged (`adjust: None`). Test it in Task 1.
3. **Hub restart while away with a boost set before leaving:** the retained decision carries `home`/`holiday`, so the transition is not re-detected and the boost isn't dropped twice. And a boost set *after* the restart, while away, survives. Test it in Task 2.
4. **Retained or junk message on `…/hold/set`:** consumed (never reaches the device registry), never acted on when `retain` is true, and malformed JSON is logged and ignored. Test it in Task 3.
5. **Clearing a boost keeps per-device hand holds:** the MQTT clear removes only `entity: None` overrides (unlike HTTP `DELETE`, which clears everything today). Test it in Task 3.

---

### Task 1: Boost decision row with correction

**Files:**
- Modify: `crates/nidoo-climate/src/doc.rs` (enum `Source`, line ~628)
- Modify: `crates/nidoo-climate/src/decide.rs` (`zone_override` ~498, `rules`/`apply_correction` ~677-735, tests module)
- Modify: `crates/nidoo-climate/src/feedback.rs` (`FeedbackState::restored` ~222, `FeedbackState::after` ~242)
- Modify: `crates/nidoo-web/src/climate/mod.rs` (~105, ~120) + its EN/FR text table (`ClimateText`)

**Interfaces:**
- Produces: `Source::Boost` (serde `"boost"`). A decision with `source: boost` has `setpoint: Some(Temp(t))` = the boost target and `adjust` = the correction offset (or `None`), with the reason `"Boost to {t} until {HH:MM}"`.

- [ ] **Step 1: Write the failing tests** in `decide.rs` `mod tests`, reusing `Case`, `corrected`, `zone_ov`, `dev_ov`:
  - `boost_runs_correction` replaces `no_correction_under_zone_override`. `corrected(21.6, 30)` plus `zone_ov(21.0, NOW + 3600)` gives `d.source == Source::Boost`, `d.setpoint == Some(Temp(21.0))`, `d.adjust.is_some()` and `d.entities["trv"].setpoint < Some(Temp(21.0))`. The room is above target, so the offset is negative, exactly as `corrected(21.6, …)` does for the schedule today.
  - `boost_without_feedback_sends_target` (`Case::new()`, feedback disabled, `zone_ov(22.0, …)`) gives `Source::Boost`, the trv at `Temp(22.0)` and `adjust == None`.
  - `mode_only_override_is_unchanged` keeps the existing `zone_override_mode_without_setpoint_uses_its_own_mode_rules` asserting `Source::Override`, and adds an `"off"` setpoint override asserting `Source::Override` with no adjust.
  - `boost_beats_away_eco_and_empty`: one case each with `home = false`, `room_eco = Some(eco_until(...))`, `room_empty = true` (`empty_room(...)`) gives `Source::Boost`.
  - `stale_reading_beats_boost` (as `stale_never_lets_schedule_override_or_away_through`) gives `Source::Safety`.
  - `held_device_under_boost`: `zone_ov(22.0)` plus `dev_ov("a", 20.0)` with `dev_step` devices gives "a" `held` at 20.0, and "b" follows the corrected boost.
  - `feedback.rs`: `after_tracks_boost` and `restored_picks_up_boost_offset` (copy `after_tracks_since_and_steps` / `restored_picks_up_the_retained_offset` with `Source::Boost`).

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test -p nidoo-climate`
Expected: compile error, `no variant Boost`.

- [ ] **Step 3: Implement**
  - Add `Boost` to `Source`, after `Override`.
  - In `zone_override`, when `ov.setpoint` is `Some(Setpoint::Temp(t))`, build the decision with `Source::Boost` and the reason `format!("Boost to {} until {}", setpoint_text(sp), clock(inp, ov.until))`. Set `d.entities = targets(..)`, then call `apply_correction(inp, &mut d, mode, t)`, then `hold_devices`. Otherwise keep today's path.
  - Widen both `matches!(decision.source, Source::Schedule | Source::Empty)` in `feedback.rs` to include `Source::Boost`.
  - Web: map `Source::Boost` to a new `ClimateText::SrcBoost` ("Boost" / "Boost"), and `"boost"` in the history parser.
  - Fix any other exhaustive `match` on `Source` that the compiler flags (`cargo check --workspace`).

- [ ] **Step 4: Run and confirm everything passes**

Run: `cargo test -p nidoo-climate && cargo test -p nidoo-web && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit** `feat(climate): a zone override with a temperature is a boost the correction drives`

---

### Task 2: Leaving the house ends a boost

**Files:**
- Modify: `crates/nidoo-climate/src/doc.rs` (`Decision`, line ~665; `same_outcome` ~692)
- Modify: `crates/nidoo-climate/src/decide.rs` (`base_decision` ~173)
- Modify: `crates/nidoo-climate/src/presence.rs` (new pure fn + tests)
- Modify: `crates/nidoo/src/climate/engine.rs` (`evaluate_zone` ~485)
- Test: `crates/nidoo/tests/climate_hub.rs` (`mod engine`, `Rig`)

**Interfaces:**
- Produces: `Decision.holiday: bool` (`#[serde(default)]`, part of `same_outcome`). `pub fn left_house(previous: Option<&Decision>, home: bool, holiday: bool) -> bool` in `nidoo_climate::presence`: true when `previous` was at home (`home && !holiday`) and now `!home || holiday`, and false with no previous decision.

- [ ] **Step 1: Write the failing tests**
  - `presence.rs`: `left_house_on_presence_leave` (prev home, now away gives true), `left_house_on_holiday_start` (prev home, now holiday gives true), `no_leave_when_already_away` (prev away, now away gives false), `no_leave_on_return` (prev away, now home gives false), `no_leave_without_previous` (gives false). Also check that a decision JSON without `holiday` deserializes with `holiday == false`.
  - `climate_hub.rs` `mod engine`, with `Rig` driving presence the way the existing presence/away tests in that file do:
    - `leaving_drops_a_boost_set_before`: set the boost via `hub.set_climate_override`, `pass`, go away (past `away_grace_min`), `pass`. The zone's override list has no `entity: None` entry, the retained `…/override` doc is republished without it, and the decision source is `away`. Come home and `pass`: the source is `schedule`, not `boost`.
    - `boost_set_while_away_runs_and_survives_return`: go away, `pass`, set the boost, `pass` gives `boost`. Come home, `pass`: still `boost`.
    - `leaving_keeps_hand_holds`: a per-device hand hold survives the leave.
    - `restart_while_away_keeps_new_boost`: go away and `pass`, then rebuild the hub from the published retained docs (or clear `climate_runtime().decisions` so `last_decision` falls back to the retained doc). Set the boost and `pass`: still `boost`.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test -p nidoo-climate presence && cargo test -p nidoo --test integration climate_hub::engine`
Expected: compile error, then FAIL.

- [ ] **Step 3: Implement**
  - Add `holiday` to `Decision` and fill it in `base_decision` from `inp.holiday`.
  - Write `left_house`.
  - In `evaluate_zone`, right after `tidy_zone` and before `decide`: `if left_house(self.last_decision(zone).as_ref(), inp.home, inp.holiday)`, call `self.update_overrides(zone, |l| l.retain(|o| o.entity.is_some()))` and drop the same entries from `inp.overrides`. Log `tracing::info!(zone, "house left: boost ended")` when something was removed.
  - No `since` field is needed: the transition pass runs before any boost set after it can exist. A boost set in the same pass window as the leave may be dropped, which is acceptable (spec §5.2 leaves the method to the builder).

- [ ] **Step 4: Run and confirm everything passes**

Run: `cargo test -p nidoo-climate && cargo test -p nidoo`
Expected: PASS.

- [ ] **Step 5: Commit** `feat(climate): leaving the house ends a boost set before it`

---

### Task 3: MQTT hold command

**Files:**
- Modify: `crates/nidoo-climate/src/topics.rs`
- Modify: `crates/nidoo/src/climate/hub.rs` (next to `ingest_climate`, ~289)
- Modify: `crates/nidoo/src/hub.rs` (`ingest_message` ~778, before `ingest_climate`)
- Modify: `crates/nidoo/src/climate/proposals.rs` (next to `clear_climate_override`, ~255)
- Test: `crates/nidoo-climate/src/topics.rs` tests, `crates/nidoo/tests/climate_hub.rs`
- Docs: README "climate" section and Oikos-os `docs/users/*` where the climate MQTT documents are listed. Add one line for the command topic and its payload.

**Interfaces:**
- Produces (topics.rs):
  - `pub fn hold_command_topic(prefix: &str, zone: &str) -> String` returns `<p>/zone/<zone>/hold/set`.
  - `pub fn parse_hold_command(prefix: &str, topic: &str) -> Option<String>` returns the zone.
  - `filters` returns `[String; 3]` with `<p>/zone/+/hold/set` added.
- Produces (proposals.rs): `pub async fn clear_climate_boost(self: &Arc<Self>, zone: &str) -> Result<(), HubError>`, which removes only `entity: None` overrides.
- Payload: JSON `OverrideRequest` (`{"setpoint": 22, "minutes": 90}`) sets the boost through `into_override(now)`, then `set_climate_override`. An empty payload calls `clear_climate_boost`.

- [ ] **Step 1: Write the failing tests**
  - `topics.rs`:
    - `hold_command_round_trip`: `hold_command_topic("nidoo/climate/", "living") == "nidoo/climate/zone/living/hold/set"`, and `parse_hold_command` returns `Some("living")`.
    - `parse_hold_command("nidoo/climate", "nidoo/climate/zone/living/override") == None`.
    - `parse("nidoo/climate", "nidoo/climate/zone/living/hold/set") == None`.
    - Update `builds_and_parses` to the 3-filter list.
  - `climate_hub.rs`:
    - Update `enabled_subscribes_house_and_zones_only` to include the hold filter.
    - `hold_command_sets_a_boost`: with an active zone, `hub.ingest_message("nidoo/climate/zone/living/hold/set", br#"{"setpoint":22,"minutes":90}"#, false)` returns an empty `Vec`. After yielding (`tokio::task::yield_now` / a paused-time tick), the zone has one `entity: None` override with `setpoint 22` and `until == now + 5400`.
    - `hold_command_empty_clears_boost_keeps_hand_hold`.
    - `hold_command_ignores_retained`, `hold_command_ignores_bad_json`, `hold_command_ignores_pending_zone`, and `hold_command_without_duration_is_rejected`: no override is added, and nothing reaches the registry (`hub.read()` has no new entity).

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test -p nidoo-climate topics && cargo test -p nidoo --test integration climate_hub::`
Expected: FAIL.

- [ ] **Step 3: Implement**
  - Topic helpers and the third filter.
  - `fn ingest_climate_command(&self, topic: &str, payload: &[u8], retain: bool) -> bool` in `climate/hub.rs`:
    - Return false when climate is disabled or the topic doesn't parse.
    - Otherwise return true. When `!retain`, upgrade `self.me` and `tokio::spawn` the set or clear, logging any `Err` with `tracing::warn!(zone, error)`.
    - Call it first in `ingest_message`.
  - Write `clear_climate_boost`.

- [ ] **Step 4: Run the full gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: PASS.

- [ ] **Step 5: Commit** `feat(climate): set and clear a room boost over MQTT`
