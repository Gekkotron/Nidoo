# `trigger.state`: hold for a duration, number crossings

Date: 2026-11-20. Backlog: TODOS.md, Home Assistant gaps, "Flow triggers".

## Why

The most common automations a Home Assistant user brings are "no motion for 5 min → lights off",
"door open for 10 min → notify" and "temperature goes above 25 → close the shutters". Nidoo's
`trigger.state` fires on every change and filters only on an exact `to` / `from`, so neither is
possible. `trigger.silent` measures something else: no report at all.

## Params (nidoo-core, done)

`TriggerStateParams` gains three optional fields:

- `above: f64`, `below: f64`: **crossing**. Let `C(v)` = `v` is a number and `v > above` (if set)
  and `v < below` (if set) (`TriggerStateParams::in_bounds`). The edge is `C(previous) == false`
  and `C(next) == true`. With both, the edge is entering the range. No dead band: a `for` debounces
  a noisy sensor. Not combinable with `to` / `from`; `above < below`; finite. A value appearing
  after unknown (the existing "confirmed unknown" path, `previous == None`) fires if `C(next)`.
- `for: "5m"`: **hold**, 1s to 168h (`flow::hold`). The edge arms a wait instead of firing.

## Engine (crates/nidoo/src/flows/engine.rs)

- An edge of a trigger with `for` arms a pending hold keyed `"{flow}/{node}"`, holding the
  `TriggerInfo` of the edge and the deadline `now + for`. A new edge of the same key while armed
  (only possible for a trigger with no `to` / `above` / `below`: any change) restarts it with the
  new info.
- **Still holds** for a reported value `v` of that watched (entity, field):
  `to` set → `filter_matches(to, v)`; `above` / `below` set → `C(v)`; neither → `v` equals the
  edge's value. A report where it no longer holds cancels the wait. A value vanishing (unknown /
  unavailable, `None`) does not cancel.
- At the deadline: re-read the entity's current value from the hub registry (same
  `watched_value` path). Fire through `fire()` (honours the flow mode) only if the value is known
  and still holds; drop the hold either way. Offline at the deadline: drop it.
- Reload: keep a hold only if its flow is still enabled and the node's params are unchanged.
- Hub restart: waits in progress are lost (documented).
- The timer branch wakes at the earliest of the existing deadlines and the holds' deadlines.

## Web (crates/nidoo-web)

- Card editor (`screens/flow_canvas/settings.rs`, `NodeKind::TriggerState`): an optional
  duration field "For" (same `TextField` style as `trigger.silent`, placeholder `30s, 5m`), and,
  when the watched value is a number, optional "Above" / "Below" number fields. Setting
  above/below clears `to` / `from` and hides those filters; setting `to` / `from` clears them.
- Sentence (`flow_text.rs`), FR and EN, e.g. "When Living temperature goes above 25 °C",
  "Quand Température salon dépasse 25 °C", "… descend sous 18 °C", "… passe entre 18 et 22 °C";
  with `for`, the hold appended: "… for 5 min" / "… pendant 5 min".

## Docs

`docs/users/flows.md`: the two options, the crossing-once rule, the restart limit.

## Non-goals

Dead band / hysteresis, persisting waits across a hub restart, `for` on other triggers,
Android app editing (the app keeps unknown params in `extra`).
