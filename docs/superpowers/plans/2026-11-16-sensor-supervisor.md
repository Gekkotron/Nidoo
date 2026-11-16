# Sensor Supervisor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An hourly hub task that flags numeric sensors that look stuck, silent or physically impossible, learns each sensor's normal from 14 days of history, and sends one push per problem (plus one "back to normal").

**Architecture:** Pure logic (baseline, checks, problem lifecycle, alert JSON) in a new `supervisor` module of `nidoo-watchdog`, fed plain structs. The hub (`crates/nidoo/src/supervisor/`) gathers readings (registry, live report clocks, `entity_latest`, a daily gap query), persists problems in a new table, sends alerts through `Hub::notify("system", …)`, and handles the Mute reply. Settings live inside the existing live `system` section (no restart). Web card on Settings → System; `GET/PUT /api/v1/supervisor`; MCP `supervisor_status`.

**Tech Stack:** Rust workspace, tokio, sqlx + TimescaleDB, axum, rmcp, Leptos (wasm).

**Spec:** `docs/superpowers/specs/2026-11-16-sensor-supervisor-design.md` — read it first; it is the source of every rule below.

## Global Constraints

- Supervised entities: `kind == Kind::Sensor` whose current value is `StateValue::Number`, not in `ignored`. Secret entities never (they are not in history).
- Baseline window 14 days; baseline needs history ≥ 7 days and ≥ 20 changes in the window; recomputed once a day (and on the first run); gaps overlapping a recorded problem of that sensor (last 14 days) are excluded.
- Stuck: `now − last_change > max(max_gap × factor, 6 h)` and a read report within `max(max_gap, 1 h)`.
- Silent: no read report for `> max(max_gap × factor, 6 h)` (clock never older than the arm time), or `unavailable` (registry `Availability::Offline`) for `> 1 h`. Paused (neither opens nor closes) while the broker link is not Online or the Zigbee2MQTT bridge is not online.
- A report = `StateTouched { read: true }`. `read: false` does not count.
- Impossible ranges: temperature (°C) −50…150; humidity, battery, moisture (%) 0…100; illuminance ≥ 0; pressure (hPa) 800…1200; voltage (V) 0…300; co2 (ppm) 0…10 000. `power` and anything else: not range-checked.
- Sensitivity → factor: `low` 3.0, `normal` 2.0, `high` 1.5.
- Settings defaults: `enabled: true`, `notify: true`, `notify_recovered: true`, `sensitivity: normal`, `ignored: []`. Saved live (no restart).
- Close only on positive evidence (value changed after `opened_at`; read report or back online after `opened_at`; value in range). A check that cannot run keeps the problem open. An entity gone from the registry closes silently.
- One push per opened problem; > 3 opened in one run → one grouped push (no Mute button); recovered pushes of one run grouped the same way.
- `notify` off: problems still tracked, nothing pushed; turning it on does not push already-open problems. `enabled` off: open problems cleared without pushes.
- Alerts: source `system`, channel `system`, `ttl` 86400, opened `warning`, recovered `info`, loc keys `supervisor.stuck` / `supervisor.silent` / `supervisor.impossible` / `supervisor.recovered` / `supervisor.group`, English `title`/`body`, `entity_id` set on single alerts, action `{"id":"mute","label":"Mute this sensor"}` on single opened alerts. Alert ids start with `supervisor-`.
- First run 1 h after start, then every hour. No database → the task does nothing and the view says `available: false`.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Hub restart with open problems** — no push re-sent, a problem that recovered while the hub was down gets its one recovered push. Test in Task 4.
2. **Flat-by-nature sensor** (battery at 100 % for 14 days, < 20 changes) — never alerts. Test in Task 1.
3. **Coordinator / bridge drop** — 30 sensors silent at once → paused while the bridge is offline; if the bridge stays online, one grouped push. Tests in Tasks 2 and 4.
4. **Settings toggled with problems open** — `enabled` off then on, `notify` off then on: no stale pushes. Test in Task 2.
5. **Database down at tick time** — the run is skipped, the in-memory problems are kept, nothing closes. Test in Task 4.

---

### Task 1: Pure checks and baseline (`nidoo-watchdog`)

**Files:**
- Create: `crates/nidoo-watchdog/src/supervisor/mod.rs`, `crates/nidoo-watchdog/src/supervisor/checks.rs`
- Modify: `crates/nidoo-watchdog/src/lib.rs` (add `pub mod supervisor;`)
- Test: `crates/nidoo-watchdog/tests/supervisor_checks.rs`

**Interfaces:**
- Produces (all times epoch seconds):
  - `pub enum Check { Stuck, Silent, Impossible }` (`Copy, Ord, Serialize, Deserialize`, serde `snake_case`) with `fn as_str(self) -> &'static str`.
  - `pub enum Sensitivity { Low, Normal, High }` with `fn factor(self) -> f64`.
  - `pub struct Baseline { pub max_gap: u64, pub changes: u32 }`
  - `pub struct GapStats { pub entity_id: String, pub changes: u32, pub first: u64, pub top_gaps: Vec<(u64, u64)> }` — the 5 largest gaps as (start, end).
  - `pub fn baseline(stats: &GapStats, now: u64, excluded: &[(u64, u64)]) -> Option<Baseline>` — `None` when `now − first < 7 d` or `changes < 20`; `max_gap` = largest gap in `top_gaps` not overlapping any `excluded` interval (fallback: the smallest of the 5 if all overlap).
  - `pub fn physical_range(device_class: Option<&str>, unit: Option<&str>) -> Option<(f64, f64)>` — table from Global Constraints; unbounded side as `f64::INFINITY`/`NEG_INFINITY`.
  - `pub struct Reading { pub entity_id: String, pub name: String, pub device_class: Option<String>, pub unit: Option<String>, pub value: Option<f64>, pub last_change: Option<u64>, pub last_report: Option<u64>, pub unavailable_since: Option<u64>, pub baseline: Option<Baseline> }`
  - `pub struct Ctx { pub now: u64, pub armed_at: u64, pub factor: f64, pub silent_paused: bool }`
  - `pub struct Finding { pub check: Check, pub evidence: Evidence }` and `pub struct Evidence { pub value: Option<f64>, pub unit: Option<String>, pub for_secs: u64, pub usual_secs: Option<u64> }` (`Serialize, Deserialize, Clone, PartialEq`).
  - `pub fn evaluate(r: &Reading, ctx: &Ctx) -> Vec<Finding>`

- [ ] **Step 1: Write failing tests** in `tests/supervisor_checks.rs`:
  - `tank_probe_flat_while_reporting_is_stuck`: baseline `max_gap` 3 h, factor 2, `last_change` = now − 7 h, `last_report` = now − 60 s → one `Stuck`, `evidence.for_secs == 7*3600`, `usual_secs == Some(3*3600)`. Same with `last_change` = now − 5 h → no finding (6 h floor).
  - `stuck_needs_a_recent_report`: same but `last_report` = now − 4 h → no `Stuck` (and `Silent` only past its own threshold).
  - `silent_clock_starts_at_arm_time`: `last_report: None`, `armed_at` = now − 2 h, `max_gap` 3 h → nothing; `armed_at` = now − 7 h → `Silent`.
  - `silent_paused_finds_nothing`: `silent_paused: true` → no `Silent` even when overdue.
  - `offline_over_an_hour_is_silent`: `unavailable_since` = now − 61 min → `Silent`; 59 min → nothing.
  - `no_baseline_means_only_the_range_check`: `baseline: None`, flat for 10 days → nothing; `device_class: "humidity"`, `value: 140.0` → one `Impossible`.
  - `battery_flat_for_14_days_has_no_baseline`: `GapStats { changes: 3, first: now − 14 d, .. }` → `baseline(..) == None`.
  - `excluded_outage_does_not_inflate_the_baseline`: `top_gaps` [(t, t+5 d), (u, u+4 h), …], excluded covers the 5 d gap → `max_gap == 4 h`.
  - `ranges`: `physical_range(Some("temperature"), Some("°C")) == Some((-50.0, 150.0))`, `Some("power")` → `None`, `None` → `None`.

- [ ] **Step 2: Run** `cargo test -p nidoo-watchdog --test supervisor_checks` — Expected: FAIL (module missing).

- [ ] **Step 3: Implement** the types and functions above in `checks.rs`, re-exported from `supervisor/mod.rs`. Add `serde` (derive) to the crate's dependencies from the workspace. Keep the crate free of workspace dependencies.

- [ ] **Step 4: Run** `cargo test -p nidoo-watchdog` — Expected: PASS.

- [ ] **Step 5: Commit** `feat(watchdog): sensor supervisor checks and baseline`.

### Task 2: Pure problem lifecycle and alerts (`nidoo-watchdog`)

**Files:**
- Create: `crates/nidoo-watchdog/src/supervisor/problems.rs`, `crates/nidoo-watchdog/src/supervisor/alerts.rs`
- Test: `crates/nidoo-watchdog/tests/supervisor_problems.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `pub struct Problem { pub entity_id: String, pub name: String, pub check: Check, pub opened_at: u64, pub evidence: Evidence, pub alert_id: Option<String> }` (`Serialize, Deserialize, Clone`).
  - `pub struct Supervisor` with `pub fn new(open: Vec<Problem>) -> Self`, `pub fn open(&self) -> impl Iterator<Item = &Problem>`, `pub fn step(&mut self, readings: &[Reading], ctx: &Ctx) -> Step`, `pub fn mute(&mut self, entity_id: &str) -> Vec<Problem>` (removes and returns), `pub fn clear(&mut self) -> Vec<Problem>`, `pub fn entity_for_alert(&self, alert_id: &str) -> Option<&str>`.
  - `pub struct Step { pub opened: Vec<Problem>, pub closed: Vec<Closed> }`, `pub struct Closed { pub problem: Problem, pub at: u64, pub silent: bool }` (`silent: true` when the entity vanished — no push).
  - `pub fn alerts(step: &mut Step, notify: bool, notify_recovered: bool, now: u64) -> Vec<serde_json::Value>` — sets `alert_id` on each opened problem it alerted; empty when `notify` is false.

- [ ] **Step 1: Write failing tests**:
  - `stuck_probe_lifecycle`: run 1 (stuck) → 1 opened, alert has `"loc.key" == "supervisor.stuck"`, `entity_id`, one action `mute`, id starting `supervisor-`; run 2 same reading → nothing; run 3 with `last_change` > `opened_at` → 1 closed, alert `supervisor.recovered`, severity `info`.
  - `no_baseline_keeps_the_problem_open`: problem open, next reading has `baseline: None` and unchanged value → stays open, nothing closed.
  - `silent_paused_neither_opens_nor_closes`: open `Silent` problem, ctx `silent_paused: true`, reading with a fresh report → stays open.
  - `vanished_entity_closes_silently`: open problem whose entity is absent from `readings` → `Closed { silent: true }`, `alerts` emits nothing for it.
  - `four_opened_in_one_run_group`: 4 sensors silent → `alerts` returns exactly 1 value, key `supervisor.group`, no `actions`, no `entity_id`, body names the first 3. 3 sensors → 3 alerts.
  - `notify_off_tracks_but_sends_nothing`: `alerts(.., notify=false, ..)` → empty, problems still open; a later run with `notify=true` and the same readings → empty (already open).
  - `recovered_suppressed`: `notify_recovered=false` → closing emits nothing.
  - `mute_removes_without_push`: `mute("e")` returns its open problems and `entity_for_alert(<its alert id>)` is `None` afterwards. (Filtering `ignored` sensors out of later readings is the hub's job, tested in Task 4.)
  - `clear_returns_everything`.

- [ ] **Step 2: Run** `cargo test -p nidoo-watchdog --test supervisor_problems` — Expected: FAIL.

- [ ] **Step 3: Implement.** Problem key is `(entity_id, check)` in a `BTreeMap`. Closing evidence per check: Stuck — `last_change > opened_at`; Silent — `last_report > opened_at` or `unavailable_since.is_none()` after an offline-opened problem (store which trigger in evidence via `value: None`/`for_secs`, builder's choice) and not `silent_paused`; Impossible — value present and in range. Alert id `supervisor-{check}-{entity_id}-{opened_at}`, grouped `supervisor-group-{check|recovered}-{now}`. English copy:
  - stuck: title `"{name} looks stuck"`, body `"Same value ({value} {unit}) for {hours} h, usually changes within {usual} h. Check the sensor."`
  - silent: `"{name} stopped reporting"`, `"No report for {hours} h. Check the sensor or its battery."` (offline: `"Offline for {hours} h."`)
  - impossible: `"{name} reads an impossible value"`, `"{value} {unit} is outside what this sensor can measure. Check the sensor."`
  - recovered: `"{name} is back to normal"`, body empty-safe one-liner.
  - group: `"{n} sensors look wrong"` / `"{n} sensors are back to normal"`, body = first 3 names + `"and {n−3} more"`.
  - loc args: `name`, `value`, `unit`, `hours`, `usual` as strings (the push validator expects string args, as the outage alerts do).

- [ ] **Step 4: Run** `cargo test -p nidoo-watchdog` — Expected: PASS.

- [ ] **Step 5: Commit** `feat(watchdog): supervisor problem lifecycle and alerts`.

### Task 3: Settings, wire types, storage

**Files:**
- Create: `crates/nidoo-api/src/supervisor.rs`, `crates/nidoo/migrations/0008_supervisor.sql`, `crates/nidoo/src/supervisor/store.rs`
- Modify: `crates/nidoo-api/src/lib.rs`, `crates/nidoo/src/setup/sections.rs` (`SystemSection`), `crates/nidoo/src/history/db.rs` (or new queries in `store.rs` using `Db::pool()`), `crates/nidoo/src/lib.rs` (`pub mod supervisor;`)
- Test: in-file `#[cfg(test)]` in `sections.rs`; DB tests in `store.rs` follow the existing skip-without-Postgres pattern.

**Interfaces:**
- Produces:
  - `nidoo_api::SupervisorSettings { enabled: bool, notify: bool, notify_recovered: bool, sensitivity: Sensitivity /* "low"|"normal"|"high" */, ignored: Vec<String> }` with a manual `Default` (the Global Constraints defaults) and `#[serde(default)]`. `nidoo-api` gets its own `Sensitivity` enum (it cannot depend on `nidoo-watchdog`); the hub maps it.
  - `nidoo_api::ProblemView { entity_id, name, check: String, opened_at: u64, evidence: serde_json::Value }`, `nidoo_api::SupervisorView { available: bool, settings: SupervisorSettings, problems: Vec<ProblemView>, learning: u32 }`.
  - `SystemSection { notify: bool, supervisor: SupervisorSettings }`.
  - Migration: `CREATE TABLE supervisor_problem (entity_id text NOT NULL, check_kind text NOT NULL, opened_at timestamptz NOT NULL, closed_at timestamptz, name text NOT NULL, alert_id text, evidence jsonb NOT NULL, PRIMARY KEY (entity_id, check_kind, opened_at));`
  - In `supervisor/store.rs`: `pub async fn load_open(db: &Db) -> Result<Vec<Problem>, sqlx::Error>`, `pub async fn insert(db: &Db, p: &Problem)`, `pub async fn close(db: &Db, p: &Problem, at: u64)`, `pub async fn delete(db: &Db, entity_id: &str)` (mute), `pub async fn delete_open(db: &Db)` (enabled off), `pub async fn intervals(db: &Db, since: u64) -> Result<BTreeMap<String, Vec<(u64, u64)>>, _>` (closed → `(opened, closed)`, open → `(opened, now)`), `pub async fn purge(db: &Db, before: u64)` (rows closed > 30 d ago), `pub async fn gap_stats(db: &Db, ids: &[String], since: u64) -> Result<Vec<GapStats>, _>` (one grouped query: `lag(time)` over `entity_state` per entity where `time >= since`, returning count, first time, and the 5 largest gaps), `pub async fn last_changes(db: &Db, ids: &[String]) -> Result<HashMap<String, u64>, _>` (from `entity_latest`).

- [ ] **Step 1: Write failing tests**: `supervisor_settings_default_on_and_round_trip` in `sections.rs` — an old row `{"notify": true}` parses with `supervisor.enabled == true`, `sensitivity == Normal`, `ignored` empty; a changed `SupervisorSettings` survives `to_rows`/`from_rows`. DB tests (skipped without Postgres): insert → `load_open` returns it; `close` → `load_open` empty and `intervals` has it; `gap_stats` on 30 seeded rows returns `changes == 30` and the largest gap first.
- [ ] **Step 2: Run** `cargo test -p nidoo sections` — Expected: FAIL.
- [ ] **Step 3: Implement** the types, migration and queries.
- [ ] **Step 4: Run** `cargo test -p nidoo sections` and `cargo test -p nidoo supervisor::store` — Expected: PASS (DB tests report skipped locally; see memory "Local test gaps").
- [ ] **Step 5: Commit** `feat(supervisor): settings, wire types and problem storage`.

### Task 4: Hub task, report clocks, Mute reply

**Files:**
- Create: `crates/nidoo/src/supervisor/mod.rs`, `crates/nidoo/src/supervisor/task.rs`
- Modify: `crates/nidoo/src/hub.rs` (field `supervisor: Mutex<SupervisorRuntime>`, `push.responses_topic()` routing branch), `crates/nidoo/src/main.rs` (`let supervisor = hub.start_supervisor();` next to `start_system`)
- Test: `crates/nidoo/src/supervisor/task.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: Tasks 1–3; `Hub::subscribe_state_touched()`, `Hub::link_epoch()`, `Hub::connection()`, `self.read().bridge().state`, `self.read().entities()` / `.state(id)`, `nidoo_core::history::num_of`, `Hub::history()?.db()`, `Hub::notify("system", &v)`, `store::save_live(ctx, "system", |s| …)`.
- Produces:
  - `SupervisorRuntime { engine: nidoo_watchdog::supervisor::Supervisor, last_report: HashMap<String, u64>, unavailable_since: HashMap<String, u64>, armed_at: u64, epoch: u64, baselines: HashMap<String, Baseline>, baselines_at: Option<u64>, loaded: bool }`.
  - `impl Hub { pub fn start_supervisor(self: &Arc<Self>) -> Option<JoinHandle<()>>; pub async fn supervisor_tick(&self, now: u64); pub fn supervisor_view(&self) -> SupervisorView; pub async fn set_supervisor(&self, s: SupervisorSettings) -> Result<SupervisorView, HubError>; pub(crate) fn on_supervisor_reply(self: &Arc<Self>, payload: &[u8]) }`.

- [ ] **Step 1: Write failing tests** (a `Hub` built like `system/mod.rs`'s `hub_with`, readings injected through a `supervisor_tick_with(readings, ctx)` seam that skips the DB):
  - `restart_does_not_resend_open_problems`: runtime built with one loaded open problem, same bad reading → no alert submitted.
  - `recovered_while_down_pushes_once`: loaded open Stuck problem, reading with `last_change > opened_at` → exactly one recovered alert.
  - `bridge_offline_pauses_silent`: 30 silent readings with bridge offline → no alert; bridge online → one grouped alert.
  - `db_down_skips_the_run`: no DB → `supervisor_tick` changes nothing and the view says `available: false`.
  - `mute_reply_ignores_the_sensor`: reply `{"id": "<alert id>", "action": "mute"}` → `ignored` contains the entity, its problem is gone, no recovered push.
  - `disable_clears_problems`: `set_supervisor(enabled: false)` → `supervisor_view().problems` empty, no alert.
- [ ] **Step 2: Run** `cargo test -p nidoo supervisor` — Expected: FAIL.
- [ ] **Step 3: Implement.**
  - The task: subscribe to `state_touched`; on `read: true` set `last_report[id] = now` (only when `epoch == link_epoch()`). On link epoch change set `armed_at = now`. Sleep until `start + 3600`, then tick every 3600 s.
  - `supervisor_tick`: return early when `!enabled` or no DB. On first tick load open problems from the store into the engine (`loaded`). Refresh baselines when `baselines_at` is `None` or older than 24 h (`gap_stats` + `intervals` + `baseline`). Build `Reading`s for supervised entities (Global Constraints), excluding `ignored`; `unavailable_since` tracked in the runtime (first tick seen `Offline`, removed when not). `Ctx.silent_paused` = link not `Online` or bridge not online. `step`, then `alerts(...)`, submit each via `notify("system", ..)`, then persist (insert opened with `alert_id`, close closed). Purge once a day. Do DB work without holding the runtime mutex across `.await`.
  - Reply routing in `hub.rs`: next to the alarm branch, an id starting `supervisor-` and `action == "mute"` goes to `on_supervisor_reply`, which resolves the entity via `entity_for_alert`, appends to `ignored` through `save_live`, calls `engine.mute`, and `store::delete`.
  - `supervisor_view().learning` = supervised sensors (not ignored) with no baseline in the last refresh.
  - `set_supervisor`: `save_live(ctx, "system", |s| s.system.supervisor = new)`; when `enabled` flips off, `engine.clear()` + `store::delete_open`.
- [ ] **Step 4: Run** `cargo test -p nidoo supervisor` and `cargo clippy -p nidoo -- -D warnings` — Expected: PASS, no warnings.
- [ ] **Step 5: Commit** `feat(supervisor): hourly hub task, report clocks and mute reply`.

### Task 5: HTTP route and MCP tool

**Files:**
- Modify: `crates/nidoo/src/http/api.rs` (routes next to `/system/notify`), `crates/nidoo/src/mcp/tools.rs` (next to `system_status`)
- Test: existing HTTP/MCP test files' patterns (`crates/nidoo/tests/` or in-file).

**Interfaces:**
- Consumes: `Hub::supervisor_view`, `Hub::set_supervisor`.
- Produces: `GET /api/v1/supervisor` → `200 SupervisorView`; `PUT /api/v1/supervisor` body `SupervisorSettings` → `200 SupervisorView`, `503` when settings cannot be stored (same mapping as `put_system_notify`); MCP tool `supervisor_status` (read-only) → `SupervisorView`.

- [ ] **Step 1: Write failing tests**: `get_supervisor_returns_defaults` (no DB → `available: false`, `settings.enabled == true`); `put_supervisor_without_database_is_503`; MCP `supervisor_status` listed with `read_only_hint`.
- [ ] **Step 2: Run** the new tests — Expected: FAIL.
- [ ] **Step 3: Implement** the handlers and the tool.
- [ ] **Step 4: Run** `cargo test -p nidoo` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(supervisor): HTTP route and MCP tool`.

### Task 6: Web card on Settings → System

**Files:**
- Modify: `crates/nidoo-web/src/screens/system_settings.rs` (new `SupervisorCard` under the Notify me block), `crates/nidoo-web/src/api/core.rs` (`get_supervisor()`, `set_supervisor(s)` next to `set_system_notify`), `crates/nidoo-web/src/i18n/` (English + French strings in the `SystemText` family), the system screen stylesheet.

**Interfaces:**
- Consumes: Task 3 wire types, Task 5 routes, `EntityPicker` (`crates/nidoo-web/src/entity_picker/view.rs:592`).

- [ ] **Step 1: Write failing tests** for the pure helpers: `problem_line(lang, &ProblemView, now)` → `"Tank temperature — stuck for 14 h"` / French `"Température ballon — bloquée depuis 14 h"`; `learning_line(lang, 3)` → `"Learning: 3 sensors have less than 7 days of history"`.
- [ ] **Step 2: Run** `cargo test -p nidoo-web system_settings` — Expected: FAIL.
- [ ] **Step 3: Implement** the card: switches Supervise / Notify me / Tell me when it is back to normal; a low/normal/high segmented control; the ignored list (chips with ✕, an "Add" entity picker filtered to sensors); open problems list with a Mute button each (calls `set_supervisor` with the id appended); the learning line; "Needs the database" when `available` is false. Optimistic update with rollback on error, like the existing notify switch.
- [ ] **Step 4: Run** `cargo test -p nidoo-web` and `cargo check -p nidoo-web --target wasm32-unknown-unknown` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): sensor supervisor card on Settings → System`.

### Task 7: Docs

**Files:**
- Modify: `docs/users/system-health.md` (new "Sensor supervisor" section: what the three checks are, learning period, one push per problem, grouping, Mute, settings), `docs/reference/configuration.md` (`system.supervisor` fields and defaults), `docs/reference/http-api.md` (`/api/v1/supervisor`), `docs/reference/mcp-tools.md` (`supervisor_status`), `docs/users/push-notifications.md` (the `supervisor.*` alert keys and args), `docs/contributors/architecture.md` (`nidoo-watchdog` row mentions the supervisor).

- [ ] **Step 1: Write the sections** in the existing docs' style (short sentences, tables for fields).
- [ ] **Step 2: Run** `rg -n "supervisor" docs/` — Expected: every file above mentions it; links resolve to existing anchors.
- [ ] **Step 3: Commit** `docs(supervisor): user guide and reference`.
