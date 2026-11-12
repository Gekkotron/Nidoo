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

- [x] **P2** Speech in: "Hey Jarvis" on a satellite, Parakeet TDT 0.6B v3 + Silero VAD on the hub (sherpa-onnx), real turns, the reply spoken on that satellite with the room's music held. Spec: `docs/superpowers/specs/2026-11-04-speech-in-design.md`. Later: snapping misheard names to the house's names, a custom wake word.
- [ ] **P2** Voice and security: before adding an alarm, lock, garage or entrance door intent, refuse it from a `satellite:` client (speech input) unless that intent is enabled for voice. Spec: `docs/superpowers/specs/2026-11-04-speech-in-design.md`. Decided 2026-11-09: shutters and windows are not part of this rule; voice opens them exactly like a typed sentence (garage, gate and door covers stay excluded from the Assistant everywhere).
- [x] **P3** Speech out: Phonon (weights CC BY-NC: download, never bundle). Spec: `docs/superpowers/specs/2026-11-03-assist-tts-design.md`. The hub speaks on request; Assist does not speak its replies until the satellites exist.
- [ ] **P3** Satellites: ESP32-S3 boards streaming 16 kHz audio over MQTT.
- [ ] **P3** LLM fallback behind the same plan and safety layer, with tiny tools (the spike: 15/16 with three purpose-built tools, 0/3 with the raw MCP tools).
- [ ] **P3** Learned shortcuts shared with the Android app; English sentences.
- [ ] **P3** The floating button opens the conversation as a panel over the page instead of navigating.

## Sound (`nidoo-sound`): future ideas

What `nidoo-sound` does not support yet (see Supported features in `crates/nidoo-sound/README.md`). Each is its own brainstorm.

- [ ] **P3** Music per room: several inputs, each played on a chosen set of rooms (Sendspin groups), instead of one source in every room; pick the rooms from Nidoo.
- [ ] **P3** Compressed audio to the satellites (FLAC, which the Sendspin spec asks servers to offer): less Wi-Fi than raw PCM (about 1.5 Mbit/s per satellite today).
- [ ] **P3** Pairing: pair each player instead of the shared unpaired-access key, so only paired players can play and a man in the middle on the LAN is shut out.
- [ ] **P3** Advertise `_sendspin-server._tcp`, so players that only dial advertised servers find the service, without giving two connections to a player that both advertises and dials.
- [ ] **P3** Controller and metadata roles, and `group/update`: now-playing title and artwork on the players, play, pause and next from them.

## Other backlog ideas (unrelated to dashboards)

### Homey-gap features

- [ ] Category views — the Home grouped by what a device *does*, not just by room. Not started (the `feat/category-views` branch is gone).

### Hub & web polish

- [x] Dashboard-wide search bar — the current search is only on the Z2M diagnostics screen (`screens/devices.rs`); add an equivalent on the dashboard/Home so any entity can be found by name across rooms. Done on branch feat/dashboard-search (2026-11-02).
- [ ] Address the ~47 deferred minors from the flow-canvas redesign SDD ledger (`.superpowers/sdd/2026-10-20-flow-canvas-redesign/`).
- [ ] Housekeeping: cap/clean `target/` so it stops filling the disk.

### Flows

- [x] Bulk actions on the Flows page — replaced by flow labels and choices: tag flows, then turn a label's flows on or off, or pick a label in a choice (winter/summer). Spec: `docs/superpowers/specs/2026-11-10-flow-labels-design.md`. Done on branch feat/flow-labels (2026-11-10).
- [ ] Switch a flow label by voice — "passe en mode été" through the Assistant; the hub and MCP already switch labels (`switch_flow_label`). Out of scope of the labels spec; needs its own brainstorm.
- [ ] Android app support for flow labels — the app does not read or write a flow's `labels`, the label catalog or the label switch yet; the hub contract is additive.
- [ ] Automatic room detection for flows — suggest a flow's `area` from its trigger and action entities instead of leaving it unassigned; today the room is set by hand only.
- [ ] Android app support for a flow's `area` — the app does not read or write it yet; the hub and web UI do.
- [ ] Android app: localise `hub.restarted_unexpectedly` and the `service.unreachable` / `service.reachable_again` texts for the new services `broker`, `zigbee2mqtt` and `database` (the hub sends the keys with their arguments; see [System health](docs/users/system-health.md)).

## Backup and restore v1 (branch feat/backup, 2026-11-11)

Spec: `docs/superpowers/specs/2026-11-11-backup-design.md`. User docs: `docs/users/backup.md`. Each item below is its own brainstorm.

- [ ] **P2** More destinations: SFTP, FTPS, SMB, S3-compatible storage, a folder on the hub host. Each is a new way to send, list and fetch a file, not a new format.
- [ ] **P3** MCP tools `run_backup` and `get_last_backup`.
