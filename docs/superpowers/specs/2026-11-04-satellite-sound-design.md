# Satellites and the sound service — design (project 1 of 3)

Date: 2026-11-04. Status: approved by the owner 2026-11-04.

## Intent

The house gets voice satellites: small ESP32-S3 boards, one per room, that
will later listen for the assistant and speak its replies in the room they sit
in. Before voice, they must do the simpler half of their job well: **play the
same music in every room, in sync**, with each room's volume and mute
controlled from Nidoo.

This replaces the owner's earlier Snapcast setup (a separate repo: snapserver
on the house server plus a custom Rust Snapcast client on an ESP32-WROOM-32).
Snapcast is turned off; the satellites run stock ESPHome firmware and receive
music over the **Sendspin** protocol (open, royalty-free, Open Home
Foundation), which ESPHome supports as an experimental component.

The work is split into three projects, built in this order:

1. **This document** — satellites + sound service: ESPHome on the boards, a
   Sendspin-compatible server (`nidoo-sound`), one music input, per-room volume
   and mute from Nidoo.
2. Hub ↔ satellites: Nidoo connects to each board over the ESPHome native API,
   knows its room, speaks TTS replies/announcements in one room and ducks the
   music there.
3. Speech in: on-board wake word, speech-to-text in the hub, the full
   "talk → act → answer in the same room" loop.

## Goals

- Two ESP32-S3 satellites (more later) play the music in sync: standing
  between two rooms there is no audible echo.
- The only music input is a phone app that sends raw PCM over TCP (the owner's
  Oikos Cast app: s16le, 48 kHz, stereo, to TCP port **4953**, reconnecting
  every 2 s while casting). The app is used **unchanged**.
- When the phone connects, every satellite plays; when it disconnects, they go
  silent. No room picking, no groups.
- Each satellite appears in Nidoo as a device in its room with a **volume**
  and a **mute** control, usable from dashboards, flows, the Android app and
  the MCP tools like any other entity.
- Volume and mute per satellite are **remembered**: they survive a satellite
  reboot, a `nidoo-sound` restart and a hub restart.
- Rebuilding or restarting the hub does **not** interrupt the music.
- The microphone is already wired in the board configuration (unused until
  project 3), so we learn now whether one S3 can run Sendspin playback and the
  microphone together.

## Non-goals (this project)

- Voice: wake word, speech-to-text, TTS on the satellites, ducking (projects
  2 and 3).
- Nidoo talking the ESPHome native API (project 2).
- Other inputs: UPnP/DLNA (BubbleUPnP → upmpdcli → MPD), Roc, URLs, files.
  The design must leave room for them (see "Inputs").
- Room selection, groups, per-room sources, the assistant starting music.
- Per-room EQ (the old Snapcast firmware had one; not carried over now).
- A dedicated "speaker" entity kind or card in Nidoo (number + switch for now).
- Changes to the Oikos Cast Android app. Its "Switch all rooms to the phone"
  option talks to Snapcast's JSON-RPC and is simply turned off by the owner.
- The old ESP32-WROOM-32 Snapcast speaker (retired).

## Anti-goals (failure even if it "works")

- A hub rebuild cutting the music in every room.
- Music Assistant, Home Assistant or Snapcast as a runtime dependency.
- Naming the owner's hosts, IPs, rooms or Wi-Fi in tracked files (the repo is
  public and generic; per-install values go in git-ignored files).
- Shipping something called "Sendspin <X>" (trademark; see Licences).

## Constraints and facts

**Hardware (checked on the board with esptool, 2026-11-04):** ESP32-S3 (QFN56,
rev v0.2), dual core 240 MHz, **8 MB embedded PSRAM**, **8 MB flash**, native
USB-Serial/JTAG. Microphone **INMP441** (I2S), DAC **PCM5102A** (I2S, line
level out to powered speakers or an amp). The S3 has two I2S peripherals: mic
and DAC each get their own. Pins: see "Reference wiring".

**ESPHome Sendspin (research, 2026-11-04 — verify against the pinned
version):** the `sendspin` component family is in ESPHome since about
2026.5 and marked **experimental** ("protocol not yet finalized, breaking
changes possible"). ESP-IDF framework only. It speaks Sendspin
**1.0.0-rc1**. Components: the `sendspin:` hub plus media player / media
source / sensor / switch / text sensor platforms; pairing options
(`static_pairing_code`, `sendspin.confirm_pairing_window`). mDNS discovery;
port 8928 appears in the docs. Not documented: coexistence with
`voice_assistant` / `micro_wake_word`, S3 limits, ducking of Sendspin audio.

**Sendspin protocol (spec: github.com/Sendspin/spec):** WebSocket, JSON
handshake then binary audio frames. mDNS `_sendspin._tcp` (server connects to
clients, port 8928 recommended) and `_sendspin-server._tcp` (clients connect
to the server, port 8927). Optional Noise `KKpsk2` encryption with a pairing
PSK/code. Roles such as `player@v1`, `controller`, `metadata`. Clock sync by
`client/time` / `server/time` exchanges (Kalman filter for offset and drift).
Each client is in exactly one group; group and player volume/mute exist
(`server/command`). Codecs include PCM, FLAC and Opus (exact list to read from
the spec).

**No usable server exists:** Music Assistant (rejected), `aiosendspin`
(Python, library, no standalone server), `sendspin-go-server` (v0.1.x, file/URL
inputs only, no volume API). The Rust crate `sendspin` (0.3.x) is player side
only. So `nidoo-sound` implements the server side itself.

**Deployment:** the house server runs Nidoo with `compose.yml` (`network_mode:
host`, data under `./data`). The music input port 4953 is freed when the owner
stops snapserver there. ESPHome boards and the server are on the same LAN
(mDNS works between them).

**Licences:** Nidoo is Apache-2.0. The Sendspin spec is under the Community
Specification License 1.0: anyone may implement it royalty-free, no
attribution needed for implementations; the **patent grant applies when a
copy of that licence sits in the source root of the implementation**, so
`crates/nidoo-sound/` carries a copy (e.g. `LICENSE-SENDSPIN-SPEC.md`).
"Sendspin" is a trademark: describe the crate as "a Sendspin-compatible
server", never name it "Sendspin …". ESPHome is not shipped, only a YAML
configuration. If the builder uses `esphome-native-api` later (project 2), it
is MIT.

## Decided (by the owner)

- Stock **ESPHome** on the boards, not custom firmware.
- **Sendspin** for music; Snapcast is turned off; no Music Assistant.
- We write our own Sendspin-compatible server in Rust.
- It lives **in the Nidoo repo as its own crate, running as its own
  service** next to the hub, not inside the hub process.
- Nidoo controls each satellite's volume and mute; both are remembered.
- Only the phone app input for now; every room plays it.
- Two satellites for now; hardware as above.

## Design

### Pieces

```
phone app ──TCP 4953, raw PCM──▶ nidoo-sound ──Sendspin (WebSocket)──▶ satellite A (ESPHome)
                                    │   ▲                         └──▶ satellite B (ESPHome)
                     MQTT discovery │   │ volume / mute commands
                       + state      ▼   │
                               broker ◀──▶ Nidoo hub ◀── dashboards, flows, app, MCP
```

1. **`satellite/` (ESPHome configuration)** — one YAML for every satellite,
   per-board values (name, Wi-Fi, GPIOs, pairing code) through substitutions
   and a git-ignored `secrets.yaml` (a committed `secrets.example.yaml`).
   Contents: Wi-Fi, logger, `psram`, `i2s_audio` × 2 (INMP441 mic,
   PCM5102A out), `microphone` (configured, unused), `speaker` +
   `sendspin` media player, OTA. ESPHome version **pinned** (the Sendspin
   protocol version must match the server). Built and flashed from a laptop
   with a pinned ESPHome CLI (`uvx esphome==<x> run …`), first by USB, then
   over Wi-Fi. A short README: wiring table, flash, add a satellite.

2. **`crates/nidoo-sound` (new binary crate)** — the sound service:
   - **Sendspin server**: discovers satellites over mDNS and/or accepts
     their connections (whichever direction ESPHome's component uses —
     verify), handshake, roles, clock sync, streaming with timestamps far
     enough ahead for every player to start in sync, per-player
     volume/mute via the protocol.
   - **Input**: a TCP listener on 4953 accepting raw s16le/48 kHz/stereo PCM.
     One phone at a time; a new connection replaces the old one. Connected
     and sending = playing in every room; disconnected = stop. A short
     silence/underrun from Wi-Fi jitter must not stop playback; a gap long
     enough to mean "phone gone" does. Behind a small `Input` trait so
     UPnP/MPD (FIFO) and others can be added later without touching the
     server core.
   - **State**: per-satellite volume (0–100) and mute in a small file under
     the data directory (same pattern as the hub's `hub.json`), restored on
     start and re-applied whenever a satellite (re)connects.
   - **Nidoo link over MQTT**: for each known satellite, publish Home
     Assistant MQTT discovery under the standard `homeassistant/` prefix so
     the hub (and the Android app, which reads only `homeassistant/`) picks
     it up with no hub code: one **device** per satellite with a `number`
     "Volume" (0–100, slider), a `switch` "Mute", and a `binary_sensor`
     "Connected" (connectivity). State topics retained; command topics
     subscribed; availability topic with a last will so a stopped service
     shows unavailable. A satellite that was seen once keeps its entities
     (state "disconnected") until removed. The room is set in Nidoo as for
     any device (areas / override), not in `nidoo-sound`.
   - **Config**: broker address/credentials (same style as `nidoo.toml`'s
     `[mqtt]`; reuse the hub's config loading if it fits), input port, data
     dir, codec choice, optional pairing PSK. CLI with `clap`, logs with
     `tracing`, like the hub.
   - **Licence file** for the spec copied into the crate root.

3. **Deployment** — `nidoo-sound` built in the same `Dockerfile` (one more
   `cargo build --release -p nidoo-sound`, binary copied to the runtime
   image) and run as its own `sound` service in `compose.yml` (`network_mode:
   host`, `./data` volume, `restart: unless-stopped`). It must be startable
   and restartable without the hub (`docker compose up -d sound`). Docs in
   `docs/users/` alongside the install guide: what it is, ports (4953 in,
   Sendspin ports), turning off snapserver, flashing a satellite, assigning
   its room in Nidoo.

### Behaviour

- Phone connects → `nidoo-sound` starts a stream to every connected
  satellite, each at its remembered volume/mute.
- A satellite connects mid-stream → it joins in sync.
- Volume/mute changed in Nidoo → MQTT command → `nidoo-sound` applies it
  through Sendspin and saves it → state echoed back (so Nidoo's "accepted"
  check passes).
- Volume/mute changed on the satellite itself (if ESPHome allows it) → reported
  back and saved, so Nidoo and the board agree.
- Hub restarts → nothing happens to the audio; retained discovery and state
  bring the entities back.
- `nidoo-sound` restarts → satellites reconnect, the phone app reconnects
  (2 s retry), volumes restored.

### Testing

- Unit tests (no hardware) for the protocol framing/handshake, clock sync
  maths, the PCM input chunker and timestamping, state persistence, and the
  MQTT discovery payloads (checked against the hub's own discovery parser in
  `nidoo-core` so the entities really appear with the right kinds).
- A fake Sendspin player in tests (or the Rust `sendspin` player crate as a
  dev-dependency, licence permitting) to run server ↔ player end to end on
  localhost.
- Hardware acceptance on the two boards: music from the phone in both rooms
  with no audible echo; volume and mute from a Nidoo dashboard; the values
  survive a board reboot, a `nidoo-sound` restart and a hub restart; the
  music keeps playing through a hub restart; a soak of a few hours without
  dropouts or board resets (watch heap/PSRAM in the ESPHome logs).

## Success

The owner turns off snapserver, starts casting from the phone, and both rooms
play together; they turn one room down from a Nidoo dashboard, rebuild the
hub, and the music never stops and the volume is still where they left it.

## Calls made at review (Claude's, not the owner's — the owner checks these)

1. **Spike first.** The plan starts with a hardware spike: flash the pinned
   ESPHome with Sendspin and the microphone on one board, and drive it from a
   minimal server. It answers: connection direction and discovery, codecs the
   player takes, whether volume/mute travel over Sendspin, whether the player
   reports local volume changes, the client id it sends, and whether
   playback + microphone run together (heap/PSRAM, no resets). If a finding
   changes this document's scope (volume not controllable over Sendspin, the
   board unstable with the microphone, sync unusable), **stop and report to
   the owner**; do not work around it silently. A microphone problem alone
   does not block project 1: it is written down as a finding for project 3.
2. **Protocol subset.** Implement only what the pinned ESPHome player needs:
   the `player@v1` role, clock sync, one codec, one group holding every
   player, player volume/mute. No controller/metadata/artwork roles, no
   multi-group logic. Target the Sendspin revision that the pinned ESPHome
   version speaks; when ESPHome moves, both are bumped together.
3. **Satellite identity.** Key everything (state file, MQTT `unique_id`s,
   discovery device identifiers) on the stable client id the player sends in
   the handshake. If the spike shows it is not stable across reboots, fall
   back to the board's MAC. Display name = the name the board announces.
   Changing the key later would orphan room assignments, so it is fixed after
   the spike.
4. **Input timing.** TCP close = stop now. Connected but no bytes for **5 s**
   = stop (players go idle), resume when bytes come back. Shorter gaps are
   filled with silence so the players never underrun. The server's monotonic
   clock paces the stream; a small jitter buffer absorbs Wi-Fi bursts and
   drift between the phone and the server is handled by dropping/inserting
   single frames. Bit-exact is not required.
5. **Commands to an offline satellite.** Store and echo at once (Nidoo
   shows the new value, its "accepted" check passes, and the board gets it
   when it reconnects). "Connected" shows the board is offline.
6. **Broker access.** The bundled Mosquitto has no ACLs, so `nidoo-sound`
   uses the same broker login as the hub (from its own config) and publishes
   under `homeassistant/` like any other integration.
7. **Docker image.** `nidoo-sound` gets **its own image tag**
   (`nidoo-sound:local`, its own `Dockerfile` target), so rebuilding and
   recreating the hub never recreates the sound service.

## Left to the builder

- Codec on the wire (PCM vs FLAC vs Opus), buffer/lead time and chunk size —
  pick what the pinned ESPHome Sendspin player supports and keeps sync tight
  on Wi-Fi; latency only needs to be "fine for music" (the old setup had ~1 s).
- Connection direction and discovery details, pairing/encryption (on by
  default if ESPHome makes it easy, else plaintext on the LAN, documented).
- Crate internals, the async structure, the state file format.
- The exact ESPHome version to pin (latest stable with Sendspin at build
  time) and the YAML details (mixer/speaker chain, sample rates).
## Reference wiring (Seeed Studio XIAO ESP32S3)

The boards are XIAO ESP32S3 (8 MB flash, 8 MB PSRAM). Default wiring, used
as the YAML defaults (substitutions, so another board can override them):

| Part | Pin | XIAO | GPIO |
|---|---|---|---|
| INMP441 | SCK | D1 | 2 |
| INMP441 | WS | D0 | 1 |
| INMP441 | SD | D3 | 4 |
| INMP441 | L/R | GND | — (left channel) |
| INMP441 | VDD / GND | 3V3 / GND | — |
| PCM5102A | BCK | D8 | 7 |
| PCM5102A | LRCK | D9 | 8 |
| PCM5102A | DIN | D10 | 9 |
| PCM5102A | SCK | GND | — (internal PLL from BCK) |
| PCM5102A | VIN / GND | 5V / GND | — |

PCM5102A pads: FLT L, DEMP L, XSMT H, FMT L. Avoided: D2/GPIO3 (strapping),
D6/D7 (GPIO43/44, UART0); D4/D5 kept free for a future I2C LED ring or sensor.
