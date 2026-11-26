# History period picker and year comparison — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On the history page, pick a calendar period (Day/Week/Month/Year/custom), step through
it, and compare it with the same period of an earlier year, backed by hub routes that accept any
date window.

**Architecture:** The hub's `series`, `fields` and `daily` routes (HTTP and MCP) accept a window
of house-local dates (`from`, `to`, optional `upto`) besides today's presets, pick the step from
the window's length, and report the window's real bounds (`span`). A new `extent` route gives the
entity's first date and the house's today. The web works out calendar periods and the compared
window as pure functions, fetches both windows, aligns them by bucket index and computes the
summary; the `HistoryChart` component draws the compared series behind the shown one.

**Tech Stack:** Rust workspace: `crates/nidoo-api` (wire types, serde + schemars),
`crates/nidoo-core` (bucket maths), `crates/nidoo` (axum hub, sqlx/TimescaleDB, rmcp tools),
`crates/nidoo-web` (Leptos 0.8 CSR, wasm, chrono + chrono-tz).

**Spec:** `docs/superpowers/specs/2026-11-26-history-period-compare-design.md`

## Global Constraints

- Every user-facing web string goes through `crates/nidoo-web/src/i18n/` (FR and EN) or is listed in `crates/nidoo-web/tests/bare_text_allow.txt`; after web changes run `cargo test -p nidoo-web` (all native targets), never only `--lib`.
- Never run `wasm-pack test` over the web crate (it links ~40 browser test binaries, ~16 GB). Run one browser test file with cargo and wasm-pack's cached runner (Task 12 gives the command); compile-check others with `cargo test -p nidoo-web --target wasm32-unknown-unknown --no-run --test <file>`.
- Leptos: closure attributes on a self-closing tag are braced (`on:click={move |_| …} />`), or `view!` misparses `/>`.
- No real room, device, host or person names in code, tests or docs (public repo). Use `sensor.t`, `counter.a`, "Living room".
- Hub integration tests compile as one binary: `cargo test -p nidoo --test integration <module>`; DB tests skip without `TEST_DATABASE_URL` (no Postgres on this Mac), so they must compile and are checked in CI.
- Existing callers keep working unchanged: `range=1h|6h|24h|7d|30d|1y`, `days=7|30|365`, dashboard `"range"` values, MCP `range`/`days`, links `/history/:id?range=…`.
- A day is a house-local day (the hub's zone, `History::zone()`), never the browser's.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Shared cargo target: a "No such file" build error is a concurrent clean in another worktree; retry.

## Review Focus

1. **DST days and windows across a DST change** (29 Mar / 25 Oct in Paris): a Day window is 23 or 25 h long; hourly bars must not drop or duplicate an hour, and day buckets of the compared year must still pair day 1 with day 1. → Task 3 `resolve_spans_dst_days`, Task 9 `bucket_index_tolerates_dst_hour`.
2. **29 February** in the shown or compared year: the shifted date exists or maps to 28 February, and the unmatched day is reported as left out, not as a saving. → Task 8 `compare_span_maps_feb_29`, Task 9 `counter_summary_leaves_out_unmatched_days`.
3. **Comparing a running period**: Nov 1–26 15:40 against Nov 1–26 15:40 a year earlier, including a partial last month on a Year view. → Task 4 `daily_window_upto_cuts_partial_month` (DB), Task 11 sequencing test.
4. **Periods before the first reading or entirely empty**: ‹ disabled at the first date, an empty compared window shows "No data for …" and no difference card; an empty window is a 200, never a 404. → Task 4 `series_window_empty_is_ok`, Task 8 `can_step_stops_at_first_date`, Task 9 `summary_none_when_compared_empty`.
5. **Week 53**: ISO week 53 of 2026 has no counterpart in 2025; the compared span falls back to week 52. → Task 8 `compare_span_week_53_falls_back`.

---

### Task 1: Wire types for windows

**Files:**
- Modify: `crates/nidoo-api/src/history.rs`
- Modify: every struct literal of `HistorySeries` / `HistoryDaily` in the workspace (`grep -rn "HistorySeries {\|HistoryDaily {" crates`)

**Interfaces:**
- Produces:
  - `pub enum DailyPer { Hour, Day, Month }`, serde `snake_case` (`"hour"`, `"day"`, `"month"`), `JsonSchema`, `Default` = `Day`, `pub fn parse(s: &str) -> Option<DailyPer>`.
  - `pub struct HistoryExtent { pub first_date: Option<String>, pub today: String }` (dates `YYYY-MM-DD`).
  - New field on `HistorySeries` and `HistoryDaily`: `#[serde(default, skip_serializing_if = "Option::is_none")] pub span: Option<(i64, i64)>` — `[start, end]` unix seconds, `None` for preset requests.
  - All exported from `nidoo_api` like the existing history types.

- [ ] **Step 1: Write the failing tests** in the existing `tests` module of `history.rs`:
  - `daily_per_parses_and_serialises`: `DailyPer::parse("month") == Some(DailyPer::Month)`, `parse("week") == None`, `serde_json::to_value(DailyPer::Hour) == json!("hour")`.
  - `series_span_is_omitted_when_none`: a series with `span: None` serialises to exactly `{"window":600,"points":[[1,2.0,1.0,3.0]]}`; with `span: Some((10, 20))` it carries `"span":[10,20]`; `{"window":600,"points":[]}` deserialises with `span == None`.
  - `extent_serialises_dates`: `HistoryExtent { first_date: None, today: "2026-11-26".into() }` → `{"first_date":null,"today":"2026-11-26"}`.
- [ ] **Step 2: Run** `cargo test -p nidoo-api history` — expect compile errors (types missing).
- [ ] **Step 3: Implement** the three items; add `span: None` to every existing struct literal.
- [ ] **Step 4: Run** `cargo test -p nidoo-api` and `cargo check --workspace --all-targets` — expect pass.
- [ ] **Step 5: Commit** `feat(history): wire types for date windows`.

### Task 2: Merge buckets from a window origin

**Files:**
- Modify: `crates/nidoo-core/src/history/buckets.rs` (and its existing test module)

**Interfaces:**
- Produces: `pub fn merge_from(buckets: &[Bucket], origin: Micros, window: Micros) -> Vec<Bucket>` — same aggregation as `merge`, but groups start at `origin + ((b.start - origin).div_euclid(window)) * window`. `merge(b, w)` becomes `merge_from(b, 0, w)`.

- [ ] **Step 1: Write the failing test** `merge_from_groups_from_origin`: hourly buckets starting at `origin = 23 * HOUR` (6 of them, `increase` 1.0 each) merged with `window = 5 * HOUR` give 2 buckets starting at `23 * HOUR` (increase 5.0) and `28 * HOUR` (increase 1.0). With `merge(…, 5 * HOUR)` the first group would start at `20 * HOUR` — assert that too, so the difference is pinned.
- [ ] **Step 2: Run** `cargo test -p nidoo-core merge_from` — FAIL (not defined).
- [ ] **Step 3: Implement** `merge_from`; make `merge` delegate to it.
- [ ] **Step 4: Run** `cargo test -p nidoo-core` — PASS.
- [ ] **Step 5: Commit** `feat(history): merge buckets from a window origin`.

### Task 3: Hub window resolution (pure)

**Files:**
- Create: `crates/nidoo/src/history/window.rs`, `crates/nidoo/src/history/window/tests.rs`
- Modify: `crates/nidoo/src/history/mod.rs` (`pub mod window;`), `crates/nidoo/src/history/query.rs` (make `local_midnight` `pub(crate)`)

**Interfaces:**
- Consumes: `query::local_midnight`, `query::local_date`, `HistoryError::BadRequest`, `DailyPer` (Task 1).
- Produces:
  - `pub struct DateWindow { pub from: NaiveDate, pub to: NaiveDate, pub upto: Option<i64> }` (`to` inclusive, `upto` seconds).
  - `pub fn parse_window(from: Option<&str>, to: Option<&str>, upto: Option<&str>) -> Result<Option<DateWindow>, HistoryError>` — `Ok(None)` when neither `from` nor `to`; one without the other, a bad date, `to < from`, or `upto <= 0` → `BadRequest`.
  - `pub struct Resolved { pub start: Micros, pub end: Micros }`.
  - `pub fn resolve(zone: Tz, w: &DateWindow, now: Micros) -> Result<Resolved, HistoryError>` — `start = local_midnight(from)`, `end = min(local_midnight(to + 1 day), start + upto, now)`; `from` after `local_date(zone, now)` → `BadRequest`.
  - `pub fn days(w: &DateWindow) -> i64` (inclusive day count).
  - `pub fn step_secs(days: i64) -> i64` — 1 → 600; ≤ 8 → 3 600; ≤ 32 → 18 000; ≤ 400 → 86 400; else 604 800.
  - `pub fn check_limits(w: &DateWindow, what: WindowUse) -> Result<(), HistoryError>` with `pub enum WindowUse { Series, Fields, Daily(DailyPer) }` — Fields over 400 days, `Daily(Hour)` over 2 days, `Daily(Day)` over 1 100 days → `BadRequest`.
  - `pub fn boundaries(zone: Tz, w: &DateWindow, r: &Resolved, per: DailyPer) -> Vec<Micros>` — bucket starts, then `r.end` as the closing boundary: every `HOUR` from `start` (Hour), each local midnight from `from` (Day), each local first-of-month from `from`'s month start clamped to `start` (Month); starts at or after `r.end` are dropped.

- [ ] **Step 1: Write the failing tests** in `window/tests.rs` (zone `chrono_tz::Europe::Paris`, `now` = 2026-11-26 15:40 Paris):
  - `parse_window_rules`: `(None, None, None)` → `Ok(None)`; `("2025-11-01", None, None)` → Err; `("2025-11-30", "2025-11-01", None)` → Err; `upto = "0"` → Err; `("2025-11-01","2025-11-30","3600")` → `upto == Some(3600)`.
  - `resolve_clips_at_now_and_upto`: window 2026-11-01..=2026-11-30 → `end == now`; same with `upto = 86_400` → `end == start + 86_400 s`; window 2026-11-27..=2026-11-27 → Err (after today).
  - `resolve_spans_dst_days`: 2026-03-29..=2026-03-29 → `end - start == 23 h`; 2026-10-25..=2026-10-25 → 25 h.
  - `step_secs_by_length`: 1→600, 7→3 600, 8→3 600, 9→18 000, 31→18 000, 365→86 400, 400→86 400, 401→604 800.
  - `check_limits_caps`: Fields 401 days Err, 400 Ok; Daily(Hour) 3 days Err; Daily(Day) 1 101 days Err; Daily(Month) 3 000 days Ok; Series 3 000 days Ok.
  - `boundaries_per_day_month_hour`: Day over 2025-11-01..=2025-11-03 (past) → 4 values (3 midnights + end); Month over 2024-12-15..=2025-02-10 → starts 2024-12-15 00:00, 2025-01-01, 2025-02-01, then end; Hour over 2026-03-29 → 23 starts + end.
- [ ] **Step 2: Run** `cargo test -p nidoo --lib history::window` — FAIL (module missing).
- [ ] **Step 3: Implement** `window.rs` with the signatures above.
- [ ] **Step 4: Run** the same command — PASS.
- [ ] **Step 5: Commit** `feat(history): resolve house-local date windows`.

### Task 4: Hub queries over a window, and the extent

**Files:**
- Modify: `crates/nidoo/src/history/query.rs`
- Test: `crates/nidoo/tests/history_db.rs`

**Interfaces:**
- Consumes: Task 2 `merge_from`; Task 3 `DateWindow`, `resolve`, `days`, `step_secs`, `check_limits`, `boundaries`; existing `hours`, `rollup`, `fill`, `kwh`, `build_tracks`, `Db::first_time_of`.
- Produces:
  - `pub async fn series_window(h: &History, entity: &str, w: &DateWindow, now: Micros) -> Result<HistorySeries, HistoryError>` — 1-day windows roll raw samples (as `Day` today, with `series_carry_at`) aligned at `start`; longer windows use `hours(h, entity, start, end)` then `merge_from(.., start, step)`; `window = step_secs(days(w))`; `span = Some((start/1e6, end/1e6))`. Factor the bucket→points mapping out of `series` into a shared `fn points_of(buckets: &[Bucket]) -> Vec<(i64, f64, f64, f64)>`.
  - `pub async fn fields_window(h: &History, entity: &Entity, w: &DateWindow, now: Micros) -> Result<HistoryFields, HistoryError>` — `check_limits(Fields)`, then the body of `fields` with `from = start`, `to = end`, `window = step_secs(days(w))` µs.
  - `pub async fn daily_window(h: &History, entity: &str, w: &DateWindow, per: DailyPer, kind: EnergyKind, now: Micros) -> Result<HistoryDaily, HistoryError>` — `check_limits(Daily(per))`, `hours(h, entity, start, end)`, one `kwh` per consecutive pair of `boundaries`, keeping `Some` only, rounded with `round3`; `span` set.
  - `pub async fn extent(h: &History, entity: &str, now: Micros) -> Result<HistoryExtent, HistoryError>` — 404 when not recorded; `first_date` = local date of `db.first_time_of(&[entity])`; `today = local_date(zone, now)`.
  - Every window query: an entity never recorded → `NotRecorded`; a recorded entity with nothing in the window → `Ok` with empty `points` / `days`.

- [ ] **Step 1: Write the failing DB tests** in `history_db.rs` (UTC zone via `History::with_db`, `rollup_once` before querying, as `series_24h_carries_into_quiet_windows` does):
  - `series_window_month_uses_5h_steps`: values every hour over 30 days → `series.window == 18_000`, `span == Some((start, end))` with `start` = midnight of `from`, first point time == `start`.
  - `series_window_empty_is_ok`: entity recorded only after the window → `Ok`, `points.is_empty()`.
  - `daily_window_per_month_sums_counter`: a counter rising 1.0 per hour from 2025-01-01 to 2025-03-01 → `per=Month` over 2025-01-01..=2025-02-28 gives `[(jan_start, 744.0), (feb_start, 672.0)]`.
  - `daily_window_upto_cuts_partial_month`: same data, window 2025-01-01..=2025-12-31 with `upto` = 40 days → two months, the second = 9 × 24 = 216.0.
  - `extent_reports_first_date_and_today`: first value 2025-03-10 12:00 UTC, `now` 2026-11-26 → `first_date == Some("2025-03-10")`, `today == "2026-11-26"`; unknown entity → `Err(NotRecorded)`.
- [ ] **Step 2: Run** `cargo test -p nidoo --test integration history_db --no-run` — compile errors for the missing functions (the tests themselves skip without a DB).
- [ ] **Step 3: Implement** the four functions in `query.rs`.
- [ ] **Step 4: Run** `cargo test -p nidoo --test integration history_db` (prints `skipping` locally; must compile) and `cargo test -p nidoo --lib history` — PASS. If a DB is reachable (`TEST_DATABASE_URL`), the five tests must pass.
- [ ] **Step 5: Commit** `feat(history): series, fields and daily over a date window; extent`.

### Task 5: HTTP routes

**Files:**
- Modify: `crates/nidoo/src/http/api.rs` (handlers `history_series`, `history_fields`, `history_daily`; new `history_extent`; route `/history/extent`)
- Test: `crates/nidoo/tests/http.rs`

**Interfaces:**
- Consumes: Task 3 `parse_window`, `check_limits`; Task 4 queries; `DailyPer::parse`.
- Produces: `GET /history/series|fields?entity=&from=&to=[&upto=]`, `GET /history/daily?entity=&kind=&from=&to=[&per=][&upto=]`, `GET /history/extent?entity=` → `HistoryExtent`. Exactly one of `range` / window (series, fields) or `days` / window (daily); otherwise 400. All parameter and limit checks run before `require_history`, so they answer 400 without a database. `per` without a window is 400.

- [ ] **Step 1: Write the failing test** `history_window_params_are_400` (detached history, like `history_bad_params_are_400`), asserting 400 for: `series?entity=x&range=24h&from=2025-11-01&to=2025-11-30`; `series?entity=x&from=2025-11-01`; `series?entity=x&from=2025-11-30&to=2025-11-01`; `daily?entity=x&kind=counter&days=7&from=2025-11-01&to=2025-11-30`; `daily?entity=x&kind=counter&from=2025-11-01&to=2025-11-03&per=hour`; `daily?entity=x&kind=counter&days=7&per=month`; `fields?entity=x&from=2020-01-01&to=2021-12-31`. And `history_extent_without_history_is_503`: `GET /api/v1/history/extent?entity=x` → 503 `{"error":"history_unavailable"}`; missing `entity` → 400.
- [ ] **Step 2: Run** `cargo test -p nidoo --test integration http::history_window` — FAIL.
- [ ] **Step 3: Implement** the handler changes and the route; update the handlers' doc comments.
- [ ] **Step 4: Run** `cargo test -p nidoo --test integration http` — PASS (existing history tests included).
- [ ] **Step 5: Commit** `feat(history): date windows and extent on the HTTP API`.

### Task 6: MCP `get_history`

**Files:**
- Modify: `crates/nidoo/src/mcp/params.rs` (`HistoryWhat`, `GetHistoryParams`, `HistoryAnswer`), `crates/nidoo/src/mcp/tools.rs` (`get_history` and its tool description)
- Test: `crates/nidoo/tests/mcp.rs`

**Interfaces:**
- Consumes: Tasks 3–4.
- Produces: `HistoryWhat::Extent` (`"extent"`); `GetHistoryParams` gains `from: Option<String>`, `to: Option<String>`, `upto: Option<i64>`, `per: Option<DailyPer>` (doc comments state the rules of Task 5); `HistoryAnswer` gains `extent: Option<HistoryExtent>`. Same one-of rules as HTTP, errors as `ToolError("bad_request: …")`. The tool description adds: "For a long window use `daily` with `per = month`, or a series; windows are house-local dates, `to` inclusive."

- [ ] **Step 1: Write the failing tests**: `get_history_schema_lists_window_params` (the tool's input schema has `from`, `to`, `per`, `upto` and `what` enum contains `"extent"`); `get_history_window_and_range_is_an_error` — with history attached detached, `{"entity":"x","what":"series","range":"24h","from":"2025-11-01","to":"2025-11-30"}` returns an error mentioning `range` and `from`.
- [ ] **Step 2: Run** `cargo test -p nidoo --test integration mcp::get_history` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p nidoo --test integration mcp` — PASS.
- [ ] **Step 5: Commit** `feat(mcp): get_history over any date window, and extent`.

### Task 7: Web API client

**Files:**
- Modify: `crates/nidoo-web/src/api/history.rs` (and its tests)

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct WindowReq { pub from: NaiveDate, pub to: NaiveDate, pub upto: Option<i64> }`
  - `pub(crate) fn series_window_url(entity: &str, w: &WindowReq) -> String`, `fields_window_url(entity, w)`, `daily_window_url(entity, w, per: DailyPer, kind: EnergyKind)`, `extent_url(entity)` — same encoding as the existing `*_url` helpers; `&upto=N` only when set.
  - `pub async fn fetch_series_window(entity, w) -> Result<HistorySeries, HistoryApiError>`, `fetch_fields_window`, `fetch_daily_window(entity, w, per, kind)`, `fetch_extent(entity) -> Result<HistoryExtent, HistoryApiError>`.

- [ ] **Step 1: Write the failing tests**: `series_window_url_has_dates` → `"/api/v1/history/series?entity=sensor.t&from=2025-11-01&to=2025-11-30"`; with `upto: Some(3600)` it ends `&upto=3600`; `daily_window_url_has_per` → `…/daily?entity=counter.a&from=2025-01-01&to=2025-12-31&per=month&kind=counter`; `extent_url_is_plain` → `"/api/v1/history/extent?entity=sensor.t"`.
- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib api::history` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same — PASS.
- [ ] **Step 5: Commit** `feat(web): history API client for date windows`.

### Task 8: Calendar periods (pure)

**Files:**
- Create: `crates/nidoo-web/src/history_period.rs`, `crates/nidoo-web/src/history_period/tests.rs`
- Modify: `crates/nidoo-web/src/lib.rs` (`pub mod history_period;`)

**Interfaces:**
- Consumes: `DailyPer`, `HistoryRange`.
- Produces:
  - `pub enum Unit { Hour1, Hour6, Day, Week, Month, Year, Custom }` with `slug()` (`"1h"`, `"6h"`, `"day"`, `"week"`, `"month"`, `"year"`, `"custom"`) and `from_slug`.
  - `pub struct Span { pub from: NaiveDate, pub to: NaiveDate }` (inclusive), `pub fn days(&self) -> i64`.
  - `pub struct PeriodState { pub unit: Unit, pub at: Option<NaiveDate>, pub custom: Option<Span>, pub vs: Option<u8>, pub show: Option<String> }`.
  - `pub fn parse_state(get: impl Fn(&str) -> Option<String>) -> PeriodState` — reads `period`, `at`, `from`, `to`, `vs`, `show`; legacy `range=24h|7d|30d|1y` → Day/Week/Month/Year current, `range=1h|6h` → Hour1/Hour6; nothing → Day current; `custom` without valid `from`/`to` → Day current.
  - `pub fn state_path(id: &str, s: &PeriodState) -> String` — `/history/{id}?period=…[&at=…][&from=…&to=…][&vs=…][&show=…]` (id encoded as `history_path` does today).
  - `pub fn span_of(unit: Unit, at: NaiveDate) -> Span` — Day; Week = ISO week Monday–Sunday; Month; Year. Not for Hour1/Hour6/Custom.
  - `pub fn shown_span(s: &PeriodState, today: NaiveDate) -> Option<Span>` — `None` for Hour1/Hour6.
  - `pub fn is_current(s: &PeriodState, today: NaiveDate) -> bool`.
  - `pub fn step(s: &PeriodState, by: i32, today: NaiveDate) -> PeriodState` — one unit (custom: by its own length); `at` becomes `None` when the result is the current period.
  - `pub fn can_step(s: &PeriodState, by: i32, today: NaiveDate, first: Option<NaiveDate>) -> bool` — › false when the next span starts after `today`; ‹ false when the shown span starts on or before `first`.
  - `pub fn switch_unit(s: &PeriodState, unit: Unit, today: NaiveDate) -> PeriodState` — keeps the shown span's `from` (or today when current); keeps `vs` and `show`.
  - `pub fn compare_span(unit: Unit, shown: Span, vs: u8) -> Span` — dates shifted by `vs` years, 29 Feb → 28 Feb; Week = the same ISO week number (week 53 → week 52 when the target year has none).
  - `pub fn compare_years(shown: Span, first: Option<NaiveDate>) -> Vec<u8>` — the offsets `1..` whose compared span ends on or after `first` (empty without `first`).
  - `pub fn per_for(unit: Unit, shown: Span) -> DailyPer` — Day → Hour; Year → Month; Custom over 200 days → Month; else Day.
  - `pub fn zoom(unit: Unit, per: DailyPer) -> Option<Unit>` — Year → Month; Week/Month/Custom with `per == Day` → Day; else `None`.
  - Quick picks: `pub fn this_winter(today) -> Span` (1 Nov – 31 Mar of the season containing `today`; from April to October, the coming one), `pub fn last_12_months(today) -> Span` (today − 1 year + 1 day ..= today), `pub fn all_data(first: NaiveDate, today) -> Span`.

- [ ] **Step 1: Write the failing tests** (`today = 2026-11-26`):
  - `parse_state_reads_legacy_ranges`: `range=7d` → `Unit::Week, at: None`; `range=1h` → `Hour1`; `period=month&at=2025-11-12&vs=1` → Month, `at = 2025-11-12`, `vs = Some(1)`; empty → Day current.
  - `state_path_round_trips`: `parse_state` of `state_path` gives the same state for a custom span with `vs` and `show`.
  - `span_of_iso_week`: Week at 2026-11-26 → 2026-11-23..=2026-11-29; at 2026-01-01 → 2025-12-29..=2026-01-04.
  - `step_moves_by_unit_and_custom_length`: Month 2026-11 −1 → Oct 2026 (`at` set); +1 from Oct → current (`at: None`); custom 2025-11-01..=2026-03-31 (151 days) −1 → 2025-06-03..=2025-10-31.
  - `can_step_stops_at_first_date`: Month at 2022-01 with `first = 2022-01-01` → ‹ false; current month → › false.
  - `compare_span_maps_feb_29`: Day 2028-02-29 vs 1 → 2027-02-28; Month Feb 2028 vs 1 → 2027-02-01..=2027-02-28.
  - `compare_span_week_is_iso_week`: Week 2026-11-23..=2026-11-29 vs 1 → 2025-11-24..=2025-11-30.
  - `compare_span_week_53_falls_back`: Week 2026-12-28..=2027-01-03 (2026-W53) vs 1 → 2025-W52 = 2025-12-22..=2025-12-28.
  - `compare_years_from_first`: shown Nov 2026, `first = 2022-01-01` → `[1, 2, 3, 4]`; `first = 2026-01-15` → `[]`.
  - `per_for_and_zoom`: Day → Hour, Week → Day, Year → Month, custom 151 days → Day, custom 400 days → Month; `zoom(Year, Month) == Some(Month)`, `zoom(Month, Day) == Some(Day)`, `zoom(Day, Hour) == None`.
  - `quick_picks`: `this_winter(2026-11-26)` → 2026-11-01..=2027-03-31; `this_winter(2026-06-10)` → 2026-11-01..=2027-03-31; `this_winter(2027-02-02)` → 2026-11-01..=2027-03-31; `last_12_months(2026-11-26)` → 2025-11-27..=2026-11-26.
- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib history_period` — FAIL.
- [ ] **Step 3: Implement** `history_period.rs`.
- [ ] **Step 4: Run** the same — PASS.
- [ ] **Step 5: Commit** `feat(web): calendar periods for history`.

### Task 9: Comparison maths (pure)

**Files:**
- Create: `crates/nidoo-web/src/history_compare.rs`, `crates/nidoo-web/src/history_compare/tests.rs`
- Modify: `crates/nidoo-web/src/lib.rs`

**Interfaces:**
- Consumes: `DailyPer`.
- Produces:
  - `pub fn bucket_index(offset_s: i64, nominal_s: i64) -> i64` — `round(offset / nominal)`; nominal = series `window`, or 3 600 / 86 400 / 2 629 746 for `per` Hour / Day / Month (`pub fn nominal(per: DailyPer) -> i64`).
  - `pub struct CounterSummary { pub shown: f64, pub compared: f64, pub diff: f64, pub pct: Option<f64>, pub left_out: usize }`.
  - `pub fn counter_summary(shown: &[(i64, f64)], shown_start: i64, cmp: &[(i64, f64)], cmp_start: i64, per: DailyPer) -> Option<CounterSummary>` — totals over the indexes present on both sides; `left_out` = buckets present on one side only; `pct = diff / compared * 100` when `compared > 0`; `None` when `cmp` is empty.
  - `pub struct SensorSummary { pub mean: f64, pub min: f64, pub max: f64, pub cmp_mean: f64, pub cmp_min: f64, pub cmp_max: f64, pub diff: f64, pub left_out: usize }`.
  - `pub fn sensor_summary(shown: &[(i64, f64, f64, f64)], shown_start: i64, cmp: &[(i64, f64, f64, f64)], cmp_start: i64, window: i64) -> Option<SensorSummary>` — means of bucket means, min of mins, max of maxes, over matched indexes.
  - Without a comparison: `pub fn counter_total(days: &[(i64, f64)]) -> f64` and `pub fn sensor_stats(points: &[(i64, f64, f64, f64)]) -> Option<(f64, f64, f64)>` (mean, min, max).
  - Test `stats_without_comparison`: `counter_total(&[(0, 1.5), (86_400, 2.5)]) == 4.0`; `sensor_stats(&[])` is `None`.

- [ ] **Step 1: Write the failing tests**:
  - `bucket_index_tolerates_dst_hour`: day buckets at offsets 0, 82 800 (after a 23 h day), 169 200 → indexes 0, 1, 2.
  - `counter_summary_matches_days`: shown days 1–3 = 10, 12, 14 and compared 12, 12, 16 → shown 36, compared 40, diff −4, pct −10.
  - `counter_summary_leaves_out_unmatched_days`: shown has day 29 (index 28) with 9.0 that compared lacks → excluded from both totals, `left_out == 1`.
  - `summary_none_when_compared_empty`: `counter_summary(&[(0, 1.0)], 0, &[], 0, DailyPer::Day) == None`.
  - `sensor_summary_means_matched`: shown means 20, 21 and compared 19, 19 (min/max given) → `diff == 1.5`, `cmp_min`/`cmp_max` from compared buckets only.
- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib history_compare` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(web): compare two history windows`.

### Task 10: Words (i18n)

**Files:**
- Modify: `crates/nidoo-web/src/i18n/history.rs` (and its tests)

**Interfaces:**
- Consumes: Task 8 `Unit`, `Span`; `i18n::format::{month_long, month_short, weekday_short, date_short}`.
- Produces: new `HistoryText` keys with these EN / FR texts —
  `UnitDay` Day / Jour · `UnitWeek` Week / Semaine · `UnitMonth` Month / Mois · `UnitYear` Year / Année · `UnitCustom` Custom… / Personnalisée… · `BackToNow` Back to now / Revenir à maintenant · `CompareWith` Compare with / Comparer avec · `CompareOff` Off / Non · `Today` Today / Aujourd'hui · `ThisWeek` This week / Cette semaine · `ThisMonth` This month / Ce mois-ci · `ThisYear` This year / Cette année · `PickTitle` Show a period / Afficher une période · `PickFrom` From / Du · `PickTo` To / Au · `PickShow` Show / Afficher · `PickCancel` Cancel / Annuler · `ThisWinter` This winter / Cet hiver · `Last12Months` Last 12 months / 12 derniers mois · `AllData` All data / Tout l'historique · `Difference` Difference / Écart · `Mean` mean / moyenne · `Total` total / total · `SoFar` so far / jusqu'ici · `SameDays` same days / mêmes jours · `PreviousPeriod` Previous period / Période précédente · `NextPeriod` Next period / Période suivante.
  Functions: `pub fn unit_chip(lang, u: Unit) -> String` (1h/6h keep today's chip text); `pub fn period_name(lang, u: Unit, s: Span) -> String` — Day "Thu 26 Nov 2026" / "jeu. 26 nov. 2026" (via `date_short` + weekday), Week "23 – 29 Nov 2026" (months shown on both ends when they differ), Month "November 2026" / "novembre 2026", Year "2026", Custom "1 Nov 2025 – 31 Mar 2026"; `pub fn no_data_for(lang, name: &str) -> String` ("No data for {name}" / "Aucune donnée pour {name}"); `pub fn left_out(lang, n: usize, per: DailyPer) -> String` ("1 day left out of the comparison" / "{n} days …", hours, months; FR "1 jour exclu de la comparaison" / "{n} jours exclus …", heures, mois).
- [ ] **Step 1: Write the failing tests**: `period_names_en_fr` (each unit, both languages, the exact strings above), `left_out_plural` (1 and 3, both languages, day and month).
- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib i18n::history` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p nidoo-web` (bare-text test included) — PASS.
- [ ] **Step 5: Commit** `feat(web): words for history periods`.

### Task 11: `HistoryChart` over a window, with a comparison

**Files:**
- Modify: `crates/nidoo-web/src/components/history_chart/view.rs`, `view/tests.rs`, `plot.rs` if the drawing helpers live there; `crates/nidoo-web/src/dashboards/history_widget.rs`, `crates/nidoo-web/src/components/system_chart.rs`, `crates/nidoo-web/src/solar/measured.rs`, `crates/nidoo-web/src/screens/system_settings.rs` (callers pass a preset)
- Create: `crates/nidoo-web/src/components/history_chart/summary.rs` (summary cards)
- Modify: `crates/nidoo-web/styles/tokens.css` (`--compare`: dark `#b3a6ff`, light `#6a5acd`), `crates/nidoo-web/styles/main.css`

**Interfaces:**
- Consumes: Tasks 7–10.
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub enum ChartWindow { Preset(HistoryRange), Dates { shown: WindowReq, unit: Unit, compare: Option<WindowReq> } }`; `HistoryChart`'s `range` prop becomes `window: Signal<ChartWindow>`; new optional prop `on_zoom: Option<Callback<NaiveDate>>` (the clicked bucket's local date).
  - `FetchPlan` gains `SeriesWindow(String, WindowReq)` and `DailyWindow(String, WindowReq, DailyPer)`; `plan_fetch(entity, window: &ChartWindow, flavor, power_pair) -> FetchPlan` — Dates + Series flavor → `SeriesWindow`; Dates + Counter → `DailyWindow` with `per_for`; presets unchanged (counter Hour1/Hour6 keep the power pair; Day is now `Dates`).
  - Fetch order: when the shown span contains today, fetch it first and request the compared window with `upto = span.1 − span.0` of the shown reply; otherwise fetch both at once. A compared reply that fails or is empty shows the chart without it, and the summary's compared card says `no_data_for`.
  - Fields (`show` chips) for `Dates` only when `span.days() <= 400`.
  - Refresh: a `Dates` window refetches with `refresh_period_s` only while its span contains today.
  - Drawing: compared series as `history-chart__line--compare` (dashed, `--compare`) behind the shown line; compared bars as `history-chart__bar--compare` (`--compare` at 28 % opacity, wider, behind); points placed by `bucket_index` from each reply's `span.0`, the compared series cut at the shown span's length; a dotted `history-chart__now` line at the shown reply's `span.1` when running; the tooltip lists the shown and the compared value, each with its own date; clicking a bucket calls `on_zoom` when `zoom(unit, per)` is `Some`.
  - `summary.rs`: `#[component] pub fn PeriodSummary(...)` rendering the cards (shown, compared, difference) from `counter_summary` / `sensor_summary` / `counter_total` / `sensor_stats`, with `SoFar` / `SameDays` / `left_out` texts; increases in `--accent-rose`, decreases in `--accent-mint` for counters.
- [ ] **Step 1: Write the failing tests** in `view/tests.rs`: `plan_fetch_dates_series_uses_window`, `plan_fetch_dates_counter_month_per_day`, `plan_fetch_dates_counter_year_per_month`, `plan_fetch_dates_counter_day_per_hour`, `plan_fetch_preset_counter_6h_keeps_power_pair` (exact `FetchPlan` values), and `compare_upto_from_shown_span`: a pure `fn compare_request(shown_span: (i64, i64), running: bool, cmp: WindowReq) -> WindowReq` sets `upto = Some(span.1 − span.0)` only when running.
- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib history_chart` — FAIL.
- [ ] **Step 3: Implement** the enum, the fetch changes, drawing, summary, CSS; update every `HistoryChart` caller to `ChartWindow::Preset(..)`.
- [ ] **Step 4: Run** `cargo test -p nidoo-web` and `cargo clippy -p nidoo-web --all-targets -- -D warnings` — PASS; compile-check `--target wasm32-unknown-unknown --no-run --test history_chips --test dashboard_screen`.
- [ ] **Step 5: Commit** `feat(web): history chart over a date window with a comparison`.

### Task 12: The history page

**Files:**
- Create: `crates/nidoo-web/src/components/period_bar.rs` (chips, ‹ name ›, "Back to now", picker popover, compare toggle)
- Modify: `crates/nidoo-web/src/screens/history_detail.rs`, `crates/nidoo-web/src/components/mod.rs`, `crates/nidoo-web/styles/main.css`
- Create: `crates/nidoo-web/tests/history_period.rs` (browser test)

**Interfaces:**
- Consumes: Tasks 7–11.
- Produces:
  - `#[component] pub fn PeriodBar(state: Signal<PeriodState>, today: Signal<NaiveDate>, first: Signal<Option<NaiveDate>>, on_change: Callback<PeriodState>, compare_allowed: Signal<bool>) -> impl IntoView`.
  - `HistoryDetailScreen` reads `PeriodState` with `parse_state`, fetches `extent` once per entity (until it arrives: `today` = the browser's local date, stepping disabled), builds the `ChartWindow` (`compare = vs.map(compare_span)`), writes changes with `state_path` (replace-navigation), passes `on_zoom` → `switch_unit(zoom unit)` at the clicked date.
  - Compare toggle: `CompareOff` or `CompareWith` + a `<select>` of `compare_years` shown as years (`shown.from.year() − vs`); hidden for Hour1/Hour6, for word-only charts (no number chip), and when `compare_years` is empty. Picker: two `<input type="date">` bounded by `first` and `today`, quick picks `this_winter`, `last_12_months`, `all_data`; Show sets `Unit::Custom`.
  - Layout as in the mockup (`.superpowers/brainstorm/73428-1795724590/content/period-compare.html`, with the compare control replaced by the year toggle): period row under the chips, compare on the right, stacked under it below 600 px.
- [ ] **Step 1: Write the failing browser test** `tests/history_period.rs`, with `FetchStub` (as `history_chips.rs` does) answering `/history/extent` with `{"first_date":"2022-01-01","today":"2026-11-26"}` and window series with two points: open `/history/sensor.t?period=month` in EN → the period name reads "November 2026" and "This month"; click ‹ → the URL has `at=2026-10-01` and the name reads "October 2026"; choose compare 2025 → a request to `/api/v1/history/series?entity=sensor.t&from=2025-10-01&to=2025-10-31` was made; open `?range=7d` → the Week chip is selected.
- [ ] **Step 2: Run** the single file (find the driver whose `--version` major matches Chrome's):
  ```
  C=~/Library/Caches/.wasm-pack
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=$C/wasm-bindgen-cargo-install-0.2.129/wasm-bindgen-test-runner \
  CHROMEDRIVER=$C/chromedriver-<matching>/chromedriver \
  cargo test -p nidoo-web --target wasm32-unknown-unknown --test history_period
  ```
  Expected: FAIL.
- [ ] **Step 3: Implement** `PeriodBar` and the screen wiring; remove the old `RANGES` chips and `range_slug` / `history_path` once `state_path` replaces them.
- [ ] **Step 4: Run** the browser test (PASS), then `cargo test -p nidoo-web`, then `rm ~/dev-scratch/cargo-target/wasm32-unknown-unknown/debug/deps/*.wasm` to free the test binaries.
- [ ] **Step 5: Commit** `feat(web): pick a period and compare it on the history page`.

### Task 13: Whole-branch verification

- [ ] **Step 1:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (with an empty `crates/nidoo-web/dist` present if the static-UI test needs it) — all green.
- [ ] **Step 2:** `cargo test -p nidoo-web --target wasm32-unknown-unknown --no-run --test history_chips --test dashboard_screen --test history_period` — compiles.
- [ ] **Step 3:** Run the repo's brand check script if present (`scripts/brand-check.sh`) — passes.
- [ ] **Step 4:** Local browser pass (throwaway mosquitto + hub + `trunk serve`, as in earlier sessions): open a sensor's history page, step months, toggle the comparison, open the picker, check the phone width. Screenshot for the PR.
- [ ] **Step 5:** Commit any fix-ups; open the PR (Gekkotron, `GH_TOKEN=$(gh auth token --user Gekkotron)`), then watch CI.
