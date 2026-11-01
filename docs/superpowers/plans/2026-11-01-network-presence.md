# Network Presence by Ping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The hub pings hosts the user picks, per device or standalone, and publishes each as a `connectivity` binary sensor that flows, dashboards, history and the Android app can use.

**Architecture:** A new hub module `crates/oikos-os/src/network/` keeps one retained definition per watch under the first discovery prefix (like groups), publishes a hub-owned `binary_sensor` per watch (like virtual entities, plus an availability topic), and runs one supervisor task that keeps one ping loop per enabled watch. A pure `Tracker` turns ping results into state changes (3 misses → off, first reply → on). The web UI gets a Settings › Network presence page and a Network panel on the entity sheet; REST and MCP get list/set/delete.

**Tech Stack:** Rust (tokio, serde, schemars, axum, rmcp), `surge-ping` 0.9 for ICMP, Leptos (wasm) for the web UI.

**Spec:** `docs/superpowers/specs/2026-11-01-network-presence-design.md` (read it first; mockups in `docs/superpowers/specs/2026-11-01-network-presence/`).

## Global Constraints

- Watch id: `[a-z0-9_]{1,48}` (reuse `virtual_entities::validate_id`). Entity unique id: `network_<id>`.
- Interval: default **30 s**, allowed **10 s to 600 s** inclusive; anything else is `HubError::Rejected`.
- Disconnected after **3** missed pings in a row (fixed constant `MISSES = 3`); connected on the **first** reply.
- Ping timeout per probe: `min(interval, 5 s)`.
- "The hub cannot ping" changes only the entity's **availability** (`offline`), never its value.
- Paused (`enabled: false`): no pings, definition kept, entity stays published with availability `offline`.
- Nothing is published on the state topic at start until the first verdict; only real value changes are published.
- One watch per device: a second watch with the same `device` is `Rejected`.
- Every user-visible string in English and French via the i18n tables; no bare literals in views (`tests/bare_text.rs`).
- Tracked files must stay generic: example hosts are `192.168.1.42`, `nas.local`; no real names, IPs or rooms.
- Commits: Gekkotron identity (`-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`), one commit per task, explicit paths in `git add`.

## Topic layout (decided; discovery prefix `P`, `oikos` in tests)

| What | Topic | Retained |
|---|---|---|
| Definition (JSON `NetworkWatchSpec`) | `P/network/<id>/definition` | yes |
| Discovery config | `P/binary_sensor/network/<id>/config` | yes |
| State | `P/network/<id>` — `ON` / `OFF` | yes |
| Availability | `P/network/<id>/avail` — `online` / `offline` | yes |

`network` is added to `RegistryConfig::default_reserved()` (`crates/oikos-core/src/registry/mod.rs:43`) so these are never parsed as discovery.

## Review Focus

1. **Hub restart with a host that is down** → no flow fires on boot: no state publish until the 3rd miss, and then only if it differs from the retained value. Test in Task 4 (`restart_publishes_nothing_before_the_first_verdict`).
2. **ICMP socket refused (Docker without ping permission)** → every enabled watch goes availability `offline`, value untouched, live status `cannot_ping`; recovers on its own when a probe succeeds again. Test in Task 4 (`cannot_ping_touches_only_availability`).
3. **Hostname that does not resolve** → counts as a miss (not "cannot ping"). Test in Task 4 (`unresolvable_host_is_a_miss`).
4. **Editing interval/host of a running watch, or pausing it** → the old loop stops at once, no stray publish after a pause. Test in Task 4 (`pausing_stops_the_loop_and_marks_offline`).
5. **Deleting a device-linked watch while its device still exists** → only the network entity disappears; the device's own entities are untouched. Test in Task 3 (`delete_clears_only_network_topics`).

---

### Task 1: API types

**Files:**
- Create: `crates/oikos-api/src/network.rs`
- Modify: `crates/oikos-api/src/lib.rs` (add `mod network;` + `pub use network::*;` next to groups)

**Interfaces:**
- Produces:
  - `NetworkWatchSpec { name: String, host: String, #[serde(rename="intervalS", default=30)] interval_s: u32, #[serde(default=true)] enabled: bool, #[serde(default)] area: Option<String>, #[serde(default)] device: Option<String> }` — derives `Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema`, `#[serde(deny_unknown_fields)]`.
  - `enum WatchState { Waiting, Connected, Disconnected, Paused, CannotPing }` — `#[serde(rename_all="snake_case")]`, `Copy, Eq`.
  - `WatchStatus { state: WatchState, since: Option<u64>, last_reply: Option<u64>, rtt_ms: Option<f64> }` (Unix seconds), camelCase.
  - `NetworkWatchView { id: String, spec: NetworkWatchSpec, entity: String, status: WatchStatus }`, camelCase. `entity` is `network_<id>`.

- [ ] **Step 1: Failing tests** in `network.rs` `#[cfg(test)]`: `spec_defaults_interval_and_enabled` (`{"name":"NAS","host":"nas.local"}` → `interval_s == 30`, `enabled == true`, `area/device == None`); `spec_rejects_unknown_fields` (`{"name":"a","host":"b","x":1}` → `Err`); `view_serializes_camel_case` (keys `intervalS`, `lastReply`, `rttMs`, state `"cannot_ping"`).
- [ ] **Step 2:** `cargo test -p oikos-api network` → fails to compile.
- [ ] **Step 3:** Implement the types above.
- [ ] **Step 4:** `cargo test -p oikos-api network` → PASS; `cargo check -p oikos-api --target wasm32-unknown-unknown` → ok.
- [ ] **Step 5:** Commit `feat(api): network watch types`.

### Task 2: Pure network core (topics, tracker, discovery config, store)

**Files:**
- Create: `crates/oikos-os/src/network/mod.rs` (re-exports), `topics.rs`, `tracker.rs`, `discovery.rs`, `store.rs`
- Modify: `crates/oikos-os/src/lib.rs` (`pub mod network;`), `crates/oikos-core/src/registry/mod.rs:43` (add `"network"` to `default_reserved`)

**Interfaces:**
- Produces (`network::topics`): `definition_topic(prefix,id)`, `config_topic(prefix,id)`, `state_topic(prefix,id)`, `availability_topic(prefix,id)`, `unique_id(id) -> String`, `id_for_device(device_id: &str) -> Option<String>` (lowercase, every char outside `[a-z0-9_]` → `_`, `None` if empty or > 48), `validate_spec(&NetworkWatchSpec) -> Result<(), String>` (non-empty trimmed `name` and `host`, host without whitespace, interval 10..=600).
- Produces (`network::tracker`): `pub const MISSES: u32 = 3;` `pub enum Probe { Reply(Duration), Miss, CannotPing(String) }`; `pub struct Tracker` with `new() -> Tracker`, `fn on_probe(&mut self, probe: &Probe, now: u64) -> Verdict`, `fn status(&self) -> WatchStatus`; `pub struct Verdict { pub value: Option<bool>, pub available: Option<bool> }` — each `Some` only when it changed and must be published.
- Produces (`network::discovery`): `build_config(prefix, id, spec, device_block: Value) -> Value` with `name`, `unique_id`, `state_topic`, `availability_topic`, `payload_on "ON"`, `payload_off "OFF"`, `payload_available "online"`, `payload_not_available "offline"`, `device_class "connectivity"`, `device`. `standalone_device(id, name) -> Value` = `{"identifiers":["oikos_network_<id>"],"name":<name>}`.
- Produces (`network::store`): `NetworkStore { defs: BTreeMap<String, NetworkWatchSpec>, rev: u64 }`, `apply(&mut self, id, payload) -> ApplyOutcome` (reuse `groups::ApplyOutcome`; empty payload deletes; bad JSON → `Rejected`), `get`, `all`, `rev()` (bumped on every change).

- [ ] **Step 1: Failing tests:**
  - `topics`: `topics_live_under_the_network_subtree` (`oikos`, `nas` → `oikos/network/nas/definition`, `oikos/binary_sensor/network/nas/config`, `oikos/network/nas`, `oikos/network/nas/avail`); `id_for_device_sanitizes` (`"roborock_40313ca8b964"` → same; `"Living-TV.1"` → `"living_tv_1"`; 49 chars → `None`); `interval_bounds` (9 and 601 rejected, 10 and 600 accepted); `blank_host_rejected`.
  - `tracker`: `waits_until_first_verdict` (new tracker, `status().state == Waiting`); `first_reply_connects` (`Reply` → `value Some(true)`, `available Some(true)`); `two_misses_change_nothing_third_disconnects` (after a reply: Miss, Miss → `value None`; third Miss → `Some(false)`); `reply_resets_miss_count` (Miss, Miss, Reply, Miss, Miss → never `Some(false)`); `cannot_ping_only_flips_availability` (after a reply: `CannotPing` → `value None, available Some(false)`, state `CannotPing`; next `Reply` → `available Some(true)`, `value None`); `three_misses_from_start_publish_off` (Miss×3 from new → `Some(false)`); `status_reports_rtt_and_last_reply` (`Reply(4 ms)` at `now=100` → `last_reply Some(100)`, `rtt_ms Some(4.0)`).
  - `discovery`: `config_is_a_connectivity_binary_sensor` (asserts the keys above and `unique_id == "network_nas"`).
  - `store`: `apply_round_trip_and_delete`, `bad_json_rejected_state_unchanged`, `rev_bumps_on_change_only`.
  - registry (`oikos-core`, next to the existing reserved-topic tests): `network_subtree_is_reserved` — `oikos/network/nas` and `oikos/network/nas/avail` classify as `TopicClass::Reserved("network")`; `oikos/binary_sensor/network/nas/config` still classifies as `Discovery`.
- [ ] **Step 2:** `cargo test -p oikos-os network:: && cargo test -p oikos-core registry` → fail.
- [ ] **Step 3:** Implement. `Tracker` keeps `misses: u32`, `value: Option<bool>`, `available: Option<bool>`, `since`, `last_reply`, `rtt`; `since` is set when `value` changes.
- [ ] **Step 4:** Same commands → PASS.
- [ ] **Step 5:** Commit `feat(network): topics, tracker, discovery config and store`.

### Task 3: Hub wiring — config, ingest, set/delete/list

**Files:**
- Modify: `crates/oikos-os/src/config.rs` (`NetworkConfig { enabled: bool }`, `#[serde(default)]`, **default `true`**, field `network` on `Config`; doc in `docs/reference/configuration.md` under a new `### network`)
- Modify: `crates/oikos-os/src/hub.rs` (fields `network_store: Mutex<NetworkStore>`, `network_enabled: bool`, `network_status: Mutex<BTreeMap<String, WatchStatus>>`, `network_rev: watch::Sender<u64>`; `ingest_network` branch next to `ingest_group` at `hub.rs:519`)
- Create: `crates/oikos-os/src/network/hub.rs` (the `impl Hub` block below)
- Test: `crates/oikos-os/tests/network.rs`

**Interfaces:**
- Consumes: Task 1 types, Task 2 functions.
- Produces:
  - `pub async fn set_network_watch(&self, id: &str, spec: NetworkWatchSpec) -> Result<NetworkWatchView, HubError>` — `require_network()` (`HubError::Publish("network presence disabled")`), `validate_id`, `validate_spec`; if `spec.device` is set: the device must exist in the registry (else `Rejected`), `id` must equal `id_for_device(device)`, and no other watch may hold that device (else `Rejected`). Device block: the device's discovery `device.identifiers` (from any of its entities' bindings) as `{"identifiers":[...]}`; when none, `standalone_device`. Publishes retained: definition, config, and availability (`offline` when `enabled == false`); state only for a new watch is **not** published. Applies locally via `self.ingest(...)` like `set_group`. When `spec.area` is set (standalone), writes the entity's UI override `area` through the existing override writer used by `set_override`.
  - `pub async fn delete_network_watch(&self, id: &str) -> Result<(), HubError>` — empty retained payloads on definition, config, state, avail; unknown → `UnknownEntity("network_<id>")`.
  - `pub fn list_network_watches(&self) -> Result<Vec<NetworkWatchView>, HubError>` — status from `network_status`, `Paused` for disabled watches, `Waiting` when absent.
  - `pub(crate) fn network_store(&self) -> MutexGuard<NetworkStore>`, `pub(crate) fn subscribe_network_rev(&self) -> watch::Receiver<u64>` (bumped by `ingest_network` on every store change), `pub(crate) fn set_network_status(&self, id: &str, status: WatchStatus)`.

- [ ] **Step 1: Failing tests** in `tests/network.rs` (helper `network_hub() -> (Arc<Hub>, mpsc::Receiver<Publish>)` modelled on `climate_hub.rs::hub`, prefix `oikos`, online):
  - `set_publishes_definition_config_and_availability` — standalone `nas` → publishes on `oikos/network/nas/definition`, `oikos/binary_sensor/network/nas/config` (retained), nothing on `oikos/network/nas`; `list_network_watches()` has one view with state `Waiting`; the registry has entity `network_nas` with `device_class == Some("connectivity")`.
  - `paused_watch_is_published_offline` — `enabled: false` → avail payload `offline`; view state `Paused`.
  - `device_watch_joins_the_device` — ingest a vacuum discovery config with `device.identifiers ["vac_1"]`, then set watch `vac_1` with `device: Some("vac_1")` → config `device.identifiers == ["vac_1"]`.
  - `second_watch_on_same_device_rejected` and `device_watch_id_must_match_device` → `HubError::Rejected`.
  - `delete_clears_only_network_topics` — after delete: empty retained payloads on the four network topics only; the vacuum entity still in the registry.
  - `retained_definition_is_loaded` — `hub.ingest("oikos/network/nas/definition", json)` → listed.
  - `disabled_feature_is_503` — `cfg.network.enabled = false` → `HubError::Publish`.
- [ ] **Step 2:** `cargo test -p oikos-os --test network` → fail.
- [ ] **Step 3:** Implement the wiring above.
- [ ] **Step 4:** `cargo test -p oikos-os --test network` and `cargo test -p oikos-os --lib` → PASS.
- [ ] **Step 5:** Commit `feat(network): hub stores, publishes and removes network watches`.

### Task 4: Prober and supervisor

**Files:**
- Modify: `Cargo.toml` (workspace dep `surge-ping = "0.9"`), `crates/oikos-os/Cargo.toml`
- Create: `crates/oikos-os/src/network/probe.rs`, `crates/oikos-os/src/network/runner.rs`
- Modify: `crates/oikos-os/src/main.rs:112-160` (`let network = hub.start_network();`, abort on shutdown)
- Test: `crates/oikos-os/tests/network.rs` (more tests), using `#[tokio::test(start_paused = true)]`

**Interfaces:**
- Produces:
  - `pub trait Prober: Send + Sync + 'static { fn probe<'a>(&'a self, host: &'a str, timeout: Duration) -> BoxFuture<'a, Probe>; }` (`futures_util::future::BoxFuture`).
  - `pub struct IcmpProber` implementing it: resolve with `tokio::net::lookup_host((host, 0))` (failure or no IPv4/IPv6 address → `Probe::Miss`); `surge_ping::Client` built **once** with `sock_type_hint(Type::DGRAM)`, falling back to `Type::RAW`; if neither opens → every probe returns `Probe::CannotPing(reason)` and the hub logs it once at `warn`. Timeout → `Miss`.
  - `impl Hub { pub fn start_network(self: &Arc<Self>) -> Option<JoinHandle<()>> }` (None when disabled) → `start_network_with(Arc<dyn Prober>)` for tests.
  - Supervisor: waits for broker Online, then the SubAck and the retained settle like `groups/migrate.rs:79`; then on every `subscribe_network_rev()` change reconciles `BTreeMap<String, (NetworkWatchSpec, JoinHandle<()>)>`: abort loops whose watch was deleted, paused or whose spec changed; spawn loops for new/changed enabled watches. A paused watch gets status `Paused`; avail `offline` was already published by `set_network_watch`.
  - Per-watch loop: first probe after a random delay in `0..interval` (jitter), then every `interval`; feeds `Tracker::on_probe`; publishes `Verdict.value` as `ON`/`OFF` on the state topic and `Verdict.available` as `online`/`offline` on the avail topic (retained, via `self.publish`), then `set_network_status`.
- [ ] **Step 1: Failing tests** with a `FakeProber` (script of `Probe` per host, behind a `Mutex<VecDeque>`; `Miss` when empty):
  - `three_misses_publish_off_once` — script Reply, Miss×3 → state publishes exactly `ON` then `OFF`.
  - `restart_publishes_nothing_before_the_first_verdict` — Miss×2 → no publish on `oikos/network/nas`.
  - `cannot_ping_touches_only_availability` — Reply, CannotPing, Reply → state publishes only `ON`; avail publishes `online`, `offline`, `online`; status passes through `CannotPing`.
  - `unresolvable_host_is_a_miss` — real `IcmpProber` unit test in `probe.rs`: host `"does-not-resolve.invalid"` → `Probe::Miss` (skip with a message if the socket cannot open: `CannotPing`).
  - `pausing_stops_the_loop_and_marks_offline` — running watch, `set_network_watch(enabled: false)` → after advancing 5 intervals no further probes are consumed and no state publish.
  - `interval_change_restarts_the_loop` — 30 s → 10 s: probes consumed at the new pace.
- [ ] **Step 2:** `cargo test -p oikos-os --test network` → fail.
- [ ] **Step 3:** Implement `probe.rs`, `runner.rs`, wire `main.rs`.
- [ ] **Step 4:** `cargo test -p oikos-os --test network` → PASS; run the hub locally against no broker is not needed.
- [ ] **Step 5:** Commit `feat(network): ping loops with a three-miss tracker`.

### Task 5: REST, MCP and reference docs

**Files:**
- Modify: `crates/oikos-os/src/http/api.rs` (routes next to groups at :67: `.route("/network", get(list_network_watches))`, `.route("/network/{id}", put(put_network_watch).delete(delete_network_watch))`; handlers shaped like `put_group`/`delete_group`)
- Modify: `crates/oikos-os/src/mcp/params.rs` (`SetNetworkWatchParams { id, spec: NetworkWatchSpec }`, `DeleteNetworkWatchParams { id }`, `NetworkWatchList { watches: Vec<NetworkWatchView> }`), `mcp/tools.rs` (tools `list_network_watches` read-only, `set_network_watch` idempotent, `delete_network_watch` → `Deleted`), `mcp/mod.rs` instructions (one sentence: what a watch is, `network_<id>`, pause with `enabled:false`, one per device)
- Modify: `docs/reference/http-api.md` (`## Network presence` after `## Virtual entities`), `docs/reference/mcp-tools.md` (`## Network presence` table; tool count 47 → 50 and its breakdown), `docs/reference/mqtt-topics.md` (four rows from the topic table)
- Test: `crates/oikos-os/tests/http.rs`, `crates/oikos-os/tests/mcp.rs`
- [ ] **Step 1: Failing tests:** `http_network_round_trips` (PUT `/api/v1/network/nas` → 200 with `entity == "network_nas"`; GET `/network` lists it; DELETE → 204; DELETE again → 404); `http_network_bad_interval_is_422` (`intervalS: 5`); MCP `lists_the_expected_tools` gains the three names; `mcp_set_network_watch_round_trips` via `call(&client, "set_network_watch", …)`.
- [ ] **Step 2:** `cargo test -p oikos-os --test http network && cargo test -p oikos-os --test mcp` → fail.
- [ ] **Step 3:** Implement routes, tools, docs.
- [ ] **Step 4:** Same commands → PASS.
- [ ] **Step 5:** Commit `feat(network): REST and MCP for network watches`.

### Task 6: Web — client, i18n, Settings › Network presence

**Files:**
- Modify: `crates/oikos-web/src/api/core.rs` (`list_network_watches()`, `set_network_watch(id, &NetworkWatchSpec)`, `delete_network_watch(id)`, copying `list_groups`/`set_group`/`delete_group` at :180-193)
- Create: `crates/oikos-web/src/i18n/network.rs` (`NetworkText` + `network_text(lang, k)` + `use_network_text()`, with the `listed`/`ALL`/`assert_table` test like `i18n/alarm.rs`); register in `i18n/mod.rs`
- Create: `crates/oikos-web/src/screens/network_settings.rs` (`NetworkSettingsPage`)
- Modify: `crates/oikos-web/src/screens/settings.rs` (variant `NetworkPresence`, slug `"network-presence"`, placed **after `Alarm`** in `ALL` — now 14 —, `key()`, `hint()`, `status` = `"N watched"` / `"N surveillés"` counting enabled watches, body match arm)
- Test: `crates/oikos-web/tests/network_page.rs` (model on `ambiances_page.rs` with `FetchStub`)

**Interfaces:**
- Consumes: Task 1 types; REST from Task 5.
- Produces: `pub fn interval_options() -> &'static [u32]` = `[10, 15, 30, 60, 120, 300, 600]` and `pub fn interval_label(s: u32, lang) -> String` (`"30 s"`, `"2 min"`) in `screens/network_settings.rs`, reused by Task 7. `pub fn state_label(status: &WatchStatus, now: u64, lang) -> String` (`"connected"`, `"disconnected for 14 min"`, `"paused"`, `"waiting for the first reply"`, `"the hub cannot send pings"`).

Layout follows the mockup `network-presence-*.webp`: panel **Watched hosts** (`sheet__panel alarm-section`, `ul.alarm-rows`, one `li.alarm-row` per watch: `dot dot--online|offline|unknown`, name, "device" tag when `spec.device` is set, host · every … · room · state, **Edit** pushed right), panel **Add host** (Name, IP or hostname, Ping every, Room — `climate-field` + `field`, two columns; hint "A device the hub already knows is switched on from its own sheet."; **Add** chip). Edit expands the row into the same fields plus the `switch` (Watch on network) and **Remove**. New standalone ids: slug of the name via the same rule as `id_for_device`, suffixed `_2`, `_3`… on clash. The list refetches every 10 s while mounted and after each save.

- [ ] **Step 1: Failing tests:** `lists_watches_with_state` (stub GET `/api/v1/network` with a connected device watch and a disconnected standalone one → two `.alarm-row`, texts contain `"device"` and `"disconnected for"`); `add_host_puts_a_standalone_watch` (fill fields, click Add → PUT `/api/v1/network/nas` with `{"name":"NAS","host":"nas.local","intervalS":60,"enabled":true,"area":…}`); `pause_from_edit_puts_enabled_false`; `remove_deletes`; `settings_list_shows_network_presence_entry`. i18n table test via `cargo test -p oikos-web --lib`.
- [ ] **Step 2:** `wasm-pack test --headless --chrome crates/oikos-web --test network_page` → fail (CI job "web UI browser tests" runs these).
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same command → PASS; `cargo test -p oikos-web --lib` and `cargo test -p oikos-web --test bare_text` → PASS.
- [ ] **Step 5:** Commit `feat(web): Settings › Network presence`.

### Task 7: Web — Network panel on the entity sheet

**Files:**
- Create: `crates/oikos-web/src/screens/network_panel.rs` (`NetworkPanel { entity: EntityView }`)
- Modify: `crates/oikos-web/src/screens/entity_sheet.rs` (`SheetBody` :239: insert after the Settings panel at :296, before the diagnostics `<details>`)
- Test: `crates/oikos-web/tests/network_panel.rs`

**Interfaces:**
- Consumes: Task 6 `interval_options`, `interval_label`, `state_label`, client functions, `NetworkText`.
- Shown when: `entity.device_id` is `Some`, no `house.devices` entry has that id (not a Zigbee2MQTT device), and `entity.id` starts with none of `group_`, `virtual_`, `network_`. The watch shown is the one whose `spec.device == entity.device_id`.

Layout follows `device-sheet-*.webp`: heading **Network**, `override__flag` row with the `switch` (Watch on network), two-column IP or hostname + Ping every, hint "Disconnected after 3 missed pings in a row ({3×interval}). Flows see it as "{device} network".", `sheet__rows` with **State** (dot + label) and **Last reply** (`"12 s ago · 4 ms"`). Switching on with no watch: shows the fields empty, PUT on save with `id = id_for_device(device_id)` (same rule as the hub; duplicate it in the web crate with the same test cases), `name = "<device name> network"`, `device = device_id`. Switching off: PUT with `enabled: false`. Edits save on change like the Settings panel does.

- [ ] **Step 1: Failing tests:** `panel_hidden_on_zigbee_device`, `panel_hidden_on_group_entity`, `panel_shows_existing_watch_state`, `switch_on_puts_device_watch` (PUT `/api/v1/network/vac_1` with `device:"vac_1"`), `switch_off_pauses` (PUT with `enabled:false`, fields kept).
- [ ] **Step 2:** `wasm-pack test --headless --chrome crates/oikos-web --test network_panel` → fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same command → PASS.
- [ ] **Step 5:** Commit `feat(web): Network panel on the device sheet`.

### Task 8: User docs and TODOS

**Files:**
- Create: `docs/users/network-presence.md` (front matter like `groups-and-virtual-entities.md`): what it does, switching it on from a device, adding a host, pause vs Remove, the 3-miss rule, a flow example (trigger.state on `network_<id>` to `off` → notify), "The hub cannot send pings" with the fix: on the host, `sysctl net.ipv4.ping_group_range="0 2147483647"` (the hub uses host networking, so a container `sysctls` entry does not apply); the Android app sees the entity.
- Modify: `TODOS.md` (remove the "Network presence by ping" item)
- [ ] **Step 1:** Write the page; `cd website && npm run build` (or the docs CI command in `.github/workflows/docs.yml`) → builds with no broken links.
- [ ] **Step 2:** Commit `docs: network presence`.

---

## Final verification

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo check -p oikos-web --target wasm32-unknown-unknown --tests`.
- Push, open the PR against `main` (push with the Gekkotron token if the active gh account is the work one), CI green (including "web UI browser tests").
- After merge: rebuild the live hub; watch the Roborock from its sheet; check `network_roborock_40313ca8b964` appears in the flow entity picker.
