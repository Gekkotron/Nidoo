# Satellites and the sound service — spike notes

Date: 2026-11-04. Board: Seeed XIAO ESP32S3 (ESP32-S3 rev v0.2, 8 MB octal
PSRAM, 8 MB flash) with an INMP441 and a PCM5102A, ESPHome **2026.10.2**
(`sendspin/sendspin-cpp` **0.9.3**), against `nidoo-sound tone`.

## Gate (plan Task 5)

| Question | Answer |
|---|---|
| Volume and mute settable over Sendspin? | **Yes.** `supported_commands: ["volume", "mute"]`; both audible on the speaker. |
| Unpaired access accepted? | **Yes.** Sentinel PSK, `psk_category: "sn"`, `unpaired_access: true` in the YAML. |
| Board resets? | None seen while streaming (see "Stability"). |

No finding changes the spec's scope.

## Connection and discovery

- Server-initiated only: the board advertises `_sendspin._tcp` (instance =
  ESPHome device name), port **8928**, TXT `path=/sendspin`. Its WebSocket
  server only answers `GET /sendspin`; no Origin or subprotocol checks.
- The board speaks first (`client/init`), then the server sends
  `server/init` and Noise message 1 back to back, as in rc1.
- Suite: **ChaCha20-Poly1305 only** (`25519_ChaChaPoly_SHA256`) on every
  target in sendspin-cpp 0.9.3. The server still supports both suites.
- Noise message 1 payload must carry `psk_category`; message 2 payload is
  `{}`. Handshake, `client/hello`, `server/hello` and the first
  `server/activate` must complete within 30 s of the WebSocket accept.

## What the board sends

- `client/hello`: `name` = the ESPHome friendly name; `client_id` is not
  repeated (it is in `client/init`); `supported_roles: ["player@v1"]`;
  `player@v1_support.supported_formats` = pcm 48000 Hz 16 bit, **2 ch then
  1 ch**; `buffer_capacity: 621312` (about 2/3 of the YAML `buffer_size`,
  capped by the board's ring); `unpaired_access: {"enabled": true}`;
  `supported_pair_methods: {"pairing_psk": …}`.
- First `client/state` arrives **~3 s after activation**: sendspin-cpp holds
  it until one clock-sync burst (8 sequential `client/time`) has completed.
  The server must not send `stream/start`, audio or commands before it.
- `client/state.player`: `volume` (50 on a fresh board), `muted`,
  `output_delay_ms: 0`, `required_lead_time_ms: 150`, `min_buffer_ms: 500`,
  `supported_commands: ["volume", "mute"]`. No `format` field.
- After every `server/command` the board echoes a full `client/state` with
  the new value: Nidoo's "accepted" check can rely on it.
- `client/time`: bursts of 8 sequential requests every ~10 s. The board
  drops a connection that is silent for 60 s; time replies cover that.

## Identity

`client_id` comes from a keypair stored in NVS (`sendspin.client: Loaded
static keypair`). It stayed the same across power cycles, USB re-plugs and
four OTA updates in this spike: it is a stable key for prefs, MQTT
`unique_id`s and the room assignment (spec "Calls made at review" 3).

## sendspin-cpp 0.9.3 vs the rc1 spec

- `group/update` is optional for the board (rc1: server MUST send it after
  the first activate); `nidoo-sound` does not send it yet.
- `server/hello` must carry `name`.
- `client/hello` may arrive before the board has processed `server/hello`.
- Each server JSON message must stay under ~2 KB (the board's JSON arena).
- A `stream/start` while a stream is active is not a clean restart on the
  board: send `stream/end` first.
- Audio chunks more than 5 ms late are skipped whole; chunks over the
  player's buffer quota are dropped (logged `OVER_QUOTA`).

## Hardware findings

- **PCM5102A VIN must be on 5V.** On the XIAO's 3V3 pad the module's power
  LED lit but the DAC was silent (only faint clicks at I2S start/stop), with
  the whole software chain running normally. Moving VIN to 5V fixed it.
  Written into `satellite/README.md`.
- The S3's USB-Serial/JTAG port can drop off the host or change name after a
  re-plug (`/dev/cu.usbmodem2101` → `…101`); the firmware kept running on
  Wi-Fi each time. After flashing from download mode, unplug and replug: a
  reset from download mode can land back in download mode.
- Ping round trips to the idle board were 10–360 ms (Wi-Fi power save).
  sendspin-cpp asks for high-performance Wi-Fi on every `stream/start` and
  releases it at `stream/end`; while streaming the median ping was 6 ms.

## Stability

Soaks with `nidoo-sound tone` (lead 800 ms, volume 3); the sender and the
board were both on Wi-Fi.

- **No reset, no leak.** 10 min of playback with the microphone capturing:
  free heap flat at ~214 KB after start (245 KB at boot), largest block
  ~170 KB, PSRAM ~6.99 MB free.
- **"Lost sync" came from the board's TCP settings, now fixed in
  `satellite.yaml`.** The first 10-min soak logged 44 `Lost sync` (up to
  1.5 s). The sender kept pace and the board kept reading (its TCP window
  never closed), but segments to the board were lost and each loss cost a
  whole-window resend plus seconds of retransmit backoff: stalls of 1–14 s
  emptied the lead and the speaker stopped. Cause: ESPHome's `psram:` sets
  `CONFIG_SPIRAM_IGNORE_NOTFOUND`, which drops ESP-IDF's out-of-order TCP
  queue (`CONFIG_LWIP_TCP_OOSEQ_MAX_PBUFS`) from no limit to 4 segments.
  With `"0"` (no limit) in `sdkconfig_options`:

  | 5 min each, back to back | Lost sync | speaker stops | stalls ≥ 0.5 s | retransmitted |
  |---|---|---|---|---|
  | limit 4 (ESPHome default) | 24 | 8 | 12 (33.6 s) | 2.2 % |
  | no limit | 2 (5 ms, 15 ms) | 0 | 0 | 0.2 % |

  A 10-min soak with the fix and the microphone running: 0 `Lost sync`,
  0 speaker stops, 0.2 % retransmitted.
- The large `Lost sync` values (hundreds of ms) are knock-on: once the I2S
  speaker drains and stops, sendspin-cpp's playback estimate goes stale.
  The first small one is the real event.
- **Microphone and playback run together.** With `microphone.capture` at
  boot, the INMP441 delivered 640000 B every 10 s (16 kHz × 32 bit) for the
  whole soak, with sound-level peaks, while the speaker played.
