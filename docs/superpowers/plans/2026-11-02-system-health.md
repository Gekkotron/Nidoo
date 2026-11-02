# System Health Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record the hub's uptime and restarts, the server's CPU / RAM / disk (with the hub's share), the database size, retained topics, devices and commands, publish each as a sensor, show them on a Settings → System tile page, and push on an unexpected restart or a broker / Z2M / database outage.

**Architecture:** A new hub module `crates/nidoo/src/system/` owns a 60 s sampler task that reads the machine with `sysinfo`, keeps the latest view in memory, publishes hub-owned `system_*` sensors through retained discovery (like `network/`), writes a heartbeat file for crash detection and feeds three outage trackers (generalised from `outage.rs`). The restart log lives in a new table; HTTP `GET /api/v1/system` and an MCP tool expose the view; the web gets a `System` settings sub-page with tiles, a two-line sparkline and a two-line chart.

**Tech Stack:** Rust (tokio, axum, sqlx/TimescaleDB, rmcp), `sysinfo` (new dependency), Leptos (wasm) web UI, wasm-bindgen-test.

**Spec:** `docs/superpowers/specs/2026-11-02-system-health-design.md`

## Global Constraints

- `#![forbid(unsafe_code)]` stays in `crates/nidoo`: no libc/statvfs; machine figures come from `sysinfo` (latest release on crates.io at build time, default features off where possible), which also builds on macOS.
- Sampling interval **60 s**; outage threshold for broker / Z2M / database **5 minutes** (Weather / Solcast keep 30); history retention of system sensors and restart log **90 days**; page charts 24 h / 7 d / 30 d; uptime % over **30 days**.
- "Notify me" switch **off by default**; a **clean restart never pushes**.
- Not measurable ⇒ "unknown" or hidden, **never** "down" or 0.
- Sampling never holds the registry lock while reading the machine or the database; it never blocks the ingest path.
- Every user-facing web string goes through an i18n table with **English and French** (`crates/nidoo-web/tests/bare_text.rs` enforces it).
- Tracked files stay generic: no real host names, IPs, sizes or room names (CI `tools/ci/brand-check.sh`).
- Commits as `Gekkotron`: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Long cargo runs with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; check `df -h ~` (< 5 GB free ⇒ per-crate tests only).
- Gate per task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the task's tests. Web tasks also: `cargo check -p nidoo-web --target wasm32-unknown-unknown` and `cd crates/nidoo-web && wasm-pack test --headless --chrome --test <file>`.

## Review Focus

1. **Docker `/data` not found among `sysinfo` disks** (bind mount filtered, odd fs type) → Disk tile reads "unknown", the `system_disk_used` sensor is not published; never 0 %. Test in Task 2.
2. **Heartbeat file unreadable or half-written after a power cut** → the restart counts as unexpected with an unknown downtime ("down ? min"), never as a clean stop and never a panic. Test in Task 3.
3. **Clock went backwards across the restart** (heartbeat later than boot) → downtime saturates at 0, uptime % stays within 0–100. Tests in Tasks 3 and 4.
4. **Broker drops and comes back** → the sampler keeps running while publishes are refused, and republishes every discovery config, state and availability after the reconnect (new `link_epoch`). Test in Task 9.
5. **"Notify me" toggled while the database is down** → `503`, nothing changes (flag, cache and DB row stay as they were); the page shows the error. Test in Task 6.

---

### Task 1: Shared API types and entity ids

**Files:**
- Create: `crates/nidoo-api/src/system.rs`
- Modify: `crates/nidoo-api/src/lib.rs` (add `pub mod system;` and a `pub use system::{…}` line next to the `network` one, line ~47)

**Interfaces:**
- Produces (all `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]`, `#[serde(rename_all = "camelCase")]`, times in Unix seconds like `NetworkWatchView`):
  - `SystemView { started_at: u64, uptime_pct: f64, restarts: Vec<RestartView>, cpu: Option<CpuView>, ram: Option<MemView>, disk: Option<DiskView>, temperature_c: Option<f64>, devices: u32, entities: u32, retained: Option<u64>, commands_today: u64, services: Vec<ServiceView>, notify: bool }`
  - `RestartView { started_at: u64, down_secs: Option<u64>, stop: StopKind }`
  - `enum StopKind { First, Clean, Unexpected }` (snake_case)
  - `CpuView { server_pct: f64, hub_pct: f64 }`
  - `MemView { used_bytes: u64, total_bytes: u64, hub_bytes: u64 }`
  - `DiskView { used_bytes: u64, total_bytes: u64, db_bytes: Option<u64> }`
  - `ServiceView { service: SystemService, state: ServiceState, since: Option<u64> }`
  - `enum SystemService { Broker, Zigbee2mqtt, Database }`, `enum ServiceState { Up, Down, Unknown }` (snake_case)
  - `NotifyInput { notify: bool }` with `#[serde(deny_unknown_fields)]`
  - `pub mod ids` with `pub const` entity ids: `CPU = "system_cpu"`, `CPU_HUB = "system_cpu_hub"`, `RAM_USED = "system_ram_used"`, `RAM_HUB = "system_ram_hub"`, `DISK_USED = "system_disk_used"`, `DB_SIZE = "system_db_size"`, `TEMPERATURE = "system_temperature"`, `DEVICES = "system_devices"`, `ENTITIES = "system_entities"`, `RETAINED = "system_retained"`, `COMMANDS = "system_commands"`, `STARTED = "system_started"`, `BROKER = "system_broker"`, `ZIGBEE2MQTT = "system_zigbee2mqtt"`, `DATABASE = "system_database"`, and `pub const ALL: [&str; 15]`.

- [ ] **Step 1: Write the failing test** `system_view_wire_shape` in `system.rs` `#[cfg(test)]`: serialize a `SystemView` with one `RestartView { started_at: 10, down_secs: None, stop: StopKind::Unexpected }` and one `ServiceView { service: Zigbee2mqtt, state: Down, since: Some(5) }`; assert the JSON has keys `startedAt`, `uptimePct`, `commandsToday`, `restarts[0].downSecs == null`, `restarts[0].stop == "unexpected"`, `services[0].service == "zigbee2mqtt"`, `services[0].state == "down"`; round-trips equal. Second test `ids_all_are_distinct_and_prefixed`: every `ids::ALL` entry starts with `system_` and they are distinct; `NotifyInput` rejects `{"notify":true,"x":1}`.
- [ ] **Step 2:** `cargo test -p nidoo-api system` → FAIL (module missing).
- [ ] **Step 3:** Implement the types above.
- [ ] **Step 4:** `cargo test -p nidoo-api system` → PASS; `cargo check -p nidoo-api --target wasm32-unknown-unknown` → OK.
- [ ] **Step 5:** Commit `feat(api): system health view types`.

---

### Task 2: Machine sampler

**Files:**
- Modify: `Cargo.toml` (workspace dep `sysinfo`), `crates/nidoo/Cargo.toml`
- Create: `crates/nidoo/src/system/mod.rs` (module root, `pub mod sample;` for now), `crates/nidoo/src/system/sample.rs`
- Modify: `crates/nidoo/src/lib.rs` (`pub mod system;`, alphabetical)

**Interfaces:**
- Produces in `sample.rs`:
  - `pub struct Resources { pub cpu: Option<CpuView>, pub ram: Option<MemView>, pub disk: Option<(u64, u64)> /* used, total */, pub temperature_c: Option<f64> }`
  - `pub struct Sampler` with `pub fn new(data_dir: PathBuf) -> Sampler` and `pub fn sample(&mut self) -> Resources` (keeps the `sysinfo::System` between calls so CPU % covers the last interval; the first call may return `cpu: None`).
  - Pure helpers: `pub fn hub_share(process_pct_of_one_core: f32, cpus: usize) -> f64` (÷ cpus, clamped 0–100); `pub fn disk_for(path: &Path, disks: &[(PathBuf, u64, u64)] /* mount, total, available */) -> Option<(u64, u64)>` (longest mount-point prefix of `path`; `None` when none matches or total is 0); `pub fn cpu_temperature(components: &[(String, f32)]) -> Option<f64>` (max of labels containing, case-insensitive, `cpu`, `package`, `coretemp`, `k10temp` or `soc`; else max of all; `None` when empty or every reading is NaN/≤ 0).

- [ ] **Step 1: Write failing tests** in `sample.rs`: `hub_share_divides_by_cpus` (`hub_share(200.0, 4) == 50.0`, `hub_share(500.0, 4) == 100.0`, `hub_share(10.0, 0) == 0.0`); `disk_for_picks_longest_mount` (mounts `/` and `/data`, path `/data` → the `/data` entry as `(total − available, total)`); `disk_for_without_match_is_none` (only `/boot`, path `/data` → `None`); `temperature_prefers_cpu_labels` (`[("acpitz", 40.0), ("coretemp Package id 0", 55.0)]` → `55.0`); `temperature_none_when_empty`; and `#[test] fn sample_reads_this_machine()` calling `sample()` twice 300 ms apart and asserting `ram` is `Some` with `used_bytes <= total_bytes` and `hub_bytes > 0`.
- [ ] **Step 2:** `cargo test -p nidoo --lib system::sample` → FAIL.
- [ ] **Step 3:** Implement. Hub process via `sysinfo::get_current_pid()`; server CPU = `global_cpu_usage()`; RAM used = `used_memory()`; `disk` from `Disks::new_with_refreshed_list()` mapped to `(mount_point, total_space, available_space)` then `disk_for(data_dir)`.
- [ ] **Step 4:** Tests PASS.
- [ ] **Step 5:** Commit `feat(system): machine sampler`.

---

### Task 3: Heartbeat file, boot classification, SIGTERM

**Files:**
- Create: `crates/nidoo/src/system/heartbeat.rs`
- Modify: `crates/nidoo/src/system/mod.rs`, `crates/nidoo/src/main.rs` (shutdown at ~line 126)

**Interfaces:**
- Produces:
  - `pub const FILE: &str = "system-heartbeat.json";`
  - `#[derive(Serialize, Deserialize)] pub struct Heartbeat { pub heartbeat: u64, pub clean_stop: Option<u64> }`
  - `pub enum Previous { None, Corrupt, Found(Heartbeat) }` and `pub fn read(dir: &Path) -> Previous` (missing file ⇒ `None`; unreadable / invalid JSON ⇒ `Corrupt`)
  - `pub fn classify(prev: &Previous, boot: u64) -> (StopKind, Option<u64>)`: `None` ⇒ `(First, Some(0))`; `Corrupt` ⇒ `(Unexpected, None)`; `clean_stop: Some(t)` ⇒ `(Clean, Some(boot.saturating_sub(t)))`; else `(Unexpected, Some(boot.saturating_sub(heartbeat)))`.
  - `pub fn write_heartbeat(dir: &Path, now: u64) -> io::Result<()>` (writes `clean_stop: None`) and `pub fn write_clean_stop(dir: &Path, now: u64) -> io::Result<()>`, both through `setup::fs::write_private_atomic`.
  - In `main.rs`: `async fn shutdown_signal() -> std::io::Result<()>` that resolves on Ctrl-C **or**, under `#[cfg(unix)]`, `tokio::signal::unix::signal(SignalKind::terminate())`. The `Cli::Run` path awaits it instead of `ctrl_c()`, then calls `hub.mark_clean_stop()` **before** `link.disconnect()`. (`run_setup` may use it too; no marker there.)
  - `Hub::mark_clean_stop(&self)` in `system/mod.rs`: writes the clean-stop marker into the setup ctx's `data_dir` (no-op without a setup ctx), logs and swallows errors.

- [ ] **Step 1: Write failing tests** in `heartbeat.rs`: `first_boot_is_first`; `clean_stop_counts_from_stop_time` (`clean_stop: Some(100)`, boot 160 ⇒ `(Clean, Some(60))`); `missing_marker_is_unexpected` (heartbeat 100, boot 400 ⇒ `(Unexpected, Some(300))`); `clock_backwards_saturates` (heartbeat 500, boot 400 ⇒ `Some(0)`); `corrupt_file_is_unexpected_unknown` (write `{"heart` to the file in a `tempfile::tempdir()`, `read` ⇒ `Corrupt`, classify ⇒ `(Unexpected, None)`); `heartbeat_then_clean_stop_round_trip` (write heartbeat 10, then clean stop 20, `read` ⇒ `Found{heartbeat: 10 or 20, clean_stop: Some(20)}`).
- [ ] **Step 2:** `cargo test -p nidoo --lib system::heartbeat` → FAIL.
- [ ] **Step 3:** Implement `heartbeat.rs`, `Hub::mark_clean_stop`, `shutdown_signal`. (`write_clean_stop` keeps the previous heartbeat value.)
- [ ] **Step 4:** Tests PASS. Manual check on macOS: `cargo run -p nidoo -- run --data-dir <tmp>` with a dev data dir, `kill -TERM <pid>`, then `cat <tmp>/system-heartbeat.json` shows `clean_stop` set.
- [ ] **Step 5:** Commit `feat(system): heartbeat file and clean stop on SIGTERM`.

---

### Task 4: Restart log, uptime %, 90-day cleanup

**Files:**
- Create: `crates/nidoo/migrations/0006_system.sql`, `crates/nidoo/src/system/restarts.rs`
- Test: `crates/nidoo/tests/history_db.rs` (DB-gated tests, same gating as the existing ones)

**Interfaces:**
- Produces:
  - Migration: `CREATE TABLE hub_restart (started_at timestamptz PRIMARY KEY, down_secs bigint, stop text NOT NULL CHECK (stop IN ('first','clean','unexpected')));`
  - `impl Db { pub async fn insert_restart(&self, r: &RestartView) -> Result<(), sqlx::Error>; pub async fn restarts_since(&self, since: u64) -> Result<Vec<RestartView>, sqlx::Error> /* newest first */; pub async fn first_start(&self) -> Result<Option<u64>, sqlx::Error>; pub async fn purge_system_before(&self, cutoff: u64) -> Result<u64, sqlx::Error> }`. `purge_system_before` deletes rows older than `cutoff` from `entity_state` and `entity_availability` (`entity_id = ANY($ids)` with `ids::ALL`), from `history_hourly` (`bucket < cutoff`, same ids) and from `hub_restart`; returns rows removed. Runtime `sqlx::query(...)` like `history/solar.rs`.
  - Pure: `pub fn uptime_pct(restarts: &[RestartView], first_start: Option<u64>, now: u64, window_secs: u64) -> f64`. Window start = `max(now − window_secs, first_start.unwrap_or(now − window_secs))`; downtime = Σ over restarts with `started_at > window_start` of `min(down_secs.unwrap_or(0), started_at − window_start)`; result = `100 × (window − downtime) / window`, clamped 0–100; window 0 ⇒ `100.0`.
  - `pub const WINDOW_SECS: u64 = 30 * 86_400; pub const KEEP_SECS: u64 = 90 * 86_400;`

- [ ] **Step 1: Write failing tests**: unit `uptime_full_month_one_hour_down` (one restart down 3600 s in a 30-day window, first start older ⇒ `≈ 99.861`); `uptime_young_hub_uses_first_start` (first start 1 day ago, one restart down 864 s ⇒ `99.0`); `uptime_without_restarts_is_100`; `uptime_clamps_downtime_to_window` (down 40 days ⇒ `0.0`, never negative). DB-gated `restart_log_round_trip` (insert 3, `restarts_since` returns newest first, `first_start` = oldest) and `purge_removes_only_system_rows` (an old `system_cpu` state row and an old `light_x` row; after purge only `light_x` remains).
- [ ] **Step 2:** `cargo test -p nidoo --lib system::restarts` → FAIL; DB-gated via the existing env gate → FAIL.
- [ ] **Step 3:** Implement migration, `impl Db`, `uptime_pct`.
- [ ] **Step 4:** Tests PASS (DB-gated ones with the local TimescaleDB the existing `history_db.rs` tests use; if unavailable say so in the report).
- [ ] **Step 5:** Commit `feat(system): restart log, uptime percentage, 90-day purge`.

---

### Task 5: Outage tracker for broker, Zigbee2MQTT and database

**Files:**
- Modify: `crates/nidoo/src/outage.rs`

**Interfaces:**
- Consumes: existing `Service`, `Outage::{new, failed, succeeded, tick}`, `THRESHOLD_SECS`.
- Produces: `Service::{Broker, Zigbee2mqtt, Database}` (keys `broker`, `zigbee2mqtt`, `database`; labels "the MQTT broker", "Zigbee2MQTT", "the database"); `pub fn threshold_secs(self) -> u64` (Weather / Solar ⇒ `THRESHOLD_SECS`, the new three ⇒ `5 * 60`), used by `tick` instead of the constant; `pub fn set_notify(&mut self, notify: bool)`. Alert JSON and loc keys (`service.unreachable`, `service.reachable_again`) unchanged.

- [ ] **Step 1: Write failing tests**: `broker_alerts_after_five_minutes` (failed at 0, `tick(299)` ⇒ `None`, `tick(300)` ⇒ down alert with `loc.args.service == "broker"`); `weather_still_waits_thirty_minutes` (`tick(300)` ⇒ `None`); `set_notify_off_silences_back_alert`.
- [ ] **Step 2:** `cargo test -p nidoo --lib outage` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** All `outage` tests PASS (old ones included).
- [ ] **Step 5:** Commit `feat(outage): broker, Zigbee2MQTT and database trackers with a 5-minute threshold`.

---

### Task 6: "Notify me" switch, stored without a restart

**Files:**
- Modify: `crates/nidoo/src/setup/sections.rs` (new `system` section), `crates/nidoo/src/config.rs` (`SystemConfig { notify: bool }` in `Config`), `crates/nidoo/src/hub.rs` (field + construction at ~line 412)
- Modify: `crates/nidoo/src/system/mod.rs`

**Interfaces:**
- Produces:
  - `#[derive(Default, …)] #[serde(default)] pub struct SystemSection { pub notify: bool }`; `Sections.system`; `SECTION_KEYS` becomes `[&str; 7]` with `"system"` last; `to_rows` / `from_rows` / `assemble` carry it into `Config.system.notify`. **Not** added to `setup/settings.rs::known`, so the generic `/settings/<section>` route (which restarts the hub) never touches it.
  - `Hub.system_notify: AtomicBool` (from config).
  - `pub async fn set_system_notify(&self, notify: bool) -> Result<bool, HubError>` on `Hub`: under `ctx.sections` lock, clones `Sections`, sets `system.notify`, `store::save_section(db, "system", &next)` **first** (DB error ⇒ `HubError` mapped to 503, nothing changed), then `store::write_cache(&ctx.data_dir, &next)`, then commits the lock value and the `AtomicBool`; returns the new value. No restart.

- [ ] **Step 1: Write failing tests**: in `sections.rs` update the `SECTION_KEYS` test and add `system_section_defaults_off_and_round_trips`; hub test `set_system_notify_writes_cache_without_restart` (setup ctx with a temp dir + test DB if the DB gate is on; asserts cache file JSON has `system.notify == true`, `ctx.restarting` is still false, `hub.system_notify` true); `set_system_notify_db_down_changes_nothing` (ctx with a `Db` pointing at an unreachable URL: returns an error whose HTTP mapping is 503; cache file unchanged; flag unchanged).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** PASS; `cargo test -p nidoo --lib setup` still green.
- [ ] **Step 5:** Commit `feat(system): notify switch stored without restarting the hub`.

---

### Task 7: Commands counter

**Files:**
- Modify: `crates/nidoo/src/hub.rs` (`send_command` publish loop ~line 1279, `on_group_command` loop ~line 1413), `crates/nidoo/src/system/mod.rs`

**Interfaces:**
- Produces: `Hub.commands_sent: AtomicU64` incremented once per **device command publish that succeeded** in both loops (not in `Hub::publish`, which also carries retained writes); `pub struct CommandsToday { day: Option<NaiveDate>, baseline: u64 }` with `pub fn count(&mut self, total: u64, today: NaiveDate) -> u64` (date change ⇒ baseline = total, returns 0; same day ⇒ `total − baseline`).

- [ ] **Step 1: Write failing tests**: unit `commands_today_resets_at_midnight` (count(5, d1) = 5 on first call with baseline 0, count(8, d1) = 8, count(9, d2) = 0, count(12, d2) = 3); hub tests in the existing hub test module: `light_command_counts_one` and `group_command_counts_each_member` (group of 2 lights ⇒ +2).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement (first day: `day: None` ⇒ set day, baseline 0).
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(system): count device commands`.

---

### Task 8: Retained-topic count and service status

**Files:**
- Modify: `crates/nidoo/src/hub.rs` (`extra_filters` ~line 1040; `ingest_message` ~line 725), `crates/nidoo/src/system/mod.rs`
- Create: `crates/nidoo/src/system/services.rs`

**Interfaces:**
- Produces:
  - `pub const RETAINED_TOPIC: &str = "$SYS/broker/retained messages/count";` added to `extra_filters`; `ingest_message` intercepts that exact topic first, parses a `u64` into `Hub.system_retained: Mutex<Option<u64>>` (unparsable ⇒ ignored) and returns no changes.
  - `pub struct ServiceStatus { state: ServiceState, since: Option<u64> }` with `pub fn observe(&mut self, state: ServiceState, now: u64)` (`since` set when the state changes, kept otherwise) and `pub fn view(&self, service: SystemService) -> ServiceView`.
  - Inputs per tick (read in Task 9): broker = `hub.connection()` current value (`Online` ⇒ Up, else Down); Zigbee2MQTT = `hub.read().bridge().state` (online ⇒ Up, offline ⇒ Down, unknown/never seen ⇒ Unknown); database = outcome of the tick's `pg_database_size` query (no history configured ⇒ Unknown).

- [ ] **Step 1: Write failing tests**: `sys_retained_count_is_captured_not_routed` (ingest `RETAINED_TOPIC` payload `"2431"` ⇒ returns empty changes, stored `Some(2431)`); `retained_unknown_until_seen` (`None` before any message); `extra_filters_include_retained_topic`; `service_since_changes_only_on_transition` (Up at 10, Up at 70 ⇒ since 10; Down at 130 ⇒ since 130).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(system): retained topic count and service status`.

---

### Task 9: System task: sensors, restart push, outage pushes, cleanup

**Files:**
- Create: `crates/nidoo/src/system/discovery.rs`
- Modify: `crates/nidoo/src/system/mod.rs`, `crates/nidoo/src/hub.rs` (field `system: Mutex<SystemState>`), `crates/nidoo/src/main.rs` (start + abort with the other tasks)

**Interfaces:**
- Consumes: Tasks 1–8.
- Produces:
  - `discovery.rs`: `pub fn configs(prefix: &str) -> Vec<(String, Value)>` — one retained config per `ids::ALL` entry. Topics (P = `hub.discovery_prefix`, as `network/topics.rs`): config `P/sensor/nidoo_hub/<id>/config` (`binary_sensor` for the three services), state `P/system/<id>`, availability `P/system/<id>/avail` (`online` / `offline`). Every config: `unique_id = <id>`, `entity_category: "diagnostic"` (kept off the Home and rooms; pickers list it under "+ Diagnostic"), `device: {"identifiers":["nidoo_hub"],"name":"Nidoo hub"}`. Units / classes: cpu & cpu_hub `%`; ram_used & ram_hub `MB` (`data_size`); disk_used & db_size `GB` (`data_size`, 2 decimals); temperature `°C` (`temperature`); devices, entities, retained, commands no unit (commands `state_class: total_increasing`); started `timestamp` (ISO 8601 state); services `connectivity` with `ON` / `OFF`.
  - `pub struct SystemState` (sampler, `CommandsToday`, three `ServiceStatus`, three `Outage`, last `Resources`, db size, `started_at`, cached restarts and uptime %, last published values + availability, last `link_epoch` seen, last purge day).
  - `Hub::start_system(self: &Arc<Self>) -> Option<JoinHandle<()>>` (always on; `None` only without a setup ctx). At start: `heartbeat::read` + `classify` with `clock::now()`, record the start (DB insert retried each tick until it succeeds), and if `stop == Unexpected` and notify is on, `raise_system` an alert `{"v":1,"id":"hub-restart-<boot>","channel":"system","severity":"warning","title":"Hub restarted","body":"The hub restarted after an unexpected stop (down <n> min).","silent":false,"ttl":86400,"loc":{"key":"hub.restarted_unexpectedly","args":{"minutes": <n or null>}}}` (body says "down an unknown time" when `None`). The push does not wait for the database.
  - Each 60 s tick (`tick_at(&self, now: u64)` public for tests): write heartbeat; sample (on `spawn_blocking`); query `pg_database_size(current_database())` (also feeds the database status); observe the three services and feed their `Outage` (`failed` / `succeeded` / `tick`, `set_notify` from the flag) → `raise_system`; refresh restarts + `uptime_pct` from the DB (keep the last good value when down); publish configs when the `link_epoch` changed since the last publish, then each state and availability on change (a value that disappears ⇒ `offline` availability, never a stale state; never-measured ⇒ config not published); once per local day run `purge_system_before(now − KEEP_SECS)`.
  - `Hub::system_view(&self) -> SystemView` (from memory, never blocks on the DB).

- [ ] **Step 1: Write failing tests** (hub tests with the in-memory publisher used by `network/` tests): `configs_cover_every_id` (15 configs, all diagnostic, device `nidoo_hub`, `system_disk_used` unit `GB`, `system_broker` class `connectivity`); `tick_publishes_states_and_avail` (after `tick_at`, the captured publishes include `P/system/system_cpu` and `P/system/system_devices`); `reconnect_republishes_configs` (bump `link_epoch`, `tick_at` again ⇒ all configs republished); `vanished_temperature_goes_offline` (inject resources with then without temperature ⇒ `P/system/system_temperature/avail` = `offline`); `unexpected_boot_pushes_when_notify_on` / `clean_boot_never_pushes` / `first_boot_never_pushes` (inspect the push submissions); `system_view_reports_unknown_retained_before_sys`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement; inject `Resources` through a test seam (e.g. `tick_with(&self, now, resources)` that `tick_at` calls after sampling).
- [ ] **Step 4:** PASS; `cargo test -p nidoo` green.
- [ ] **Step 5:** Commit `feat(system): publish system sensors, restart and outage pushes, daily purge`.

---

### Task 10: HTTP and MCP

**Files:**
- Modify: `crates/nidoo/src/http/api.rs` (routes ~line 40–110, handlers next to the network ones ~line 794), `crates/nidoo/src/mcp/tools.rs`, `crates/nidoo/src/mcp/mod.rs` (instructions: one sentence), `crates/nidoo/tests/http.rs`, `crates/nidoo/tests/mcp.rs` (`EXPECTED_TOOLS` line 31), `crates/nidoo/tests/mcp_stdio.rs` (`EXPECTED_TOOLS` line 26)

**Interfaces:**
- Produces: `GET /api/v1/system` → `Json<SystemView>`; `PUT /api/v1/system/notify` with `Body<NotifyInput>` → `Json<NotifyInput>` (503 when the DB save fails, 422 on a bad body). MCP `#[tool(name = "system_status", annotations(read_only_hint = true))] async fn system_status(&self) -> Json<SystemView>`.

- [ ] **Step 1: Write failing tests**: `http.rs` `system_view_is_served` (200, has `startedAt`, `services` length 3) and `put_system_notify_rejects_unknown_fields` (422); `mcp.rs` adds `"system_status"` to both `EXPECTED_TOOLS` and a call test asserting `services` is present.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p nidoo --test http --test mcp --test mcp_stdio` → PASS.
- [ ] **Step 5:** Commit `feat(system): HTTP view, notify endpoint and MCP tool`.

---

### Task 11: Web: System settings page with tiles

**Files:**
- Create: `crates/nidoo-web/src/i18n/system.rs`, `crates/nidoo-web/src/screens/system_settings.rs`, `crates/nidoo-web/tests/system_page.rs`
- Modify: `crates/nidoo-web/src/i18n/mod.rs` (`mod system; pub use system::*;`, `Key::System` in enum / `ALL` / `text()` = `("System", "Système")`), `crates/nidoo-web/src/i18n/settings.rs` (`HintSystem`), `crates/nidoo-web/src/screens/settings.rs` (`SettingsPage::System`, `ALL` 14 → 15, `slug` `"system"`, `key`, `hint`, `glyph` (a pulse/heart-rate icon from `icons`, colour `"mint"`), `group` ⇒ `GroupHub`, row `status` = "Up 12 d 4 h" from a fetched view, page switch ⇒ `<SystemSettingsPage />`), `crates/nidoo-web/src/screens/mod.rs`, `crates/nidoo-web/src/api/core.rs` (`get_system()`, `set_system_notify(bool)`), `crates/nidoo-web/styles/main.css` (`.system__*` block, using `--card-bg`, `--card-border`, `--radius-card`, `--grid-gap`; Night Glass and Ivoire both checked)

**Interfaces:**
- Consumes: Task 1 types, Task 10 endpoints.
- Produces: `pub async fn get_system() -> Result<SystemView, ApiError>` (like `list_network_watches`, `GET /api/v1/system`); `pub async fn set_system_notify(notify: bool) -> Result<NotifyInput, ApiError>`; `#[component] pub fn SystemSettingsPage() -> impl IntoView` fetching on mount and every 60 s while mounted; `SystemText` enum with `system_text(lang, k)` / `use_system_text()` (pattern of `i18n/network.rs`); `pub fn uptime_text(lang: Lang, secs: u64) -> String` ("12 d 4 h" / "12 j 4 h", "3 h 5 min", "4 min").
- Layout (spec, layout B): uptime card (`Up …`, `since <date time>`, `<pct> % up over 30 days`, `<n> restarts`; links to `?show=restarts`); grid of tiles in this order: CPU, RAM, Disk, Temperature (hidden when `None`), Devices (sub-line entities), Retained ("unknown" when `None`), Commands today, Broker, Zigbee2MQTT, Database (`dot dot--online|offline|unknown`, "since <time>"); "Notify me" switch via `SwitchRow` (`screens/alarm_settings.rs:257`) with a hint line; a failed `set_system_notify` reverts the switch and shows the error text. Chart tiles link to `?show=cpu|ram|disk|temperature` (Task 12 renders those; until then the link may be inert).

- [ ] **Step 1: Write failing tests** (`system_page.rs`, `FetchStub` like `network_page.rs`): `settings_list_shows_system_entry`; `tiles_show_values_and_units` (stub view ⇒ CPU tile `23 %` with `hub 4 %`, RAM `32 %`, Disk sub-line database size); `unknown_retained_reads_unknown` and `missing_temperature_hides_tile`; `down_service_is_red` (Zigbee2MQTT `down` ⇒ `.dot--offline`); `notify_switch_puts_and_reverts_on_503`. Unit test `uptime_text_formats` (`86400*12+4*3600` ⇒ `"12 d 4 h"` EN, `"12 j 4 h"` FR; `240` ⇒ `"4 min"`).
- [ ] **Step 2:** `cd crates/nidoo-web && wasm-pack test --headless --chrome --test system_page` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** PASS; `cargo test -p nidoo-web` (bare_text, settings `ALL` tests) PASS.
- [ ] **Step 5:** Commit `feat(web): Settings → System tiles`.

---

### Task 12: Web: two-line sparkline, full chart, restart list

**Files:**
- Modify: `crates/nidoo-web/src/components/sparkline.rs`, `crates/nidoo-web/src/screens/system_settings.rs`, `crates/nidoo-web/src/nav.rs` (`parent_of`: `/settings/system?show=…` ⇒ `/settings/system`), `crates/nidoo-web/src/i18n/system.rs`, `crates/nidoo-web/styles/main.css`
- Create: `crates/nidoo-web/src/components/system_chart.rs`
- Test: `crates/nidoo-web/tests/system_page.rs`, unit tests in `sparkline.rs`

**Interfaces:**
- Produces:
  - `pub fn sparkline_pair_points(a: &[(i64, f64)], b: &[(i64, f64)], width: f64, height: f64) -> (Option<String>, Option<String>)` — one shared time and value scale over both series (refactor `sparkline_points` onto a shared `(t_lo, t_hi, v_lo, v_hi)` helper, behaviour of the single version unchanged).
  - `#[component] pub fn SparklinePair(server: String, #[prop(optional)] hub: Option<String>) -> impl IntoView` using `day_series` / `cached_day_series`, two `<polyline>`s with classes `spark__server` / `spark__hub`. Pairs: CPU (`CPU`, `CPU_HUB`), RAM (`RAM_USED`, `RAM_HUB`), Disk (`DISK_USED`, `DB_SIZE`), Temperature (`TEMPERATURE`, none).
  - `#[component] pub fn SystemChart(server: String, hub: Option<String>, range: Signal<HistoryRange>, server_label: Signal<String>, hub_label: Signal<String>) -> impl IntoView` — `fetch_series` per entity, shared `y_scale` over both, `smooth_path` per series, two `<path class="history-chart__line …">`, two-item key reusing `.history-chart__key*`. Range chips 24 h / 7 d / 30 d only.
  - In the page: `?show=cpu|ram|disk|temperature` renders the chart view with a back link; `?show=restarts` renders the restart list (newest first: start date-time, "down X" or "down ?", "Clean stop" / "Unexpected stop" / "First start").

- [ ] **Step 1: Write failing tests**: unit `pair_points_share_one_scale` (a = flat 50, b = flat 10 over the same times, height 28 ⇒ b's y values greater than a's, both polylines `Some`); `pair_points_without_hub_matches_single` (b empty ⇒ first equals `sparkline_points(a, …)`); wasm `cpu_tile_opens_two_line_chart` (stub both series ⇒ `?show=cpu` has two `.history-chart__line` paths and the key texts "Server", "Hub"); `restarts_list_newest_first_with_unknown_downtime`; `back_from_chart_returns_to_tiles`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** PASS; existing `sparkline` / `history_chart` tests still PASS.
- [ ] **Step 5:** Commit `feat(web): system charts with the hub line and the restart list`.

---

### Task 13: Docs and TODOS

**Files:**
- Create: `docs/users/system-health.md`
- Modify: `docs/users/web-ui.md` (Settings → System line), `docs/users/push-notifications.md` (new `hub.restarted_unexpectedly` key + `service.unreachable` services `broker`, `zigbee2mqtt`, `database`), `docs/users/assistants-mcp.md` (`system_status`), `docs/users/history.md` (90-day cap for `system_*` only), `docs/users/install-on-linux.md` (temperature may be missing in a VM; `$SYS` must be readable by the hub's broker user for the retained count), `TODOS.md` (drop the "System health screen" item; add "Android app: localise `hub.restarted_unexpectedly` and the three new `service.*` services" under Flows / app items)

- [ ] **Step 1:** Write the page: what each tile means, the sensors table (id, unit), clean vs unexpected stop (SIGTERM), Notify me, retention. Generic examples only.
- [ ] **Step 2:** `tools/ci/brand-check.sh` → OK; docs site build if the repo's docs workflow has a local command (see `docs/contributors/development.md`).
- [ ] **Step 3:** Commit `docs: system health`.

---

## After the plan

Whole-branch review, then push and open the PR (Gekkotron token if `403`). After merge: rebuild hub + web on the live hub, check `system-heartbeat.json` appears in the data dir, that the Disk and Temperature tiles read real values there, and that a `docker compose restart hub` is listed as a clean stop with no push.
