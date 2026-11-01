# TODOS

Running backlog of ideas and deferred follow-ups. Priority: **P1** = correctness or user-visible safety (do next), **P2** = polish or test coverage, **P3** = cosmetic or opportunistic.

## Dashboards (landed on `feat/dashboards`, 2026-10-22; triaged 2026-10-24)

### P3 — cosmetic / backlog

- Two concurrent PUTs can pick the same `next_free_order` (lock released between read and publish). Fine single-user; add a guard if the hub grows multi-writer.

## Climate engine v1 (landed on sc-critical-meissner-823c, 2026-10-22)

Spec: `docs/superpowers/specs/2026-10-22-climate-engine-design.md`. Plan: `docs/superpowers/plans/2026-10-22-climate-engine.md`.

- Follow-up designs, each its own brainstorm: shutters, comfort ↔ cost slider with prices, ML layers.

### Deferred follow-ups (from the SDD reviews; none blocks enabling the feature)

- **T3** hold_devices skips a device override the device can't do; device keeps zone target (decide.rs hold_devices)
- **T5** combine() float summation order depends on device order (learn.rs combine)
- **T7** no test pins seq tie-break for equal times (history_db.rs)
- **T8** unconfirmed entry expiry 120 s can turn a late-applied engine command into a hand change — Ruling: real but rare; holds the engine's own value (safe) — deferred.
- **T8** a dropped pass's abandoned command is not moved to unconfirmed (engine.rs:61)
- **T8** a safety alert resolved less than six hours before a restart can be pushed again: the repeat window is in memory, and the push history keeps no resolve time (an open alert is no longer re-pushed).
- **T9** autumn repeated hour merges samples in micros_local (learning.rs:68) — Ruling: real, ≤1 h/year of evidence — deferred.
- **Final** overlapping learning sequences / manual learn_climate_zone bypasses claim_run — Ruling: practically unreachable, conclude is serialised — deferred.
- **Final** one failed re-assert right after reconnect turns drift into a hold after ~121 s — Ruling: never fights the user; frost overrides — deferred.

## Other backlog ideas (unrelated to dashboards)

### Homey-gap features

- [ ] Category views — the Home grouped by what a device *does*, not just by room. Not started (the `feat/category-views` branch is gone).

### Hub & web polish

- [ ] Dashboard-wide search bar — the current search is only on the Z2M diagnostics screen (`screens/devices.rs`); add an equivalent on the dashboard/Home so any entity can be found by name across rooms.
- [ ] Address the ~47 deferred minors from the flow-canvas redesign SDD ledger (`.superpowers/sdd/2026-10-20-flow-canvas-redesign/`).
- [ ] Housekeeping: cap/clean `target/` so it stops filling the disk.
- [ ] System health screen + uptime history — record the hub's uptime, CPU, RAM, disk and temperature over time (and the broker / Z2M / database up or down), and add a screen that shows the current values and their history. Today `oikos-watchdog` only pushes gateway and ACL alerts; nothing is stored or shown.

### Flows

- [ ] Bulk actions on the Flows page — enable, disable, delete or move several flows at once from the list. Deferred from the flow-list redesign (2026-10-22): the list acts on one flow at a time.
- [ ] Automatic room detection for flows — suggest a flow's `area` from its trigger and action entities instead of leaving it unassigned; today the room is set by hand only.
- [ ] Android app support for a flow's `area` — the app does not read or write it yet; the hub and web UI do.
