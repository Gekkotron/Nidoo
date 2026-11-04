# Speech in several languages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hub's single active speech model and voice with an ordered list of languages (code, model, voice). The first entry is the house language, every model in use stays loaded, and the settings screen uses language tabs.

**Architecture:** A pure module (`assist/tts/languages.rs`) owns the list rules and the migration. The `Tts` facade keeps one `Engine` (thread + queue) per model in use and reconciles that set whenever the list changes. Speakers are built with `Normalize::OFF`, and each request is normalised with its own language. The web screen splits into a language-tabs component plus the models and add cards.

**Tech Stack:** Rust 2024, axum hub (`crates/nidoo`), wire types in `crates/nidoo-api`, Leptos 0.8 wasm UI (`crates/nidoo-web`), `ptts` 0.6 (feature `tts`).

**Spec:** `docs/superpowers/specs/2026-11-04-speech-languages-design.md`

**Base:** this needs PR #152 (async `Tts::speak`, voice sort, ▶ preview). Before Task 1, run `git rebase --onto origin/main <old base> feat/speech-languages` once #152 is merged. Until then, rebase onto `fix/assist-voice-sort-docs`.

## Global Constraints

- No language or language pair is hard-coded in hub code or screen logic. Language names come from `Intl.DisplayNames`. Only the test-sentence table names languages.
- Codes are trimmed, lower-cased and must match `[a-z]{2,3}(-[a-z0-9]+)*`, else `422`.
- A model used by several languages is loaded once. A model no language uses is not loaded.
- An unknown or unconfigured `language` on speech is `422`, never swapped for the house language. A language with no model is `409`. An empty list makes speech `409` with "no language is set up".
- `nidoo-assist` stays pure. `ptts` stays behind feature `tts`, and the no-speech build still answers management routes (CI lints it).
- JSON field naming follows the existing `nidoo-api::tts` types (snake_case, no `rename_all`).
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage explicit paths.
- Long cargo runs: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
- The project stays generic: no house names, hosts or rooms in tracked files.

## Review Focus

1. **Removing the model a language uses while it is speaking.** The clip in flight ends with an error, the language shows "No model", and the other languages keep speaking. (Task 4: `removing_a_used_model_leaves_languages_without_one`.)
2. **Two languages sharing one model, then one switches model.** The shared engine stays loaded for the other language, and the voice of the language that switched resets to `default`. (Task 4: `switching_one_of_two_shared_languages_keeps_the_engine`.)
3. **A state file from #151 at boot.** The user's model and catalog voice come back as the first tab, and the voice is registered once the engine is Ready. (Task 4: `old_state_boots_as_one_language_with_its_voice`.)
4. **Catalog voice of another language named on speech** (for example an English voice on `fr`). This is `422` and nothing is downloaded. (Task 5: `a_voice_of_another_language_is_refused`.)
5. **Code typed as `FR` or ` fr `.** It lands on the existing `fr` entry and never creates a second one. (Task 2: `codes_are_normalised_before_comparing`.)

---

### Task 1: Spike: per-request normalisation and memory

**Files:**
- Modify: `crates/nidoo/tests/tts_real.rs` (add one `#[ignore]` test)
- Create: `docs/superpowers/plans/2026-11-04-speech-languages-spike.md` (numbers)

**Interfaces:** none produced. This task only confirms the spec's "Facts" or triggers its fallback.

- [ ] **Step 1:** Add `#[ignore] fn normalising_outside_the_builder_matches()` in `tts_real.rs`, using the same model discovery as the existing real tests. Build one `Synth` with `Normalize::for_lang(Lang::Fr)` and one with `Normalize::OFF`. For `"Il fait 19,5 degrés le 3 novembre."`, assert `Normalize::for_lang(Lang::Fr).apply(text)` fed to the OFF synth gives the same number of samples ±5 % as the Fr synth (sampling is seeded; use `stream_seeded` with the same seed on both if exact equality holds, then assert equality).
- [ ] **Step 2:** Run `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --release --test tts_real normalising -- --ignored --nocapture`. Expected: PASS.
- [ ] **Step 3:** Measure the resident set before and after loading a second q8 `Synth` (`ps -o rss= -p <pid>` from the test, printed). Write both numbers and the result of Step 2 to the spike notes file.
- [ ] **Step 4:** If Step 2 fails, stop and switch Task 3 to one engine per (model, language) pair. If that pair costs more than 300 MB, report back to the user before going on.
- [ ] **Step 5:** Commit `test(assist): spike per-request normalisation and the memory of a second model`.

### Task 2: The language list rules (pure)

**Files:**
- Create: `crates/nidoo/src/assist/tts/languages.rs`
- Modify: `crates/nidoo/src/assist/tts/mod.rs` (add `pub mod languages;`)
- Modify: `crates/nidoo/src/assist/tts/store.rs:103-125` (`TtsState`)

**Interfaces:**
- Produces, in `languages.rs`:
  - `pub struct LanguageEntry { pub code: String, pub model: Option<String>, pub voice: String }` (Serialize, Deserialize, Clone, Debug, PartialEq)
  - `pub fn normalise_code(raw: &str) -> Result<String, TtsError>` (`Invalid` on a bad code)
  - `pub fn speaks(model_languages: &[String], code: &str) -> bool`: true when the list contains `code`, or when it is empty
  - `pub fn upsert(list: &mut Vec<LanguageEntry>, code: &str, model: &str, voice: Option<&str>)`: appends a new code at the end. On an existing code it sets the model, and the voice becomes `voice.unwrap_or("default")` when the model changed, else `voice.unwrap_or(<current>)`
  - `pub fn remove(list: &mut Vec<LanguageEntry>, code: &str) -> bool`
  - `pub fn make_house(list: &mut Vec<LanguageEntry>, code: &str) -> bool`: moves it to index 0
  - `pub fn models_in_use(list: &[LanguageEntry]) -> BTreeSet<String>`
  - `pub fn used_by(list: &[LanguageEntry], model: &str) -> Vec<String>`: codes, in list order
  - `pub fn detach_model(list: &mut [LanguageEntry], model: &str)`: sets `model: None` and `voice: "default"` on every entry using it
  - `pub fn migrate(state: &mut TtsState, first_language_of: impl Fn(&str) -> Option<String>)`: when `languages` is empty and legacy `active` is `Some(id)`, it pushes `{code: first_language_of(id).unwrap_or("fr"), model: Some(id), voice: legacy voice}`, then clears the legacy fields
- `TtsState` becomes `{ languages: Vec<LanguageEntry> (serde default), active: Option<String> (serde default, skip_serializing), voice: Option<String> (serde default, skip_serializing) }`. The legacy fields are read-only.

- [ ] **Step 1: Write the failing tests** in `languages.rs` `#[cfg(test)]`:
  - `codes_are_normalised_before_comparing`: `normalise_code(" FR ")` is `"fr"`; `"pt-BR"` → `"pt-br"`; `"french"`, `""`, `"f"` and `"fr_FR"` are `Err`; `upsert` with `"fr"` after an existing `"fr"` keeps `len() == 1`.
  - `upsert_appends_then_updates`: the second code goes to index 1; changing the model resets the voice to `"default"`; same model with `voice: None` keeps the voice.
  - `make_house_moves_to_front`, and `remove_first_promotes_next`.
  - `models_in_use_dedups`: two codes on model `m` → `{"m"}`.
  - `empty_model_languages_speak_anything`: `speaks(&[], "de")` is true; `speaks(&["fr".into()], "de")` is false.
  - `migrate_old_state`: `{active: Some("m"), voice: Some("Élise")}` with `first_language_of = |_| Some("fr")` → one entry `fr/m/Élise`; no active → empty; a manifest without languages → `fr`. Also a serde test: parsing `{"active":"m","voice":"Élise"}` then serialising has no `active` key.
- [ ] **Step 2:** Run `cargo test -p nidoo --lib languages`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement the functions above. Compile errors elsewhere from the `TtsState` change are expected; leave `mod.rs` call sites for Task 4, but keep the crate building by temporarily mapping `state.active` reads to `state.languages.first().and_then(|l| l.model.clone())`.
- [ ] **Step 4:** Run `cargo test -p nidoo --lib languages`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(assist): language list rules and migration of the single-model state`.

### Task 3: Speakers take a language; one engine per model

**Files:**
- Modify: `crates/nidoo/src/assist/tts/engine.rs` (`Speaker::speak`, `Msg::Speak`, `Engine::speak`)
- Modify: `crates/nidoo/src/assist/tts/phonon.rs` (build with `Normalize::OFF`, normalise per request)
- Modify: `crates/nidoo/src/assist/tts/tone.rs` (record languages)
- Modify: `crates/nidoo/src/assist/tts/speech.rs` (clip remembers its rate)

**Interfaces:**
- `Speaker::speak(&mut self, text: &str, language: Option<&str>, voice: Option<&str>, chunk: &mut dyn FnMut(&[f32]) -> bool) -> Result<(), String>`
- `Engine::speak(&self, text: String, language: Option<String>, voice: Option<String>, sink: Sink) -> Result<(), TtsError>`
- `engine::start(loader)` is unchanged; each `Engine` owns one thread, and dropping it ends that thread once its queue drains (already true: `rx.recv()` errors).
- `tone::RecordingLoader { pub seen: Arc<Mutex<Vec<Option<String>>>> }`: a `SpeakerLoader` whose speaker pushes each request's `language` to `seen`, then behaves like `ToneSpeaker`.
- `speech::Clips::create(&self, sample_rate: u32) -> (String, Arc<Clip>)` and `Clip::sample_rate(&self) -> u32`. `http/tts.rs::speech_file` uses `clip.sample_rate()` instead of `tts.engine().sample_rate()`.
- Phonon: `PhononSpeaker` normalises with `Normalize::for_lang(l).apply(text)` when `language` parses as `ptts::Lang` (`str::parse`), else speaks `text` unchanged.

- [ ] **Step 1: Write the failing tests** in `engine.rs` tests:
  - `the_language_reaches_the_speaker`: with a `RecordingLoader`, `speak(.., Some("en".into()), None, sink)` then `speak(.., None, ..)` → `seen == [Some("en"), None]`.
  - `two_engines_speak_independently`: start two engines with `ToneLoader`, load both, make one `Error` (`fail` marker dir) → the other still reaches `Ready` and speaks.
  - In `speech.rs`: `a_clip_keeps_its_rate`: `create(16_000).1.sample_rate() == 16_000`.
- [ ] **Step 2:** Run `cargo test -p nidoo --lib tts::engine tts::speech`. Expected: FAIL to compile (signatures).
- [ ] **Step 3:** Change the signatures and thread `language` through `Msg::Speak`. Update every caller, including the `tts_real.rs` calls with `None`.
- [ ] **Step 4:** Run `cargo test -p nidoo --lib tts` and `cargo check -p nidoo --no-default-features`. Expected: PASS and a clean check.
- [ ] **Step 5:** Commit `feat(assist): speakers take the request's language; clips keep their rate`.

### Task 4: Languages in the hub: state, engines, routes

**Files:**
- Modify: `crates/nidoo-api/src/tts.rs`
- Modify: `crates/nidoo/src/assist/tts/mod.rs`
- Modify: `crates/nidoo/src/http/tts.rs`
- Test: `crates/nidoo/tests/tts_http.rs`

**Interfaces:**
- Consumes: Task 2 (`languages::*`), Task 3 (`Engine::speak` with language).
- `nidoo-api` wire types:
  - `LanguageView { code: String, model: Option<String>, voice: String, engine: EngineState }`, where `engine` is `NoModel` when `model` is `None`
  - `TtsStatus { languages: Vec<LanguageView>, token_set, download, speech_built }` (drop `engine`, `active`, `voice`)
  - `ModelView`: drop `active` and `french`; add `used_by: Vec<String>`
  - `LanguageRequest { model: String, #[serde(default)] voice: Option<String> }`
  - `HouseRequest { code: String }`
  - `AvailableLanguage { code: String, models: Vec<String> }`
  - Delete `VoiceRequest` and the `PUT /active` body type
- `Tts` (in `mod.rs`):
  - field `engines: Mutex<HashMap<String, Arc<Engine>>>` replaces `engine`; keep `loader: Arc<dyn SpeakerLoader>`
  - `fn reconcile(self: &Arc<Self>)`: starts and loads engines for `models_in_use` that are missing, drops the others, then spawns `apply_voices(model)` for each newly loaded model. `apply_voices` waits for Ready (≤ `READY_WAIT`) and registers each using language's chosen catalog voice.
  - `pub fn engine_for(&self, model: &str) -> Option<Arc<Engine>>`
  - `pub fn status(&self) -> TtsStatus`
  - `pub fn models(&self) -> Vec<ModelView>`
  - `pub async fn set_language(self: &Arc<Self>, code: &str, req: LanguageRequest) -> Result<TtsStatus, TtsError>`: `NotFound` for an unknown model; `Invalid` when `!speaks(model.languages, code)`; registers a named catalog voice first (under `voice_lock`), then `upsert`, save, reconcile
  - `pub fn remove_language(self: &Arc<Self>, code: &str) -> Result<TtsStatus, TtsError>` (`NotFound` if absent)
  - `pub fn set_house(&self, code: &str) -> Result<TtsStatus, TtsError>`
  - `pub fn available_languages(&self) -> Vec<AvailableLanguage>`: codes from manifests (sorted; a model with no languages contributes nothing here, but is offered for any code by `set_language`), minus codes already listed
  - `remove(id)`: `detach_model`, save, reconcile, then delete the files
  - `start()`: read state, `migrate` with `|id| store.get(id).and_then(|m| m.languages.first().cloned())`, drop entries whose model is no longer installed (set them to `None`), save if changed, reconcile
  - `set_active` and `set_voice` are deleted
- Routes (`http/tts.rs`): `GET /assist/tts/languages/available`, `PUT /assist/tts/languages/house` (registered **before** the `{code}` route), `PUT /assist/tts/languages/{code}`, `DELETE /assist/tts/languages/{code}`. Remove `PUT /active` and `PUT /voice`.

- [ ] **Step 1: Write the failing tests** in `tts_http.rs`. The rig's fake repo gets a second model with `config.json` languages `["fr","en"]` (add `fn repo_bilingual() -> FakeRepo`) and the catalog gets `Bob` (`en`) on disk:
  - `languages_start_empty_and_speech_is_409`
  - `add_two_languages_on_one_model_loads_one_engine`: PUT `fr` and `en` with the bilingual model → status shows both Ready with the same `model`; `GET /models` → `used_by == ["fr","en"]`.
  - `switching_one_of_two_shared_languages_keeps_the_engine`: then PUT `en` with the French model → `fr` stays Ready (same model), and `en`'s voice is `"default"`.
  - `a_model_that_does_not_speak_the_code_is_422`
  - `house_moves_first_and_survives_restart` (use the existing restart pattern from `state_survives_a_restart`)
  - `removing_a_used_model_leaves_languages_without_one`: → `model: null`, `engine.state == "no_model"`
  - `old_state_boots_as_one_language_with_its_voice`: write `state.json` `{"active":"<id>","voice":"Noemie"}` before `rig_with` boots, with the model installed → status has one `fr` entry with voice `Noemie`, and `voices/AAA.safetensors` is on disk once Ready.
  - `available_lists_codes_not_yet_used`: after adding `fr` → available is `[{code:"en", models:[bilingual id]}]`.
  - Update `set_voice_rules`, `a_catalog_voice_can_be_chosen_end_to_end`, `remove_active_unloads`, `download_then_activate_then_ready` and `state_survives_a_restart` to the new routes (same behaviour, expressed with `PUT /languages/fr`).
- [ ] **Step 2:** Run `cargo test -p nidoo --test tts_http`. Expected: FAIL.
- [ ] **Step 3:** Implement the types, facade and routes above.
- [ ] **Step 4:** Run `cargo test -p nidoo --test tts_http --test tts_voices --lib tts` and `cargo clippy -p nidoo --all-targets -- -D warnings`. Expected: PASS and clean. `nidoo-web` does not compile against the new `TtsStatus` until Task 7, so check only `-p nidoo` and `-p nidoo-api` until then.
- [ ] **Step 5:** Commit `feat(assist): speech languages in the hub, one engine per model in use`.

### Task 5: Speech requests take a language

**Files:**
- Modify: `crates/nidoo-api/src/tts.rs` (`SpeechRequest.language`)
- Modify: `crates/nidoo/src/assist/tts/mod.rs` (`speak`, `voices`)
- Modify: `crates/nidoo/src/http/tts.rs` (`GET /voices?language=`)
- Test: `crates/nidoo/tests/tts_speech.rs`, `crates/nidoo/tests/tts_http.rs`

**Interfaces:**
- `SpeechRequest { text, #[serde(default)] language: Option<String>, voice }`
- `Tts::speak(&self, req) -> Result<SpeechTicket, TtsError>` (async from #152):
  1. Validate the text, as today.
  2. Resolve the language: `normalise_code`, else `languages[0]`. An empty list → `Conflict("no language is set up")`; a code not in the list → `Invalid("this hub does not speak <code>")`.
  3. No model → `Conflict("<code> has no model")`.
  4. Voice: the named voice is registered for this language (catalog voices are matched with `languages = [code]`, so another language's voice is `Invalid`). Without a name, use the entry's voice if the engine has it, else `None`.
  5. `engine.speak(text, Some(code), voice, sink)`. The clip is created with `engine.sample_rate().unwrap_or(24_000)`.
- `Tts::voices(&self, language: Option<&str>) -> Result<VoicesView, TtsError>`: defaults to the house language; an unknown code is `Invalid`; `VoicesView.chosen` is that entry's voice; the catalog is filtered with `&[code]`. With no languages, it returns the empty view, not an error.
- `GET /assist/tts/voices` takes `Query<VoicesQuery { language: Option<String> }>` (type in `nidoo-api`).

- [ ] **Step 1: Write the failing tests:**
  - `tts_speech.rs`, `speech_uses_the_house_language_by_default`: a rig with a `RecordingLoader`; PUT `fr`, POST speech without language → `seen == [Some("fr")]`; with `"language":"EN"` and `en` configured → `Some("en")`.
  - `tts_speech.rs`, `an_unconfigured_language_is_422`.
  - `tts_speech.rs`, `a_language_without_a_model_is_409`: add `fr`, remove its model.
  - `tts_http.rs`, `a_voice_of_another_language_is_refused`: `fr` configured, POST `{voice:"Bob"}` → 422, and `voices/BBB.safetensors` does not exist.
  - `tts_http.rs`, `voices_follow_the_requested_language`: `GET /voices?language=en` lists `Bob` and not `Noemie`.
  - Adapt `a_catalog_voice_can_be_heard_without_choosing_it` (#152) to configure `fr` first.
- [ ] **Step 2:** Run `cargo test -p nidoo --test tts_speech --test tts_http`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the two suites plus `cargo clippy -p nidoo --all-targets -- -D warnings` and `cargo clippy -p nidoo --no-default-features -- -D warnings`. Expected: PASS and clean.
- [ ] **Step 5:** Commit `feat(assist): speech requests name their language; voices per language`.

### Task 6: Web client, language names and test sentences

**Files:**
- Modify: `crates/nidoo-web/src/api/tts.rs`
- Create: `crates/nidoo-web/src/i18n/speech_languages.rs` (registered in `i18n/mod.rs`)
- Modify: `crates/nidoo-web/src/i18n/assist_settings.rs`

**Interfaces:**
- `api/tts.rs`: add (the old `set_active` and `set_voice` stay until Task 7 deletes them)
  - `get_available_languages() -> Result<Vec<AvailableLanguage>, ApiError>`
  - `set_language(code: &str, req: &LanguageRequest) -> Result<TtsStatus, ApiError>`
  - `remove_language(code: &str) -> Result<TtsStatus, ApiError>`
  - `set_house(code: &str) -> Result<TtsStatus, ApiError>`
  - `get_voices(language: &str) -> Result<VoicesView, ApiError>`; the code goes through `urlencoding`/the existing query helper
- `speech_languages.rs`:
  - `pub fn language_name(code: &str, ui: Lang) -> String`: on wasm, `Intl.DisplayNames([ui], {type:"language"}).of(code)` via `js_sys::Reflect` (it may throw on a bad tag, so catch that) with the first letter upper-cased; on native, and as the fallback, `code.to_uppercase()`
  - `pub fn test_sentence(code: &str) -> Option<&'static str>`: matched on the part before `-`. Exact table: `fr` "Bonjour, je suis l'assistant de la maison." · `en` "Hello, I am the house assistant." · `de` "Hallo, ich bin der Assistent des Hauses." · `es` "Hola, soy el asistente de la casa." · `it` "Ciao, sono l'assistente di casa." · `nl` "Hallo, ik ben de assistent van het huis." · `pt` "Olá, sou o assistente da casa."
- New `AssistSettingsText` variants, `(en, fr)`:
  - `LanguagesHeading` ("Languages", "Langues")
  - `AddLanguage` ("+ Add", "+ Ajouter")
  - `HouseLanguage` ("House language: speaks when no language is asked", "Langue de la maison : parle quand aucune langue n'est demandée")
  - `MakeHouse` ("Make house language", "Choisir comme langue de la maison")
  - `ModelLabel` ("Model", "Modèle")
  - `NoModel` ("No model", "Aucun modèle")
  - `RemoveLanguage` ("Remove language", "Retirer la langue")
  - `RemoveLanguageQuestion` ("Remove this language?", "Retirer cette langue ?")
  - `NoLanguages` ("No language yet. Add a model, then a language.", "Aucune langue. Ajoutez un modèle, puis une langue.")
  - `NoOtherLanguage` ("Installed models speak no other language.", "Les modèles installés ne parlent aucune autre langue.")
  - `WhichModel` ("Which model?", "Quel modèle ?")
  - `UsedBy` ("Used by", "Utilisé par")
  - `RemoveUsedQuestion` ("Remove it? These languages will have no model:", "Le supprimer ? Ces langues n'auront plus de modèle :")
  - `LanguagesUnknown` ("languages unknown", "langues inconnues")
  - `TokenFold` ("Hugging Face token: only needed for gated repositories", "Jeton Hugging Face : seulement pour les dépôts protégés")
  - `TypeSentence` ("Type a sentence", "Tapez une phrase")
- Task 7 deletes the variants it leaves unused: `NoFrench`, `ActiveNoFrench`, `SpeaksFrench`, `UseModel`, `ActiveLabel`, `TokenHeading`, `NoActiveModel`, `ActiveModel` and `TestSentence` (moved to the table; its pinned test moves to `the_french_sentence_is_pinned`).

- [ ] **Step 1: Write the failing tests:** in `speech_languages.rs`, `the_table_matches_regions` (`test_sentence("pt-br") == test_sentence("pt")`, `test_sentence("sv") == None`) and `the_french_sentence_is_pinned`; `native_name_falls_back_to_the_code` (`language_name("de", Lang::En) == "DE"` off wasm); the existing `every_text_has_both_languages` updated with the new variants.
- [ ] **Step 2:** Run `cargo test -p nidoo-web speech_languages assist_settings`. Expected: FAIL.
- [ ] **Step 3:** Implement. Only add; the old screen keeps compiling.
- [ ] **Step 4:** Run the same tests. Expected: PASS.
- [ ] **Step 5:** Commit `feat(web): speech language names, test sentences and client calls`.

### Task 7: The settings screen with language tabs

**Files:**
- Create: `crates/nidoo-web/src/screens/assist_languages.rs` (component `LanguagesCard`, registered in `screens/mod.rs`)
- Modify: `crates/nidoo-web/src/screens/assist_settings.rs`
- Modify: `crates/nidoo-web/styles/assist.css`
- Test: `crates/nidoo-web/tests/assist_settings.rs`

**Interfaces:**
- `#[component] pub fn LanguagesCard(status: RwSignal<Option<TtsStatus>>, models: RwSignal<Vec<ModelView>>, reload: Callback<()>) -> impl IntoView`
- Inside it:
  - the selected tab is a `RwSignal<Option<String>>` saved to localStorage key `nidoo.assist.tts.tab` (wrap every access in a fallible helper). It falls back to the house language when the saved code is gone.
  - typed sentences live in a `RwSignal<HashMap<String, String>>` for the session only.
  - the **Model** drop-down lists installed models where `speaks(model.languages, code)`; a model with no languages is shown with `LanguagesUnknown`. Choosing one calls `set_language(code, {model, voice: None})`.
  - the ▶ preview from #152 moves here and passes `language: Some(code)`.
  - **+ Add** fetches `get_available_languages`. One model → `set_language` directly; several → a small inline picker labelled `WhichModel`. None → `NoOtherLanguage`.
  - the tab dot uses the existing state colours: `--connected` (ready), `--warning` (loading), `--danger` (error), `--muted` (no model).
- `assist_settings.rs` order: Back link and title → `LanguagesCard` → Models card (rows show languages, size, licence, short commit and `UsedBy` names; Remove names the affected languages via `RemoveUsedQuestion`) → Add card (download, upload, and the token as a `<details>` with summary `TokenFold` and "set"/"not set"). The separate Speech, Try it, Voice and Token sections are removed.

- [ ] **Step 1: Write the failing tests** in `tests/assist_settings.rs` (wasm, with the existing `FetchStub`):
  - rename and adapt `the_six_sections_render…` to `the_three_cards_render_even_when_the_hub_does_not_answer`: headings `["Languages","Installed models","Add a model"]`.
  - `tabs_follow_the_status_order`: status with `[fr, en]` → tab labels start "French", then "English" (`Intl` names in the `Lang::En` UI), the first panel shows `HouseLanguage`, and the second tab shows `MakeHouse`.
  - `preview_sends_the_tab_language`: click ▶ on a voice in the `en` tab → the stub records a POST `/api/v1/assist/speech` whose body has `"language":"en"`.
  - `a_language_without_a_model_says_so`: entry with `model: null` → its tab shows `NoModel`.
- [ ] **Step 2:** Run `wasm-pack test --headless --chrome crates/nidoo-web --test assist_settings`, or the repo's usual wasm test command (see `.github/workflows`). Expected: FAIL.
- [ ] **Step 3:** Implement. Reuse the `assist-set__*` classes; add only `.assist-set__tabs`, `.assist-set__tab`, `.assist-set__tab--on` and `.assist-set__dot` (with the state colours).
- [ ] **Step 4:** Run the wasm tests, `cargo clippy -p nidoo-web --target wasm32-unknown-unknown -- -D warnings`, and `trunk build` in `crates/nidoo-web`. Expected: PASS, clean, and a successful build.
- [ ] **Step 5:** Commit `feat(web): speech settings by language tabs, models used by, token folded into Add`.

### Task 8: Docs

**Files:**
- Modify: `docs/users/assistant.md` ("Voice" section and its subsections)
- Modify: `docs/reference/http-api.md` (speech table)
- Modify: `docs/superpowers/specs/2026-11-04-speech-languages-design.md` (only if the spike changed a fact)

- [ ] **Step 1:** Rewrite "Choosing the model and the voice" as "Languages, models and voices". Cover the tabs, the house language and *Make house language*, *+ Add*, one model per language or one model for several, models in use staying loaded, ▶ with the tab's sentence, a model removed leaving its languages with no model, and the token now inside *Add a model*. Remove "the screen warns that it does not speak French".
- [ ] **Step 2:** In `http-api.md`: add the four language routes, `language` on speech requests (and its 409/422 cases), `?language=` on voices, and the new status and model shapes. Delete `PUT /active` and `PUT /voice`.
- [ ] **Step 3:** Run `rg -n "PUT /api/v1/assist/tts/(active|voice)|does not speak French" docs crates`. Expected: no hits.
- [ ] **Step 4:** Commit `docs(assist): speech languages`.

After the last task: run `cargo test -p nidoo` and `cargo clippy --workspace --all-targets -- -D warnings`. Then push `feat/speech-languages` and open the PR against `main`, noting "rebuild hub + web after merge; the state file migrates on first boot".
