# Hardware echo cancellation (fallback)

This is the plan B for the sound satellites. We first try software AEC
(ESP-SR `esp_aec` with `i2s_audio_duplex`). Use this note only if that is
not good enough.

## 1. When to switch

Switch when the software AEC spike fails this test:

- Music plays at normal volume.
- The speaker sits 1 to 2 m from the board.
- A person says "Hey Jarvis" at normal voice.
- After tuning the reference delay and mic gain, the wake word is still
  missed in most tries (roughly 3 in 10 or more).

Also switch if the echo tail stays long enough to break a request, or if
the board cannot hear a person talk over the music.

## 2. Options

Prices are rough retail figures from shop pages. They change often.

**Seeed ReSpeaker Lite (XMOS XU316).**
- 2 microphones. It has speaker outputs (a 3.5 mm jack and a speaker
  connector). The wiki does not name the output codec.
- Price: about $29 for the board. The version with a XIAO ESP32-S3 costs more.
- Replaces: the INMP441 microphone. Keep the PCM5102A or use the board's own output.
- Playback reference in hardware: not confirmed. The XMOS AEC needs the
  speaker signal. We did not find a wiring guide for the playback reference.
- Needs the I2S firmware on the XU316. Sample rate must match (16 kHz or 48 kHz build).

**Seeed ReSpeaker XVF3800 (Flex or Circular-4 versions).**
- 4 microphones. Runs AEC, beamforming and noise suppression. Works in I2S or USB mode.
- Price: about £38 to £53 for the array, about $65 for the version with a XIAO ESP32-S3.
- Replaces: the INMP441 (and only the mics). The PCM5102A stays.
- Playback reference in hardware: not confirmed. We did not find the I2S
  reference format in public pages. Check the Seeed datasheet first.

**Home Assistant Voice Preview Edition (XU316 + ESP32-S3).**
- Internal dual mic, speaker, ESP32-S3 with 8 MB PSRAM.
- Price: about $59.
- Replaces: the whole board.
- Status for music: a community report says the stock firmware does not
  cancel music. The XMOS chip treats the speaker sound as voice. The thread
  had no confirmed fix when we checked. Do not buy it for music AEC unless
  that is fixed.

**Codec boards with a hardware loopback reference (ES8311 and ES7210).**
- Waveshare ESP32-S3-AUDIO-Board: ES8311 speaker codec and ES7210 4-channel
  ADC. Price: about $16 to $18 (August 2025 article). Replaces the whole board.
- Espressif ESP32-S3-Korvo-2: ES7210 can capture the AEC echo reference.
  Price not found.
- The `intercom-api` project supports two hardware reference methods:
  - ES8311 stereo digital feedback. The codec sends its DAC output on one
    stereo channel and the mic on the other, in the same I2S frame.
  - ES7210 + ES8311 analog loopback. The DAC output goes into an ES7210
    input, and arrives in the same TDM frame as the mics.
- Also used there: a software ring buffer (any codec, but never exactly aligned).
- Replaces: the whole board. The reference is captured in hardware.

## 3. Key principle

AEC removes a signal only when it knows the exact signal the speaker plays.
This is the reference.

So the music must go through the AEC chip's output, or be looped back into
the chip. Two ways to do that:

- The chip is the DAC. The music is sent to the chip, the chip sends it to
  the speaker, and it also keeps a copy as the reference.
- A loopback. The DAC output is wired back into an input of the same chip
  (or into the codec's ADC). The reference then arrives in hardware, in the
  same clock frame as the mic.

If the speaker plays a signal that never went through the chip, the chip
cannot cancel it. A separate DAC (like the PCM5102A) with no loopback
will not work with a hardware AEC chip, unless the chip sees the same
samples on a second input.

## 4. What changes in satellite.yaml

High level only.

- **I2S wiring.** A codec board uses one duplex I2S bus for mic and speaker.
  That means one BCLK and one LRCLK shared by both. Our current two buses
  (`i2s_mic` and `i2s_dac`) become one. The pin substitutions change.
- **Parts blocks.** Remove the INMP441 `microphone:` block and the
  PCM5102A `speaker:` block when the whole board is replaced. Keep the
  mixer, the `sendspin` source, and the `micro_wake_word` setup. Point
  `micro_wake_word` at the processed mic.
- **Firmware on the chip.** An XMOS board needs its I2S firmware, built for
  the right sample rate. A codec board needs no extra firmware.
- **ESPHome components.** Use the vendor or community component for the codec
  (for example the `intercom-api` components `i2s_audio_duplex`, `esp_aec`
  or its hardware reference options). Check that it works with ESPHome 2026.10.2,
  our pinned version.
- **Unchanged.** Sendspin, the voice UDP path, the LED, the chime, and the
  voice key stay as they are.
- **Hardware pins.** Keep to the pin rules in `satellite/README.md`
  (avoid GPIO3, GPIO43 and GPIO44). Write a new wiring table for the new board.

## 5. Recommendation

| Option | Rough price | Replaces | Reference captured in hardware | Verdict |
|---|---|---|---|---|
| Codec board, ES8311 + ES7210 (Waveshare class) | about $16 to $18 | whole board | Yes (TDM or stereo loopback) | First choice. Cheap, and `intercom-api` already supports it |
| ReSpeaker Lite (XU316) | about $29 (board) | mic, maybe output | Not confirmed | Second choice. Confirm the playback reference path with Seeed first |
| ReSpeaker XVF3800 | about £38 to £53 | mic array only | Not confirmed | Mic-only upgrade. Only if the DAC path can stay as it is |
| Home Assistant Voice PE | about $59 | whole board | Not working for music (stock) | Not for this use |

## Sources

- Home Assistant forum, XU316 AEC not working on VPE:
  https://community.home-assistant.io/t/help-needed-xmos-xu316-aec-not-working-on-voice-preview-edition-vpe/975285
  (the page returned HTTP 403 when fetched. The search summary was used.)
- intercom-api (hardware and reference options):
  https://github.com/samuelthng/intercom-api
- Seeed ReSpeaker Lite FAQ:
  https://wiki.seeedstudio.com/respeaker_lite_faq/
- ReSpeaker Lite retail listings (Switch Science, The Pi Hut, Robot Shop,
  Core Electronics): https://thepihut.com/products/respeaker-lite-with-xiao-esp32s3 ,
  https://core-electronics.com.au/respeaker-lite-2-mic-array-voice-kit.html.plain.md
- ReSpeaker XVF3800 listings (The Pi Hut, Robot Shop):
  https://thepihut.com/products/respeaker-xmos-xvf3800-ai-powered-4-mic-array-for-clear-voice-even-in-noise ,
  https://www.robotshop.com/products/respeaker-flex-xvf3800-circular-4-microphone-array-w-xiao-esp32s3-for-robotics-embedded-applications
- Home Assistant Voice Preview Edition price (CNX Software):
  https://cnx-software.com/2024/12/20/59-voice-preview-edition-adds-an-offline-smart-speaker-to-your-home-assistant-server
- Waveshare ESP32-S3-AUDIO-Board (CNX Software review, August 2025):
  https://www.cnx-software.com/2025/08/30/esp32-s3-audio-board-smart-speaker-devkit-dual-mic-array-lcd-camera-rgb-leds/
- Waveshare wiki: https://docs.waveshare.com/ESP32-S3-AUDIO-Board
- Espressif ESP32-S3-Korvo-2 user guide:
  https://docs.espressif.com/projects/esp-adf/en/latest/multimedia-boards/dev-boards/user-guide-esp32-s3-korvo-2.html
- XMOS AEC tuning notes: https://www.xmos.com/documentation/XM-015450-TM/html/doc/rst/aec_tuning.html
