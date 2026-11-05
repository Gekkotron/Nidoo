# Assist shutters, and the assistant settings in Settings

Date: 2026-11-05. Status: draft for approval.

Three independent parts, built in this order: (A) two visual fixes on the speech
settings, (B) the assistant settings move into Settings, (C) shutters in Assist.
(A) lands as part of (B) because the fixed page is the moved page.

## Why

- The speech settings page (`/assistant/settings`) has two visual bugs the user
  pointed at: the first card is glued to the page title, and the model picker
  shows a doubled green ring when focused next to a "Prêt" tag much shorter than
  the field.
- Everything else configurable lives in Settings. The assistant's speech output
  (TTS), speech input (STT) and the speaker satellites (Settings → Sound) are
  spread over two places. The user wants them together in Settings, TTS and STT
  configured separately.
- Assist understands lights and heating. The user wants to say "ouvre / ferme les
  volets" and "ouvre les volets à 30 %", and to close every shutter in the house
  with one sentence.

## Goals

1. Assist opens, closes and positions shutters from a French sentence, typed on
   the Assistant page or spoken to a satellite (same planner, so both work).
2. Settings has an **Assistant** group with three rows: **Voix** (TTS), **Écoute**
   (STT) and **Son** (the satellites page, moved from the House group).
3. The ⚙ button stays on the Assistant page and becomes a plain link to that
   group in Settings.
4. The two visual bugs are gone on the new pages.

## Non-goals

- "stop", "un peu plus / moins", "à moitié", reading a shutter's position back
  ("les volets sont ouverts ?"). Not asked for; left out (my call).
- English sentences. Assist's lexicon and replies are French today; shutters
  follow that.
- Any change to what the speech pages do: same cards, same controls, same API.
  This is a move and a restyle of two details, not a redesign.
- Shutters in ambiances or flows: they already handle covers.

## Anti-goals

- Assist must never move a shutter the user did not ask for. A sentence it is
  unsure about gets a question or "pas compris", never a guess. In particular
  "monte le salon" must still never mean heating (existing rule) and must not
  start meaning shutters either.
- No bookmark or doc link to the old page may break.

---

## Part A — the two visual fixes

- **Gap under the title.** On the new Settings sub-pages the page title comes
  from the Settings sub-page frame (`SettingsPageScreen`), which already spaces
  its title from the content like every other Settings page. The speech pages
  drop their own back link and `<h1>` (`assist-set__back`, `assist-set__title`).
  Check the result against another Settings sub-page (System, Alarm) at desktop
  and phone width: same title, same gap.
- **Model picker.** The model `<select>` in the Languages card shows one focus
  ring, the house's usual one (same as other selects in Settings), not its
  border plus an offset outline. The status tag beside it ("Prêt", "Chargement…",
  "Erreur", …) has the field's height and is vertically centred with it, at
  every width. Same treatment for any other select + tag row on these pages.

## Part B — the assistant settings in Settings

### The Settings list

A new list section **Assistant** (EN "Assistant"), placed between the House and
Hub sections, with three rows in this order:

| Row (FR / EN) | Slug | Hint (FR / EN) | Status on the right |
|---|---|---|---|
| Voix / Voice | `assistant-voice` | Synthèse vocale (TTS) : langues, voix, modèles / Text to speech (TTS): languages, voices, models | the house language's state: "Prêt" when its model is loaded, otherwise its engine state or "Aucun modèle" |
| Écoute / Listening | `assistant-listening` | Reconnaissance vocale (STT) et clé des satellites / Speech recognition (STT) and the satellites' key | the STT engine state ("Prêt", "Chargement…", "Aucun modèle", …) |
| Son / Sound | `sound` (unchanged) | unchanged | unchanged |

`SettingsPage` gets `AssistantVoice` and `AssistantListening`; `Sound` moves from
the House group to the new Assistant group. Glyphs and tones: builder's choice,
consistent with the list (e.g. a speech bubble for Voix, a microphone for
Écoute).

### The pages

- **Voix** (`/settings/assistant-voice`): today's Languages card, the Installed
  models card, and the add-a-model form with the Hugging Face token, in that
  order. The token only serves speech-output models, so it stays here.
- **Écoute** (`/settings/assistant-listening`): today's `SpeechInputSection`
  (recognition model, its settings, the satellites' shared key), unchanged.
- **Son**: unchanged page, unchanged slug.

Each page is rendered by `SettingsPageScreen`, so it gets the Settings breadcrumb,
the "← Réglages" back link and the Settings title spacing. The page bodies move
out of `assist_settings.rs` into components the Settings frame renders
(`AssistVoicePage`, `AssistListeningPage`); their polling and load logic is
unchanged. Each page polls only what it shows (Voix the TTS status, Écoute the
STT status).

### Links

- ⚙ on the Assistant page links to `/settings#assistant` and the Settings list
  scrolls the Assistant section into view when the URL carries that anchor. If
  the router makes anchors awkward, `/settings` with the section scrolled into
  view by a query (`/settings?section=assistant`) is fine: builder's choice, the
  user only asked that it go to the global Settings.
- `/assistant/settings` stays as a route that redirects (replace, not push) to
  `/settings/assistant-voice`, so bookmarks work.
- The Settings breadcrumb of the three pages reads "Réglages / Voix" etc.
- User docs updated: `docs/users/assistant.md` (the "gear button … Speech
  settings" paragraph becomes Settings → Assistant → Voix / Écoute),
  `docs/users/sound.md` and `docs/users/talk-to-a-satellite.md` (Settings → Sound
  becomes Settings → Assistant → Son; STT key location), and any path in
  `docs/reference/voice-protocol.md`. Doc comments naming `/assistant/settings`
  are updated.

## Part C — shutters in Assist

### What the user can say

| Sentence | Does |
|---|---|
| "ouvre les volets du salon" | opens every shutter in the Salon |
| "ferme les volets du salon" | closes them |
| "ouvre les volets du salon à 30 %" | moves them to 30 % open |
| "ferme le volet de la baie" | the shutter whose name holds "baie" (same name matching as lights) |
| "ferme tous les volets" | closes every shutter in the house, no question |
| "ouvre tous les volets" | opens every shutter in the house, no question (my call: the mirror of the line above) |
| "ferme les volets" (no room, no "tous") | asks "Quelle pièce ?" with the rooms that have shutters, unless only one room has any |
| "ne ferme pas les volets" | nothing (existing negation rule) |

Words (my call, French only): nouns `volet(s)`, `store(s)`, `volet(s) roulant(s)`;
open verbs `ouvre / ouvrir / ouvrez / remonte / remonter / monte / monter` when a
shutter noun is in the sentence; close verbs `ferme / fermer / fermez / baisse /
baisser / descends / descendre` likewise. Polite forms ("peux-tu fermer…")
follow the existing Request rule. "monte" / "baisse" without a shutter noun keep
today's meaning (no heating guess; "monte le salon" stays not understood).

Position: "à N %" or "à N pour cent", N in 0–100, means N % open, the convention
the cover card shows (0 = closed, 100 = open). Only with an open verb. "ferme …
à 30 %" is ambiguous (30 % closed or 30 % open?), so Assist answers with a hint:
« Pour une position, dites par exemple « ouvre les volets à 30 % ». » (my call).
N outside 0–100 answers that the position goes from 0 to 100 %.

### How it is resolved

It mirrors the lights path in `crates/nidoo-assist/src/plan.rs`:

- A new `Intent::Covers { open: bool, position: Option<u8>, all: bool, names,
  room }` in `intent.rs`. The shutter nouns leave `Class::OtherDevice` in
  `lexicon.rs` (plugs, sirens… stay there). The test
  `assert_eq!(p("ferme les volets du salon"), None)` becomes a positive test.
- Members come from `category_members` with `Category::Covers` (same filters as
  ambiances: primary, enabled, not grouped, not a group or virtual).
- Room, name and "which one?" questions work like lights: words naming no
  shutter but naming a room are the room; a named shutter not found in the room
  gets "Quel volet du Salon ?" with up to `MAX_CHOICES` choices; no room and no
  `all` → `pick_room` over the rooms that have shutters.
- `all` with no room → every shutter in the house; `all` with a room → that
  room's shutters.
- A new `Action::Cover { entity, command }`. Command: `Open` / `Close` when the
  cover has `open_close`, else `SetPosition { 100 / 0 }` when it has `position`;
  a position request → `SetPosition { position }`.
- Position asked, some chosen covers lack `position`: act on those that have it
  and the reply names the others as unable ("Baie ne sait pas se mettre à une
  position."). None has it → plain answer, nothing sent.
- A new `Done::Covers { area: Option<String>, open, position, names }`
  (`area: None` = whole house) for the reply.

### Execution (hub, `crates/nidoo/src/assist/hub.rs`)

`Action::Cover` runs like `Action::Light`: unknown → failed, offline → skipped
as offline, already there → skipped, Simulation → ok without sending, otherwise
`send_command` and its accepted/reason. "Already there": state `open` / `closed`
for open/close, or the reported position equal to the asked one. Acceptance is
the existing echo rule (the cover's first state echo, e.g. `opening`); Assist
never waits for the shutter to finish moving. `what()` gives "ouvrir", "fermer",
"ouvrir à 30 %" for the trace.

### Replies (`reply.rs`)

Same shape as lights, French agreement included. Examples:

- one shutter: « Volet « Baie » fermé dans le Salon. » / « … ouvert à 30 % … »
- a room: « Salon : volets fermés (3 volets). » / « … ouverts à 30 % (3 volets). »
- whole house: « Tous les volets fermés (7 volets). »
- already there, failures and offline: the lights wording with « volet(s) ».

Exact wording: builder's choice within that shape; it must read naturally when
spoken by a satellite.

### Docs

`docs/users/assistant.md` gains a "Shutters" part with the sentences table
above.

## Testing

- `nidoo-assist`: intent tests for each sentence in the table (and the
  not-understood ones: "monte le salon", "ferme à 30 %" hint), plan tests with a
  fixture house holding covers with and without `position`, in two rooms
  (room, named, all-house, ask room, single-room shortcut, partial position).
  `fixture.rs` gets a `cover(...)` builder.
- Hub: an execute test that a cover action sends `Open` / `SetPosition` and is
  skipped when already there, and that Simulation sends nothing.
- Web: unit tests for the new `SettingsPage` slugs and groups (Sound in
  Assistant), the `/assistant/settings` redirect, and the ⚙ href; wasm tests
  that each page mounts its card. Screenshot check of Voix at 1440 px and phone
  width for Part A.

## Settled after the builder check

- **Inverted shutters.** The snapshot's cover state and position are already
  normalised (`nidoo-core/src/state/kinds/cover.rs` maps
  `position_closed..=position_open` onto 0–100, and the command layer maps back).
  The `invert` UI override only applies to switches. Assist reads and sends
  0 = closed, 100 = open with no extra mapping.
- **The early rejection in `intent.rs`** (Percent / OtherDevice / Negation →
  not understood) stays for every sentence without a shutter noun. With a shutter
  noun, the shutter branch runs first: Percent is allowed there, and "ferme … à
  N %" gets the hint. Negation still rejects everything. (my call)
- **Capabilities.** Plain open/close: `Open` / `Close` with `open_close`, else
  `SetPosition 100 / 0` with `position`, else that cover is reported as unable
  and gets nothing. A position: only covers with `position`; the others are
  named as unable. (my call)
- **Redirect.** If the router has no redirect helper, a small component that
  navigates with `replace` on mount is fine.
- **Text keys.** FR and EN only, like every other key.
- **Statuses on the Settings list.** The list fetches the TTS and STT status
  once when it opens (no polling), like its other row statuses. A failed fetch
  leaves the status empty. (my call)

## Left to the builder

Glyphs and tones of the new rows; anchor vs query for the ⚙ link; exact reply
wording within the shape above; how to share the polling code between the two
pages; CSS class names.
