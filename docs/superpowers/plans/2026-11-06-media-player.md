# Media Player Implementation Plan (hub, Assist, sound, web)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Any program that speaks the media player MQTT contract becomes a `media_player` entity in Nidoo: it can be controlled from the hub, flows, MCP, Assist (pause, resume, skip, ask, play by name, satellite volume) and a tinted dashboard card.

**Architecture:** A dedicated `Kind::MediaPlayer` in `nidoo-core` with a typed `MediaPlayerState`, two new `Command` variants and a per-command echo rule the hub applies in `send_command`. `nidoo-assist` gains media and volume intents that read a new `speaker_room` on `Request`. `nidoo-sound` marks its volume and mute entities with `device_class`. The web gets a `MediaPlayerCard`. The Android app is a separate plan: `2026-11-06-media-player-android.md`.

**Tech Stack:** Rust 2024 workspace (serde, chrono, tokio, leptos 0.8 wasm, wasm-bindgen-test).

**Spec:** `docs/superpowers/specs/2026-11-06-media-player-design.md` (read it first; this plan argues from it).

## Global Constraints

- **Never name the first music server** in any tracked file: code, tests, docs, commit messages, PR text. Say "a media player" / "the music server". Fixtures use neutral names ("Music", `player_music`).
- The MQTT contract is frozen (spec § "The MQTT contract"): topics, the JSON keys `state`, `title`, `artist`, `album`, `image_url`, `color`, `duration_s`, `position_s`, `position_at`, and the command JSON `{"command": "play" | "pause" | "next" | "previous"}` / `{"command": "play_media", "query": "…"}`. Do not rename them.
- `state` words: `playing`, `paused`, `idle`. Idle means every other field is null.
- `position_at` may be up to ~1 s in the future; elapsed = `position_s + max(0, now − position_at)`, clamped to `duration_s`. A null `position_at` while `playing` means the song is loading: the line stays still at `position_s` (spec amended 11-06).
- No HTTP from the hub to a player. No volume on the media player. No new override fields, `cardStyle` or `showAs` values.
- "mets …" never means play by name; only "joue …" does. Existing sentences ("mets le chauffage à 20", "passe le chauffage à 20", "coupe la lumière", "monte le volet") must parse exactly as today.
- Volume steps: 10 points for a bare "monte / baisse", 5 for "un peu".
- Assist is French-only today (the spec's "English as it already handles it" means: none). Replies are French. Do not add English parsing.
- Base the work on `origin/main` in a fresh branch `feat/media-player` (see memory `feedback_shared_worktree_sessions`: stage explicit paths, check `origin/main..HEAD` before pushing). Commit identity: `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`.
- Builds: prefix long cargo runs with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; check `~/dev-scratch/cargo-target` size and `df -h` first (memories `feedback_cargo_target_5gb_clean`, `feedback_disk_contention_builds`).
- CI gate before the PR: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo check -p nidoo-core -p nidoo-api -p nidoo-web --target wasm32-unknown-unknown`, `cargo test --workspace`, the nidoo-web wasm tests (memory `reference_wasm_tests_interactive`), `tools/ci/brand-check.sh`.

## Review Focus

1. **A hostile or sloppy `color` / `image_url`** (e.g. `"red; background:url(x)"`, `"javascript:…"`): `color` must be exactly `#rrggbb` or it is dropped (no tint), `image_url` must start with `http://` or `https://` or it is dropped (note placeholder). The web puts `color` into a `style` attribute. Pinned in Task 1 (parse) and Task 8 (card).
2. **A state message caused by someone else while a command waits** (another app pauses while Assist asked for `next`): only the echo rule for *that* command confirms it; an unrelated report keeps waiting until the timeout. Pinned in Task 3.
3. **"son" as a possessive** ("mets son bureau à 20", "allume son bureau"): only "le / du / au / de / l' son" is the sound word; a bare "son" stays a free word and the old parse is unchanged. Pinned in Task 5.
4. **A player that goes idle or offline between plan and reply** ("c'est quoi cette chanson ?" on idle → "Rien ne joue."; an offline player → "Le lecteur ne répond pas.", nothing sent). Pinned in Tasks 6 and 7.
5. **`position_at` in the future, missing `duration_s`, or `position_s` past the end**: progress never runs backwards, never exceeds 100 %, and hides without a duration. Pinned in Task 8 (`elapsed_s` tests).

---

### Task 1: Core kind and state

**Files:**
- Modify: `crates/nidoo-core/src/model/kind.rs` (enum, `ALL`, `component()`)
- Modify: `crates/nidoo-core/src/model/state.rs` (new struct and `StateValue` variant)
- Modify: `crates/nidoo-core/src/model/mod.rs` (re-export)
- Create: `crates/nidoo-core/src/state/kinds/media_player.rs`
- Modify: `crates/nidoo-core/src/state/kinds/mod.rs` (module, dispatch, `pub(crate) use media_player::STATES as MEDIA_STATES`)
- Modify: `crates/nidoo-core/src/state/mod.rs` (re-export `MEDIA_STATES` next to `VACUUM_STATES`)
- Modify (compile only, exhaustive matches): `crates/nidoo-core/src/discovery/capabilities.rs` (`Kind::MediaPlayer => Capabilities::default()` for now), `crates/nidoo-core/src/command/kinds.rs` (`Kind::MediaPlayer => Err(m.unsupported())` for now), `crates/nidoo-core/src/history/mod.rs` (`num_of` → `None`), `crates/nidoo-core/src/flow/scalar.rs`, `crates/nidoo-core/src/vocabulary.rs`, and every other `match` the compiler names in the workspace (web `icons.rs::kind_icon` → temporary `SENSOR`, `screens/home.rs::card_for` → `().into_any()`, groups `topics.rs`/`state.rs`, `nidoo-api/src/search.rs::kind_words`). Run `cargo check --workspace` and `cargo check -p nidoo-web --target wasm32-unknown-unknown` to find them.
- Test: `crates/nidoo-core/tests/discovery.rs`, `crates/nidoo-core/tests/state.rs`

**Interfaces:**
- Produces:
  - `Kind::MediaPlayer` (serde `"media_player"`, `component() == "media_player"`), declared after `WaterHeater`, before `Generic`, same place in `Kind::ALL`.
  - ```rust
    pub struct MediaPlayerState {            // Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars
        pub state: Option<String>,           // "playing" | "paused" | "idle"
        pub title: Option<String>,
        pub artist: Option<String>,
        pub album: Option<String>,
        pub image_url: Option<String>,       // http(s) only
        pub color: Option<String>,           // "#rrggbb" lowercase only
        pub duration_s: Option<f64>,         // finite, >= 0
        pub position_s: Option<f64>,         // finite, >= 0
        pub position_at_ms: Option<i64>,     // Unix ms parsed from the RFC 3339 `position_at`
    }
    ```
  - `StateValue::MediaPlayer(MediaPlayerState)` (JSON tag `"media_player"`).
  - `pub(crate) const STATES: &[&str] = &["playing", "paused", "idle"]` re-exported as `crate::state::MEDIA_STATES`.

- [ ] **Step 1: Write the failing tests**

In `tests/discovery.rs`, following the vacuum discovery tests:
- `media_player_discovers_from_three_and_two_level_topics`: `homeassistant/media_player/node1/player/config` and `homeassistant/media_player/player/config` with the spec's discovery JSON both give one entity with `kind == Kind::MediaPlayer`, `component == "media_player"`, `area` from the device as usual.

In `tests/state.rs`:
- `media_player_full_state`: the spec's example payload → `MediaPlayerState { state: Some("playing"), title, artist, album, image_url: Some("https://…"), color: Some("#3d2466"), duration_s: Some(431.0), position_s: Some(42.5), position_at_ms: Some(<2026-11-06T10:20:00Z in ms>) }`.
- `media_player_idle_clears_every_field`: `{"state":"idle","title":"x","color":"#000000"}` → only `state == Some("idle")`, all other fields `None`.
- `media_player_message_replaces_the_whole_state`: a full state, then `{"state":"paused","title":"B"}` → `artist`, `album`, `color`… are `None` (a missing field is null; no merge, unlike the vacuum).
- `media_player_ignores_unknown_fields_and_bad_values`: `{"state":"playing","volume":3,"color":"red; x:y","image_url":"javascript:alert(1)","duration_s":-4,"position_s":"x","position_at":"yesterday"}` → `state` playing, `color`, `image_url`, `duration_s`, `position_s`, `position_at_ms` all `None`.
- `media_player_unknown_state_word_is_unchanged`: `{"state":"buffering"}` → `Applied::Unchanged`, value untouched. Non-JSON payload → `Applied::Unchanged`.
- `media_player_color_is_lowercased`: `"#3D2466"` → `Some("#3d2466")`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p nidoo-core --test discovery --test state media_player`
Expected: compile errors (`Kind::MediaPlayer`, `MediaPlayerState` not defined).

- [ ] **Step 3: Implement**

`media_player::apply(entity, state, topic, payload, templates) -> Result<Applied, StateError>` with the vacuum mapper's shape (only `state_topic`, `util::parse_object`). The message builds a fresh `MediaPlayerState`; `state` must be one of `STATES` (case-insensitive) or the message is `Unchanged`. `color` passes only `^#[0-9a-fA-F]{6}$` (check by hand, no regex crate) and is lowercased; `image_url` passes only with an `http://` / `https://` prefix; numbers pass only when finite and ≥ 0; `position_at` through `chrono::DateTime::parse_from_rfc3339(..).timestamp_millis()`. Idle drops everything but `state`. Return `Changed` only when the new value differs (reuse `set_value`). Fill the compile-only matches listed above.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nidoo-core` then `cargo check --workspace && cargo check -p nidoo-web --target wasm32-unknown-unknown`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-core crates/nidoo-api/src/search.rs crates/nidoo/src/groups crates/nidoo-web/src/icons.rs crates/nidoo-web/src/screens/home.rs
git commit -m "feat(core): media player kind and state"
```

### Task 2: Capabilities, commands and the echo rule

**Files:**
- Modify: `crates/nidoo-core/src/model/entity.rs` (`Capabilities.media_features`)
- Modify: `crates/nidoo-core/src/discovery/capabilities.rs` (`fn media_player(cfg)`)
- Modify: `crates/nidoo-core/src/model/command.rs` (two variants)
- Modify: `crates/nidoo-core/src/command/mod.rs` (`command_name`, `pub mod echo` re-export)
- Modify: `crates/nidoo-core/src/command/kinds.rs` (`fn media_player(m)`)
- Create: `crates/nidoo-core/src/command/echo.rs`
- Modify: `crates/nidoo-core/src/vocabulary.rs` (`reports`, `actions`), `crates/nidoo-core/src/flow/field.rs` (`fields`, `field_scalar`), `crates/nidoo-core/src/flow/scalar.rs` (`of_state`)
- Test: `crates/nidoo-core/tests/command.rs`, unit tests in `echo.rs`, `field.rs`, `vocabulary.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `Capabilities.media_features: Vec<String>`: the subset of `["play", "pause", "next", "previous", "play_media"]` (in that order) listed in `supported_features`; when `supported_features` is absent, `["play", "pause", "next", "previous"]`.
  - `Command::MediaAction { action: String }` (JSON `{"type":"media_action","action":"next"}`; action is `play` | `pause` | `next` | `previous`) and `Command::PlayMedia { query: String }` (JSON `{"type":"play_media","query":"…"}`). `command_name`: `"media_action"`, `"play_media"`.
  - `pub fn echo_confirms(kind: Kind, cmd: &Command, before: &StateValue, after: &StateValue) -> bool` in `nidoo_core::command` (true for every non-media kind).
  - Flows: headline `Scalar::Text(state)`; fields `title`, `artist` (`Reports::Text`, not recorded); actions one bare `MediaAction` per supported action plus, with `play_media`, `PlayMedia { query: "" }` with `Param::Text { key: "query", secret: false, optional: false }`.

- [ ] **Step 1: Write the failing tests**

`tests/command.rs` (helper style of the vacuum tests):
- `media_actions_publish_json_commands`: each of play/pause/next/previous → one publish on `command_topic`, payload `{"command":"next"}`, not retained.
- `play_media_publishes_the_query_as_heard`: `PlayMedia { query: "du Daft Punk" }` → `{"command":"play_media","query":"du Daft Punk"}` (compare parsed JSON, not strings).
- `media_command_outside_supported_features_is_unsupported`: features `["play","pause"]`, `next` → `CommandError::Unsupported { .. }`; `PlayMedia` → `Unsupported`; nothing published.
- `play_media_needs_words`: `query: "  "` → `CommandError::OutOfRange(_)`.
- `media_player_refuses_other_commands`: `TurnOn` → `Unsupported`.
- capabilities: `media_features_follow_supported_features_in_order` (`["play_media","play"]` → `["play","play_media"]`), `media_features_default_without_the_key`.

`echo.rs` unit tests, `p(state, title, at_ms)` building `StateValue::MediaPlayer`:
- `play_is_confirmed_by_playing`: before paused, after playing → true; after paused → false.
- `pause_is_confirmed_by_paused`.
- `next_needs_a_new_title_or_a_changed_position_at`: same title same `position_at_ms` → false; new title → true; same title, `position_at_ms` 1000 → 2000 → true; before `None`, after `Some` → true; before `Some`, after `None` (loading) → true.
- `previous_restarting_the_same_song_is_confirmed` (same title, newer `position_at_ms`).
- `play_media_needs_playing_and_news`: after paused → false; playing + same title + same `position_at_ms` → false; playing + same title + newer `position_at_ms` → true; playing + new title + `position_at_ms` `None` (loading) → true.
- `before_unknown_counts_as_empty`: before `StateValue::Unknown`, `next`, after with a title → true.
- `other_kinds_always_confirm`: `Kind::Switch`, any values → true.

`field.rs` / `vocabulary.rs`: `media_player_fields_are_title_and_artist`, `media_player_reports_its_three_states`, `media_player_actions_follow_features` (play_media carries the `query` text param).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p nidoo-core media`
Expected: compile errors on the new variants and functions.

- [ ] **Step 3: Implement**

`fn media_player(m: &Mapper) -> Result<Vec<Publish>, CommandError>` in `kinds.rs`: action/query checked against `m.entity.capabilities.media_features` (`m.unsupported()` when missing), query trimmed and non-empty (the trimmed text is sent), payload built with `serde_json::json!`, published on `command_topic` through `m.publish("command_topic", None, &payload, &[])`. `echo_confirms`: rules exactly as the spec's echo table plus the changed-`position_at` clause (newer, or `Some` → `None`) for `next`, `previous` and `play_media`; a `before` that is not `MediaPlayer` reads as `MediaPlayerState::default()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nidoo-core && cargo check -p nidoo-core --target wasm32-unknown-unknown`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-core
git commit -m "feat(core): media player commands, capabilities and echo rule"
```

### Task 3: Hub — confirmation, groups, search, MCP

**Files:**
- Modify: `crates/nidoo/src/hub.rs` (`send_command`, ~line 1305)
- Modify: `crates/nidoo-api/src/ambiances.rs` (`already_there`: pause when paused, play when playing → true)
- Modify: `crates/nidoo-api/src/search.rs` (`kind_words`: `&["media_player", "musique", "music", "lecteur", "player"]`)
- Modify: `crates/nidoo/src/mcp/mod.rs`, `crates/nidoo/src/mcp/params.rs` (`send_command` docs: the two command shapes; kind list gains `media_player`)
- Test: `crates/nidoo/tests/hub.rs`, `crates/nidoo-api` unit tests next to `already_there` and `kind_words`, `crates/nidoo/tests/groups*.rs` (whichever holds the group-kind refusal tests)

**Interfaces:**
- Consumes: `echo_confirms` (Task 2).
- Produces: `Hub::send_command` for a media player resolves `accepted: true` only on a state that passes `echo_confirms`; otherwise `accepted: false`, reason `"no state change within N ms"` (unchanged wording, so `reason_fr` keeps mapping it).

- [ ] **Step 1: Write the failing tests**

In `tests/hub.rs`, a `feed_player(&hub)` helper ingesting the spec's discovery JSON and a playing state:
- `media_next_is_confirmed_by_a_new_title`: send `MediaAction { action: "next" }`, echo a state with another title → `accepted`, `state` carries the new title.
- `media_next_ignores_a_report_that_changes_nothing_it_needs`: echo the same title and same `position_at` but `state: "paused"` (someone else paused) → `accepted: false` with the timeout reason.
- `media_pause_on_a_player_already_paused_is_confirmed_by_the_retained_report` (same-state report through `state_seen`).
- `media_player_cannot_join_a_group`: creating a group with a media player member is refused (assert the existing refusal error of the group store / API).
- In nidoo-api: `already_there_for_media` (pause/paused true, play/playing true, next never), `search_finds_players_by_music_words` ("musique" and "player" hit a media player).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p nidoo --test hub media && cargo test -p nidoo-api media`
Expected: FAIL (`next` accepted on the paused report; search finds nothing).

- [ ] **Step 3: Implement**

In `send_command`, keep `entity.protocol_kind` from the read block and, in both `select!` arms, accept the candidate state only when `echo_confirms(kind, &cmd, &before, &state.value)`; otherwise keep looping. Group refusal: the media player must simply be absent from the groupable kind lists (`groups/topics.rs`, web `group_wizard.rs`); add it nowhere. MCP text: name `{"type":"media_action","action":"play|pause|next|previous"}` and `{"type":"play_media","query":"…"}`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nidoo --test hub && cargo test -p nidoo-api`
Expected: PASS (the existing hub tests too: other kinds unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo/src/hub.rs crates/nidoo/src/mcp crates/nidoo/tests/hub.rs crates/nidoo-api/src
git commit -m "feat(hub): confirm media commands by their own echo"
```

### Task 4: nidoo-sound volume and mute markers

**Files:**
- Modify: `crates/nidoo-sound/src/mqtt.rs` (the `volume` number and `mute` switch configs, ~lines 58-85)
- Test: same file's `tests` module

**Interfaces:**
- Produces: the Music volume `number` discovery carries `"device_class": "volume"`; the Mute `switch` carries `"device_class": "mute"`. The Speech volume number gets **no** marker (Assist must only touch music volume). The hub keeps them in `Entity.device_class` as plain strings (already the case: `discovery/parse.rs:250`).

- [ ] **Step 1: Write the failing test**

`volume_and_mute_carry_their_markers`: in `discovery_messages(&prefs(), "homeassistant")`, the `nidoo_sound_{sid}_volume` config has `device_class == "volume"`, `nidoo_sound_{sid}_mute` has `"mute"`, `nidoo_sound_{sid}_speech` has no `device_class`. Also, in `crates/nidoo-core/tests/discovery.rs`, `unknown_device_class_is_kept`: a number with `"device_class": "volume"` → `entity.device_class == Some("volume")`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nidoo-sound volume_and_mute && cargo test -p nidoo-core --test discovery unknown_device_class`
Expected: the sound test FAILS (no key); the core test PASSES already (it only pins the assumption; if it fails, stop and pick the "equivalent explicit key" the spec allows, and note it in the spec).

- [ ] **Step 3: Implement** — add the two keys to the `json!({…})` extras.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nidoo-sound`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-sound/src/mqtt.rs crates/nidoo-core/tests/discovery.rs
git commit -m "feat(sound): mark the music volume and mute for the assistant"
```

### Task 5: Assist — media and volume intents

**Files:**
- Modify: `crates/nidoo-assist/src/lexicon.rs` (new classes and words)
- Modify: `crates/nidoo-assist/src/intent.rs` (`Intent` variants, `media()` branch in `parse`, `play_by_name`)
- Test: unit tests in `intent.rs` and `lexicon.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum MediaIntent {
      Pause,
      Play { resume: bool },          // resume: "reprends / relance / remets la musique"
      Next,
      Previous,
      NowPlaying,
      PlayMedia { query: String },    // the words after "joue", as typed
  }
  Intent::Media(MediaIntent)
  Intent::Volume { change: Change, room: Vec<String> }   // reuses `Change`; Relative in points
  Intent::Mute { on: bool, room: Vec<String> }           // on = cut the sound
  Intent::MediaHint      // "mets de la musique classique": answered with the "joue" hint
  Intent::VolumeRange    // an absolute volume outside 0–100
  pub const VOLUME_STEP: f64 = 10.0;
  pub const VOLUME_LITTLE: f64 = 5.0;
  /// "joue …" (any form of jouer) anywhere: `Some(Intent::Media(..))`, read on the raw sentence.
  pub fn play_by_name(sentence: &str) -> Option<Intent>;
  ```
- New lexicon classes: `Media` (musique(s), chanson(s), morceau(x), titre(s)), `Sound` (son, volume), `Pause` (pause), `Next` (suivante, suivant), `Previous` (precedente, precedent), `Resume` (reprends, reprend, reprendre, relance, relancer, relances, remets, remettre), `Launch` (lance, lancer, lances, lancez), `Stop` (stop), `Back` (reviens, revenir, retourne), `Listen` (ecoute, ecoutes, ecouter, ecoutez). Add "quoi" to `Question`. None of these may be fuzzy (exact match only, keep them out of `fuzzy()`): "son"/"sont", "titre"/"titres" are one edit apart from other words.

Rules (in `parse`, before the percent/`OtherDevice` early returns, and only when no `Light`, `LightNamed`, `Cover`, `Heating`, `Temperature`, `Degree`, `Tv`, `Plug`, `OtherDevice` or `Negation` word is present; if `media()` returns `None`, the old path runs unchanged):
- `Sound` counts only when the token before it is `le`, `du`, `au`, `de` or `l` ("le son"); otherwise "son" becomes a `Free` word before anything else reads the tokens.
- A media sentence needs one of `Media`, `Sound`, `Pause`, `Next`, `Previous`, `Stop`, `Listen`, or `Back` followed by the free word "arriere".
- Pause: `Pause` (with or without `Set`), or `Stop` / `Off` + `Media`.
- Play: `Resume` + `Media` → `Play { resume: true }`; `Set` / `Launch` + `Media`, no free word, no number → `Play { resume: false }`; `Set` / `Launch` + `Media` + free words → `MediaHint`.
- Next / Previous: `Next` → `Next`; `Previous` or `Back`+"arriere" → `Previous`; both → `None`. A bare `Set` ("passe") is never `Next`.
- NowPlaying: (`Media` or `Listen`) with `Question` (incl. "quoi", "?") or `Statement`, and no action verb.
- Volume: `Raise` / `Lower` + (`Sound` or `Media`) → `Relative(±VOLUME_STEP)`, `±VOLUME_LITTLE` with `Little`, `±n` with "de n"; `Set` + `Sound` + "à n" (with or without %) → `Absolute(n)`; outside 0–100 → `VolumeRange`. Free words are the room.
- Mute: `Off` + `Sound` → `Mute { on: true }`; `Resume` + `Sound` → `Mute { on: false }`.
- `play_by_name`: split the raw sentence on whitespace; the first word whose `fold` is `joue`, `joues`, `jouez` or `jouer` starts it; the rest, trimmed of spaces and trailing `?!.`, is the query, as typed (case and accents kept). Empty, or (after one leading article du / de la / de l' / des / le / la / les / un / une) folding to `musique`, `de la musique`, `quelque chose`, `un morceau`, `une chanson` → `Play { resume: false }`.

- [ ] **Step 1: Write the failing tests** (`intent.rs` tests, helper `p(s)`; add `m(s)` for `play_by_name`)

```rust
assert_eq!(p("mets pause"), Some(Intent::Media(MediaIntent::Pause)));
assert_eq!(p("pause la musique"), Some(Intent::Media(MediaIntent::Pause)));
assert_eq!(p("stop la musique"), Some(Intent::Media(MediaIntent::Pause)));
assert_eq!(p("arrête la musique"), Some(Intent::Media(MediaIntent::Pause)));
assert_eq!(p("reprends la musique"), Some(Intent::Media(MediaIntent::Play { resume: true })));
assert_eq!(p("relance la musique"), Some(Intent::Media(MediaIntent::Play { resume: true })));
assert_eq!(p("mets de la musique"), Some(Intent::Media(MediaIntent::Play { resume: false })));
assert_eq!(p("lance de la musique"), Some(Intent::Media(MediaIntent::Play { resume: false })));
assert_eq!(p("mets de la musique classique"), Some(Intent::MediaHint));
assert_eq!(p("chanson suivante"), Some(Intent::Media(MediaIntent::Next)));
assert_eq!(p("passe à la suivante"), Some(Intent::Media(MediaIntent::Next)));
assert_eq!(p("chanson précédente"), Some(Intent::Media(MediaIntent::Previous)));
assert_eq!(p("reviens en arrière"), Some(Intent::Media(MediaIntent::Previous)));
assert_eq!(p("c'est quoi cette chanson ?"), Some(Intent::Media(MediaIntent::NowPlaying)));
assert_eq!(p("qu'est-ce qu'on écoute ?"), Some(Intent::Media(MediaIntent::NowPlaying)));
assert_eq!(p("monte le son"), Some(Intent::Volume { change: Change::Relative(10.0), room: vec![] }));
assert_eq!(p("baisse un peu le son"), Some(Intent::Volume { change: Change::Relative(-5.0), room: vec![] }));
assert_eq!(p("monte le son de 20"), Some(Intent::Volume { change: Change::Relative(20.0), room: vec![] }));
assert_eq!(p("mets le son à 60 %"), Some(Intent::Volume { change: Change::Absolute(60.0), room: vec![] }));
assert_eq!(p("mets le son à 140"), Some(Intent::VolumeRange));
assert_eq!(p("baisse le son du salon"), Some(Intent::Volume { change: Change::Relative(-10.0), room: v(&["salon"]) }));
assert_eq!(p("coupe le son"), Some(Intent::Mute { on: true, room: vec![] }));
assert_eq!(p("remets le son"), Some(Intent::Mute { on: false, room: vec![] }));
// unchanged sentences (assert against what `parse` returns on origin/main, copied into the test)
assert_eq!(p("mets le chauffage à 20"), set(Change::Absolute(20.0), &[]));
assert_eq!(p("passe le chauffage à 20"), set(Change::Absolute(20.0), &[]));
assert_eq!(p("coupe la lumière"), lights(false, false, &[], &[]));
assert!(matches!(p("monte le volet"), Some(Intent::Covers { open: true, .. })));
assert_eq!(p("passe"), None);
assert_eq!(p("mets son bureau à 20"), set(Change::Absolute(20.0), &["bureau"]));  // "son" possessive: the room is "bureau"
assert_eq!(p("ne coupe pas le son"), None);
// play by name
assert_eq!(m("joue du Daft Punk"), Some(Intent::Media(MediaIntent::PlayMedia { query: "du Daft Punk".into() })));
assert_eq!(m("Joue Beyoncé !"), Some(Intent::Media(MediaIntent::PlayMedia { query: "Beyoncé".into() })));
assert_eq!(m("peux-tu jouer la vie en rose"), Some(Intent::Media(MediaIntent::PlayMedia { query: "la vie en rose".into() })));
assert_eq!(m("joue de la musique"), Some(Intent::Media(MediaIntent::Play { resume: false })));
assert_eq!(m("joue"), Some(Intent::Media(MediaIntent::Play { resume: false })));
assert_eq!(m("mets Daft Punk"), None);
```
(The "unchanged" expectations must be copied from running `parse` on `origin/main` first; if one differs from the line above, keep the origin/main value.) Lexicon: the existing `no_word_is_in_two_tables` must still pass.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p nidoo-assist intent`
Expected: compile errors, then failures on the media rows.

- [ ] **Step 3: Implement** the classes, the "son" possessive rewrite in `tag()` or right after it, `fn media(toks, roles, has) -> Option<Intent>`, and `play_by_name`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p nidoo-assist`
Expected: PASS, including `tests/regression.rs` unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-assist/src/lexicon.rs crates/nidoo-assist/src/intent.rs
git commit -m "feat(assist): understand music and volume sentences"
```

### Task 6: Assist — plans and replies

**Files:**
- Modify: `crates/nidoo-assist/src/plan.rs` (`Request`, `Action`, `Done`, `plan`, `plan_request`, `fn media`, `fn volume`, `fn mute`)
- Modify: `crates/nidoo-assist/src/reply.rs` (`ActionResult::Echo`, `done` for the new `Done` variants)
- Modify: `crates/nidoo-assist/src/fixture.rs` (players and satellite controls)
- Modify: `crates/nidoo-assist/src/pending.rs` (`examples` unchanged unless the compiler asks)
- Test: `crates/nidoo-assist/tests/regression.rs`, unit tests in `plan.rs` and `reply.rs`

**Interfaces:**
- Consumes: Task 5 intents; `Kind::MediaPlayer`, `Command::MediaAction`, `Command::PlayMedia`, `MediaPlayerState`, `Capabilities.media_features` (Tasks 1–2).
- Produces:
  - `Request` gains `pub player: Option<String>` (a chosen player id) and `pub speaker_room: Option<String>`; `Request::new` sets both `None`.
  - `pub fn plan(sentence: &str, house: &House, speaker_room: Option<&str>) -> Plan` — tries `play_by_name` first, then `parse`; the request it builds carries `speaker_room`. Update every caller (hub, tests, `pending.rs`).
  - `Action::Media { entity: String, command: Command }`, `Action::Speaker { entity: String, command: Command }` (volume `SetNumber`, mute `TurnOn`/`TurnOff`).
  - `Done::Media { player: String /* name */, what: MediaIntent }`, `Done::Volume { area: String, level: Option<u8> }`, `Done::Mute { area: String, on: bool }`.
  - `ActionResult::Echo(MediaPlayerState)`: accepted, with the player's state after the command.

Rules:
- **Players** = entities with `kind == Kind::MediaPlayer` and `commandable(e)`. Pick: `req.player`; else the one online player; else the online players in `req.speaker_room`, when exactly one; else ask "Quel lecteur ?" with one choice per player (≤ `MAX_CHOICES`, each `Request` with `player` set). No player: "Il n'y a pas de lecteur de musique." Players exist but none online: "Le lecteur ne répond pas.", nothing sent.
- **Capabilities:** an action the player lacks answers "Ce lecteur ne sait pas faire ça." and `PlayMedia` without `play_media` answers "Ce lecteur ne sait pas chercher un titre."; nothing sent.
- **NowPlaying:** plays/paused with a title → "<title>, de <artist>." ("<title>." without artist; prefix "En pause : " when paused); otherwise "Rien ne joue."
- **Volume / Mute:** room = `req.area`, else the room words (`resolve_room`), else `req.speaker_room`, else `ask_room` among the areas that hold a volume control. Controls in the room: `Kind::Number` with `device_class == Some("volume")` (volume) or `Kind::Switch` with `device_class == Some("mute")` (mute), `commandable`. None: "Pas d'enceinte <dans le salon>." Relative: each control's `StateValue::Number` ± step, rounded, clamped 0–100; controls with no known value are left out; none left: "Je ne connais pas le volume actuel." Absolute: the value for each.
- **Replies** (`reply::done`, French, naming the room for volume like the other replies):

  | Done | `Echo(s)` | `Ok` (simulation, no state) | `Skipped` |
  |---|---|---|---|
  | `Pause` | "Musique en pause." | same | "La musique est déjà en pause." |
  | `Play { resume: true }` | "Je reprends <title>." | "Je reprends la musique." | "La musique joue déjà." |
  | `Play { resume: false }`, `PlayMedia` | "Je joue <title>." | "Je lance la musique." / "Je cherche « q »." | "La musique joue déjà." |
  | `Next`, `Previous` | "<title>, <artist>." | "Chanson suivante." / "Chanson précédente." | — |
  | `Volume` | — | "Volume à 60 % dans le salon." | — |
  | `Mute` | — | "Son coupé dans le salon." / "Son remis dans le salon." | — |

  `Offline` → "Le lecteur ne répond pas."; `Failed(r)` with `r == "pas de réponse de l'appareil"` → "Le lecteur n'a pas répondu."; any other `Failed` keeps the existing refusal style. A missing title in an `Echo` falls back to the `Ok` column.
- `MediaHint` → "Pour un titre ou un artiste, dites « joue » suivi du nom."; `VolumeRange` → "Le volume va de 0 à 100 %." Trace intents: `media_pause`, `media_play`, `media_resume`, `media_next`, `media_previous`, `media_now_playing`, `media_play_by_name`, `volume`, `mute`.

- [ ] **Step 1: Write the failing tests**

Fixture: add `player_music` ("Musique", area `salon`, features all five, playing "Get Lucky" by "Daft Punk") and, in a second fixture fn `two_players()`, `player_bureau` in `bureau`; add satellite controls `number_salon_volume` (`device_class: "volume"`, value 40) and `switch_salon_mute` (`device_class: "mute"`, off) in `salon`, and `number_bureau_volume` in `bureau`. `ROWS` is a `const`, so a `Command` holding a `String` cannot sit in it: extend `Expect` with `Media(&'static str /*entity*/, &'static str /*action, or "play_media"*/, Option<&'static str> /*query*/)` and `Speaker(&'static [(&'static str, Command)])` (`SetNumber`, `TurnOn`, `TurnOff` are const-friendly). Rows (the existing rows stay and must still pass):

```
("mets pause", Media("player_music", "pause", None))
("chanson suivante", Media("player_music", "next", None))
("joue du Daft Punk", Media("player_music", "play_media", Some("du Daft Punk")))
("mets de la musique", Media("player_music", "play", None))
("c'est quoi cette chanson ?", Answer("Get Lucky, de Daft Punk."))
("monte le son du salon", Speaker(&[("number_salon_volume", Command::SetNumber { value: 50.0 })]))
("baisse un peu le son du salon", Speaker(&[("number_salon_volume", Command::SetNumber { value: 35.0 })]))
("coupe le son du salon", Speaker(&[("switch_salon_mute", Command::TurnOn { brightness: None, color: None, effect: None })]))
("monte le son", Ask(&["Salon", "Bureau"]))   // web: no speaker room; the rooms with a volume control
```
`plan.rs` unit tests (they need `speaker_room`): `two_players_pick_the_speaker_room` (`plan("mets pause", &house, Some("bureau"))` → `player_bureau`), `two_players_without_speaker_room_ask_which` (labels both names; answering choice 1 resolves to `player_bureau` via `Pending::resolve` + `plan_request`), `speaker_room_survives_the_question` (the asked `Request`s keep `speaker_room`), `volume_uses_the_speaker_room` (`plan("monte le son", …, Some("salon"))` → `number_salon_volume` 50), `no_player` / `offline_player_answers_it_does_not_answer`, `player_without_play_media_cannot_search`, `idle_now_playing_says_nothing_plays`. `reply.rs`: one test per table row above, plus the timeout row.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p nidoo-assist`
Expected: compile errors, then failures.

- [ ] **Step 3: Implement** the rules above, mirroring `switches()` for room picking and `ask_room` for questions.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p nidoo-assist && cargo check -p nidoo-assist --target wasm32-unknown-unknown`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-assist
git commit -m "feat(assist): plan music, volume and mute requests"
```

### Task 7: Hub — Assist wiring

**Files:**
- Modify: `crates/nidoo/src/assist/hub.rs` (`assist_at`, `what`, `execute`)
- Test: `crates/nidoo/tests/assist_hub.rs`

**Interfaces:**
- Consumes: `plan(…, speaker_room)`, `Action::Media`, `Action::Speaker`, `ActionResult::Echo` (Task 6); `crate::sound::room_of(&Registry, sid)`.
- Produces: a voice turn (`client` = `satellite:<sid>`) plans with `speaker_room = sound::room_of(&reg, sid)` (computed inside the same read block as `area_views`); a web turn passes `None`.

- [ ] **Step 1: Write the failing tests** (`assist_hub.rs`, with a `player_echo(rx, hub)` task in the style of `plug_echo` that answers each published command with a matching state: `play` → playing, `pause` → paused, `next` → title "Song 2", `play_media` → playing with title "Found")
- `joue_x_sends_play_media_and_replies_with_the_echoed_title`: text "joue du Daft Punk" → published `{"command":"play_media","query":"du Daft Punk"}`; reply "Je joue Found."
- `next_replies_with_the_new_song`: reply "Song 2, <artist>."
- `a_player_that_never_echoes_times_out`: no echo task → reply "Le lecteur n'a pas répondu.", outcome `Failed`.
- `satellite_turn_uses_its_room`: two players (salon, bureau), a sound satellite whose `_volume` entity is in `bureau`, client `satellite:<sid>`, "mets pause" → the bureau player gets `{"command":"pause"}`; the same text from client `web-1` → a question listing both.
- `simulated_media_turn_sends_nothing`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p nidoo --test assist_hub media`
Expected: FAIL.

- [ ] **Step 3: Implement**: `Action::Media` and `Action::Speaker` join the `Light | Cover | Switch` arm of `execute`; for `Action::Media`, an accepted response whose `state.value` is `StateValue::MediaPlayer(s)` becomes `ActionResult::Echo(s)`. `what()` labels: "pause", "lecture", "suivante", "précédente", "jouer « q »", "volume 60 %", "couper le son", "remettre le son".

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p nidoo --test assist_hub`
Expected: PASS (all existing assist hub tests too).

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo/src/assist/hub.rs crates/nidoo/tests/assist_hub.rs
git commit -m "feat(assist): voice turns know their room; music replies read the echo"
```

### Task 8: Web card (design B)

**Files:**
- Create: `crates/nidoo-web/src/cards/media_player.rs`
- Modify: `crates/nidoo-web/src/cards/mod.rs`, `crates/nidoo-web/src/screens/home.rs` (`card_for`: `Kind::MediaPlayer => <MediaPlayerCard/>`)
- Modify: `crates/nidoo-web/src/icons.rs` (new `MEDIA_PLAYER` glyph, `kind_icon`, plus `PREVIOUS` / `NEXT` / `PAUSE` glyphs if the set lacks them; `PLAY` exists), `crates/nidoo-web/src/i18n/*` (kind name, states playing/paused/idle in `states.rs`, "Nothing playing" / "Rien ne joue", "Play some music" / "Lancer de la musique", button labels for screen readers), `crates/nidoo-web/src/format.rs` (`StateValue::MediaPlayer` → state word), `crates/nidoo-web/src/flow_text.rs` and `if_value.rs` (`StateCtx::Media`), `crates/nidoo-web/src/command_form.rs` (labels and keys `media:<action>`, `play_media` with its text field), `crates/nidoo-web/src/dashboards/library.rs` (the kind is listed; it does not chart), `crates/nidoo-web/styles/cards.css`
- Test: unit tests in `media_player.rs`; `crates/nidoo-web/tests/rich_cards.rs` (wasm)

**Interfaces:**
- Consumes: `MediaPlayerState`, `Capabilities.media_features`, `Command::MediaAction`/`PlayMedia` (Tasks 1–2).
- Produces: `#[component] pub fn MediaPlayerCard(entity: EntityView) -> impl IntoView`; pure helpers
  - `pub(crate) fn elapsed_s(s: &MediaPlayerState, now_ms: i64) -> Option<f64>` — `None` without `duration_s`; paused → `position_s`; playing → `position_s + max(0, now − position_at_ms)/1000`; clamped to `0..=duration_s`.
  - `pub(crate) fn tint_style(color: Option<&str>) -> Option<String>` — `Some("--media-tint: #3d2466")` only for a valid `#rrggbb` (re-checked here, defence in depth), else `None`.

Card per spec § "Web card": `CardFrame` with `wide=!in_sheet` (like the vacuum); cover `<img>` from `image_url` (`referrerpolicy="no-referrer"`, `on:error` swaps to the note placeholder); label "name · room", bold title, muted "artist · album"; ⏮ ▶/❚❚ ⏭ as drawn SVG from `icons.rs` (32 px round tiles, 40 px amber play/pause with a dark icon and glow); progress line from `elapsed_s`, refreshed once a second while playing (`gloo_timers` interval dropped with the component), hidden when `None`. Buttons hidden when missing from `media_features`. Idle: note placeholder, title "Rien ne joue", ▶ with hint, sends `MediaAction { action: "play" }`. Tint: CSS reads `--media-tint` (`background: linear-gradient(135deg, var(--media-tint), color-mix(in srgb, var(--media-tint) 55%, var(--bg)))`, border `color-mix(in srgb, var(--media-tint) 60%, white)`, cover glow, `transition: background 0.6s`), white text in both Night and Ivoire; a thin dark ring on the play button under a tint. No tint → the plain card. Unavailable → the usual offline treatment of `CardFrame`.

- [ ] **Step 1: Write the failing tests**

Unit (native): `elapsed_waits_for_a_future_position_at` (`position_at_ms = now + 800` → `position_s`), `elapsed_moves_while_playing` (+3 s), `elapsed_is_clamped_to_the_duration`, `elapsed_is_none_without_duration`, `paused_elapsed_is_the_position`, `loading_elapsed_stays_at_the_position` (playing, `position_at_ms` `None`), `tint_accepts_only_hex` (`"#3d2466"` ok; `"red"`, `"#3d2466; x:y"`, `None` → `None`).

wasm (`rich_cards.rs`, `entity(…, Kind::MediaPlayer, StateValue::MediaPlayer(…))`): `media_card_playing_shows_cover_title_and_pause` (img `src`, title text, a pause button, the `--media-tint` style), `media_card_paused_shows_play`, `media_card_idle_shows_the_placeholder_and_play`, `media_card_without_color_is_plain` (no `--media-tint`), `media_card_hides_buttons_the_player_lacks` (features `["play","pause"]` → no next/previous), `media_card_offline` (offline class like the other cards), `media_card_follows_the_language` (French then English labels).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p nidoo-web media` and the wasm runner (`wasm-pack test --headless --chrome crates/nidoo-web -- --test rich_cards`, or the interactive runner from memory `reference_wasm_tests_interactive`)
Expected: compile errors, then failures.

- [ ] **Step 3: Implement** the card, glyphs, strings, CSS and the exhaustive-match entries.

- [ ] **Step 4: Run to verify they pass**

Run: same commands, plus `cargo clippy -p nidoo-web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS. Then look at it: run the web against a hub fed with a fake player (publish the spec's discovery and state with `mosquitto_pub` or a test broker) in Night and Ivoire, phone and desktop widths; screenshot with headless Chrome (memory `reference_headless_chrome_cdp_screenshots`).

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-web
git commit -m "feat(web): media player card tinted by the cover colour"
```

### Task 9: Docs and the name guard

**Files:**
- Modify: `docs/users/assistant.md` (new phrases, the "joue" rule, volume by room)
- Create: `docs/users/media-players.md` (how any program becomes a media player: the contract written generically, the card, flows, Assist)
- Modify: `docs/users/sound.md` (one paragraph: the volume and mute markers, and that other speakers can publish them)
- Modify: `tools/ci/brand-check.sh` (fail when the first music server's name appears in any tracked file outside `docs/superpowers/`; store the word base64-encoded like the existing Google-brand check, so the script itself does not name it; the executor asks the owner for the exact word — it is in the session memory `project_media_player_2026-11-06`, not in this plan)
- Test: `tools/ci/brand-check.sh` run

- [ ] **Step 1:** Write the docs. The contract section copies the spec's topics and JSON verbatim, with `<root>` / `<node_id>` placeholders and no server named.
- [ ] **Step 2:** Add the brand-check rule; run `tools/ci/brand-check.sh`. Expected: exit 0. Then temporarily put the word in an untracked-then-staged scratch file, run it, expect exit 1, and remove the file.
- [ ] **Step 3:** Run the full CI gate from Global Constraints. Expected: all green.
- [ ] **Step 4: Commit**

```bash
git add docs/users/assistant.md docs/users/media-players.md docs/users/sound.md tools/ci/brand-check.sh
git commit -m "docs: media players, music phrases and the volume markers"
```

---

## After the branch

- Open the PR against `main` (one PR; the Android plan ships in Nidoo-app separately).
- After merge: rebuild **hub + web + sound** on the house server (`docker compose up -d --build`), check the sound discovery now carries the markers, then point the music server at the broker and try every Assist phrase from a satellite.
