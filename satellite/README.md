# Sound satellites

Small ESP32-S3 boards, one per room, running stock [ESPHome](https://esphome.io).
They play the music streamed by `nidoo-sound` (a Sendspin-compatible server,
see `crates/nidoo-sound/`), all rooms in sync. The microphone is wired now and
used later, for the voice assistant.

`satellite.yaml` is the single configuration for every satellite. Per-board
values come from substitutions; Wi-Fi and the OTA password come from the
git-ignored `secrets.yaml`.

ESPHome is pinned to **2026.10.2**: its Sendspin component must speak the same
protocol version as `nidoo-sound`, so the two are updated together.

## Parts

- Seeed Studio **XIAO ESP32S3** (8 MB flash, 8 MB PSRAM). Not the XIAO
  ESP32C3: it has a single I2S peripheral and no PSRAM.
- **INMP441** I2S microphone.
- **PCM5102A** I2S DAC, line-level out to powered speakers or an amplifier.

## Wiring

Wire by the pad names printed on the XIAO (D0…D10). The GPIO column is what
`satellite.yaml` uses; change the substitutions if you wire differently.

| Part | Pin | XIAO pad | GPIO |
|---|---|---|---|
| INMP441 | WS | D0 | 1 |
| INMP441 | SCK | D1 | 2 |
| INMP441 | SD | D3 | 4 |
| INMP441 | L/R | GND | (left channel) |
| INMP441 | VDD / GND | 3V3 / GND | |
| PCM5102A | BCK | D8 | 7 |
| PCM5102A | LRCK | D9 | 8 |
| PCM5102A | DIN | D10 | 9 |
| PCM5102A | SCK | GND | (internal PLL from BCK) |
| PCM5102A | VIN / GND | 5V / GND | |

PCM5102A configuration pads: FLT low, DEMP low, XSMT high, FMT low.

Power the PCM5102A module from **5V**, not 3V3: most modules feed VIN
through an onboard 3.3 V regulator, so on 3V3 the power LED lights but the
DAC stays silent (at most a faint click when playback starts and stops).
Avoided: D2/GPIO3 (strapping pin), D6/D7 (GPIO43/44, UART0). D4/D5 are kept
free for a future I2C LED ring or sensor.

## First flash (USB)

You need [uv](https://docs.astral.sh/uv/) (`uvx` runs the pinned ESPHome
without installing it).

```sh
cd satellite
cp secrets.example.yaml secrets.yaml   # then fill in Wi-Fi, an OTA password and the voice_key
uvx esphome==2026.10.2 run satellite.yaml --device /dev/cu.usbmodem*   # macOS; /dev/ttyACM0 on Linux
```

If the upload says "No serial data received", put the board in download
mode: hold **B** (BOOT), press and release **R** (RESET), release **B**, and
run the command again. After flashing, **unplug and replug** the board without
touching the buttons: a reset from download mode can land back in download
mode instead of starting the firmware.

The boot log should show Wi-Fi connected, `PSRAM … Size: 8192 KB`, the
microphone and speaker set up, and `sendspin.client: Loaded static keypair;
client_id=…`. From another computer on the same network the satellite
appears as a Sendspin player:

```sh
dns-sd -B _sendspin._tcp local.          # macOS
avahi-browse -r _sendspin._tcp           # Linux
```

## Later updates (Wi-Fi)

```sh
uvx esphome==2026.10.2 run satellite.yaml --device nidoo-satellite.local
uvx esphome==2026.10.2 logs satellite.yaml --device nidoo-satellite.local
```

The logs include free heap, largest free block and free PSRAM once a minute;
they should stay steady over hours.

## Adding a satellite

Every board needs its own name. Either pass it on the command line:

```sh
uvx esphome==2026.10.2 run satellite.yaml --substitution name nidoo-satellite-2 --substitution friendly_name "Nidoo satellite 2"
```

or keep a small per-board file under `satellite/boards/` (git-ignored) that
includes the shared configuration:

```yaml
# satellite/boards/satellite-2.yaml
substitutions:
  name: nidoo-satellite-2
  friendly_name: Nidoo satellite 2
packages:
  satellite: !include ../satellite.yaml
```

A new satellite shows up in Nidoo as **not allowed**: it plays nothing until
you switch its **Allowed** control on. Set its room in Nidoo like any other
device.

## Security note

The satellites accept the Sendspin protocol's unpaired access, so any
Sendspin server on your network can play on them. Keep them on a trusted
network.

## Speech input

Say **"Hey Jarvis"** near a satellite, then a request in French: the user LED
lights while it listens, and the house answers on that satellite (see
[Talk to a satellite](../docs/users/talk-to-a-satellite.md)). The wake word
runs on the board (`micro_wake_word`); the microphone goes to the hub over
UDP only inside that window, signed with the shared `voice_key` from the
hub's Assistant → Speech settings → Speech input. The board listens on UDP
4961 and streams to the hub's port (4960 by default) at the address the hub
sends it; no hub address is written in the firmware. The LED blinking twice
after the wake word means the hub has not greeted this board yet: check the
key, that the satellite is Allowed, and that the hub has the model ready.

