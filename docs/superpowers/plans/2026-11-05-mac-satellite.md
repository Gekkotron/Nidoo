# Mac Satellite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A menu-bar Mac app, "Nidoo Satellite", that is a full house satellite: synchronized Sendspin music, "Hey Jarvis" voice over the signed UDP protocol, a Listening toggle and Quit.

**Architecture:** All protocol and decision logic lives in a platform-neutral library crate `crates/nidoo-satellite` (in the workspace, tested on Linux CI, end-to-end against the real `nidoo-sound` player code and a fake hub socket). The voice datagrams move out of the hub into a tiny shared crate `crates/nidoo-voice`. The macOS shell (`satellite/mac/`, its own Cargo workspace, not built by CI) only adds the microphone and speaker (`cpal`), the wake-word engine, the menu-bar icon (`tray-icon`) and the `.app` bundle.

**Tech Stack:** Rust 2024, tokio, tokio-tungstenite, snow (Noise KKpsk2), mdns-sd, hmac/sha2, cpal, tray-icon, a wake-word engine chosen in Task 1.

**Spec:** `docs/superpowers/specs/2026-11-05-mac-satellite-design.md`

## Global Constraints

- Nothing on the hub, in `nidoo-sound` or in the voice protocol changes behavior. Task 2 only moves code. If anything else seems needed, stop and ask.
- The mic is open only while **Listening** is on **and** the hub greeted us within the last **15 s**. Audio leaves the Mac only inside a window: wake word **8 s**, follow-up **6 s**.
- Voice datagrams go to the address **inside the Hello**, never to a datagram's source. The satellite listens on UDP **4961**.
- Sendspin: `player@v1`, PCM **48 kHz stereo 16-bit** (`PCM_48K_STEREO_16`), Noise `KKpsk2` with the Sentinel PSK, the satellite as **responder**. mDNS `_sendspin._tcp`, TXT `path=/sendspin`.
- The Sendspin `client_id` is the base64url public key of a Curve25519 keypair made once and kept. `sid = short_id(client_id)` (first 6 bytes of SHA-256, hex), the same as `nidoo_sound::state::short_id`.
- Stored in `~/Library/Application Support/Nidoo Satellite/`, files mode `0600`. The voice key is never logged.
- Player name is the Mac's computer name. Bundle id `io.github.gekkotron.nidoo-satellite`. Attribution "Gekkotron".
- The repo is public and generic: no owner rooms, hosts, IPs or names in tracked files.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` stay green on Linux. Run cargo with `CARGO_PROFILE_DEV_DEBUG=0`. No per-repo cargo config.

## Review Focus

1. **Mac sleeps, or the Wi-Fi changes.** Expected: the icon goes gray within 15 s and returns to normal by itself on the next Hello. A new Sendspin connection is accepted after the old one died. Tests: Task 4 (`greeted_expires_after_15s_and_comes_back`) and Task 5 (`accepts_a_new_connection_after_the_first_drops`).
2. **Listening switched off in the middle of a window.** Expected: **End** is sent, the mic closes, and the music comes back. Test: Task 7 (`listening_off_mid_window_ends_it`).
3. **Junk on the clipboard for "Set voice key…".** Expected: refused with a clear status, and the old key is kept. Test: Task 3 (`parse_voice_key_*`, `set_voice_key_keeps_old_on_junk`).
4. **Replayed, foreign or stale hub datagrams** (another sid, `hub_ctr` not rising, Stop for another session). Expected: ignored. Test: Task 4 (`ignores_*`).
5. **Late, overlapping or missing audio chunks, a stream end mid-buffer, volume 0.** Expected: silence, never noise or a stale replay. Test: Task 6 (`late_chunks_are_dropped`, `gap_plays_silence`, `stream_end_clears`).

---

### Task 1: Wake-word spike (gate)

Throwaway. Its output is a decision and a small `WakeDetector` implementation for Task 8.

**Files:**
- Create: `satellite/mac/spike/` (own `Cargo.toml` with an empty `[workspace]`, `src/main.rs`)
- Create: `docs/superpowers/plans/2026-11-05-mac-satellite-wake-spike-notes.md`

- [ ] **Step 1: Record test clips.** Record 16 kHz mono WAVs on the Mac's built-in mic: 10 × "Hey Jarvis" at desk distance, 10 × at ~3 m, 5 × "Hey Jarvis" over music playing from the Mac's speakers, and 5 min of ordinary talk/TV with no wake word. Keep them out of git (`satellite/mac/spike/clips/`, ignored).
- [ ] **Step 2: Try candidates in this order** and stop at the first that passes: (a) sherpa-onnx keyword spotting (`sherpa-onnx = "=2.0.1"`, already the hub's version) with an English KWS model and the keyword "HEY JARVIS"; (b) openWakeWord `hey_jarvis` ONNX (melspectrogram + embedding + classifier) through `ort`. Also note the model licence: anything non-commercial must be downloaded at first run, never committed.
- [ ] **Step 3: Measure.** The binary `wake-spike <threshold> <wav…>` prints the detections per file, plus real-time factor on one core.
Pass: ≥ 8/10 near, ≥ 6/10 at 3 m, ≤ 1 false trigger in 5 min, RTF < 0.2.
- [ ] **Step 4: Live check.** `wake-spike --live` on the default mic prints "WAKE" on detection.
- [ ] **Step 5: Write the notes** (engine, model URLs + SHA-256, licence, threshold, numbers) and commit them.

```bash
git add docs/superpowers/plans/2026-11-05-mac-satellite-wake-spike-notes.md satellite/mac/spike satellite/.gitignore
git commit -m "spike(satellite-mac): wake word on a Mac CPU"
```

**GATE:** if no candidate passes, stop here and report to the owner (spec: voice is the point; no music-only v1 is assumed).

---

### Task 2: Extract the voice datagrams into `nidoo-voice`

**Files:**
- Create: `crates/nidoo-voice/Cargo.toml`, `crates/nidoo-voice/src/lib.rs` (the contents of `crates/nidoo/src/assist/stt/packet.rs`, tests included, unchanged)
- Modify: `crates/nidoo/src/assist/stt/packet.rs` → only `pub use nidoo_voice::*;`
- Modify: `crates/nidoo/Cargo.toml` (add `nidoo-voice = { path = "../nidoo-voice" }`)

**Interfaces:**
- Produces: `nidoo_voice::{Key, HubMsg, BoardMsg, WakeKind, PacketError, encode_hub, decode_hub, encode_board, decode_board, MAX_SAMPLES, SID_LEN}`, same signatures as today.

- [ ] **Step 1:** Move the file, keeping only the deps it uses (`hmac 0.12`, `sha2 0.10`, `thiserror`, plus whatever `Key::generate` uses).
- [ ] **Step 2:** Run `cargo test -p nidoo-voice` (test vectors pass) and `cargo test -p nidoo stt` (the hub still passes). Then run `cargo clippy -p nidoo --no-default-features --all-targets -- -D warnings`.
- [ ] **Step 3: Commit** `refactor(voice): voice datagrams in their own crate`.

---

### Task 3: `nidoo-satellite` crate, identity and settings store

**Files:**
- Create: `crates/nidoo-satellite/Cargo.toml` (lib only; deps: `nidoo-voice`, `nidoo-sound` (for `proto::audio`, `proto::noise` constants, `state::short_id`), tokio, tokio-tungstenite, futures-util, snow, x25519-dalek, mdns-sd, serde, serde_json, base64, rand, anyhow, thiserror, tracing, bytes; dev: tempfile)
- Create: `crates/nidoo-satellite/src/lib.rs`, `src/store.rs`
- Test: in `src/store.rs`

**Interfaces:**
- Produces:
  - `pub struct Store` with `Store::open(dir: &Path) -> anyhow::Result<Store>`, `fn identity(&self) -> &PlayerIdentity`, `fn settings(&self) -> &Settings`, `fn set_listening(&mut self, on: bool) -> anyhow::Result<()>`, `fn set_voice_key(&mut self, text: &str) -> Result<(), KeyError>`
  - `pub struct PlayerIdentity { pub client_id: String, pub sid: String, /* private */ }` with `fn private_key(&self) -> [u8; 32]`. Its file is `player.key` (32 raw bytes, `0600`).
  - `pub struct Settings { pub voice_key: Option<nidoo_voice::Key>, pub listening: bool /* default true */, pub wake_threshold: Option<f32> }`. Its file is `settings.json` (`0600`, the key as hex).
  - `pub fn parse_voice_key(text: &str) -> Option<nidoo_voice::Key>`
  - `pub enum KeyError { NotAKey }`

- [ ] **Step 1: Failing tests**

```rust
#[test] fn identity_is_stable_across_opens() // open twice in one tempdir → same client_id and sid; sid == nidoo_sound::state::short_id(&client_id)
#[test] fn files_are_0600()                  // player.key and settings.json mode & 0o777 == 0o600
#[test] fn parse_voice_key_accepts_bare_hex_and_yaml_line()
    // 64 hex → Some; `voice_key: "<64 hex>"` → Some; surrounding whitespace/newline → Some; uppercase hex → Some
#[test] fn parse_voice_key_rejects_junk()     // "", 63 hex, 65 hex, "hello", a URL → None
#[test] fn set_voice_key_keeps_old_on_junk()  // set valid, then set "junk" → Err(NotAKey), settings().voice_key still the first
#[test] fn listening_survives_reopen()        // set_listening(false), reopen → false; fresh store → true
```

- [ ] **Step 2:** Run `cargo test -p nidoo-satellite store`. Expected: FAIL to compile.
- [ ] **Step 3:** Implement. Use `create_new` + mode `0o600` as in `nidoo_sound::proto::noise::write_private`. Settings are written atomically (temp file + rename, same mode).
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(satellite): identity and settings store`.

---

### Task 4: `VoiceLink`, the board side of the voice protocol (no I/O)

**Files:**
- Create: `crates/nidoo-satellite/src/voice.rs`

**Interfaces:**
- Consumes: `nidoo_voice::*` (Task 2).
- Produces:
  - `pub struct VoiceLink` with `VoiceLink::new(key: Key, sid: String)`
  - `fn on_datagram(&mut self, bytes: &[u8], now: Instant) -> Option<VoiceEvent>`
  - `fn greeted(&self, now: Instant) -> bool`. True if a valid Hello came within `GREETING_TTL = 15 s`.
  - `fn hub(&self) -> Option<SocketAddrV4>`. The address from the last Hello.
  - `fn open(&mut self, kind: WakeKind, listen_ctr: u64, now: Instant) -> Option<Vec<u8>>`. Returns the Wake datagram, or None if not greeted or a window is already open. The window lasts 8 s for `WakeWord` and 6 s for `FollowUp`.
  - `fn audio(&mut self, samples: &[i16]) -> Vec<Vec<u8>>`. Audio datagrams of ≤ `MAX_SAMPLES` samples each, with `seq` rising. Returns an empty list when no window is open.
  - `fn tick(&mut self, now: Instant) -> Option<Vec<u8>>`. Once the deadline passes, closes the window and returns End.
  - `fn close_local(&mut self) -> Option<Vec<u8>>`. Closes the open window and returns End.
  - `fn window_open(&self) -> bool`
  - `pub enum VoiceEvent { Greeted, Stop, Listen { listen_ctr: u64 } }`

Rules, from `docs/reference/voice-protocol.md`:
- Ignore a datagram whose tag fails, whose sid isn't ours, or whose `hub_ctr` is ≤ the last one accepted.
- A Wake echoes the latest Hello `nonce`. `session` starts at 1 and rises for the life of the process.
- `Stop` counts only for the open session. It closes the window and sends no End.
- `Listen` counts only when no window is open. It yields `listen_ctr = hub_ctr`.

- [ ] **Step 1: Failing tests** (build hub datagrams with `encode_hub`, read ours with `decode_board`)

```rust
#[test] fn no_wake_before_hello()                       // open(..) == None
#[test] fn wake_echoes_latest_nonce_and_sessions_rise() // two Hellos (nonce 5 then 9) → Wake{nonce:9, session:1}; after close, next Wake session 2
#[test] fn streams_to_hello_address_not_source()        // hub() == the Hello's hub field
#[test] fn audio_splits_at_640_with_rising_seq()        // 1500 samples → 3 datagrams, 640/640/220, seq 1,2,3 (the board starts each window at 1: `satellite/nidoo_voice.h`)
#[test] fn stop_closes_only_its_session()               // Stop{session: other} ignored; Stop{session: current} → Some(Stop), window closed, audio() empty
#[test] fn window_times_out_with_end()                   // tick at +8 s → End{session}; follow-up at +6 s
#[test] fn listen_opens_follow_up()                     // Listen{hub_ctr: 77} → Listen{listen_ctr:77}; open(FollowUp, 77) → Wake{kind: FollowUp, listen_ctr: 77}
#[test] fn ignores_wrong_key_other_sid_and_stale_ctr()  // each returns None and changes nothing
#[test] fn greeted_expires_after_15s_and_comes_back()
```

- [ ] **Step 2:** Run `cargo test -p nidoo-satellite voice`. Expected: FAIL.
- [ ] **Step 3:** Implement. `seq` restarts at 1 for every window.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(satellite): voice link state machine`.

---

### Task 5: Sendspin player side (Noise responder, messages, clock, server, mDNS)

**Files:**
- Create: `crates/nidoo-satellite/src/sendspin/{mod.rs,messages.rs,noise.rs,clock.rs,conn.rs}`
- Test: `crates/nidoo-satellite/tests/sendspin_e2e.rs`

**Interfaces:**
- Consumes: `nidoo_sound::proto::audio::{Reassembler, MSG_JSON, MSG_PLAYER_AUDIO, CHUNK_HEADER_LEN}`, `nidoo_sound::proto::noise::{SENTINEL_PSK, psk_id, Suite}`, `PlayerIdentity` (Task 3).
- Produces:
  - `pub struct PlayerConfig { pub name: String, pub buffer_capacity: u32 /* 1_048_576 */ }`
  - `pub async fn serve(listener: TcpListener, identity: Arc<PlayerIdentity>, cfg: PlayerConfig, events: mpsc::Sender<SendspinEvent>, local: watch::Receiver<LocalPlayerState>) -> anyhow::Result<()>`. Accepts WebSocket upgrades on `/sendspin`, one live connection at a time; a newer connection replaces the older one.
  - `pub fn advertise(port: u16, instance: &str) -> anyhow::Result<mdns_sd::ServiceDaemon>`. Advertises `_sendspin._tcp` with TXT `path=/sendspin`.
  - `pub enum SendspinEvent { Connected, Disconnected, StreamStart(AudioFormatOwned), Chunk { local_play_at_us: i64, pcm: bytes::Bytes }, StreamEnd, Volume(u8), Mute(bool) }`
  - `pub struct LocalPlayerState { pub volume: u8, pub muted: bool }`. A change triggers a `client/state`.
  - `pub struct ClockSync` with `fn on_sample(&mut self, client_tx: i64, server_rx: i64, server_tx: i64, client_rx: i64)` and `fn to_local(&self, server_us: i64) -> Option<i64>`. Keeps the lowest-RTT sample of the last 8. Asks `client/time` every 1 s for the first 5 samples, then every 10 s.
  - `pub fn now_us() -> i64`. The local monotonic clock that every `local_play_at_us` refers to.

Handshake (mirror of `nidoo_sound::player::handshake`): send cleartext `client/init {client_id, version: 1, suite: "25519_ChaChaPoly_SHA256"}`, then read `server/init` (learn `server_id`). The prologue is init text + server-init text. Read `noise/handshake` msg1 as the Noise **responder** (`local_private_key` = ours, `remote_public_key` = decoded `server_id`, `psk(2, SENTINEL_PSK)`). Check its payload `psk_id`, then reply msg2 with payload `{}`. After that, every message is an encrypted binary: `[MSG_JSON] + json` or an audio chunk. Send `client/hello` (`player@v1`, `supported_formats: [pcm 48000/2/16]`, `buffer_capacity`, `supported_commands: ["volume","mute"]`), then `client/state {available: true, player: {volume, muted}}`.

- [ ] **Step 1: Failing unit tests**

```rust
#[test] fn clock_picks_lowest_rtt_sample()   // samples with rtt 10 ms (offset A) and 1 ms (offset B) → to_local uses B
#[test] fn clock_none_before_first_sample()
#[test] fn server_messages_parse()           // server/init, server/hello, server/time, server/command {player:{volume:40}}, stream/start {player:{codec:"pcm",…}}, stream/end, unknown type → Other
```

- [ ] **Step 2: Failing e2e test** in `tests/sendspin_e2e.rs`. It runs the real server side: `nidoo_sound::player::run_player(url, Arc::new(ServerIdentity::load_or_create(tmp)?), Arc::new(MonotonicClock::new()), ev_tx, cmd_rx)` against our `serve` on `127.0.0.1:0`.

```rust
#[tokio::test] async fn nidoo_sound_connects_and_streams()
    // nidoo-sound sees PlayerEvent::Connected { name: "Test Mac", supported_formats ∋ PCM_48K_STEREO_16, .. }
    // then PlayerEvent::State { available: true, .. }
    // send Activate{playback:true}, StreamStart(PCM_48K_STEREO_16), Audio{timestamp_us: now+500_000, data: 1920 bytes}
    // our side yields Connected, StreamStart(pcm 48k/2/16), Chunk { pcm == the same 1920 bytes, local_play_at_us within ±50 ms of the expected time }
#[tokio::test] async fn volume_and_mute_round_trip()
    // send PlayerCmd::Volume(40) → our SendspinEvent::Volume(40); set LocalPlayerState{volume:40} → nidoo-sound sees State{volume: Some(40)}
#[tokio::test] async fn accepts_a_new_connection_after_the_first_drops()
    // abort the first run_player; a second one reaches Connected; our side yields Disconnected then Connected
```

- [ ] **Step 3:** Run `cargo test -p nidoo-satellite --test sendspin_e2e`. Expected: FAIL.
- [ ] **Step 4:** Implement. If a `nidoo-sound` item you need isn't `pub`, make it `pub` without changing its behavior, and say so in the commit.
- [ ] **Step 5:** Run `cargo test -p nidoo-satellite`. Expected: PASS.
- [ ] **Step 6: Commit** `feat(satellite): Sendspin player side`.

---

### Task 6: `Playout`, the timed output buffer

**Files:**
- Create: `crates/nidoo-satellite/src/playout.rs`

**Interfaces:**
- Consumes: `SendspinEvent::Chunk` times (Task 5).
- Produces:
  - `pub struct Playout` with `fn push(&mut self, local_play_at_us: i64, pcm: &[u8])` (s16le stereo 48 kHz), `fn clear(&mut self)`, `fn set_volume(&mut self, v: u8)`, `fn set_muted(&mut self, m: bool)`, `fn set_wake_muted(&mut self, m: bool)`
  - `fn fill(&mut self, out: &mut [f32], channels: usize, first_frame_at_us: i64)`. Writes the frames due at `first_frame_at_us` and after, and up-mixes or down-mixes to `channels`. Output gain is `(volume/100)^2`, or 0 when `muted || wake_muted`.
  - `pub type SharedPlayout = Arc<std::sync::Mutex<Playout>>`

- [ ] **Step 1: Failing tests**

```rust
#[test] fn plays_chunk_at_its_time()        // chunk at t=1_000_000 µs; fill from t=990_000 → 480 silent frames then the chunk's samples
#[test] fn gap_plays_silence()
#[test] fn late_chunks_are_dropped()         // a chunk entirely before first_frame_at_us is discarded, never played later
#[test] fn overlapping_chunk_does_not_double() // two chunks overlapping in time → each frame played once, from the later-pushed chunk
#[test] fn stream_end_clears()               // clear() → only silence
#[test] fn gain_volume_mute_and_wake_mute()  // volume 50 → ×0.25; volume 0 → 0; muted → 0; wake_muted → 0 with volume unchanged after release
#[test] fn mono_output_mixes_channels()
```

- [ ] **Step 2:** Run `cargo test -p nidoo-satellite playout`. Expected: FAIL.
- [ ] **Step 3:** Implement. Index chunks by start frame, and convert µs to frames at 48 kHz.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(satellite): timed playout buffer`.

---

### Task 7: The satellite runtime (decisions + wiring), testable without a Mac

**Files:**
- Create: `crates/nidoo-satellite/src/brain.rs` (pure decisions), `src/run.rs` (tokio wiring)
- Test: in `brain.rs`, and `crates/nidoo-satellite/tests/voice_e2e.rs`

**Interfaces:**
- Consumes: Tasks 3–6.
- Produces:
  - `pub trait WakeDetector: Send { fn push(&mut self, samples_16k: &[i16]) -> bool; fn reset(&mut self); }`
  - `pub trait MicSwitch: Send { fn set(&mut self, on: bool) -> anyhow::Result<()>; }`. The shell opens or closes the real input stream. An `Err` means mic access is missing.
  - `pub enum Control { SetListening(bool), SetVoiceKey(String), Quit }`
  - `pub enum IconState { Gray, Ready, Listening, MicOff }`
  - `pub struct UiState { pub icon: IconState, pub status: String, pub listening: bool }`
  - `pub struct Config { pub store_dir: PathBuf, pub name: String, pub voice_port: u16 /* 4961 */, pub sendspin_port: u16 /* 0 = any */, pub advertise: bool }`
  - `pub struct Handles { pub playout: SharedPlayout, pub control: mpsc::Sender<Control>, pub ui: watch::Receiver<UiState> }`
  - `pub async fn start(cfg: Config, mic: Box<dyn MicSwitch>, mic_rx: mpsc::Receiver<Vec<i16>>, detector: Box<dyn WakeDetector>) -> anyhow::Result<Handles>`

Decisions in `brain.rs`:
- **Mic.** The mic is wanted when `listening && voice key set && greeted`.
- **Icon.** Gray when no Sendspin connection, no voice key, or not greeted. Otherwise Listening when a window is open, MicOff when listening is off, and Ready in every other case.
- **Status line**, one per case: waiting for the hub / allow this Mac in Nidoo (Sendspin connected, no Hello) / set the voice key / microphone access needed / connected.
- **Wake word.** Set wake-muted, send Wake, pause the detector, and route mic samples to `VoiceLink::audio`.
- **Window close** (Stop, timeout or listening off). Stop streaming, `detector.reset()` and resume it. Release wake-mute no earlier than 1 s after the wake (board rule).
- **Listen.** Open a follow-up window, without wake-mute.

- [ ] **Step 1: Failing brain tests**

```rust
#[test] fn mic_closed_until_greeted_and_listening()
#[test] fn icon_states_cover_spec()                 // table: (sendspin, key, greeted, listening, window) → IconState, incl. Gray when no key
#[test] fn wake_mutes_then_unmutes_not_before_1s()  // Stop at +300 ms → wake_muted stays until +1000 ms
#[test] fn listening_off_mid_window_ends_it()       // → End datagram, mic off, wake-mute released (after the 1 s floor), icon MicOff
#[test] fn listen_opens_follow_up_without_mute()
#[test] fn mic_error_shows_access_needed()
```

- [ ] **Step 2: Failing e2e** in `tests/voice_e2e.rs`. It uses a fake hub (a tokio `UdpSocket` speaking `nidoo_voice`), a `FakeDetector` that fires when it sees the sample value `12345`, a recording `MicSwitch`, `advertise: false`, and `nidoo_sound::player::run_player` connected to the Sendspin port.

```rust
#[tokio::test] async fn wake_streams_until_stop()
    // Hello(nonce 42, hub = fake socket) → ui icon Ready, mic switched on
    // push samples containing 12345 → fake hub receives Wake{nonce:42, session:1, kind: WakeWord}, then Audio
    // send Stop{session:1} → no more Audio for that session; ui icon back to Ready
#[tokio::test] async fn follow_up_after_listen()
#[tokio::test] async fn gray_without_hello_for_15s() // use a short TTL through a test-only constructor, not a 15 s sleep
```

- [ ] **Step 3:** Run `cargo test -p nidoo-satellite`. Expected: FAIL.
- [ ] **Step 4:** Implement `brain.rs` as a sans-IO struct (inputs in, `Vec<Action>` out). Then implement `run.rs`. It binds UDP `voice_port`, binds the TCP listener, optionally advertises, ticks every 20 ms (board rate), applies `SendspinEvent`s to the `Playout`, and publishes `UiState` on every change. `Control::Quit` closes the connection and stops the tasks.
- [ ] **Step 5:** Run `cargo test -p nidoo-satellite`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`. Expected: all pass.
- [ ] **Step 6: Commit** `feat(satellite): runtime and decisions`.

---

### Task 8: The macOS app (`satellite/mac/`)

**Files:**
- Create: `satellite/mac/Cargo.toml` (bin `nidoo-satellite-mac`, empty `[workspace]`, deps: `nidoo-satellite` by path, the wake engine from Task 1, `cpal`, `tray-icon` plus the event loop it needs, a clipboard crate, a resampler)
- Create: `satellite/mac/src/{main.rs,audio.rs,wake.rs,menu.rs,login.rs}`, `satellite/mac/icons/*.png` (template images: ready, listening, mic-off, gray)
- Create: `satellite/mac/Info.plist`, `satellite/mac/build-app.sh`
- Modify: `satellite/.gitignore`

**Interfaces:**
- Consumes: `nidoo_satellite::{start, Config, Handles, Control, UiState, IconState, WakeDetector, MicSwitch, SharedPlayout}`.

- [ ] **Step 0: `main.rs`.** Build a tokio runtime on a background thread and call `start` with `Config { store_dir: ~/Library/Application Support/Nidoo Satellite, name: the Mac's computer name (`scutil --get ComputerName`, falling back to the hostname), voice_port: 4961, sendspin_port: 0, advertise: true }`. The menu event loop runs on the main thread.
- [ ] **Step 1: `audio.rs`.** Use a `cpal` output stream on the default device whose callback calls `playout.lock().fill(...)` with the device's output-latency-corrected time, and outputs silence if the lock is busy. Reopen the stream when the default device changes. The input stream is opened and closed through `MicSwitch`, and resamples to 16 kHz mono i16 into `mic_rx`.
- [ ] **Step 2: `wake.rs`.** Implement `WakeDetector` with the Task 1 engine. On first run, download the model files (URLs and SHA-256 from the spike notes) into the store dir and verify them. While downloading, the status line says so. The threshold comes from `Settings.wake_threshold`, or the spike's value when unset.
- [ ] **Step 3: `menu.rs`.** Use `tray-icon`, with no Dock icon. Items:
  - a status line (disabled item showing `UiState.status`)
  - **Listening** (checkbox)
  - **Set voice key…**: reads the clipboard and sends `Control::SetVoiceKey`
  - **Start at login** (checkbox)
  - **Quit**

  The icon follows `UiState.icon`.
- [ ] **Step 4: `login.rs`.** Start at login writes or removes `~/Library/LaunchAgents/io.github.gekkotron.nidoo-satellite.plist` pointing at the bundle's executable. The checkbox reflects whether the file exists.
- [ ] **Step 5: Bundle.** `Info.plist` sets:
  - `CFBundleIdentifier io.github.gekkotron.nidoo-satellite`
  - `CFBundleName "Nidoo Satellite"`
  - `LSUIElement true`
  - `NSMicrophoneUsageDescription` ("Listens for “Hey Jarvis” and sends your request to your Nidoo hub.")
  - `NSLocalNetworkUsageDescription`
  - `NSBonjourServices [_sendspin._tcp]`

  `build-app.sh` runs `cargo build --release` in `satellite/mac`, assembles `Nidoo Satellite.app` under `$CARGO_TARGET_DIR`, ad-hoc signs it (`codesign -s -`), and with `--install` copies it to `~/Applications`. It must pass `shellcheck`.
- [ ] **Step 6: Manual verification on the Mac**, written into the PR description:
  1. The app appears in the menu bar with no Dock icon, and the icon is gray.
  2. In Nidoo it appears as a sound satellite; after Allow and a room, music plays in sync with a board.
  3. After Set voice key, the icon becomes normal within 5 s.
  4. "Hey Jarvis, allume la lumière du salon" works: music drops, the house acts, the answer plays.
  5. A question back gets a follow-up without the wake word.
  6. With Listening off, the orange mic dot disappears and the wake word is ignored, while music continues.
  7. Turning Wi-Fi off makes the icon gray in ≤ 15 s; turning it back on returns to normal by itself.
  8. Quit removes the satellite from the music.
  9. After a restart, the same satellite comes back (no new entry in Nidoo).
- [ ] **Step 7: Commit** `feat(satellite-mac): menu-bar app`.

---

### Task 9: Docs

**Files:**
- Create: `docs/users/mac-satellite.md`. Covers build (`satellite/mac/build-app.sh --install`), first launch (mic and Local Network prompts, again possibly after a rebuild), Set voice key, Allow plus room, the menu, the gray icon, the orange mic dot, Start at login, and the privacy section (same rules as the board).
- Modify: `satellite/README.md` (a pointer to the Mac satellite), `docs/users/talk-to-a-satellite.md` and `docs/users/sound.md` (one line each linking the new page).

- [ ] **Step 1:** Write the pages in the style of `talk-to-a-satellite.md`, with no owner-specific names.
- [ ] **Step 2:** Run `tools/ci/brand-check.sh` and the docs build (`cd website && npm run build`). Expected: both pass.
- [ ] **Step 3: Commit** `docs: the Mac satellite`.
