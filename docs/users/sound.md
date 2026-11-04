---
title: Sound satellites
sidebar_position: 20
description: Play music from your phone in every room, in sync, on small ESP32-S3 satellites, with each room's volume and mute in Nidoo.
---

Sound satellites are small ESP32-S3 boards, one per room, running stock
ESPHome. `nidoo-sound`, a Sendspin-compatible server that runs next to the
hub, takes the music from your phone and plays it on every satellite at the
same time: standing between two rooms you hear one sound, not an echo.

Each satellite shows up in Nidoo as a device with:

- **Volume** (0–100) and **Mute**, usable from dashboards, flows, the Android
  app and the assistants like any other control;
- **Allowed** (a configuration switch): a new satellite plays nothing until
  you switch it on;
- **Connected** (diagnostic): whether the satellite is online.

Volume, mute and Allowed are remembered: they survive a satellite reboot, a
`nidoo-sound` restart and a hub restart. Restarting or rebuilding the hub does
not interrupt the music.

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

It reads the broker from `NIDOO_SOUND_MQTT__HOST`, `NIDOO_SOUND_MQTT__USER`
and `NIDOO_SOUND_MQTT__PASSWORD` in `.env` (default `localhost:1883`, no
login). Use the same login as the hub; with the bundled broker it is
`mqtt_user` and `mqtt_password` in `data/bundled.json`. Every setting is
listed in `crates/nidoo-sound/README.md`.

If something else already listens on port 4953 (a Snapcast server from an
older setup, for example), stop it first. With a firewall on the host, allow
4953/TCP in from the phones, 8927/TCP in from players that connect themselves
and 5353/UDP (mDNS).

Check that it runs and found the satellites:

```sh
docker compose logs -f sound
```

Stop it with `docker compose stop sound`; `docker compose rm -sf sound` removes
the container (the satellites' settings stay in `data/sound/`).

## Add a satellite

1. Build and flash it: [`satellite/README.md`](https://github.com/Gekkotron/Nidoo/tree/main/satellite)
   has the parts, the wiring and the commands.
2. Within a few seconds it appears in Nidoo under its ESPHome name, with
   **Allowed** off.
3. Switch **Allowed** on. If music is playing, the satellite joins in sync.
4. Put the device in its room like any other device.

## Known limits

- Audio travels as uncompressed PCM (about 1.5 Mbit/s per satellite), fine on
  a home LAN.
- `nidoo-sound` connects to the satellites; satellites that connect to a
  server themselves (`_sendspin-server._tcp`) are not supported.
- Satellites accept the protocol's **unpaired access**: the connection is
  encrypted, but anything on your LAN that speaks Sendspin can play on them.
  Keep them on a trusted network.
- Only one music source, played in every room: no room selection yet.
