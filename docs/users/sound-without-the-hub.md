---
title: Sound without the hub
sidebar_position: 20.5
description: Run nidoo-sound on its own to try multi-room music, in sync, with ESP32 speakers and browser tabs, and no Nidoo hub, broker or database.
---

`nidoo-sound` is the part of Nidoo that plays music in every room at once. It
runs on its own: no hub, no MQTT broker, no database. Use it to try
multi-room audio with a music app such as [Pinson](https://github.com/Gekkotron/Pinson),
then add the hub later if you want dashboards, rooms and voice.

What you get without the hub:

- one music input played on every speaker, in sync;
- speakers: ESP32-S3 boards running ESPHome, and browser tabs;
- approve, forget, and set each speaker's volume and mute over a small HTTP API
  (Pinson does it from its Speakers sheet).

What needs the hub: names and rooms given in Nidoo, dashboards and flows,
spoken answers and announcements, "Hey Jarvis".

## 1. Start nidoo-sound

Pick a Linux machine that stays on (a mini PC, for example), on the
same network as the speakers: speakers are found with mDNS, which does not
cross routers.

**With Docker** (the simplest):

```sh
git clone https://github.com/Gekkotron/Nidoo.git
cd Nidoo
echo 'NIDOO_SOUND_HTTP_BIND=0.0.0.0' >> .env   # only if the music app runs on another machine
docker compose up -d --build sound
```

This builds and starts `nidoo-sound` alone. The hub, broker and database in the
same `compose.yml` are not started. The first build takes several minutes.

**Without Docker**, from the same clone, with [Rust](https://rustup.rs):

```sh
NIDOO_SOUND_HTTP_BIND=0.0.0.0 cargo run --release -p nidoo-sound -- run
```

Settings and speaker approvals are kept in `./data/sound/` (`data/sound/` in the
clone with Docker). Keep that folder: it holds the server's identity key.

Check that it runs:

```sh
docker compose logs -f sound
```

The log says `MQTT off: no broker set` and, once a speaker shows up,
`satellite connected`. Every setting is in `crates/nidoo-sound/README.md`.

**Firewall:** allow in 4953/TCP (music in), 4954/TCP (HTTP API, from the music
app), 8927/TCP (browser speakers) and 5353/UDP (mDNS).

## 2. Add speakers

New speakers play nothing until you **allow** them (step 3), so a neighbour's
device on the same network can't join by itself.

**An ESP32 board.** A Seeed Studio XIAO ESP32S3 and a PCM5102A DAC, wired to
powered speakers or an amplifier. Flash the music-only configuration, which
needs no microphone and no key from the hub: see "Music only" in
[`satellite/README.md`](https://github.com/Gekkotron/Nidoo/tree/main/satellite).
Within a few seconds of joining Wi-Fi it is waiting for approval.

**A browser tab, no hardware.** In Pinson, open **Speakers** and turn on
**Also play on this device**. Two phones are enough to hear two rooms in sync.

## 3. Allow them

**From Pinson:** choose the **Nidoo** output with the address of the machine
running `nidoo-sound` (port 4953). Open **Speakers**: new speakers are listed
under **Waiting**. Press **Allow**; the speaker joins the music in sync. **✕**
forgets one.

**From a terminal** (replace `HOST`):

```sh
curl http://HOST:4954/players/waiting            # [{"id":"0123456789ab","name":"satellite-kitchen","connected":true}]
curl -X POST http://HOST:4954/players/0123456789ab/allow
curl -X DELETE http://HOST:4954/players/0123456789ab   # forget it
curl http://HOST:4954/players                    # the allowed speakers, with volume and mute
```

A forgotten speaker that is still on the network comes back under **Waiting**.
To keep one out for good, switch it off or don't allow it.

The HTTP API has no login. With `NIDOO_SOUND_HTTP_BIND=0.0.0.0`, anyone on your
network can allow a speaker or change a volume: keep it off networks you don't
trust.

## 4. Play music

Send the music to port **4953**: raw PCM, 16-bit little-endian, 48 kHz, stereo.
Pinson does it when its output is **Nidoo**. One sender at a time.

## Adding the hub later

Install the hub ([Install on Linux](install-on-linux.md)) in the same clone.
Once its broker is set up, restart the service
(`docker compose up -d --force-recreate sound`): it finds the broker in
`data/hub.json`, the speakers appear in **Settings → Assistant → Sound** with
their volumes and approvals kept, and everything in [Sound satellites](sound.md)
applies.
