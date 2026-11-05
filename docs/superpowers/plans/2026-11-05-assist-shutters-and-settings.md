# Assist shutters and the assistant settings in Settings — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the speech settings into Settings (an Assistant group: Voix, Écoute, Son), fix the two visual bugs there, and let Assist open, close and position shutters.

**Architecture:** Web: two new `SettingsPage` variants render the existing TTS and STT components inside `SettingsPageScreen`; `/assistant/settings` becomes a redirect. Assist: a new `Intent::Covers` parsed in `nidoo-assist/src/intent.rs`, planned like lights in `plan.rs` into a new `Action::Cover` / `Done::Covers`, replied in `reply.rs`, and executed by the hub through the same path as lights (`already_there` already knows covers).

**Tech Stack:** Rust, Leptos (wasm) web UI, `wasm-bindgen-test`, tokio tests.

**Spec:** `docs/superpowers/specs/2026-11-05-assist-shutters-and-settings-design.md`

## Global Constraints

- Every user-facing string has FR and EN, added to the text enum, its `ALL` list and its match (the i18n completeness tests fail otherwise).
- Never name the user's hosts, IPs, rooms or family in tracked files; fixtures use generic rooms (Salon, Bureau, Chambre Léa…).
- Shutter position: 0 = closed, 100 = open, as the snapshot reports it (already normalised; no `invert` handling).
- Assist never moves a shutter it was not asked to; unsure → question or "Je n'ai pas compris.".
- Commits: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage explicit paths only (an unrelated untracked `crates/nidoo/examples/` must stay out).
- Long cargo runs: prefix `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
- Wasm tests: `wasm-pack test --headless --chrome --chromedriver ~/Library/Caches/.wasm-pack/chromedriver-a38bbfd5b5ddf399/chromedriver crates/nidoo-web --test <name>`.

## Review Focus

1. "monte le salon" / "baisse le salon" (no shutter noun) must stay not understood — no heating guess, no shutter guess. → Task 4 test `raise_without_shutter_noun_stays_not_understood`.
2. "ferme la lumière du salon" (close verb on a light) must not switch lights or shutters. → Task 4 test `close_verb_on_a_light_is_not_understood`.
3. A whole-house "ferme tous les volets" in a house where some shutters are offline: the others still close and the reply names the offline ones. → Task 5 test `whole_house_with_one_offline_names_it` (reply level).
4. "ouvre les volets à 30 %" when a room has no shutter with `position`: nothing is sent. → Task 5 test `position_on_open_close_only_room_answers`.
5. An old bookmark `/assistant/settings` lands on Voix without leaving a Back entry. → Task 2 wasm test `old_address_redirects_to_voice`.

---

### Task 1: Settings — the Assistant group, its pages' slugs, titles and row statuses

**Files:**
- Modify: `crates/nidoo-web/src/screens/settings.rs` (`SettingsPage`, `ALL`, `slug`, `key`, `hint`, `glyph`, `group`, `SettingsScreen` status + groups, tests at the bottom)
- Modify: `crates/nidoo-web/src/i18n/settings.rs` (keys `GroupAssistant`, `HintAssistantVoice`, `HintAssistantListening`)
- Modify: `crates/nidoo-web/src/i18n/mod.rs` (`Key::AssistantVoice`, `Key::AssistantListening`)
- Modify: `crates/nidoo-web/src/shell/route.rs` (crumbs test only; the `["settings", slug, ..]` arm already uses `SettingsPage::key`)
- Create: `crates/nidoo-web/src/screens/assist_status.rs` (row status helpers), register in `screens/mod.rs`

**Interfaces:**
- Produces: `SettingsPage::AssistantVoice` (slug `assistant-voice`), `SettingsPage::AssistantListening` (slug `assistant-listening`); `SettingsText::GroupAssistant`; `pub fn tts_row_status(status: &TtsStatus, lang: Lang) -> String` and `pub fn stt_row_status(status: &SttStatus, lang: Lang) -> String` in `screens/assist_status.rs`.

Copy (EN, FR):
- `GroupAssistant`: ("Assistant", "Assistant")
- `Key::AssistantVoice`: ("Voice", "Voix"); `Key::AssistantListening`: ("Listening", "Écoute")
- `HintAssistantVoice`: ("Text to speech (TTS): languages, voices, models", "Synthèse vocale (TTS) : langues, voix, modèles")
- `HintAssistantListening`: ("Speech recognition (STT) and the satellites' key", "Reconnaissance vocale (STT) et clé des satellites")

Order of `ALL`: … `NetworkPresence`, `AssistantVoice`, `AssistantListening`, `Sound`, `System`, … . `group()`: `AssistantVoice | AssistantListening | Sound => T::GroupAssistant`. The list renders `{group(T::GroupHouse)} {group(T::GroupAssistant)} {group(T::GroupHub)}`; the Assistant `<section class="settings__group">` gets `id="assistant"` (Task 2 links to it). Glyphs: `AssistantVoice => (icons::SPEECH, "cyan")`; `AssistantListening => (icons::MIC, "violet")` with a new `pub const MIC` in `icons.rs` in the house stroke style: `svg!(r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0"/><path d="M12 17.5V21"/>"#)`.

Row statuses: the list fetches `api::tts::get_status()` and `api::stt::get_status()` once each in an `Effect` (like `system`), no polling; failure → empty string.
- `tts_row_status`: the first language's engine (house language) → `StateReady` "Prêt" / `StateLoading` "Chargement" / `StateError` "Erreur" / `StateNoModel` "Aucun modèle" (keys in `AssistSettingsText`); no languages or `!speech_built` → `NoModel`/"Aucun modèle".
- `stt_row_status`: `!built || !installed` → "Aucun modèle"; else same mapping on `engine`.

- [ ] **Step 1: Write the failing tests** (in `settings.rs` tests and `assist_status.rs` tests)

```rust
#[test]
fn assistant_pages_sit_in_the_assistant_group() {
    for p in [SettingsPage::AssistantVoice, SettingsPage::AssistantListening, SettingsPage::Sound] {
        assert_eq!(p.group(), T::GroupAssistant);
    }
    assert_eq!(SettingsPage::from_slug("assistant-voice"), Some(SettingsPage::AssistantVoice));
    assert_eq!(SettingsPage::from_slug("assistant-listening"), Some(SettingsPage::AssistantListening));
    assert_eq!(SettingsPage::from_slug("sound"), Some(SettingsPage::Sound));
}

// assist_status.rs
#[test]
fn tts_row_follows_the_house_language() { /* languages[0].engine Ready → "Prêt" (Fr), Loading → "Loading" (En), empty languages → "Aucun modèle" */ }
#[test]
fn stt_row_needs_an_installed_model() { /* installed=false + engine Ready → "No model"; installed + Error → "Erreur" */ }
```

Extend `shell/route.rs::settings_crumbs` with `labels(&crumbs("/settings/assistant-voice", Lang::Fr, names)) == ["Réglages", "Voix"]`.

- [ ] **Step 2: Run** `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo-web --lib settings assist_status route` — Expected: FAIL (unknown variants).
- [ ] **Step 3: Implement** the variants, keys, helpers and list wiring above. In `SettingsPageScreen`, temporarily map both new pages to `view! { <p class="settings__empty">…NoSuchPage…</p> }` — Task 2 replaces it.
- [ ] **Step 4: Run** the same command plus `cargo test -p nidoo-web --lib i18n` — Expected: PASS (existing `every_page_has_a_group` / `every_page_round_trips_its_slug` included).
- [ ] **Step 5: Commit** `feat(web): an Assistant group in Settings with Voice, Listening and Sound`.

---

### Task 2: The Voix and Écoute pages, the redirect and the ⚙ link

**Files:**
- Modify: `crates/nidoo-web/src/screens/assist_settings.rs` (rename `AssistSettingsScreen` → `AssistVoicePage`; drop the `assist-set__back` link and `assist-set__title` `<h1>`; drop the now-unused `Back`/`Title` text keys if nothing else uses them; update module doc to `/settings/assistant-voice`)
- Create: `AssistListeningPage` in `crates/nidoo-web/src/screens/speech_input.rs`: `<div class="assist-set"><SpeechInputSection /></div>`
- Modify: `crates/nidoo-web/src/screens/settings.rs` (`SettingsPageScreen` arms: `AssistantVoice => <AssistVoicePage />`, `AssistantListening => <AssistListeningPage />`)
- Modify: `crates/nidoo-web/src/app.rs` (`/assistant/settings` → `AssistSettingsRedirect`, a component that `navigate("/settings/assistant-voice", replace: true)` like `HouseRedirect`)
- Modify: `crates/nidoo-web/src/screens/assistant.rs:573` (`href="/settings#assistant"`)
- Modify: `crates/nidoo-web/src/screens/settings.rs` (`SettingsScreen`: after mount, when `location.hash == "#assistant"`, `scroll_into_view` the `#assistant` section)
- Test: `crates/nidoo-web/tests/assist_settings.rs`, `crates/nidoo-web/tests/speech_input.rs`

**Interfaces:**
- Consumes: Task 1's `SettingsPage::AssistantVoice / AssistantListening`.
- Produces: `pub fn AssistVoicePage()`, `pub fn AssistListeningPage()`.

- [ ] **Step 1: Update/write the wasm tests**
  - `assist_settings.rs`: mount `AssistVoicePage`; the test asserting `.assist-set__back` (line ~128) becomes `voice_page_has_no_own_title_or_back`: `query_selector(".assist-set__back")` and `(".assist-set__title")` are `None`; Languages card and Installed models card still render.
  - New `old_address_redirects_to_voice` (in `assist_settings.rs` or a new `tests/assist_redirect.rs`): mount a `Router` with the app's two routes, `history.pushState` to `/assistant/settings`, settle, assert `location.pathname == "/settings/assistant-voice"` and `history.length` did not grow by more than the initial push.
  - `speech_input.rs`: one test mounts `AssistListeningPage` and finds the speech-input card.
  - Unit test in `assistant.rs` (or wasm in `tests/assistant.rs`): the `.assist__settings` link's `href` is `/settings#assistant`.
- [ ] **Step 2: Run** the three wasm test files — Expected: FAIL (missing components / old href).
- [ ] **Step 3: Implement** the files above. Each page polls only its own status (unchanged code paths).
- [ ] **Step 4: Run** the wasm tests and `cargo clippy -p nidoo-web --target wasm32-unknown-unknown -- -D warnings` — Expected: PASS, no warnings.
- [ ] **Step 5: Commit** `feat(web): speech settings move into Settings → Assistant; ⚙ links there`.

---

### Task 3: The two visual fixes, and the docs for the move

**Files:**
- Modify: `crates/nidoo-web/styles/assist.css`
- Modify: `docs/users/assistant.md:54`, `docs/users/sound.md:17,142`, `docs/users/talk-to-a-satellite.md:26-28`, `docs/reference/voice-protocol.md:11`, `crates/nidoo-web/src/screens/assist_languages.rs:1`, `crates/nidoo-web/src/i18n/assist_settings.rs:1` (doc comments)

CSS decisions:
- Gap: the page now sits in `.sheet` (gap 12px) under `.sheet__header`; remove `.assist-set__back`, `.assist-set__title` rules. If the first card still touches the header, give `.sheet > .assist-set` `margin-top: 4px` to match other Settings pages (compare with `/settings/system`).
- One focus ring: `.assist-set .field:where(select, input):focus-visible { outline-offset: -1px; }` — the mint ring is drawn over the field's border instead of 2px outside it. Scoped to these pages.
- Status tag: `.assist-set__state { flex: none; display: inline-flex; align-items: center; min-height: var(--touch-min); box-sizing: border-box; padding: 0 14px; }` so it matches the field's height and stays centred in `.assist-set__row` (`align-items: center` already).

Docs: "gear … **Speech settings**" → "**Settings → Assistant → Voice** (the gear on the Assistant page opens the Assistant group)"; "Assistant → Speech settings → Speech input" → "Settings → Assistant → Listening"; "Settings → Sound" → "Settings → Assistant → Sound".

- [ ] **Step 1: Apply the CSS.**
- [ ] **Step 2: Visual check.** `trunk serve` with a scratch Trunk.toml proxying `/api` to the live hub (see memory `reference_headless_chrome_cdp_screenshots`), CDP screenshots of `/settings/assistant-voice` at 1440×1100 and 390×844 with the model select focused (`document.querySelector('.assist-set select').focus()`), plus `/settings/system` for comparison. Expected: header-to-card gap equal to System's; one ring on the select; "Prêt" tag the select's height, centred.
- [ ] **Step 3: Update the docs and doc comments.** `grep -rn "assistant/settings\|Speech settings" docs crates/nidoo-web/src` returns only the redirect route.
- [ ] **Step 4: Run** `cargo test -p nidoo-web --test bare_text` (bare-text check) — Expected: PASS.
- [ ] **Step 5: Commit** `fix(web): speech settings spacing, one focus ring, status tag height; docs: new paths`.

---

### Task 4: Shutter sentences — lexicon and intent

**Files:**
- Modify: `crates/nidoo-assist/src/lexicon.rs`
- Modify: `crates/nidoo-assist/src/intent.rs`

**Interfaces:**
- Produces:

```rust
// intent.rs
pub enum Intent { …,
    /// `position`: % open, only with an open verb. `all`: "tous"/"toutes" was said.
    Covers { open: bool, position: Option<u8>, all: bool, names: Vec<String>, room: Vec<String> },
    /// "ferme les volets à 30 %": a position with a close verb; answered with a hint.
    CoverPositionHint,
    /// A position outside 0–100.
    CoverPositionRange,
}
// lexicon.rs
pub enum Class { …, Cover, Open, Close }
```

Lexicon: `Cover`: volet, volets, store, stores, roulant, roulants (removed from `OtherDevice`; rideau/rideaux stay there). `Open`: ouvre, ouvrir, ouvres, ouvrez, rouvre. `Close`: ferme, fermer, fermes, fermez, referme.

Parse rules (before the existing `Percent | OtherDevice | Negation` rejection, which stays for sentences without `Cover`):
- `Negation` → `None` (always). `OtherDevice` also present → `None`. `Light`/`LightNamed`/`Heating` also present → `None`.
- open = `Open` or `Raise`; close = `Close` or `Lower`; both or neither → `None`. `blocked` (statement/question without request) → `None`.
- A number with role `AfterA` followed by `Percent` is the position. Position with close → `CoverPositionHint`; with open and n ∉ 0..=100 → `CoverPositionRange`; a `Percent` without such a number → `None`; an `AfterA` number without `Percent` → `None`.
- names/room: same split as `lights()` (free words before the first linker are names, after it the room); a count before the cover noun is dropped (extend `Role::Count` to `Class::Cover`).
- Without a `Cover` word, `Open`/`Close` words make the sentence `None` (they never reach lights or heating).

- [ ] **Step 1: Write the failing tests** in `intent.rs`

```rust
fn covers(open: bool, position: Option<u8>, all: bool, names: &[&str], room: &[&str]) -> Option<Intent> { … }

#[test] fn shutter_open_close() {
    assert_eq!(p("ouvre les volets du salon"), covers(true, None, false, &[], &["salon"]));
    assert_eq!(p("ferme les volets du salon"), covers(false, None, false, &[], &["salon"]));
    assert_eq!(p("baisse les stores du bureau"), covers(false, None, false, &[], &["bureau"]));
    assert_eq!(p("remonte les volets roulants du salon"), covers(true, None, false, &[], &["salon"]));
    assert_eq!(p("peux-tu fermer les volets du salon"), covers(false, None, false, &[], &["salon"]));
}
#[test] fn shutter_position() {
    assert_eq!(p("ouvre les volets du salon à 30 %"), covers(true, Some(30), false, &[], &["salon"]));
    assert_eq!(p("ouvre les volets à 30 pour cent"), covers(true, Some(30), false, &[], &[]));
    assert_eq!(p("ferme les volets à 30 %"), Some(Intent::CoverPositionHint));
    assert_eq!(p("ouvre les volets à 130 %"), Some(Intent::CoverPositionRange));
    assert_eq!(p("ouvre les volets à 30"), None);
}
#[test] fn shutter_all_and_names() {
    assert_eq!(p("ferme tous les volets"), covers(false, None, true, &[], &[]));
    assert_eq!(p("ouvre tous les volets du salon"), covers(true, None, true, &[], &["salon"]));
    assert_eq!(p("ferme le volet baie du salon"), covers(false, None, false, &["baie"], &["salon"]));
}
#[test] fn shutter_never_guessed() {
    assert_eq!(p("ne ferme pas les volets"), None);
    assert_eq!(p("les volets sont fermés"), None);
    assert_eq!(p("ferme les rideaux"), None);
}
#[test] fn raise_without_shutter_noun_stays_not_understood() {
    assert_eq!(p("monte le salon"), None);
    assert_eq!(p("baisse le salon"), None);
}
#[test] fn close_verb_on_a_light_is_not_understood() {
    assert_eq!(p("ferme la lumière du salon"), None);
    assert_eq!(p("ouvre le chauffage"), None);
}
```

Replace the old `assert_eq!(p("ferme les volets du salon"), None)` (line ~468).

- [ ] **Step 2: Run** `cargo test -p nidoo-assist intent` — Expected: FAIL (no `Covers`).
- [ ] **Step 3: Implement** the classes, a `covers(toks, roles, open, position, all) -> Intent` beside `lights()`, and the early branch in `parse`. Give `plan_request` a temporary arm `Intent::Covers {..} | Intent::CoverPositionHint | Intent::CoverPositionRange => not_understood(house)` so the crate compiles; Task 5 replaces it.
- [ ] **Step 4: Run** `cargo test -p nidoo-assist` — Expected: PASS (all existing intent, plan and pending tests too).
- [ ] **Step 5: Commit** `feat(assist): understand shutter sentences`.

---

### Task 5: Shutter plans, replies and the hub's execution

**Files:**
- Modify: `crates/nidoo-assist/src/plan.rs` (`Request.covers`, `Action::Cover`, `Done::Covers`, `fn covers(...)`, `plan_request` arms)
- Modify: `crates/nidoo-assist/src/reply.rs` (`Done::Covers` text)
- Modify: `crates/nidoo-assist/src/fixture.rs` (`cover(...)`, two rooms with shutters in `house()` — see below)
- Modify: `crates/nidoo/src/assist/hub.rs` (`what()`, `execute()`)

**Interfaces:**
- Consumes: Task 4's `Intent::Covers { open, position, all, names, room }`, `CoverPositionHint`, `CoverPositionRange`.
- Produces:

```rust
pub struct Request { …, pub covers: Option<Vec<String>> }   // picked from "Quel volet ?"
pub enum Action { …, Cover { entity: String, command: Command } }
pub enum Done { …, Covers { area: Option<String>, open: bool, position: Option<u8>, names: Vec<String>, unable: Vec<String> } }
// fixture.rs
pub fn cover(id: &str, name: &str, area: &str, position: Option<u8>, can_position: bool) -> EntityView
```

Fixture additions to `house()`: Salon `cover_baie` "Baie" (position 100, `position` + `open_close`), `cover_fenetre` "Fenêtre" (position 100, both caps); Chambre Léa `cover_lea` "Volet" (state "open", `open_close` only). Keep existing light/plan tests green (they filter by kind).

Planning (`fn covers(req, open, position, all, names, room, house) -> Plan`), trace intent `"covers_open"` / `"covers_close"`:
- Members: `category_members(&CategoryAction { what: Category::Covers, to: Target::Open|Close, brightness: None, area, except: vec![] }, house.entities)`.
- `all && room.is_empty() && names.is_empty() && req.area.is_none()` → every member in the house, `area: None`. Otherwise reuse the lights flow: names-as-room fallback, `pick_room` over `areas_of(candidates)` with none-text « Pas de volet dans la maison. », then name filter, else `Plan::Ask { question: "Quel volet {de(area)} ?", choices }` with `next.covers = Some(vec![id])`. Empty room → « Pas de volet {dans(area)}. ».
- Command per cover: position `Some(p)` → `SetPosition { position: p }` if `capabilities.position`, else the cover goes to `unable`; `None` → `Open`/`Close` if `open_close`, else `SetPosition { 100 | 0 }` if `position`, else `unable`.
- All chosen `unable` → `answer`: position asked « {Le volet « X » ne sait | Ces volets ne savent} pas se mettre à une position. »; otherwise « … ne sait pas s'ouvrir / se fermer. ».
- `CoverPositionHint` → `answer("Pour une position, dites par exemple « ouvre les volets à 30 % ».")`; `CoverPositionRange` → `answer("La position va de 0 à 100 %.")`.

Replies (`reply.rs`, `fn covers(area: Option<&str>, open, position, names, unable, results) -> String`), lights' shape with "volet(s)", verb `ouvert(s)` / `fermé(s)`, position suffix « à 30 % »:
- one: « Volet « Baie » fermé dans le Salon. » / « Volet « Baie » ouvert à 30 % dans le Salon. » / already: « Volet « Baie » déjà fermé dans le Salon. »
- room: « Salon : volets fermés (2 volets). »; whole house: « Tous les volets fermés (3 volets). »; all skipped → « … déjà fermés … ».
- failures/offline: same wording as lights (« n'a pas répondu », « Rien n'a répondu … »).
- `unable` non-empty: append « {names} ne sait/savent pas se mettre à une position. ».

Hub: `execute` matches `Action::Light { entity, command } | Action::Cover { entity, command }` in the same arm (offline, `already_there`, simulate, `send_command`). `what()`: `Cover` → `Open` "ouvrir", `Close` "fermer", `SetPosition { p }` "ouvrir à {p} %".

- [ ] **Step 1: Write the failing tests** (`plan.rs` tests, `reply.rs` tests)

```rust
#[test] fn room_shutters_close() { /* "ferme les volets du salon" → Act, entities [cover_baie, cover_fenetre], all Command::Close, Done::Covers { area: Some("salon"), open: false, .. } */ }
#[test] fn whole_house_needs_no_room() { /* "ferme tous les volets" → Act on cover_baie, cover_fenetre, cover_lea; area None */ }
#[test] fn no_room_asks_rooms_with_shutters() { /* "ferme les volets" → labels == ["Salon", "Chambre Léa"] */ }
#[test] fn named_shutter() { /* "ferme le volet baie du salon" → only cover_baie */ }
#[test] fn unknown_named_shutter_asks_which() { /* "ferme le volet porte du salon" → Ask "Quel volet du Salon ?" labels ["Baie", "Fenêtre"] */ }
#[test] fn position_sets_position() { /* "ouvre les volets du salon à 30 %" → SetPosition { position: 30 } x2 */ }
#[test] fn position_on_open_close_only_room_answers() { /* "ouvre les volets de la chambre de léa à 30 %" → Answer containing "ne sait pas se mettre à une position", no actions */ }
#[test] fn close_with_position_gives_the_hint() { /* "ferme les volets à 30 %" → Answer starting "Pour une position" */ }
// reply.rs
#[test] fn covers_reply_shapes() { /* one / room / whole house / already / position suffix strings above */ }
#[test] fn whole_house_with_one_offline_names_it() { /* results [Ok, Offline, Ok] → contains "1 sur 3 n'a pas répondu : Fenêtre" */ }
```

- [ ] **Step 2: Run** `cargo test -p nidoo-assist` — Expected: FAIL.
- [ ] **Step 3: Implement** plan, reply, fixture, and the two hub match arms.
- [ ] **Step 4: Run** `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo-assist && cargo check -p nidoo` — Expected: PASS / compiles.
- [ ] **Step 5: Commit** `feat(assist): open, close and position shutters`.

---

### Task 6: Hub test for shutters, and the user docs

**Files:**
- Test: `crates/nidoo/tests/assist_hub.rs`
- Modify: `docs/users/assistant.md` (a "Shutters" part with the spec's sentence table)

Test setup: ingest `cover__0x847127fffe08106c__cover.json` at `nidoo/cover/0x847127fffe08106c/cover/config`, `place(&hub, "0x847127fffe08106c", "living")`, state `{"position":100,"state":"OPEN"}` on `zigbee2mqtt/rooms/living/shutter_south`. Extend `echo` (or a cover-specific echo) to answer `…/shutter_south/set` with the matching state (`{"state":"CLOSE"}` → `{"position":0,"state":"CLOSE"}`, `{"position":n}` → `{"position":n,"state":"OPEN"}`).

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn shutters_close_and_skip_already_closed() {
    // "ferme les volets du salon" → Acted, reply contains "fermé", a publish on …/shutter_south/set, trace result "ok";
    // again → reply contains "déjà", trace result "skipped", no publish.
}
#[tokio::test] async fn shutter_position_publishes_the_position() {
    // "ouvre les volets du salon à 30 %" → payload of …/set has position 30.
}
#[tokio::test] async fn simulated_shutters_publish_nothing() { /* simulated("ferme tous les volets") → no …/set publish */ }
```

- [ ] **Step 2: Run** `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --test assist_hub shutter` — Expected: FAIL before Task 5's hub arms exist, PASS after (if they pass at once, check the test fails with the arms reverted).
- [ ] **Step 3: Write the docs part.**
- [ ] **Step 4: Run the full gate:** `CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, wasm tests of Tasks 2–3. Expected: all green.
- [ ] **Step 5: Commit** `test(assist): shutters on the hub; docs: shutters in Assist`.
