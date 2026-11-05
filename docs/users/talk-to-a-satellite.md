---
title: Talk to a satellite
sidebar_position: 21
description: Say "Hey Jarvis" and a request near a sound satellite; the house acts and answers out loud in the same room.
---

In a room with a [sound satellite](sound.md), say **"Hey Jarvis"**, then a
request in French, as you would type it in the [Assistant](assistant.md):

> Hey Jarvis… allume la lumière du salon.

The satellite's light comes on while it listens. The music in that room goes
quiet as soon as it hears the wake word and comes back once the house has
answered; the other rooms are not touched. The house does what you asked and
answers on the satellite you spoke to. When the answer is a question ("which
room?", "are you sure?"), the satellite listens again right after it, without
the wake word: just answer.

Each exchange shows in the Assistant history with the satellite's name and
room. A request is a real one, with the same rules and confirmations as
typing it.

A Mac can listen too: see [A Mac as a satellite](mac-satellite.md).

## Set it up

1. **Speech output.** The answers are spoken with the hub's voice: set up a
   French voice first (Settings → Assistant → Voice). Without one the house
   still acts, and the answer stays in the history.
2. **The model.** In Settings → Assistant → **Listening**, press
   **Download**. The hub fetches the recognition model (Parakeet, about
   490 MB, 670 MB on disk) and an end-of-phrase detector, checks every file,
   and loads them (about 1 GB of memory, a couple of seconds).
3. **The key.** Press **Create key** and copy the line it shows
   (`voice_key: "…"`) into the satellite's `secrets.yaml`, then flash the
   satellite again (see `satellite/README.md`). Every satellite uses the same
   key, so keep that line with your satellite secrets: the hub shows a key
   only once, when it is made. **Replace key** makes a new one; satellites
   with the old key are no longer heard until they are flashed with it.
4. **Allow the satellite** in Nidoo, as for music, and give it a room.

The satellite's light blinking twice after "Hey Jarvis" means the hub has not
greeted it: check the key, that the satellite is Allowed and connected, and
that the model shows **Ready**.

## What it understands

Everything the Assistant understands: lights, heating setpoints (inside the
safe range, with the usual confirmation) and temperatures. One request per
sentence works best. A request it does not understand gets the usual "not
understood" answer, spoken.

## Privacy

- The satellite sends its microphone to the hub **only** after the wake word,
  or right after the house asked you a question, and stops as soon as you
  have finished your sentence (8 seconds at most, 6 for an answer).
- The sound is transcribed in memory on the hub, on its own CPU, and thrown
  away: nothing is stored, nothing leaves your network.
- Every message between the satellites and the hub is signed with the shared
  key, so a device without it can neither speak to the hub nor send a
  satellite's microphone elsewhere. The messages are not encrypted: someone
  on your network could hear the audio of a request while it is sent.
- Anything sensitive for the house's security (the alarm, locks, doors) is
  never done from a satellite unless it is turned on for voice, because
  anyone in earshot, or a TV, can talk to it.

## Network and resources

- UDP **4960** on the hub (Speech input → *Hub UDP port*; a new port applies
  when the hub restarts) and UDP **4961** on each satellite. With the Compose
  install the hub uses the host's network, so nothing else needs opening.
- Recognition uses 4 CPU threads by default (*Recognition threads*); a short
  request takes about a tenth of a second on a small home server.
- *Microphone gain* (4 by default) boosts the quiet microphone before the end
  of the sentence is detected; *Wait before a follow-up* (300 ms) is the pause
  between the end of a spoken question and the satellite listening again.

The messages are described in the [voice protocol](../reference/voice-protocol.md).
