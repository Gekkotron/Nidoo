# Satellite Speech and Satellite Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Say a sentence on the satellites of one room, of the whole house, or on one satellite, over ducked music; and show each satellite as one card with a test button.

**Architecture:** The hub resolves the target satellites (room → `nidoo_sound_<sid>` devices that are Allowed and Connected), makes the clip with the existing `Tts::speak`, and publishes `nidoo-sound/speak`. nidoo-sound fetches the WAV from the hub, converts it to 48 kHz stereo, and a speech mixer mixes it into the target players' chunks on the shared clock (music ducked), or streams it alone. The web gets a satellite card (hub folds Mute/Allowed/Connected into Volume), a "Say something" box on Settings → Sound, and a `speak` flow node.

**Tech Stack:** Rust 2024 workspace; tokio, rumqttc, reqwest 0.13 (workspace dep) in nidoo-sound; axum + rmcp in the hub; Leptos (wasm) web with wasm-bindgen-test.

**Spec:** `docs/superpowers/specs/2026-11-04-satellite-speech-design.md` (approved 2026-11-04). Branch `fix/sound-followups`, which already holds (uncommitted at plan time): Allowed published as a primary switch, the hub.json broker layer in `crates/nidoo-sound/src/config.rs`, and the Settings → Sound page (`crates/nidoo-web/src/screens/sound_settings.rs`).

## Global Constraints

- Public, generic repo: no house-, room- or person-specific word, host, IP or phone-app brand in code, tests or docs.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, explicit paths only (never `git add -A`; `crates/nidoo/examples/` is not ours).
- Cargo runs prefixed `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; check `df -h ~` ≥ 5 GB before a workspace build.
- Every task ends green on: `cargo fmt --all -- --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, the crate's tests; web tasks also `cargo check -p nidoo-web --target wasm32-unknown-unknown` and `cd crates/nidoo-web && wasm-pack test --headless --chrome`.
- User-facing web text goes through the i18n tables, English and French (`crates/nidoo-web/tests/bare_text.rs` stays green).
- Stream format is fixed: s16le, 48 kHz, stereo, 20 ms chunks (`pacer::CHUNK_FRAMES = 960`, `CHUNK_BYTES = 3840`, `CHUNK_US = 20_000`).
- Duck: music gain `10^(-15/20) = 0.17783` under speech; fade 150 ms (7200 frames) into and out of the duck.
- One sentence at a time; queue at most 4 waiting sentences (a 5th is dropped with a warning).
- Speak topic `nidoo-sound/speak`, not retained, QoS 1, payload `{"id": "<clip id>", "url": "/api/v1/assist/speech/<id>.wav", "players": ["<sid>", …]}`.
- Satellite device ids are `nidoo_sound_<sid>`; the prefix is one constant `SOUND_DEVICE_PREFIX = "nidoo_sound_"` in `nidoo-core` (Task 4), used by the hub and the web.

## Review Focus

1. A sentence sent to a room whose satellites are all muted or offline → the hub answers 409 "no satellite can speak in this room" when none is Allowed+Connected; a muted one counts as able (it stays silent, as for music). Test in Task 5.
2. Two quick sends to the same room → the second plays after the first, never overlapping. Test in Task 3.
3. Music stopping during a sentence (phone app paused) → the sentence finishes, then the stream ends. Test in Task 3.
4. A satellite that disconnects mid-sentence → the rest keep speaking; nothing panics; the link's queue drops as for music. Test in Task 3.
5. A room name typed in French or with accents ("Séjour", "cuisine") in the flow node, the API or the MCP tool → resolved like other room fields (`Registry::resolve_area`). Test in Task 5.

---

### Task 0: commit the work already on the branch

Before Task 1, commit the uncommitted branch work as three commits, explicit paths, as Gekkotron, after the full nidoo-sound and nidoo-web checks pass: (1) `fix(sound): Allowed shows with the other controls, and the log names the broker user` (`crates/nidoo-sound/src/{mqtt,main}.rs` Allowed + log parts, `crates/nidoo-sound/README.md` table row); (2) `feat(sound): use the hub's broker login, no .env needed` (`crates/nidoo-sound/src/config.rs`, the rest of `main.rs`/README, `compose.yml`); (3) `feat(web): Settings → Sound lists the satellites` (the settings page files and `docs/users/sound.md`). If the hunks cannot be split cleanly, one commit per crate is fine.

### Task 1: nidoo-sound — speak message, WAV conversion, hub address

**Files:**
- Create: `crates/nidoo-sound/src/speech.rs` (register in `src/lib.rs`)
- Modify: `crates/nidoo-sound/src/config.rs` (add `hub_url`), `crates/nidoo-sound/Cargo.toml` (add `reqwest.workspace = true`; dev-dep only if needed)

**Interfaces:**
- Produces:
  - `pub const SPEAK_TOPIC: &str = "nidoo-sound/speak";`
  - `#[derive(Debug, Clone, PartialEq, Eq, Deserialize)] pub struct SpeakRequest { pub id: String, pub url: String, pub players: Vec<String> }`
  - `pub fn parse_speak(payload: &[u8]) -> Option<SpeakRequest>` — `None` on bad JSON or empty `players`.
  - `pub fn wav_to_stream_pcm(wav: &[u8]) -> anyhow::Result<bytes::Bytes>` — RIFF/WAVE, PCM format 1, 16-bit, 1 or 2 channels, any rate → s16le 48 kHz stereo; errors on anything else. Skips unknown chunks; a `data` size of 0 or `0xFFFF_FFFF` (streamed header) means "to the end of the buffer".
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct Speech { pub players: Vec<String>, pub pcm: bytes::Bytes }` — players are short ids, pcm is stream format.
  - `pub async fn fetch_speech(client: &reqwest::Client, hub_url: &str, req: &SpeakRequest) -> anyhow::Result<Speech>` — GET `hub_url + req.url`, 10 s timeout, reads to the end, converts; rejects a clip longer than 60 s of audio.
  - `SoundConfig.hub_url: Option<String>` (env `NIDOO_SOUND_HUB_URL`) and `pub fn hub_base_url(&self) -> String`: `hub_url` if set; else from `<data_dir>/hub.json` `http.bind` (`0.0.0.0`/`[::]` → `127.0.0.1`, as `http://host:port`); else `http://127.0.0.1:8081`. Read hub.json once in `load` alongside the broker (extend the private `HubFile` there with `http: Option<HubHttp { bind: String }>`) and store the derived value in a `#[serde(skip)] pub hub_http: Option<String>` field.

- [ ] **Step 1: Write failing tests** in `speech.rs` `#[cfg(test)]`:
  - `parse_speak_reads_the_hub_message`: `{"id":"a","url":"/api/v1/assist/speech/a.wav","players":["afd56455c914"]}` → the struct; `{"id":"a","url":"/x","players":[]}` → `None`; `b"nope"` → `None`.
  - `mono_24k_is_doubled_and_duplicated`: a WAV built in the test (helper `wav(rate, channels, samples: &[i16]) -> Vec<u8>`) of 24 kHz mono `[0, 1000]` → frames `[(0,0),(500,500),(1000,1000),(1000,1000)]` (linear interpolation, output length `ceil(n × 48000 / rate)`, position `i × rate / 48000`, last sample held).
  - `stereo_48k_passes_through`: 48 kHz stereo `[1,2,3,4]` → same bytes.
  - `streamed_header_reads_to_the_end`: data size `0xFFFF_FFFF` with 4 samples → 4 samples read.
  - `rejects_non_pcm_and_8_bit`: format 3 → `Err`; 8-bit → `Err`.
  - config: `hub_url_comes_from_hub_json` (`"http":{"bind":"0.0.0.0:9090"}` → `http://127.0.0.1:9090`), `hub_url_env_wins` (`NIDOO_SOUND_HUB_URL=http://hub:1` wins), `hub_url_default` (`http://127.0.0.1:8081`).
- [ ] **Step 2: Run** `cargo test -p nidoo-sound --lib speech config` → FAIL (missing items).
- [ ] **Step 3: Implement** the items above. `fetch_speech` test: `fetch_reads_a_clip_from_the_hub` with a `tokio::net::TcpListener` serving one fixed HTTP/1.1 response (WAV body) — no new server dependency.
- [ ] **Step 4: Run** the tests → PASS; clippy, fmt.
- [ ] **Step 5: Commit** `feat(sound): read speak messages and turn hub speech into stream audio`.

### Task 2: nidoo-sound — speech mixer

**Files:**
- Create: `crates/nidoo-sound/src/mixer.rs` (register in `src/lib.rs`)

**Interfaces:**
- Consumes: `speech::Speech`, `pacer::{CHUNK_FRAMES, CHUNK_US, FRAME_BYTES}`.
- Produces (pure, no I/O, no clock):
  - `pub const DUCK_GAIN: f32 = 0.177_83;` `pub const FADE_FRAMES: usize = 7_200;` `pub const MAX_QUEUED: usize = 4;`
  - `pub struct SpeechMixer` with:
    - `pub fn new() -> Self`
    - `pub fn push(&mut self, speech: Speech) -> bool` — `false` (dropped) when `MAX_QUEUED` are already waiting.
    - `pub fn is_playing(&self) -> bool`, `pub fn has_queued(&self) -> bool`
    - `pub fn start_next(&mut self, start_us: i64) -> Option<&[String]>` — when nothing plays, takes the next sentence and anchors its window at `start_us`; returns its targets.
    - `pub fn targets(&self) -> &[String]` (empty when idle)
    - `pub fn end_us(&self) -> Option<i64>` — window end: `start_us + (FADE_FRAMES × 2 + speech frames) × 1e6 / 48000`.
    - `pub fn mix_chunk(&self, timestamp_us: i64, music: Option<&[u8]>) -> Option<bytes::Bytes>` — the target players' 20 ms chunk at `timestamp_us`: `None` when the chunk does not overlap the window; otherwise each frame `f` (relative to window start, `f = round((timestamp_us − start_us) × 48 / 1000) + i`) is `clamp_i16(music × gain(f) + speech[f − FADE_FRAMES])`, music absent = silence, `gain` ramps linearly 1 → `DUCK_GAIN` over the first `FADE_FRAMES`, stays, ramps back over the last `FADE_FRAMES`.
    - `pub fn finish_if_done(&mut self, now_sent_us: i64) -> bool` — clears the current sentence once `now_sent_us ≥ end_us()`; returns whether it did.

- [ ] **Step 1: Write failing tests** (`#[cfg(test)]` in `mixer.rs`), music = constant `10_000` both channels, speech = constant `8_000`, one chunk long:
  - `before_the_window_music_is_untouched`: `mix_chunk(start − CHUNK_US, Some(music))` → `None`.
  - `under_speech_music_is_ducked_and_speech_added`: chunk fully inside the speech part → every sample `round(10_000 × 0.17783) + 8_000 = 9_778`.
  - `fade_starts_at_full_music`: first frame of the window → `10_000`.
  - `after_the_window_nothing`: `mix_chunk(end_us, …)` → `None`; `finish_if_done(end_us)` → `true`, `is_playing()` false.
  - `speech_alone_without_music`: `music = None`, inside speech → `8_000`.
  - `loud_mix_saturates`: music `30_000` + speech `30_000` → `32_767`.
  - `queue_is_bounded`: 5 pushes while one plays → 5th returns `false`.
  - `next_sentence_waits`: `start_next` while playing → `None`.
- [ ] **Step 2: Run** `cargo test -p nidoo-sound --lib mixer` → FAIL.
- [ ] **Step 3: Implement** `mixer.rs` (i16 arithmetic in f32, round half away from zero).
- [ ] **Step 4: Run** → PASS; clippy, fmt.
- [ ] **Step 5: Commit** `feat(sound): speech mixer that ducks the music under a sentence`.

### Task 3: nidoo-sound — wire speech into MQTT and the group

**Files:**
- Modify: `crates/nidoo-sound/src/mqtt.rs` (subscribe `SPEAK_TOPIC`; `run_mqtt` gains `speak: mpsc::Sender<SpeakRequest>`), `crates/nidoo-sound/src/group.rs` (`Group::run` gains `speech: mpsc::Receiver<Speech>`; mixer in the tick), `crates/nidoo-sound/src/main.rs` (spawn `run_speech_fetcher`), `crates/nidoo-sound/src/speech.rs` (add the fetcher task)
- Test: `crates/nidoo-sound/src/group.rs` tests (fake clock + fake players harness already there), `crates/nidoo-sound/tests/` if an integration test fits better

**Interfaces:**
- Consumes: Task 1 (`SpeakRequest`, `Speech`, `fetch_speech`, `SoundConfig::hub_base_url`), Task 2 (`SpeechMixer`).
- Produces: `pub async fn run_speech_fetcher(hub_url: String, requests: mpsc::Receiver<SpeakRequest>, to_group: mpsc::Sender<Speech>)` — fetches each request in order, logs and skips failures (`warn!` with id and error).

Group behaviour (the spec's "nidoo-sound plays it"):
- On `Speech`: keep only players in the list that are connected and Allowed at that moment; none left → `info!` and drop. Else `mixer.push` (warn when dropped).
- Each tick: if idle and queued, `start_next(start)` with `start` = the next music chunk timestamp not yet sent when music plays, else `now + stream_lead()`; send `StreamStart` to targets not already streaming.
- Music chunks: targets get `mix_chunk(ts, Some(music))` when `Some`, others the shared chunk unchanged (same `Bytes`).
- No music: the group emits its own 20 ms speech chunks for the targets from `start` (due when `ts − lead ≤ now`), `mix_chunk(ts, None)`.
- After the window: `finish_if_done`; targets that are streaming only because of speech get `StreamEnd` (music streams stay).
- Music `Stopped` mid-sentence: targets' streams stay open until the window ends. Music `Started` mid-sentence: its first chunk timestamp must be after the last speech-only chunk sent (no overlap, strictly increasing timestamps per player).
- The tick runs while `self.playing || mixer.is_playing() || mixer.has_queued()`.

- [ ] **Step 1: Write failing tests** with the existing harness (`start_with`, `Fake`):
  - `speech_with_music_reaches_only_its_targets`: two allowed players streaming music; speech to one → its chunks inside the window differ from the shared chunk; the other's chunks are byte-identical to the shared ones.
  - `speech_without_music_starts_and_ends_a_stream`: target gets `StreamStart`, ≥1 `Audio`, `StreamEnd`; the other player gets nothing.
  - `targets_of_one_sentence_get_identical_chunks`: two targets (no music, then with music) receive the same timestamps and bytes for every speech chunk (anti-goal: no echo).
  - `speech_skips_players_not_allowed_or_offline`: list names a not-allowed player → it gets nothing.
  - `two_sentences_play_one_after_the_other`: second sentence's first `Audio` timestamp ≥ first sentence's window end.
  - `music_stopping_mid_sentence_keeps_the_stream`: `InputEvent::Stopped` during speech → no `StreamEnd` before the window end, then `StreamEnd`.
  - `music_starting_mid_sentence_never_overlaps`: timestamps per target strictly increasing across the switch.
  - `a_target_leaving_mid_sentence_does_not_stop_the_others`.
  - mqtt: `speak_topic_is_routed` — `parse_speak` is called for `SPEAK_TOPIC` payloads (unit-test the routing function you extract from the event loop).
- [ ] **Step 2: Run** `cargo test -p nidoo-sound` → FAIL.
- [ ] **Step 3: Implement** the wiring above; keep `on_tick` readable by moving per-link chunk choice into a helper.
- [ ] **Step 4: Run** all nidoo-sound tests → PASS (lib + integration, previous 75 + 17 still green); clippy, fmt.
- [ ] **Step 5: Commit** `feat(sound): speak sentences on chosen satellites over ducked music`.

### Task 4: nidoo-core — fold a satellite's controls into its Volume

**Files:**
- Modify: `crates/nidoo-core/src/registry/folding.rs`, `crates/nidoo-core/src/model/mod.rs` (export the constant)
- Test: `crates/nidoo-core/src/registry/folding.rs` tests or `crates/nidoo-core/tests/registry.rs` (follow where cover folding is tested)

**Interfaces:**
- Produces: `pub const SOUND_DEVICE_PREFIX: &str = "nidoo_sound_";` in `nidoo_core::model` (doc comment: devices published by `crates/nidoo-sound/src/mqtt.rs`). The settings page's local prefix constant (`crates/nidoo-web/src/screens/sound_settings.rs`) is replaced by this one in this task.
- Rule: on a device whose id starts with `SOUND_DEVICE_PREFIX` and holds exactly one `Kind::Number`, every `Switch` and `BinarySensor` of that device folds into that number (`part_of = <volume unique_id>`). Add the rule to the module doc list.

- [ ] **Step 1: Write failing test** `sound_satellite_controls_fold_into_volume`: entities `nidoo_sound_x_volume` (Number), `_mute`, `_allowed` (Switch), `_connected` (BinarySensor), device `nidoo_sound_x` → the three map to `nidoo_sound_x_volume`; a non-sound device with one number and one switch → no fold.
- [ ] **Step 2: Run** `cargo test -p nidoo-core folding` → FAIL.
- [ ] **Step 3: Implement** the rule in `compute`.
- [ ] **Step 4: Run** `cargo test -p nidoo-core` and `cargo test -p nidoo-web --lib` (web unit tests that count room entities) → PASS.
- [ ] **Step 5: Commit** `feat(core): a sound satellite's mute, approval and link fold into its volume`.

### Task 5: hub — target satellites, `Hub::speak_in`, `POST /api/v1/sound/speak`

**Files:**
- Create: `crates/nidoo/src/sound.rs` (register in `lib.rs`), `crates/nidoo/src/http/sound.rs` (route, register where `http/tts.rs` routes are merged), `crates/nidoo-api/src/sound.rs` (register in `lib.rs`)
- Modify: `docs/reference/http-api.md`, `docs/reference/mqtt-topics.md` (`nidoo-sound/speak`)

**Interfaces:**
- Consumes: `nidoo_core::model::SOUND_DEVICE_PREFIX` (Task 4), `Hub::tts()`, `Tts::speak(SpeechRequest) -> Result<SpeechTicket, TtsError>`, `Hub::publish(Publish { topic, payload, retain })`, `Registry::{entities, resolve_area, effective_areas}`.
- Produces:
  - nidoo-api: `#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct SoundSpeakRequest { pub text: String, #[serde(default)] pub room: Option<String>, #[serde(default)] pub satellite: Option<String>, #[serde(default)] pub language: Option<String>, #[serde(default)] pub voice: Option<String> }` and `pub struct SoundSpeakReply { pub players: usize }`.
  - hub: `pub enum SpeakTarget { All, Room(String), Satellite(String) }`;
    `pub fn speaking_satellites(reg: &Registry, target: &SpeakTarget) -> Result<Vec<String>, SpeakError>` — short ids of `SOUND_DEVICE_PREFIX` devices whose `<device>_allowed` is on and `<device>_connected` is on; `Room(raw)` resolves with `resolve_area` and matches the area of `<device>_volume`; an unknown room → `SpeakError::UnknownRoom(raw)`; empty → `SpeakError::NoSatellite`.
    `impl Hub { pub async fn speak_in(&self, target: SpeakTarget, text: &str, language: Option<String>, voice: Option<String>) -> Result<usize, SpeakError> }` — resolves, then `Tts::speak`, then publishes the speak payload (Global Constraints) with `retain: false`; returns the player count.
    `#[derive(Debug, thiserror::Error)] pub enum SpeakError { UnknownRoom(String), NoSatellite, NotSetUp, Tts(TtsError), Publish(HubError) }`.
  - HTTP `POST /api/v1/sound/speak` (`SoundSpeakRequest` → 202 `SoundSpeakReply`): `room` and `satellite` both set → 422; `UnknownRoom` and `TtsError::Invalid` → 422; `NoSatellite`, `NotSetUp` and TTS conflicts → 409; same `HttpError` style as `http/tts.rs`.

- [ ] **Step 1: Write failing tests**:
  - `speaking_satellites_*` (unit, building a `Registry` the way other hub tests do): all-rooms picks every allowed+connected satellite; a room picks only its own; a not-allowed or disconnected one is left out; a muted one is kept; `"Séjour"`/`"sejour"` resolves like other room fields; unknown room → `UnknownRoom`; nothing left → `NoSatellite`.
  - HTTP: `speak_route_rejects_room_and_satellite_together` (422), `speak_route_without_tts_is_409`, and, with the fake TTS engine the assist tests use, `speak_route_publishes_the_speak_message` (asserts topic `nidoo-sound/speak`, not retained, payload `players` = the resolved ids, `url` starts with `/api/v1/assist/speech/`).
- [ ] **Step 2: Run** `cargo test -p nidoo sound` → FAIL.
- [ ] **Step 3: Implement**; docs: the route in `http-api.md`, the topic in `mqtt-topics.md`.
- [ ] **Step 4: Run** `cargo test -p nidoo -p nidoo-api` → PASS; clippy, fmt.
- [ ] **Step 5: Commit** `feat(hub): say a sentence on the satellites of a room or the whole house`.

### Task 6: hub — `speak` flow node and MCP tool

**Files:**
- Modify: `crates/nidoo-core/src/flow/model.rs` (`NodeKind::Speak`), `crates/nidoo-core/src/flow/validate.rs`, `crates/nidoo/src/flows/engine.rs` (run it), `crates/nidoo/src/flows/hub.rs` (room check at save, next to `check_area`), `crates/nidoo/src/mcp/tools.rs` + `params.rs` (tool `speak`), `docs/reference/flow-nodes.md`, `docs/reference/mcp-tools.md`

**Interfaces:**
- Consumes: Task 5 `Hub::speak_in`, `SpeakTarget`, `SpeakError`.
- Produces:
  - `#[serde(rename = "speak")] Speak(SpeakParams)`, `pub struct SpeakParams { pub text: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub room: Option<String> }` (`None` = all rooms); `type_name` → `"speak"`. Text takes the same `{entity_id}` placeholders as `notify` (reuse its `render`/`fill`).
  - Validation: empty text → `FlowError::BadSpeak { node, reason }`; at save, a `room` that does not resolve to a known area → `HubError::InvalidFlow("unknown area …")` (same rule as `check_area`), and the resolved id is stored.
  - Engine: `NodeKind::Speak` → `hub.speak_in(target, &text, None, None).await`; `Ok(n)` → `StepOutcome::Ok`, detail `"spoke on {n} satellite(s): {text}"`; `Err(e)` → `failed(e.to_string())`.
  - MCP tool `speak` (params `{text: String, room: Option<String>, language: Option<String>, voice: Option<String>}`; no room = all rooms) returning `{"players": n}` or the error text, documented in its tool description like the others.

- [ ] **Step 1: Write failing tests**: `validate` refuses empty speak text; `set_flow` refuses an unknown room and stores the resolved id for `"Séjour"`; engine test with the fake TTS + a registry holding one allowed satellite → the run step is `Ok` and a `nidoo-sound/speak` publish happened; MCP `speak` tool test following the existing tool tests' pattern; serde round-trip of a `speak` node.
- [ ] **Step 2: Run** `cargo test -p nidoo-core flow` and `cargo test -p nidoo flows mcp` → FAIL.
- [ ] **Step 3: Implement**; document the node (`flow-nodes.md`) and the tool (`mcp-tools.md`).
- [ ] **Step 4: Run** `cargo test -p nidoo-core -p nidoo` → PASS; clippy, fmt.
- [ ] **Step 5: Commit** `feat(flows): speak node and MCP speak tool`.

### Task 7: web — satellite card with the test button

**Files:**
- Create: `crates/nidoo-web/src/cards/satellite.rs` (register in `cards/mod.rs`), `crates/nidoo-web/tests/satellite_card.rs`
- Modify: `crates/nidoo-web/src/screens/home.rs` (`card_for`: a `Kind::Number` whose `device_id` starts with `SOUND_DEVICE_PREFIX` → `SatelliteCard`), `crates/nidoo-web/src/api/` (add `pub async fn sound_speak(req: &SoundSpeakRequest) -> Result<SoundSpeakReply, ApiError>` next to `api/tts.rs::speak`), i18n tables

**Interfaces:**
- Consumes: Task 4 folding (Mute/Allowed/Connected are `part_of` the volume and no longer listed), Task 5 `SoundSpeakRequest { satellite: Some(sid), .. }`, `api::core::send_command` for volume/mute/allowed.
- Produces: `#[component] pub fn SatelliteCard(entity: EntityView) -> impl IntoView` — the approved layout:
  - header: speaker icon, name, online dot (from `<device>_connected` and availability);
  - Allowed on: volume slider (0–100) with value, Mute toggle button, small speak (🗣) button;
  - Allowed off: "Waiting for approval" line and an **Allow** button instead of slider/mute/speak;
  - speak button sends `SoundSpeakRequest { text: <i18n "Speaker test: {name}" / "Test du haut-parleur : {name}">, satellite: Some(sid), ..Default }`; a failure shows the API error inline for a few seconds.

- [ ] **Step 1: Write failing wasm tests** in `satellite_card.rs` (mount like `generic_card.rs`/`lock_card.rs`): allowed satellite shows slider value 53 and Mute; not-allowed shows "Waiting for approval" and Allow, no slider; clicking Allow sends `allowed` ON; clicking speak posts to `/api/v1/sound/speak` with `satellite` = the sid and the translated test text (capture requests the way existing tests do); offline satellite shows the offline dot.
- [ ] **Step 2: Run** `wasm-pack test --headless --chrome -- --test satellite_card` → FAIL.
- [ ] **Step 3: Implement** (reuse the number card's slider and the switch card's toggle pieces where they exist).
- [ ] **Step 4: Run** the full web checks (Global Constraints) → PASS.
- [ ] **Step 5: Commit** `feat(web): one card per sound satellite, with a speaker test`.

### Task 8: web — "Say something" on Settings → Sound and the flow editor's speak node

**Files:**
- Modify: `crates/nidoo-web/src/screens/sound_settings.rs`, flow editor files that list `Notify` (`flow_picker.rs`, `flow_text.rs`, `icons.rs`, `flows.rs`, `flow_edit.rs`, `screens/flow_canvas/settings.rs`), i18n tables
- Test: `crates/nidoo-web/tests/` (the settings page test file the earlier agent created; flow editor tests next to `flow_canvas.rs`)

**Interfaces:**
- Consumes: Task 5 `api::sound_speak`, Task 6 `NodeKind::Speak(SpeakParams)`.
- Produces:
  - Settings → Sound: a text box (max length = the hub's `MAX_SPEECH_CHARS`, exported from nidoo-api if not already), a room select listing the rooms that hold at least one satellite plus "All rooms" (first, default), a Send button disabled while empty or sending; success clears the box and shows "Said on N satellite(s)"; errors inline.
  - Flow editor: "Speak" in the action picker (icon: speaker), settings form with Text (placeholders hint as for Notify) and Room (rooms select with "All rooms" = `null`), node summary text `Speak "<text>" — <room or All rooms>`.

- [ ] **Step 1: Write failing wasm tests**: the box posts `{text, room: null}` for All rooms and `{text, room: "<id>"}` for a room; Send disabled when empty; the flow picker lists Speak; a speak node's form edits `text` and `room` into its params; the summary text matches.
- [ ] **Step 2: Run** the new tests → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** the full web checks → PASS.
- [ ] **Step 5: Commit** `feat(web): say something from Settings → Sound, and a Speak flow node`.

### Task 9: docs and whole-branch check

**Files:**
- Modify: `docs/users/sound.md` (new "Speak in a room" section: the card's test button, Settings → Sound box, the Speak flow node, the MCP tool, ducking, Allowed/Connected rule; rewrite the "Start it" login paragraph: no `.env` needed with the Compose install, the login comes from `data/hub.json`, `NIDOO_SOUND_*` still override), `docs/users/flows.md` (one line for Speak), `docs/users/assistants-mcp.md` (one line for `speak`), `TODOS.md` (tick "speech on satellites" if listed)

- [ ] **Step 1: Edit the docs** above in the existing plain style.
- [ ] **Step 2: Run** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo check -p nidoo-web --target wasm32-unknown-unknown`, and the wasm tests → all PASS.
- [ ] **Step 3: Scan** `git diff origin/main` for hosts, IPs, names, phone-app brands → none.
- [ ] **Step 4: Commit** `docs(sound): speaking in a room, and no .env needed`.
