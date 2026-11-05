# Flow `ask` node Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A flow card that speaks a question on the satellites, listens once, and leaves by the
exit of the answer heard (or `none`), keeping a spoken number as a run result.

**Architecture:** `nidoo-core` gets the node model, validation and a pure answer matcher. The
voice service (`assist/stt`) gains a hub-initiated exchange: the session speaks a question clip,
reopens the mic as a follow-up and hands the transcript to a waiting asker instead of Assist.
`Hub::ask_aloud` bridges the flow engine to the voice loop over a channel; the engine's `ask`
step matches the answer, retries once, and picks the port. The web gets the picker entry, card
editor, exits and sentence.

**Tech Stack:** Rust (tokio, serde, schemars), Leptos web (`nidoo-web`).

**Spec:** `docs/superpowers/specs/2026-11-05-flow-ask-node-design.md`

## Global Constraints

- Wire tag `ask`; params `text`, `room?`, `answers` (1–6 × `{label, words?}`), `name?`, `default?`.
- Ports: `"1"`..`"n"` in `answers` order, then `"none"`.
- `name` follows math names (`nidoo_core::flow::math::is_valid_name`, `[a-z][a-z0-9_]{0,31}`); `default` only with `name`.
- Only satellites that both speak and listen are asked; busy ones are skipped.
- Silence → `none` at once; heard-but-unmatched → one retry on the answering satellite, then `none`.
- Whole step bounded at 45 s (`ASK_LIMIT`).
- No speech input / no satellite → port `none` (step outcome Ok, detail says why), never a failed step.
- Generic project: no real hosts, rooms or names in tracked files; examples use `kitchen`, `sejour`.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage explicit paths only (never `crates/nidoo/examples/`).
- Cargo: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; run per-crate tests (`-p nidoo-core`, `-p nidoo`, `-p nidoo-web`).

## Review Focus

1. Article "un/une/one" is not a number: "oui, un peu" → port of *oui*, no result. (Task 2 test `article_un_is_not_a_number`.)
2. "non" never matches an *oui* answer, and "oui mais non merci" goes to *non* (longer phrase wins). (Task 2 test `longest_phrase_wins`.)
3. Wake word inside the answer ("Hey Jarvis, oui") still matches *oui* — the session strips it. (Task 3 test `asked_transcript_is_stripped_of_the_wake_word`.)
4. A run cancelled mid-question (flow disabled/deleted) releases the room's music: closed asker → session back to idle with `Release`. (Task 4 test `closed_asker_releases_the_satellite`.)
5. Every satellite busy in an Assist exchange → `none` with detail `no free satellite`, nothing spoken. (Task 4 test `busy_satellites_are_skipped`.)

---

### Task 1: Node model, ports and validation (nidoo-core)

**Files:**
- Modify: `crates/nidoo-core/src/flow/model.rs` (NodeKind, type_name, ports, `math_names`, new params)
- Modify: `crates/nidoo-core/src/flow/mod.rs` (re-export `AskParams`, `AskAnswer`)
- Modify: `crates/nidoo-core/src/flow/validate.rs` (`FlowError::BadAsk`)

**Interfaces:**
- Produces:
  - `NodeKind::Ask(AskParams)` with `#[serde(rename = "ask")]`, doc "Says a question on the satellites and branches on the spoken answer."
  - `pub struct AskParams { pub text: String, #[serde(default, skip_serializing_if)] pub room: Option<String>, pub answers: Vec<AskAnswer>, pub name: Option<String>, pub default: Option<f64>, #[serde(flatten)] pub extra: BTreeMap<String, Value> }` (derive like `SpeakParams` + schemars, `Default`).
  - `pub struct AskAnswer { pub label: String, #[serde(default, skip_serializing_if = "Vec::is_empty")] pub words: Vec<String> }`.
  - `pub const ASK_NONE_PORT: &str = "none";` `pub const ASK_MAX_ANSWERS: usize = 6;`
  - `NodeKind::ports()` for Ask: `["1",…,"n","none"]`.
  - `math_names(flow)` also yields `ask` names (doc updated: "names of math results, from math cards and ask cards").
  - `FlowError::BadAsk { node: String, reason: String }`, Display like `BadSpeak`.

- [ ] **Step 1: Write the failing tests** in `model.rs` tests and `validate.rs` tests:

```rust
#[test]
fn ask_node_round_trips_and_lists_ports() {
    let json = r#"{"id":"q","type":"ask","params":{"text":"Je chauffe ?","room":"sejour","answers":[{"label":"Oui"},{"label":"Seulement en haut","words":["en haut"]},{"label":"Non"}],"name":"temp","default":21.0}}"#;
    let node: Node = serde_json::from_str(json).unwrap();
    assert_eq!(node.kind.type_name(), "ask");
    assert_eq!(node.kind.ports(), vec!["1", "2", "3", "none"]);
    assert_eq!(serde_json::to_value(&node).unwrap(), serde_json::from_str::<Value>(json).unwrap());
}
#[test]
fn math_names_include_ask_names() { /* flow with math "a" and ask name "temp" → {"a","temp"} */ }
```

validate.rs (helper `ask_flow(params)` like `speak_flow`): each of these is `Err(FlowError::BadAsk{..})` with the reason prefix shown:
- `{"text":" ","answers":[{"label":"Oui"}]}` → "text must not be empty"
- `{"text":"T {x","answers":[{"label":"Oui"}]}` → starts with "text:"
- `{"text":"T","answers":[]}` → "1 to 6 answers"
- 7 answers → "1 to 6 answers"
- `[{"label":"Oui"},{"label":" oui "}]` and `[{"label":"Éte"},{"label":"ete"}]` → "duplicate answer"
- `[{"label":""}]` → "answer label must not be empty"
- `"name":"Temp"` → "name:"
- `"default":21` without name → "default needs a name"
- ok: `{"text":"T","answers":[{"label":"Oui"},{"label":"Non"}],"name":"temp"}`
- an edge from the ask node with port `"4"` on a 3-answer card is refused by the existing port check.

- [ ] **Step 2: Run** `cargo test -p nidoo-core flow::` — Expected: FAIL (no `Ask` variant).
- [ ] **Step 3: Implement** the model and the `NodeKind::Ask` arm in `validate`. Label comparison folds case, accents and surrounding spaces (reuse the accent folding Task 2 adds, or a local `fold` — Task 2 moves it to `answer::normalize`). Fix every exhaustive `match` on `NodeKind` the compiler flags in `nidoo-core` (role/ordering lists: Ask behaves like Speak, an action with branching ports).
- [ ] **Step 4: Run** `cargo test -p nidoo-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): ask flow node — question, named answers, none port`.

### Task 2: Answer matcher and number reader (nidoo-core)

**Files:**
- Create: `crates/nidoo-core/src/flow/answer.rs` (+ `pub mod answer;` in `flow/mod.rs`)

**Interfaces:**
- Consumes: `AskAnswer` (Task 1), `Lang` (`flow::template::Lang`, `Fr`/`En`).
- Produces:
  - `pub fn normalize(text: &str) -> String` — lowercase, accents folded, `-` `'` `’` and punctuation → space, spaces collapsed.
  - `pub fn match_answer(answers: &[AskAnswer], heard: &str) -> Option<usize>` — 0-based index.
  - `pub fn first_number(heard: &str) -> Option<f64>`.
  - `pub fn not_understood(lang: Lang) -> &'static str` — `"Je n'ai pas compris."` / `"I didn't understand."`.

Matching rule: phrases of an answer = its label + `words` + built-ins when the normalized label is `oui`/`yes` (`oui, ouais, ouai, yes, yeah, vas y, d accord, ok, okay, volontiers, bien sur, go`) or `non`/`no` (`non, no, nope, pas maintenant, non merci, laisse, laisse tomber, pas la peine, not now, no thanks`). A phrase matches when its normalized tokens appear as a contiguous run in the heard tokens. Score = token count of the longest matching phrase; best score wins; tie → lowest index; no match → `None`.

Number rule: first token sequence that is digits (`21`, `21,5`, `21.5`) or number words in French or English (both always on) up to 100 (`vingt et un`, `vingt-et-un`, `twenty one`, `quatre vingt dix`), returned as `f64`. `un`, `une`, `one` alone are never a number; they count only inside a compound (`vingt et un`).

- [ ] **Step 1: Write the failing tests** (module tests):

```rust
fn a(labels: &[&str]) -> Vec<AskAnswer> { labels.iter().map(|l| AskAnswer { label: (*l).into(), words: vec![] }).collect() }
#[test] fn oui_phrasings_match_oui() { let x = a(&["Oui","Non"]); for h in ["oui oui vas-y","Ouais","d'accord","OK"] { assert_eq!(match_answer(&x,h), Some(0), "{h}"); } }
#[test] fn non_never_matches_oui() { assert_eq!(match_answer(&a(&["Oui","Non"]),"non"), Some(1)); assert_eq!(match_answer(&a(&["Oui"]),"non"), None); }
#[test] fn longest_phrase_wins() { assert_eq!(match_answer(&a(&["Oui","Non"]),"oui mais non merci"), Some(1)); }
#[test] fn custom_label_and_words() { let x = vec![AskAnswer{label:"Oui".into(),words:vec![]}, AskAnswer{label:"Seulement en haut".into(),words:vec!["l'étage".into()]}]; assert_eq!(match_answer(&x,"seulement en-haut"), Some(1)); assert_eq!(match_answer(&x,"juste l etage"), Some(1)); }
#[test] fn tie_goes_to_first() { assert_eq!(match_answer(&a(&["Oui","Yes"]),"oui yes"), Some(0)); }
#[test] fn numbers() { assert_eq!(first_number("oui à 21"), Some(21.0)); assert_eq!(first_number("21,5"), Some(21.5)); assert_eq!(first_number("vingt-et-un"), Some(21.0)); assert_eq!(first_number("vingt et un degrés"), Some(21.0)); assert_eq!(first_number("twenty two"), Some(22.0)); assert_eq!(first_number("quatre-vingt-dix"), Some(90.0)); assert_eq!(first_number("oui"), None); }
#[test] fn article_un_is_not_a_number() { assert_eq!(first_number("oui, un peu"), None); assert_eq!(first_number("one more"), None); }
```

- [ ] **Step 2: Run** `cargo test -p nidoo-core flow::answer` — Expected: FAIL.
- [ ] **Step 3: Implement** the four functions. Use `normalize` from Task 1's label check (replace any local fold there).
- [ ] **Step 4: Run** `cargo test -p nidoo-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): match a spoken answer and read its number`.

### Task 3: Hub-initiated exchange in the voice session

**Files:**
- Modify: `crates/nidoo/src/assist/stt/session.rs`

**Interfaces:**
- Produces:
  - `Input::AskStart` — Idle → `Phase::Preparing { since }`, sets `asking = true`, outputs `[Hold]`. Ignored when not idle.
  - `Input::AskClip(Option<String>)` — in `Preparing`: `Some(clip)` → `Speaking { clip, question: true, since }` (then the existing Spoken → Margin → FollowUp path); `None` → release.
  - `Input::AskCancel` — any non-idle phase of an asking session → release **without** `Answer`.
  - `Output::Answer(Option<String>)` — `Some(text)` from `Transcribing` when asking (text after `strip_wake_word`, non-empty), followed by `Release`; `None` whenever an asking session releases any other way (silence, empty transcript, expiry, no clip), always before `Release`.
  - `Session::is_asking(&self) -> bool`.
  - `Preparing` expires after `SPOKEN_WAIT_MS` like `Thinking`.
  - A non-asking session behaves exactly as today (all existing tests unchanged).

- [ ] **Step 1: Write the failing tests** in the `Rig` test module:
  - `ask_speaks_then_listens_then_answers`: `AskStart` → `[Hold]`; `AskClip(Some("c1"))`; `Spoken{clip:"c1"}` + ticks past margin → `[Listen]`; `listening(7)`; `Wake{kind:FollowUp, listen_ctr:7}`; loud audio until phrase → `[Stop, Transcribe(_)]`; `Transcript("oui à 21")` → `[Answer(Some("oui à 21")), Release]`; `is_idle()`.
  - `asked_transcript_is_stripped_of_the_wake_word`: transcript `"hey jarvis oui"` → `Answer(Some("oui"))` (use the exact wake phrase `strip_wake_word` strips).
  - `ask_silence_answers_none`: no follow-up wake within `FOLLOW_UP_START_MS` → `[Answer(None), Release]`.
  - `ask_without_clip_answers_none`: `AskClip(None)` → `[Answer(None), Release]`.
  - `ask_cancel_releases_without_answer`: after `AskClip(Some)`, `AskCancel` → `[Release]`.
  - `ask_start_ignored_when_busy`: after a wake word, `AskStart` → `[]`.
  - `wake_word_during_ask_is_ignored`: in `Speaking` of an ask, `Wake{WakeWord}` → `[]`.
- [ ] **Step 2: Run** `cargo test -p nidoo assist::stt::session` — Expected: FAIL.
- [ ] **Step 3: Implement**: add `asking: bool` to `Session`; `release()` pushes `Answer(None)` first when `asking` and clears it; the transcript arm branches on `asking`.
- [ ] **Step 4: Run** `cargo test -p nidoo assist::stt` — Expected: PASS (old tests too).
- [ ] **Step 5: Commit** `feat(voice): a session can ask the hub's question and hand back the answer`.

### Task 4: Voice loop asks; `Hub::ask_aloud`

**Files:**
- Create: `crates/nidoo/src/assist/stt/ask.rs` (types; `pub mod ask;` in `assist/stt/mod.rs`)
- Modify: `crates/nidoo/src/assist/stt/service.rs` (channel, pending asks, outputs)
- Modify: `crates/nidoo/src/hub.rs` (`voice_asks: OnceLock<mpsc::Sender<AskRequest>>` beside `stt`)

**Interfaces:**
- Consumes: Task 3 inputs/outputs; `speaking_satellites`, `voice_satellites`, `Hub::speak_clip`, `room_of`, `Rooms`.
- Produces (in `ask.rs`):
  - `pub const ASK_LIMIT: Duration = Duration::from_secs(45);`
  - `pub struct AskRequest { pub target: SpeakTarget, pub text: String, pub reply: oneshot::Sender<AskOutcome> }`
  - `pub enum AskOutcome { Heard { sid: String, text: String }, Silence, NoSatellite(String), Off }` (`NoSatellite` carries the detail: `"no satellite"`, `"no satellite in <room>"`, `"no free satellite"`).
  - `impl Hub { pub async fn ask_aloud(&self, target: SpeakTarget, text: &str) -> AskOutcome }` — `Off` when `voice_asks` unset or the send fails; otherwise sends and awaits the reply under `ASK_LIMIT` (timeout or dropped reply → `Silence`).
  - `impl Hub { pub fn set_voice_asks(&self, tx: mpsc::Sender<AskRequest>) }` (pub so tests can install a fake voice).
  - `run_voice_with` creates the channel (capacity 8), calls `hub.set_voice_asks`, and selects on it.

Voice behaviour on an `AskRequest`:
1. Candidates = `speaking_satellites(target)` ∩ satellites in `self.sats` that are in `voice_satellites`; error from `speaking_satellites` → `NoSatellite(err.to_string())`. Keep those whose session is idle and whose room `Rooms::admit` takes. None → `NoSatellite("no free satellite")`.
2. New ask id (u64 counter); `pending.insert(id, Pending { reply, sats })`; each sat gets `ask_id = Some(id)` and `Input::AskStart`; spawn `speak_clip(SpeakTarget::Satellite(sid), text, None, None)` per sat → `Event::AskClip { sid, clip: Option<String> }` → `Input::AskClip`.
3. `Output::Answer(Some(text))` from `sid` → send `Heard { sid, text }`, feed `AskCancel` to the pending's other sats, drop the pending. `Output::Answer(None)` → remove `sid` from the pending; none left → send `Silence`.
4. On every tick, a pending whose `reply.is_closed()` → feed `AskCancel` to its sats and drop it.
5. `Output::Release` keeps doing hold 0 + `rooms.done` (so cancelled/answered sats free their room).

- [ ] **Step 1: Write the failing tests** in `service.rs` tests (reuse the existing UDP rig that fakes a board and a `Stt` with `heard` text):
  - `ask_aloud_hears_the_answer`: one greeted satellite; `hub.ask_aloud(Room("kitchen"), "Je chauffe ?")` while the rig plays the follow-up wake + audio with `heard = "oui"` → `Heard { sid, text: "oui" }`.
  - `busy_satellites_are_skipped`: the satellite in a wake-word exchange → `NoSatellite("no free satellite")`, no `nidoo-sound/speak` publish.
  - `ask_aloud_without_voice_is_off`: hub without `set_voice_asks` → `Off`.
  - `closed_asker_releases_the_satellite`: start an ask, drop the future after the clip; next ticks → hold 0 published, session idle.
  - `silence_answers_silence`: no follow-up wake → `Silence`.
- [ ] **Step 2: Run** `cargo test -p nidoo assist::stt::service` — Expected: FAIL.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** `cargo test -p nidoo assist::stt` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(voice): the hub can ask a question out loud and wait for the answer`.

### Task 5: Flow engine `ask` step, save checks, MCP text

**Files:**
- Modify: `crates/nidoo/src/flows/engine.rs` (`NodeKind::Ask` arm → `run_ask`)
- Modify: `crates/nidoo/src/flows/hub.rs` (`check_speak_rooms` covers Ask; name-not-entity check covers Ask names; text-placeholder list includes `ask.text`)
- Modify: `crates/nidoo/src/mcp/mod.rs` and `crates/nidoo/src/mcp/params.rs` (one sentence on `ask` beside `speak`)
- Test: `crates/nidoo/tests/flows.rs`

**Interfaces:**
- Consumes: `Hub::ask_aloud`, `AskOutcome` (Task 4); `match_answer`, `first_number`, `not_understood` (Task 2); `fill_text`, `speech_lang`.
- Produces: `async fn run_ask(hub: &Hub, results: &mut BTreeMap<String, f64>, trigger: …, params: &AskParams) -> (StepOutcome, String, Vec<String>)` (same tuple as other arms).

Step logic: `text = fill_text(…)`; target from `room` (`Room`/`All`); `ask_aloud`. `Heard{sid,text:h}` → `match_answer`; miss → `ask_aloud(Satellite(sid), "<not_understood> <text>")`, `Heard` again → match; still miss/`Silence` → `none`. On a match `i`: number = `first_number(h).or(params.default)`; when `name` is set and number is `Some`, `results.insert(name, n)`. Port `(i+1).to_string()`. `Silence` / `NoSatellite(r)` / `Off` → port `none`, outcome Ok. Details:
`asked "<text>" → heard "<h>" → <label>` (+ ` (<n>)` when a number was kept), `asked "<text>" → no answer`, `asked "<text>" → heard "<h1>", "<h2>" → none`, `ask: <reason>`, `ask: speech input is off`.

- [ ] **Step 1: Write the failing tests** in `tests/flows.rs` (helper `ask_node(params)` like `speak_node`; fake voice: `hub.set_voice_asks(tx)` and a task answering each `AskRequest` from a scripted `Vec<AskOutcome>`, recording the requests):
  - `set_flow_resolves_the_ask_room_and_refuses_an_unknown_one` (mirror of the speak test).
  - `ask_yes_takes_port_1`: script `[Heard{sid:"a",text:"oui"}]` → run step ports `["1"]`.
  - `ask_number_feeds_value_from`: answers Oui/Non, `name:"temp"`, edge 1 → `command` `set_temperature` with `value_from:"temp"`; script `"oui à 21"` → the climate command publishes 21.
  - `ask_default_used_without_number`: `default: 20.0`, `"oui"` → command gets 20.
  - `ask_retries_once_then_none`: script `["bof","toujours bof"]` → ports `["none"]`; second request target is `Satellite("a")` and its text starts with `"Je n'ai pas compris."` (hub language fr).
  - `ask_without_voice_goes_none`: no fake installed → ports `["none"]`, outcome Ok, detail contains `speech input is off`.
- [ ] **Step 2: Run** `cargo test -p nidoo --test flows ask` — Expected: FAIL.
- [ ] **Step 3: Implement** `run_ask`, the save checks and the MCP sentence (`ask` — says a question in a room or the house, exits `1..n` per answer then `none`; a number heard is kept under `name`).
- [ ] **Step 4: Run** `cargo test -p nidoo --test flows` and `cargo test -p nidoo mcp` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(flows): ask step — speak, listen, branch on the answer`.

### Task 6: Web model helpers, picker and sentence

**Files:**
- Modify: `crates/nidoo-web/src/flow_edit.rs` (answer helpers; `ask` in `next_node_id` naming)
- Modify: `crates/nidoo-web/src/flow_picker.rs` (Then → Ask)
- Modify: `crates/nidoo-web/src/flow_text.rs` (`role`, `sentence`, `exit_label`, `title`/`subline`)
- Modify: `crates/nidoo-web/src/flows.rs` and `crates/nidoo-web/src/i18n/flows.rs` (FlowText entries EN+FR)

**Interfaces:**
- Consumes: `AskParams`, `AskAnswer`, `ASK_NONE_PORT` (Task 1).
- Produces:
  - `pub fn add_answer(flow: &mut Flow, node: &str, answer: AskAnswer)` (no-op at 6)
  - `pub fn move_answer(flow: &mut Flow, node: &str, from: usize, to: usize)`
  - `pub fn delete_answer(flow: &mut Flow, node: &str, index: usize) -> usize` (wires removed) — all three reuse `remap_exits` like the switch rule helpers; `none` untouched.
  - FlowText: `Ask`, `AskSub` ("Asks out loud and waits for the answer" / "Pose la question à voix haute et attend la réponse"), `DefaultAskText` ("Shall I turn the heating up?" / "Je monte le chauffage ?"), `AskTextLabel`, `AskRoomLabel` (reuse Speak's if identical), `AskAnswersLabel` ("Answers"/"Réponses"), `AskAddAnswer`, `AskWordsLabel` ("Also accepts"/"Accepte aussi"), `AskNameLabel` ("Keep the number as"/"Garder le nombre sous"), `AskDefaultLabel` ("If no number"/"Sans nombre"), `AskNoAnswer` ("No answer"/"Pas de réponse").
  - Picker default node: `Ask(AskParams { text: DefaultAskText, answers: [Yes/Oui, No/Non] in the UI language, ..Default })`.
  - `sentence`: `Ask "<text>" in <room>: Oui / Non / no answer` (FR `Demande « <text> » dans <pièce> : Oui / Non / pas de réponse`; no room → "everywhere"/"partout", as Speak words it).
  - `exit_label`: port `"k"` → answer k's label; `"none"` → `AskNoAnswer`.
- [ ] **Step 1: Write the failing tests**: `delete_answer_renumbers_later_exits` (mirror of `delete_rule_renumbers_later_exits`, plus `none` edge kept), `move_answer_keeps_wires_with_their_answer`, picker default test like the Speak one (`p.answers.len()==2 && p.room.is_none()`), `ask_sentence` EN and FR, `ask_exit_labels`.
- [ ] **Step 2: Run** `cargo test -p nidoo-web` — Expected: FAIL.
- [ ] **Step 3: Implement**; fix exhaustive matches the compiler flags.
- [ ] **Step 4: Run** `cargo test -p nidoo-web` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): ask card in the picker, its sentence and exits`.

### Task 7: Web card editor, result pickers, user docs

**Files:**
- Modify: `crates/nidoo-web/src/screens/flow_canvas/settings.rs` (`ask_fields`, dispatch arm beside `speak_fields`)
- Modify: the `value_from` and `if` result pickers (wherever they list `math_names`; they now get ask names through Task 1)
- Modify: `docs/users/flows.md` (an **Ask** section after **Speak**)

**Interfaces:**
- Consumes: Task 6 helpers and FlowText; `speak_fields`' text and room widgets (factor a shared `question_fields` if both use the same markup).

Editor: question textarea (placeholders, as Speak) · room select (as Speak) · answers list: each row label input, "Also accepts" comma-separated words input, ↑/↓ and delete buttons (delete disabled at 1 answer), "+ Add an answer" hidden at 6 · "Keep the number as" input validated with `is_valid_name`, error shown inline · "If no number" numeric input shown only when a name is set (clearing the name clears `default`). Canvas exits render from `ports()` + `exit_label` as for switch, so no canvas change beyond what the compiler asks.

Docs: what the card does, an example (coming home → "Je monte le chauffage ?" Oui / Non, number into a climate command), the no-answer exit, the one retry, needs speech input on a satellite with a mic.

- [ ] **Step 1: Write the failing tests**: a `settings.rs` (or flow_text) unit test that the name input rejects `"Temp"` and accepts `"temp"`, and that clearing the name drops `default` (put the pure logic in a small fn `apply_ask_name(p: &mut AskParams, raw: &str) -> Result<(), String>` and test that).
- [ ] **Step 2: Run** `cargo test -p nidoo-web` — Expected: FAIL.
- [ ] **Step 3: Implement** the editor and docs.
- [ ] **Step 4: Run** `cargo test -p nidoo-web` and `cargo clippy -p nidoo-web --target wasm32-unknown-unknown -- -D warnings` (or the repo's usual web check from CI) — Expected: PASS. Then build the web locally and look at the card in the browser (Night theme) with an Ask node added, three answers, one deleted.
- [ ] **Step 5: Commit** `feat(web): edit an ask card; docs: the Ask card`.

### Finish

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, per-crate tests.
- [ ] Push `feat/flow-ask-node`, open PR against `main`. After merge: rebuild hub + web on the house server.
