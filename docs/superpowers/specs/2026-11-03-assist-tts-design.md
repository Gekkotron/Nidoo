# Assist speaks: text-to-speech in the hub

Date: 2026-11-03 · Status: approved

## Intent

Assist (`docs/superpowers/specs/2026-11-02-assist-design.md`) understands typed
French sentences and replies with one line of text. The next step towards a
local voice assistant is for the hub to **say** that line. The user will later
build ESP32-S3 satellites (microphone + speaker) that talk to the hub; the hub
must be able to speak **before any board exists**, so the board project only
has to fetch and play audio.

This project (A) is the hub side only: a text-to-speech engine inside the hub,
a screen to install and choose models and voices, and an HTTP address that
serves spoken audio. The satellite link (project B: ESPHome API, Sendspin…) is
a separate brainstorm.

Engine: Gradium **Phonon**, through its Rust crate
[`ptts`](https://crates.io/crates/ptts) (code MIT OR Apache-2.0, no PyTorch,
24 kHz mono, streams audio in chunks). The French-capable models
(`gradium/phonon-40m-fr`, `gradium/phonon-40m-multilingual`) are
**CC BY-NC 4.0 and gated** on Hugging Face: each user accepts the terms and
downloads them with their own token. Nidoo never ships the weights.

## Goals

1. The hub synthesises French speech from text with a locally installed
   Phonon model, on the CPU, with no cloud call at speaking time.
2. An **Assist settings** screen at `/assistant/settings`, opened by a ⚙
   button in the Assistant page header *(user decision: inside Assist, not
   global Settings)*, to:
   - store a Hugging Face **token** (masked, write-only);
   - **add a model** by pasting a Hugging Face URL: the hub lists the repo's
     runnable weight files with their sizes, the user picks one (q8
     preselected), and the hub **downloads it in the background** with a
     progress bar *(user decision)*;
   - **upload** a model from the computer as the offline path *(user
     decision)*;
   - see **several installed models** in a list, with an **Active** radio and
     **Remove** *(user decision)*; only the active one is loaded in memory;
   - pick a **voice** for the active model and **Test** it: type a sentence,
     hear it in the browser *(user decision)*.
3. A **protocol-agnostic speech address**: any client (the browser today, a
   satellite tomorrow, whatever protocol B picks) gets spoken audio by
   fetching an HTTP URL that streams it as it is generated *(my call)*.
4. Everything survives a hub restart: installed models, active model, voice,
   token.

## Non-goals

- Satellites, microphones, speech-to-text, wake word, the satellite protocol
  (project B).
- Speaking Assist replies automatically anywhere (on the Assistant page or in
  a room). This project makes speech *available*; who speaks when is decided
  with the satellites.
- Other engines (Piper, cloud TTS), voice cloning or designing voices.
- GPU / NPU backends: CPU only.
- An MCP tool or Android app support.
- English replies (the engine can speak English; Assist speaks French).

## Anti-goals

- **Shipping or committing model weights**, or fetching gated files with any
  credential other than the user's own token. *(my call, forced by the
  licence)*
- Leaking the token: in a response body, a log line, an error message, the
  page source, or a URL.
- Speech that slows the house down: synthesis must never block the hub's
  event loop, MQTT handling or the web UI.
- A hub that fails to start, or an Assist that stops answering in text,
  because a model is missing, half-downloaded or broken. Speech is optional;
  its failure is a status on the settings screen, never a crash.
- A download that fills the disk or leaves a half-written model marked as
  installed.
- Any house-, room- or person-specific word in code, tests or docs (project
  rule: generic).

## Facts the design rests on (checked 2026-11-03)

- `ptts` 0.6.0 (crates.io, 2026-10-30; six releases in three weeks, so pin
  the exact version). Edition 2024, like the hub. Features used: `hf`
  (tokenizer). **Not** `hub`: that pulls `hf-hub` and reads `HF_TOKEN` from the
  environment; the hub does its own downloading so it controls the token,
  progress and size checks. Library API: `Checkpoint::resolve(folder,
  ResolveOptions { quant, weights })` → `builder(Lang::Fr).device(Cpu).build()`
  → `Synth`; `say`, `say_with(text, SpeechOptions::voice(name))`,
  `stream(text)` (bounded PCM chunks), `voices()`, `languages()`,
  `sample_rate()`, `add_voice_file(name, path)`, `session_default` for
  repeated requests in one voice.
- A Phonon model repo (all three checked) holds: `config.json`,
  `generation_config.json`, `processor_config.json`, `tokenizer.json`,
  `tokenizer.5.0.0.json`, `tokenizer.model`, `tokenizer_config.json`,
  `chat_template.jinja`, `default-voice.safetensors` (256 KB), and three
  alternative weight files: `model.q8.gguf` (~75 MB for 40m),
  `model.q4k.gguf` (~60 MB), `model.safetensors` (~122 MB, f32).
- **A model repo has one voice, `default`.** The named voices are in a
  separate public catalog, `gradium/phonon-voices` (Apache-2.0, not gated):
  `voices.jsonl` lists 102 voices (15 French) with name, language, locale,
  gender, age, description, and a 256 KB `voices/<id>.safetensors` each. The
  crate pins the catalog to a commit (`DEFAULT_VOICE_CATALOG`).
- Hugging Face API: `GET https://huggingface.co/api/models/{repo}` (gives
  `gated`, `sha`, licence), `GET …/api/models/{repo}/tree/{rev}?recursive=true`
  (paths, sizes, and `lfs.oid` = sha256 for large files), and files at
  `https://huggingface.co/{repo}/resolve/{rev}/{path}` with
  `Authorization: Bearer <token>`. Listing works on gated repos without a
  token; downloading their files needs a token whose account accepted the
  terms.

## How it works

### 1. The engine in the hub

- A new module `crates/nidoo/src/assist/tts/` owns the engine. `ptts` is a
  dependency of the hub crate only (never of `nidoo-assist`, which stays pure
  and wasm-buildable), behind a cargo feature `tts`, **on by default** *(my
  call: a platform where `ptts`/`xn` fails to build can still build the hub
  without speech)*.
- Exactly one model is loaded: the active one. Loading happens in the
  background at boot and on activation; until it is ready the engine reports
  `loading`, and on failure `error` with a readable message. Switching
  models loads the new one first, then drops the old one.
- Synthesis runs on a **dedicated thread** (not the async runtime), one
  request at a time, fed by a small bounded queue. A full queue answers
  `503` at once rather than piling up *(my call; the builder picks the queue
  length, 4 is a sensible start)*. The builder limits the engine's CPU
  threads so the hub stays responsive.
- Language is French for every request (`Lang::Fr`). A model whose config does
  not list French can be installed and activated, but the screen warns that
  it does not speak French *(my call)*.

### 2. The speech address

*(my call)* Two steps, the shape Home Assistant also uses for satellites:

1. `POST /api/v1/assist/speech` `{ text, voice? }` → `{ id, url }`, where
   `url` is `/api/v1/assist/speech/{id}.wav`. Text is capped (the builder
   picks the cap; ~500 characters is plenty for Assist replies). No active
   model or engine not ready → `409` with the reason.
2. `GET /api/v1/assist/speech/{id}.wav` streams **WAV, 24 kHz, 16-bit, mono**
   as chunks come out of the synthesiser (header with unknown length, then
   PCM). WAV because every target can play it without a codec: browsers, and
   ESPHome's media player.

An id is single-use-or-short-lived (the builder chooses: one fetch, or a
minute, so a client can retry once) and unguessable. Ids live in memory only.
The text never appears in the URL, so it does not land in access logs or
browser history. Resampling (to 16 or 48 kHz) is the player's job, not the
hub's.

### 3. Models on disk

- Stored under the hub's data directory (`NIDOO_DATA_DIR`, `./data`;
  `/data` in Docker): `data/assist/tts/models/<model-id>/`, one folder per
  installed model, holding the repo's small files, the **one** chosen weight
  file, and a `manifest.json`: repo, commit sha, weight file, sizes, sha256s,
  source (download or upload), installed time. *(my call: files, not the
  database, because the database is optional in Nidoo and these are files
  anyway.)*
- `<model-id>` is derived from repo and weight file
  (e.g. `gradium--phonon-40m-fr--q8`); the same repo with two weight files is
  two models.
- A folder without a valid manifest is not a model: it is ignored by the list
  and deleted on boot. Downloads and uploads write into a temporary folder
  and are **renamed into place only once complete and verified** (sizes and,
  where the listing gave one, sha256).
- Active model id and chosen voice in `data/assist/tts/state.json`.
- Catalog voices downloaded on demand go to `data/assist/tts/voices/`
  (shared by all models).
- Removing a model deletes its folder; removing the active one unloads it and
  leaves no model active.

### 4. Adding a model from Hugging Face

1. The user pastes a URL. Accepted forms: `https://huggingface.co/owner/repo`
   (with or without `/tree/<rev>` or a trailing path), `hf://owner/repo[@rev]`,
   or `owner/repo`. Anything else, or any host other than `huggingface.co`, is
   refused *(my call: the hub only ever fetches from Hugging Face)*.
2. The hub reads the repo (`api/models` + `tree`), pins the **commit sha** it
   saw, and answers with: the repo, whether it is gated, its licence, and the
   **runnable weight files** with sizes: `model.q8.gguf`, `model.q4k.gguf`,
   `model.safetensors` (only those `ptts` can load on CPU; the builder confirms
   q4k). q8 is preselected *(user's request, q8 default my call)*. A repo
   without `config.json`, a tokenizer and at least one runnable weight file
   is "not a Phonon model".
3. Gated repo and no token stored → the screen says so before downloading.
   A `401/403` during download → "This account has not accepted the model's
   terms on Hugging Face (or the token is wrong)", with a link to the repo
   page.
4. **Download** starts a background job: small files + the chosen weight
   file + `default-voice.safetensors`, streamed to disk (never held whole in
   memory), each checked against its listed size and sha256. Before starting,
   the hub checks free disk space for the total plus a margin and refuses
   otherwise. One download at a time; another request while one runs → `409`.
5. Progress (bytes done / total, current file, state) is read by polling
   while the screen shows a running job *(my call: polling, ~1 s, no new
   WebSocket message type)*. **Cancel** stops it and deletes the temporary
   folder. A failed job keeps its error until the next attempt. A hub
   restart drops an unfinished job (its temporary folder is cleaned on boot);
   no resume *(my call)*.

### 5. Uploading a model (offline path)

The user picks the model's files from their computer (a folder picker, or a
multi-file picker) and the browser sends them **one file per request**,
streamed to the hub, into a temporary model folder; a final request closes it
*(my call)*. Limits: only the file names a Phonon repo holds (list in §Facts),
one weight file, each file capped (the builder picks the
caps; the largest real file is ~122 MB, so ~200 MB for weights and a few MB
for the rest). The closing request checks the folder is a loadable model
(config, tokenizer, one weight file) and renames it into place, with source
"upload" and no repo / sha. No sha256 to compare against; sizes and a load
test are the check.

This is a new kind of request body for the hub: today's only upload (the push
key) is a small JSON. The upload routes need their own body limit and must
stream to disk; every other route keeps the current limit.

### 6. Voices

*(my call — the facts above change what "the active model's voices" can
mean)* The voice list shows:

- the model's own voice, **Default**;
- the catalog voices **in the languages the model speaks** (for a French
  model: the 15 French voices), with name, gender, age and the catalog's
  one-line description.

The catalog list (`voices.jsonl`, public) is fetched from the commit the
crate pins and cached on disk. Choosing a catalog voice downloads its 256 KB
file once, then registers it with `add_voice_file`. Offline, the list shows
only the voices already on disk. The chosen voice is saved per hub (not per
model); a voice the newly active model cannot use falls back to Default with a
note on the screen.

### 7. Token

*(my call)* Stored with the same rules as the push key: write-only, reads
return only `{ "set": bool }`, `PUT` with absent / `null` / string = keep /
clear / set (`setup::secret`). It is sent only to `huggingface.co`, in the
`Authorization` header, never in a URL. Changing the token **does not restart
the hub** (the settings-section `PUT` restarts it, which would kill a running
download); the builder chooses where it is stored as long as it survives a
restart, is readable only by the hub, and stays out of logs and backups'
plain-text exports if any exist. Clearing the token does not remove installed
models.

### 8. The settings screen

Route `/assistant/settings`, reached by a ⚙ button in the Assistant page
header; a back link returns to `/assistant`. Sections, top to bottom:

1. **Speech**: engine status (No model · Loading · Ready · Error + message)
   and the active model.
2. **Test**: a text field prefilled with a generic French sentence, a
   **Play** button that requests speech and plays the returned URL in an
   `<audio>` element, and the time to first sound.
3. **Voice**: the picker of §6.
4. **Installed models**: one row per model: name (repo or "Uploaded"), weight
   file and size, short commit, licence, French yes/no; an **Active** radio;
   **Remove** with the app's usual confirmation.
5. **Add a model**: URL field → **List files** → radio list of weight files
   with sizes → **Download**; progress bar with **Cancel**. Below it,
   **Upload from this computer**.
6. **Hugging Face token**: masked field, Save / Clear, and one line saying
   the 40m models are gated, CC BY-NC (non-commercial), and must be accepted
   on their page first.

All strings via the i18n tables (FR/EN), as in the rest of the web UI.

### 9. HTTP

All under `/api/v1/assist/tts`, errors in the existing `HttpError` style,
types in `nidoo-api` *(names and exact shapes are the builder's)*:

| Route | Does |
|---|---|
| `GET /status` | engine state, active model, voice, token set, running job |
| `GET /models` · `DELETE /models/{id}` · `PUT /active` | list, remove, activate (or none) |
| `POST /inspect` `{ url }` | the repo listing of §4.2 |
| `POST /downloads` `{ repo, rev, weights }` · `GET /downloads/current` · `DELETE /downloads/current` | start, progress, cancel |
| upload routes | §5 |
| `GET /voices` · `PUT /voice` | the list of §6, choose |
| `PUT /token` | §7 |

Plus the speech routes of §2 at `/api/v1/assist/speech`.

## Constraints

- CI as today: `cargo fmt --check`, `clippy -D warnings`, wasm32 check of the
  web crates, the web `bare_text` guard. CI never downloads a model (gated,
  large); the engine sits behind a small trait so tests use a fake
  synthesiser that emits a tone.
- The Docker image builds the hub with `tts`. Its build stage already has
  `clang` and `cmake`; any further system dependency `xn` needs is added
  there, and the runtime image stays `debian:bookworm-slim`.
- The target is the house's x86 mini PC, CPU only, shared with the rest of the
  hub.
- Generic code and docs; Gekkotron attribution; branch from `origin/main`.

## Builder's first step: a measured spike

Before building the screen, prove the engine on the target hardware and
record the numbers in the plan:

- `ptts` 0.6.0 builds inside the hub workspace and in the Docker image
  (x86_64 Linux), and the extra build time and binary size.
- With `phonon-40m-fr` q8 on the mini PC (the release `ptts` command is
  enough for the timings): load time, memory, **time to first audio** and
  real-time factor for a typical Assist reply ("Salon allumé (3 lumières).",
  "Il fait 17,5 °C dans le salon.").
- Target *(my call)*: first audio under 1 s and synthesis faster than
  real time. If the numbers miss badly, stop and come back to the user
  before building the rest (Piper would be the alternative).

## Tests (behaviour to pin)

- URL parsing: every accepted form, refused hosts and shapes.
- Repo inspection on recorded Hugging Face responses: runnable files, sizes,
  gated flag, "not a Phonon model".
- Download against a local fake Hugging Face server: progress, size and
  sha256 mismatch → failure and cleanup, `401/403` message, cancel, disk-space
  refusal, one job at a time, token in the header and nowhere else.
- Upload: allowed names only, size caps, closing check, cleanup on abort.
- Boot: half-written folders deleted, missing active model → no model, a
  broken model → `error` status while the hub and text Assist run normally.
- Speech: WAV header and streamed chunks from the fake synthesiser, single-use
  ids, queue full → `503`, no model → `409`.
- Web: settings screen renders each state, ⚙ navigates, bare-text guard.
- One `#[ignore]` test that runs the real model when `HF_TOKEN` and a model
  folder are present, for manual runs.

## Docs

`docs/users/assistant.md` gets a "Voice" section (installing a model, the
licence, the token, voices, Test); `docs/reference/http-api.md` the routes;
`docs/contributors/architecture.md` the module and the `tts` feature;
`TODOS.md` ticks "Speech out" and keeps the satellite items.

## Left to the builder

Module layout; exact type and route names; queue length, text and file caps;
thread count; the speech id's lifetime; how the token is stored within §7's
rules; whether progress polling uses one route or the status route; the
screen's visual design within the existing tokens; the generic test sentence.
