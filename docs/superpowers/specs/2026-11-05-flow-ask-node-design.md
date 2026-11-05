# Flow `ask` node — design

Date: 2026-11-05 · Status: draft for approval

## Intent

The house should be able to **offer** something out loud at a good moment and act on the spoken
answer, instead of acting on its own or staying silent. Examples from the owner:

- On coming home: "Il fait 17 °C, je monte le chauffage ?" → *oui* / *oui, à 21* / *non*.
- "Je chauffe l'étage ?" → *oui* / *seulement la chambre* / *non*.
- Solar is exporting: "On exporte 1,2 kW, j'allume le chauffe-eau ?" → *oui* / *non*.

Flows already decide *when* (triggers, `if`, `limit`) and *what* (`command`, `math`). `speak`
already says a sentence in a room. Speech input (wake word, follow-up window, Parakeet
transcription) landed on 11-04/11-05. `ask` joins the two: speak a question, listen once,
branch on the answer.

## Goals

- A new flow node `ask`: a question (with the same placeholders as `speak`), an optional room,
  a list of answers the owner names, and a timeout path.
- One output port per named answer, plus a fixed `none` port (silence or no usable answer).
- A number heard in the answer ("oui, à 21") is kept as a run result usable by later cards,
  like a `math` card's result.
- One "didn't understand" retry when something was heard but matched no answer.
- Editable on the web flow canvas, readable in the flow's sentence summary, settable over MCP
  `set_flow` (schema follows from the model).

## Non-goals

- Free commands in the answer ("oui, et allume la cuisine"): not routed to Assist.
- Asking over the phone (push notification with buttons): not in this change.
- Multi-turn dialogues inside one node: one question, at most one retry.

## Anti-goals (fails even if it technically works)

- A run left hanging: every wait has a limit, an empty house always ends on `none`.
- An `ask` hijacking a wake-word conversation already in progress on a satellite.
- The room's music left held/ducked after the exchange.
- Nagging: the node itself never repeats beyond the one retry; repetition control stays with
  the flow (`limit`).

## The node

Wire tag `ask`, params `AskParams`:

| Field | Type | Meaning |
|---|---|---|
| `text` | string, required | The question; same rules and placeholders as `speak.text`. |
| `room` | area id, optional | Satellites of that room; absent = every satellite. Resolved and checked like `speak.room` (`check_speak_rooms` extends to `ask`). |
| `answers` | list, 1–6 entries, required | Each `{ "label": string, "words"?: [string] }`. `label` is the port name shown on the card and matched against what was heard; `words` are extra phrasings (synonyms). Labels unique (case- and accent-insensitive), non-empty. |
| `name` | optional, `[a-z][a-z0-9_]{0,31}` | Result name for a number heard in the answer; same namespace and rules as a `math` card's `name` (two `ask`/`math` cards may share a name on different branches, as math cards already may). |
| `default` | optional number, only with `name` | The result when the answer carries no number ("oui" alone → `default`). Without it, no number heard = no result, and a later `value_from` on that name fails as a missing math result does today (`result_of` → step failed). |

Ports (`NodeKind::ports`): `"1"`..`"n"` in `answers` order (like `switch` rules), then `"none"`.
Ports are positional so renaming a label keeps the wiring. The card shows the labels.

Built-in yes/no words: when a label is (case/accent-insensitive) *oui*/*yes* or *non*/*no*, the
common phrasings of the hub's speech language are added automatically (oui: ouais, vas-y,
d'accord, ok, volontiers…; non: pas maintenant, laisse, non merci…; English equivalents).
**Builder's call**: the exact lists, kept in one place next to the assist French words.

## Runtime

1. Fill `text` (`fill_text`, `speech_lang`), as `speak` does.
2. Pick the satellites: those that can both speak and listen (in `speaking_satellites` *and*
   `voice_satellites`), of the room or of the house. A speaker without a mic is never asked. Skip any satellite already in a
   voice exchange (its `Session` is not idle). None left → port `none`, run detail
   `no free satellite in <room>` (or `no satellite`).
3. Speak the question as a clip on each chosen satellite, then reopen each one's window as a
   follow-up once its clip is spoken (the existing `Spoken { clip }` → `FollowUp` path, same
   `FOLLOW_UP_MS`/`FOLLOW_UP_START_MS`). The music hold works as for an Assist question.
4. The transcript of that follow-up goes to the waiting `ask`, **not** to Assist. First
   satellite with a transcript wins; the others' windows are closed/released.
5. Match the transcript (normalized: lowercase, accents folded, punctuation stripped, wake word
   stripped as today) against each answer's label + words. Best match wins; ties → first in
   order. The transcript to match is the one the session already produced (`strip_wake_word`
   applied in `session.rs`); the ask path diverts it at the point where a session would call
   the Assist `AskFn`. **Builder's call**: the matcher (token containment / fuzzy like Assist's name
   matching), as long as "oui oui vas-y" → oui, "seulement la chambre" → that label, and
   "non" never matches "oui".
6. Number: the first number in the transcript (digits; French and English number words up to
   100 are both always active, hyphenated or not — "vingt et un", "vingt-et-un", "twenty-one";
   decimal comma accepted) becomes the run result `name` when `name` is set. No number →
   no result (later cards fall back as they do when a math result is missing).
7. Heard but no match → speak "Je n'ai pas compris." + the question again (localized), listen
   once more on **the satellite that answered**; still no match or silence → `none`.
8. Silence (window closed with no speech) → `none`, no retry.
9. Whole node bounded: never longer than ~45 s (question playback + 2 windows + retry), on top
   of `SPOKEN_WAIT_MS` limits already in sessions.

Run detail lines (shown in the run log), e.g. `asked on 1 satellite(s): "…" → heard "oui à 21" → oui (21)`,
`asked …→ no answer`, `asked … → heard "bof" twice → none`.

Without speech input enabled on the hub (`Stt` absent) the node still saves; at run time it
goes straight to `none` with detail `speech input is off`.

### Plumbing (shape, builder decides details)

- The voice service (`assist/stt/service.rs`) owns sessions; the flow engine reaches it through
  the hub (e.g. a `hub.ask(target, text, timeout) -> AskOutcome` that sends a request over a
  channel the voice loop selects on, with a `oneshot` for the transcript).
- `Session` gets a hub-initiated path: idle → speak clip → follow-up → transcript delivered to
  the waiting asker instead of the `ask` callback to Assist. Pure-state tests in `session.rs`
  like the existing follow-up ones.
- The run awaits the outcome in the engine like `delay` does; other runs keep going.

### Settled after the builder review

- **Clip ownership**: the voice loop owns the question like an Assist reply: it calls
  `speak_clip(SpeakTarget::Satellite(sid), …)` (returns the clip id) and waits for that
  satellite's `Spoken { clip }` before the follow-up, exactly as `default_ask` does today.
- **Wake word during an ask**: the session is not idle while it asks, so a board wake on that
  satellite is ignored, as it is during an Assist reply. On "every satellite", the ones not
  chosen (busy at start) are untouched.
- **First answer wins**: the other satellites' windows are closed and holds released; their
  question clips are left to finish (no stop).
- **Cancellation / restart**: the session holds the asker as a `oneshot::Sender`; when the
  receiver is gone (run cancelled, flow disabled or deleted), the session goes back to idle and
  releases the hold at its next tick. A hub restart loses the run like a pending `delay`.
- **Web answer delete**: same as `switch` rules (`flow_edit.rs` delete-rule): the deleted
  answer's wires go, later ports are renumbered; `none` is untouched.

## Web

- Picker: **Then → Ask** next to Speak, sub-line e.g. "Asks out loud and waits for the answer".
  Default node: question "Je lance … ?"-style default in the UI language, answers *Oui* / *Non*.
- Card editor: question (same field as Speak, placeholders), room picker (same as Speak),
  an answers list (add/remove/reorder, label + optional extra words), "Keep the number as"
  name field (optional, same validation as math name).
- Canvas: one output handle per answer label + "No answer", like `switch`'s rule handles.
- `flow_text` sentence: `Ask "<text>" in <room>: Oui / Non / no answer`. EN + FR strings.
- `value_from` pickers and `if`'s result picker list `ask` names alongside math names.

## Validation (`nidoo-core` `validate`)

`BadAsk { node, reason }`: empty text, broken placeholders (as `BadSpeak`), 0 or > 6 answers,
empty/duplicate labels, bad `name`. Edges from an `ask` node must use one of its ports.

## Docs & MCP

- The MCP instructions' node list gains `ask` (one sentence + ports).
- A section on the flows docs page next to Speak.

## Tests (minimum)

- core: round-trip, ports, validation errors.
- session: hub-initiated ask → follow-up → transcript to asker; busy satellite skipped; silence.
- engine/hub (`tests/flows.rs` style, fake voice): yes → port 1; "oui à 21" → port 1 + result
  21 feeding a `command` `value_from`; unmatched twice → `none`; no satellite → `none`.
- matcher unit tests with the French phrasings above.
- web: picker default, flow_text sentence.

## Calls made by the designer (check these)

- `room` absent = ask on every satellite, first answer wins.
- A busy satellite is skipped, not waited for.
- Silence → `none` immediately; only an unmatched answer gets the one retry.
- Ports positional (`1..n`, `none`), like `switch`.
- Number goes into the math-result namespace via `name`, not a fixed `{answer_number}`.
- No-speech-input hub: node still saves, runs to `none`.
- Up to 6 answers.
- Only satellites with both a speaker and a mic are asked.
- Optional `default` number for "oui" without a number.
- French and English number words always both understood.
- A wake word during an ask is ignored on that satellite.
