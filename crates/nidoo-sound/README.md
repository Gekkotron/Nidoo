# nidoo-sound

A Sendspin-compatible server for Nidoo's sound satellites. It plays one raw
PCM input on every approved satellite in sync and exposes each satellite's
volume, mute and approval to Nidoo through Home Assistant MQTT discovery.
User guide: `docs/users/sound.md`; satellites: `satellite/README.md`.

```sh
nidoo-sound run [--config nidoo-sound.toml]
```

## Configuration

Defaults, then the hub's broker, then the TOML file (`nidoo-sound.toml` if
present, or `--config`), then environment variables (`__` nests a section).
See `nidoo-sound.example.toml`.

The hub's broker is the `mqtt` section of `<data_dir>/hub.json`, the file the
hub writes when its broker is set up: host, port, user and password (not the
client id). With the Docker Compose install both services share `data/` and
the host network, so nothing needs setting. The file is read at startup:
restart the service after changing the hub's broker. A hub that takes its
broker from `NIDOO_MQTT__*` variables instead does not write them there; set
the same values as `NIDOO_SOUND_MQTT__*`.

| Key | Environment | Default | Meaning |
|---|---|---|---|
| `input_port` | `NIDOO_SOUND_INPUT_PORT` | `4953` | TCP port of the PCM input (s16le, 48 kHz, stereo) |
| `listen_port` | `NIDOO_SOUND_LISTEN_PORT` | `8927` | players that connect themselves open `ws://<host>:<port>/sendspin`; `0` turns it off |
| `data_dir` | `NIDOO_SOUND_DATA_DIR` | `./data` | holds `sound/identity.key` and `sound/players.json` |
| `lead_ms` | `NIDOO_SOUND_LEAD_MS` | `500` | how far ahead audio is timestamped; raised to cover the players' `min_buffer_ms + output_delay_ms` |
| `input_buffer_ms` | `NIDOO_SOUND_INPUT_BUFFER_MS` | `250` | phone audio gathered before playback starts, so Wi-Fi stalls up to about this long play through without gaps; adds this much delay |
| `idle_stop_ms` | `NIDOO_SOUND_IDLE_STOP_MS` | `5000` | silence on a connected input that stops playback |
| `discovery_prefix` | `NIDOO_SOUND_DISCOVERY_PREFIX` | `homeassistant` | MQTT discovery prefix |
| `mqtt.host` | `NIDOO_SOUND_MQTT__HOST` | the hub's, else `localhost` | broker |
| `mqtt.port` | `NIDOO_SOUND_MQTT__PORT` | the hub's, else `1883` | |
| `mqtt.user` | `NIDOO_SOUND_MQTT__USER` | the hub's, else none | |
| `mqtt.password` | `NIDOO_SOUND_MQTT__PASSWORD` | the hub's, else none | |
| `mqtt.client_id` | `NIDOO_SOUND_MQTT__CLIENT_ID` | `nidoo-sound` | |
| `hub_url` | `NIDOO_SOUND_HUB_URL` | the hub's `http.bind` from `hub.json`, else `http://127.0.0.1:8081` | where spoken sentences are fetched (`/api/v1/assist/speech/<id>.wav`) |

`sound/identity.key` is the server's Sendspin identity (a Curve25519 key):
keep it with your backups.

## MQTT

Per satellite (`<sid>` = first 12 hex characters of SHA-256 of its Sendspin
`client_id`), retained:

| Entity | Discovery | State | Command |
|---|---|---|---|
| Volume (number, 0–100) | `homeassistant/number/nidoo_sound_<sid>/volume/config` | `nidoo-sound/<sid>/volume` | `nidoo-sound/<sid>/volume/set` |
| Mute (switch) | `…/switch/nidoo_sound_<sid>/mute/config` | `nidoo-sound/<sid>/mute` | `…/mute/set` (`ON`/`OFF`) |
| Allowed (switch) | `…/switch/nidoo_sound_<sid>/allowed/config` | `nidoo-sound/<sid>/allowed` | `…/allowed/set` |
| Connected (binary sensor, diagnostic) | `…/binary_sensor/nidoo_sound_<sid>/connected/config` | `nidoo-sound/<sid>/connected` | |
| Address (sensor, diagnostic) | `…/sensor/nidoo_sound_<sid>/address/config` | `nidoo-sound/<sid>/address` (IPv4, empty when not connected) | |

Availability: `nidoo-sound/status` (`online`, last will `offline`).

Speech: the hub publishes `nidoo-sound/speak` (not retained) as
`{"id", "url", "players": ["<sid>", …]}`. The service fetches `hub_url + url`
(a WAV, at most 60 s), and plays it on those players over the music, which is
turned down by 15 dB with a 150 ms fade while the sentence plays. One sentence
at a time; up to four wait, a fifth is dropped with a warning.

Hold: while someone talks to a satellite, the hub publishes `nidoo-sound/hold`
(not retained) as `{"id", "players": ["<sid>", …], "for_ms"}`. Those players get
silence instead of music (speech is still mixed in) until `for_ms` runs out (at
most 60 s) or a `for_ms: 0` releases that `id`; the music then fades back in over
150 ms. Holds are never saved, so a restart brings the music back at once.

Played: once a sentence's last sample plays on the satellites (on the shared
clock), the service publishes `nidoo-sound/spoken` (not retained) as
`{"id": "<clip id>", "played": true}`; a sentence that is dropped (no satellite,
queue full, fetch failed) is reported at once with `"played": false`.

## Protocol and known deviations

It speaks the Sendspin protocol **1.0.0-rc1** as implemented by ESPHome
2026.10.2 (sendspin-cpp 0.9.3): Noise `KKpsk2` with the published Sentinel
PSK (the satellites' unpaired access), the `player@v1` role, clock sync, one
group holding every player, player volume and mute. ESPHome and this server
are updated together. Players that list their commands in `client/hello` and
send only changed fields in later `client/state` messages (sendspin-js 5.0.0,
the browser player) work too.

Deliberate subset of the spec, which asks more of a full server:

- **PCM only** (48 kHz, stereo, 16 bit); the spec says servers must also
  produce FLAC.
- **Both connection directions, one advertised**: the server connects to
  players advertising `_sendspin._tcp` and accepts players on port 8927, path
  `/sendspin`, but does not advertise `_sendspin-server._tcp` itself (a
  player that both advertises and dials advertised servers would get two
  connections); a client-initiated player is given the URL.
- **No pairing**: unpaired access with the Sentinel PSK only, so no
  protection against a man in the middle on the LAN.
- No controller, metadata, artwork or visualizer roles; no `group/update`.

`LICENSE-SENDSPIN-SPEC.md` is the Sendspin specification's licence
(Community Specification License 1.0), kept in this crate's root for its
patent grant. "Sendspin" is a trademark of its owners; this crate is an
independent implementation.
