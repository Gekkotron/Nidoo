# Speech in several languages: one model per language

Date: 2026-11-04. Builds on the speech design of 2026-11-03
(`2026-11-03-assist-tts-design.md`, PR #151) and the voice sort and ▶ preview
of PR #152.

## Intent

The hub speaks French today, with one active model and one voice for the
whole house. The user will want French and English later, and the project is
generic, so another house may want French and German, or German and English.
Speech should therefore be set **per language**: each language the house
speaks has its own model, its own voice and its own test sentence, and a
speech request says which language it wants.

At the same time, the settings screen's order is reworked. Today it reads
Speech → Try it → Voice → Installed models → Add a model → Token. You test
and pick a voice before you can see which model is in use, and the token,
which you need while adding a model, sits at the very bottom.

## Goals

1. **Languages, not one active model.** The hub keeps an ordered list of the
   languages it speaks. Each entry is a language code, the model that speaks
   it and the voice to use. The **first entry is the house language**: it
   speaks when a request names no language *(user decision: tabs, first is
   the house language)*.
2. **Generic languages.** A language is a plain code taken from the models'
   own `config.json` (`fr`, `en`, `de`…). Nothing in the hub or the screen
   lists "French and English". Any code the engine can normalise works
   *(user requirement)*.
3. **One model can serve several languages.** A multilingual model chosen for
   both `fr` and `en` is loaded **once**.
4. **Every model in use stays loaded**, so each language speaks without a
   load delay *(my call; the user did not pick, see "Facts")*. A model that no
   language uses is not loaded.
5. **Speech requests take a language.** `POST /api/v1/assist/speech` takes
   `{text, language?, voice?}`. Without `language` it uses the house language.
   `voice` is resolved within that language's model. A named voice still
   speaks without being chosen, as in #152.
6. **The settings screen uses language tabs (layout B)** *(user decision,
   picked from a mockup)*: see "The screen".
7. **Upgrading is invisible.** The current `{active, voice}` state becomes a
   one-entry list: the active model's first language, with its model and
   voice.

## Non-goals

- Assist understanding or replying in another language. Assist stays French.
  This project only makes speech in several languages *available*. Nothing
  calls speech from Assist today (no spoken replies yet, checked
  2026-11-04), so no reply path changes. When spoken replies come, they pass
  the language the reply is written in, not the house language.
- Picking the language automatically from the text (language detection).
- Satellites choosing a language. They will pass `language` later.
- The Android app and MCP.
- Per-language engines other than Phonon.

## Anti-goals

- Hard-coding French, English or any language pair anywhere: in code, in the
  screen, or in i18n beyond display names.
- Loading the same model twice because two languages use it.
- A speech request silently speaking the wrong language. An unknown or
  unconfigured `language` is refused (`422`), never swapped for the house
  language.
- Losing the user's current model or voice on upgrade.

## Facts the design rests on (checked 2026-11-04)

- **Language is only text normalisation in `ptts` 0.6.** `Checkpoint::builder`
  takes a `Normalize` (today `Lang::Fr` in `phonon.rs`). `Normalize::OFF`
  hands text through unchanged, and `Normalize::for_lang(lang).apply(text)`
  can be run by the caller. So one loaded `Synth` built with `Normalize::OFF`
  can serve every language its model speaks: the hub normalises each request
  with that request's language first. The builder checks this in the spike
  (step 1 of "Builder's first steps").
- `ptts::Lang` knows `en fr de es pt ar hi ko pl sv fi no it nl ja zh da tr`
  and parses from a string (`FromStr`). A model language that `Lang` cannot
  parse can still be configured, and speaks with no normalisation *(my
  call)*.
- `Synth::languages()` lists the languages a model was trained for. The
  store's manifest already records them (`ModelManifest.languages`).
- **Memory:** a Phonon q8 weight file is about 75 MB, and the f32 one about
  122 MB. The house hub host has 16 GB of RAM, with about 7 GB in use and the
  hub at about 680 MB (system_status, 2026-11-04). Two or three loaded models
  cost a few hundred MB, so keeping them all loaded is affordable. The builder
  records the real resident size per extra model in the plan.
- The engine (`engine.rs`) is one dedicated thread holding one `Speaker`,
  with a queue of four requests. Catalog voices (`gradium/phonon-voices`)
  carry a `language` per voice.

## How it works

### 1. State

`TtsState` becomes:

```text
languages: [ { code: "fr", model: "<model id>" | null, voice: "default" | "<name>" }, … ]
```

- Order matters: index 0 is the house language.
- A code appears at most once.
- `model` may be `null`: the language is listed but has no model (for
  example after its model was removed). It cannot speak, and the screen says
  so.
- Codes are normalised before use and comparison: trimmed, lower-cased, and
  must match `[a-z]{2,3}(-[a-z0-9]+)*` (`fr`, `en`, `pt-br`), else `422`.
  Normalisation uses the part before `-` when parsing a `ptts::Lang` *(my
  call)*.
- A model whose manifest lists **no languages** can be used for any code. It
  is offered under every language with "languages unknown", so it is never
  stranded *(my call)*.
- Removing the first entry makes the next one the house language. An empty
  list is allowed; speech then answers `409` *(my call)*.
- Migration on read: an old file with `active` and `voice` becomes one entry.
  The code is the active model's first manifest language, or `fr` when the
  manifest lists none *(my call)*. With no active model, the list is empty.
  The new shape is written back on the next save.

### 2. Engines

- One **engine per model in use** (`Engine` as today: its own thread and
  queue), keyed by model id. The set of loaded models is the set of distinct
  non-null `model` values in `languages`.
- Any change to `languages` reconciles: start and load engines for newly used
  models, and drop engines for models no longer used. A model used by two
  languages keeps its one engine.
- Each `Speaker` is built with `Normalize::OFF`. `Engine::speak` gains the
  language, and the speaker applies `Normalize::for_lang(lang)` to the text
  when the code parses as a `ptts::Lang`.
- CPU threads: the engines share the existing limit (half the cores by
  default, `RAYON_NUM_THREADS`). How engines share the pool is left to the
  builder, as long as two languages speaking at once do not starve the hub.
- **Failures stay per engine** *(my call)*. A model that fails to load puts
  its own languages in Error and leaves the others speaking. There is no
  fallback to another language's engine (see Anti-goals), and no automatic
  retry: as today, a model reloads when the list changes or the hub
  restarts. `fellBack` stays what it is today, a voice that fell back to
  Default; it never means another engine.
- Catalog voices are registered per engine, for the voice each language
  chose. A catalog voice is only offered for a language that matches its
  `language`.

### 3. Speech requests

`POST /api/v1/assist/speech` `{text, language?, voice?}`:

1. Resolve the language: the given code, else the house language. No
   languages configured → `409` "no language is set up". An unknown code, or
   a code that is not in the list → `422`.
2. That language has no model → `409` with the reason.
3. Voice: the given one (downloaded first if it is a catalog voice for this
   language, as in #152; unknown → `422`), else the language's chosen voice,
   else the model's default.
4. Speak on that model's engine, normalising with that language.

`GET /assist/speech/{id}.wav` is unchanged. The WAV header needs the
speaking engine's sample rate, so the clip remembers which engine made it.

### 4. HTTP management routes

Under `/api/v1/assist/tts`. The exact shapes are the builder's, in the same
`HttpError` style; types live in `nidoo-api`:

| Route | Does |
|---|---|
| `GET /status` | `{languages: [{code, model, voice, engine: EngineState, fellBack}], tokenSet, download, speechBuilt}`, where `engine` is that language's model state |
| `GET /languages/available` | the codes that installed models speak and that are not yet in the list, each with the models that speak it |
| `PUT /languages/{code}` | `{model, voice?}`: adds the language at the end, or changes its model or voice. The model must speak `code` (`422`). Changing the model resets the voice to `default` unless one is given |
| `DELETE /languages/{code}` | removes the language; its model unloads if nothing else uses it |
| `PUT /languages/house` | `{code}`: moves that language to the front |
| `GET /voices?language=fr` | the voice list of #151/#152, for that language's model and catalog voices in that language |
| `GET /models` | each model also lists `usedBy: [codes]` |
| `DELETE /models/{id}` | as today. Each language using the model keeps its entry with `model: null`. The screen confirms first (§5) |

`PUT /active` and `PUT /voice` go away. The web screen is their only client
(the app is single-user and free to change).

### 5. The screen (layout B, language tabs)

Route `/assistant/settings`, unchanged. Top to bottom:

1. **Languages** card.
   - A tab row: one tab per language, in list order, labelled with the
     language's name in the UI language and a status dot (green Ready, amber
     Loading, red Error, grey No model), then a **+ Add** tab.
   - Under the tabs, for the selected language:
     - a line saying "House language — speaks when no language is asked"
       on the first tab, or a **Make house language** button on the others
       *(my call: a button rather than dragging tabs)*;
     - **Model**: a drop-down of installed models that speak this language,
       with the state pill (Ready · Loading · Error + message · No model);
     - **Try it**: the sentence field for this language, ▶ Play, and the
       first-sound time;
     - **Voice**: the sorted list with gender, age and description, ▶ on
       each row, and the chosen voice highlighted, as in #152;
     - a quiet **Remove language** link at the bottom of the tab, with the
       usual confirmation.
   - **+ Add** lists the languages from `/languages/available`. Picking one
     with a single model adds it with that model; with several models, it
     asks which one.
   - The selected tab is remembered per browser *(my call)*.
2. **Models** card: installed models, each with its languages, size,
   licence, short commit and **Used by** (the language names). **Remove**
   confirms, and when the model is used it names the languages that will be
   left without a model.
3. **Add a model** card: Hugging Face link → List files → Download (with
   progress and Cancel), and Upload. The **Hugging Face token** is folded
   inside this card ("only needed for gated repositories", showing set or
   not set), instead of having its own card at the bottom.

Language names: the browser's `Intl.DisplayNames` in the UI language
(`fr` → "Français" / "French"), with the upper-cased code as a fallback
*(my call: no hand-written table, so any code works)*.

Test sentences: a small table of one neutral sentence per common language
(at least `fr`, `en`, `de`, `es`, `it`, `nl`, `pt`). The French one stays
the pinned "Bonjour, je suis l'assistant de la maison." For a language not in
the table, the field starts empty with a placeholder, and ▶ is disabled until
something is typed *(my call)*. Whatever is typed is remembered per language
for the session.

All strings go through `i18n/assist_settings.rs` (EN/FR), as today. The
Night and Ivoire themes come from the existing tokens. The mockup the user
picked is `.superpowers/brainstorm/…/speech-settings-layout.html`, option B
(not committed; it is a sketch, and the real page reuses the existing
`assist-set__*` styles).

## Constraints

- `nidoo-assist` stays pure and wasm-buildable. `ptts` stays behind the
  `tts` feature, and a build without it still answers the management routes
  (speech `409`), as today.
- The CI lints of #151 (including the no-speech build) keep passing.
- Token rules are unchanged (write-only, only sent to huggingface.co).
- Project rules: generic, no house names or hosts in tracked files.

## Builder's first steps

1. **Spike the normalisation fact.** Load the real 40m French model with
   `Normalize::OFF`, speak a French sentence with numbers ("il fait 19,5
   degrés") normalised by the caller, and check the audio matches today's.
   If `ptts` cannot normalise outside the builder, fall back to one engine
   per (model, language) pair, and come back to the user only if that costs
   more than about 300 MB on the hub host.
2. Measure the resident size of one extra loaded q8 model and record it.

## Tests (behaviour to pin)

- Migration: old `{active, voice}` → one entry; no active model → empty list.
- Two languages on one multilingual model → one engine. Removing one language
  keeps it loaded, and removing both unloads it.
- Two languages on two models → two engines, each Ready on its own.
- Speech: no `language` → house language; unknown or unconfigured code → `422`;
  a language with no model → `409`; a catalog voice of another language → `422`.
- `PUT /languages/{code}` with a model that does not speak `code` → `422`.
- `PUT /languages/house` reorders, and survives a restart.
- `DELETE /models/{id}` leaves the using languages with `model: null`.
- The text sent to the speaker is normalised with the request's language
  (the tone loader records what it received).
- Web: the tab for the house language is first; **+ Add** offers only
  available codes; ▶ uses the tab's language; the i18n table test covers new
  strings.

## Docs

- `docs/users/assistant.md`, "Voice": languages, house language, tabs, one
  model per language, all used models loaded.
- `docs/reference/http-api.md`, speech: the new routes, `language` on speech
  requests, removed `PUT /active` and `PUT /voice`.
- The Speech settings screenshot, if one exists in the docs, is retaken
  (Night and Ivoire pair).

## Left to the builder

- Exact route and type names, and how engines share the CPU pool.
- Whether `fellBack` stays per language or becomes a status message.
- Small visual details of the tabs, within the existing styles.
