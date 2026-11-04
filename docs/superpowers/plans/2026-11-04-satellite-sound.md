# Satellites and the sound service — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two ESPHome satellites (XIAO ESP32S3 + INMP441 + PCM5102A) play the phone's raw PCM in sync, served by a new Rust service `nidoo-sound` that speaks the Sendspin protocol and exposes per-satellite volume / mute / approval to Nidoo over MQTT discovery.

**Architecture:** `nidoo-sound` is a standalone tokio binary in the Nidoo workspace. It browses mDNS for `_sendspin._tcp` players (ESPHome advertises; the server connects: "server-initiated"), runs the Noise `KKpsk2` handshake with the Sentinel PSK (unpaired access), keeps each player's clock answered, and streams 20 ms PCM chunks timestamped `lead` ahead of its monotonic clock to every approved player. A single `Group` actor owns the players, the pacer fed by the TCP PCM input (port 4953) and the persisted per-player prefs; an MQTT task maps prefs to Home Assistant discovery entities and commands back.

**Tech Stack:** Rust 2024, tokio, tokio-tungstenite (WebSocket client), snow (Noise), mdns-sd, rumqttc, serde/serde_json, figment + clap, tracing; ESPHome 2026.10.2 (YAML); Docker compose.

**Spec:** `docs/superpowers/specs/2026-11-04-satellite-sound-design.md`

**Protocol references (read the sections named in each task, at these exact versions):**
- Sendspin spec at tag **`1.0.0-rc1`**: `README.md` (connection, encryption, messaging, core messages) and `roles/player/v1.md` — `gh api "repos/Sendspin/spec/contents/<file>?ref=1.0.0-rc1" --jq .content | base64 -d`.
- The player actually on the boards: ESPHome **2026.10.2** `esphome/components/sendspin/`, which pins **`sendspin/sendspin-cpp` 0.9.3**. Where the spec and sendspin-cpp 0.9.3 disagree, **sendspin-cpp 0.9.3 wins** (it is what we must interoperate with); note the difference in the spike notes.

## Global Constraints

- Crate name `nidoo-sound`; describe it as "a Sendspin-compatible server". Never name a crate, binary, service or entity "Sendspin …" (trademark).
- `crates/nidoo-sound/LICENSE-SENDSPIN-SPEC.md` = verbatim copy of the spec repo's `LICENSE.md` at `1.0.0-rc1` (patent grant condition). Crate licence `license.workspace = true` (Apache-2.0).
- ESPHome pinned to **2026.10.2** everywhere (`uvx esphome==2026.10.2 …`); bump ESPHome and the server together.
- Public repo, generic: no owner hosts, IPs, room names, Wi-Fi names or passwords in tracked files. Per-install values live in git-ignored `satellite/secrets.yaml` and `nidoo-sound.toml`.
- Music input: TCP **4953**, raw **s16le / 48 000 Hz / 2 ch** (frame = 4 bytes), one sender at a time, newest connection wins.
- Codec on the wire: **`pcm`, 48 000 Hz, 2 ch, 16 bit** only (deliberate subset; spec says servers MUST also do FLAC — recorded as a known deviation). ESPHome YAML lists `codecs: [pcm]`.
- Connection direction: server-initiated only (deliberate subset; recorded as a known deviation).
- Chunk = **20 ms = 960 frames = 3 840 bytes** of PCM (+13-byte header), well under the 65 518-byte Noise limit: no fragmentation sending; receiving fragments is still handled (reject = close, per spec).
- Defaults: `lead_ms = 500`, `idle_stop_ms = 5000`, new player volume **30**, muted **false**, allowed **false**.
- Satellite identity = Sendspin `client_id` (43-char base64url Curve25519 public key). Topic-safe short id = first **12 hex chars of SHA-256(client_id)**.
- MQTT: discovery under `homeassistant/`, own topics under **`nidoo-sound/`** (not `nidoo/`, which the hub owns). Availability `nidoo-sound/status` `online`/`offline` (last will, retained).
- Server identity (Curve25519 keypair) persisted at `<data_dir>/sound/identity.key`, mode 0600; prefs at `<data_dir>/sound/players.json`, written atomically (temp + rename).
- Lint/test gate per task: `cargo clippy -p nidoo-sound --all-targets -- -D warnings` and `cargo test -p nidoo-sound`. Long runs: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`; stage explicit paths only.
- Any spike finding that changes the spec's scope (volume/mute not settable, board unstable, sync unusable, unpaired access refused) → **stop and report to the owner**.

## Review Focus

1. **The phone reconnects every 2 s after a Wi-Fi blip** — a new TCP connection while the old one is half-open must take over cleanly (old one dropped, no doubled audio, no stop/start of the stream if the gap is < 5 s). Test in Task 6: `second_connection_replaces_first_without_stream_restart`.
2. **A satellite reboots mid-song** — it re-advertises, the server reconnects, re-handshakes, and it rejoins on the running timeline at its saved volume/mute. Test in Task 7: `late_joiner_gets_stream_start_and_current_timeline`; Task 8: `prefs_applied_on_connect`.
3. **A new, unknown board appears on the network** — it must show up in Nidoo as not allowed and never receive audio until "Allowed" is switched on (spec: operator approval). Test in Task 7: `unapproved_player_gets_no_playback_activity`.
4. **Volume set in Nidoo while the board is off** — stored, echoed, applied on reconnect; a board-side change (client/state) updates Nidoo. Tests in Task 9: `command_for_offline_player_is_stored_and_echoed`, `client_state_change_publishes_new_volume`.
5. **The phone pauses (connected, no bytes)** — after 5 s the players get `stream/end`; when bytes return a fresh `stream/start` follows; gaps < 5 s are silence-filled so players never underrun. Tests in Task 6: `idle_five_seconds_stops`, `short_gap_is_silence_filled`.

---

## File Structure

```
satellite/
  satellite.yaml            ESPHome config (substitutions for pins/name)
  secrets.example.yaml      committed template; secrets.yaml git-ignored
  README.md                 wiring table, flash, add a satellite
crates/nidoo-sound/
  Cargo.toml
  LICENSE-SENDSPIN-SPEC.md
  README.md                 what it is, config keys, ports, deviations
  nidoo-sound.example.toml
  src/main.rs               clap CLI (`run`), tracing init, wires tasks
  src/lib.rs                pub mods
  src/config.rs             SoundConfig (figment: toml + NIDOO_SOUND_* env)
  src/clock.rs              Clock trait, MonotonicClock, FakeClock (test)
  src/proto/mod.rs
  src/proto/messages.rs     JSON message types (rc1 subset)
  src/proto/audio.rs        audio chunk encode, fragment reassembly
  src/proto/noise.rs        identity, PSK ids, KKpsk2 server, transport
  src/player.rs             one player connection (WebSocket + Noise + protocol)
  src/discover.rs           mDNS browse for _sendspin._tcp
  src/input.rs              PCM TCP input (newest wins, idle stop)
  src/pacer.rs              PCM → timestamped 20 ms chunks, silence fill, drift
  src/state.rs              per-player prefs, persisted
  src/group.rs              Group actor: players + pacer + prefs + events
  src/mqtt.rs               discovery payloads, command parsing, MQTT task
  tests/handshake.rs        fake player end-to-end over localhost
Dockerfile                  + `sound` stage/target
compose.yml                 + `sound` service, image nidoo-sound:local
docs/users/sound.md         user doc
.gitignore                  + satellite/secrets.yaml, satellite/.esphome/, nidoo-sound.toml
```

---

### Task 1: Satellite ESPHome config and board bring-up (hardware)

**Files:**
- Create: `satellite/satellite.yaml`, `satellite/secrets.example.yaml`, `satellite/README.md`
- Modify: `.gitignore`

**Interfaces:**
- Produces: a board on Wi-Fi advertising `_sendspin._tcp` with TXT `path`; device name = substitution `name` (default `nidoo-satellite`).

- [ ] **Step 1: Ask the owner** for Wi-Fi SSID/password (goes only into git-ignored `satellite/secrets.yaml`) and confirm the board is a XIAO ESP32S3 wired per the spec's "Reference wiring".
- [ ] **Step 2: Write `satellite.yaml`.** Board `esp32-s3-devkitc-1`, `variant: esp32s3`, framework `esp-idf`, flash 8MB; `psram: {mode: octal, speed: 80MHz}`; `wifi` from secrets; `logger`; `api:` **omitted** (project 2); `ota: platform: esphome`; `mdns`. Substitutions with the spec's default pins (mic `ws 1 / sck 2 / sd 4`, DAC `bck 7 / lrck 8 / din 9`). Two `i2s_audio` buses; `microphone: platform: i2s_audio` (INMP441: `adc_type: external`, `channel: left`, `bits_per_sample: 32bit`, `sample_rate: 16000`); `speaker: platform: i2s_audio` on the DAC (`dac_type: external`, 48000 Hz, 16 bit, stereo); `sendspin:` with `unpaired_access: true` and a media player/`media_source` routed to that speaker with `codecs: [pcm]`. Take exact keys from ESPHome 2026.10.2 docs/source (`esphome/components/sendspin/__init__.py`, `media_player/`), not from memory.
- [ ] **Step 3: Validate.** Run: `cd satellite && uvx esphome==2026.10.2 config satellite.yaml` — Expected: prints the resolved config, exit 0.
- [ ] **Step 4: Flash by USB.** Run: `uvx esphome==2026.10.2 run satellite.yaml --device /dev/cu.usbmodem*` — Expected: boot log shows Wi-Fi connected, PSRAM 8 MB found, i2s mic + speaker set up, sendspin started.
- [ ] **Step 5: Check discovery from the Mac.** Run: `dns-sd -B _sendspin._tcp local.` (Ctrl-C after 5 s) — Expected: one instance named after the board.
- [ ] **Step 6: Stability.** Leave it 10 min with `esphome logs`; Expected: no reset, free heap steady. Write the wiring table + flash steps in `satellite/README.md`.
- [ ] **Step 7: Commit** `satellite/` + `.gitignore` — `feat(satellite): ESPHome config for XIAO ESP32S3 sound satellites`.

### Task 2: Crate skeleton and wire types

**Files:**
- Create: `crates/nidoo-sound/{Cargo.toml,LICENSE-SENDSPIN-SPEC.md,src/lib.rs,src/main.rs,src/clock.rs,src/proto/mod.rs,src/proto/messages.rs,src/proto/audio.rs}`

**Interfaces:**
- Produces:
  - `clock::Clock: Send + Sync { fn now_us(&self) -> i64 }`; `MonotonicClock::new()` (µs since construction); `FakeClock { pub fn set(&self, us: i64); pub fn advance(&self, us: i64) }`.
  - `proto::messages::{ClientMsg, ServerMsg}` — serde enums `#[serde(tag = "type", content = "payload")]`, variant renames = wire names. `ClientMsg`: `Init`(`client/init`), `NoiseHandshake`(`noise/handshake`), `Hello`(`client/hello`), `Time`(`client/time`), `State`(`client/state`), `Goodbye`(`client/goodbye`), `Other` (catch-all, ignored). `ServerMsg`: `Init`(`server/init`), `Error`(`server/error`), `NoiseHandshake`, `Hello`(`server/hello`), `Activate`(`server/activate`), `Time`(`server/time`), `Command`(`server/command`), `StreamStart`(`stream/start`), `StreamEnd`(`stream/end`). Field sets exactly as in the rc1 README / player v1 sections; unknown fields ignored.
  - `proto::messages::AudioFormat { codec: String, sample_rate: u32, channels: u8, bit_depth: u8 }` and `PCM_48K_STEREO_16: AudioFormat`.
  - `proto::audio::encode_chunk(timestamp_us: i64, send_ahead_us: u32, data: &[u8]) -> Vec<u8>`; `proto::audio::Reassembler::push(&mut self, msg: &[u8]) -> Result<Option<(u8, Vec<u8>)>, ProtoError>`.

- [ ] **Step 1: Write failing tests** in `messages.rs` / `audio.rs`:

```rust
#[test] fn client_hello_parses_player_support() {
    // JSON copied from the rc1 README client/hello example + player@v1 support object
    let m: ClientMsg = serde_json::from_str(CLIENT_HELLO_EXAMPLE).unwrap();
    let ClientMsg::Hello(h) = m else { panic!() };
    assert_eq!(h.client_id.len(), 43);
    assert!(h.player_support().unwrap().supported_formats.contains(&PCM_48K_STEREO_16));
}
#[test] fn unknown_type_is_other() {
    let m: ClientMsg = serde_json::from_str(r#"{"type":"client/whatever","payload":{}}"#).unwrap();
    assert!(matches!(m, ClientMsg::Other));
}
#[test] fn audio_chunk_layout() {
    let b = encode_chunk(0x0102030405060708, 500_000, &[9, 9]);
    assert_eq!(b, [4, 1,2,3,4,5,6,7,8, 0,0x07,0xA1,0x20, 9,9]);
}
#[test] fn fragments_reassemble_and_bad_sequences_error() { /* first[1,0b10,4,..] + last[1,0b01,..] → Some((4, data)); last-without-first → Err; reserved flag bit → Err; orig_type 1 → Err */ }
```

- [ ] **Step 2: Run** `cargo test -p nidoo-sound` — Expected: FAIL (types missing).
- [ ] **Step 3: Implement** the types above; `Cargo.toml` deps: tokio, tokio-tungstenite, futures-util, serde, serde_json, snow (features `default-resolver`, ChaChaPoly + AES-GCM), sha2, base64, rand, mdns-sd, rumqttc, figment, clap, tracing, tracing-subscriber, anyhow, thiserror, url, bytes; dev: `nidoo-core` (Task 9). `main.rs` = clap `run` subcommand stub. Copy the spec `LICENSE.md` at `1.0.0-rc1` verbatim.
- [ ] **Step 4: Run** tests + clippy gate — Expected: PASS, no warnings.
- [ ] **Step 5: Commit** — `feat(sound): nidoo-sound crate with Sendspin wire types`.

### Task 3: Noise layer (identity, PSK ids, KKpsk2 server)

**Files:**
- Create: `crates/nidoo-sound/src/proto/noise.rs`

**Interfaces:**
- Consumes: none.
- Produces:
  - `SENTINEL_PSK: [u8; 32]`, `fn psk_id(psk: &[u8; 32]) -> String` (base64url no pad of SHA-256(`"sendspin-psk-id-v1"` ‖ psk)).
  - `enum Suite { ChaChaPoly, AesGcm }` + `Suite::from_wire(&str) -> Option<Suite>` (`25519_ChaChaPoly_SHA256`, `25519_AESGCM_SHA256`).
  - `struct ServerIdentity { pub server_id: String, .. }`; `ServerIdentity::load_or_create(path: &Path) -> anyhow::Result<Self>` (creates parent dir, writes 32-byte private key, mode 0600).
  - `NoiseServer::start(id: &ServerIdentity, client_id: &str, suite: Suite, prologue: &[u8], psk: &[u8; 32], psk_category: &str) -> anyhow::Result<(NoiseServer, Vec<u8>)>` → message 1 (payload JSON `{"psk_id":…,"psk_category":…}`); `NoiseServer::finish(self, msg2: &[u8]) -> anyhow::Result<NoiseTransport>` (payload must be the bytes `{}`).
  - `NoiseTransport::{encrypt(&mut self, plain: &[u8]) -> anyhow::Result<Vec<u8>>, decrypt(&mut self, cipher: &[u8]) -> anyhow::Result<Vec<u8>>}`.

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn sentinel_constants_match_spec() {
    assert_eq!(hex::encode(SENTINEL_PSK), "1b5e24dbc1aed95fc2a5a338a90c05df44bd10f5ec1f4cd66cbf86272767b9d3");
    assert_eq!(psk_id(&SENTINEL_PSK), "GFsV9tLaSQm9HcFWpKsgYQOr7wFTvNUtkmFwuVz3zoo");
}
#[test] fn identity_persists_and_is_private() { /* load_or_create twice → same server_id (43 chars); file mode & 0o777 == 0o600 */ }
#[test] fn kkpsk2_round_trip_with_snow_responder() {
    // test-side responder: snow Builder "Noise_KKpsk2_25519_ChaChaPoly_SHA256", local=client key, remote=server pub, psk(2, SENTINEL)
    // server.start → msg1 → responder reads, payload has psk_id == sentinel id and psk_category "sn" → responder writes "{}" → server.finish
    // then encrypt/decrypt both ways; also for Suite::AesGcm
}
#[test] fn wrong_prologue_fails() { /* responder with different prologue → finish() Err */ }
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** with `snow` (`Noise_KKpsk2_25519_<cipher>_SHA256`, initiator = server, remote static = decoded `client_id`, `psk(2, psk)`, prologue = exact bytes of `client/init` then `server/init`).
- [ ] **Step 4: Run** tests + clippy — PASS.
- [ ] **Step 5: Commit** — `feat(sound): Noise KKpsk2 server side with the Sentinel PSK`.

### Task 4: Player connection (handshake, time, state, commands)

**Files:**
- Create: `crates/nidoo-sound/src/player.rs`, `crates/nidoo-sound/tests/handshake.rs`

**Interfaces:**
- Consumes: Task 2 messages/audio, Task 3 noise, `Clock`.
- Produces:
  - `pub struct PlayerInfo { pub client_id: String, pub name: String, pub supported_formats: Vec<AudioFormat>, pub buffer_capacity: u32, pub supported_commands: Vec<String> }`
  - `pub enum PlayerEvent { Connected(PlayerInfo), State { client_id: String, volume: Option<u8>, muted: Option<bool>, supported_commands: Vec<String>, min_buffer_ms: Option<u32> }, Gone { client_id: String } }`
  - `pub enum PlayerCmd { Activate { playback: bool }, Volume(u8), Mute(bool), StreamStart(AudioFormat), Audio { timestamp_us: i64, data: bytes::Bytes }, StreamEnd, Close }`
  - `pub async fn run_player(url: url::Url, identity: Arc<ServerIdentity>, clock: Arc<dyn Clock>, events: mpsc::Sender<PlayerEvent>, cmds: mpsc::Receiver<PlayerCmd>) -> anyhow::Result<()>` — connects (no `Origin` header), runs init → Noise → hello → waits for `PlayerCmd::Activate` before sending `server/activate` (roles `player@v1`; activities `["playback"]` only when `playback: true`, else empty), answers every `client/time` with `server/time` immediately (receive stamp taken on arrival, transmit stamp just before encryption), forwards `client/state`, sends `server/command` only for commands listed in `supported_commands`, stamps `send_ahead = timestamp - now` at send time (saturating 0 / u32::MAX), never exceeds `buffer_capacity` (header 13 + payload accounting; chunks whose timestamp has passed are released), sends `Gone` on any exit.

- [ ] **Step 1: Failing integration test** `tests/handshake.rs` with an in-test fake player (tokio-tungstenite server on 127.0.0.1:0 at `/sendspin` + the snow responder from Task 3):

```rust
#[tokio::test] async fn handshake_then_hello_then_activate() {
    // fake sends client/init {version:1, client_id, suite}, checks server/init + msg1, answers msg2,
    // receives encrypted server/hello, sends client/hello (player@v1, pcm 48k/2/16, buffer_capacity 400_000),
    // expects PlayerEvent::Connected with name/client_id; after PlayerCmd::Activate{playback:true}
    // receives server/activate whose activities contain "playback" and active roles contain "player@v1"
}
#[tokio::test] async fn time_requests_are_answered() { /* client/time{client_transmitted:42} → server/time echoes 42, server_received <= server_transmitted */ }
#[tokio::test] async fn volume_command_only_when_supported() { /* supported_commands ["mute"] → PlayerCmd::Volume(10) sends nothing; ["volume"] → server/command {player:{command:"volume",volume:10}} */ }
#[tokio::test] async fn buffer_capacity_is_respected() { /* capacity 3*3853 bytes, 5 chunks queued with future timestamps → only 3 sent until FakeClock passes the first timestamp */ }
#[tokio::test] async fn origin_header_absent_and_goodbye_ends_with_gone() { /* fake asserts no Origin; client/goodbye → PlayerEvent::Gone */ }
```

- [ ] **Step 2: Run** `cargo test -p nidoo-sound --test handshake` — FAIL.
- [ ] **Step 3: Implement** `run_player` as one task with a `select!` over WebSocket reads, `cmds`, and a buffer-release timer.
- [ ] **Step 4: Run** tests + clippy — PASS.
- [ ] **Step 5: Commit** — `feat(sound): player connection with handshake, clock answers and commands`.

### Task 5: Spike gate on the real board (hardware)

**Files:**
- Create: `crates/nidoo-sound/src/discover.rs`; `src/main.rs` gains a hidden `tone` subcommand
- Create: `docs/superpowers/plans/2026-11-04-satellite-sound-spike-notes.md`

**Interfaces:**
- Produces: `pub struct Discovered { pub instance: String, pub url: url::Url }`; `pub fn browse() -> anyhow::Result<mpsc::Receiver<DiscoveryEvent>>` with `enum DiscoveryEvent { Up(Discovered), Down { instance: String } }` (mdns-sd, `_sendspin._tcp.local.`, URL `ws://<ipv4>:<port><txt path>`, skip loopback/unspecified, skip paths not starting with `/`).
- `nidoo-sound tone --seconds 10 --volume 30` connects to every discovered player, activates playback, sends `stream/start` (PCM 48k/2/16) and a 440 Hz tone with `lead_ms = 500`, then sets volume, mute on, mute off, and `stream/end`.

- [ ] **Step 1: Unit test** `browse` URL building: `url_from_service("10.0.0.5", 8928, "/sendspin") == "ws://10.0.0.5:8928/sendspin"`; `"127.0.0.1"` → None; path `"x"` → None.
- [ ] **Step 2: Implement** `discover.rs` and the `tone` subcommand; run unit tests.
- [ ] **Step 3: Run on the Mac** with the board from Task 1: `cargo run -p nidoo-sound -- tone --seconds 10 --volume 30` — Expected: tone audible, volume/mute changes audible, board log shows the server connected and no reset.
- [ ] **Step 4: Record findings** in the spike notes: connection direction, handshake differences vs rc1, `client/hello` contents (client_id, formats, buffer_capacity, supported_commands), whether `client/state` reports volume after a command, whether mic + playback ran 10 min without reset (heap), and any sendspin-cpp 0.9.3 vs rc1 deviations.
- [ ] **Step 5: Gate.** If volume/mute are not in `supported_commands`, unpaired access is refused, or the board resets: **stop and report to the owner** before Task 6.
- [ ] **Step 6: Commit** — `feat(sound): mDNS discovery and a test-tone spike against the satellite`.

### Task 6: PCM input and pacer

**Files:**
- Create: `crates/nidoo-sound/src/input.rs`, `crates/nidoo-sound/src/pacer.rs`

**Interfaces:**
- Produces:
  - `pub enum InputEvent { Started, Pcm(bytes::Bytes), Stopped }`
  - `pub async fn run_pcm_input(listener: tokio::net::TcpListener, idle_stop: Duration, out: mpsc::Sender<InputEvent>) -> anyhow::Result<()>` — newest connection replaces the current one without emitting `Stopped`/`Started`; TCP close of the current connection → `Stopped`; no bytes for `idle_stop` → `Stopped`, bytes again → `Started`.
  - `pub struct Pacer` — `Pacer::new(lead_us: i64)`, `fn start(&mut self, now_us: i64)`, `fn push(&mut self, pcm: &[u8])`, `fn due_chunks(&mut self, now_us: i64) -> Vec<Chunk>`, `fn stop(&mut self)`; `pub struct Chunk { pub timestamp_us: i64, pub data: bytes::Bytes }` (always 3 840 bytes). Timeline: first chunk at `now + lead`, each next `+20 000 µs`; a chunk is due when `timestamp - lead <= now`; if no PCM is buffered when due, emit silence; keep at most 3 chunks of PCM queued beyond the timeline — drop one frame (4 bytes) per chunk when above, which absorbs phone/server drift. Odd byte counts are carried to the next push (never split a frame).

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn timeline_starts_lead_ahead_and_steps_20ms() { /* start(1_000_000); push 7680 bytes; due_chunks(1_000_000) → [ts 1_500_000]; due_chunks(1_020_000) → [ts 1_520_000] */ }
#[test] fn short_gap_is_silence_filled() { /* no push; due_chunks advances → chunks of 3840 zero bytes */ }
#[test] fn excess_buffer_drops_frames() { /* push 10 chunks at once; queue shrinks by 4 bytes per emitted chunk until <= 3 chunks */ }
#[test] fn partial_frames_carry_over() { /* push 3 bytes then 1 → one whole frame */ }
#[tokio::test] async fn idle_five_seconds_stops() { /* paused time: connect, send bytes → Started, Pcm; advance 5 s → Stopped; send → Started */ }
#[tokio::test] async fn second_connection_replaces_first_without_stream_restart() { /* A connects/sends; B connects/sends → no Stopped; A's later bytes ignored */ }
#[tokio::test] async fn close_stops_immediately() { /* drop the client socket → Stopped */ }
```

- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** tests + clippy — PASS.
- [ ] **Step 5: Commit** — `feat(sound): phone PCM input and the 20 ms pacer`.

### Task 7: Group actor (fan-out, approval, late join)

**Files:**
- Create: `crates/nidoo-sound/src/group.rs`

**Interfaces:**
- Consumes: Tasks 4–6, Task 8's `StateStore` (implement Task 8 first if executing out of order — `group.rs` only calls `get`, `observe`).
- Produces:
  - `pub enum Control { SetVolume { short_id: String, volume: u8 }, SetMute { short_id: String, muted: bool }, SetAllowed { short_id: String, allowed: bool } }`
  - `pub enum Published { Player { short_id: String, prefs: PlayerPrefs, connected: bool } }` (sent to the MQTT task on every change)
  - `pub struct Group` with `pub async fn run(self, discovery: mpsc::Receiver<DiscoveryEvent>, input: mpsc::Receiver<InputEvent>, control: mpsc::Receiver<Control>, publish: mpsc::Sender<Published>) -> anyhow::Result<()>` — spawns `run_player` per discovered player (reconnect with 2 s backoff while advertised), sends `Activate{playback: prefs.allowed}`; on `Connected` applies saved volume/mute; while input is `Started`, each pacer tick (every 20 ms) sends `Audio` to every allowed+connected player; a player that connects mid-stream gets `StreamStart` then chunks from the current timeline only; `Stopped` → `StreamEnd` to all; `SetAllowed` true → `Activate{playback:true}` (+ `StreamStart` if playing), false → `Activate{playback:false}`.

- [ ] **Step 1: Failing tests** (players simulated by channel pairs, `FakeClock`):

```rust
#[tokio::test] async fn unapproved_player_gets_no_playback_activity() { /* new client_id → Activate{playback:false}; no Audio while input Started */ }
#[tokio::test] async fn approved_players_get_identical_timestamps() { /* two allowed players → same Chunk.timestamp_us sequence */ }
#[tokio::test] async fn late_joiner_gets_stream_start_and_current_timeline() { /* join after 1 s → StreamStart, first Audio ts >= now+lead and on the 20 ms grid */ }
#[tokio::test] async fn input_stopped_sends_stream_end() {}
#[tokio::test] async fn allowing_mid_stream_starts_that_player() {}
```

- [ ] **Step 2–4:** run (FAIL) → implement → run + clippy (PASS).
- [ ] **Step 5: Commit** — `feat(sound): group actor streams in sync to approved satellites`.

### Task 8: Persisted per-player prefs

**Files:**
- Create: `crates/nidoo-sound/src/state.rs`

**Interfaces:**
- Produces: `pub struct PlayerPrefs { pub client_id: String, pub name: String, pub volume: u8, pub muted: bool, pub allowed: bool }` (Serialize/Deserialize); `pub fn short_id(client_id: &str) -> String`; `pub struct StateStore` with `load(path: PathBuf) -> anyhow::Result<Self>` (missing file = empty), `get(&self, client_id) -> Option<&PlayerPrefs>`, `by_short(&self, short_id) -> Option<&PlayerPrefs>`, `observe(&mut self, client_id, name) -> &PlayerPrefs` (creates with defaults 30/false/false, updates name), `update(&mut self, short_id, f: impl FnOnce(&mut PlayerPrefs)) -> anyhow::Result<Option<PlayerPrefs>>` (saves atomically), `all(&self) -> impl Iterator<Item=&PlayerPrefs>`.

- [ ] **Step 1: Failing tests:** `defaults_for_new_player` (30, false, false); `short_id_is_12_hex` (`short_id("AAAA…")` == first 12 hex of SHA-256); `save_and_reload_round_trip`; `corrupt_file_is_an_error_not_a_reset` (load returns Err; the file is left untouched); `prefs_applied_on_connect` (in `group.rs` tests: saved volume 70/muted true → `PlayerCmd::Volume(70)`, `PlayerCmd::Mute(true)` after `Connected`).
- [ ] **Step 2–4:** FAIL → implement → PASS + clippy.
- [ ] **Step 5: Commit** — `feat(sound): remember each satellite's volume, mute and approval`.

### Task 9: MQTT link to Nidoo

**Files:**
- Create: `crates/nidoo-sound/src/mqtt.rs`

**Interfaces:**
- Consumes: `Published`, `Control` (Task 7), `PlayerPrefs`, `short_id` (Task 8), `MqttSettings` (Task 10 — define the struct here first if Task 10 is not done yet; Task 10 only adds loading).
- Produces:
  - `pub fn discovery_messages(p: &PlayerPrefs, discovery_prefix: &str) -> Vec<(String, String)>` — retained configs:
    - `homeassistant/number/nidoo_sound_<sid>/volume/config`: name "Volume", `unique_id nidoo_sound_<sid>_volume`, min 0, max 100, step 1, mode `slider`, `state_topic nidoo-sound/<sid>/volume`, `command_topic nidoo-sound/<sid>/volume/set`.
    - `homeassistant/switch/nidoo_sound_<sid>/mute/config`: name "Mute", `…/mute`, `…/mute/set`, payloads `ON`/`OFF`.
    - `homeassistant/switch/nidoo_sound_<sid>/allowed/config`: name "Allowed", `entity_category config`, `…/allowed`, `…/allowed/set`.
    - `homeassistant/binary_sensor/nidoo_sound_<sid>/connected/config`: name "Connected", `device_class connectivity`, `entity_category diagnostic`, `…/connected`.
    - Every config: `availability_topic nidoo-sound/status`, `device { identifiers: ["nidoo_sound_<sid>"], name: <player name>, model: "Sound satellite", manufacturer: "Nidoo" }`.
  - `pub fn state_messages(p: &PlayerPrefs, connected: bool) -> Vec<(String, String)>` (retained).
  - `pub fn parse_command(topic: &str, payload: &[u8]) -> Option<Control>` (volume 0–100 integer or float rounded/clamped; `ON`/`OFF`; anything else → None).
  - `pub async fn run_mqtt(cfg: &MqttSettings, publish: mpsc::Receiver<Published>, control: mpsc::Sender<Control>) -> anyhow::Result<()>` — last will `nidoo-sound/status offline` retained, `online` on connect, subscribe `nidoo-sound/+/+/set`, republish all discovery + state on every (re)connect and on `homeassistant/status online`.

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn discovery_parses_in_the_hub() {
    // for each (topic, payload): nidoo_core::discovery::classify(topic, &["homeassistant".into()], &[]) → discovery topic;
    // nidoo_core::discovery::parse(..) → Parsed::Entity with kinds Number, Switch, Switch, BinarySensor and the unique_ids above
}
#[test] fn volume_command_parses_and_clamps() { /* "45" → SetVolume 45; "120" → 100; "abc" → None */ }
#[test] fn mute_and_allowed_commands_parse() {}
#[tokio::test] async fn command_for_offline_player_is_stored_and_echoed() { /* group: SetVolume for a disconnected player → prefs saved, Published with connected:false and new volume */ }
#[tokio::test] async fn client_state_change_publishes_new_volume() { /* PlayerEvent::State{volume:Some(12)} → prefs 12 saved, Published */ }
```

- [ ] **Step 2–4:** FAIL → implement (add `nidoo-core` as a dev-dependency) → PASS + clippy.
- [ ] **Step 5: Commit** — `feat(sound): satellites appear in Nidoo with volume, mute and approval`.

### Task 10: Config and the `run` command

**Files:**
- Create: `crates/nidoo-sound/src/config.rs`, `crates/nidoo-sound/nidoo-sound.example.toml`
- Modify: `crates/nidoo-sound/src/main.rs`, `.gitignore`

**Interfaces:**
- Produces: `pub struct SoundConfig { pub mqtt: MqttSettings, pub input_port: u16 /*4953*/, pub data_dir: PathBuf /*./data*/, pub lead_ms: u32 /*500*/, pub idle_stop_ms: u32 /*5000*/, pub discovery_prefix: String /*"homeassistant"*/ }`; `pub struct MqttSettings { pub host: String /*localhost*/, pub port: u16 /*1883*/, pub user: Option<String>, pub password: Option<String>, pub client_id: String /*"nidoo-sound"*/ }`; `SoundConfig::load(path: Option<&Path>) -> anyhow::Result<Self>` (defaults ← toml file (default `nidoo-sound.toml`, optional) ← env `NIDOO_SOUND_*`, `__` for nesting, e.g. `NIDOO_SOUND_MQTT__HOST`).
- `nidoo-sound run [--config PATH]` wires: identity, StateStore, discovery, PCM input listener on `0.0.0.0:<input_port>`, Group, MQTT; Ctrl-C/SIGTERM → `stream/end` to all, `offline` published, exit 0.

- [ ] **Step 1: Failing tests:** `defaults_without_file`; `env_overrides_file` (temp toml with `input_port = 5000` + env `NIDOO_SOUND_INPUT_PORT=6000` → 6000); `mqtt_nested_env` (`NIDOO_SOUND_MQTT__HOST=broker` → "broker").
- [ ] **Step 2–4:** FAIL → implement → PASS + clippy. Then `cargo run -p nidoo-sound -- run` against a local broker + the board + Oikos Cast from the phone: music plays; entities appear in the dev hub.
- [ ] **Step 5: Commit** — `feat(sound): config and the run command`.

### Task 11: Docker, compose, docs

**Files:**
- Modify: `Dockerfile` (new `sound-builder` + `sound` stages), `compose.yml` (service `sound`), `.github/workflows/ci.yml` only if `crates/*` is not already covered (it is: workspace members glob)
- Create: `docs/users/sound.md`, `crates/nidoo-sound/README.md`

**Interfaces:**
- Produces: `docker build --target sound -t nidoo-sound:local .`; compose service `sound` (`image: nidoo-sound:local`, `build: {context: ., target: sound}`, `network_mode: host`, `restart: unless-stopped`, `volumes: ./data:/data`, `env_file: .env` optional, command `run`), **no `depends_on` on hub**; the hub's build stays `cargo build --release -p nidoo` so hub rebuilds never touch `nidoo-sound:local`.

- [ ] **Step 1: Build:** `docker build --target sound -t nidoo-sound:local .` — Expected: image built; `docker run --rm nidoo-sound:local --help` prints usage.
- [ ] **Step 2: Check isolation:** `docker compose config` shows `sound` without `depends_on`; `docker compose up -d --build hub` (dry: `docker compose up --no-start --build hub` in CI-free local check) does not recreate `sound`.
- [ ] **Step 3: Docs.** `docs/users/sound.md`: what it is, ports (4953/TCP in, outgoing WebSocket to satellites, mDNS 5353/UDP), turning off snapserver first, flashing a satellite (`satellite/README.md`), switching **Allowed** on in Nidoo, assigning the room, known deviations (PCM only, server-initiated only, unpaired access/Sentinel = no MITM protection on the LAN). Crate README: config keys + env names.
- [ ] **Step 4: Commit** — `build(sound): docker target and compose service for nidoo-sound`.

### Task 12: Acceptance on two boards (hardware)

**Files:**
- Modify: spike notes (results section); `satellite/README.md` if anything changed

- [ ] **Step 1:** Flash the second board (`substitutions: name: nidoo-satellite-2` via `--substitution` or a per-board file under git-ignored `satellite/boards/`). Both appear in Nidoo; switch **Allowed** on for both.
- [ ] **Step 2:** Cast from the phone. Expected: both rooms play; standing between them, no audible echo.
- [ ] **Step 3:** Volume/mute from a Nidoo dashboard on each; reboot one board → it rejoins in sync at its saved level; restart `nidoo-sound` → phone reconnects within ~2 s, levels restored; `docker compose up -d --build hub` → music never stops.
- [ ] **Step 4:** Soak 2 h casting: no dropouts, no board resets (ESPHome logs: free heap/PSRAM steady). Record results and numbers in the spike notes.
- [ ] **Step 5: Commit** notes — `docs(sound): satellite acceptance results`.
