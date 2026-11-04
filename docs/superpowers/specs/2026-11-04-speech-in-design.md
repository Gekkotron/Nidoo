# Speech in: talk to a satellite, the house answers in the same room

Date: 2026-11-04. Status: design, awaiting approval. Builds on
[satellites + nidoo-sound](2026-11-04-satellite-sound-design.md) (music),
[speech on the satellites](2026-11-04-satellite-speech-design.md) (the hub
says a sentence in a room), [assist TTS](2026-11-03-assist-tts-design.md) and
the [French assistant](2026-11-02-assist-design.md). This is "project 3" of the
satellite plan; it replaces the ESPHome native API idea with the simple
stream proven in a spike on 2026-11-04.

## Intent

The owner wants to **talk to the house**: in any room with a satellite, say
"Hey Jarvis", then a French command; the house acts and answers out loud on
the satellite they spoke to. It must feel like talking to a person in the
room: the music goes quiet while they talk, the answer comes fast, and when
the house asks something back they just answer, without the wake word again.

## What the spike proved (2026-11-04, throwaway code, not in the repo)

- Board: stock ESPHome on the XIAO ESP32-S3 (INMP441 mic), the normal
  `satellite.yaml` plus `micro_wake_word` (`hey_jarvis`) and a small C++
  include that sends the mic as UDP datagrams (u32 sequence + 16 kHz mono
  s16le, at most 640 samples each) only during a window opened by the wake
  word. Music via Sendspin kept working.
- Listener: Rust, `sherpa-onnx` crate 2.0.1 (static prebuilt libs fetched by
  its build script), **Parakeet TDT 0.6B v3 int8** (NeMo transducer, CC-BY-4.0,
  ~490 MB archive from the sherpa-onnx `asr-models` release) and **Silero VAD**
  to find the end of the phrase.
- Owner's voice, 3/3: wake word detected (0.91–0.93), « Allume la lumière du
  salon. » transcribed exactly, the assistant acted.
- The hub's mini PC (AMD Ryzen 7 5825U, 16 GB, hub running): **~0.1 s** to transcribe a
  1.5 s command with 4 threads (RTF 0.07); 2 threads 0.12 s; 8 threads is
  slower than 4 (contends with the hub). Model load 1.9 s.
- Always listening without a wake word was unusable: every household
  conversation went to the assistant.
- Gotchas found:
  - `micro_wake_word` **stops itself** after a detection and releases the
    mic; it must be re-armed (`micro_wake_word.start`).
  - Restarting the I2S mic costs ~1 s of silence: start `microphone.capture`
    at boot and never stop it.
  - The INMP441 is quiet: `gain_factor: 4` for the wake word source and ×4
    before the VAD; `probability_cutoff: 0.85` for `hey_jarvis` (default 0.97
    missed most tries).
  - The board has no echo cancellation: the mic hears its own speaker.

## Goals

1. "Hey Jarvis" on any Allowed satellite opens a listening window; the board's
   LED shows it.
2. The music goes quiet on **every satellite of that room** from the wake word
   until the exchange is over, then comes back by itself. Other rooms are
   untouched.
3. The hub transcribes the phrase (Parakeet), sends the text to the
   assistant **for real** (same rules, safety gates and confirmations as
   typing it in the Assistant screen), and speaks the reply on the satellite
   the owner spoke to.
4. When the reply is a question (`question.pending`: a choice or a
   confirmation), the satellite listens again right after the reply has
   finished playing, without the wake word, so the owner just answers.
5. Each exchange appears in the Assistant history, with the satellite (and its
   room) as the client.
6. Fast: from the end of the phrase to the start of the spoken reply, aim
   under ~1 s on that mini PC (VAD silence + ~0.1 s ASR + assistant + TTS first
   audio + stream lead).

## Non-goals

- Echo cancellation or talking over the reply (barge-in).
- A custom wake word (« Nidoo »): later, its own project.
- Languages other than French on the input side; an LLM fallback.
- The ESPHome native API; the Android app; the browser mic.
- Snapping misheard names to the house's names: later, if real use shows it
  is needed (the owner can already mark a turn as wrongly understood).
- Keeping audio recordings (see anti-goals).

## Anti-goals (a failure even if it works)

- Mic audio leaves the board when no window is open (no wake word and no
  follow-up asked by the hub).
- The music stays quiet after an exchange (stuck), in any failure case: hub
  restart, nidoo-sound restart, board reboot, Wi-Fi loss.
- The house acts on its own spoken reply (the follow-up window must open only
  after the reply has finished playing).
- A device on the LAN that does not hold the key can make the hub act, or
  can make a board send its mic to it.
- Audio is stored on disk by default.
- Typing in the Assistant screen behaves differently than before.
- Any house-, room- or person-specific word, address or id in tracked files.

## How it works (decided)

### Roles

- **Board** (`satellite/satellite.yaml`, stock ESPHome 2026.10.2, plus one
  committed C++ include): wake word, LED, local mute, the audio stream, and a
  small control listener. Every value specific to a house (key, Wi-Fi) comes
  from `secrets.yaml`; no hub address is written in the firmware.
- **Hub** (`crates/nidoo`): owns speech input. It receives the audio,
  runs VAD + Parakeet, calls the assistant, asks nidoo-sound to hold the
  room's music and to speak the reply, and sends control messages to the
  board. It already owns the assistant, TTS and room resolution.
- **nidoo-sound**: a new **hold** (music silenced for a set of players, speech
  still mixed in), and it tells the hub when a sentence has finished playing.

### The stream (two ways, UDP, one hub port)

- **Hub → board, "hello"**: the hub greets each satellite it knows, every few
  seconds, at the board's address (nidoo-sound already discovers the boards
  over mDNS; how the hub learns their addresses is left to the builder, e.g.
  nidoo-sound publishing it with each satellite). The board streams to the
  address of the last valid hello. A board that has heard no hello does
  nothing on a wake word except its LED blink (no audio leaves).
- **Board → hub, audio**: after the wake word (or a "listen"), datagrams of
  16 kHz mono s16le, each carrying the satellite's short id (the same id as
  nidoo-sound's `nidoo_sound_<id>` device), a session number and a sequence
  number. A "wake" packet opens each session.
- **Hub → board, "stop"**: as soon as the VAD sees the end of the phrase, so
  the LED and the stream stop at once (not after a fixed time).
- **Hub → board, "listen"**: opens a follow-up window (no wake word), sent
  only once the reply has finished playing on that board. *(My call)*
  "Finished playing" is the instant the reply's last sample plays on the
  board, on the shared Sendspin clock: nidoo-sound timestamps every chunk, so
  it knows that instant (not "last chunk sent", which is ~0.5 s early because
  of the stream lead). nidoo-sound reports it to the hub, and the hub sends
  "listen" at that instant + 300 ms margin (setting). Echo of the reply in the
  room is then over; the hold keeps the music away.
- **Board-side safety**: a window closes by itself after 8 s even without a
  "stop"; a follow-up window after 6 s.
- **Key** *(scheme is my call)*: one shared secret, in each board's
  `secrets.yaml` and in the hub's settings.
  - Every datagram, both ways, audio included, carries a truncated
    HMAC-SHA256 tag (e.g. 8 bytes) under that key over its whole content. The
    board ignores hub messages without a valid tag; the hub ignores packets
    without one. So a device without the key can neither inject audio into an
    open window nor redirect the mic. (~25 audio packets/s; the S3 has SHA
    hardware.)
  - Replay: the board has no clock and its counters reset on reboot, so each
    hello carries a fresh hub nonce; the board's wake (and follow-up audio)
    packets echo the latest nonce plus a per-boot session counter; the hub
    accepts only its recent nonces and rising counters, and a board accepts
    only hub messages with a rising hub counter.
  - Byte layout is the builder's, within those rules.
- One UDP port on the hub (default 4960, configurable). Compose runs with
  host networking, so no Docker change is needed; the docs name the port.

### Music during an exchange

1. Wake word: the board mutes its own output at once (local, instant, so the
   first words are clean).
2. First valid audio of the session: the hub sends nidoo-sound a **hold** for
   every Allowed satellite in that room (room resolution as for speak: the
   room of each satellite's Volume entity).
3. "stop" (end of phrase): the board unmutes itself; the hold keeps the music
   away, so only the reply is heard.
4. The reply is spoken on the satellite the owner spoke to (the existing
   speak path). If it asks a question, the hold stays through the follow-up
   window and the next reply.
5. The hold ends when the exchange is over: after the last reply has played,
   or right away if nothing was heard (empty window) or nothing is spoken.
6. A hold always carries a timeout (e.g. 30 s, refreshed while the exchange
   lasts), so a hub that dies mid-exchange never leaves a room silent. A board
   that reboots comes back unmuted.
7. *(My call)* A satellite whose Volume entity has no room gets a hold on
   itself only.

### Recognition

- sherpa-onnx (`sherpa-onnx` crate) behind a cargo feature `stt`, default on
  like `tts`; offline transducer with Parakeet TDT 0.6B v3 int8, greedy search,
  **4 threads** by default (setting), loaded once at start (~1.9 s) and kept
  (~1 GB RAM).
- Silero VAD per session (threshold 0.5, end after ~0.5 s of silence; tune),
  input ×4 gain (setting) before the VAD, as in the spike.
- A leading "Jarvis" left over from the wake word is removed from the text.
- One recognizer shared by all satellites; phrases are transcribed one at a
  time (each takes ~0.1 s).
- Model files: downloaded by the hub from the pinned sherpa-onnx release URLs
  (Parakeet archive + `silero_vad.onnx`), checked against pinned SHA-256, into
  `data/assist/stt/`. Same download UX as the TTS models (progress, disk-space
  check, cancel). Never bundled.
- An empty transcript ends the exchange silently. Text the assistant does not
  understand gets its normal "not understood" reply, spoken.

### Assistant

- Each satellite is an assistant client (`client` = `satellite:<id>`), so the
  existing per-client pending question (60 s) carries the follow-up answer.
- Turns are real (`simulate: false`). The history shows the satellite's name
  and room as the source.
- *(My call)* Voice may do everything the assistant does today: lights,
  heating setpoints (inside the existing safety gate and confirmations) and
  reading temperatures. None of these is security-sensitive. **Rule for
  later intents**: anything security-sensitive (alarm, locks, garage or
  entrance doors, covers that open the house) is refused from a satellite
  unless that intent is explicitly enabled for voice, because a voice turn can
  come from anyone in earshot or from a TV.
- *(My call)* When speech output is not set up, or no satellite can speak, a
  voice turn still runs and acts; the reply is only in the history, and the
  hold ends at once.

### Web UI

- Assistant → Speech settings gets a **Speech input** section: model status
  and download, threads, gain, the shared key (generate / show / copy, with
  the line to paste into `secrets.yaml`), and the hub's UDP port.
- Satellite card: shows "listening" while a window is open (nice to have).
- Assistant history: the satellite's name instead of a browser client.

### Firmware and docs

- `satellite.yaml` gets the voice part; the spike's gotchas above are
  required, with comments saying why.
- `docs/users/` gets a "Talk to a satellite" page (wake word, LED, what it
  understands, privacy: audio only after the wake word, nothing stored, the
  key); `docs/reference/` documents the UDP messages and the new HTTP and MQTT
  pieces.

## Left to the builder

- Packet layouts and the authentication scheme (within the rules above).
- How the hub learns board addresses and room membership, reusing what
  nidoo-sound and the hub already know.
- The nidoo-sound hold message and the "sentence finished" event (topic names,
  payloads), and how they fit the existing mixer and group actor.
- VAD fine-tuning, exact window lengths, hello period, hold timeout.
- Module layout (`crates/nidoo/src/assist/stt/` mirroring `tts/` is the
  obvious one), settings storage, i18n wording (English and French), tests
  following each crate's patterns (fake clock and fake players in nidoo-sound;
  a WAV fixture through VAD + recognizer behind the feature).

## Constraints

- Public, generic repo: no house words, addresses or ids in tracked files.
- The hub, nidoo-sound and the boards restart independently; every failure
  path ends with the music back and the LED off.
- The Docker image must build with the `stt` feature (sherpa-onnx's build
  script downloads its prebuilt libraries at build time; this needs network
  during `docker build`, as the spike's Docker build did).
- CPU: 4 recognition threads by default, next to TTS's half-the-cores rule.
