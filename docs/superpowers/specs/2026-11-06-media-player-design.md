# Media player: design

*Gekkotron · 2026-11-06 · status: approved by the owner (Android app added on request)*

## Why

The owner wants to control the house music by voice and from the dashboard: pause, resume, skip,
go back, ask what is playing, start "some music", or ask for something by name. The first music
source is a separate house music server that streams through `nidoo-sound`, but **Nidoo must stay
generic**: it never names that server. Any program that speaks the MQTT contract below is a media
player to Nidoo, the same way any Zigbee2MQTT device is a light or a shutter.

## Goals

- A new entity kind, **media player**, discovered over MQTT like every other entity.
- Commands: play, pause, next, previous, and optional play-by-name (`play_media`).
- State: playing / paused / idle, title, artist, album, cover image, cover colour, duration and
  position.
- **Assist** (French and English) can pause, resume, skip, go back, say what is playing, start
  music, play by name, and change the room satellite's music volume or mute.
- The same media player in the **Android app** (owner's request): card, control sheet, icon.
- A **dashboard card** (design "B" below) with the cover, its colour as the card tint, the
  controls and a progress line.
- Flows and ambiances can use the media player's state and commands like any other entity's.

## Non-goals

- Volume on the media player. Volume belongs to the speakers: each `nidoo-sound` satellite already
  publishes a **Music volume** number and a **Mute** switch. Assist drives those (see Assist).
- Browsing a library, queues, playlists or favourites in Nidoo. The music server's own app does that.
- Choosing which speakers play. The music server plays on every approved Sendspin player.
- History charts of what was played.

## Anti-goals

- Anything in tracked Nidoo text that names the first music server, its API or its routes.
- A Nidoo feature that only works with one music server (no HTTP calls from the hub to a player).
- "mets X" stealing house commands: "mets le chauffage à 20" must keep working exactly as today.

## The MQTT contract

This is the contract already sent to the music server's maintainer. Keep it identical; change it
only by agreement on both sides.

1. **Discovery**, retained: `<discovery_prefix>/media_player/<node_id>/<object_id>/config`
   (the usual two- and three-level discovery topic forms both apply):

   ```json
   {"name": "Music", "unique_id": "<node_id>_player",
    "state_topic": "<root>/player/state", "command_topic": "<root>/player/set",
    "availability_topic": "<root>/status",
    "supported_features": ["play", "pause", "next", "previous", "play_media"],
    "device": {"identifiers": ["<node_id>"], "name": "…", "model": "…", "manufacturer": "…"}}
   ```

   `supported_features` lists what the player accepts. A player without search leaves out
   `play_media`. Standard discovery keys (`availability` list, `device`, `object_id`,
   `entity_category`, `icon`) work as they do for the other kinds.
2. **Availability**, retained: `online` / `offline` (last will).
3. **State**, retained JSON on `state_topic`, sent on every change:

   ```json
   {"state": "playing", "title": "…", "artist": "…", "album": "…",
    "image_url": "https://…", "color": "#3d2466",
    "duration_s": 431, "position_s": 42.5, "position_at": "2026-11-06T10:20:00Z"}
   ```

   - `state` is `playing`, `paused` or `idle`. When it is `idle`, the other fields are null.
   - `color` is the cover's colour, already darkened by the player so white text stays readable.
     It may be null.
   - `position_s` is measured at `position_at`. It is only resent on play, pause, seek or song
     change. The UI moves the progress line forward itself while `state` is `playing`.
   - `position_at` may be slightly in the future, up to about 1 s: it's when the speakers will
     really start after play, resume, seek or previous. The UI shows
     `position_s + max(0, now - position_at)`, clamped to `duration_s`, so the line waits still
     until then.
   - `position_at` is null while a song is still loading after `play_media`, `next` or
     `previous` (2–9 s for the first music server): the player can't tell when the speakers will
     start. The state is then `playing` with `position_s` 0, and the line stays still at 0. The
     player publishes again with a real `position_at` once audio starts. *(agreed with the music
     server's session on 2026-11-06, after a real-broker test)*
   - Unknown fields are ignored. A missing field is the same as null.
4. **Commands**, JSON on `command_topic`, not retained:
   - `{"command": "play"}`: resumes. When the player is idle, it means "play something" and the
     player picks what.
   - `{"command": "pause"}`
   - `{"command": "next"}`
   - `{"command": "previous"}`
   - `{"command": "play_media", "query": "daft punk"}`: plays by name, using the words as heard.

The first music server's answers to these commands are its own business. For reference, the
agreed behaviour is: idle `play` shuffles favourites (or plays a mix when there are none), and
`play_media` plays the top search result, followed by similar songs when its Autoplay is on.

## Hub (core, registry, API, MCP)

- **Kind.** A new `Kind::MediaPlayer` for the `media_player` component, with its own state
  (`StateValue::MediaPlayer`) parsed from the JSON state topic. It sits beside `Vacuum` and
  `WaterHeater`. It is not a type-table generic, because the card, Assist and flows need typed
  fields.
- **Capabilities.** The capabilities come from `supported_features`: which of play, pause, next,
  previous and play_media the entity accepts. A command outside that list is refused with the
  usual "not supported" error, and nothing is published.
- **Commands.** These go through the existing command mapping and `send_command`. What counts as
  the echo that confirms a command:

  | Command | Echo |
  |---|---|
  | `play` | `state` becomes `playing` |
  | `pause` | `state` becomes `paused` |
  | `next`, `previous` | the title or position changes |
  | `play_media` | `state` is `playing` and the title changes |

  For `next`, `previous` and `play_media`, a state message whose `position_at` changed (newer,
  or now null while the new song loads) also counts. This
  covers the case where `previous` restarts the same song at 0. Use the existing echo timeout. A
  `play_media` that echoes with the same title (the same song was already playing) still counts
  as accepted.
- **Room.** A media player gets its room the way every entity does: the area from its device or
  its `nidoo/ui/<id>` override (`area`). Nothing new. With no area, it simply isn't in any room.
- **Snapshot and SSE** carry the new state, like every other kind.
- **Flows.**
  - `trigger.state` and the value/compare cards can read `state` (headline: playing / paused /
    idle), `title` and `artist`.
  - Device actions can send the five commands.
  - The flow editor lists the commands, with a text field for `play_media`.
- **Ambiances.** The `device` action works as it is, through `send_command`. No new category.
- **MCP.** The `send_command` docs gain the media player command shapes. `search` finds media
  players by kind words ("musique", "music", "lecteur", "player").
- **Groups.** A media player can't join a group.
- **History.** The `state` is recorded the way other text states are, if that comes for free. No
  chart work.

## Assist

Assist is the rule-based French parser in `nidoo-assist` (pure, wasm-safe), with English as it
already handles it. New phrases:

| Said (examples) | Action | Reply (from the echo) |
|---|---|---|
| "mets pause", "pause la musique", "stop la musique" | `pause` | "Musique en pause." |
| "reprends la musique", "relance la musique" | `play` | "Je reprends <title>." |
| "chanson suivante", "musique suivante", "passe à la suivante" | `next` | "<title>, <artist>." |
| "chanson précédente", "reviens en arrière" | `previous` | "<title>, <artist>." |
| "c'est quoi cette chanson ?", "qu'est-ce qu'on écoute ?" | none, reads the state | "<title>, de <artist>." / "Rien ne joue." |
| "mets de la musique", "lance de la musique" | `play` | "Je joue <title>." |
| "joue <words>" | `play_media` with `<words>` | "Je joue <title>." |
| "monte le son", "baisse le son" (+ "un peu", a %) | room satellite's music volume ± step | "Volume à 60 %." |
| "coupe le son", "remets le son" | room satellite's Mute | "Son coupé." / "Son remis." |

The replies are examples. Wording and English versions follow the existing reply style.

Rules:

- **A media phrase needs its media word.** Several verbs are already taken: "mets", "passe" and
  "règle" are value verbs, "monte" and "baisse" raise and lower, "coupe" switches things off.
  A sentence becomes a media or volume intent only when one of these words is present: "pause",
  "musique", "chanson", "morceau", "titre", "son", "suivante" or "précédente" (or "joue", see
  below).
  - Without such a word, every existing parse stays as it is: "passe le chauffage à 20", "coupe
    la lumière", "monte le volet".
  - A bare "passe" is not `next`.
  - "stop la musique" means `pause`. The contract has no stop, and a paused player resumes
    cleanly.
- **Play by name is "joue …" only.** "mets …" never means play by name (owner's decision), so "mets
  le chauffage à 20" and other "mets" sentences keep their meaning. "joue" has no other meaning in
  the lexicon today. If it ever gains one, music still wins after "joue".
- **The words after "joue" go to the player as heard**, without the leading article removed
  ("joue du Daft Punk" sends "du Daft Punk"; the player's search copes). *(builder may strip one
  leading "du / de la / des / le / la / les" if it tests better)*
- **Which player.**
  - With one available media player, that one is the target.
  - With several, Assist uses the one in the room of the satellite that heard the sentence.
  - If none is in that room, or the sentence came from the web, Assist asks which one, using the
    existing pending-question mechanism.
  - An unavailable player answers "Le lecteur ne répond pas."
- **Which volume.** "monte le son" targets the music volume of the satellite in the room that heard
  the sentence. From the web, or with no satellite in that room, Assist asks which room.
  - **How Assist recognises the volume controls.** `nidoo-sound` (same repo, in scope) adds an
    explicit marker to its discovery that other speakers can copy. On the Music volume `number`
    it adds `"device_class": "volume"`, and the Mute `switch` gets `"device_class": "mute"`. The
    hub keeps unknown device classes as plain strings.
  - Assist looks only for these markers. It doesn't match names, so there's no fallback.
  - The feature works once the sound container is rebuilt, which already happens after every
    `nidoo-sound` change. *(If the hub's discovery parsing refuses unknown device classes, the
    builder picks an equivalent explicit key and notes it here.)*
  - The step is 10 points for a bare "monte / baisse" and 5 for "un peu".
- **Speaker room is new context.** `plan()` doesn't know which satellite heard the sentence today.
  The hub knows (`sound::room_of`).
  - Add an optional `speaker_room` field to `Request`, so it survives a pending question and its
    answer ("which room?" → "salon"). `plan()` gains the same optional argument.
  - The web (wasm) passes `None`. The hub passes the satellite's room for voice turns.
  - Media and volume intents use it as described above.
  - Existing intents ignore it. Changing that is out of scope here: a separate decision, if ever.
- **When the echo doesn't come.**
  - A timed-out command answers "Le lecteur n'a pas répondu." (EN: "The player didn't answer.").
  - A refused command (unsupported, unavailable) answers with the existing refusal style.
- If the player lacks `play_media`, "joue X" answers that this player can't search, and nothing is
  sent.

## Web card (design B)

Approved from mockups (`.superpowers/brainstorm/…/music-card-b-tint.html`, not tracked):

- **Where it shows.** Like every other kind:
  - in its room's page when it has an area
  - in the dashboard widget library
  - in the devices list

  The existing overrides apply (name, area, hidden, favorite, order). No new override fields,
  `cardStyle` or `showAs` values.
- **Size and layout.** Two cards wide on room pages and dashboards. The cover is on the left
  (about 100 px, rounded). On the right:
  - a small label: the entity name · room
  - the title (bold)
  - "artist · album" (muted)
  - a row of ⏮ ▶/❚❚ ⏭
  - a thin progress line
- **Buttons.** Previous and next are round 32 px tiles. Play/pause is a round 40 px amber button
  with a dark icon and a soft amber glow. All icons are drawn SVG (the existing icon set), never
  text glyphs.
- **Tint.** When `color` is present, the card background is a gradient from that colour to a
  darker mix of it with the page background. The border is a lighter mix of it. The cover gets a
  soft glow in it. The tint fades (about 0.6 s) when the song changes.
  - No `color` means the plain card.
  - On a warm tint the amber play button gets a thin dark ring, so it doesn't blend in.
    *(designer's call, approved)*
- **Idle.** A note placeholder replaces the cover. "Nothing playing" is the title, and there's a ▶
  with the hint "Play some music". It sends `play`, which means "play something".
- **Unavailable.** The usual offline treatment of other cards.
- **Covers.** The cover image loads straight from `image_url` (it's another origin, so it's only
  displayed, never read). A broken image falls back to the note placeholder.
- **Buttons the player lacks** are hidden, based on the capabilities.
- **Progress.** Computed from `position_s`, `position_at` and `duration_s` while `state` is
  `playing`. It's hidden when the duration is unknown.
- **Themes.** It must look right in Night and Ivoire. In Ivoire the tint sits over the light
  background with the same darkening, so the card stays dark with white text.
- **Strings** go through the i18n layer (FR/EN). There's a new icon for the kind in pickers and
  lists.
- **Dashboard widget library and device pages** list the new kind.

## Android app (Nidoo-app repo)

The app parses MQTT discovery itself: per-kind parser, state mapper, card text, control sheet,
icon and home-screen widget tile. So it gets its own media player kind, mirroring the hub's and
following the `VacuumMapper` pattern.

- **Model.** A `MEDIA_PLAYER` kind for the `media_player` component, plus:
  - a `StateValue.MediaPlayer` with the contract's fields
  - commands play, pause, next, previous and `play_media(query)`
  - capabilities from `supported_features`

  The discovery parser and kind mapper accept the JSON state and command topics from the contract.
- **Optimistic state.** The app shows its guess until the state topic confirms, like the vacuum:
  - play → playing
  - pause → paused
  - next and previous show no guess (the title is unknown until the echo)
- **Home card.** The design B idea at the app's card size:
  - cover thumbnail, title, artist
  - a play/pause button on the card
  - the card tinted with `color` when present, else the app's normal card
  - idle: a note icon, "Rien ne joue", and ▶ sends `play`
- **Control sheet** (tap the card):
  - large cover, title, artist · album
  - ⏮ ▶/❚❚ ⏭
  - the progress line, using the same `position_at` rule as the web
  - the sheet background tinted with `color`
  - buttons the player lacks are hidden
  - no volume (it stays on the satellites, which already show in the app)
  - no search field: play by name stays a voice feature, through the app's existing hub Assist
    voice
- **Icon** in the design system (a note or speaker glyph), FR/EN strings, and an entry in the
  demo house.
- **Covers** load from `image_url` with the app's existing image loading, if it has any. If not,
  add a small one (builder's call: Coil or similar).
- **Home-screen widget tile.** Title + play/pause state as the value, with the media icon.
  Controlling playback from the widget is out of scope.
- **Discovery prefix.** The first music server publishes under `homeassistant/` by default, which
  the app already reads. No change needed.

## Docs

- `docs/users/assistant.md`: the new phrases and the "joue" rule.
- `docs/users/sound.md` (or a new `docs/users/media-players.md`, builder's call): how any program
  becomes a media player. Use the contract above, written generically, with no music server named.
- The MCP tool docs (`send_command`).

## Testing

- **Core:**
  - discovery parsing (two- and three-level topics, `supported_features`)
  - state parsing (all fields, nulls, unknown fields, idle)
  - command mapping for each command, and refusal outside `supported_features`
  - the echo rules
- **Assist:**
  - regression tests for every phrase above, FR and EN
  - "mets le chauffage à 20" and the other "mets" sentences unchanged
  - player choice with 0, 1 and 2 players, by speaker room, and the pending question
  - volume by speaker room
- **Hub:** a fake player in the assist hub tests, covering `play_media` echo and reply.
- **Android:**
  - mapper unit tests (discovery, state parsing, commands, optimistic state)
  - card and sheet UI tests for playing / paused / idle / no colour / missing capabilities
- **Web:** card rendering for playing / paused / idle / no colour / unavailable / missing
  capabilities, plus the language toggle.

## Calls made (not discussed with the owner)

- A dedicated kind rather than a type-table generic.
- A JSON state on one topic and JSON commands on one topic, rather than one topic per field.
- The echo rules and the volume steps (10 / 5).
- Asking when the target is ambiguous, rather than acting on all players.
- No media volume and no history charts.
- In the app: no search field in the sheet (voice only), and the widget tile displays without
  controlling.
- Media phrases need a media word. A bare "passe" is not `next`. "stop la musique" means pause.
- `speaker_room` goes on `Request` and `plan()`. Existing intents ignore it.
- The volume and mute markers are `device_class` `volume` / `mute`, published by `nidoo-sound`,
  with no name fallback.
- The echo for `next` / `previous` / `play_media` also accepts a changed `position_at` (newer, or
  null while loading). A timeout answers "Le
  lecteur n'a pas répondu."
- A player's room comes from the existing area system. Assist asks when a player has no area and
  there are several.

## Left to the builder

- Exact Rust names, the command JSON shape inside `send_command` (follow the existing kinds), file
  layout, and how the flow editor shows the commands.
- Exact reply wording, within the existing style.
- The generic marker for a satellite's music volume and mute (see Assist).
