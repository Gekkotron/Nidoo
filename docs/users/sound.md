---
title: Sound satellites
sidebar_position: 20
description: Play music from your phone in every room, in sync, on small ESP32-S3 satellites, with each room's volume and mute in Nidoo, and have the house say sentences in a room.
---

Sound satellites are small ESP32-S3 boards, one per room, running stock
ESPHome. `nidoo-sound`, a Sendspin-compatible server that runs next to the
hub, takes the music from your phone and plays it on every satellite at the
same time: standing between two rooms you hear one sound, not an echo.

Each satellite shows up in Nidoo as a device with:

- **Music volume** and **Speech volume** (both 0–100) and **Mute**, usable
  from dashboards, flows, the Android app and the assistants like any other
  control (see [Music and Speech volume](#music-and-speech-volume));
- **Allowed**: a new satellite plays nothing until you switch it on (in
  **Settings → Assistant → Sound**);
- **Connected** (diagnostic): whether the satellite is online.

The volumes, mute and Allowed are remembered: they survive a satellite reboot, a
`nidoo-sound` restart and a hub restart. Restarting or rebuilding the hub does
not interrupt the music.

## Music and Speech volume

A satellite has two volume settings, **Music** and **Speech**, so that a
spoken reply can be louder (or quieter) than the music. The satellite itself
has one volume: it plays at the louder of the two settings, and `nidoo-sound`
turns the other one down in the audio it sends.

- Moving the slider that is currently the louder one acts at once. Moving the
  quieter one is heard after a few seconds, because the audio already sent
  ahead of the satellite has to play out first.
- **Speech at 0 is silent**, like Music at 0: spoken replies are not heard on
  that satellite.
- **Mute** silences both.
- Speech starts equal to Music, for satellites that already exist when you
  upgrade and for new ones.
- A volume change made on the satellite itself moves both settings and keeps
  the gap between them.
- Only the ESP32 satellite (ESPHome 2026.10.2) and the [Mac
  satellite](mac-satellite.md) are tuned for this. Another Sendspin player
  keeps Speech above Music but not the exact gap.

To set Speech by ear, press the speech-bubble test button on the satellite's
card (or in **Settings → Assistant → Sound**) and move the **Speech** slider
until the sentence sounds right.

## How the music gets there

```
phone ──TCP 4953, raw PCM──▶ nidoo-sound ──WebSocket (Sendspin)──▶ satellites
                                 │  ▲
                 MQTT discovery  ▼  │  volume / mute / allowed
                              broker ◀──▶ Nidoo hub
```

The input is raw PCM: 16-bit little-endian, 48 kHz, stereo, sent to TCP port
**4953** (an Android casting app does this). One sender at a time: a new
connection replaces the current one without a break. When the phone stops
sending for 5 seconds, or closes the connection, the satellites go quiet.
The phone's audio arrives over Wi-Fi in bursts, so `nidoo-sound` gathers a
quarter of a second of it before playing (`input_buffer_ms`): stalls up to
that long play through without a click, longer ones are filled with silence.

## Ports

| Port | Direction | What |
|---|---|---|
| 4953/TCP | phone → `nidoo-sound` | music in |
| 8928/TCP | `nidoo-sound` → satellite | Sendspin WebSocket (the server connects to each satellite) |
| 4954/TCP | client → `nidoo-sound` | HTTP API: list the satellites, set volume and mute (`GET /players`, `PUT /players/{id}`; see the crate's README) |
| 8927/TCP | player → `nidoo-sound` | Sendspin WebSocket at `/sendspin`, for players that connect themselves |
| 5353/UDP | both | mDNS: satellites advertise `_sendspin._tcp` |
| 1883/TCP | `nidoo-sound` → broker | MQTT, same broker as the hub |

`nidoo-sound` and the satellites must be on the same LAN (mDNS does not cross
routers).

A Sendspin player that connects to the server itself instead of advertising
(a web player, for example) opens `ws://<host>:8927/sendspin`. It shows up in
Nidoo like a satellite, with **Allowed** off, and reconnects on its own when
`nidoo-sound` restarts. Set `listen_port = 0` to turn this endpoint off.

## Start it

With the Docker Compose install, the `sound` service is in `compose.yml` but
starts only when you name it:

```sh
docker compose up -d --build sound
```

It uses its own image (`nidoo-sound:local`) and the `sound` profile, so
neither `docker compose up -d --build hub` nor a whole-stack
`docker compose up -d --build` (what `tools/update.sh` runs) restarts it.
Update it yourself with the same command when you update ESPHome on the
satellites: the two go together.

With the Compose install there is nothing to set: the service reads the
hub's broker (host, port, user and password) and the hub's web address from
`data/hub.json`, the file the hub writes when its broker is set up, and the
two share `data/` and the host network. The file is read at startup, so
restart the service after changing the hub's broker:
`docker compose up -d --force-recreate sound`.

`NIDOO_SOUND_MQTT__HOST`, `NIDOO_SOUND_MQTT__USER`,
`NIDOO_SOUND_MQTT__PASSWORD` and `NIDOO_SOUND_HUB_URL` in `.env` still
override what `hub.json` says; set them when the hub takes its broker from
`NIDOO_MQTT__*` variables (it then does not write them to `hub.json`) or runs
on another machine. Every setting is listed in `crates/nidoo-sound/README.md`.

A broker with access rules accepts a connection with no login and then drops
its messages without an error: the service finds the satellites, but they
never show up in Nidoo, so you cannot switch them on. The log names the
broker user it uses (`broker_user=(none)` when there is none) and warns at
startup when no login is set.

If something else already listens on port 4953 (a Snapcast server from an
older setup, for example), stop it first. With a firewall on the host, allow
4953/TCP in from the phones, 4954/TCP in from clients of the HTTP API, 8927/TCP in
from players that connect themselves and 5353/UDP (mDNS).

Check that it runs and found the satellites:

```sh
docker compose logs -f sound
```

Stop it with `docker compose stop sound`; `docker compose rm -sf sound` removes
the container (the satellites' settings stay in `data/sound/`).

## Add a satellite

1. Build and flash it: [`satellite/README.md`](https://github.com/Gekkotron/Nidoo/tree/main/satellite)
   has the parts, the wiring and the commands.
2. Within a few seconds it appears in **Settings → Assistant → Sound** under its ESPHome
   name, marked "Waiting for approval", with **Allowed** off.
3. Switch **Allowed** on there, or with **Allow** on its card on Home. If
   music is playing, the satellite joins in sync. The same page has each
   satellite's **Mute**, **Music** and **Speech** volume; on Home, the
   satellite is one card with its music volume, mute and test button.
4. Pick its **Room** on the same page. The picker moves every entity of the
   satellite at once; the satellite then shows in that room and speaks there.
   Setting the room on its own page works too: the satellite's name opens its
   **Music volume** page, and its room is the satellite's room.

A Mac can be a satellite too, with no hardware: see
[A Mac as a satellite](mac-satellite.md).

## Remove a satellite

Every satellite in **Settings → Assistant → Sound** has a trash button, connected or not.
Press it, then **Remove** to confirm (or **Cancel**). `nidoo-sound` forgets
the satellite: its volumes, mute and Allowed, and the broker messages that made
it a device in Nidoo. Its card, controls and room go at once, with no restart.

Use it for a board you replaced, reflashed under a new name, or gave away. A
satellite that is still powered and on the network comes back a few seconds
later as a new one, "Waiting for approval", with no room, exactly as on its
first install. To keep a board out for good, switch it off or leave
**Allowed** off.

While `nidoo-sound` is stopped, removing is refused ("the sound service is
offline"): start it first.

## Speak in a room

The house can say a sentence on the satellites: a reminder in the kitchen, an
announcement in every room. The sentence is made by the hub's speech (set it
up under **Settings → Assistant → Voice**, also reached from the gear on the Assistant page, see
[Assistant](assistant.md#voice)); without it the satellites cannot speak.

- **The test button.** Each satellite's card on Home has a small speech
  bubble that says "Speaker test: <name>" on that satellite only.
- **Settings → Assistant → Sound.** Type a sentence in **Say something**, pick **All
  rooms** or a room (only rooms that hold a satellite are listed) and press
  **Send**. The page says how many satellites it was sent to, or why it could
  not be said.
- **A flow.** The **Speak** action (`speak` card) says its sentence in a room
  or every room. Its text takes the same values as a notification
  (`{weather_temperature}`), see [Flows](flows.md#values-in-a-notification).
- **An assistant.** The MCP tool `speak` does the same: "say dinner is ready
  in the kitchen".

Only satellites whose **Allowed** and **Connected** are both on speak, and only
while `nidoo-sound` runs. A muted
satellite counts but stays silent, as it does for music. When no satellite can
speak where the sentence was sent, it is refused rather than lost: "no
satellite can speak in this room", "no satellite can speak in the house", or,
for one satellite, "this satellite cannot speak now".

If music is playing, it is turned down while the sentence plays and comes back
after it. Sentences sent close together play one after the other, never on top
of each other; up to four can wait, a fifth is dropped, and so is one still
waiting 45 seconds after it was sent (the hub keeps its audio only a minute).
A satellite that drops off in the middle of a sentence does not stop the
others.

Over the network, the hub publishes the sentence's address on the broker topic
`nidoo-sound/speak` and `nidoo-sound` fetches the audio from the hub (its
address comes from `data/hub.json`, or `NIDOO_SOUND_HUB_URL`). The route and
the topic are in the [HTTP API](../reference/http-api.md#sound) and
[MQTT topics](../reference/mqtt-topics.md) references.

## Talk to a satellite

A satellite also listens: say "Hey Jarvis" and a request near it, and the
house answers on it while the room's music is held. See
[Talk to a satellite](talk-to-a-satellite.md).

## Supported features

- **Music in sync** in every room, from one source: the raw PCM a phone app
  sends to port 4953.
- **Satellites found on their own** over mDNS, and **players that connect
  themselves** (a web player) on port 8927.
- **Per satellite:** Allowed (approval), Music and Speech volume, Mute, Room, a Connected and
  an Address sensor, and **Remove** (see [Remove a satellite](#remove-a-satellite)).
  Volumes, mute and Allowed are remembered across restarts.
- **Speech** on one satellite, one room or every room, over the music, which
  is turned down while the sentence plays (see [Speak in a room](#speak-in-a-room)).
- **Talking to a satellite**: the room's music is held while it listens (see
  [Talk to a satellite](talk-to-a-satellite.md)).

Not supported yet: several sources or different music per room, compressed
audio (FLAC), pairing. They are ideas for later, listed in the repository's
`TODOS.md`; the details are in `crates/nidoo-sound/README.md`.

## Known limits

- Audio travels as uncompressed PCM (about 1.5 Mbit/s per satellite), fine on
  a home LAN.
- `nidoo-sound` connects to the satellites; satellites that connect to a
  server themselves (`_sendspin-server._tcp`) are not supported.
- Satellites accept the protocol's **unpaired access**: the connection is
  encrypted, but anything on your LAN that speaks Sendspin can play on them.
  Keep them on a trusted network.
- Only one music source, played in every room: no room selection yet.
