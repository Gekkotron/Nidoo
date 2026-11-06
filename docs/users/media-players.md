---
title: Media players
sidebar_position: 20.5
description: Any program that speaks the media player MQTT contract shows up in Nidoo as a music player, with a card tinted by the cover, flows, the MCP tools and assistant phrases.
---

A **media player** is any program that plays music and publishes the small
MQTT contract below. Nidoo discovers it like any other device: no plugin, no
HTTP calls from the hub, nothing that depends on one music server. Once it is
there you get:

- a **card**, two cards wide, with the cover, the song, ⏮ ▶/❚❚ ⏭ and a
  progress line. When the player sends the cover's colour, the whole card is
  tinted with it. An idle player shows "Nothing playing" and a ▶ that asks the
  player to play something.
- the same card on **dashboards** (the **Music** section of the widget
  library).
- **flows**: a trigger or an `if` on the state (`playing`, `paused`, `idle`),
  the `title` or the `artist`; a device action that sends play, pause, next,
  previous or a search.
- the **assistant**: « mets pause », « chanson suivante », « joue … » and the
  others listed on the [Assistant](assistant.md) page.
- the **MCP tools**: `send_command` with
  `{"type":"media_action","action":"next"}` or
  `{"type":"play_media","query":"…"}`.

A media player has no volume in Nidoo: the volume belongs to the speakers
(see [Sound satellites](sound.md)). It cannot join a group.

## The MQTT contract

`<prefix>` is a discovery prefix the hub reads (`homeassistant` by default),
`<node_id>` names your player and `<root>` is the topic root it uses.

**Discovery**, retained, on `<prefix>/media_player/<node_id>/<object_id>/config`
(the two-level form `<prefix>/media_player/<object_id>/config` works too):

```json
{"name": "Music", "unique_id": "<node_id>_player",
 "state_topic": "<root>/player/state", "command_topic": "<root>/player/set",
 "availability_topic": "<root>/status",
 "supported_features": ["play", "pause", "next", "previous", "play_media"],
 "device": {"identifiers": ["<node_id>"], "name": "…", "model": "…", "manufacturer": "…"}}
```

`supported_features` lists what the player accepts; Nidoo hides the buttons
and refuses the commands it leaves out. A player that cannot search leaves
out `play_media`. Without the key, Nidoo assumes play, pause, next and
previous. The usual discovery keys (`availability` list, `device`,
`object_id`, `entity_category`, `icon`) work as for any entity.

**Availability**, retained, on `availability_topic`: `online` / `offline`
(set `offline` as the MQTT last will).

**State**, retained JSON on `state_topic`, sent on every change:

```json
{"state": "playing", "title": "…", "artist": "…", "album": "…",
 "image_url": "https://…", "color": "#3d2466",
 "duration_s": 431, "position_s": 42.5, "position_at": "2026-11-06T10:20:00Z"}
```

- `state` is `playing`, `paused` or `idle`. When it is `idle`, the other
  fields are null. A message with another `state` is ignored.
- Each message is the whole state: a missing field is null. Unknown fields
  are ignored.
- `image_url` must be `http://` or `https://`. The cover is only displayed,
  loaded straight from that address.
- `color` is the cover's colour as `#rrggbb`, already darkened enough for
  white text. Anything else leaves the card plain.
- `position_s` is the position at `position_at` (RFC 3339). Send it again
  only on play, pause, seek or a new song: Nidoo moves the progress line
  itself while `state` is `playing`, as
  `position_s + max(0, now - position_at)`, up to `duration_s`.
- `position_at` may be up to about a second in the future (when the speakers
  will really start): the line waits until then.
- While a new song is still loading, send `"position_s": 0` and
  `"position_at": null`: the line holds at 0 until a message with a real
  `position_at` arrives.

**Commands**, JSON on `command_topic`, not retained:

| Payload | Meaning |
|---|---|
| `{"command": "play"}` | resume; on an idle player, play something of its choice |
| `{"command": "pause"}` | pause |
| `{"command": "next"}` | next song |
| `{"command": "previous"}` | previous song |
| `{"command": "play_media", "query": "daft punk"}` | play what the words name, as they were heard |

Publish the new state as soon as a command has acted. Nidoo waits for it to
confirm the command, and the assistant reads the song from it:

| Command | Confirmed by |
|---|---|
| `play` | `state` is `playing` |
| `pause` | `state` is `paused` |
| `next`, `previous` | a new `title`, or a `position_at` that changed (null while loading counts) |
| `play_media` | `state` is `playing` with a new `title` or a changed `position_at` |

Any other report (someone pausing from another app while a `next` waits)
does not confirm it.
