# Flow labels and choices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** User-defined labels on flows, choices with one current label, a `flow.label` action, label grouping on the Flows page, and the trigger chips folded into one menu.

**Architecture:** Pure catalog types and the switch planner live in `nidoo-core` (`flow/labels.rs`, shared by hub and web). The hub owns one retained catalog document, serialises every label write behind one async lock, and applies switches through the existing `set_flow_enabled` paths. The web reads the catalog through HTTP + a new `WsEvent`, and renders grouping, the manager, tagging and the node card.

**Tech Stack:** Rust workspace; serde/schemars; tokio; axum (HTTP); rmcp (MCP); Leptos (web, wasm-bindgen browser tests).

**Spec:** `docs/superpowers/specs/2026-11-10-flow-labels-design.md` (+ the two `2026-11-10-flow-labels-*-mockup.html` beside it).

## Global Constraints

- Branch `feat/flow-labels` (already created, spec committed as `809217fe`). Commit as `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Green before each commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the touched crates' tests (`cargo test -p nidoo-core`, `-p nidoo-api`, `-p nidoo-web` native, `cargo test -p nidoo --test integration <module>`), and for web UI tasks `cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test <file>`. Also `cargo check -p nidoo-web --target wasm32-unknown-unknown`.
- Never override `CARGO_TARGET_DIR`.
- Public, generic repo: fixtures and docs use generic names only (`Hiver`/`Été`/`Vacances` are fine as examples).
- Every user-visible web string goes through `FlowText` (EN + FR) and passes `every_word_has_both_languages` and `crates/nidoo-web/tests/bare_text.rs`.
- Retained documents stay backward compatible: new fields `#[serde(default)]`, unknown keys kept in `extra`.
- Ids: `[a-z0-9_-]{1,64}`, never changed by a rename.
- Spec values: 8 colours; a choice needs ≥ 2 labels to be pickable; default topic `nidoo/flow_labels`, config key `[topics] flow_labels`; node type `flow.label` with `action: pick|on|off`.
- Run-step detail stays in the hub's existing English detail style (like `enabled b`); the localized FR/EN sentence is built by the web from `LabelSwitchResult` (toast, run history line). *(Plan decision: the hub has no UI language.)*

## Review Focus

1. **Relabelling a flow while it waits in a `delay`**: the run must survive (`same_behaviour` ignores `labels`). Test in Task 1.
2. **Picking with a flow in both labels of the choice**: it is turned on, never off. Test in Task 3.
3. **A `flow.label` step that turns its own flow off**: the run finishes its remaining steps. Test in Task 7.
4. **Deleting a label while a flow save races it**: the saved flow never re-gains the deleted id (lock + re-read). Test in Task 6 (sequential: delete, then `set_flow` with the stale id not previously stored → refused).
5. **Old browser prefs / old retained docs**: `{"grouping":"label"}` decodes; a flow JSON without `labels` and a catalog with unknown keys round-trip unchanged. Tests in Tasks 1, 2 and 9.

---

### Task 1: `Flow.labels` in the model and store

**Files:**
- Modify: `crates/nidoo-core/src/flow/model.rs` (struct `Flow` 26-54, manual `impl Default` 56-70)
- Modify: `Flow { … }` literals without `..Default::default()`: `crates/nidoo-core/src/flow/validate.rs:926`, `crates/nidoo-web/src/flow_math.rs:280`, `crates/nidoo-web/src/if_value.rs:413`, `crates/nidoo-web/src/flow_edit.rs:581`, check `crates/nidoo-web/src/flow_list.rs:454`
- Modify: `crates/nidoo/src/flows/engine.rs:230-238` (`same_behaviour`)
- Modify: `crates/nidoo/src/flows/store.rs` (`Entry::Invalid` 20-28, `invalid()` 121-136, `get()` 155-175)
- Test: `model.rs` tests; `crates/nidoo/tests/flows.rs`

**Interfaces:**
- Produces: `Flow.labels: Vec<String>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`, doc: "Label ids the UI groups the flow under; they do not change what the flow does.")

- [ ] **Step 1: Write failing tests**
  - `model.rs`: `labels_default_empty_and_are_omitted` — `Flow::decode(br#"{"id":"a","name":"A"}"#)` → `labels.is_empty()`, and `encode()` does not contain `"labels"`; with `"labels":["ete","hiver"]` it round-trips.
  - `tests/flows.rs`: `relabelling_keeps_the_run` — copy `moving_to_another_room_keeps_the_run` (~845), but change `labels` instead of `area`; the waiting run is still active afterwards.
  - `tests/flows.rs`: `invalid_flow_keeps_its_labels` — ingest `{"id":"bad","name":"Bad","labels":["ete"],"nodes":"oops"}` → `hub.flow("bad").definition.labels == ["ete"]` and `invalid.is_some()`.
- [ ] **Step 2: Run** `cargo test -p nidoo-core labels_default` and `cargo test -p nidoo --test integration flows::relabelling flows::invalid_flow_keeps` → FAIL (no field).
- [ ] **Step 3: Implement** the field (next to `area`), `labels: Vec::new()` in `Default`, the literals, `labels: Vec::new()` in `same_behaviour`'s literal, and `labels: Vec<String>` on `Entry::Invalid` read from the raw JSON as an array of strings (non-strings dropped).
- [ ] **Step 4: Run** the three tests + `cargo check -p nidoo-web --target wasm32-unknown-unknown` → PASS.
- [ ] **Step 5: Commit** `feat(flows): a flow carries labels`.

### Task 2: Catalog types and the switch planner (`nidoo-core`)

**Files:**
- Create: `crates/nidoo-core/src/flow/labels.rs`; Modify: `crates/nidoo-core/src/flow/mod.rs` (module + re-exports)

**Interfaces (Produces)** — all `Clone, Debug, PartialEq, Serialize, Deserialize`, `#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]`, unknown keys in `#[serde(flatten)] extra: BTreeMap<String, Value>` on the three structs:
- `pub enum LabelColor { Blue, Amber, Green, Red, Purple, Teal, Pink, Slate }` (`snake_case`, `Default = Blue`, `pub const ALL: [LabelColor; 8]` in that order)
- `pub struct FlowLabel { pub id: String, pub name: String, pub color: LabelColor, #[serde(default, skip_serializing_if = "Option::is_none")] pub choice: Option<String>, extra }`
- `pub struct FlowChoice { pub id: String, pub name: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub current: Option<String>, extra }`
- `#[derive(Default)] pub struct FlowLabelCatalog { #[serde(default)] pub labels: Vec<FlowLabel>, #[serde(default)] pub choices: Vec<FlowChoice>, extra }` with `decode(&[u8]) -> Result<Self, String>` (empty → default), `encode(&self) -> String`, `label(&self, id) -> Option<&FlowLabel>`, `choice(&self, id) -> Option<&FlowChoice>`, `members(&self, choice: &str) -> Vec<&FlowLabel>`
- `pub enum LabelAction { Pick, On, Off }` (`snake_case`)
- `pub enum LabelError { UnknownLabel(String), NotInChoice(String), IncompleteChoice(String) }` with `Display`: ``unknown label `x` ``, ``label `x` is no longer in a choice``, ``choice `x` has fewer than two labels``
- `pub struct SwitchPlan { pub enable: Vec<String>, pub disable: Vec<String>, pub skipped_invalid: Vec<String>, pub choice: Option<String> }` (flow ids, sorted)
- `pub fn plan_switch(catalog: &FlowLabelCatalog, flows: &[(&Flow, bool)], label: &str, action: LabelAction) -> Result<SwitchPlan, LabelError>` — the `bool` is "invalid". Only flows whose `enabled` differs from the target appear in `enable`/`disable` (spec "only changes are written"). `Pick`: target on = carries `label`; target off = carries another member of the label's choice and not `label`. `On`/`Off`: every flow carrying `label`. Invalid flows that *would* be switched go to `skipped_invalid`. `choice` = the label's choice for `Pick`.
- `pub fn label_id(name: &str, taken: &[&str]) -> String` — lower-case, fold accents (reuse the core's folding if one exists, else NFD-free table for Latin-1), non-`[a-z0-9]` runs → `-`, trim `-`, cap 64, empty → `"label"`, collisions → `-2`, `-3`…
- `pub fn next_color(catalog: &FlowLabelCatalog) -> LabelColor` — least used, ties in `ALL` order.

- [ ] **Step 1: Write failing tests** in `labels.rs`:
  - `pick_turns_on_the_label_and_off_the_rest_of_the_choice` (Hiver on/Été off flows; pick `ete` → `enable == [ete flows that are off]`, `disable == [hiver flows that are on]`)
  - `pick_keeps_a_flow_carrying_both_labels_on` (Review Focus 2)
  - `flows_already_right_are_not_listed` and `untouched_flows_without_the_choice`
  - `pick_errors`: unknown → `UnknownLabel`; standalone label → `NotInChoice`; choice with 1 member → `IncompleteChoice`
  - `on_off_work_for_any_label` and `invalid_flows_are_skipped`
  - `label_id_folds_and_suffixes`: `("Été", [])→"ete"`, `("Été", ["ete"])→"ete-2"`, `("  ", [])→"label"`, `("Mode Télétravail!", [])→"mode-teletravail"`
  - `next_color_is_least_used`
  - `catalog_round_trips_unknown_keys` (Review Focus 5) and `empty_payload_is_an_empty_catalog`
- [ ] **Step 2: Run** `cargo test -p nidoo-core flow::labels` → FAIL.
- [ ] **Step 3: Implement** the types and functions above.
- [ ] **Step 4: Run** → PASS; `cargo check -p nidoo-core --target wasm32-unknown-unknown`.
- [ ] **Step 5: Commit** `feat(flows): label catalog and switch planner`.

### Task 3: The `flow.label` node kind (model + every match)

**Files:**
- Modify: `crates/nidoo-core/src/flow/model.rs` (`NodeKind` 134-192, `type_name` 195-220), `flow/mod.rs:29` (re-export), `flow/validate.rs` (per-node match ~180-187)
- Modify (exhaustive matches, minimal arms): `crates/nidoo/src/flows/hub.rs` no-entity arm list (501-510), `crates/nidoo/src/flows/engine.rs:1347` (temporary `failed("flow.label not supported yet")` — replaced in Task 7), `crates/nidoo-web/src/flow_text.rs` (`Role::Action` ~49; `sentence` ~409 uses the label id until Task 12), `crates/nidoo-web/src/flows.rs` `summary_label` (~81), `crates/nidoo-web/src/flow_edit.rs` `next_node_id` (~407, prefix `"flow_label"`), `crates/nidoo-web/src/icons.rs` `node_icon` (~351, reuse `NODE_SWITCH` or add a tag icon), `crates/nidoo-web/src/screens/flow_canvas/settings.rs` kind label (356) and form dispatch (1735, empty view until Task 12)
- Modify: `crates/nidoo-web/src/i18n/flows.rs` — add `NodeFlowLabel` ("Pick a label", "Choisir une étiquette") to the enum, `ALL`, and the match

**Interfaces:**
- Produces: `#[serde(rename = "flow.label")] NodeKind::FlowLabel(FlowLabelParams)`; `pub struct FlowLabelParams { pub label: String, pub action: LabelAction, extra }` (doc-comments become the MCP schema)

- [ ] **Step 1: Failing tests**: `model.rs` `flow_label_round_trips` (`{"type":"flow.label","params":{"label":"ete","action":"pick"}}` ↔ `FlowLabel(FlowLabelParams{label:"ete",action:Pick,..})`, `type_name() == "flow.label"`); `validate.rs` `flow_label_needs_a_label` (empty `label` → error naming the node); `flow_edit.rs` `next_id_prefixes` gains `flow_label`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** the variant and every arm listed.
- [ ] **Step 4: Run** `cargo test -p nidoo-core`, `cargo test -p nidoo-web`, `cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 5: Commit** `feat(flows): the flow.label node kind`.

### Task 4: Config key `[topics] flow_labels`

**Files:** Modify `crates/nidoo/src/config.rs` (`TopicsConfig` 74-92, `STRING_ENV_KEYS` ~803-830, `validate_all` 859-867); Test in the same file.

**Interfaces:** Produces `TopicsConfig.flow_labels: String` (default `"nidoo/flow_labels"`), `TopicsConfig::validate(&self) -> Result<(), String>` called from `validate_all`.

- [ ] **Step 1: Failing tests** `mod topics_config_tests`: `flow_labels_defaults` (value + `validate() == Ok(())`), `flow_labels_from_toml` (`[topics]\nflow_labels = "home/labels"`), `flow_labels_rejects` — for `["", "/", "nidoo/+/x", "nidoo/#", "nidoo/automation", "nidoo/automation/labels"]` the error contains `flow_labels` (the last two: equal to or under `automation_prefix`).
- [ ] **Step 2: Run** `cargo test -p nidoo --lib topics_config` → FAIL.
- [ ] **Step 3: Implement**; add `"topics.flow_labels"` to `STRING_ENV_KEYS`.
- [ ] **Step 4: Run** → PASS. **Step 5: Commit** `feat(config): the flow labels topic`.

### Task 5: Hub catalog store, ingest, edits

**Files:**
- Create: `crates/nidoo/src/flows/labels.rs` (hub methods), `crates/nidoo-api/src/flow_labels.rs` (wire types), `crates/nidoo/tests/flow_labels.rs` (+ `mod flow_labels;` in `tests/integration.rs`)
- Modify: `crates/nidoo/src/hub.rs` (fields beside `flow_store` ~234 / `automation_prefix` ~298, init ~461, early intercept in `ingest` ~578 next to `_home`, `extra_filters` 1110-1135), `crates/nidoo-core/src/registry/mod.rs:42-52` (`flow_labels` reserved), `crates/nidoo-api/src/events.rs` (variant beside `HomeSettingsChanged` 87), `crates/nidoo-api/tests/snapshot.rs`

**Interfaces:**
- Hub fields: `flow_labels: Mutex<FlowLabelCatalog>`, `flow_labels_topic: String`, `pub(crate) labels_pass: tokio::sync::Mutex<()>` (pattern: `alarm/mod.rs:30`).
- `nidoo-api`: `pub struct FlowLabelsView { pub catalog: FlowLabelCatalog, pub counts: BTreeMap<String, usize> }`; `#[serde(tag = "op", rename_all = "snake_case")] pub enum LabelEdit { CreateLabel { name: String, choice: Option<String> }, CreateChoice { name: String }, RenameLabel { id, name }, RecolorLabel { id, color: LabelColor }, MoveLabel { id, choice: Option<String> }, RenameChoice { id, name }, DeleteLabel { id }, DeleteChoice { id } }`; `WsEvent::FlowLabelsChanged { labels: FlowLabelsView }`.
- Hub: `pub(crate) fn ingest_flow_labels(&self, payload: &[u8])` (copy `ingest_home`, `home.rs:244-264`: empty → default, undecodable → warn + keep, change → broadcast); `pub fn flow_labels(&self) -> FlowLabelsView` (counts over valid and invalid flows' `labels`); `pub async fn edit_flow_labels(&self, edit: LabelEdit) -> Result<FlowLabelsView, HubError>`.
- `edit_flow_labels` rules (spec "Deleting" and "Catalog edits and current"): takes `labels_pass`; offline → `HubError::Publish("broker offline")` before any write; names trimmed and non-empty (`Rejected`); ids from `label_id`, colour from `next_color`; unknown id → `Rejected("unknown label `x`")`; `MoveLabel` out of a choice clears that choice's `current` if it was this label; `DeleteLabel` removes the id from every flow carrying it (one `write_flow` each, valid flows only) then the catalog; `DeleteChoice` keeps its labels as standalone. Catalog written with `publish(retain)` + `ingest_flow_labels`.
- `extra_filters`: add the topic only when it is not covered by a configured `<discovery prefix>/#` (avoids double delivery, `mqtt.rs:290-293`).

- [ ] **Step 1: Failing tests** (`tests/flow_labels.rs`, hub built like `flows.rs:30-37`): `create_label_publishes_the_catalog_retained` (topic `nidoo/flow_labels`, `retain`, id `ete`, colour `blue` then next `amber`); `ingest_broadcasts_flow_labels_changed`; `undecodable_catalog_keeps_the_previous`; `create_in_choice_and_counts`; `move_current_label_out_clears_current`; `delete_label_strips_it_from_flows` (two flows carrying it are republished without it; a third is not republished); `delete_choice_keeps_its_labels`; `edits_refused_offline` (`set_connection(Connecting)` → `HubError::Publish`, nothing published); `catalog_topic_is_not_discovery` (ingesting the topic creates no entity). Snapshot test for the new api types.
- [ ] **Step 2: Run** `cargo test -p nidoo --test integration flow_labels` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS (+ `cargo test -p nidoo-api`).
- [ ] **Step 5: Commit** `feat(hub): the flow label catalog`.

### Task 6: Labels on flow writes; `flow.label` save checks

**Files:** Modify `crates/nidoo/src/flows/hub.rs` (`set_flow` 110-120, new `set_flow_labels` beside `set_flow_area` 153-168, `check_house` FlowEnable branch 488-499 → add FlowLabel branch), `crates/nidoo/src/flows/labels.rs`; Test `crates/nidoo/tests/flow_labels.rs`.

**Interfaces:**
- `pub async fn set_flow_labels(&self, id: &str, labels: Vec<String>) -> Result<FlowView, HubError>` (skips the entity checks like `set_flow_area`; refuses invalid flows).
- `fn check_labels(&self, flow: &mut Flow) -> Result<(), HubError>` — sort + dedupe; each id must be in the catalog or already stored on that flow (copy `check_area` 371-394), else `InvalidFlow("unknown label `x`")`.
- `check_house` `FlowLabel` branch: label exists, and for `Pick` it is in a choice — unless an identical node (same id and params) is already stored on that flow. Error text = `LabelError` Display + `` in node `n` ``.
- `set_flow` and `set_flow_labels` take `labels_pass` and re-read the stored flow inside it.

- [ ] **Step 1: Failing tests**: `set_flow_labels_sets_sorts_and_dedupes`; `unknown_label_is_rejected`; `stale_label_is_kept_on_resave` (label deleted from the catalog directly via ingest, flow re-saved unchanged → OK); `deleted_label_cannot_be_added_back` (Review Focus 4); `set_flow_rejects_flow_label_unknown_or_standalone_pick`; `broken_flow_label_node_still_saves_when_unchanged`.
- [ ] **Step 2–4:** run (FAIL) → implement → run (PASS).
- [ ] **Step 5: Commit** `feat(hub): labels on flows and flow.label save checks`.

### Task 7: Switching labels; the engine step

**Files:** Modify `crates/nidoo/src/flows/labels.rs`, `crates/nidoo/src/flows/engine.rs` (`run_node` FlowLabel arm, sibling of 1347-1368), `crates/nidoo-api/src/flow_labels.rs`; Test `crates/nidoo/tests/flow_labels.rs`.

**Interfaces:**
- `nidoo-api`: `pub struct SkippedFlow { pub id: String, pub name: String, pub reason: String }`; `pub struct LabelSwitchResult { pub label: String, pub action: LabelAction, pub enabled: Vec<String>, pub disabled: Vec<String>, pub skipped: Vec<SkippedFlow> }` (flow ids).
- Hub: `pub async fn switch_flow_label(&self, label: &str, action: LabelAction, by_run: Option<(&str, &str)>) -> Result<LabelSwitchResult, HubError>` (`by_run` = (flow id, run id)). Takes `labels_pass`; offline → `Publish("broker offline")` up front; `plan_switch` (`LabelError` → `Rejected(err.to_string())`); applies each change with `set_flow_enabled` — or `set_flow_enabled_by_run` for the run's own flow; a failed write → `skipped` with the error; invalid flows → `skipped` with reason `invalid`; for `Pick` sets the choice's `current` and writes the catalog even if some flows failed.
- Engine arm: `hub.switch_flow_label(&p.label, p.action, Some((flow_id, run_id)))`; Ok → `(StepOutcome::Ok, detail, vec![])` with detail `"picked ete: 2 on, 1 off"` / `"on vacances: 3 on"` / `"off vacances: 3 off"` / `"picked ete: nothing to change"`, plus `", 1 skipped (name: reason)"` when any; Err → `failed(err.to_string())`.

- [ ] **Step 1: Failing tests**: `pick_switches_only_what_changes` (asserts publishes are exactly the changed flows; `current` updated); `pick_again_reapplies_after_hand_changes`; `on_off_standalone_label`; `switch_refused_offline`; `invalid_flows_reported_as_skipped`; `incomplete_choice_is_refused`; engine: `flow_label_step_picks_and_reports` (detail `"picked ete: 1 on, 1 off"`), `flow_label_self_disable_keeps_the_run` (copy `self_disable_keeps_the_running_run` ~1152; Review Focus 3), `flow_label_unknown_label_fails_the_step`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(flows): switch a label from the hub and from a flow`.

### Task 8: HTTP, MCP and reference docs

**Files:** Modify `crates/nidoo/src/http/api.rs` (routes 60-69, handlers like `set_flow_area` 757-770), `crates/nidoo/src/mcp/flow_tools.rs`, `crates/nidoo/src/mcp/params.rs` (`SetFlowParams` doc: node list "seventeen" + `flow.label`; `FlowSummary` + `labels`), `crates/nidoo/src/mcp/mod.rs:143-162` (instructions); tests `crates/nidoo/tests/http.rs`, `crates/nidoo/tests/mcp.rs` (`EXPECTED_TOOLS` 33-90), `crates/nidoo/tests/mcp_stdio.rs:26-47`; docs `docs/reference/http-api.md` (§Flows 135), `mcp-tools.md` (§Flows 48, line 62, count line 168), `flow-nodes.md` (`labels` row ~35, refusal list ~52-62, new `## flow.label` after `## flow.enable` 444), `mqtt-topics.md` (row after line 22, reserved list 67).

**Interfaces (routes under `/api/v1`):**
- `GET /flow-labels` → `FlowLabelsView`; `POST /flow-labels` body `LabelEdit` → `FlowLabelsView`
- `POST /flow-labels/{id}/switch` body `{"action":"pick"|"on"|"off"}` → `LabelSwitchResult`
- `POST /flows/{id}/labels` body `{"labels":[…]}` (key required) → `FlowView`
- MCP: `list_flow_labels` (read-only) → `FlowLabelsView`; `edit_flow_labels` (`LabelEdit`) → `FlowLabelsView`; `switch_flow_label` (`{label, action}`) → `LabelSwitchResult`. Tagging a flow over MCP goes through `set_flow` (its `labels` field), so there is no per-flow labels tool. *(Plan decision: the spec's `set_flow_labels` MCP name is `edit_flow_labels` here, so it does not clash with the hub's per-flow `set_flow_labels`.)* Instructions sentence: "A flow's `labels` group it for the user; picking a label in a choice (`switch_flow_label`) turns its flows on and the choice's other labels' flows off."

- [ ] **Step 1: Failing tests**: http `flow_labels_edit_list_and_switch` (create choice + 2 labels, tag flows via `/flows/{id}/labels`, switch `pick`, assert body counts; offline → 503; unknown label → 422); mcp `switch_flow_label_over_mcp`, tool lists updated, instructions contain `switch_flow_label`, `list_flows` rows carry `labels`.
- [ ] **Step 2–4:** FAIL → implement + docs → PASS.
- [ ] **Step 5: Commit** `feat(api): flow labels over HTTP and MCP`.

### Task 9: Web data: client, house state, i18n, grouping logic

**Files:** Create `crates/nidoo-web/src/api/flow_labels.rs`; Modify `crates/nidoo-web/src/api/mod.rs`, `crates/nidoo-web/src/house.rs` (signal beside `home_settings` 35, WS apply ~262, refresh like 152), `crates/nidoo-web/src/flow_list.rs` (`Grouping` 56-63, `ListRow` 74-86, `rows()`, `matches_query` ~195, `row_key` ~301, sections), `crates/nidoo-web/src/i18n/flows.rs`.

**Interfaces:**
- Client: `get_flow_labels() -> Result<FlowLabelsView, ApiError>`, `edit_flow_labels(&LabelEdit)`, `switch_flow_label(label: &str, action: LabelAction) -> Result<LabelSwitchResult, ApiError>`, `set_flow_labels(id: &str, labels: &[String]) -> Result<FlowView, ApiError>` (copy `set_flow_area`, `api/core.rs:162-167`).
- `House.flow_labels: RwSignal<FlowLabelsView>`, refreshed on load and on `WsEvent::FlowLabelsChanged`.
- `Grouping::Label` (serde `"label"`); `ListRow.labels: Vec<FlowLabel>` (resolved, catalog order; unknown ids dropped); `rows(…, catalog: &FlowLabelCatalog)`.
- `pub enum SectionHead { Plain, Choice { choice: FlowChoice, members: Vec<FlowLabel> }, Label(FlowLabel), NoLabel }` and `Section.head: SectionHead` (room sections use `Plain`).
- `pub fn label_sections(lang, rows: Vec<ListRow>, catalog: &FlowLabelCatalog) -> Vec<Section>` — choices (catalog order; rows carrying any member), then standalone labels, then "Sans étiquette"/"No label"; a row appears in each matching section; empty choice and label sections are **kept** (their header still offers the switch), empty "No label" dropped. Keys: `choice:<id>`, `label:<id>`, `__no_label__`.
- `matches_query` also matches label names; `row_key` hashes label ids + colours.
- `FlowText` additions (EN, FR): `GroupLabel` (Label, Étiquette), `NoLabel` (No label, Sans étiquette), `Labels` (Labels, Étiquettes), `AllOn` (Turn all on, Tout activer), `AllOff` (Turn all off, Tout désactiver), `NewLabel` (New label…, Nouvelle étiquette…), `NewChoice` (Choice, Choix), `AddLabelInChoice` (Label in this choice, Étiquette dans ce choix), `TriggerMenu` (Trigger, Déclencheur), `ChoiceIncomplete` (Needs two labels, Il faut deux étiquettes), `LabelPick`/`LabelOn`/`LabelOff` (Pick, Choisir / Turn on, Activer / Turn off, Désactiver), `DeleteLabel`… as needed. Sentences: `pub fn label_switch_result(lang, r: &LabelSwitchResult, catalog) -> String` → FR « Été choisi : 5 activées, 12 désactivées. », « Vacances : 3 désactivées. », « Été choisi : rien à changer. », + « 1 non modifiée : Nom (invalide). »; EN "Été picked: 5 on, 12 off." etc. `pub fn label_preview(lang, on: usize, label: &str, off: usize, others: &[&str]) -> String` for the card.

- [ ] **Step 1: Failing native tests** (`flow_list.rs`, `i18n/flows.rs`): `grouping_label_decodes` (`{"grouping":"label"}` → `Label`; Review Focus 5), `label_sections_order_and_duplicates`, `empty_choice_section_is_kept`, `query_matches_label_names`, `row_key_changes_with_labels`, `label_switch_result_sentences` (the exact FR/EN strings above), table test passes.
- [ ] **Step 2–4:** FAIL → implement → PASS (`cargo test -p nidoo-web`, wasm check).
- [ ] **Step 5: Commit** `feat(web): flow label data and grouping`.

### Task 10: Flows page: B2 top bar, label sections, row chips

**Files:** Modify `crates/nidoo-web/src/screens/flows/bar.rs` (`Segmented` options 120-126; replace `trigger_chips` 87-109/138-143 with a `TriggerMenu` popover on the status row), `crates/nidoo-web/src/screens/flows/mod.rs` (`section_view` 107-172 headers; `FlowsScreen` grouping match ~246-252), `row.rs` (chips after the badge in `flow-row__name`), `card.rs` (chips in the tile), the page stylesheet; Test `crates/nidoo-web/tests/flows_list.rs`.

**Interfaces:**
- `#[component] fn TriggerMenu(prefs: RwSignal<FlowListPrefs>)` — button "Déclencheur ▾" (`· n` when n kinds selected), popover with the 7 kinds as `menuitemcheckbox`, same toggling of `prefs.triggers`; closes on Escape / outside (reuse `OpenMenu`, `room_menu.rs:24-50`).
- Choice header: `Segmented`-style picker of members, current highlighted; click → `api::switch_flow_label(id, Pick)` → `toasts.push(label_switch_result(..))`; incomplete choice shows `ChoiceIncomplete` instead of a picker. Label header: `AllOn` / `AllOff` buttons (`On`/`Off`). Every header: ⚙ opens the manager (Task 11) on that item; "No label" header: "＋ Étiquette".
- Chip: `<span class="label-chip label-chip--{color}">name</span>`; 8 colour classes with light and dark values.

- [ ] **Step 1: Failing browser tests** (`flows_list.rs`, `FetchStub`): `grouping_offers_label`; `label_sections_show_picker_and_switch` (click Été → POST `/api/v1/flow-labels/ete/switch` body `{"action":"pick"}`, toast shows the stubbed result sentence); `standalone_label_all_off`; `trigger_menu_replaces_the_chips` (no `.flow-chips__set[aria-label]` trigger set; menu filters like before); `rows_show_label_chips`.
- [ ] **Step 2–4:** FAIL → implement → PASS (`wasm-pack test --headless --chrome -- --test flows_list`).
- [ ] **Step 5: Commit** `feat(web): group flows by label and switch from the list`.

### Task 11: Label manager, row "Étiquettes…" menu, editor field

**Files:** Create `crates/nidoo-web/src/screens/flows/label_manager.rs`; Modify `crates/nidoo-web/src/screens/flows/room_menu.rs` (checklist section after "Move to room" ~201-207; `menu_for` in `mod.rs:59-74` passes labels), `crates/nidoo-web/src/screens/flow_canvas/toolbar.rs` (labels field beside the room select 349-380; drop stale ids next to `clear_stale_area` at 75); Test `crates/nidoo-web/tests/flows_list.rs`, `crates/nidoo-web/tests/flow_canvas.rs`.

**Interfaces:**
- `#[component] pub fn LabelManager(focus: Option<String>, on_close: Callback<()>)` — dialog per `2026-11-10-flow-labels-manage-mockup.html` panel 1; every action calls `api::edit_flow_labels(&LabelEdit::…)`; delete confirms first, and when any flow's `flow.label` node names the label the prompt lists those flow names.
- Row menu: `menuitemcheckbox` per label (choice name in grey), toggling calls `api::set_flow_labels`; "New label…" creates (`CreateLabel`) then tags. Disabled for invalid flows (like "Move to room").
- Toolbar: chips with ✕ + an "add" combobox (existing labels; typing a new name creates it on Enter); edits the draft (`editor.draft.update(|f| f.labels = …)` with history recorded as `pick_room` does, 282-292).

- [ ] **Step 1: Failing browser tests**: `manager_creates_choice_and_labels`; `manager_delete_names_flows_using_it`; `row_menu_tags_a_flow` (POST `/flows/{id}/labels`); `editor_labels_field_saves_with_the_flow` (`flow_canvas.rs`; saved PUT body has `"labels":["ete"]`); `editor_drops_stale_labels_on_save`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(web): manage labels and tag flows`.

### Task 12: The "Choisir une étiquette" card

**Files:** Modify `crates/nidoo-web/src/flow_picker.rs` (`generic` list 249-268, `Group::Flow`; picker test ~548), `crates/nidoo-web/src/screens/flow_canvas/settings.rs` (new `fn flow_label_fields(form: Form, id: String, p: FlowLabelParams) -> impl IntoView` beside `flow_enable_fields` 441; dispatch 1735), `crates/nidoo-web/src/flow_text.rs` (`Ctx` gains `labels: &FlowLabelCatalog`; `sentence` arm → « Choisir une étiquette : Saison → Été » / « Activer « Vacances » » / « Désactiver « Vacances » »); Test `crates/nidoo-web/tests/flow_canvas.rs`, `flow_text.rs` tests.

**Interfaces:** library entry `(Group::Flow, NodeKind::FlowLabel(FlowLabelParams { label: String::new(), action: LabelAction::Pick, .. }), Some(FlowText::NodeFlowLabel), FlowText::NodeFlowLabelSub)`. Card fields: mode select (Choisir / Activer / Désactiver); for Choisir a **Choix** select then **Étiquette** among its members; otherwise **Étiquette** among all labels; live preview via `label_preview` computed with `plan_switch` over `house.flows`.

- [ ] **Step 1: Failing tests**: `flow_label_sentence` (native, the three strings); `library_lists_pick_a_label`; browser `flow_label_card_saves_pick` (choose Saison → Été; saved node `{"type":"flow.label","params":{"label":"ete","action":"pick"}}`; preview text shown).
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(web): the pick-a-label card`.

### Task 13: User docs and backlog

**Files:** Modify `docs/users/flows.md` (new `## Labels and choices` after `## Rooms` 273 with the winter/summer example and a `0 0 21 6 *` switching flow; update `## The Flows page` 277-293 for B2 and label grouping; node list line ~53 gains `flow.label`), `TODOS.md` (replace "Bulk actions on the Flows page" with a done note; add follow-ups: switch a label by voice, labels in the Android app).

- [ ] **Step 1:** Write the docs (plain short sentences, generic names).
- [ ] **Step 2:** Run the whole gate: fmt, clippy, `cargo test --workspace` (needs `TEST_DATABASE_URL` as in CI, else run the crates above), the three touched web browser test files.
- [ ] **Step 3: Commit** `docs: flow labels and choices`.
