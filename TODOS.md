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

## Security audit (2026-11-17)

Found by a read-only audit of `main` (31134e23). The documented v1 posture (no login on HTTP, WebSocket and MCP; see [Security](docs/users/security.md)) is not repeated here: these go beyond it. Most broker items share one fix, an ACL with one user per client. Line numbers are as of that commit.

- [x] **P1** `GET /api/v1/setup/mobile-code` returned the broker, Frigate and gateway passwords to any web page open in a LAN browser (CORS `*`). It now answers `403` to a cross-origin page (`Sec-Fetch-Site`, else `Origin` vs `Host`); curl and scripts still get it (#204).
- [ ] **P2** The phone setup code still carries the secrets in clear: a DNS-rebinding page and any non-browser LAN client can read it. Swap them for a one-time, short-lived token the app exchanges; refusing rebinding needs the list of names the hub is reached by (plan with auth).
- [x] **P1** The bundled Mosquitto ships an ACL and the phone app gets its own `nidoo-app` login, denied the push notification topics, `nidoo/alarm/settings`, `nidoo/flow_labels` and `$SYS`; the setup QR carries it (#205).
- [ ] **P1** The rest of the broker split: the hub, Zigbee2MQTT, satellites and nidoo-sound still share the `nidoo` login with full access, on plaintext 1883 (`assets/mosquitto.conf`). One user per client with only its own topics; closes the push, flows and panel-command items below.
- [x] **P1** A forged push reply disarmed the alarm: any `alarm-` id was accepted while armed. Only ids of open alerts disarm now, and an old notification no longer disarms an alarm armed again since (#206).
- [ ] **P2** The panel command topic (`<alarm prefix>/panel/set`) disarms with no source check (`alarm/hub.rs:326`), and the app login needs it, so the ACL cannot deny it. Documented in `security.md` (#206). Options: move the app to another path, or a hub-checked / signed disarm.
- [x] **P1** SSRF with read-back on `GET /entities/{id}/picture`: the fetch now refuses loopback, link-local, unspecified, multicast and broadcast addresses after resolving, pins the checked IPs, exempts only the Frigate origin, serves `image/*` replies only, and every picture reply carries `Content-Security-Policy: sandbox` (#207).
- [ ] **P3** Picture fetch leftovers (#207 review): the DNS lookup is outside the 10 s timeout; NAT64 / 6to4 embedded addresses are not blocked; a self-fetch loop through the hub's own LAN IP is possible; private ranges stay reachable for a blind GET (LAN cameras need them).
- [x] **P2** Covers with no `device_class` count as shutters for the Assistant, so "ouvre les volets" opens a garage door discovered without a class (`nidoo-api/src/ambiances.rs:319`). Fixed in #209: an unclassed cover is a shutter only once marked as one; the covers already in the house were marked once by a migration.
- [ ] **P2** Push: any broker publisher can register its own FCM token (and receive every alarm and camera alert), send any push, or delete real tokens (`nidoo-push/src/tokens.rs:45`, `service.rs:144-205`). The token list is capped at 32 installs and broker alerts are rate-limited per source; the ACL part waits for the broker login split (P1).
- [x] **P2** minijinja templates from discovery run with no fuel: a retained `value_template` with nested loops spins a thread on every message and survives restarts (`nidoo-core/src/template.rs:61`). Each render now gets 50 000 instructions of fuel, templates are capped at 16 KiB and output at 64 KiB.
- [x] **P2** nidoo-sound PCM input (4953), HTTP API (4954) and the satellites' Sendspin socket took any LAN peer; a newer connection replaced the live one, so anyone played audio in every room or set volume 100. Now a different device cannot take over a stream that is playing, the HTTP API listens on `127.0.0.1` unless `http_bind` opens it, and the Mac satellite pins the first server that completes the Noise handshake and replaces its connection only after a full handshake. WebSocket messages are capped at 128 KiB and pending handshakes are capped too (#212).
- [ ] **P3** Sound leftovers: the PCM input still plays from any LAN device when nothing is playing (the casting app cannot send a key), and the ESP32 satellites use Sendspin's unpaired (Sentinel) key, so any LAN peer can be their server. Real Sendspin pairing on both ends would close both.
- [ ] **P2** `POST /api/v1/assist/stt/key` mints a new satellite key for any caller: it cuts every real satellite off and hands out a key to forge speech (`http/stt.rs:30`). Require the current key (or auth) to rotate.
- [ ] **P2** Flows are retained on the broker: any publisher can add or replace flows with `command`, `speak` and `notify` nodes; entries and payloads are unbounded (`flows/store.rs:61-100`). ACL on the flows prefix, cap count and size.
- [ ] **P3** The hub container runs as root with host networking (no `USER` in the Dockerfile). Add a non-root user and `chown` `./data` in `init-bundled`.
- [ ] **P3** FCM `token_uri` comes from the uploaded service-account JSON with no host check (`nidoo-push/src/fcm.rs:40`): pin `https://oauth2.googleapis.com/token`.
- [ ] **P3** `POST /setup/test/{mqtt,database,frigate}` works as an unauthenticated LAN port scanner, and `test/database` runs migrations on any database given; rate-limit, then put behind auth.
- [ ] **P3** `/ws`: no `Origin` check, no connection cap, axum's default message size. The in-memory picture store has no total cap (5 MiB per entity, any number of entities): use an LRU by bytes.
- [ ] **P3** Alerts carry attacker-chosen `image_url` / `video_url` that the browser loads when not on the Frigate origin (`alert_detail.rs:64-72`): allow http(s) on the hub or Frigate origin only.
- [ ] **P3** One `voice_key` is shared by every satellite, and `last_hub_ctr` is memory-only, so a reboot reopens a short replay window (`nidoo-satellite/src/voice.rs:70-85`). Per-satellite keys.
- [ ] **P3** Backup routes have no login: anyone on the LAN can start a restore or change the FTP target; plain FTP sends the FTP login in clear (the backup file itself is encrypted).
- [ ] When auth lands: put `/setup/*`, `/settings/*`, `/backup/*`, `/assist/stt/key` behind it; restore MCP's Host allowlist (`disable_allowed_hosts()` in `mcp/mod.rs`); replace `CorsLayer::permissive()`; add an `Origin` check on `/ws`; CSRF tokens if the session is a cookie.
- [ ] Run `cargo audit` in CI (no dependency scan was done).

## Home Assistant gaps (2026-11-17)

What a Home Assistant user misses in the first week. Each is its own brainstorm.

- [ ] **P1** Prebuilt images: CI only tests and builds the docs, so every install and update compiles the workspace on the home server. Publish multi-arch images to `ghcr.io` on release, let `compose.yml` pull them, and show "update available" in the web UI.
- [ ] **P1** A Home Assistant bridge: read entities and send commands over HA's WebSocket API, so Z-Wave JS, Matter, Hue, Shelly, Sonos and cloud integrations show up. Lets HA users run Nidoo beside HA instead of migrating.
- [ ] **P2** Flow triggers: a `for` duration on `trigger.state` ("no motion for 5 min"), numeric above/below crossings, a webhook trigger, a raw MQTT topic trigger.
- [ ] **P2** Flow actions: an HTTP request, an MQTT publish, reusable scripts (a flow called from flows, with parameters).
- [ ] **P2** Notification channels beyond Android/FCM: ntfy, Telegram, email (SMTP). Also an iOS answer (ntfy covers it).
- [ ] **P2** Users and login, with long-lived tokens for MCP and scripts (the fix behind most of the security list).
- [ ] **P3** Flows as files: export and import a flow (or all) as JSON, so they live in git like HA's YAML; a shared catalogue later (HA blueprints).
- [ ] **P3** English sentences for the Assistant (French only today).
- [ ] **P3** A documented Grafana setup on the TimescaleDB history (a ready dashboard JSON).
