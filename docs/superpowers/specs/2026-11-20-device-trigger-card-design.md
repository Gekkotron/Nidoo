# Device trigger card — design

Date: 2026-11-20 · Status: draft, awaiting approval · Size: small change (web editor only)

## Intent

The user wants a flow to start on any event of any device — a door opening or closing, a motion
sensor, a remote button, a tag scan. The flow engine already does this (`trigger.state` fires on
any value change of any entity, with optional `to` / `from` filters; `trigger.signal` fires on
remote presses, `event` entities and tag scans). The problem is the editor:

- The Déclencheurs list shows Programmation, Événement solaire, Heures creuses, Silencieux
  depuis, Manuel — **no state or signal trigger**. Both appear only when a device name is typed
  in the library search ("Quand Porte entrée change"), so the user could not find them.
- Once found, the state card's "Fires when" / "Only after" are raw JSON boxes: the user has to
  know whether the door reports `true`, `"ON"` or `"open"`.

## Goals

- A visible **Appareil / Device** card in the Déclencheurs list.
- Its entity picker offers every entity, whether it reports values or sends signals.
- After picking, a **Quand** list of what that entity can report (s'ouvre / se ferme, détecte /
  ne détecte plus, the signals of a remote…), plus "any change"; no JSON typing.
- Search results offer ready-made value entries for sensors: "Quand Porte entrée devient ouvert".

## Non-goals

- No engine, flow JSON, store or MCP schema change. Existing flows load and run unchanged.
- No "any door in the house" wildcard trigger (one trigger over a class of devices).
- No new trigger features from TODOS (`for` duration, numeric crossings, webhook, raw MQTT).

## Anti-goals

- A second way to express the same trigger in the saved JSON (e.g. a new `trigger.device` type).
- Losing a filter an existing flow already has (a `to` / `from` value the list doesn't contain
  must still show and stay saved).

## Approaches considered

1. **Visible card built on the existing triggers (picked).** One library entry; it saves as
   `trigger.state` or `trigger.signal` depending on the picked entity. Fixes the real problem
   (discoverability and JSON fields) with web-only changes.
2. New `trigger.device` engine node merging state and signal. Cleaner model on paper, but engine,
   migration and MCP churn for no new capability. Rejected.
3. Device-level picker listing every event of every entity of a physical device. Long noisy
   lists (an Aqara contact sensor has ~6 entities). Rejected as a model; at most, the picker can
   group entities by device.

## Design

**Library entry.** Add a generic When entry to `flow_picker::entries` (first of the triggers,
before Programmation): label "Appareil" / "Device", sub-line "Quand un appareil change ou envoie
un événement" / "When a device changes or sends an event". It inserts
`trigger.state` with an empty entity. Searchable by its names in both languages, like the other
generic cards.

**Card title.** The `trigger.state` card's title (`FlowText::NodeTrigger`, today "Déclencheur" /
"Trigger") becomes "Appareil" / "Device", so the card reads the same as the entry that made it.
The signal card keeps "Signal".

**Entity picker, one pool.** The state card's `EntityPickField` drops the `!is_signal` filter and
the signal card's drops the `is_signal` filter. Picking a signal entity (`device_automation`,
`event`, `tag`) on a state card converts the node to `trigger.signal { entity, signal: None }`;
picking a non-signal entity on a signal card converts it to `trigger.state { entity }`. The
node id, position and edges are kept.

**Quand list (state card).** Replace the "Fires when" JSON field with a value list built from
`if_value::value_input_of(entity, field)` — the same source as the If card:

- first option "N'importe quel changement" / "Any change" → `to: None`;
- then each listed value (Ouvert, Fermé, …) → `to: <value>`;
- "Autre valeur…" → typed value, parsed with `parse_other`;
- for a number entity: a number box with the unit, empty = any change;
- a saved `to` not in the list is shown as its own option (as `with_current` does for If).

"Only after" (`from`) uses the same list, first option "N'importe quelle valeur" / "Any value".
The `IfValueField` component gains an optional "none" first option (or a thin wrapper does), so
the If card's behaviour stays unchanged.

**Search entries.** For each matching primary **binary sensor** (door, window, motion, leak,
smoke…) whose `value_input` is a `List`, add one When entry per listed value after the existing
"changes" entry, inserting `trigger.state { entity, to: value }`. Its label is the card's existing
sentence ("Quand Porte entrée devient ouvert"). Other kinds get no extra entries: their values are
in the card's list, and a room search ("salon") would otherwise list every value of every light,
shutter and climate in the room. Signal entries stay as they are.

**Card summary on the canvas.** Already done by `flow_text::sentence`: a `to` reads "Quand
<entité> devient <valeur>". A card with no entity yet reads the Device sub-line.

## Testing

- `flow_picker` unit tests: the generic list has the Device entry first among When; a door
  entity search yields "changes" + one entry per value with the right `to`; labels in both
  languages.
- Conversion state ↔ signal on entity change keeps id and edges (pure helper, unit-tested in
  `flow_edit`).
- Value list: "any change" maps to `None`; a saved unlisted `to` survives a load and save.
- Existing `trigger.state` / `trigger.signal` tests keep passing; no engine test changes.

## Left to the builder

- Exact placement of the "none" option inside `IfValueField` vs a wrapper component.
