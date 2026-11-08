---
title: A Mac as a satellite
sidebar_position: 22
description: Nidoo Satellite, a menu-bar app that makes a Mac one of the house satellites, with the music in sync and "Hey Jarvis".
---

**Nidoo Satellite** is a small menu-bar app that makes a Mac one of the house
satellites. To Nidoo it is like a [sound satellite](sound.md) board: it plays
the house music in sync with the other rooms, and you can
[talk to it](talk-to-a-satellite.md). Say **"Hey Jarvis"** and a request, and
the house acts and answers out loud on the Mac. It has no window and no Dock
icon, only an icon in the menu bar.

## Build and install

You need a Mac (macOS 13 or later) with [Rust](https://rustup.rs) installed.
The app is built from source on the Mac that runs it:

```sh
satellite/mac/build-app.sh --install
```

This builds `Nidoo Satellite.app`, signs it for this Mac only, and copies it
to `~/Applications`. Open it from there.

The app's Finder icon comes from `satellite/mac/icon/AppIcon.svg`. After
editing it, run `satellite/mac/icon/make-icns.sh` (it needs Google Chrome) to
remake `AppIcon.icns`.

On first launch, macOS asks:

- whether the app may find and talk to **devices on your local network**:
  allow it, or the music and the hub cannot reach it;
- for the **microphone**, the first time it starts listening: allow it, or
  "Hey Jarvis" cannot work.

The firewall may also ask whether to accept incoming connections: accept.
After a rebuild macOS can see a new app and ask again.

The first launch also downloads the wake-word model (about 4 MB) into the
app's folder.

## Set it up

1. **Allow it** in Nidoo, as for a board: within a few seconds the Mac appears
   in **Settings → Assistant → Sound** under its computer name, "Waiting for
   approval". Switch **Allowed** on and pick its **Room**. Music now plays on
   the Mac too, in sync.
2. **The voice key.** Copy the hub's `voice_key: "…"` line (from **Create
   key** in Settings → Assistant → Listening, the same line the boards use) and
   choose **Set voice key from the clipboard** in the app's menu. The bare
   64-character key works too. Anything else is refused and the old key kept.
   Within 5 seconds the icon stops being gray.
3. Say **"Hey Jarvis"**, then a request.

## Train Hey Jarvis

The wake word was taught on English voices, so it may miss a "Hey Jarvis"
said with another accent. If it rarely wakes for you, choose **Train Hey
Jarvis…** and follow the status line at the top of the menu:

1. "Say Hey Jarvis — 4 of 15": say it 15 times, the way you naturally do,
   with a short pause after each.
2. "Now talk normally — 30 s": talk about anything, or read something aloud,
   for 30 seconds. This teaches it what is *not* the wake word.
3. "Training…" for a few seconds, then "Hey Jarvis trained".

Meanwhile the app also has the Mac read some everyday French and English
sentences to itself with its built-in voices (nothing is played aloud).

The trained model then replaces the stock one for this Mac. Running it again
replaces it; **Cancel training**, quitting, or too few tries heard keep the
previous one. Only the trained model is kept, in the app's folder: your
recordings stay in memory, are dropped as soon as training ends, and never
leave the Mac.

## The menu

- The **status line**: waiting for the hub, allow this Mac in Nidoo, set the
  voice key, microphone access needed, connected, or the training steps.
- **Listening**: off closes the microphone completely (no wake word, no
  follow-up questions). Music and spoken announcements keep playing. The
  choice survives a restart.
- **Set voice key from the clipboard**, **Train Hey Jarvis…** (or **Cancel
  training** while it runs).
- **Start at login**: opens the app when you log in.
- **Quit Nidoo Satellite**: the Mac leaves the house, with no music and no
  voice, until you open it again.

## The icon

- **A microphone**: ready.
- **With sound waves**: listening to a request, like a board's light. A
  short two-note chime also rings when the wake word is heard (not when the
  house asks you a question), even while the music is muted.
- **Crossed out**: Listening is off, or the microphone cannot be used.
- **Faded (gray)**: the hub cannot be reached. Nothing is connected for music,
  or the hub has not greeted the Mac for 15 seconds (another network, Wi-Fi
  off, the Mac just woke up). It comes back by itself.

While the microphone is open (Listening on and the hub reachable), macOS
shows its orange microphone dot. It goes away when Listening is off or the
icon is gray. If the status says **Microphone access needed**, or the dot
never shows, allow the app in System Settings → Privacy & Security →
Microphone, then switch **Listening** off and on.

## Privacy

The same rules as a board:

- The microphone is sent to the hub **only** after the wake word, or right
  after the house asked you a question, and stops as soon as you have
  finished your sentence (8 seconds at most, 6 for an answer). The wake word
  itself is detected on the Mac.
- Every message to the hub is signed with the shared key; the key is kept in
  the app's folder, readable only by you, and never written to a log.
- The app's folder is `~/Library/Application Support/Nidoo Satellite`: the
  Mac's player identity (`player.key`), the settings, the wake-word model and
  the trained model. Keep `player.key`: without it the Mac shows up in Nidoo
  as a new satellite to allow again.

## Settings without a menu item

`settings.json` in the app's folder can hold `wake_threshold` (the stock wake
word, 0.5 by default) and `personal_threshold` (the trained one, 0.9). A
higher value wakes less often by mistake but needs a clearer "Hey Jarvis".
Quit and reopen the app after a change.
