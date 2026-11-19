# Large live pictures on the vacuum and climate cards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Draw #217's live AC and robot scenes as large pictures: the robot in its room replaces the vacuum card's big drawing, and every climate card shows the AC unit above its dial.

**Architecture:** `live_icon::view` gains a `LivePicture` component that renders the existing `AcIcon` / `VacuumIcon` with a `picture` flag (own CSS class, cropped viewBox for the AC, suffixed SVG ids). The phase store is keyed by entity and part, so a tile and a picture of one entity share one phase. The vacuum card swaps `components::robot::Robot` for it (the old drawing and its CSS are deleted); the climate card puts it above the `Dial`.

**Tech Stack:** Rust, Leptos 0.8 (csr), wasm-bindgen-test via wasm-pack, CSS in `crates/nidoo-web/styles/cards.css`.

**Spec:** `docs/superpowers/specs/2026-11-19-live-icon-pictures-design.md`

## Global Constraints

- Reuse #217's drawing and loop. No CSS keyframes and no second animation system for these pictures.
- No change to tile icons, state mapping, commands, pace tables or tones (`live_icon/{ac,robot,clock}.rs` stay as they are).
- The picture draws in `--ink`, like the tile (`.card__icon .live-icon`), so the toned parts stand out in both themes.
- Reduced motion: the pictures use the same `animations_on()` still poses as the tiles. Nothing extra.
- Commands, all from the repo root:
  - Browser tests: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test <file>)`. wasm-pack fetches chromedriver.
  - Gate: `cargo fmt --all --check && cargo clippy -p nidoo-web --all-targets -- -D warnings && cargo check -p nidoo-web --target wasm32-unknown-unknown && cargo test -p nidoo-web`.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Disk is tight (about 10 GB free): build only `nidoo-web`, and don't run the whole workspace.

## Review Focus

1. An offline AC draws the off pose in the picture too (vane closed, no air), not a moving stale state. Pinned in Task 3.
2. A tile and a picture of one entity must not share SVG ids (`live-air-…`, `live-fade-…`): a duplicate id makes the second mask resolve to the first. Pinned in Task 1.
3. A climate card in a narrow dashboard tile (the `climate` container under 180 px) must still fit the dial and the − / + buttons: the picture shrinks. Pinned in Task 3.
4. A vacuum in `error` or with no state draws the still idle robot in the picture, not the old drawing and not nothing. Pinned in Task 2.
5. The tile and the picture of one AC move in step: the same airflow offset in the same frame. Pinned in Task 3.

---

### Task 1: `LivePicture` in `live_icon::view`

**Files:**
- Modify: `crates/nidoo-web/src/live_icon/view.rs`
- Modify: `crates/nidoo-web/styles/cards.css` (the live-icon block, near line 832)
- Test: `crates/nidoo-web/tests/live_icon.rs`

**Interfaces:**
- Produces: `pub fn LivePicture(entity: EntityView) -> impl IntoView` (a `#[component]`) in `nidoo_web::live_icon::view`. Climate → `AcIcon` with `picture=true`; Vacuum → `VacuumIcon` with `picture=true`; any other kind → nothing. The root `<svg>` carries the class `live-icon--picture` beside its current classes.
- `AcIcon` and `VacuumIcon` gain `#[prop(optional)] picture: bool`. With it, the AC's `viewBox` is `"1 2.5 22 22"` (unit and airflow, no empty margins), and `svg_id` prefixes become `live-air-picture` / `live-fade-picture`.

- [ ] **Step 1: Write the failing tests** in `tests/live_icon.rs`. Add a `mount_picture(entity: EntityView) -> web_sys::Element` helper next to `mount`, which mounts `view! { <LivePicture entity=entity /> }` with the same contexts.
  - `picture_draws_the_cool_unit`: `mount_picture(climate("cool"))` → `has("svg.live-icon.live-icon--picture.live-icon--cool")`, `count(".live-icon__flow") == 3`, and the svg's `viewBox` attribute is `"1 2.5 22 22"`.
  - `picture_draws_the_cleaning_room`: `mount_picture(vacuum("cleaning"))` → `has("svg.live-icon--picture.live-icon--moving .live-icon__trail")`.
  - `picture_ids_differ_from_the_tile`: mount the climate card with `mount(climate("cool"))` and, in the same host, a `LivePicture` of the same entity. Then every `mask[id]` and `linearGradient[id]` id in the host is unique (collect the ids, check `len == dedup len`), and `count("mask") >= 2` (Task 3 adds the card's own picture, so don't pin an exact count).- [ ] **Step 2: Run them and check they fail.** Run: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test live_icon)`. Expected: a compile error, because `LivePicture` doesn't exist yet.
- [ ] **Step 3: Implement** the `picture` prop on both icons, plus `LivePicture`. The tile path (`picture=false`) must render exactly as today.
- [ ] **Step 4: CSS.** In `cards.css` after `.card__icon .live-icon`, add `.live-icon--picture { display: block; color: var(--ink); height: auto; stroke-width: 1.2; }`. Then thin the parts drawn with their own widths so a 150 px drawing isn't heavy: `.live-icon--picture .live-icon__robot`, `.live-icon__vane`, `.live-icon__flow` at about 1.0 to 1.1. Leave the trail at 4.4, since it represents the cleaned strip. CSS `stroke-width` overrides the SVG attributes. The exact values are the builder's call.
- [ ] **Step 5: Run the tests and check they pass.** Same command. Expected: every test in `live_icon` passes, the existing 13 included.
- [ ] **Step 6: Commit.** `feat(web): a large live picture for the AC and the robot vacuum`.

### Task 2: The vacuum card draws the robot in its room

**Files:**
- Modify: `crates/nidoo-web/src/cards/vacuum.rs:12,196-198`
- Delete: `crates/nidoo-web/src/components/robot.rs`; remove `pub mod robot;` from `crates/nidoo-web/src/components/mod.rs:16`
- Modify: `crates/nidoo-web/styles/cards.css`: remove `.robot` through the `robot-dust` keyframes and the reduced-motion block for `.robot*` (about lines 447–571). Keep `.vacuum__picture` and its phone rule.
- Modify: `crates/nidoo-web/styles/tokens.css`: remove `--robot-dust` (lines 63 and 158).
- Test: `crates/nidoo-web/tests/rich_cards.rs:373,381-394`, `crates/nidoo-web/tests/live_icon.rs:221-226`

**Interfaces:**
- Consumes: `LivePicture` from Task 1.

- [ ] **Step 1: Update the tests.**
  - `rich_cards.rs` `vacuum_tile_is_wide_with_sentence`: replace `has("svg.robot")` with `has(".vacuum__picture svg.live-icon--picture")`.
  - `rich_cards.rs`: replace `vacuum_drawing_sweeps_dust_only_while_cleaning` with `vacuum_picture_follows_the_activity`. `cleaning` → `has(".vacuum__picture .live-icon--moving .live-icon__trail")`; `docked` → `has(".vacuum__picture .live-icon--docked .live-icon__bolt")` and no `.live-icon__trail`; `error` → `has(".vacuum__picture .live-icon--plain")`; and in every case `!has("svg.robot")`.
  - `live_icon.rs`: rename `the_big_robot_picture_stays` to `the_big_picture_is_the_room` and assert `has(".vacuum__picture svg.live-icon--picture.live-icon--moving")` and `!has("svg.robot")`.
- [ ] **Step 2: Run them and check they fail.** Run: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test rich_cards --test live_icon)`. Expected: the three tests fail on the picture selectors.
- [ ] **Step 3: Implement.** In `vacuum.rs`, render `<LivePicture entity=… />` in place of `<Robot activity=activity />`. Clone the entity before `CardFrame` takes it, and drop `activity` there if nothing else needs it. Add `.vacuum__picture .live-icon--picture { width: 150px; max-width: 100%; }`. Then delete `robot.rs`, its `mod` line, the `.robot*` CSS and `--robot-dust`.
- [ ] **Step 4: Run the tests and check they pass.** Same command as Step 2, then `cargo test -p nidoo-web --test bare_text`. Expected: all pass. `bare_text`'s `"robot robot--{}"` is a literal fixture, not a reference.
- [ ] **Step 5: Commit.** `feat(web): the vacuum card's picture is the robot cleaning its room`.

### Task 3: The climate card shows the AC above its dial

**Files:**
- Modify: `crates/nidoo-web/src/cards/climate.rs` (the view, before `<Dial`)
- Modify: `crates/nidoo-web/styles/cards.css` (after `.climate`, near line 273)
- Test: `crates/nidoo-web/tests/live_icon.rs`

**Interfaces:**
- Consumes: `LivePicture` from Task 1.

- [ ] **Step 1: Write the failing tests** in `tests/live_icon.rs`.
  - `climate_card_shows_the_unit_above_the_dial`: `mount(climate("cool"))`. Then `has(".climate > .climate__picture svg.live-icon--picture.live-icon--cool")`, and `.climate__picture` comes before the dial in `.climate`'s children (compare the indexes of `.climate__picture` and the dial's root in `children()`).
  - `offline_picture_is_off`: offline `climate("cool")` → `has(".climate__picture svg.live-icon--off")` and `count(".climate__picture .live-icon__flow") == 0`.
  - `tile_and_picture_move_together`: `mount(climate("cool"))`, `sleep_ms(250)`. Read the first `.card__tile .live-icon__flow` and the first `.climate__picture .live-icon__flow` `stroke-dashoffset` with no `await` between the two reads; they must be equal.
  - `narrow_card_shrinks_the_picture`: call `with_styles()`, mount `climate("cool")`, set the host's `style` attribute to `width: 160px`, `tick().await`. Then the `.climate__picture svg` has a `getBoundingClientRect().width` of at most 72, and the − and + buttons are both still in the host's box (their `right` ≤ the host's `right`).
- [ ] **Step 2: Run them and check they fail.** Run: `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test live_icon)`. Expected: the four new tests fail on `.climate__picture`.
- [ ] **Step 3: Implement.** In `climate.rs`, clone the entity before `CardFrame` takes it and render `<div class="climate__picture"><LivePicture entity=… /></div>` as the first child of `.climate`, in every card place. In CSS: `.climate__picture .live-icon--picture { width: 110px; }` and `@container climate (max-width: 180px) { .climate__picture .live-icon--picture { width: 72px; } }`.
- [ ] **Step 4: Run the tests and check they pass.** The same command, then `(cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test rich_cards --test card_frame)`. Expected: all pass. The existing climate tests don't depend on the card's height.
- [ ] **Step 5: Run the gate.** Run the Global Constraints gate command. Expected: clean, with every native test passing.
- [ ] **Step 6: Commit.** `feat(web): every climate card shows the live AC above its dial`.
