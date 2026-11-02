# System health — design

Date: 2026-11-02. Status: awaiting approval.

## Why

Today nothing says how long the hub has been running, when it last stopped, or how loaded the server is. When the hub
restarts after a crash or a power cut, nobody notices unless something stops working. `nidoo-watchdog` only pushes
gateway and broker-ACL alerts; nothing about the hub or its server is stored or shown.

The user wants to see, in one place:

- the hub's **uptime**: since when it runs, every restart with how long it was down, and an **uptime percentage**;
- the **whole server's** CPU, RAM and disk, each with the **hub's own share** drawn as a second line on the same chart;
- the database size, the number of retained topics, devices and commands;
- whether the **broker, Zigbee2MQTT and the database** are up;
- a **push when the hub comes back after a crash or power cut**.

## Goals

- Each figure is an ordinary **sensor** on one hub device, so it is recorded in history, can be charted on a
  dashboard and used in a flow (for example "database size above N → notify"), like the `network_<id>` sensors of
  network presence.
- A new **Settings → System** page, laid out as **tiles** (layout B from the brainstorm): one tile per figure with its
  current value and a small trend line; tapping a tile opens its full chart; tapping the uptime card opens the
  restart list.
- A restart that followed a **clean stop** (update, rebuild, `docker compose restart`) is told apart from one that
  followed an **unexpected stop** (crash, kill, power cut).
- Opt-in pushes for the unexpected restart and for the broker / Z2M / database being unreachable.

## Non-goals

- No new screen in the Android app. The app receives the pushes and can read the sensors like any other entity.
- No per-process breakdown of the other containers (broker, database, Frigate): the server line and the hub line only.
- No automatic action on high load or a full disk. The user builds that with a flow on the sensors.
- No alerting thresholds on CPU, RAM, disk or temperature built into the feature (flows do that).

## Anti-goals (a failure even if it works)

- The health page **filling the disk** it is meant to watch: its own history must stay bounded (see Retention).
- A **clean rebuild reported as a crash**: the user rebuilds the hub often; a push after every rebuild would train
  them to ignore it.
- A status tile reading **"down" because the hub cannot measure** (no temperature sensor in a VM, `$SYS` refused by
  the broker ACL). Not measurable must read "unknown" or hide the tile, never "down" or 0.
- Measuring that **slows the hub** or blocks the ingest path.
- Real hosts, IPs, sizes or names of the user's house in tracked files, docs or screenshots (the project stays generic).

## What the user sees

### Settings → System (tiles)

Top: the **uptime card** — "Up 12 d 4 h", "since <date time>", "<pct> % up over 30 days", "<n> restarts". Tap → the
restart list.

Then a grid of tiles, each with the current value, a sub-line, and a small trend line (24 h) where it makes sense:

| Tile | Value | Sub-line | Trend lines |
|---|---|---|---|
| CPU | server CPU % | hub CPU % | server + hub |
| RAM | server RAM used % | hub RAM (MB) | server + hub |
| Disk | disk used % of the filesystem holding the hub's data | database size | server + database |
| Temperature | CPU temperature °C | — | server |
| Devices | device count | entity count | — |
| Retained | retained topics on the broker | — | — |
| Commands | commands sent today | — | — |
| Broker | up / down (red when down) | since <time> | — |
| Zigbee2MQTT | up / down | since <time> | — |
| Database | up / down | since <time> | — |

Tapping a chart tile opens its **full chart** with the 24 h / 7 d / 30 d ranges, server and hub as two lines (two
colours, with a key), reusing the look of the existing entity history view.

At the bottom: the **"Notify me" switch** (off by default), with a one-line explanation of what it sends.

### Restart list

Newest first, last 30 days: start time, how long the hub was down before it, and "clean stop" or "unexpected stop".
The uptime percentage is (window − sum of downtimes in the window) / window, where the window is the last 30 days, or
since the first recorded start when the log is younger than that. With no recorded restart yet it reads 100 %.
*(My call.)*

## Behaviour

### Sampling

- Every **60 s** the hub samples its figures and publishes the sensors. Sampling runs on its own task; it never holds
  the registry lock while reading `/proc`, the file system or the database.
- **Server CPU %**: busy share of all CPUs over the last interval. **Hub CPU %**: the hub process's share of the whole
  machine over the same interval (so the hub line is directly comparable to the server line).
- **Server RAM**: used (total − available) as % and bytes. **Hub RAM**: the hub process's resident memory.
- **Disk**: used % and bytes of the filesystem that holds the hub's data directory (`/data` in Docker, the bundled
  database lives there too). **Database size**: `pg_database_size(current_database())`.
- **Temperature**: the CPU temperature when the host exposes one; otherwise the sensor is not published and the tile is
  hidden.
- **Devices / entities**: the registry counts (as `Hub::counts`).
- **Retained topics**: Mosquitto's `$SYS/broker/retained messages/count`. The hub does not subscribe to `#` today, so
  this needs a new subscription to that one `$SYS` topic, and the ingest path must let it through. If the broker
  never sends it (another broker, ACL refuses `$SYS`), the sensor is not published and the tile reads "unknown".
- **A figure that was measured and then stops being measurable** (for example `$SYS` goes silent after a broker
  restart, the temperature file disappears) makes that one sensor **unavailable** through its own availability
  topic, like a network watch's `avail`, rather than keeping a stale value. *(My call.)*
- **Commands today**: every command the hub publishes to a device (`send_command`, flows, ambiances, group fan-out,
  climate engine), counted in memory, reset at local midnight. A restart loses today's count so far (accepted).

### Where the hub runs

The hub runs in Docker with `network_mode: host`, in its own PID namespace, with `./data` mounted at `/data`. Inside
the container, `/proc/stat` and `/proc/meminfo` show the **host's** totals, `/proc/self` the hub process, and the
filesystem of `/data` is the host disk holding the data. The design relies on that; the builder must check it on the
real Linux host. On macOS (development) the server figures may be unavailable or approximate; that must not break the
build or the tests.

### Service status

- **Broker**: the hub's own MQTT connection (connected / not). While the broker is down the sensor obviously cannot be
  published; the page reads the hub's in-memory view over HTTP, so the Broker tile still shows "down".
- **Zigbee2MQTT**: the bridge's `bridge/state` (`online` / `offline`) under the configured Z2M base topic.
- **Database**: the history pool's last query outcome (a failing sample or write = down, the next success = up).
- Each is a `connectivity` binary sensor, like network presence.

### Restarts and the clean / unexpected stop

- While running, the hub writes a **heartbeat** (current time) to a small file in its data directory every minute.
- On a clean shutdown it writes a **clean-stop marker** with the stop time. Today `main.rs` only waits for Ctrl-C
  (SIGINT, both entry points); `docker compose stop / restart / up --build` sends **SIGTERM**, so the hub must also
  handle SIGTERM and write the marker before anything else on the way out (well inside compose's 10 s grace
  period). Without it every rebuild would read as a crash, which is an anti-goal. *(My call, after review.)*
- At boot it reads the file:
  - marker present → **clean stop**; downtime = boot − stop time;
  - heartbeat only → **unexpected stop**; downtime = boot − last heartbeat (±1 min);
  - no file → first boot ever, nothing recorded.
- The start is appended to a **restart log** in the database (start time, downtime, clean or not). If the database is
  down at boot, the entry is kept in memory and written once it is reachable.
- A file in the data directory, not MQTT or the database, so that a broker or database outage cannot fake a crash.

### Pushes ("Notify me", off by default)

- **The switch** is stored like PR #140's `notify_unreachable`: a field in a `system` section of the `settings` table,
  mirrored in `settings-cache.json`, so both the switch and the push key are readable while the database is down.
  *(My call.)*
- **Unexpected restart**: one `system` push after boot: "The hub restarted after an unexpected stop (down X min)". A
  clean restart sends nothing. It goes out as soon as the hub can push, even if the database is still down (the
  restart-log row waits, the push does not). *(My call.)*
- **Broker, Zigbee2MQTT or database unreachable for 5 minutes** without a success in between: one "down" push; when it
  comes back, one "back" push, only if the down push went out. Shorter blips are silent both ways. This reuses the
  outage tracker from PR #140 (`outage.rs`) with a 5-minute threshold instead of 30.
- The pushes need the push key, stored in the database settings; the hub's settings cache in the data directory must
  keep the database-down push working.
- Texts go through the localisation keys like `service.unreachable` (English and French).

### Retention

The system sensors produce about one row per minute per sensor. Their recorded history is **kept 90 days**; older rows
of these sensors only are removed. The rest of the history is untouched (it has no retention policy today, only
compression). The restart log is kept 90 days too. 90 days is a storage cap; the page's chart ranges stop at 30 days,
and a dashboard chart of a system sensor can look back up to 90. `drop_chunks` cannot be used (chunks hold every
entity), so the cleanup is a row delete on these entities or a separate store (builder's choice, see below).

## Interfaces

- **Entities**: hub-owned sensors on one device ("Nidoo hub"), published through retained discovery under the hub's
  prefix, ids starting `system_`. Units and device classes set so the existing history and dashboard widgets chart them
  without special cases.
- **HTTP**: one endpoint returning the System page view (current values, statuses with "since", uptime figures, the
  restart list, the notify switch) and one to set the switch. The page polls it or follows the snapshot; charts use the
  existing history query on the sensor entities.
- **MCP**: a read tool returning the same view, so an assistant can answer "how is the hub doing".
- **Docs**: a new `docs/users/system-health.md` page and a line in `web-ui.md`; drop the TODOS item.

## Decided by the user (brainstorm, 2026-11-02)

- Uptime **and** restarts with an uptime percentage.
- CPU, RAM, disk of the **whole server**, with the **hub's share as a second line** on the chart.
- Database size, retained topics, devices, commands.
- Each figure a **sensor** (dashboard + flows) **and** a Settings page.
- **Layout B**: tiles with a trend line, tap for the full chart.
- **Broker, Z2M, database** up/down tiles, red when down.
- A **push when the hub comes back after a crash or power cut**.
- Accepted as proposed: 60 s sampling, 30-day charts, clean vs unexpected stop, CPU temperature, commands per day,
  one opt-in "Notify me" switch, 5-minute threshold for service pushes, 90-day retention for these sensors, no app
  screen.

## Left to the builder

- How to read the figures (hand-read `/proc` / `/sys` vs a crate such as `sysinfo`), as long as the hub line and the
  server line are measured over the same interval and macOS builds.
- The exact entity ids, units and the heartbeat file format.
- The 90-day cleanup mechanism (a periodic delete on these entity ids vs a separate table), keeping in mind
  `entity_state` chunks are compressed after 7 days.
- How the two-line chart is built (extend the history view or a dedicated chart).
- Whether to base the branch on PR #140 or wait for it to merge (it must be merged first either way: `outage.rs` and
  the notify-switch pattern only exist there). Migration number: the next free one on `main` at build time (0006 today).
