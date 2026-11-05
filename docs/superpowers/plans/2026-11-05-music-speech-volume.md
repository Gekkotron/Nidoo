# Music and Speech volume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each sound satellite gets a Music volume and a Speech volume, so spoken replies can sit clearly above the music.

**Architecture:** `nidoo-sound` sends each player `max(music, speech)` as its Sendspin volume and scales the quieter one down in the samples by the dB gap on one shared curve (−49 dB … 0 dB, linear in volume). A new MQTT number `…_speech` carries the Speech setting; the registry folds it into the satellite's `…_volume` card; Settings → Sound shows two sliders. The Mac satellite adopts the same curve.

**Tech Stack:** Rust (tokio, serde, bytes), Leptos web UI, MQTT discovery.

**Spec:** `docs/superpowers/specs/2026-11-05-music-speech-volume-design.md`

## Global Constraints

- Volumes are `u8`, 0–100. Volume 0 is silence on the shared curve.
- Shared curve: `db(v) = -49 × (1 − v/100)` for `v ≥ 1`; the gap between the louder `hi` and the quieter `lo` is `49 × (hi − lo) / 100` dB, and `lo = 0` gives gain 0.
- New entity: object `speech`, unique id `nidoo_sound_<sid>_speech`, name "Speech volume", topics `nidoo-sound/<sid>/speech` and `nidoo-sound/<sid>/speech/set`, number 0–100 step 1 slider. The existing `volume` entity keeps its topics and unique id; its name becomes "Music volume".
- Old `players.json` without the field: Speech = that player's `volume`. New player: Speech = `DEFAULT_VOLUME` (30).
- Speech = hub speech clips only (`nidoo-sound/speak`); everything else in the stream is Music.
- `DUCK_GAIN` (−15 dB) and `FADE_FRAMES` unchanged; mute unchanged.
- Satellites of one sentence with the same (music, speech) pair get byte-identical chunks.
- Run cargo with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; check `df -h ~` first (per-crate tests when < 5 GB free).
- Commits as `Gekkotron` (`-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`), stage explicit paths only.

## Review Focus

1. Player reports a volume that is not the echo while Speech > Music: both settings must shift by the same delta and keep their gap (Task 3 test `device_change_keeps_the_gap`).
2. Changing the quieter slider must not send a Sendspin volume command (device volume unchanged) (Task 3 test `quieter_change_sends_no_volume`).
3. Echo of the sent `max` must not be read as a device-side change (Task 3 test `echo_of_max_is_not_a_change`).
4. Speech-only stream (no music) with two targets having different pairs: each gets its own speech gain (Task 3 test `speech_only_targets_get_their_own_gain`).
5. A satellite with both numbers still shows as one card with Mute/Allowed/Connected folded (Task 4 test `satellite_with_speech_still_folds`).

---

### Task 1: The shared curve and the level split

**Files:**
- Create: `crates/nidoo-sound/src/level.rs`
- Modify: `crates/nidoo-sound/src/lib.rs` (add `pub mod level;`)

**Interfaces:**
- Produces:
  - `pub struct Levels { pub device: u8, pub music_gain: f32, pub speech_gain: f32 }` (`Debug, Clone, Copy, PartialEq`)
  - `pub fn split(music: u8, speech: u8) -> Levels` — inputs clamped to 100; `device = max`; the louder side's gain is exactly `1.0`; the quieter side's gain is `10^(−49 × (hi − lo) / 100 / 20)`, or `0.0` when it is 0.

- [ ] **Step 1: Write the failing tests** in `level.rs` `mod tests`:
  - `equal_settings_change_nothing`: `split(40, 40) == Levels { device: 40, music_gain: 1.0, speech_gain: 1.0 }`.
  - `speech_louder_scales_music`: `split(30, 80)`: device 80, speech_gain 1.0, music_gain within 1e-4 of `10f32.powf(-24.5 / 20.0)`.
  - `music_louder_scales_speech`: `split(90, 40)`: device 90, music_gain 1.0, speech_gain within 1e-4 of `10f32.powf(-24.5 / 20.0)`.
  - `zero_is_silence`: `split(0, 60).music_gain == 0.0` and `split(60, 0).speech_gain == 0.0`; `split(0, 0)` has device 0.
  - `over_100_is_clamped`: `split(150, 100) == split(100, 100)`.
- [ ] **Step 2: Run** `cargo test -p nidoo-sound level::` — expected FAIL (module missing).
- [ ] **Step 3: Implement** `Levels` and `split`.
- [ ] **Step 4: Run** `cargo test -p nidoo-sound level::` — expected PASS.
- [ ] **Step 5: Commit** `feat(sound): split music and speech levels on one curve`.

### Task 2: Speech setting in state and MQTT

**Files:**
- Modify: `crates/nidoo-sound/src/state.rs`, `crates/nidoo-sound/src/mqtt.rs`, `crates/nidoo-sound/src/group.rs` (only `Control`: new variant, `short_id`, `apply`)

**Interfaces:**
- Produces:
  - `PlayerPrefs.speech_volume: u8`. Deserialization: field optional in the file; `StateStore::load` fills a missing one with that player's `volume`. Serialized always.
  - `Control::SetSpeech { short_id: String, volume: u8 }`; `apply` sets `p.speech_volume = volume.min(100)`.
  - `mqtt::parse_command` maps object `speech` like `volume` (finite number, rounded, clamped 0–100) to `Control::SetSpeech`.
  - `discovery_messages` adds the `number`/`speech` config (name "Speech volume", icon `mdi:account-voice`, same min/max/step/mode as volume); the volume config's name becomes "Music volume". `state_messages` adds `(nidoo-sound/<sid>/speech, speech_volume)`; `removal_messages` then clears it automatically.

- [ ] **Step 1: Write the failing tests**:
  - state.rs `old_file_speech_equals_volume`: a JSON file with one player `volume: 55` and no `speech_volume` loads with `speech_volume == 55`.
  - state.rs: extend the new-player test to assert `speech_volume == DEFAULT_VOLUME`, and the round-trip test to persist `speech_volume = 80`.
  - mqtt.rs `speech_command_parses`: `parse_command("nidoo-sound/abc/speech/set", b"72.6") == Some(Control::SetSpeech { short_id: "abc".into(), volume: 73 })`; `b"nan"` → `None`.
  - mqtt.rs `speech_entity_is_published`: discovery has a config with `unique_id` `nidoo_sound_<sid>_speech`, `name` "Speech volume", `command_topic` `nidoo-sound/<sid>/speech/set`; the volume config's name is "Music volume"; `state_messages` contains `("nidoo-sound/<sid>/speech", "<n>")`; `removal_messages` contains that topic with `""`.
- [ ] **Step 2: Run** `cargo test -p nidoo-sound state:: mqtt::` — expected FAIL.
- [ ] **Step 3: Implement** the field, load default, discovery/state entries, parsing and `Control::SetSpeech`. In `on_control` the new variant only saves and publishes for now (Task 3 adds the player side); exhaustive matches get an arm.
- [ ] **Step 4: Run** `cargo test -p nidoo-sound` — expected PASS (all existing tests too).
- [ ] **Step 5: Commit** `feat(sound): Speech volume setting per satellite`.

### Task 3: Device volume, sample gains and device-side changes

**Files:**
- Modify: `crates/nidoo-sound/src/group.rs`, `crates/nidoo-sound/src/mixer.rs`

**Interfaces:**
- Consumes: `level::split`, `PlayerPrefs.speech_volume`, `Control::SetSpeech`.
- Produces:
  - `SpeechMixer::mix_chunk(&self, timestamp_us: i64, music: Option<&[u8]>, gains: (f32, f32)) -> Option<Bytes>` — per sample `m × music_gain × duck + sp × speech_gain`; `(music_gain, speech_gain)`.
  - `Link.levels: Levels` (set from prefs on connect and on every volume/speech change; `Levels { device: 0, music_gain: 1.0, speech_gain: 1.0 }` before connect).
  - `fn music_for(link, ts, music, mixer, target, held) -> Bytes` applies `link.levels.music_gain` to the base chunk when it is not `1.0` (and passes both gains to `mix_chunk` for targets).

- [ ] **Step 1: Write the failing tests**:
  - mixer.rs `gains_scale_music_and_speech`: with a sentence of constant speech 1000 and music 1000, a chunk in the middle of the window with gains `(0.5, 2.0)` has samples `round(1000 × 0.5 × DUCK_GAIN + 2000)`; with `(1.0, 1.0)` it equals today's output.
  - group.rs (harness, prefs volume 30):
    - `connect_sends_the_louder_setting`: prefs volume 30 / speech 80 → first `PlayerCmd::Volume(80)`.
    - `quieter_change_sends_no_volume`: with volume 30 / speech 80, `SetVolume 20` publishes the new prefs and sends no `PlayerCmd::Volume`.
    - `louder_change_sends_volume`: `SetSpeech 90` sends `Volume(90)`.
    - `echo_of_max_is_not_a_change`: after `Volume(80)` is sent, a state report of 80 changes no prefs and publishes nothing.
    - `device_change_keeps_the_gap`: volume 30 / speech 80, echo consumed, then report 70 → prefs volume 20 / speech 70, published. Equal 50/50, report 60 → 60/60. Volume 10 / speech 40, report 100 → 70/100; report 5 with 10/40 → 0/5 (clamped, gap shrinks).
    - `music_is_scaled_for_a_speech_louder_player`: volume 30 / speech 80 while music plays: the player's chunks equal the music chunk scaled by `split(30, 80).music_gain`, rounded per sample.
    - `speech_only_targets_get_their_own_gain`: two targets, pairs (50, 50) and (90, 40), no music: their speech chunks differ, and two targets with equal pairs get identical bytes.
- [ ] **Step 2: Run** `cargo test -p nidoo-sound` — expected FAIL on the new tests.
- [ ] **Step 3: Implement**:
  - Connect, `SetVolume`, `SetSpeech`: recompute `split`; send `PlayerCmd::Volume(device)` and set `expect_volume = Some(device)` only when `device` differs from the device volume last sent to that link (always on connect).
  - `PlayerEvent::State`: compare the fresh report against `split(saved.volume, saved.speech_volume).device` instead of `saved.volume`; on a change by `delta`, add `delta` to both settings, clamping each to 0–100; save, update `link.levels`, publish.
  - The speech-only branch of `on_tick` mixes once per distinct `(music_gain, speech_gain)` pair among the targets (compare with `to_bits`), not once for all.
- [ ] **Step 4: Run** `cargo test -p nidoo-sound` and `cargo clippy -p nidoo-sound --all-targets -- -D warnings` — expected PASS, no warnings.
- [ ] **Step 5: Commit** `feat(sound): the louder setting drives the device, the other is scaled`.

### Task 4: Registry fold and Settings → Sound sliders

**Files:**
- Modify: `crates/nidoo-core/src/registry/folding.rs`, `crates/nidoo-web/src/screens/sound_settings.rs`, `crates/nidoo-web/src/i18n/sound.rs`

**Interfaces:**
- Consumes: entity `nidoo_sound_<sid>_speech` (number) next to `nidoo_sound_<sid>_volume`.
- Produces:
  - Folding: for a device starting with `SOUND_DEVICE_PREFIX`, the anchor is the number whose unique id ends in `_volume` (falls back to today's single-number rule when none does); its switches, binary sensors, sensors and other numbers fold into it.
  - `Satellite.speech: Option<EntityView>` filled by the `_speech` suffix; the row shows two sliders labelled `SoundText::Music` ("Music" / "Musique") and `SoundText::Speech` ("Speech" / "Voix"), built by one shared slider helper (the existing volume block becomes that helper).

- [ ] **Step 1: Write the failing tests**:
  - folding.rs `satellite_with_speech_still_folds`: a sound device with `_volume` and `_speech` numbers, `_mute`/`_allowed` switches and a `_connected` binary sensor: all four other entities map to the `_volume` unique id.
  - sound_settings.rs: extend the existing satellites test fixture with a `_speech` entity and assert `speech.is_some()`, that `volume` is still the `_volume` entity, and that the satellite's room is still the `_volume` entity's area.
- [ ] **Step 2: Run** `cargo test -p nidoo-core folding` and `cargo test -p nidoo-web sound_settings` — expected FAIL.
- [ ] **Step 3: Implement** the anchor rule, the `speech` slot, the second slider and the two labels (`SoundText::Volume` stays for the Home card).
- [ ] **Step 4: Run** both test commands plus `cargo clippy -p nidoo-core -p nidoo-web --all-targets -- -D warnings` — expected PASS.
- [ ] **Step 5: Commit** `feat(web): Music and Speech sliders in Settings → Sound`.

### Task 5: Docs

**Files:**
- Modify: `docs/users/sound.md`, `docs/users/web-ui.md` (Settings → Sound row, ~line 293), `docs/reference/mqtt-topics.md` (`nidoo-sound/<sid>/…` table)

- [ ] **Step 1:** In `sound.md`, describe Music and Speech volumes, that the louder one acts at once and the quieter one after a few seconds, that Speech 0 is silent, that only the ESP32 and Mac satellites are tuned, and that the speech-bubble test button is how to set Speech by ear.
- [ ] **Step 2:** In `web-ui.md`, the Settings → Sound row has two sliders.
- [ ] **Step 3:** In `mqtt-topics.md`, add rows for `volume` and `speech` (state, `/set`, 0–100).
- [ ] **Step 4: Commit** `docs: Music and Speech volume`.

### Task 6: Mac satellite on the shared curve (branch `feat/mac-satellite-app`, PR #168)

**Files:**
- Modify: `crates/nidoo-satellite/src/playout.rs` (`fn gain`, test `gain_volume_mute_and_wake_mute`)

- [ ] **Step 1: Update the test**: volume 50 on a 0.5 sample gives `0.5 × 10^(−24.5/20)` (within 1e-4); volume 100 gives 0.5; volume 0 gives 0.
- [ ] **Step 2: Run** `cargo test -p nidoo-satellite playout::` — expected FAIL.
- [ ] **Step 3: Implement** `gain` as the shared curve (0 → 0.0, else `10^(−49 × (1 − v/100) / 20)`), mute and wake-mute unchanged.
- [ ] **Step 4: Run** `cargo test -p nidoo-satellite` — expected PASS.
- [ ] **Step 5: Commit** on `feat/mac-satellite-app`: `fix(satellite-mac): volume on the ESP32's curve`.

## After merge (owner)

- Rebuild `sound`, hub and web on the house server; Speech starts equal to Music, so nothing changes until a slider moves.
