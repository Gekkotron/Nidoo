---
title: System health
sidebar_position: 18
description: How the hub's own figures are shown in Settings → System, what each tile means, how a clean stop is told from a crash, and the restart push.
---

The hub measures itself and the machine it runs on, and keeps the figures for 90 days. **Settings → System** shows them as tiles, and each figure is also an ordinary sensor, so a dashboard can chart it and a flow can act on it, like any other entity.

The hub samples every minute. The machine figures are the whole server, and the hub's own share is shown next to them, so you can see whether the load comes from the hub or from something else on the host, such as the database or the broker.

## The page

The top of **Settings → System** is the **uptime card**: how long the hub has been up, since when, the share of the last 30 days it was up, and how many restarts it had. Tap it to open the [restart list](#restarts).

Under it, one tile per figure. Each shows its current value, a line of detail and, where it makes sense, a small trend over the last 24 hours:

| Tile | Shows | Detail line |
| --- | --- | --- |
| CPU | the whole server's processor use, in % | the hub's share |
| RAM | the server's memory in use, in % | the hub's memory, in MB |
| Disk | the used share of the disk that holds the hub's data | the database size |
| Temperature | the processor temperature, in °C | |
| Devices | the devices the hub knows | the number of entities |
| Retained | the messages the broker keeps for the retained topics | |
| Commands | the commands the hub sent today | |
| Broker | up or down (red when down) | since when |
| Zigbee2MQTT | up or down (red when down) | since when |
| Database | up or down (red when down) | since when |

Tap a CPU, RAM, Disk or Temperature tile to open its full chart. It has the 24 h, 7 d and 30 d ranges, with the server and the hub as two lines and a key, the same look as the [history](history.md) chart of an entity.

A tile reads **unknown** when the hub cannot measure it right now. This is not a fault. A machine that exposes no temperature sensor, such as many virtual machines, has no Temperature tile at all. A broker that does not report its retained count leaves the Retained tile unknown.

## Restarts

Tap the uptime card for the restart list. It covers the last 30 days, newest first. Each row has the time the hub started, how long it was down before that, and whether the stop before it was a **clean stop**, an **unexpected stop**, or the **first start** (the first time the hub ever ran on this data).

The same view is at [`GET /api/v1/system`](../reference/http-api.md#system-health), and an assistant reads it with the `system_status` tool (see [MCP tools](../reference/mcp-tools.md)).

The uptime percentage is the time the hub was up in the last 30 days, divided by that window. A hub younger than 30 days is measured from its first start, so it is not marked down for time before it existed. With no restart recorded it reads 100 %.

## Clean and unexpected stops

The hub writes a small file, `system-heartbeat.json`, in its data directory (`./data/` for the Docker install). While it runs it refreshes the file every minute. When it stops on purpose, it writes a note that the stop was clean, with the time.

At the next start the hub reads that file:

- **Clean stop**: the note is there. Stopping with `docker compose stop`, `docker compose restart` or `docker compose up -d --build` counts as clean, so a rebuild or an update does not look like a fault. A stop that arrives while the hub is still starting is clean too.
- **Unexpected stop**: there is no note, only the last heartbeat. A crash, a `kill -9`, a power cut or a host that froze all look like this. The downtime is counted from the last heartbeat, so it is accurate to about a minute.

Only an unexpected restart sends a push. A clean restart is recorded and shown in the list, and nothing is sent.

## Notify me

At the bottom of the page, **Notify me** is off by default. Turn it on to receive these pushes on the Android app (see [Push notifications](push-notifications.md)):

- **Hub restarted unexpectedly**: one push after the restart, with how long the hub was down when that is known. A clean restart sends nothing.
- **Broker, Zigbee2MQTT or database unreachable** for 5 minutes in a row: one "unreachable" push. When it answers again, one "reachable again" push follows. A shorter outage sends nothing, in either direction.

The switch is also `PUT /api/v1/system/notify` in the [HTTP API](../reference/http-api.md#system-health).

The restart push still goes out if the database is down at that moment, because the hub keeps the restart record until the database is back.

## Sensor supervisor

The sensor supervisor checks the numeric sensors of the house once an hour and tells you when one looks wrong. It runs one hour after the hub starts, then every hour. It needs the database. Without one it does nothing, and its card says **Needs the database**.

Only sensors that report a number are checked. Secret entities are never checked.

### The three checks

- **Stuck**: the sensor still reports, but its value has not changed for much longer than usual. "Usual" is the longest gap between two value changes over the last 14 days, recomputed once a day. Gaps that overlap an earlier problem of the sensor are left out. The alert needs the time since the last change to pass both the usual gap times the sensitivity factor and 6 hours. It also needs a report from the sensor in the last hour, or in the usual gap if that is longer.
- **Stopped reporting**: the sensor has not reported for longer than the same limit, or it has been unavailable for more than 1 hour. The report clock starts when the hub starts or when the broker link comes back. This check pauses while the broker link is down or the Zigbee2MQTT bridge is offline, because that outage has its own push.
- **Impossible value**: the value is outside what the sensor can measure. Only the units in this table are checked. Other device classes, including power, and other units are not checked.

| Sensor | Unit | Accepted range |
| --- | --- | --- |
| Temperature | °C | -50 to 150 |
| Humidity, battery, moisture | % | 0 to 100 |
| Illuminance | any | 0 or more |
| Pressure | hPa | 800 to 1200 |
| Voltage | V | 0 to 300 |
| CO2 | ppm | 0 to 10 000 |

The impossible-value check works from the first day.

### Learning period

A sensor needs at least 7 days of history and 20 value changes in the 14 days before the stuck and stopped-reporting checks apply to it. The impossible-value check and the "unavailable for more than 1 hour" trigger apply from the first day.

A sensor that is flat by nature, such as a battery at 100 %, never gets a usual gap, so it never alerts as stuck.

### Sensitivity

The sensitivity sets the factor that the usual gap is multiplied by: **low** 3, **normal** 2, **high** 1.5. A higher sensitivity raises alerts sooner.

### Problems and pushes

A problem is one sensor with one check.

- When a problem opens, one warning push is sent. While it stays open, nothing more is sent.
- A problem closes only on clear evidence: the value changes, the sensor reports again or is back online, or the value is back in range. Then one "back to normal" push is sent. A check that can no longer run keeps its problem open.
- Open problems are stored. A hub restart does not send their push again and does not lose the "back to normal" push.
- When more than three problems open in one run, they are sent as one grouped push with no Mute button. The "back to normal" pushes of one run are grouped the same way.
- Removing a sensor from the house closes its problems without a push.

Each single warning push has a **Mute this sensor** button. Tapping it adds the sensor to the ignored list and closes its problems without a push. To un-mute, remove the sensor from the ignored list on the page.

The push texts and their keys are listed in [Push notifications](push-notifications.md#sensor-supervisor-pushes).

### The card

The card **Sensor supervisor** is on **Settings → System**, under **Notify me**. It has:

- the switches **Supervise sensors**, **Notify me** and **Tell me when it is back to normal**. Turning off **Supervise sensors** clears the open problems without a push. Turning **Notify me** back on does not push the problems already open;
- the sensitivity chips **Low**, **Normal** and **High**;
- the open problems, such as "Tank temperature — stuck for 14 h", each with a **Mute** button;
- a line "Learning: N sensors have less than 7 days of history";
- the ignored sensors, each with a remove button, and an **Add** button that opens a sensor picker.

Each change is saved at once. The hub does not restart.

The same settings are available over HTTP and MCP, see [HTTP API](../reference/http-api.md#system-health) and [MCP tools](../reference/mcp-tools.md#house).

## Retention

The system figures are recorded like any other sensor, about one reading a minute each. Their history and the restart log are kept **90 days**, and older rows of those sensors are removed once a day. The rest of the history is not touched: its own rules are the same as before, see [History](history.md#retention).

The charts stop at 30 days on this page. A dashboard chart of a system sensor can look back the full 90 days.

## The sensors

Each figure is an entity on a device called **Nidoo hub**, in the diagnostic section, with an id that starts with `system_`. They are listed here for flows and dashboards.

| Id | Name | Unit |
| --- | --- | --- |
| `system_cpu` | CPU | % |
| `system_cpu_hub` | Hub CPU | % |
| `system_ram_used` | RAM used | MB |
| `system_ram_hub` | Hub RAM | MB |
| `system_disk_used` | Disk used | GB |
| `system_db_size` | Database size | GB |
| `system_temperature` | Temperature | °C |
| `system_devices` | Devices | none (count) |
| `system_entities` | Entities | none (count) |
| `system_retained` | Retained messages | none (count) |
| `system_commands` | Commands today | none (count, back to 0 at local midnight) |
| `system_started` | Started | timestamp |
| `system_broker` | Broker | connectivity: on when up |
| `system_zigbee2mqtt` | Zigbee2MQTT | connectivity: on when up |
| `system_database` | Database | connectivity: on when up |
| `system_last_backup` | Last backup | timestamp of the last successful [backup](backup.md) |

A sensor that cannot be measured is marked unavailable rather than reading an old value. `system_last_backup` is unavailable until the first successful backup, and keeps its time when a later backup fails. The temperature sensor is not published at all when the machine has none.

In a flow, a service's connectivity sensor works like a [network](network-presence.md) sensor: a **When** card on `system_database` with the value off fires when the database goes down, and the value is `false` in JSON.

## In the Android app

The app receives the restart and outage pushes like any other. It shows the system sensors like any other entity, in the diagnostic section. There is no System screen in the app.
