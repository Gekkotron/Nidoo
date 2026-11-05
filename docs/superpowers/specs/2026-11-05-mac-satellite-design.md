# Mac satellite: a menu-bar app that makes a Mac one of the house satellites

Date: 2026-11-05. Status: design, approved 2026-11-05. Builds on
[satellites + nidoo-sound](2026-11-04-satellite-sound-design.md),
[speech on the satellites](2026-11-04-satellite-speech-design.md) and
[speech in](2026-11-04-speech-in-design.md). The board it imitates is
`satellite/satellite.yaml`; the voice messages are in
`docs/reference/voice-protocol.md`.

## Intent

The owner wants to talk to the house from their Mac the same way they talk to
an ESP32 satellite: say "Hey Jarvis" and a request, the house acts and answers
out loud on the Mac. The Mac also plays the house music, in sync with the
other rooms. It should be something they use every day, not a test tool, and
easy to silence: a menu-bar icon with a microphone switch and Quit.

To the hub and `nidoo-sound`, the Mac is **just another satellite**: same
Sendspin player role, same signed UDP voice protocol, same Allow + room setup
in Nidoo. Nothing on the hub or in `nidoo-sound` is meant to change for it.

## Goals

1. **Music.** The Mac advertises itself as a Sendspin player
   (`_sendspin._tcp` over mDNS), `nidoo-sound` connects to it, and it plays
   the stream on the Mac's current output device, in sync with the other
   satellites. Spoken replies and announcements arrive already mixed into that
   stream (the hub's `nidoo-sound/speak` and `nidoo-sound/hold` work as for a
   board), so nothing extra is needed to hear them.
2. **Voice.** The app listens for "Hey Jarvis" on the Mac's microphone. On a
   detection it does what the board does (`satellite.yaml`,
   `on_wake_word_detected`): mutes its own playback at once, sends **Wake**,
   streams **Audio** (16 kHz mono s16le) until the hub's **Stop** or its own
   8 s limit (then **End**), and answers a hub **Listen** with a follow-up
   window (6 s) without the wake word. All replay rules and counters exactly
   as in the protocol reference. It talks only after a **Hello** and only to
   the address inside the Hello.
3. **Menu-bar icon** (no Dock icon, no window). The menu holds:
   - a status line (connected and room, waiting for the hub, not allowed yet,
     microphone access missing; exact wording is the builder's);
   - **Listening** — a checkbox. Off closes the microphone completely (no wake
     word, no follow-ups); music and spoken announcements keep playing. The
     choice survives a restart;
   - **Set voice key…** — reads the 64-hex `voice_key` from the clipboard
     (the line copied from the hub's **Create key**, with or without the
     `voice_key: "…"` wrapper), validates it and stores it;
   - **Start at login** — a checkbox;
   - **Quit** — the Mac leaves the house: no music, no voice.
4. **Icon states.**
   - **Gray** whenever the hub cannot be reached (the owner asked for this
     explicitly): no Sendspin connection, or no valid Hello for a while.
   - Normal when ready.
   - A distinct "listening" look while a voice window is open, like the
     board's LED.
   - A distinct look when **Listening** is off (e.g. a crossed-out mic).
5. **Identity.** A stable Sendspin client id and static keypair made on first
   launch and kept, so the Mac keeps its `nidoo_sound_<sid>` entities, its
   Allow and its room across restarts. The player name is the Mac's computer
   name.
6. **Volume and mute** come from Nidoo like any satellite (Sendspin
   commands), applied to the app's own playback, not the Mac's system volume.
7. **Docs.** A user page (next to `docs/users/talk-to-a-satellite.md` and the
   sound page) on building, installing and setting up the Mac satellite, and a
   pointer from `satellite/README.md`.

## Non-goals

- Windows, iOS, push-to-talk or a hotkey, a settings window.
- Working away from the home network: outside it the icon is simply gray.
- Signed or notarized distribution, an installer, auto-update. It is built
  from source on the Mac it runs on.
- Echo cancellation. The hub already holds the room's music during a voice
  window; wake-word misses while loud music plays are accepted for v1.
- Any change to the hub, `nidoo-sound` or the voice protocol. If one turns out
  to be unavoidable, stop and ask.

## Anti-goals (failure even if it works)

- The microphone is ever sent anywhere outside a voice window, or while
  **Listening** is off.
- The app hears itself: a spoken reply re-triggering "Hey Jarvis" or the
  follow-up window capturing the reply.
- The Mac shows up in Nidoo as a new satellite after each restart.
- The key is logged or stored readable by other users.
- Anything personal (the owner's rooms, hosts, names) in tracked files: the
  project is public and generic.

## Constraints

- Rust, in this repository, sharing code with what exists instead of a second
  implementation: the Sendspin messages and Noise layer live in
  `crates/nidoo-sound/src/proto/` (written for the **server** side; the
  player side, Noise responder included, is new work), the voice datagrams in
  `crates/nidoo/src/assist/stt/packet.rs` (with test vectors). How to share
  them (a small extracted crate, a feature, or depending on the existing
  crates) is the builder's call, but the app must not pull in the whole hub.
- The menu bar through the `tray-icon` crate (with whatever event loop it
  needs on macOS).
- A real `.app` bundle (`Nidoo Satellite.app`) with `LSUIElement` (no Dock
  icon) and `NSMicrophoneUsageDescription`, so macOS asks for the mic once.
  Ad-hoc signing is enough.
- The Linux CI must stay green: the macOS-only parts must not break
  `cargo build/test --workspace` on Linux (exclude the crate from default
  members, gate by target, or similar).
- Wake word: the `hey_jarvis` model family the board uses, or the closest
  open equivalent that runs on a Mac CPU (openWakeWord `hey_jarvis` ONNX, or
  sherpa-onnx, already a hub dependency). Spike it first and record the
  result; the detection threshold must be adjustable without a rebuild.
- Shared cargo target under `$DEV_SCRATCH`; no per-repo cargo config.
- Attribution "Gekkotron" in any byline or bundle metadata.

## Decided with the owner

- Menu-bar app, not a hidden background program.
- Both music (synchronized) and voice, with "Hey Jarvis".
- **Listening** toggle in the menu stops only the microphone; **Quit** stops
  everything.
- Gray icon when the hub cannot be reached.
- Setup like a board: Allow it and give it a room in Nidoo; key from the hub's
  **Create key**.

## My calls (not discussed; change them if wrong)

- **Order of work: the wake-word spike first.** If "Hey Jarvis" cannot be
  detected reliably on a Mac CPU, stop and report to the owner before
  building the Sendspin player or the menu bar; voice is the point of the
  app, so a music-only v1 is not assumed.

- Key and identity stored in `~/Library/Application Support/Nidoo
  Satellite/` with `0600` permissions (the board keeps its key in a
  file too). Keychain would be the alternative.
- Start at login through a LaunchAgent plist (or `SMAppService` if it works
  with ad-hoc signing); off by default.
- Playback on the system's default output device, following it when it
  changes (headphones plugged in).
- Gray after 15 s without a Hello (three missed greetings).
- The microphone is open only while **Listening** is on **and** the hub
  greets the app (not gray). Gray or Listening off → capture stopped, so the
  macOS orange mic dot goes away; the detector restarts on the next Hello.
  While open, the orange dot stays on: that is macOS, not a bug.
- No self-trigger gating beyond what the board does: the detector runs
  during the app's own music and announcements, like the board's. A reply
  re-opening a window is prevented by the protocol (the hub sends **Listen**
  only after the reply has played, plus a margin); a "Hey Jarvis" inside a
  song or announcement is rare and accepted.
- `nidoo-sound` connects to every advertised player (Allow is decided after
  the connection); the Noise handshake is in `proto/noise.rs`. macOS asks for
  Local Network access (and maybe the firewall) on first run and possibly
  after an ad-hoc-signed rebuild: the user docs say so.
- Several interfaces (VPN, Tailscale): stream to the address inside the
  Hello, as the protocol says.

## Left to the builder

- Crate name and layout, the bundle build script, icon artwork (template
  images so they follow light/dark menu bars).
- Audio I/O library (e.g. `cpal`), resampling the mic to 16 kHz.
- Sendspin buffering and clock sync on the Mac, within what `nidoo-sound`
  expects from a player.
- Status-line wording and the exact gray timeout logic.
