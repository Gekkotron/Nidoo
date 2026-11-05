# Music and Speech volume per satellite

Date: 2026-11-05. Status: draft, awaiting approval.

## Intent

Spoken replies, announcements and flow "Say" lines get lost under the music:
the owner can't hear the speech. Today a satellite has one volume, and it
scales the whole Sendspin stream, music and speech together; nothing levels
the speech against the music (the hub's TTS clip and `nidoo-sound`'s
`wav_to_stream_pcm` apply no gain).

The owner wants **two sliders per satellite, Music and Speech**, set once and
rarely touched afterwards.

## Goals

- Each satellite has a Music volume and a Speech volume, 0–100, independent.
- Speech at a higher setting than Music is clearly louder than the music,
  even at full scale on both sides of the mix.
- Works the same on the ESP32 satellite and the Mac satellite.
- Nothing changes for anyone until they move the new slider.

## Non-goals

- Automatic loudness normalization of TTS clips.
- Changing the −15 dB music duck under speech (`mixer.rs` `DUCK_GAIN`).
- A whole-house Speech level (per satellite only; see "Calls made").
- A Speech slider on the Home satellite card or the Android app.

## Anti-goals

- Music jumping louder or quieter by itself when Speech is moved.
- Satellites in one sentence drifting apart (speech chunks for a sentence's
  satellites must stay byte-identical when their settings are identical).
- Breaking the hub's satellite identity, which is the `…_volume` entity
  (`crates/nidoo/src/sound.rs:113,139,159`).

## Constraints (found in the code)

- The stream is PCM s16le, 48 kHz, stereo, no codec (`pacer.rs:16-27`).
- `nidoo-sound` sends about 5 s ahead; anything done in the samples is heard
  up to ~5 s after the change. A Sendspin volume command acts at once.
- Device curves differ today:
  - ESP32 (ESPHome 2026.10.2 `i2s_audio` speaker): gain in dB is linear in
    volume, from −49 dB at 1 % to 0 dB at 100 %; 0 is silence
    (`SOFTWARE_VOLUME_MIN_DB = -49`).
  - Mac satellite (`crates/nidoo-satellite/src/playout.rs` `fn gain`):
    amplitude `v²`.
- Speech can't be made louder in the samples than full scale, so a Speech
  setting above Music must raise the device volume itself.

## How it works

**Device volume = the louder of the two.** `nidoo-sound` sends each player
`max(music, speech)` as its Sendspin volume, and scales the other one down in
the samples by the dB gap on the shared curve:

    gap_db = 49 × (max − quieter) / 100      (quieter = 0 → silence)

- Speech louder than Music: the device plays at Speech; music samples are
  scaled by −gap_db everywhere, inside and outside speech windows (the
  −15 dB duck applies on top).
- Music louder than Speech: the device plays at Music; only the speech
  samples are scaled.
- Equal: nothing is scaled, exactly as today.

**One curve everywhere.** The Mac satellite's `gain` moves to the ESP32
curve (−49 dB … 0 dB linear in volume, 0 = silence), so one formula gives
the right ratio on both. This changes how loud the Mac is at a given volume
(louder at low settings); its tests move with it. The Mac satellite is not
on `main` yet (PR #168): the change goes wherever it lands first, and the
other PR is rebased onto it.

**Where in `nidoo-sound`:**
- `state.rs`: per-player `speech_volume: u8`. Old state files have none:
  it starts equal to that player's `volume` (serde default after load), so
  nothing changes on upgrade. A new player gets the same default as Music (30).
- `mqtt.rs`: a second number entity, **Speech volume**, 0–100 slider, object
  `speech`: unique id `nidoo_sound_<sid>_speech`, topics
  `nidoo-sound/<sid>/speech` and `…/speech/set`. Not `speech_volume`: the
  web and the hub find a satellite by the `_volume` suffix. The existing
  entity keeps its topic `volume` (it is the satellite's identity) and is
  shown as **Music volume**.
- `group.rs`: `Control::SetSpeechVolume`; on either change, save, then send
  `PlayerCmd::Volume(max)` and set `expect_volume = Some(max)` (the echo
  check compares against what was sent). Connect sends `max` too.
- Device-side volume changes (a report that isn't the expected echo): read
  as moving the louder setting to the reported value; the other moves by
  the same amount, clamped to 0–100, so the gap is kept. Both are saved and
  published.
- `music_for` (`group.rs`) applies the music gain per player after the base
  chunk is chosen (covers hold and fade-in), allocating only when the gain
  isn't 1. `mixer.rs` `mix_chunk` takes the music and speech gains as
  arguments; targets with the same pair share the result, as the speech-only
  branch already does.

**Hub and web:**
- Settings → Sound (`crates/nidoo-web/src/screens/sound_settings.rs`): each
  satellite row shows two sliders, **Music** and **Speech**, folding the new
  `_speech` entity by suffix like `_volume` (`:125`). The row's
  existing speech-bubble test button is how the owner sets Speech by ear.
- The registry (`crates/nidoo-core/src/registry/folding.rs`) folds a sound
  satellite's entities into its `…_volume` number, today only when the
  device has exactly one number. With two numbers the anchor becomes the
  number whose unique id ends in `_volume`, and the `…_speech` number folds
  into it too, so the satellite stays one card everywhere.
- The Home satellite card keeps one slider, Music.
- MCP needs no change: the new entity appears and `send_command` sets it.

**Docs:** `docs/users/sound.md` (volume mentions), `docs/users/web-ui.md:293`
(Settings → Sound row), `docs/reference/mqtt-topics.md` (add the
`speech` topic; add the missing `volume` row too).

**What is Speech.** Every hub speech clip (`nidoo-sound/speak`: Assist
replies, announcements, flow Say, the Settings → Sound test button) takes
the Speech setting. Everything else in the stream is Music, including the
hardware-spike test tone (`nidoo-sound` `main.rs`). The wake chimes are
played by the satellites themselves, outside the stream: the ESP32's follows
the device volume (the louder setting), the Mac's ignores volume, as today.

**Mute** is unchanged: its own Sendspin command and echo check; unmuting
needs no volume resend.

**Supported players.** Only the ESP32 satellite (ESPHome 2026.10.2) and the
Mac satellite are tuned; another Sendspin client with a different curve gets
the right order (Speech above Music) but not the exact gap. Say so in
`docs/users/sound.md`.

## Behaviour to expect (say it in the docs)

- Moving the slider that is currently the louder one acts at once (device
  volume). Moving the quieter one is heard after a few seconds (it is in the
  audio already sent ahead).
- Mute silences both.
- Speech at 0 is silent, like Music at 0: spoken replies are not heard on
  that satellite.

## Calls made (not asked)

- Per satellite, not whole-house: the owner didn't choose; it matches the
  existing per-satellite volume.
- The Mac satellite adopts the ESP32 curve rather than `nidoo-sound` knowing
  each player's curve.
- A device-side change keeps the gap between the two settings.
- Speech starts equal to Music on upgrade and for new players.
- Device-side change with both settings equal: both move. At 0 or 100 the
  moved pair clamps and the gap shrinks; it is not restored later.
- Branches: this work is based on `main`. The Mac curve change is a commit
  on `feat/mac-satellite-app` (PR #168), whose `playout.rs` tests change
  with it; nothing on `main` has the Mac satellite yet, so the order of the
  two merges doesn't matter.
- The Home card's one slider is the Music setting (the `_volume` entity),
  never the device volume. No Android code lives in this repo; the app sees
  the new entity like any other.
- No Speech slider on the Home card.

## Left to the builder

- Exact names of types and functions, test layout, rounding of `gap_db` to
  a linear gain.
- Whether the gain-1 fast path is a check on the factor or on the settings.
