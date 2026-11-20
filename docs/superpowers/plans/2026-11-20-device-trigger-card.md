# Device Trigger Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A visible "Appareil / Device" trigger in the flow editor's library, whose card picks any entity (values or signals) and offers a list of what it reports instead of raw JSON.

**Architecture:** Web editor only (`crates/nidoo-web`). The card keeps saving as the existing `trigger.state` / `trigger.signal`; picking a signal entity converts the node between the two. The "Fires when" / "Only after" JSON boxes become the If card's value list (`IfValueField`) with an "any" first option. No engine, flow JSON or MCP change.

**Tech Stack:** Rust, Leptos (wasm), native unit tests in the `nidoo-web` lib.

**Spec:** `docs/superpowers/specs/2026-11-20-device-trigger-card-design.md`

## Global Constraints

- No change outside `crates/nidoo-web` (plus the spec file). `nidoo-core` flow model, engine, store and MCP schema stay untouched.
- Every new UI string is a `FlowText` variant with English and French text, added to `FlowText::ALL` (the `assert_table` test enforces it).
- Exact copy: card title `("Device", "Appareil")` (replaces `NodeTrigger`'s `("Trigger", "Déclencheur")`); sub-line `("When a device changes or sends an event", "Quand un appareil change ou envoie un événement")`; first list option for `to` `("Any change", "N'importe quel changement")`; for `from` `("Any value", "N'importe quelle valeur")`.
- A saved `to` / `from` that the list does not hold must stay shown and saved (`with_current` already does this for If).
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Tests: `cargo test -p nidoo-web --lib <filter>` (native). Do not run `wasm-pack test` (links every test wasm, fills the disk). Compile check for the browser: `cargo check -p nidoo-web --target wasm32-unknown-unknown`.

## Decisions taken while planning (differ from or sharpen the spec)

- **Search value entries only for binary sensors** (`Kind::BinarySensor`: door, window, motion, leak, smoke…). Lights, covers, climates are reachable through the card's list; adding their values to search would flood room searches ("salon" matches every entity in the room). Task 2 updates the spec line.
- **Wording reuses the existing sentences**: a `to` already reads "Quand Porte devient ouvert" / "When Door turns on" (`flow_text::sentence`). No new "passe à" phrase, and no separate canvas-summary work: the card already shows that sentence. Task 2 updates the spec.
- **Changing a signal card's entity resets its signal to "any"** (today it keeps a stale signal of the old remote).

## Review Focus

1. A flow saved before this change with `to: "ON"` on an entity whose list holds `true`/`false` → the card shows "ON" selected and saving keeps `"ON"` (test in Task 4).
2. Picking a remote on a state card that already has edges → the node becomes `trigger.signal`, keeping id, position and edges (test in Task 3).
3. Clearing the entity of a Device card → it stays the same node type with an empty entity, not a panic or a type flip (test in Task 3).
4. A freshly added Device card with no entity reads a sentence, not "Quand  change" (test in Task 1).
5. Number entity (temperature) on the card → number box; emptying it means "any change" (`to: None`) (test in Task 4).

---

### Task 1: Device card in the library

**Files:**
- Modify: `crates/nidoo-web/src/i18n/flows.rs` (enum near line 184, `ALL` near 348, texts near 918)
- Modify: `crates/nidoo-web/src/flow_picker.rs` (`generic` array at line ~93, `card_search_words` line 78, tests)
- Modify: `crates/nidoo-web/src/flow_text.rs:307` (`sentence`, `NodeKind::TriggerState` arm)

**Interfaces:**
- Produces: `FlowText::DeviceSub`, `FlowText::AnyChange`, `FlowText::AnyValue` (used by Task 4); `NodeTrigger` now reads "Device" / "Appareil".

- [ ] **Step 1: Write the failing tests** in `flow_picker.rs` tests and `flow_text.rs` tests

```rust
// flow_picker.rs — update the existing expected list: Device comes first.
#[test]
fn empty_query_has_generic_entries() {
    // … expected list now starts with:
    //   (Group::When, "Device"), (Group::When, "Schedule"), …
}

#[test]
fn device_card_is_an_empty_state_trigger_found_in_either_language() {
    let entities = house(vec![]);
    let got = run("", &entities);
    let first = &got[0];
    assert_eq!(first.group, Group::When);
    assert_eq!(first.sub.as_deref(), Some("When a device changes or sends an event"));
    assert!(matches!(&first.kind, NodeKind::TriggerState(p) if p.entity.is_empty() && p.to.is_none()));
    for q in ["device", "appareil", "event", "evenement"] {
        assert!(run(q, &entities).iter().any(|e| matches!(&e.kind, NodeKind::TriggerState(p) if p.entity.is_empty())), "{q}");
    }
}

// flow_text.rs
#[test]
fn state_trigger_without_entity_reads_the_device_sentence() {
    // node TriggerState(TriggerStateParams::default()), lang En
    assert_eq!(sentence(&node, &ctx), "When a device changes or sends an event");
}
```

- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib device_card` and `--lib state_trigger_without_entity` — Expected: FAIL.

- [ ] **Step 3: Implement**
  - `FlowText`: add `DeviceSub`, `AnyChange`, `AnyValue` (texts from Global Constraints) to the enum and `ALL`; change `NodeTrigger` to `("Device", "Appareil")`.
  - `flow_picker::entries`: first element of `generic` is `(Group::When, NodeKind::TriggerState(TriggerStateParams::default()), Some(FlowText::NodeTrigger), FlowText::DeviceSub)`.
  - `card_search_words`: `NodeKind::TriggerState(_) => "trigger declencheur event evenement sensor capteur"`. Check how the generic match folds accents; keep words accent-free like the existing ones.
  - `sentence`: in the `TriggerState` arm, an empty `p.entity` returns `flow_text(lang, FlowText::DeviceSub)`.

- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib` — Expected: PASS. Fix other tests that asserted the old "Trigger" text or the generic list order, if any.

- [ ] **Step 5: Commit** — `feat(web): a Device trigger card in the flow library`

---

### Task 2: Door / motion value entries in search

**Files:**
- Modify: `crates/nidoo-web/src/flow_picker.rs` (entity loop at line ~345, tests)
- Modify: `docs/superpowers/specs/2026-11-20-device-trigger-card-design.md` ("Search entries" and "Card summary" paragraphs, per "Decisions taken while planning")

**Interfaces:**
- Consumes: `crate::if_value::{value_input, ValueInput, Choice}` (`value_input(lang, Some(&e)) -> ValueInput`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn door_search_offers_one_when_entry_per_value() {
    let mut door = ent("d1", "Front door", Kind::BinarySensor);
    door.device_class = Some("door".into());
    let got = run("front", &house(vec![door]));
    let when: Vec<(String, Option<Value>)> = got.iter()
        .filter(|e| e.group == Group::When)
        .filter_map(|e| match &e.kind {
            NodeKind::TriggerState(p) => Some((e.label.clone(), p.to.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(when[0], ("When Front door changes".into(), None));
    assert_eq!(when[1].1, Some(json!(true)));
    assert_eq!(when[2].1, Some(json!(false)));
    assert_eq!(when.len(), 3);
}
```

Existing tests `table_query_lists_light_entries` (When count 1 for a light) and `accent_insensitive` must keep passing unchanged: they show that only binary sensors get the extra entries.

- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib door_search` — Expected: FAIL (`when.len()` is 1).

- [ ] **Step 3: Implement** right after the existing "changes" `TriggerState` push: when `e.kind == Kind::BinarySensor` and `value_input(ctx.lang, Some(e))` is `ValueInput::List(list)`, push one `Group::When` entry per choice: `TriggerState { entity: e.id.clone(), to: Some(choice.value), ..Default::default() }`. Labels come from `entry()` → `sentence`; do not build them by hand.

- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib flow_picker` — Expected: PASS.

- [ ] **Step 5: Edit the spec** (two paragraphs named above) and **commit** — `feat(web): search offers "door opens / closes" triggers`

---

### Task 3: One entity pool; state ↔ signal conversion

**Files:**
- Modify: `crates/nidoo-web/src/flow_edit.rs` (new pure fn + tests)
- Modify: `crates/nidoo-web/src/screens/flow_canvas/settings.rs` (`NodeEditor` line ~261; `TriggerState` arm line ~1329; `TriggerSignal` arm line ~1520)

**Interfaces:**
- Produces: `pub fn retarget_trigger(kind: &NodeKind, entity: String, signal: bool) -> NodeKind` in `flow_edit.rs`.
  - `signal` = whether the picked entity is a signal source (`entity_picker::needs::is_signal`).
  - Empty `entity`: same type, entity cleared, other params as for a changed entity.
  - Same entity and same type: returns `kind.clone()`.
  - State → state with another entity: `field`, `to`, `from` reset to `None`, `extra` kept.
  - Signal → signal with another entity: `signal` reset to `None`, `extra` kept.
  - State → signal: `TriggerSignal { entity, signal: None, extra }`; signal → state: `TriggerState { entity, ..Default::default(), extra }`.
  - Any other `kind`: returned unchanged.

- [ ] **Step 1: Write the failing tests** in `flow_edit.rs` tests

```rust
#[test]
fn picking_a_remote_turns_a_state_trigger_into_a_signal_trigger() {
    let k = NodeKind::TriggerState(TriggerStateParams { entity: "door".into(), to: Some(json!(true)), ..Default::default() });
    assert!(matches!(retarget_trigger(&k, "remote".into(), true),
        NodeKind::TriggerSignal(p) if p.entity == "remote" && p.signal.is_none()));
}

#[test]
fn picking_a_sensor_turns_a_signal_trigger_back_into_a_state_trigger() { /* Signal{remote, Some("on")} + ("door", false) → State{door, to None} */ }

#[test]
fn another_entity_of_the_same_kind_resets_the_filters() { /* State{door,to true,field Some("battery")} + ("window", false) → State{window, field None, to None, from None} ; Signal{r1, Some("on")} + ("r2", true) → Signal{r2, None} */ }

#[test]
fn clearing_the_entity_keeps_the_card_type() { /* Signal{r1} + ("", false) → Signal{""}; State{door} + ("", true) → State{""} */ }

#[test]
fn retargeting_keeps_id_position_and_edges() {
    // a Flow with node "trigger_1" (State, position Some) wired to "notify_1";
    // replace its kind with retarget_trigger(.., "remote", true) via the same path settings uses
    // (form.update → n.kind = …): node id, position and the edge are unchanged.
}
```

- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib retarget` and the four names above — Expected: FAIL (fn missing).

- [ ] **Step 3: Implement `retarget_trigger`**, then wire `settings.rs`:
  - Both arms: `EntityPickField` `pool=None` (every entity), and `entity_write` becomes: look up the picked id in `expect_context::<House>().entities` and call `n.kind = retarget_trigger(&n.kind, id, entity.is_some_and(is_signal))`.
  - `NodeEditor`: re-mount the body when the selected node's type changes. Add a `Memo` of `draft.with(|f| f.node(&id).map(|n| n.kind.type_name()))` and `.track()` it next to `editor.restored.track()`. Without this, the old arm's fields stay on screen after a conversion.

- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib` and `cargo check -p nidoo-web --target wasm32-unknown-unknown` — Expected: PASS / no errors.

- [ ] **Step 5: Commit** — `feat(web): the Device card picks any entity and becomes a signal trigger for remotes`

---

### Task 4: "Quand" value lists replace the JSON boxes

**Files:**
- Modify: `crates/nidoo-web/src/if_value.rs` (new pure fn + tests)
- Modify: `crates/nidoo-web/src/screens/flow_canvas/value_field.rs` (`IfValueField` props and pick handling)
- Modify: `crates/nidoo-web/src/screens/flow_canvas/settings.rs` (`TriggerState` arm: replace both `JsonField`s)

**Interfaces:**
- Consumes: `FlowText::AnyChange`, `FlowText::AnyValue`, `FlowText::FiresWhen`, `FlowText::OnlyAfter` (Task 1).
- Produces in `if_value.rs`:
  ```rust
  /// What a pick in the value list means.
  #[derive(Clone, Debug, PartialEq)]
  pub enum ListPick { Any, Other, Value(Value) }
  /// `picked` is the `<option>` value: "" → Any (only when `any` is offered), "other" → Other,
  /// an index into `choices` → that choice's value; anything else → None.
  pub fn list_pick(picked: &str, choices: &[Choice], any: bool) -> Option<ListPick>
  ```
- Produces on `IfValueField`, all optional so the If card is unchanged:
  `label: Option<FlowText>` (default `FlowText::ParamValue`), `any: Option<FlowText>` (text of the first "no filter" option), `clear: Option<Callback<()>>` (called for `ListPick::Any`, and for a blank number or typed box when `any` is set).

- [ ] **Step 1: Write the failing tests** in `if_value.rs` tests

```rust
#[test]
fn list_pick_reads_any_other_and_indexes() {
    let choices = vec![Choice { value: json!(true), label: "Open".into() }, Choice { value: json!(false), label: "Closed".into() }];
    assert_eq!(list_pick("", &choices, true), Some(ListPick::Any));
    assert_eq!(list_pick("", &choices, false), None);
    assert_eq!(list_pick("other", &choices, true), Some(ListPick::Other));
    assert_eq!(list_pick("1", &choices, true), Some(ListPick::Value(json!(false))));
    assert_eq!(list_pick("9", &choices, true), None);
}

#[test]
fn a_saved_unlisted_trigger_value_stays_in_the_list() {
    let mut door = ent(Kind::BinarySensor);
    door.device_class = Some("door".into());
    let l = list(with_current(value_input(Lang::En, Some(&door)), Some(&json!("ON")), Lang::En, Some(&door)));
    assert_eq!(l[0].value, json!("ON"));
}
```

- [ ] **Step 2: Run** `cargo test -p nidoo-web --lib list_pick` — Expected: FAIL (fn missing).

- [ ] **Step 3: Implement**
  - `list_pick` in `if_value.rs`.
  - `IfValueField`: the list's `on:change` goes through `list_pick`. When `any` is set, the disabled "Choose…" placeholder is replaced by an enabled `<option value="">` with the `any` text, selected while `value` is `None`. Number box: a blank value calls `clear` when `any` is set. Typed box: blank text calls `clear` when `any` is set. Label text from `label` (default `ParamValue`).
  - `settings.rs` `TriggerState` arm: build `entity: Signal<Option<EntityView>>` from `House` and `entity_value` (pattern of `resolved` at line ~1682, without the upstream fallback). Replace the two `JsonField`s with
    `IfValueField value=<to signal> entity field=field_value write=… label=FlowText::FiresWhen any=FlowText::AnyChange clear=<sets to None>` and the same for `from` with `OnlyAfter` / `AnyValue`. Keep the existing `reads` re-mount so a changed entity or field shows cleared values.

- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib`, `cargo check -p nidoo-web --target wasm32-unknown-unknown`, `cargo clippy -p nidoo-web --all-targets -- -D warnings` — Expected: PASS, no warnings.

- [ ] **Step 5: Commit** — `feat(web): the Device card lists what the device reports instead of JSON`

---

### Final check

- [ ] Run the hub and web UI (project `run` skill). In Flows → new flow: "Appareil" is first under Déclencheurs. Pick a door sensor: Quand lists *N'importe quel changement / Ouvert / Fermé / Autre valeur…*. Pick a remote: the card becomes a signal card with its chips. Search "porte": "Quand Porte … devient ouvert" entries. Open an existing flow with a `trigger.state` and check it shows and saves unchanged.
- [ ] Whole-branch review, then push and open the PR (asks first).
