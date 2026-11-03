# Assist: a French text assistant in the hub

Date: 2026-11-02 · Status: design, awaiting approval

## Intent

Nidoo should understand short French sentences — "allume la lumière du salon",
"mets la chambre de Léa à 19", "monte le chauffage d'un degré au bureau",
"il fait combien dehors ?" — and act on the house or answer, with a one-line
reply. This is the first step towards a local voice assistant: today the text
is typed in the web UI; later the same brain will receive text from speech
recognition (on the hub, from ESP32 satellites, from the Android app).

The user's bar is **never do the wrong thing**: switching off instead of on,
or setting the wrong setpoint on a heater, would make them stop using it.
Being a bit limited is fine; being confidently wrong is not.

### Why rules, not a model

A dry-run test on the target hardware (an x86 mini PC, CPU only) compared
small local LLMs driving the house through tools:

- Given Nidoo's MCP tools directly, a 1.7B model got 0/3 right, took
  60–100 s, called a tool it was not given and set a zone to "1" °C.
- Given three tiny purpose-built tools (lights, heating, temperature) with
  room resolution and safety checks done in code, the same model got 15/16
  in ~10 s median. Its one mistake ("d'un degré" sent as setpoint 1) was
  caught by the safety check in code.
- The Android app's voice design already measured LLMs at 3–6 s on the same
  machine and chose deterministic rules.

So version 1 is **deterministic rules only**: instant, predictable,
testable. An LLM fallback may come later behind the same safety layer; it is
out of scope here.

## Goals

1. A new pure crate `nidoo-assist` that turns a French sentence plus a
   snapshot of the house into a **plan**: an understood intent with its
   targets and actions, a question back to the user, or "not understood".
2. Three intents, done well:
   - **Lights on/off** for a room, or for named lights in a room.
   - **Heating setpoint** for a climate zone: absolute ("à 19", "19,5
     degrés") or relative ("+1", "d'un degré", "monte un peu", "baisse").
   - **Temperature reading** for a room, or outdoors.
3. Safety decided in code, never guessed: ambiguous room → ask; setpoint
   outside a comfort range → ask "oui ?" before acting; anything unclear →
   do nothing and say so.
4. Replies written by code from templates: short, French, always naming the
   room and, for heating, the exact value ("Chambre de Léa réglée à 19 °C").
5. A web **Assistant** page (conversation + "how it understood" trace) and a
   **floating button** on every page that opens it.
6. A **Simulation** switch on the page: the plan and reply are produced but
   nothing is sent to the house — the safe way to try phrasings.
7. A per-turn log in the hub so the page shows past turns, and a **Faux**
   button that marks a turn as wrongly understood.

## Non-goals (version 1)

- Speech: no microphone, speech-to-text, wake word, text-to-speech or
  satellites. (Direction for later is in *Future*.)
- No LLM fallback.
- No other intents: shutters, brightness levels, colours, alarm, scenes,
  ambiances, flows, sensor readings other than temperature, setpoint queries.
- No learned shortcuts ("Apprendre"): only **Faux**, which flags a turn in
  the log. *(User decision; shortcuts are in Future.)*
- English sentences (the UI chrome stays bilingual; the assistant speaks
  French).
- No change to the Android app. No MCP tool.
- No multi-step dialogue beyond one pending question (a choice or a
  confirmation).

## Anti-goals

- Acting when unsure. A wrong action is worse than "Je n'ai pas compris".
- Any device-, house- or person-specific word in code, tables or tests
  (architecture rule: generic over the registry). Room and person names come
  only from the house's areas.
- A reply that hides what was done ("C'est fait") — the reply always names
  the room, the action, and the value.

## Constraints

- `nidoo-assist` is pure data and logic: no I/O, builds for `wasm32`, like
  `nidoo-alarm` / `nidoo-climate` (serde, optional `schemars` feature).
- The hub executes through existing paths only: `Hub::send_command` for
  lights, `Hub::set_climate_override` for setpoints, existing views for
  readings. No new device code.
- CI: `cargo fmt --check`, `clippy -D warnings`, wasm32 `cargo check` on the
  web crates, web `bare_text` guard (UI literals via i18n).
- Parse and plan in well under 10 ms for a house of a few hundred entities.
- The database is optional in Nidoo; the assistant must work without it.

## How it works

### 1. Normalising the sentence

Lowercase, fold accents (reuse the folding of `nidoo_api::search`; make it
public or share it), split into words, drop apostrophe elisions ("l'",
"d'"), and turn French number words and decimals into numbers ("dix-neuf" →
19, "19,5" / "19.5" → 19.5, "un" / "d'un" → 1 where a quantity is expected).
Typo tolerance as in the app: a word of 5+ letters may differ by one edit
from a lexicon word — **except** that on/off verbs never fuzzy-match each
other and a fuzzy match never decides between two intents.

### 2. Lexicon (French, generic)

Port the French tables of the app's rule engine — repository
`Gekkotron/Nidoo-app`, `core/voice/src/commonMain/kotlin/com/gekkotron/nidoo/voice/`
(`VoiceLexicon.kt`, `RuleMatcher.kt`, tests in `core/voice/src/commonTest/`) —
for the words this version needs, and extend them for heating:

| Concept | Words (examples, not exhaustive) |
|---|---|
| on | allume, allumer, allumez, active, mets en marche… |
| off | éteins, éteindre, éteignez, coupe, couper, désactive… |
| light | lumière(s), lampe(s), led(s), éclairage, plafonnier… |
| heating | chauffage, radiateur(s), température (with a value), clim… |
| raise / lower | monte, augmente, hausse / baisse, descends, diminue |
| a little | un peu, légèrement |
| temperature question | quelle température, il fait combien, combien fait-il, température (with "?") |
| everything | tout, toutes les lumières |
| outdoors | dehors, extérieur |
| room linkers | dans, du, de la, au, chez, chambre de |
| noise | s'il te plaît, peux-tu, tu peux, merci, stp |

The exact tables are the builder's; tests pin the behaviour.

### 3. Resolving the room

Candidates come only from the house's areas (id and display name, folded).
"chez Léa", "chambre de Léa", "la chambre de Léa" and "Léa" resolve to an
area named "Léa" or "Chambre Léa". Exact folded-name match beats substring
match; two equally good areas → ask. No room said:

- lights / temperature → ask "Dans quelle pièce ?" with the candidate rooms
  (rooms that have lights / a thermometer) as choices, at most 6; when only
  one room can be meant (a named light that exists in a single room, or a
  house with lights in a single room) the lights act there without asking
  *(user decision, 2026-11-03)*;
- heating → same, with active climate zones as choices; a house with a
  single active zone uses it.

A room that has no target for the intent gets a plain answer ("Pas de
chauffage piloté dans le Bureau.", "Pas de lumière dans le Garage.").

### 4. Intents

**Lights.** Verb on/off is required; "tout" with on/off in a room means its
lights. The room's lights are exactly the set an ambiance's "lights" action
uses — `nidoo_api::ambiances::category_members` (primary, enabled, not
folded, not a group, not virtual, not a Z2M group, effective kind Light,
effective area). This deliberately includes lights hidden in the UI and any
light-kind entity such as a TV's ambient lighting: "éteins le salon" does
what the ambiance "lights off" does. *(User decision.)* Words left after removing verb, room and noise filter that
set by name ("la led du salon" → lights in Salon whose name contains "led");
a filter that matches nothing → ask with the room's lights as choices.
Execute each member with `Hub::send_command` (`TurnOn` / `TurnOff`), skipping
members already in the target state or offline, like ambiances do.

**Heating setpoint.** Applies to a climate zone (zone id = area id).
- Absolute: a number with "à", "degrés", "°" or after "mets/règle … à".
- Relative: "+1", "-0,5", "d'un degré", "de deux degrés"; "monte"/"baisse"
  with no amount, or "un peu", = **0.5 °C**.
- The base of a relative change is the zone's current decided setpoint
  (`ZoneView.decision.setpoint`). If it is "off" or unknown → ask for an
  absolute value.
- Round to the zone's step and clamp-check against its range, taken from
  the zone's climate entities' capabilities (`temp_step`, `min_temp`,
  `max_temp`): with several devices, the coarsest step and the narrowest
  range; no step → 0.5 °C. A target outside the range is refused with the
  range in the reply, never clamped silently. *(Builder-check call.)*
- **Safety gate**: act directly when the target is within the house's
  assistant range (default **16–23 °C**, configurable); outside it, or more
  than 3 °C from the current *reading*, ask "Passer le Salon à 25 °C ?"
  with Oui / Non. Never act on a value the zone's own range check would
  refuse; report the hub's refusal verbatim in French.
- Execute with `Hub::set_climate_override(zone, …)`: a **zone-wide**
  override (`entity: None`), origin `User` — the same shape the web and MCP
  overrides use — with `until` = the zone's next scheduled change, at most
  `manual_hold_max_min` from now: the hold a hand change on a device gets.
  That computation lives privately in `climate/engine.rs`
  (`record_hand_change` → `hold_until`, `secs_until_next_change`); extract
  it into a shared hub function used by both, with a test that both paths
  give the same `until`. *(Builder-check call.)* A zone that is not active →
  "Le chauffage du Salon n'est pas encore actif."
- "Allume / éteins le chauffage" (no value) is not a setpoint: reply that a
  temperature is needed ("Dis-moi une température, par exemple « mets le
  salon à 19 »"). Turning climate off is out of scope.

**Temperature reading.** A climate zone's reading (`ZoneView.reading`) if
the room is a zone; otherwise the room's thermometer by the same rule the
climate engine uses (`climate/inputs.rs::room_sensor`: primary temperature
sensor of the area, smallest id). Outdoors → the house's `outdoor_entity`,
else `weather_temperature`. A reading older than `stale_after_min` is said
with its age ("17,5 °C, mesuré il y a 3 h"). No thermometer → say so.

### 5. Plans and pending questions

`nidoo-assist` returns one of:

- `Act { intent, actions, reply }` — the hub executes, then fills the reply
  from what actually happened (members skipped, a command not echoed).
- `Answer { reply }` — readings and plain refusals.
- `Ask { question, choices, pending }` — ambiguous room / light / missing
  value; each choice is a resolved follow-up.
- `Confirm { question, pending }` — the safety gate.
- `NotUnderstood { reply, examples }` — "Je n'ai pas compris." plus up to
  3 example sentences built from the house (a real room name, a real zone).

The hub keeps **one pending question per client** in memory for 60 s. The
answer is either a tapped choice / Oui / Non (sent as a structured reply) or
a typed "oui", "non", "le salon"… Any other sentence cancels the pending
question and is handled fresh.

### 6. Replies (templates, French)

Examples: "Salon allumé (3 lumières).", "Lumière table éteinte.",
"Chambre de Léa réglée à 19 °C (avant 17,5 °C).", "Il fait 17,5 °C dans le
salon.", "Il fait 7 °C dehors.", "Quelle pièce ? Salon, Bureau ou Chambre de
Léa.", "Un des 3 n'a pas répondu : Led." In Simulation the reply is prefixed
"Simulation :" and says what *would* happen. The builder owns the wording;
every reply names room + action + value.

### 7. Hub side

- `crates/nidoo/src/assist/` wires the crate: builds the house snapshot
  (areas, entity views, climate view, house settings) per request, runs the
  plan, executes actions through `Hub`, writes the log, keeps pending
  questions.
- HTTP under `/api/v1` (types in `nidoo-api` or `nidoo-assist::view`):
  - `POST /assist` `{ text?, reply?: { pending, choice | yes | no }, simulate, client }`
    → the turn (reply, outcome, trace, pending question if any).
  - `GET /assist/turns?limit=` → recent turns, newest first.
  - `POST /assist/turns/{id}/wrong` → marks a turn as wrongly understood.
- Errors follow the existing `HttpError` style.
- Settings: the safety range (min/max °C) in the existing settings store,
  defaults 16 / 23; nothing else configurable in v1.

### 8. Log

Each turn: id, time, client, text, simulate flag, outcome kind, the trace
(normalised words, intent, room and how it was resolved, entities, value and
arithmetic, safety gate result, actions and their results, elapsed µs), and
the "wrong" flag. Stored in a new DB table (migration), purged after 90 days
like the restart log; without a DB, the hub keeps the last 100 turns in
memory and the page says history is not kept.

### 9. Web

- **Assistant page** at `/assistant`, in the sidebar. Left: the conversation
  (user line, reply line, choice / Oui-Non chips while a question is pending)
  and the input field (Enter sends; focus on load). Right (below on narrow
  screens): **how it understood** for the selected turn — the trace above in
  readable form — and a **Faux** button. A **Simulation** switch near the
  input, off by default, remembered per browser.
- **Floating button** on every page except `/assistant`: a round button in
  the bottom-right corner (above the toasts' z-index rules; not covering the
  drawer or sheets) that navigates to `/assistant`. Later it will open the
  conversation as a panel over the page; not in v1.
- All UI strings via the i18n tables (FR/EN); the assistant's replies are
  French.

## Tests (behaviour that must be pinned)

- Crate unit tests on a fictional house (Salon with 3 lights incl. a group
  and a hidden one, Bureau without heating, two bedrooms named after people,
  an outdoor sensor, two active zones and one pending): every intent,
  every room phrasing, "la chambre" ambiguity, relative/absolute setpoints,
  "d'un degré" ≠ setpoint 1, decimals with comma, the safety gate both
  sides, on/off never confused, unknown sentence → no action.
- The 16 sentences of the spike, rewritten generically, as a regression
  table.
- Hub HTTP tests: act, simulate (nothing published), pending choice then
  answer, confirm then Oui / Non, expiry, wrong flag, no-DB fallback.
- Web: page renders, sends, shows trace, chips answer a pending question,
  floating button navigates, bare-text guard.

## Docs

`docs/users/assistant.md` (what it understands, examples, safety rules,
Simulation, Faux), a short section in `docs/users/web-ui.md`,
`docs/reference/http-api.md` (the three endpoints), and the crate in the
Crates table of `docs/contributors/architecture.md`. `TODOS.md` gets the
future items below.

## Future (direction, not scope)

- **Speech in, on the hub, in Rust.** Follow Home Assistant's
  Speech-to-Phrase idea: a NeMo CTC acoustic model in ONNX (French:
  Conformer CTC large) run with `ort`, decoded against a grammar compiled
  from the *same* sentences this crate understands plus the house's names,
  with a confidence gate; whisper.cpp as fallback. Silero VAD for end of
  speech. Validate accuracy (allume/éteins) and CPU latency in a spike
  first.
- **Speech out**: Phonon (Rust runtime; weights are CC BY-NC, so download,
  never bundle) or Piper.
- **Satellites**: own ESP32-S3 boards streaming 16 kHz audio over MQTT, in the
  spirit of the earlier Athena-Voice protocol.
- **LLM fallback** behind the same plan + safety layer, with tiny tools.
- **Learned shortcuts** shared with the Android app; English.

## Left to the builder

Exact lexicon tables and reply wording; module layout inside the crate;
snapshot type shapes; DB schema details; the page's visual design within the
existing tokens; whether the pending store is keyed by a client id the page
generates.
