# Speech on the satellites: say a sentence in one room or the whole house

Date: 2026-11-04. Status: design, awaiting approval. Builds on
[satellites + nidoo-sound](2026-11-04-satellite-sound-design.md) (project 1,
shipped in PR #156) and [assist TTS](2026-11-03-assist-tts-design.md) (the hub
already turns text into a WAV clip). This replaces the speech half of the old
"project 2" plan; the ESPHome native API part is not needed for it.

## Intent

The owner wants to make the house speak **when they decide**: pick a room (or
all rooms), type or send a sentence, and the satellites there say it over the
music. Nothing speaks on its own: no automatic assistant replies, no
announcements the hub invents. A small button on each satellite's card says a
test sentence on that satellite, to check it works.

## Goals

- Say a sentence on: every satellite of one room, every satellite of the
  house ("all rooms"), or one satellite (the test button).
- While the sentence plays, the music on the satellites that speak is turned
  down (ducked) and comes back after; satellites elsewhere keep playing as is.
- With no music playing, the sentence plays on its own.
- Ways to send one, all through one hub function:
  1. a **Speak** flow action (text + room or all rooms), so flows, schedules
     and ambiances' "run flow" can speak;
  2. an MCP tool `speak` (text + room or all), so assistants can be told to;
  3. a "Say something" box on the web **Settings → Sound** page (text + room
     picker with "All rooms");
  4. the test button on each satellite card (one satellite, fixed sentence).
- The text is spoken in the hub's house language with its chosen voice (the
  existing TTS settings). Optional language/voice in the API and the MCP tool,
  as `speak` already accepts.

## Non-goals

- The assistant speaking its replies on satellites, wake word, microphone,
  speech-to-text (later projects).
- The Android app (it keeps today's controls).
- Talking to the boards over the ESPHome native API.
- Speech on players other than nidoo-sound satellites (browser tabs, the
  phone).
- Pausing the music instead of ducking it.

## Anti-goals (would be a failure even if it works)

- A sentence plays on a satellite the owner has not **Allowed**, or in a room
  that was not asked for.
- Satellites in one room speak out of sync with each other (one echo).
- The music on other rooms glitches, drops or restarts because one room speaks.
- A sentence sent while another is still playing cuts it off or overlaps it.
- Any house-, room- or person-specific word in code, tests or docs.

## How it works (decided)

### Who resolves the room: the hub

nidoo-sound knows satellites only by short id; rooms live in the hub
(each satellite's device is put in a room like any device). So the hub turns
"room X" / "all rooms" / "this satellite" into a list of short ids:

- the devices whose id starts with `nidoo_sound_` (the short id is the rest),
- in the asked room (the room of the device's Volume entity), or all of them,
- whose **Allowed** switch is ON and whose **Connected** is ON.

An empty list is an error the caller sees ("no satellite can speak in this
room"), not a silent no-op.

### The hub's speak function

One function in the hub, used by the HTTP route, the flow action, the MCP tool
and the web UI:

1. resolve the target satellites (above);
2. call the existing `Tts::speak` (text, optional language, optional voice) →
   a clip id and its URL `/api/v1/assist/speech/{id}.wav` (WAV, 16-bit mono,
   the engine's rate, 24 kHz today);
3. publish, not retained, QoS 1, on `nidoo-sound/speak`:
   `{"id": "<clip id>", "url": "/api/v1/assist/speech/<id>.wav", "players": ["<sid>", …]}`.

HTTP: `POST /api/v1/sound/speak` with
`{"text": "...", "room": "<area id or name>" | null, "satellite": "<sid>" | null, "language"?: "...", "voice"?: "..."}`
(`room: null` and no `satellite` = all rooms; both set = 422). Answers 202 with
`{"players": n}`, 422 for a bad text/room, 409 when TTS has no language set up
or no satellite can speak there. Same error style as the other routes.

Flow action: node type `speak`, params `{"text": "...", "room": "<area>" | null}`
(null = all rooms), validated against the house like the other actions (an
unknown room is refused at save). A failed speak fails the node like a failed
notify.

MCP tool `speak`: `{text, room?, language?, voice?}`; omitted room = all rooms.

### nidoo-sound plays it

- It finds the hub at the address in `<data_dir>/hub.json` (`http.bind`,
  host replaced by `127.0.0.1` when it is `0.0.0.0`), overridable with
  `NIDOO_SOUND_HUB_URL`; it already reads that file for the broker login.
- On `nidoo-sound/speak` it fetches the WAV (the clip streams while the hub
  synthesises; it waits for the end of the file before playing), converts it
  to the stream format (48 kHz stereo 16-bit), and queues it.
- Timeline: the sentence starts on the shared clock at `now + lead`, the same
  way music chunks are timestamped, so every target satellite plays it at the
  same instant (anti-goal: no echo).
- With music playing: for the target players only, each 20 ms chunk becomes
  `music × duck + speech`, with a short fade into and out of the duck; other
  players get the untouched chunk.
- Without music: the targets get a stream start, the speech chunks, then a
  stream end (a satellite already streaming keeps its stream).
- One sentence at a time: a new one waits until the previous ends.
- *(My call)* Music that starts or stops while a sentence plays never cuts
  it: music that starts joins the target satellites already ducked under the
  rest of the sentence; music that stops leaves the targets' stream open until
  the sentence ends, then the stream ends as usual.
- *(My call)* The hub keeps a clip 60 s, so nidoo-sound fetches it as soon as
  the message arrives (no queueing before the fetch).
- Only players in the list that are connected and Allowed at play time speak.
  Mute and volume apply as for music (a muted satellite stays silent).

### Web UI

- Settings → Sound (being built on the same branch): a "Say something" box,
  a room picker listing the rooms that hold at least one satellite plus
  "All rooms", and a Send button. Errors from the API are shown inline.
- Satellite card (approved separately: one card per satellite, volume slider,
  mute, online dot, "Allow" for new ones): a small speak button that sends the
  test sentence to that one satellite. The sentence is a fixed, translated
  text naming the satellite ("Speaker test: <satellite name>" / "Test du
  haut-parleur : <nom>"), spoken in the house language.

## Left to the builder

- The duck level and fade lengths (start around −15 dB and 150 ms, tune by
  ear), the WAV parsing and the resampling method (24 → 48 kHz is ×2; keep it
  general for other engine rates).
- How the per-player mixed chunk is built in the group actor, as long as
  non-target players keep receiving the shared chunk unchanged.
- HTTP client choice in nidoo-sound (prefer one already in the workspace).
- Limits: queue length (a few sentences), fetch timeout, max clip length (the
  hub already caps text length).
- Exact i18n wording in the existing tables, English and French.
- Test layout, following each crate's existing patterns (fake clock and fake
  players in nidoo-sound, wasm UI tests in nidoo-web, flow validation tests).

## Constraints

- No new service, port or `.env` value is needed for a Docker Compose install.
- The hub and nidoo-sound restart independently; a speak message that arrives
  while nidoo-sound is down is lost (not retained) and that is fine.
- Public, generic repo: no house words.
