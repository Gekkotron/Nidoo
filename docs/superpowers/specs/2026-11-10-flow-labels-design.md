# Flow labels and choices — design

Date: 2026-11-10 · Status: draft, awaiting approval

## Why

The house has dozens of flows, and some only make sense part of the year. Today, switching
"the winter flows" off means remembering which flows they are and turning each one off by
hand, then doing the reverse in spring. Nothing in Nidoo marks those flows.

The user wants to name groups of flows themselves, switch a whole group in one action, and
let a flow do the switching on a date ("on 21 June, go to summer"). It must stay generic:
nothing season-specific is built in. The same feature serves "Vacances", "Invités",
"Télétravail", "Tests" or anything else a house needs.

This replaces the "Bulk actions on the Flows page" backlog item (`TODOS.md`) and the
non-goal "Bulk actions / tags" of `2026-10-22-flow-list-design.md`.

## Concepts

- **Label**: a name the user creates ("Hiver", "Vacances"), with a colour. A flow carries
  zero or more labels.
- **Choice**: a named set of two or more labels where exactly one is *current*
  ("Saison: Hiver | Été", "Présence: Présent | Absent"). A label belongs to at most one
  choice; a label in no choice is a *standalone* label.
- **Picking** a label in a choice: every flow carrying that label is turned on, every flow
  carrying another label of the same choice (and not the picked one) is turned off, and the
  choice's current label becomes the picked one. Flows carrying no label of the choice are
  not touched.
- **Turning a label on / off**: every flow carrying it is turned on / off. Works for any
  label, in a choice or not.

## Goals

- Labels and choices the user defines, managed from the Flows page.
- Tag a flow with labels from the flow editor and from the list.
- Group the Flows list by label, with each section header carrying its switch.
- A flow action that picks a label, or turns a label on or off, so a scheduled flow can
  switch seasons by itself.
- The same through MCP, so assistants can read and switch labels.
- Tidy the Flows top bar: the seven trigger-kind chips fold into one "Déclencheur" menu.

## Non-goals

- Switching a label by voice (the Assistant). Later, its own brainstorm.
- Label support in the Android app. The hub contract is additive; the app adopts it later.
- Labels on anything other than flows (ambiances, dashboards, entities).
- A label that keeps flows in sync continuously. Picking or switching acts **once**, at that
  moment; afterwards each flow can still be switched by hand.
- Nested choices, or a flow being "required" to carry a label.

## Anti-goals

- A label switch that silently leaves some flows in the wrong state. Every flow it could not
  switch is reported (in the UI, the run history, and the MCP result).
- Fighting the user: a flow turned on by hand after a pick stays on until the next pick or
  switch.
- A season baked into the product (no "Hiver/Été" defaults, no calendar logic).
- A busier Flows top bar than today (it must lose a row, not gain one).
- A flow save that fails because of a stale label (mirror the `area` rule below).

## Behaviour details (decided)

- **Choice semantics**: a flow carrying **both** the picked label and another label of the
  same choice counts as carrying the picked one: it is turned on.
- **Already current**: picking the current label still runs and re-applies on/off to all its
  flows (useful after hand changes).
- **Only changes are written** *(my call)*: a flow already in the target state is not
  rewritten (a rewrite reloads the flow and could cancel its active run). The counts are the
  flows actually changed: « Été choisi : 2 activées, 0 désactivées » after two hand changes.
  When nothing changes: « Été choisi : rien à changer. »
- **Broker offline** *(my call)*: every label switch and catalog write is refused up front
  while the broker link is down, like `run_flow_from` and the other writers.
- **A write failing mid-way** *(my call)*: the switch keeps going, and the failed flows join
  the "not switched" list with their reason. The choice's `current` is still updated (it
  records what the user asked for); the result names what did not follow.
- **Invalid flows** are skipped by every label switch (today `set_flow_enabled` refuses
  them) and counted as "not switched" in the result.
- **A flow switching itself off**: when a run's label action turns off the run's own flow,
  the run finishes its remaining steps, exactly as `flow.enable` already does
  (`set_flow_enabled_by_run` / `EngineRequest::KeepRun`).
- **Result text**: French and English, e.g. « Été choisi : 5 activées, 12 désactivées. »,
  « Vacances : 3 désactivées. », plus « 1 non modifiée (invalide) : <names> » when any were
  skipped. The run step detail, the UI toast and the MCP result carry the same counts.
- **Catalog edits and `current`** *(my call)*: renaming or recolouring changes nothing;
  moving a label into a choice leaves the choice's `current` as it is; moving the current
  label out of its choice (or deleting it) clears `current`. No catalog edit switches flows.
- **Choice with no current label yet** (just created): no label shows as current until the
  first pick; nothing is switched by creating a choice.

## Data

### `Flow.labels`

- New field on `nidoo_core::flow::Flow`: `labels: Vec<String>`, label ids. Serialized as
  `"labels"`, omitted when empty (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`).
  Stored with the flow in its retained `nidoo/automation/<id>` payload, like `area`.
- Order is not meaningful; the hub stores them sorted and de-duplicated.
- **Hub validation on write** (HTTP save, MCP `set_flow`, the new set-labels endpoint): a
  label id that is not in the catalog is refused with a clear error, *unless it was already
  stored on that flow* (an unchanged stale id passes, mirroring `check_area` in
  `crates/nidoo/src/flows/hub.rs`).
- Loading retained flows stays lenient: an unknown label id never makes a flow invalid;
  readers ignore it. The web editor drops stale ids when the flow is next saved.
- Invalid flows: the hub fills the stub's `labels` from the raw JSON when readable, as it
  does for `name` and `area`, so an invalid flow still lists under its labels.

### The label catalog

- One retained JSON document owned by the hub, on its own topic (default
  `nidoo/flow_labels`; it must **not** sit under `nidoo/automation/+`, which is the flow
  wildcard). Shape:

  ```json
  {
    "labels":  [{ "id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison" },
                { "id": "ete", "name": "Été", "color": "amber", "choice": "saison" },
                { "id": "vacances", "name": "Vacances", "color": "green" }],
    "choices": [{ "id": "saison", "name": "Saison", "current": "hiver" }]
  }
  ```

- Ids are `[a-z0-9_-]{1,64}`, generated from the name on creation (accents folded,
  collisions suffixed) and **never change on rename**.
- `color` is one of a fixed palette of 8 named colours (the web maps them to its theme, light
  and dark). A new label gets the least-used colour; the user can change it.
- Order in the arrays is the display order (creation order; no reordering UI in this
  version).
- Unknown keys are preserved (same `extra` pattern as `Flow`).
- A choice must end up with **two or more labels** to be pickable; the UI shows a choice
  with fewer as "incomplete" and offers no picker. *(Allowed transiently while building it.)*
  A pick in an incomplete choice is also refused by the hub (HTTP, MCP and at run time:
  "choice `x` has fewer than two labels"). *(my call)*
- **Topic** *(my call)*: a config key `[topics] flow_labels`, default `nidoo/flow_labels`,
  validated like `automation_prefix`'s neighbours. No retained message (a fresh hub) means
  an empty catalog.

### Deleting

- **Delete a label**: it is removed from every flow carrying it (each flow rewritten), and
  from its choice (if it was current, the choice has no current label). If any flow's
  label action names it, the UI asks first and names those flows; after deletion those
  actions fail at run time with "unknown label" (so the flow shows under "À vérifier"),
  and saving such a flow is refused until the action is fixed.
- **Delete a choice**: its labels stay, as standalone labels. Label actions that *pick* in
  it fail at run time with "label no longer in a choice"; on/off actions keep working.
- **Remove a label from a choice** (in the manager): same as above for pick actions.

## Flow action

New node kind, beside `flow.enable` in `crates/nidoo-core/src/flow/model.rs`:

```json
{ "type": "flow.label", "params": { "label": "ete", "action": "pick" } }
```

- `action`: `pick` (only valid for a label currently in a choice), `on`, or `off`.
- **Save-time validation**: the label exists; for `pick` it is in a choice. Refused
  otherwise, like an unknown flow in `flow.enable` today. *Exemption (my call), mirroring
  `check_area`*: a `flow.label` node identical to the one already stored on that flow
  passes even if its label is gone, so a broken flow can still be saved to fix another
  node. Turning a flow off never runs these checks (`set_flow_enabled` skips them).
- **Run time**: re-checks (the catalog may have changed since the save); a failure fails
  the step with the reason. Success: the step detail is the result text above.
- **Concurrency** *(my call)*: one hub-level async lock serialises every operation that
  touches labels across flows: label switches (UI, MCP, `flow.label` steps), catalog writes,
  label and choice deletion, `set_flow` and set-flow-labels. Each re-reads the flows inside
  the lock, so a save cannot resurrect a deleted label. The builder confirms a `flow.label`
  step (engine awaiting the hub) cannot deadlock with the engine's reload path, as
  `flow.enable` already does.

## HTTP API (hub)

Follow the existing style in `crates/nidoo/src/http/api.rs` (e.g. `set_flow_area`):

- `GET /api/flow-labels` → the catalog plus, per label, how many flows carry it.
- Create / rename / recolour / move-to-choice / delete for labels and choices.
- `PUT /api/flows/:id/labels` → set a flow's labels (skips the entity checks, like
  `set_flow_area`).
- `POST /api/flow-labels/:id/pick`, `POST /api/flow-labels/:id/on`, `…/off` → the result
  counts and the names of skipped flows.
- Exact paths and payloads: builder's call, documented in `docs/reference/http-api.md`.

## MCP

- `list_flow_labels` (catalog with counts), `set_flow_labels` (create/rename/delete labels
  and choices), `switch_flow_label` (`label`, `action: pick|on|off`, same result as HTTP).
- `set_flow` / `get_flow` / `list_flows` carry `labels`.
- One sentence in the server instructions: labels group flows for the user; picking a label
  in a choice turns its flows on and the choice's other labels' flows off.

## Web UI

Mockups agreed in the browser, committed next to this spec as guidance for layout (the
text here wins where they differ): `2026-11-10-flow-labels-list-mockup.html` (option
**B2**) and `2026-11-10-flow-labels-manage-mockup.html`.

### Flows page top bar (B2)

- **Regroupement** becomes **Pièce | Étiquette | Aucun** (prefs persisted as today).
- The seven trigger-kind chips (État d'un appareil, Programmation, Soleil, Heures creuses,
  Silencieux depuis, Bouton / événement, Manuel) fold into one **Déclencheur ▾** menu on
  the status row: a multi-select with the same behaviour as today's chips, showing the
  number of selected kinds when filtering. The page goes from three control rows to two.
- Search also matches label names.

### Grouped by label

- Sections, in this order: each **choice** (in catalog order), then each **standalone
  label**, then **Sans étiquette**.
- A choice section lists every flow carrying any of its labels; its header shows the choice
  name and a segmented picker of its labels with the current one highlighted. Tapping
  another label picks it (no confirmation; the toast shows the result counts).
- A standalone label section header has **Tout activer** / **Tout désactiver**.
- Every header has ⚙ (opens the manager on that label or choice). "Sans étiquette" has
  **＋ Étiquette** (opens the manager to create one).
- A flow carrying several labels appears in each of their sections.
- Each row shows its labels as small coloured chips, in every grouping mode.

### Label manager

A dialog (from ⚙ or ＋ Étiquette): choices with their labels, then standalone labels; each
line with colour, name, flow count, rename/recolour (✎) and delete (🗑, with the
confirmation described above); **＋ Étiquette**, **＋ Choix**, and "＋ Étiquette dans ce
choix" inside a choice. Moving a label into or out of a choice is done from its ✎.

### Tagging a flow

- Flow editor settings: an **Étiquettes** field under **Pièce**: chips with ✕ and
  "＋ ajouter…" (pick an existing label or type a new name to create it).
- List row "⋯" menu: **Étiquettes…** next to "Déplacer vers une pièce", a checklist of all
  labels (choice name shown in grey) plus "＋ Nouvelle étiquette…".

### The action card

**Choisir une étiquette** in the node library, next to "Activer / désactiver une
automatisation". Two fields: **Choix** then **Étiquette** to pick; or, for a standalone
label (or "Activer/Désactiver" selected), **Étiquette** plus **Activer / Désactiver**. The
card shows a live preview: « Active les 5 automatisations « Été », désactive les 12
« Hiver ». » The canvas summary line reads « Choisir une étiquette : Saison → Été ».

## Docs

- `docs/users/flows.md`: a "Labels and choices" section with the winter/summer example
  and a scheduled flow that switches on 21 June (`0 0 21 6 *`).
- `docs/reference/flow-nodes.md` (`flow.label`), `mqtt-topics.md` (the catalog topic),
  `http-api.md`, `mcp-tools.md`.
- `TODOS.md`: replace the "Bulk actions on the Flows page" item; add the voice and Android
  follow-ups as non-goals above.

## Constraints

- Public, generic repo: no house words in code, fixtures or docs beyond generic examples.
- Every user-visible string in English and French (`crates/nidoo-web/src/i18n/flows.rs`).
- Retained documents stay backward compatible: new fields default, unknown keys preserved.
- Commits as `Gekkotron`. Green before each commit: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, the touched crates' tests and the
  web browser tests for the Flows page.

## Left to the builder

- Module layout, the catalog's in-memory store and how it hooks into the hub's retained
  ingestion, exact HTTP paths and payloads, MCP param structs.
- The 8 colour names and their theme values.
- Id generation details (folding, suffixing).
- Component structure of the manager dialog and the Déclencheur menu.
- Test layout, following each crate's existing patterns (hub integration tests for the
  switch semantics and self-disable; web browser tests for the top bar, grouping and the
  picker).
