# TODOS

Running backlog of ideas and deferred follow-ups. Priority: **P1** = correctness or user-visible safety (do next), **P2** = polish or test coverage, **P3** = cosmetic or opportunistic.

## Climate engine v1 (landed on sc-critical-meissner-823c, 2026-10-22)

Spec: `docs/superpowers/specs/2026-10-22-climate-engine-design.md`. Plan: `docs/superpowers/plans/2026-10-22-climate-engine.md`.

- Follow-up designs, each its own brainstorm: shutters, comfort ↔ cost slider with prices, ML layers.

### Deferred follow-ups (from the SDD reviews; none blocks enabling the feature)

- **T8** unconfirmed entry expiry 120 s can turn a late-applied engine command into a hand change — Ruling: real but rare; holds the engine's own value (safe) — deferred.
- **T9** autumn repeated hour merges samples in micros_local (learning.rs:68) — Ruling: real, ≤1 h/year of evidence — deferred.
- **Final** overlapping learning sequences / manual learn_climate_zone bypasses claim_run — Ruling: practically unreachable, conclude is serialised — deferred.
- **Final** one failed re-assert right after reconnect turns drift into a hold after ~121 s — Ruling: never fights the user; frost overrides — deferred.

## Assistant v1 (branch feat/audio-assistant-mcp-tools, 2026-11-02)

Spec: `docs/superpowers/specs/2026-11-02-assist-design.md`. Plan: `docs/superpowers/plans/2026-11-02-assist.md`. Typed French sentences only; the items below are the direction, each its own brainstorm.

- [ ] **P2** Speech in, on the hub, in Rust: a French CTC acoustic model (ONNX, run with `ort`) decoded against a grammar compiled from the sentences `nidoo-assist` understands plus the house's names, with a confidence gate; whisper.cpp as fallback; Silero VAD for the end of speech. Spike accuracy (allume / éteins) and CPU latency first.
- [ ] **P3** Speech out: Phonon (weights CC BY-NC: download, never bundle) or Piper.
- [ ] **P3** Satellites: ESP32-S3 boards streaming 16 kHz audio over MQTT.
- [ ] **P3** LLM fallback behind the same plan and safety layer, with tiny tools (the spike: 15/16 with three purpose-built tools, 0/3 with the raw MCP tools).
- [ ] **P3** Learned shortcuts shared with the Android app; English sentences.
- [ ] **P3** The floating button opens the conversation as a panel over the page instead of navigating.

## Other backlog ideas (unrelated to dashboards)

### Homey-gap features

- [ ] Category views — the Home grouped by what a device *does*, not just by room. Not started (the `feat/category-views` branch is gone).

### Hub & web polish

- [x] Dashboard-wide search bar — the current search is only on the Z2M diagnostics screen (`screens/devices.rs`); add an equivalent on the dashboard/Home so any entity can be found by name across rooms. Done on branch feat/dashboard-search (2026-11-02).
- [ ] Address the ~47 deferred minors from the flow-canvas redesign SDD ledger (`.superpowers/sdd/2026-10-20-flow-canvas-redesign/`).
- [ ] Housekeeping: cap/clean `target/` so it stops filling the disk.

### Flows

- [ ] Bulk actions on the Flows page — enable, disable, delete or move several flows at once from the list. Deferred from the flow-list redesign (2026-10-22): the list acts on one flow at a time.
- [ ] Automatic room detection for flows — suggest a flow's `area` from its trigger and action entities instead of leaving it unassigned; today the room is set by hand only.
- [ ] Android app support for a flow's `area` — the app does not read or write it yet; the hub and web UI do.
- [ ] Android app: localise `hub.restarted_unexpectedly` and the `service.unreachable` / `service.reachable_again` texts for the new services `broker`, `zigbee2mqtt` and `database` (the hub sends the keys with their arguments; see [System health](docs/users/system-health.md)).
