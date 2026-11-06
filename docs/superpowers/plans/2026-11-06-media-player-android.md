# Media Player Implementation Plan (Android app)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Nidoo Android app discovers `media_player` entities itself and shows them as a tinted card with play/pause, a control sheet with ⏮ ▶/❚❚ ⏭ and progress, a widget tile, and a demo-house entry.

**Architecture:** A new `Kind.MEDIA_PLAYER` mapped by a `MediaPlayerMapper : KindMapper`, following `VacuumMapper` end to end (discovery map, `StateValue`, `Command`, `Capabilities`, optimistic prediction). The home card gains a per-entity tint through a new `NidooCard` parameter; covers load with Coil 3 (already in the catalog).

**Tech Stack:** Kotlin 2.4 (KMP `core:model` / `core:state`), Jetpack Compose, Hilt, Coil 3.5, Glance 1.2, kotlin.test, JUnit4 + Robolectric + `createComposeRule`.

**Spec:** Oikos-os `docs/superpowers/specs/2026-11-06-media-player-design.md`, § "Android app" and § "The MQTT contract" (read with `git show origin/main:docs/superpowers/specs/2026-11-06-media-player-design.md` in Oikos-os once merged, or from branch `docs/media-player-design`). Hub-side plan: `2026-11-06-media-player.md`.

## Global Constraints

- **Never name the first music server** in code, tests, strings, demo house, commits or PR text. The demo entity is "Music".
- The MQTT contract is frozen: discovery component `media_player`; state JSON keys `state` (`playing` | `paused` | `idle`), `title`, `artist`, `album`, `image_url`, `color`, `duration_s`, `position_s`, `position_at` (RFC 3339); commands `{"command":"play"|"pause"|"next"|"previous"}` and `{"command":"play_media","query":"…"}` on `command_topic`, not retained.
- Same value rules as the hub: a message replaces the whole state (missing = null); idle nulls every other field; `color` only `#rrggbb` (lowercased); `image_url` only `http(s)://`; numbers only finite and ≥ 0; an unknown `state` word ignores the message.
- Progress: `position_s + max(0, now − position_at)`, clamped to `duration_s`; hidden without a duration. While `playing` with a null `position_at` (the song is loading), the line stays still at `position_s`.
- No volume control and no search field in the app. The widget tile only displays.
- Every string in EN (`values/strings.xml`) and FR (`values-fr/strings.xml`); `StringParityTest` enforces it.
- Branch `feat/media-player` from `origin/main` in Nidoo-app; commit identity `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`. Gradle caches stay in `$DEV_SCRATCH` (no per-repo overrides).

## Review Focus

1. **A malformed `color` or `image_url`** must never reach `Color(...)` parsing or Coil: dropped at decode (Task 1), so the card is plain and shows the note placeholder.
2. **A cover that fails to load** (404, offline CDN): the note placeholder replaces it (Task 2 `error` slot).
3. **`position_at` in the future / no duration / position past the end**: progress never negative, never > 1, hidden without duration (Task 2 `mediaProgress` tests).
4. **A player missing `next`/`previous`/`pause`**: those buttons are absent on the card and in the sheet (Task 2).
5. **next/previous pressed while a prediction is pending**: no guessed title is shown; the card keeps the old title until the echo (Task 1 `predict` returns null).

---

### Task 1: Model and mapper

**Files:**
- Modify: `core/model/src/commonMain/kotlin/com/gekkotron/nidoo/model/Kind.kt` (add `MEDIA_PLAYER` after `WATER_HEATER`)
- Modify: `core/model/src/commonMain/kotlin/com/gekkotron/nidoo/model/StateValue.kt`, `Command.kt`, `Entity.kt` (`Capabilities.mediaFeatures`)
- Create: `core/state/src/commonMain/kotlin/com/gekkotron/nidoo/state/kinds/MediaPlayerMapper.kt`
- Modify: `core/state/src/commonMain/kotlin/com/gekkotron/nidoo/state/kinds/KindMapper.kt` (`forKind`)
- Modify: `core/source/src/jvmMain/kotlin/com/gekkotron/nidoo/source/discovery/DiscoveryParser.kt` (`KINDS`, `capabilities`; add `MEDIA_PLAYER` to `REQUIRES_COMMAND`)
- Modify (compile only, exhaustive `when`s): `feature/home/.../presentation/HomeUiBuilder.kt` `tapFor` (→ `TapAction.OPEN_SHEET`; add to `SHEET_KINDS`), `feature/home/.../ui/KindIcon.kt` `iconSpec` (temporary `IconSpec.Static(NidooGlyph.PLAY, false)`), `feature/home/.../presentation/CardFormatter.kt` (temporary `CardValue.Text`-style fallback), `feature/widget/.../TileMapper.kt` `valueOf`/`iconOf`, `feature/widget/.../TileActions.kt` `forTap` (→ `TileAction.OpenHome`). Build both feature modules to find any others.
- Test: `core/state/src/commonTest/kotlin/com/gekkotron/nidoo/state/kinds/MediaPlayerMapperTest.kt`, `core/source/src/jvmTest/.../discovery/DiscoveryParserTest.kt`

**Interfaces:**
- Produces:
  ```kotlin
  data class MediaPlayer(
      val state: String?, val title: String?, val artist: String?, val album: String?,
      val imageUrl: String?, val color: String?,
      val durationS: Double?, val positionS: Double?, val positionAtMs: Long?,
  ) : StateValue
  data class MediaAction(val action: String) : Command   // play | pause | next | previous
  data class PlayMedia(val query: String) : Command
  // Capabilities
  val mediaFeatures: List<String> = emptyList()           // subset of play, pause, next, previous, play_media, in that order
  object MediaPlayerMapper : KindMapper {
      fun features(b: MqttBindings): List<String>          // default without the key: play, pause, next, previous
  }
  ```

- [ ] **Step 1: Write the failing tests** (`MediaPlayerMapperTest`, fixture `private val player = bindings("media_player", """{"state_topic":"m/player/state","command_topic":"m/player/set","supported_features":["play","pause","next","previous","play_media"]}""")`)
  - `full state` (the spec's example → every field, `positionAtMs` = epoch ms of `2026-11-06T10:20:00Z`)
  - `a message replaces the whole state` (second message without `artist` → `artist == null`)
  - `idle clears every other field`
  - `bad values are dropped` (`color` `"red; x"`, `image_url` `"javascript:x"`, `duration_s` `-1`, `position_at` `"yesterday"` → null; `"#3D2466"` → `"#3d2466"`)
  - `unknown state word is ignored` (`Decoded.Ignored`; non-JSON → `Decoded.Error`)
  - `commands publish json` (`MediaAction("next")` → `Outgoing("m/player/set", """{"command":"next"}""", false, 0)`; `PlayMedia("du Daft Punk")` → `{"command":"play_media","query":"du Daft Punk"}`; compare parsed JSON)
  - `unsupported command is refused` (features without `next` → `Encoded.NotSupported`; blank query → `NotSupported`)
  - `prediction` (`play` → state `playing`, `pause` → `paused`, `next`/`previous` → `null`)
  - `DiscoveryParserTest`: `media player discovers with its features` (`homeassistant/media_player/n/player/config`, abbreviated keys `stat_t`, `cmd_t`, `sup_feat` → `Kind.MEDIA_PLAYER`, `capabilities.mediaFeatures`)

- [ ] **Step 2: Run to verify they fail**

Run: `./gradlew :core:state:jvmTest --tests 'com.gekkotron.nidoo.state.kinds.MediaPlayerMapperTest' :core:source:jvmTest`
Expected: compilation failure (unknown types).

- [ ] **Step 3: Implement** `MediaPlayerMapper` like `VacuumMapper` (same `stateTopics`, `feedbackTopics = emptySet()` so the prediction shows at once and the next state corrects it). Parse `position_at` with `kotlin.time.Instant.parse(it).toEpochMilliseconds()` inside `runCatching` (add `@OptIn(ExperimentalTime::class)` only if the compiler asks). Fill the compile-only branches.

- [ ] **Step 4: Run to verify they pass**

Run: `./gradlew :core:model:jvmTest :core:state:jvmTest :core:source:jvmTest :feature:home:test :feature:widget:test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/model core/state core/source feature/home/src/main feature/widget/src/main
git commit -m "feat: media player kind, state and commands"
```

### Task 2: Home card and control sheet

**Files:**
- Modify: `feature/home/build.gradle.kts` (`implementation(libs.coil.compose)`)
- Modify: `core/designsystem/src/main/kotlin/com/gekkotron/nidoo/designsystem/components/NidooCard.kt` (new `tint: Color? = null` parameter: when set, the background is a vertical gradient from `tint` to `lerp(tint, c.card, 0.45f)` and the border `lerp(tint, Color.White, 0.4f)`; content colour white)
- Modify: `core/designsystem/.../icons/StaticIcon.kt` (new `NidooGlyph.MUSIC` note glyph, plus `PREVIOUS`, `NEXT`, `PAUSE` if absent; `PLAY` exists)
- Modify: `feature/home/.../presentation/HomeModels.kt` (`CardValue.MediaPlayer(state, title, artist, album, imageUrl, color, durationS, positionS, positionAtMs)`), `CardFormatter.kt`, `CardText.kt` (`cardValueText`: "title · artist", idle → `R.string.media_nothing_playing`), `HomeUiBuilder.kt` (`isOn` = playing), `KindIcon.kt` (`IconSpec.Static(NidooGlyph.MUSIC, on)`)
- Create: `feature/home/src/main/kotlin/com/gekkotron/nidoo/feature/home/ui/MediaPlayerCard.kt` (card body used by `EntityCard` for `Kind.MEDIA_PLAYER`), `feature/home/src/main/kotlin/com/gekkotron/nidoo/feature/home/ui/MediaPlayerControls.kt` (sheet, dispatched from `ControlSheet.kt`'s `when (card.kind)`), `feature/home/src/main/kotlin/com/gekkotron/nidoo/feature/home/presentation/MediaProgress.kt`
- Modify: `feature/home/src/main/res/values/strings.xml`, `values-fr/strings.xml` (`media_nothing_playing` "Nothing playing" / "Rien ne joue", `media_play_some` "Play some music" / "Lancer de la musique", `media_play`, `media_pause`, `media_next`, `media_previous`, `media_state_playing|paused|idle`)
- Test: `feature/home/src/test/.../presentation/MediaProgressTest.kt`, `CardFormatterTest.kt`, `ui/MediaPlayerCardTest.kt`, `ui/MediaPlayerControlsTest.kt`, `ui/IconSpecTest.kt`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `fun mediaProgress(durationS: Double?, positionS: Double?, positionAtMs: Long?, playing: Boolean, nowMs: Long): Float?` — `null` without a duration > 0; else elapsed / duration in `0f..1f` (elapsed = position, plus `max(0, now − positionAt)` while playing).
  - `fun parseTint(color: String?): Color?` — `#rrggbb` → `Color`, anything else `null`.

Card (spec § "Home card"): cover thumbnail via `SubcomposeAsyncImage` (Coil 3; `error` and `loading` slots show the `MUSIC` glyph), title, artist, one play/pause `IconButton` (hidden when the player lacks that feature), `NidooCard(tint = parseTint(color))`; idle shows the glyph, "Rien ne joue" and ▶ sending `Command.MediaAction("play")`. Sheet (spec § "Control sheet"): large cover, title, "artist · album", ⏮ ▶/❚❚ ⏭ (absent when unsupported), a `LinearProgressIndicator` from `mediaProgress` re-read every second while playing (`LaunchedEffect` + `delay(1000)`), the sheet background tinted with the same colour; no volume, no search.

- [ ] **Step 1: Write the failing tests**
  - `MediaProgressTest`: `waits for a future position_at`, `stays still while loading` (playing, `positionAtMs` null → `positionS / duration`), `moves while playing`, `is clamped to one`, `is null without duration`, `paused stays at the position`; `parseTint` accepts `#3d2466`, refuses `red`, `#3d2466x`, `null`.
  - `CardFormatterTest`: `media player shows title and artist`; idle shows nothing-playing.
  - `MediaPlayerCardTest` (Robolectric, template `EntityCardIconTest`: `HomeUiBuilder.build(...).card(id)!!` in `NidooTheme(ThemeChoice.NIGHT_GLASS)`): `playing shows title and pause` (click sends `MediaAction("pause")` through the `onSend` lambda), `paused shows play`, `idle shows nothing playing and play sends play`, `no colour keeps the plain card` (assert via a `testTag("media-tint")` present only when tinted), `missing pause hides the button`.
  - `MediaPlayerControlsTest`: `sheet shows previous, play-pause and next`, `features without next and previous hide them`, `progress is shown with a duration and hidden without`.
  - `IconSpecTest`: media player → `IconSpec.Static(NidooGlyph.MUSIC, on = playing)`.

- [ ] **Step 2: Run to verify they fail**

Run: `./gradlew :feature:home:test --tests '*Media*' --tests '*CardFormatterTest' --tests '*IconSpecTest'`
Expected: compilation failure.

- [ ] **Step 3: Implement** the files above.

- [ ] **Step 4: Run to verify they pass**

Run: `./gradlew :core:designsystem:test :feature:home:test`
Expected: PASS (including `StringParityTest`, `IconsSmokeTest`).

- [ ] **Step 5: Commit**

```bash
git add core/designsystem feature/home
git commit -m "feat(home): media player card and sheet tinted by the cover colour"
```

### Task 3: Widget tile, dashboard widget and demo house

**Files:**
- Modify: `feature/widget/.../TileMapper.kt` (`valueOf`: `TileValue.Text(title)` when playing or paused, `TileValue.None` when idle; `iconOf`: a new `TileIcon.MEDIA`), `feature/widget/.../ui/WidgetContent.kt` (`TileIcon.MEDIA` → new drawable `ic_widget_media`), `feature/widget/.../config/ConfigViewModel.kt` (media players allowed in the picker)
- Create: `feature/widget/src/main/res/drawable/ic_widget_media.xml` (the note glyph as a vector)
- Modify: `core/demo/src/main/kotlin/com/gekkotron/nidoo/demo/DemoHouse.kt` (`mediaPlayer("demo_living_music", "Music", "Living room", "demo/living/music")` in `init`; a `private fun mediaPlayer(id, device, area, topic)` with `config("media_player", …)` carrying `state_topic`, `command_topic` `"$topic/set"`, all five `supported_features`; `doc(topic, …)` playing a made-up song with `color` `"#3d2466"`, no `image_url`; `echo("$topic/set", topic)` answering play/pause with the state, next/previous with another made-up title)
- Modify: `core/dashboards/src/commonMain/kotlin/com/gekkotron/nidoo/dashboards/Dashboard.kt` (`WidgetKind.MEDIA_PLAYER("media_player")`: the hub's new dashboard widget kind, spans 2x1 and 2x2; today it decodes as `UNKNOWN`), and the dashboard renderer that switches on `WidgetKind` (`feature/home/.../ui/DashboardTiles.kt` and `DashboardUiBuilder.kt`) so a `MEDIA_PLAYER` widget shows the Task 2 media card
- Test: `feature/widget/src/test/.../TileMapperTest.kt` (or the existing tile tests), `core/demo/src/test/kotlin/com/gekkotron/nidoo/demo/DemoBrokerTest.kt` (add `"demo_living_music"` to the expected id list), `core/dashboards/src/commonTest/.../DashboardDecodeTest.kt` (`media_player decodes as MEDIA_PLAYER`)

- [ ] **Step 1: Write the failing tests**: `media player tile shows the title`, `idle media tile shows nothing`, `media tile icon is media`, `media tile tap opens home`; `DemoBrokerTest` expects `demo_living_music` and that a `{"command":"pause"}` on its command topic echoes `"state":"paused"`.
- [ ] **Step 2: Run to verify they fail** — `./gradlew :feature:widget:test :core:demo:test` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run to verify they pass** — `./gradlew :core:model:jvmTest :core:state:jvmTest :core:source:jvmTest :core:demo:test :core:designsystem:test :feature:home:test :feature:widget:test` → PASS. Then install the debug build in the demo house and look at the card, sheet and widget in light and dark.
- [ ] **Step 5: Commit**

```bash
git add feature/widget core/demo
git commit -m "feat: media player widget tile and demo house entry"
```

---

## After the branch

Open the PR against `main` in Nidoo-app. The app needs no hub change to work (it reads discovery itself, and the music server publishes under `homeassistant/`), so it can merge before or after the hub PR.
