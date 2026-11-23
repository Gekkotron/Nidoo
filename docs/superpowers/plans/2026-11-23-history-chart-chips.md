# History chart chips Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the stacked value rows on `/history/:id` with one full-size chart and a chip row: at most one number line plus at most one word value drawn as shaded periods.

**Architecture:** Three new pure modules under `components/history_chart/` hold the logic (periods, selection, what-to-plot), natively tested. `HistoryChart` takes over the `/history/fields` fetch from `FieldTracks`, renders the chip row, and draws a number (headline series or field track) with an optional period overlay, or a word value alone. `FieldTracks` is deleted. The page only wires the `show` URL parameter.

**Tech Stack:** Rust, Leptos 0.8 (CSR, wasm32), wasm-bindgen-test browser tests, plain CSS in `styles/main.css`.

**Spec:** `docs/superpowers/specs/2026-11-23-history-chart-chips-design.md`

## Global Constraints

- Every user-facing string goes through `src/i18n/` (FR and EN); `tests/bare_text.rs` fails CI on a bare literal. Run `cargo test -p nidoo-web` (all native targets), not just `--lib`.
- Browser tests: never `wasm-pack test` (links ~40 test wasms, ~16 GB, fills the disk). Run one file:
  ```
  C=~/Library/Caches/.wasm-pack
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=$C/wasm-bindgen-cargo-install-0.2.129/wasm-bindgen-test-runner \
  CHROMEDRIVER=$C/chromedriver-<dir whose --version major matches Chrome>/chromedriver \
  cargo test -p nidoo-web --target wasm32-unknown-unknown --test history_chips
  ```
  then `rm ~/dev-scratch/cargo-target/wasm32-unknown-unknown/debug/deps/*.wasm`.
- `nidoo-web` must keep building for wasm32: `cargo check -p nidoo-web --target wasm32-unknown-unknown`.
- Clippy clean: `cargo clippy -p nidoo-web --all-targets -- -D warnings`. `cargo fmt --all`.
- Leptos `view!`: a closure attribute right before `/>` must be braced (`on:click={move |_| …} />`) or the macro misparses.
- No change to `nidoo-api` or the hub (`crates/nidoo`).
- Energy counters (`Flavor::Counter`) render exactly as today: no chips, no fields fetch.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- URL key for the headline is `state`; for a field, its name. `show` lists number key first, then word key, comma-separated.
- Minimum drawn width of a coloured period: 3 px (CSS `width: max(<pct>, 3px)`).

## Review Focus

1. Changing the range with chips selected keeps `show` in the URL → Task 4 browser test `range_change_keeps_the_selection`.
2. A `show` key whose track is absent at the new range (no data in 1 h) falls back per slot, and to the default when nothing valid remains → Task 2 `parse_show_drops_unknown_keys_and_falls_back`.
3. Fields fetch fails for a vacuum (series reply has no points) → error + Retry, never a blank chart → Task 3 `fields_failure_with_an_empty_series_is_an_error`.
4. A word track that is entirely unknown (`null` all window) → no periods, empty legend, no panic → Task 1 `all_unknown_has_no_periods_or_legend`.
5. Entity with a single value (thermometer, no fields) → no chip row, chart as today → Task 2 `a_single_chip_hides_the_row` + Task 4 browser test `a_plain_sensor_has_no_chip_row`.

---

### Task 1: Periods (pure)

**Files:**
- Create: `crates/nidoo-web/src/components/history_chart/periods.rs`
- Modify: `crates/nidoo-web/src/components/history_chart/mod.rs` (add `pub mod periods;`)
- Source to move from: `crates/nidoo-web/src/components/field_tracks/plan.rs` (`x_frac`, `Band`, `value_key`, `bands`, `scalar_text`, `state_ctx`, `track_label`, `band_title`, `PALETTE`, and their tests). Leave `plan.rs` compiling (re-export or keep until Task 4 deletes it).

**Interfaces:**
- Produces:
  - `pub const PALETTE: usize = 5;`
  - `pub fn x_frac(t: i64, from: i64, to: i64) -> f64`
  - `pub struct Band { pub x0: f64, pub x1: f64, pub start: i64, pub end: i64, pub value: Option<Scalar>, pub color: Option<usize> }` (no `fits_label`)
  - `pub fn bands(changes: &[(i64, Option<Scalar>)], from: i64, to: i64) -> Vec<Band>` (unchanged behaviour)
  - `pub fn periods(changes: &[(i64, Option<Scalar>)], from: i64, to: i64) -> Vec<Band>` — `bands`, then adjacent equal values merged, then recoloured: unknown and the baseline get `color: None`; other values get palette slots by first appearance, skipping the baseline.
  - `pub fn baseline(bands: &[Band]) -> Option<Scalar>` — the non-null value with the longest total `end - start`; ties → first seen; `None` when fewer than 2 distinct non-null values.
  - `pub fn legend(periods: &[Band]) -> Vec<(Scalar, Option<usize>)>` — distinct non-null values in first-appearance order with their colour (`None` = baseline).
  - `pub fn value_at(changes: &[(i64, Option<Scalar>)], t: i64) -> Option<Scalar>` — value of the last change at or before `t`.
  - `pub fn scalar_text(lang, kind, field, unit, value) -> String`, `pub fn track_label(lang, kind, field) -> String`, `pub fn band_title(...)` — moved as-is.

- [ ] **Step 1: Move the helpers and their tests** into `periods.rs`; point `field_tracks/plan.rs` at them (`pub use`). Run `cargo test -p nidoo-web --lib periods` → the moved tests PASS.
- [ ] **Step 2: Write failing tests**

```rust
#[test]
fn adjacent_equal_values_merge() {
    let p = periods(&[(0, text("docked")), (10, text("docked")), (50, text("cleaning")), (60, text("docked"))], 0, 100);
    assert_eq!(p.len(), 3);
    assert_eq!((p[0].start, p[0].end), (0, 50));
}

#[test]
fn the_longest_value_is_the_uncoloured_baseline() {
    // docked 0–90 (90 s) vs cleaning 90–100 (10 s)
    let p = periods(&[(0, text("docked")), (90, text("cleaning"))], 0, 100);
    assert_eq!(p[0].color, None);
    assert_eq!(p[1].color, Some(0), "first non-baseline value takes palette slot 0");
    assert_eq!(baseline(&p), text("docked"));
}

#[test]
fn a_single_value_has_no_baseline_and_is_coloured() {
    let p = periods(&[(0, text("turbo"))], 0, 100);
    assert_eq!(baseline(&p), None);
    assert_eq!(p[0].color, Some(0));
}

#[test]
fn baseline_ties_go_to_the_first_seen_and_unknown_never_counts() {
    let p = periods(&[(0, None), (60, text("a")), (80, text("b"))], 0, 100);
    // null 60 s ignored; a 20 s, b 20 s → a
    assert_eq!(baseline(&p), text("a"));
    assert_eq!(p[0].color, None);
}

#[test]
fn legend_lists_each_value_once_in_order() {
    let p = periods(&[(0, text("docked")), (40, text("cleaning")), (45, text("docked")), (90, text("error"))], 0, 100);
    assert_eq!(legend(&p), vec![
        (Scalar::Text("docked".into()), None),
        (Scalar::Text("cleaning".into()), Some(0)),
        (Scalar::Text("error".into()), Some(1)),
    ]);
}

#[test]
fn all_unknown_has_no_periods_or_legend() {
    let p = periods(&[(0, None)], 0, 100);
    assert!(p.iter().all(|b| b.color.is_none()));
    assert!(legend(&p).is_empty());
}

#[test]
fn value_at_reads_the_change_in_force() {
    let c = [(0, text("a")), (50, text("b"))];
    assert_eq!(value_at(&c, 49), text("a"));
    assert_eq!(value_at(&c, 50), text("b"));
    assert_eq!(value_at(&c, -1), None);
}
```

- [ ] **Step 3: Run** `cargo test -p nidoo-web --lib periods` → new tests FAIL (not defined).
- [ ] **Step 4: Implement** `periods`, `baseline`, `legend`, `value_at` in `periods.rs` (compare values with the existing `value_key`).
- [ ] **Step 5: Run** `cargo test -p nidoo-web --lib periods` → PASS.
- [ ] **Step 6: Commit** `refactor(web): periods of a word value, with a quiet baseline`.

---

### Task 2: Selection (pure) + strings

**Files:**
- Create: `crates/nidoo-web/src/components/history_chart/selection.rs`
- Modify: `crates/nidoo-web/src/components/history_chart/mod.rs`, `crates/nidoo-web/src/i18n/history.rs`

**Interfaces:**
- Consumes: `track_label` (Task 1); `nidoo_api::{FieldTrack, TrackData}`.
- Produces:
  - `pub const HEADLINE_KEY: &str = "state";`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Slot { Number, Word }`
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Chip { pub key: String, pub slot: Slot, pub track: Option<usize> }` — `track` = index into `HistoryFields::tracks`; `None` for the headline number (drawn from `/history/series`).
  - `pub fn chips(tracks: &[FieldTrack]) -> Vec<Chip>` — if no track has `field == None`, a headline number chip `{state, Number, None}` first; then one chip per track in order: `field None` → `{state, Word, Some(i)}`, `Number` → `Number`, `Steps` → `Word` (even numeric scalars).
  - `#[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct Selection { pub number: Option<String>, pub word: Option<String> }`
  - `pub fn default_selection(chips: &[Chip]) -> Selection` — first chip in its slot; if it is a `Word`, also the first `Number` chip.
  - `pub fn parse_show(raw: Option<&str>, chips: &[Chip]) -> Selection` — split on `,`; first valid key per slot; nothing valid → `default_selection`.
  - `pub fn show_param(sel: &Selection) -> String` — `number,word`, skipping `None`.
  - `pub fn toggle(sel: &Selection, chip: &Chip) -> Selection` — selected chip clears its slot unless that leaves both empty (then unchanged); unselected chip replaces its slot.
  - `pub fn chip_label(lang: Lang, kind: Kind, device_class: Option<&str>, tracks: &[FieldTrack], chip: &Chip) -> String` — a track chip uses `track_label`; the headline number chip uses `field_label(lang, device_class)` when present, else `HistoryText::Value`.
  - `HistoryText::Value` ("Value" / "Valeur") and `HistoryText::ValuesShown` ("Values shown" / "Valeurs affichées", the chip row's `aria-label`).

- [ ] **Step 1: Write failing tests** (vacuum = `[headline steps, battery number, fan_speed steps]`, sensor = `[]`, climate-like = `[current number, hvac steps]` built with a small `track(field, number: bool)` helper)

```rust
#[test]
fn a_vacuum_gets_word_headline_then_its_fields() {
    let c = chips(&vacuum());
    assert_eq!(c.iter().map(|c| (c.key.as_str(), c.slot)).collect::<Vec<_>>(),
        [("state", Slot::Word), ("battery", Slot::Number), ("fan_speed", Slot::Word)]);
}

#[test]
fn a_numeric_headline_comes_first_from_the_series() {
    let c = chips(&[track(Some("battery"), true)]);
    assert_eq!(c[0], Chip { key: "state".into(), slot: Slot::Number, track: None });
    assert_eq!(c[1].track, Some(0));
}

#[test]
fn a_single_chip_hides_the_row() {
    assert_eq!(chips(&[]).len(), 1);
}

#[test]
fn vacuum_default_is_activity_and_battery() {
    assert_eq!(default_selection(&chips(&vacuum())),
        Selection { number: Some("battery".into()), word: Some("state".into()) });
}

#[test]
fn numeric_headline_default_is_the_line_alone() {
    assert_eq!(default_selection(&chips(&[])),
        Selection { number: Some("state".into()), word: None });
}

#[test]
fn parse_show_drops_unknown_keys_and_falls_back() {
    let c = chips(&vacuum());
    assert_eq!(parse_show(Some("battery,fan_speed,state"), &c),
        Selection { number: Some("battery".into()), word: Some("fan_speed".into()) });
    assert_eq!(parse_show(Some("ghost,fan_speed"), &c),
        Selection { number: None, word: Some("fan_speed".into()) });
    assert_eq!(parse_show(Some("ghost"), &c), default_selection(&c));
    assert_eq!(parse_show(None, &c), default_selection(&c));
}

#[test]
fn show_param_round_trips() {
    let s = Selection { number: Some("battery".into()), word: Some("state".into()) };
    assert_eq!(show_param(&s), "battery,state");
    assert_eq!(parse_show(Some(&show_param(&s)), &chips(&vacuum())), s);
}

#[test]
fn toggle_swaps_a_slot_clears_it_and_never_empties() {
    let c = chips(&vacuum());
    let s = default_selection(&c);
    let fan = &c[2];
    assert_eq!(toggle(&s, fan).word.as_deref(), Some("fan_speed"));
    let battery = &c[1];
    let no_line = toggle(&s, battery);
    assert_eq!(no_line, Selection { number: None, word: Some("state".into()) });
    assert_eq!(toggle(&no_line, &c[0]), no_line, "the last selected chip stays");
}

#[test]
fn chip_labels() {
    let t = vacuum();
    let c = chips(&t);
    assert_eq!(chip_label(Lang::Fr, Kind::Vacuum, None, &t, &c[0]), "Activité");
    assert_eq!(chip_label(Lang::Fr, Kind::Vacuum, None, &t, &c[2]), "Vitesse d'aspiration");
    let s = chips(&[]);
    assert_eq!(chip_label(Lang::Fr, Kind::Sensor, None, &[], &s[0]), "Valeur");
}
```

- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib selection` → FAIL.
- [ ] **Step 3: Implement** the interfaces above and the two `HistoryText` keys.
- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib selection` and `cargo test -p nidoo-web --test bare_text` → PASS.
- [ ] **Step 5: Commit** `feat(web): which values the history chart shows, as chips`.

---

### Task 3: What to plot (pure)

**Files:**
- Create: `crates/nidoo-web/src/components/history_chart/plot.rs`
- Modify: `crates/nidoo-web/src/components/history_chart/mod.rs`

**Interfaces:**
- Consumes: `Chip`, `Selection`, `Slot` (Task 2); `Flavor` (view.rs); `nidoo_api::{HistoryFields, HistorySeries, TrackData}`.
- Produces:
  - `#[derive(Clone, Debug)] pub enum FieldsState { Loading, Ready(HistoryFields), Failed(String) }` — `Failed` carries the error message; `NotRecorded`/`Unavailable`/`NoGridMeter` from `/history/fields` map to `Ready` with no tracks (the series side reports those).
  - `#[derive(Clone, Debug, PartialEq)] pub enum Plot { Loading, Bars, Headline { overlay: Option<usize> }, Field { track: usize, overlay: Option<usize> }, Words { track: usize }, FieldsError(String) }` — `overlay`/`track` index `HistoryFields::tracks`.
  - `pub fn plot_for(flavor: Flavor, fields: &FieldsState, sel: &Selection, chips: &[Chip]) -> Plot` — `Counter` → `Bars` (fields ignored). `Loading` → `Loading`. `Failed(m)` → `Headline { overlay: None }` (the view turns it into `FieldsError(m)` via `headline_or_error`). `Ready` → number chip `None` track → `Headline`, number chip `Some(i)` → `Field`, no number → `Words`; `overlay` = the word chip's track.
  - `pub fn headline_or_error(fields: &FieldsState, series: &HistorySeries) -> Option<String>` — `Some(msg)` when fields `Failed(msg)` and the series has no points.
  - `pub fn field_series(track: &FieldTrack) -> Option<(HistorySeries, Option<String>)>` — a `Number` track as a series plus its unit; `None` for `Steps`.
  - `track_scale` moved here from `field_tracks/plan.rs` (with its test).

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn counters_keep_their_bars() {
    assert_eq!(plot_for(Flavor::Counter, &FieldsState::Loading, &Selection::default(), &[]), Plot::Bars);
}

#[test]
fn waits_for_the_fields_reply() {
    assert_eq!(plot_for(Flavor::Series, &FieldsState::Loading, &sel(Some("state"), None), &[]), Plot::Loading);
}

#[test]
fn vacuum_default_is_battery_over_activity() {
    let f = ready(vacuum_tracks());
    let c = chips(&vacuum_tracks());
    assert_eq!(plot_for(Flavor::Series, &f, &default_selection(&c), &c),
        Plot::Field { track: 1, overlay: Some(0) });
}

#[test]
fn a_word_alone_fills_the_chart() {
    let f = ready(vacuum_tracks());
    let c = chips(&vacuum_tracks());
    assert_eq!(plot_for(Flavor::Series, &f, &sel(None, Some("fan_speed")), &c), Plot::Words { track: 2 });
}

#[test]
fn a_numeric_headline_draws_from_the_series() {
    let t = vec![track(Some("hvac"), false)];
    let c = chips(&t);
    assert_eq!(plot_for(Flavor::Series, &ready(t), &sel(Some("state"), Some("hvac")), &c),
        Plot::Headline { overlay: Some(0) });
}

#[test]
fn fields_failure_with_an_empty_series_is_an_error() {
    let failed = FieldsState::Failed("boom".into());
    let empty = HistorySeries { window: 3600, points: vec![] };
    assert_eq!(headline_or_error(&failed, &empty), Some("boom".into()));
    let some = HistorySeries { window: 60, points: vec![(0, 1.0, 1.0, 1.0)] };
    assert_eq!(headline_or_error(&failed, &some), None);
    assert_eq!(headline_or_error(&ready(vec![]), &empty), None);
}

#[test]
fn a_number_track_becomes_a_series_with_its_unit() {
    let (s, unit) = field_series(&vacuum_tracks()[1]).unwrap();
    assert_eq!(unit.as_deref(), Some("%"));
    assert!(!s.points.is_empty());
    assert!(field_series(&vacuum_tracks()[2]).is_none());
}
```

- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib plot` → FAIL.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib plot` → PASS.
- [ ] **Step 5: Commit** `feat(web): decide what the history chart draws`.

---

### Task 4: HistoryChart owns the fields, the chips and the number line; FieldTracks goes

**Files:**
- Modify: `crates/nidoo-web/src/components/history_chart/view.rs`, `crates/nidoo-web/src/screens/history_detail.rs`, `crates/nidoo-web/src/components/mod.rs`, `crates/nidoo-web/styles/main.css`
- Delete: `crates/nidoo-web/src/components/field_tracks/` (both files), the `.field-tracks*` CSS block, `crates/nidoo-web/tests/field_tracks.rs`
- Create: `crates/nidoo-web/tests/history_chips.rs` (reuse the harness from the deleted `field_tracks.rs`: `vacuum()`, `vacuum_fields()`, `mount`, `all`, `texts`, `wait_for`; also stub `/api/v1/history/series?entity=vacuum.roborock&range=24h&`)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `HistoryChart` new props: `#[prop(into)] show: Signal<Option<String>>` (raw `show` query value), `on_show: Callback<String>` (called with `show_param(&new_selection)`), `#[prop(into)] device_class: Signal<Option<String>>`. `kind` stays a `Signal<String>`; parse it to `Kind` where `chip_label` needs it, or add a `Signal<Kind>` prop — builder's call.
  - The fields fetch (moved from `FieldTracks`, same stale-request guard and refresh tick) runs in the same effect as the series fetch, only when `flavor == Series`; Retry refetches both.
  - Chip row: `<div class="chips history-chart__chips" role="group" aria-label=ValuesShown>` with one `<button class="chip" aria-pressed>` per chip, rendered only when `chips.len() > 1` and fields are `Ready`.
  - `render_series` takes the series, unit and an `Option<Overlay>` parameter (`None` in this task; Task 5 fills it). For `Plot::Field`, the series and unit come from `field_series`; a `%` unit uses `track_scale` (0–100) instead of `nice_scale`.
  - Page: drop `FieldTracks`, the `headline` signal and its `<Show>`; pass `show` from `query.get("show")`, `device_class` from the entity, and `on_show` that navigates (replace) to `/history/{id}?range={slug}&show={param}`. `on_range` keeps the current `show` when present.
  - `Plot::Words` renders the shimmer-free empty plot area for now (Task 5 draws it).

- [ ] **Step 1: Write the browser tests** in `tests/history_chips.rs`

```rust
#[wasm_bindgen_test]
async fn a_vacuum_opens_on_battery_and_activity() {
    // stub fields (headline=true) and series (empty points)
    let m = mount(Lang::Fr, "/history/vacuum.roborock?range=24h");
    wait_for(&m.host, ".history-chart__chips .chip", 3).await;
    assert_eq!(texts(&m.host, ".history-chart__chips .chip"), ["Activité", "Batterie", "Vitesse d'aspiration"]);
    assert_eq!(texts(&m.host, ".history-chart__chips .chip[aria-pressed=true]"), ["Activité", "Batterie"]);
    assert_eq!(all(&m.host, ".history-chart__line").len(), 1, "battery line drawn");
    assert!(all(&m.host, ".field-tracks").is_empty());
}

#[wasm_bindgen_test]
async fn tapping_a_word_chip_swaps_the_word_and_writes_the_url() {
    // click "Vitesse d'aspiration"; expect pressed = ["Batterie", "Vitesse d'aspiration"]
    // and location.search contains "show=battery%2Cfan_speed" or "show=battery,fan_speed"
}

#[wasm_bindgen_test]
async fn range_change_keeps_the_selection() {
    // mount at ?range=24h&show=battery,fan_speed; stub the 7d fields+series URLs too;
    // click the "7 j" chip; location.search contains both "range=7d" and "show=battery"
}

#[wasm_bindgen_test]
async fn a_plain_sensor_has_no_chip_row() {
    // fields reply with no tracks, series with two points
    // → no .history-chart__chips, one .history-chart__line
}
```

- [ ] **Step 2: Run** the browser file (command in Global Constraints) → FAIL (no chips).
- [ ] **Step 3: Implement** the interfaces above; delete `field_tracks` and its CSS; add `.history-chart__chips { margin: 0 0 10px; flex-wrap: wrap; }`.
- [ ] **Step 4: Run** `cargo test -p nidoo-web`, `cargo clippy -p nidoo-web --all-targets -- -D warnings`, `cargo check -p nidoo-web --target wasm32-unknown-unknown`, then the browser file → all PASS.
- [ ] **Step 5: Commit** `feat(web): one history chart with value chips`.

---

### Task 5: Word periods — overlay, alone, legend, tooltip

**Files:**
- Modify: `crates/nidoo-web/src/components/history_chart/view.rs`, `crates/nidoo-web/styles/main.css`, `docs/users/history.md`
- Test: `crates/nidoo-web/tests/history_chips.rs`

**Interfaces:**
- Consumes: `periods`, `legend`, `value_at`, `scalar_text`, `band_title` (Task 1); `Plot` (Task 3); `render_series(…, overlay: Option<Overlay>)` (Task 4).
- Produces:
  - `struct Overlay { changes: Vec<(i64, Option<Scalar>)>, periods: Vec<Band>, words: Vec<String> /* per legend entry */, legend: Vec<(Scalar, Option<usize>)> }`, built from a Steps track + `from`/`to` of the fields reply (the chart's x window is `now - span .. now`; use the same `t_start`/`now` as the line so they align).
  - Periods layer: an absolutely positioned `<div class="history-chart__periods">` inside `.history-chart__plot`, behind the SVG, one `<i class="history-chart__period history-chart__period--{n}" style="left: X%; width: max(W%, 3px)" title=band_title(..)>` per period with `color: Some(n)`. Baseline and unknown periods are not drawn.
  - Opacity: alone `.history-chart__periods` at 0.55; with a line, modifier `--behind` at 0.18. Colours `--accent-cyan`, `--accent-mint`, `--accent-amber`, `--accent-violet`, `--accent-rose` for n = 0..4 (builder may tune).
  - Legend: under the chart in `.history-chart__legend`, one `<span class="history-chart__key-item">` per entry with `<i class="history-chart__swatch history-chart__swatch--{n}">` (empty outline swatch for the baseline) and the word from `scalar_text`.
  - Tooltip: when an overlay is present, a `<span class="history-chart__tip-word">` with `scalar_text(value_at(changes, hovered_t))` under the value.
  - `Plot::Words`: the plot box with only the periods layer (opacity 0.55), x axis, legend; hovering shows the time and the word in force.
  - `Plot::FieldsError(msg)` (from `headline_or_error`) renders the existing error block with Retry.
  - `docs/users/history.md`: replace the "one row per value" paragraph with the chips behaviour (FR app wording: Activité, Batterie…).

- [ ] **Step 1: Write the browser tests**

```rust
#[wasm_bindgen_test]
async fn activity_shades_the_cleaning_behind_the_battery_line() {
    // default vacuum mount: exactly one .history-chart__period (cleaning), modifier --behind on the layer,
    // legend texts == ["À la base", "Nettoyage"]
}

#[wasm_bindgen_test]
async fn a_word_alone_has_periods_and_no_line() {
    // mount with show=fan_speed: no .history-chart__line, one .history-chart__period ("Turbo", single value → coloured),
    // legend == ["Turbo"]
}

#[wasm_bindgen_test]
async fn a_failed_fields_reply_with_no_series_offers_retry() {
    // fields 500, series {"window":3600,"points":[]} → .history-chart__err with a Retry button
}
```

- [ ] **Step 2: Run** the browser file → the three new tests FAIL.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** `cargo fmt --all`, `cargo test -p nidoo-web`, clippy, the wasm32 check and the browser file → all PASS. Free the test wasms afterwards.
- [ ] **Step 5: Commit** `feat(web): word values shade the history chart, with a legend`.
