# Live AC and Robot Vacuum Icons Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Climate and vacuum card tiles in the hub's web UI show live icons, driven by state, that match the Android app's.

**Architecture:** A new `live_icon` module in `crates/nidoo-web`:
- The icon maths is plain Rust with native tests: AC tones and periods, robot path and poses, and a phase store.
- One shared requestAnimationFrame loop bumps a frame signal, and each icon's changing SVG attributes read their phase from a store keyed by entity id. A card rebuilt by a state update therefore carries on where it was.
- `CardFrame` renders `<LiveIcon>` for climate and vacuum entities.

**Tech Stack:** Rust, Leptos 0.8 (csr), web-sys/js-sys, wasm-bindgen-test via wasm-pack, CSS in `styles/cards.css`.

**Spec:** `docs/superpowers/specs/2026-11-19-live-ac-robot-icons-design.md`

## Global Constraints

- `#![forbid(unsafe_code)]` crate. No new dependencies; a web-sys feature may be added only if the compiler asks for one.
- **Drawing:** 24-unit viewBox `0 0 24 24`.
  - Outlines (AC unit, dock): stroke 1.75, round caps and joins, as in `icons.rs`.
  - Small details keep the app's widths: vane 1.3, airflow 1.5, robot 1.3.
  - The room hint is stroke 1 at opacity .28.
- **Colours,** only through tokens. Each is a class on the `<svg>` that sets `--live-tone`. Tinted parts use `var(--live-tone)` and neutral parts use `currentColor`.
  - `live-icon--cool`: `var(--ring-cool)`
  - `live-icon--heat`: `var(--ring-heat)`
  - `live-icon--auto`: `var(--ring-other)`
  - `live-icon--plain` and `live-icon--off`: `currentColor`
  - `live-icon--moving`: `var(--warning)`
  - `live-icon--docked`: `var(--connected)`
- **AC airflow periods,** matched case-insensitively: silent/silence/quiet/low/"1" → 2400 ms; "2" → 2000; medium/med/"3" → 1600; "4" → 1300; high/"5" → 1000; anything else or none → 1800.
- **Robot row periods,** matched case-insensitively: quiet → 11000 ms; turbo → 6000; max → 4500; anything else → 8000.
- **Other fixed timings:** vane 3200 ms; drops 1400 ms; return glide 3000 ms, arriving at 0.75 of the cycle; bolt 3600 ms for a full breath (0.4 → 0.9 → 0.4).
- Frame `dt` is clamped to 100 ms.
- Animations run only on wasm, and only when `energy_dots::reduced_motion()` is false. Otherwise every icon shows its still pose.
- The `bare_text` guard: no user-facing literals in views. The SVG only has class and geometry attributes.
- **Commits:** `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`. Every message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Commands,** all run from the worktree root `/Users/julienhuguel/superconductor/projects/Nidoo-live-icons`:
  - Native tests: `cargo test -p nidoo-web --lib live_icon`.
  - Browser tests: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test live_icon)`. wasm-pack downloads chromedriver itself.

## Review Focus

- **A card rebuilt mid-clean,** for example by a battery update: the new icon is mounted before the old one is cleaned up. The robot must carry on, not restart. Pinned by `store_keeps_phase_across_a_rebuild` (Task 3).
- **Fan speed spelt in another case** ("Turbo", "HIGH"): it gets the same pace as lower case. Pinned by `periods_ignore_case` (Tasks 1 and 2).
- **The tab hidden for minutes, then shown:** icons step at most 100 ms, with no jump. Pinned by `advance_clamps_long_frames` (Task 3).
- **An offline AC whose last mode was "cool":** it shows the off pose, with no airflow. Pinned by `offline_is_off` (Task 1) and `offline_climate_tile_is_off` (Task 4).
- **Every live icon leaving the screen:** the loop stops. Pinned by `store_goes_idle_when_released` (Task 3).

---

### Task 1: AC maths

**Files:**
- Create: `crates/nidoo-web/src/live_icon/mod.rs`, `crates/nidoo-web/src/live_icon/ac.rs`
- Modify: `crates/nidoo-web/src/lib.rs` (add `pub mod live_icon;`, alphabetical)

**Interfaces:**
- Produces, in `live_icon::ac`:
  - `enum AcTone { Cool, Heat, Auto, Plain }` (`Copy, Debug, PartialEq`), with `fn class(self) -> &'static str` returning `"live-icon--cool" | "--heat" | "--auto" | "--plain"`.
  - `fn ac_tone(mode: Option<&str>, available: bool) -> Option<AcTone>` (`None` means off).
  - `fn is_dry(mode: Option<&str>) -> bool`.
  - `fn airflow_period_ms(fan_mode: Option<&str>) -> f64`.
  - `const VANE_PERIOD_MS: f64 = 3200.0`, `const DROPS_PERIOD_MS: f64 = 1400.0`.
  - `const VANE_REST_Y: f64 = 14.4`, `const VANE_SWING: f64 = 0.35`, `fn vane_y(phase: f64) -> f64`.
  - `fn flow_dash_offset(phase: f64) -> f64`.
  - `fn drop_fall(phase: f64) -> (f64, f64)`, returning (dy, opacity).
- `mod.rs` declares `pub mod ac;`. Later tasks add `robot`, `clock` and `view`.

- [ ] **Step 1: Write the failing tests** (`#[cfg(test)] mod tests` in `ac.rs`)

```rust
#[test] fn modes_map_to_tones() {
    assert_eq!(ac_tone(Some("cool"), true), Some(AcTone::Cool));
    assert_eq!(ac_tone(Some("dry"), true), Some(AcTone::Cool));
    assert_eq!(ac_tone(Some("heat"), true), Some(AcTone::Heat));
    assert_eq!(ac_tone(Some("fan_only"), true), Some(AcTone::Plain));
    assert_eq!(ac_tone(Some("heat_cool"), true), Some(AcTone::Auto));
    assert_eq!(ac_tone(Some("auto"), true), Some(AcTone::Auto));
    assert_eq!(ac_tone(Some("off"), true), None);
    assert_eq!(ac_tone(None, true), None);
}
#[test] fn offline_is_off() { assert_eq!(ac_tone(Some("cool"), false), None); }
#[test] fn airflow_periods_follow_the_fan() {
    for (fan, ms) in [("quiet", 2400.0), ("low", 2400.0), ("1", 2400.0), ("2", 2000.0), ("medium", 1600.0),
                      ("med", 1600.0), ("3", 1600.0), ("4", 1300.0), ("high", 1000.0), ("5", 1000.0), ("auto", 1800.0)] {
        assert_eq!(airflow_period_ms(Some(fan)), ms, "{fan}");
    }
    assert_eq!(airflow_period_ms(None), 1800.0);
}
#[test] fn periods_ignore_case() {
    assert_eq!(airflow_period_ms(Some("HIGH")), 1000.0);
    assert_eq!(airflow_period_ms(Some("Silent")), 2400.0);
}
#[test] fn vane_swings_below_the_unit() {
    assert!((vane_y(0.0) - 14.4).abs() < 1e-9);
    assert!((vane_y(0.25) - 14.75).abs() < 1e-9);
    assert!((vane_y(0.75) - 14.05).abs() < 1e-9);
    // Top edge of the 1.3 vane stays clear of the unit's bottom edge (12.4 + 1.75 / 2).
    let top = (0..100).map(|i| vane_y(i as f64 / 100.0)).fold(f64::MAX, f64::min) - 0.65;
    assert!(top > 12.4 + 0.875, "{top}");
}
#[test] fn flow_moves_down_one_dash_cycle_per_loop() {
    assert_eq!(flow_dash_offset(0.0), 0.0);
    assert!((flow_dash_offset(0.5) + 5.0).abs() < 1e-9);
}
#[test] fn drops_fade_in_fall_and_fade_out() {
    assert_eq!(drop_fall(0.0), (0.0, 0.0));
    let (_, o) = drop_fall(0.15); assert!((o - 1.0).abs() < 1e-9);
    let (dy, o) = drop_fall(1.0); assert!((dy - 7.0).abs() < 1e-9 && o.abs() < 1e-9);
}
#[test] fn tones_have_their_classes() {
    assert_eq!(AcTone::Heat.class(), "live-icon--heat");
}
```

- [ ] **Step 2: Run the tests and check they fail.** Run: `cargo test -p nidoo-web --lib live_icon`. Expected: compile errors for the missing items.

- [ ] **Step 3: Implement `ac.rs`.**
  - `vane_y = VANE_REST_Y + VANE_SWING * sin(2π·phase)`.
  - `flow_dash_offset = -10 * phase`, because the dash array is `2.6 2.4` and two cycles make up one loop.
  - `drop_fall`: `dy = 7·phase²`; opacity is `phase / 0.15` below 0.15, then `(1 - phase) / 0.85`.
  - Fan matching uses `to_ascii_lowercase`.

- [ ] **Step 4: Run the tests and check they pass.** Run the same command. Expected: 8 passed.

- [ ] **Step 5: Commit** with the message `feat(web): AC icon maths: mode tones, airflow pace, vane and drops`.

---

### Task 2: Robot maths

**Files:**
- Create: `crates/nidoo-web/src/live_icon/robot.rs` (add `pub mod robot;` to `mod.rs`)

**Interfaces:**
- Produces, in `live_icon::robot`:
  - `enum VacuumScene { Idle, Cleaning, Returning, Docked }` (`Copy, Debug, PartialEq`).
  - `fn vacuum_scene(state: Option<&str>, available: bool) -> VacuumScene`. It matches case-insensitively. Unavailable → `Idle`; "cleaning", "returning" and "docked" map to their scenes; anything else → `Idle`.
  - `struct RobotPose { x: f64, y: f64, heading_deg: f64 }` (`Copy, Debug, PartialEq`), with `fn transform(&self) -> String` returning `"translate({x} {y}) rotate({heading_deg})"`.
  - `const ROWS: [(f64, f64); 8] = [(5.,5.5),(19.,5.5),(19.,10.),(5.,10.),(5.,14.5),(19.,14.5),(19.,19.),(5.,19.)]`.
  - `const PATH_LENGTH: f64 = 69.5`, and `const ROWS_D: &str = "M5 5.5H19V10H5V14.5H19V19H5"`, the trail's `d`.
  - `fn pose_at(t: f64) -> RobotPose`. `t` is clamped to 0..=1, and the heading follows the current segment: +x → 0, +y → 90, −x → 180.
  - `fn trail_dash_offset(t: f64) -> f64`, equal to `PATH_LENGTH * (1 - t)`. The trail uses `stroke-dasharray="69.5 69.5"`.
  - `fn row_period_ms(fan_speed: Option<&str>) -> f64`.
  - `const RETURN_FROM: (f64, f64) = (6.5, 8.5)`, `const DOCK_AT: (f64, f64) = (19.5, 19.0)`, `const RETURN_HEADING: f64 = 39.0`, `const RETURN_PERIOD_MS: f64 = 3000.0`.
  - `fn return_pose(phase: f64) -> RobotPose`. It moves along a smoothstep `3s² - 2s³` with `s = min(phase / 0.75, 1)`, then holds at the dock.
  - `const BOLT_PERIOD_MS: f64 = 3600.0`, `fn bolt_opacity(phase: f64) -> f64`, a triangle wave of 0.4 → 0.9 → 0.4.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn path_ends_and_corners() {
    assert_eq!(pose_at(0.0), RobotPose { x: 5.0, y: 5.5, heading_deg: 0.0 });
    let c = pose_at(14.0 / PATH_LENGTH); assert!((c.x - 19.0).abs() < 1e-9 && (c.y - 5.5).abs() < 1e-9);
    let mid_down = pose_at(16.0 / PATH_LENGTH); assert_eq!(mid_down.heading_deg, 90.0);
    let back = pose_at(25.0 / PATH_LENGTH); assert_eq!(back.heading_deg, 180.0);
    let end = pose_at(1.0); assert!((end.x - 5.0).abs() < 1e-9 && (end.y - 19.0).abs() < 1e-9);
}
#[test] fn path_clamps() { assert_eq!(pose_at(-1.0), pose_at(0.0)); assert_eq!(pose_at(2.0), pose_at(1.0)); }
#[test] fn rows_add_up_to_the_length() {
    let len: f64 = ROWS.windows(2).map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()).sum();
    assert!((len - PATH_LENGTH).abs() < 1e-9);
}
#[test] fn trail_grows_with_the_robot() {
    assert_eq!(trail_dash_offset(0.0), PATH_LENGTH); assert_eq!(trail_dash_offset(1.0), 0.0);
}
#[test] fn row_periods_follow_the_fan() {
    assert_eq!(row_period_ms(Some("quiet")), 11000.0); assert_eq!(row_period_ms(Some("turbo")), 6000.0);
    assert_eq!(row_period_ms(Some("max")), 4500.0); assert_eq!(row_period_ms(Some("standard")), 8000.0);
    assert_eq!(row_period_ms(None), 8000.0);
}
#[test] fn periods_ignore_case() { assert_eq!(row_period_ms(Some("Turbo")), 6000.0); }
#[test] fn scenes_follow_the_state() {
    assert_eq!(vacuum_scene(Some("cleaning"), true), VacuumScene::Cleaning);
    assert_eq!(vacuum_scene(Some("Returning"), true), VacuumScene::Returning);
    assert_eq!(vacuum_scene(Some("docked"), true), VacuumScene::Docked);
    for s in ["idle", "paused", "error", "unknown"] { assert_eq!(vacuum_scene(Some(s), true), VacuumScene::Idle); }
    assert_eq!(vacuum_scene(Some("cleaning"), false), VacuumScene::Idle);
}
#[test] fn return_glides_then_holds_at_the_dock() {
    assert_eq!(return_pose(0.0), RobotPose { x: 6.5, y: 8.5, heading_deg: 39.0 });
    assert_eq!(return_pose(0.75), RobotPose { x: 19.5, y: 19.0, heading_deg: 39.0 });
    assert_eq!(return_pose(0.9), return_pose(0.75));
}
#[test] fn bolt_breathes() {
    assert!((bolt_opacity(0.0) - 0.4).abs() < 1e-9); assert!((bolt_opacity(0.5) - 0.9).abs() < 1e-9);
    assert!((bolt_opacity(1.0) - 0.4).abs() < 1e-9);
}
```

- [ ] **Step 2: Run the tests and check they fail.** Run: `cargo test -p nidoo-web --lib live_icon::robot`. Expected: compile errors.
- [ ] **Step 3: Implement `robot.rs`.**
- [ ] **Step 4: Run the tests and check they pass.** Expected: 9 passed.
- [ ] **Step 5: Commit** with the message `feat(web): robot icon maths: rows, trail, return glide and bolt`.

---

### Task 3: Phase store and shared frame loop

**Files:**
- Create: `crates/nidoo-web/src/live_icon/clock.rs` (add `pub mod clock;` to `mod.rs`)

**Interfaces:**
- Produces, in `live_icon::clock`:
  - **`PhaseStore`,** natively tested:
    - `PhaseStore::default()`.
    - `acquire(&mut self, key: &str, period_ms: f64)`: adds a user and sets the period, which the newest caller wins. It keeps any stored phase.
    - `release(&mut self, key: &str)`: removes a user, saturating at zero. The phase is kept.
    - `advance(&mut self, dt_ms: f64)`: only tracks with users move; `dt` is clamped to `0..=MAX_DT_MS`, where `const MAX_DT_MS: f64 = 100.0`. Each phase becomes `(phase + dt / period).rem_euclid(1.0)`, with the period floored at 1.
    - `phase(&self, key: &str) -> f64`: 0 for an unknown key.
    - `is_active(&self) -> bool`: true while any track has users.
  - **`fn animations_on() -> bool`:** true only on wasm when `crate::energy_dots::reduced_motion()` is false.
  - **`fn use_phase(key: String, period_ms: f64) -> Signal<f64>`,** for use inside a component:
    - It acquires the key in a thread-local `RefCell<PhaseStore>`, and releases it in `on_cleanup`.
    - It starts the loop if it isn't running.
    - It returns `Signal::derive` that reads a thread-local `ArcRwSignal<u64>` frame counter, then the store's phase.
    - **The loop:** `request_animation_frame` with `js_sys::Date::now()` deltas. Each frame advances the store and bumps the counter. The loop stops, and clears its running flag, when `!is_active()`.
    - Off wasm, it returns a constant `Signal::stored(0.0)` and touches nothing.

- [ ] **Step 1: Write the failing tests** (native, in `clock.rs`)

```rust
#[test] fn advance_moves_by_dt_over_period() {
    let mut s = PhaseStore::default(); s.acquire("a", 1000.0); s.advance(50.0); s.advance(50.0);
    assert!((s.phase("a") - 0.1).abs() < 1e-9);
}
#[test] fn a_new_period_carries_on_from_the_current_phase() {
    let mut s = PhaseStore::default(); s.acquire("a", 1000.0); s.advance(100.0); s.advance(100.0); // 0.2
    s.acquire("a", 500.0); s.release("a"); // rebuild with a faster fan
    s.advance(50.0);
    assert!((s.phase("a") - 0.3).abs() < 1e-9);
}
#[test] fn store_keeps_phase_across_a_rebuild() {
    let mut s = PhaseStore::default(); s.acquire("r", 1000.0); s.advance(100.0); s.advance(100.0);
    s.acquire("r", 1000.0); s.release("r"); // new card mounts before the old one cleans up
    assert!((s.phase("r") - 0.2).abs() < 1e-9); assert!(s.is_active());
    s.advance(100.0); assert!((s.phase("r") - 0.3).abs() < 1e-9);
}
#[test] fn advance_clamps_long_frames() {
    let mut s = PhaseStore::default(); s.acquire("a", 1000.0); s.advance(60_000.0);
    assert!((s.phase("a") - 0.1).abs() < 1e-9);
}
#[test] fn store_goes_idle_when_released() {
    let mut s = PhaseStore::default(); s.acquire("a", 1000.0); s.release("a"); s.release("a");
    assert!(!s.is_active()); s.advance(50.0); assert_eq!(s.phase("a"), 0.0);
}
#[test] fn phases_wrap() {
    let mut s = PhaseStore::default(); s.acquire("a", 100.0); s.advance(100.0); s.advance(30.0);
    assert!((s.phase("a") - 0.3).abs() < 1e-9);
}
#[test] fn animations_are_off_natively() { assert!(!animations_on()); }
```

- [ ] **Step 2: Run the tests and check they fail.** Run: `cargo test -p nidoo-web --lib live_icon::clock`. Expected: compile errors.
- [ ] **Step 3: Implement `clock.rs`.** The wasm-only parts go behind `#[cfg(target_arch = "wasm32")]`, as in `energy_dots::run`.
- [ ] **Step 4: Run the tests and check they pass.** Expected: 7 passed. Also run `cargo check -p nidoo-web --target wasm32-unknown-unknown`. Expected: no errors.
- [ ] **Step 5: Commit** with the message `feat(web): one shared frame loop and a phase store that survives card rebuilds`.

---

### Task 4: `LiveIcon` for climate, wired into `CardFrame`

**Files:**
- Create: `crates/nidoo-web/src/live_icon/view.rs` (add `pub mod view;` to `mod.rs`)
- Create: `crates/nidoo-web/tests/live_icon.rs` (copy the `entity` and `mount` helpers and the `climate` and `vacuum` fixtures from `tests/rich_cards.rs`)
- Modify:
  - `crates/nidoo-web/src/components/card_frame.rs`, at the tile, `<span class="card__icon" inner_html=icon>`
  - `crates/nidoo-web/src/icons.rs` (`CLIMATE`)
  - `crates/nidoo-web/styles/cards.css`

**Interfaces:**
- Consumes: `ac::*` (Task 1), `clock::{use_phase, animations_on}` (Task 3).
- Produces:
  - `live_icon::view::LiveIcon(entity: EntityView) -> impl IntoView`. A climate entity draws the AC. A vacuum entity draws `VacuumIcon`, which Task 5 adds; until then it falls back to the static glyph. Any other kind falls back to the static glyph too.
  - `fn has_live_icon(kind: Kind) -> bool`, true for `Kind::Climate | Kind::Vacuum`.
- **The DOM contract** that the tests rely on:
  - `<svg class="live-icon live-icon--{tone|off}" viewBox="0 0 24 24">`, which holds:
    - the unit: `rect` x2.5 y4 w19 h8.4 rx2.6, `currentColor`, fill-opacity .22
    - the light line `M5.5 7.3h9`, opacity .55
    - the LED: a circle at 18.2,7.3, r .85, `class="live-icon__tint"`, opacity .3 when off
    - the vane: a `line` from 6 to 18, `class="live-icon__vane live-icon__tint"` when on (`live-icon__vane` alone when off), y from `vane_y`
  - When on and not dry: three `path.live-icon__flow` at x = 7, 12 and 17, d=`M{x} 14.2c-1.1 1.6 1.1 3.2 0 4.8s1.1 3.2 0 4.8`, dasharray `2.6 2.4`. They're faded downward by a `<mask>` with a white-to-black vertical gradient over y 13–24. Mask and gradient ids are suffixed with the entity id, sanitised to `[a-z0-9_-]`, so two cards on one page don't collide.
  - When dry: three `path.live-icon__drop` at x = 7, 12 and 17 (`M{x} 14.6c.8 1.1 1.3 1.8 1.3 2.4a1.3 1.3 0 0 1-2.6 0c0-.6.5-1.3 1.3-2.4z`), each moved by `drop_fall` and phase-shifted by 0, 1/3 and 2/3.
  - The three flows are phase-shifted by 0, 1/3 and 2/3 of the same phase.
- **Phase keys:** `"{id}/vane"` (`VANE_PERIOD_MS`), `"{id}/flow"` (`airflow_period_ms(fan_mode)`) and `"{id}/drops"` (`DROPS_PERIOD_MS`). A key is acquired only when its part is shown and `animations_on()` is true. Otherwise the still values are vane 14.4, flow offset 0 and drop phase 0.5.
- **`CardFrame`:** when `icon` (the prop) is `None` and `has_live_icon(entity.kind)`, the tile's `.card__icon` span holds `<LiveIcon entity/>` instead of `inner_html`.
- **`icons.rs`:** `CLIMATE` is redrawn as the off pose, using the same unit, light line and vane at 14.4, in the `svg!` line style.
- **`cards.css`:**
  - `.live-icon { --live-tone: currentColor }`, plus one rule per tone class from Global Constraints.
  - `.live-icon__tint` uses `stroke: var(--live-tone)` (or `fill` for the LED, drops and bolt).
  - The svg fills `.card__icon` the way the static glyph does (see `main.css` `.card__icon svg`).

- [ ] **Step 1: Write the failing browser tests** in `tests/live_icon.rs`

```rust
#[wasm_bindgen_test] async fn cooling_tile_blows_cool_air() {
    let host = mount(climate("cool"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon.live-icon--cool"));
    assert_eq!(count(&host, ".card__tile .live-icon__flow"), 3);
    assert_eq!(count(&host, ".card__tile .live-icon__drop"), 0);
}
#[wasm_bindgen_test] async fn heating_tile_is_heat_toned() {
    let host = mount(climate("heat"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--heat"));
}
#[wasm_bindgen_test] async fn drying_tile_drips() {
    let host = mount(climate("dry"), None); tick().await;
    assert_eq!(count(&host, ".card__tile .live-icon__drop"), 3);
    assert_eq!(count(&host, ".card__tile .live-icon__flow"), 0);
}
#[wasm_bindgen_test] async fn off_tile_hangs_the_vane_closed() {
    let host = mount(climate("off"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--off .live-icon__vane"));
    assert_eq!(count(&host, ".card__tile .live-icon__flow"), 0);
}
#[wasm_bindgen_test] async fn offline_climate_tile_is_off() {
    let mut e = climate("cool"); e.state.availability = Availability::Offline;
    let host = mount(e, None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--off"));
}
#[wasm_bindgen_test] async fn airflow_moves_between_frames() {
    let host = mount(climate("cool"), None); tick().await;
    let flow = host.query_selector(".live-icon__flow").unwrap().unwrap();
    let before = flow.get_attribute("stroke-dashoffset");
    sleep_ms(250).await; // setTimeout wrapped in a js_sys::Promise + JsFuture
    assert_ne!(before, flow.get_attribute("stroke-dashoffset"));
}
```

- [ ] **Step 2: Run the tests and check they fail.** Run: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test live_icon)`. Expected: compile errors, or failures on `svg.live-icon`.
- [ ] **Step 3: Implement** `view.rs` (the AC part and the fallback), the `CardFrame` switch, the `CLIMATE` redraw and the CSS.
- [ ] **Step 4: Run the tests and check they pass.**
  - Run the browser tests again. Expected: 6 passed.
  - Then run `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test rich_cards --test card_frame)`. Expected: pass, since existing cards are unchanged.
  - Then run `cargo test -p nidoo-web --lib icons`. Expected: `every_kind_has_a_distinct_svg` passes.
- [ ] **Step 5: Commit** with the message `feat(web): the climate card tile shows a live AC blowing air in its mode's colour`.

---

### Task 5: `VacuumIcon`

**Files:**
- Modify:
  - `crates/nidoo-web/src/live_icon/view.rs`
  - `crates/nidoo-web/src/icons.rs` (`VACUUM`)
  - `crates/nidoo-web/styles/cards.css`
  - `crates/nidoo-web/tests/live_icon.rs`

**Interfaces:**
- Consumes: `robot::*` (Task 2), `clock::*` (Task 3), and `LiveIcon`'s vacuum branch (Task 4).
- **The DOM contract:** `<svg class="live-icon live-icon--{moving|docked|plain}">`, which always holds the room hint, a `rect` x1.5 y2 w21 h20.5 rx3 at stroke 1 and opacity .28. Then, per scene:
  - **Cleaning** (`--moving`):
    - `path.live-icon__trail`, d=`ROWS_D`, `live-icon__tint`, stroke 4.4, opacity .16, dasharray `69.5 69.5`, dashoffset from `trail_dash_offset`.
    - `g.live-icon__robot`, its transform from `pose_at`. It holds:
      - a circle r3.1 with fill-opacity .35 and stroke 1.3
      - the bumper, `M2.05 -2.05A2.9 2.9 0 0 1 2.05 2.05`
      - the puck, a circle r1.05
  - **Returning** (`--moving`): `rect.live-icon__dock` x18 y16.5 w4 h5 rx1, neutral, stroke 1.75. Then the robot, with its transform from `return_pose`.
  - **Docked** (`--docked`):
    - the dock with the tint stroke and fill-opacity .22
    - the robot at `DOCK_AT`, heading 39, neutral, fill-opacity .22
    - `path.live-icon__bolt`, `M12.3 8 L10 12 h2.5 l-.8 3.8 L14.4 11.5 H11.9 L12.8 8z`, filled with the tint, its opacity from `bolt_opacity`
  - **Idle** (`--plain`): the robot at `pose_at(0)`, neutral, fill-opacity .22.
- **Phase keys:** `"{id}/rows"` (`row_period_ms(fan_speed)`), `"{id}/return"` (`RETURN_PERIOD_MS`) and `"{id}/bolt"` (`BOLT_PERIOD_MS`). Each is acquired only in its scene, and only when animations are on. The still values are rows 0.5, return 1.0 and bolt 0.75 (opacity 0.65).
- **`icons.rs`:** `VACUUM` is redrawn as the idle pose (room hint and robot at the start), in the `svg!` line style.
- **The vacuum card's big `Robot` picture is unchanged.**

- [ ] **Step 1: Write the failing browser tests** (append to `tests/live_icon.rs`)

```rust
#[wasm_bindgen_test] async fn cleaning_tile_drives_rows() {
    let host = mount(vacuum("cleaning"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--moving .live-icon__trail"));
    assert!(has(&host, ".card__tile .live-icon__robot"));
    let robot = host.query_selector(".card__tile .live-icon__robot").unwrap().unwrap();
    let before = robot.get_attribute("transform"); sleep_ms(250).await;
    assert_ne!(before, robot.get_attribute("transform"));
}
#[wasm_bindgen_test] async fn returning_tile_heads_for_the_dock() {
    let host = mount(vacuum("returning"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--moving .live-icon__dock"));
    assert!(!has(&host, ".card__tile .live-icon__trail"));
}
#[wasm_bindgen_test] async fn docked_tile_charges() {
    let host = mount(vacuum("docked"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--docked .live-icon__bolt"));
}
#[wasm_bindgen_test] async fn error_tile_is_still() {
    let host = mount(vacuum("error"), None); tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--plain .live-icon__robot"));
    assert!(!has(&host, ".card__tile .live-icon__trail") && !has(&host, ".card__tile .live-icon__bolt"));
}
#[wasm_bindgen_test] async fn the_big_robot_picture_stays() {
    let host = mount(vacuum("cleaning"), None); tick().await;
    assert!(has(&host, "svg.robot--cleaning"));
}
```

- [ ] **Step 2: Run the tests and check they fail.** Run: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test live_icon)`. Expected: the 5 new tests fail.
- [ ] **Step 3: Implement** `VacuumIcon` in `view.rs`, route `LiveIcon`'s vacuum branch to it, and redraw `VACUUM`.
- [ ] **Step 4: Run the checks and make sure they pass.**
  - Run the browser tests again. Expected: 11 passed.
  - Run `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test rich_cards)`. Expected: pass.
  - Run `cargo fmt --all --check && cargo clippy -p nidoo-web --all-targets -- -D warnings && cargo check -p nidoo-web --target wasm32-unknown-unknown && cargo test -p nidoo-web`. Expected: clean, with every native test passing, including `bare_text`.
- [ ] **Step 5: Commit** with the message `feat(web): the vacuum card tile cleans its room in rows, heads home and charges`.
