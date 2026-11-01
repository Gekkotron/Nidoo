# Network presence by ping — design

Date: 2026-11-01. Status: awaiting approval.

## Why

A device can drop off the Wi-Fi without anyone noticing. The first case is the robot vacuum: it reaches the hub
through an MQTT bridge whose availability can keep saying `online` while the vacuum itself is gone, so you
only find out when it has not cleaned. The hub should be able to ping a host and say "connected" or
"disconnected", so that a flow can act on it (typically: notify me).

## Goals

- Watch any host by IP or hostname: a device the hub already knows (the vacuum, a TV) or one it does not (a NAS, a
  phone).
- Each watched host becomes an ordinary binary sensor with device class `connectivity`: usable in flows, on
  dashboards, in history and in the entity picker like any other entity.
- Switched on **per device**: nothing is pinged unless the user asked for it.
- Ping interval set **per host**.
- One lost reply never flips the state.

## Non-goals

- No notification built into the feature. The user builds it with a flow (trigger.state on the network entity →
  notify).
- No network scan or discovery of hosts.
- No latency charts or packet-loss statistics. The last reply time and round trip are shown on the sheet, nothing
  more is kept.
- No change to the existing `availability` of entities: the network entity sits next to it, it does not replace it.

## Anti-goals (a failure even if it works)

- A device shown as **disconnected because the hub cannot ping at all** (missing permission in Docker). That must
  read as "the hub cannot ping", never as "the device is gone", or the user's flows would fire falsely.
- A state that flaps on a single lost packet.
- Real hosts, IPs or room names of the user's house in tracked files (the project must stay generic).

## Behaviour

- Each host is pinged every **interval** (per host; default **30 s**, from **10 s** to **10 min**).
- It turns **disconnected after 3 missed pings in a row** (90 s at the default interval) and **connected on the
  first reply**. The 3 is fixed, not a setting.
- At hub start nothing is published until the first verdict (first reply, or the third miss): the last retained
  state stays, and the sheet shows "Waiting for the first reply" from the live state. Only a real change of the
  value is published.
- "The hub cannot ping" changes only the entity's **availability**, never its value. The flow engine already
  ignores availability-only reports and a first value seen after boot (`flows/engine.rs`, `state_changed`), so a
  restart or a ping permission failure never fires a flow. A test must pin this for the network entity.
- A hostname is resolved on every ping, so a host whose address changes keeps working. A name that does not resolve
  counts as a miss.
- If the hub cannot send pings at all (permission refused when opening the socket), every watch is **unavailable**
  (entity availability `offline`), not disconnected, and the sheet and the Settings page say "The hub cannot send
  pings" with a one-line hint (see "Docker" below).

## What the user sees

Mockups, captured from the live web UI with the new parts injected using the app's own components (Night Glass
and Ivoire), are in [`2026-11-01-network-presence/`](2026-11-01-network-presence/). The dashed outline in them
only marks what is new.

1. **Device sheet → "Network" panel** (`device-sheet-*.webp`), placed between the Settings panel and
   "Device & diagnostics":
   - **Watch on network** switch. Off pauses the watch (fields kept, state row "Paused"); the first switch-on on a
     device that never had a watch creates it.
   - **IP or hostname** (required when the switch is on) and **Ping every** (select).
   - Hint: "Disconnected after 3 missed pings in a row (90 s). Flows see it as "<device> network"." The seconds
     follow the chosen interval.
   - Rows: **State** (dot + Connected / Disconnected / Waiting for the first reply / The hub cannot send pings) and
     **Last reply** ("12 s ago · 4 ms").
   - Shown on devices that live on the IP network: not on Zigbee devices (a Zigbee node has no IP to ping), and not
     on the hub's own devices (groups, virtual entities, the watches' own devices) — my call.
2. **Settings list** (`settings-list-night.webp`): a new **Network presence** entry, with "N watched" as its
   status.
3. **Settings › Network presence** (`network-presence-*.webp`):
   - **Watched hosts**: one row per watch, linked to a device or not: state dot, name, a "device" tag for a watch
     on a known device, host, interval, room, state ("connected", "disconnected for 14 min"), **Edit**.
   - **Add host**: Name, IP or hostname, Ping every, Room (optional), **Add**. Hint: "A device the hub already knows
     is switched on from its own sheet."
   - A paused watch stays in the list with a grey dot and "paused"; the "N watched" status counts only the active
     ones (my call).
   - Edit opens the same fields in place, plus the on/off switch and **Remove** (the only way to delete a watch).

All text in English and French, like every screen.

## Model (decided)

- A **watch** is: `id` (`[a-z0-9_]{1,48}`), `name`, `host`, `interval_s`, `enabled`, optional `area`, optional
  `device` (the id of the hub device it belongs to).
- **Switching a watch off pauses it** (decided by the user): `enabled: false`, no more pings, the host and interval
  are kept, and the entity stays published with availability `offline`, so a flow that uses it stays valid.
  Switching it back on resumes with nothing to retype. Only **Remove** deletes a watch.
- Watches are **stored retained on the broker**, the same way groups keep their definition
  (`<prefix>/group/<id>/definition`): one retained definition per watch, so they survive restarts and need no
  database.
- Each watch is **published as a hub-owned entity** the way virtual entities are: a retained discovery config under
  the first discovery prefix (component `binary_sensor`, device class `connectivity`, unique id `network_<id>`) and
  a retained state topic. Because it lives under the discovery prefix, the Android app sees it as well.
- A watch with `device` set joins that device (same device identifiers in its discovery config), so it lists with
  the device's other entities and takes the device's room. **One watch per device**; its id is derived from the
  device id, and the API refuses a second watch on the same device (my call). The vacuum's bridge publishes stable
  `device.identifiers` (checked on the live hub). A device with no identifiers is linked by room only (my call). A watch without `device` gets its own small device named
  after the watch, in `area` if given.
- Removing a watch removes its definition and its entity (empty retained payloads), like `delete_virtual`.

## API surface (decided)

- REST: list, create or replace, delete watches (same shape as the groups and virtual-entity routes), plus the live
  state of each (state, last reply, round trip, whether the hub can ping).
- MCP: `list_network_watches`, `set_network_watch`, `delete_network_watch`, following the existing group tools.
- Docs: a user page (`docs/users/network-presence.md`), the REST reference and the MCP tool list.

## Docker (constraint)

The live hub runs in Docker on a Linux server. An unprivileged ICMP echo needs the kernel's
`net.ipv4.ping_group_range` to include the process's group; current Docker sets it wide open by default, older
setups may not. If the socket cannot be opened, the hub logs it once at start, marks every watch unavailable and
shows the message above. The docs page gives the one-line fix (the `sysctls` entry in `compose.yml`, or
`cap_add: NET_RAW`).

## Left to the builder

- The ping implementation (crate or hand-rolled ICMP), and whether to add a TCP-connect fallback when ICMP is
  impossible. If added, it must be visible on the sheet ("checked by TCP port N"), never silent.
- The exact retained topic names under the hub's prefix, following `groups/topics.rs`.
- How "devices that live on the IP network" is decided for showing the panel (e.g. not a Zigbee2MQTT device).
- Scheduling: one task per watch or one loop for all, with jitter so watches with the same interval do not fire
  together.
- How the live state (last reply, round trip) reaches the web UI: entity attributes or a dedicated endpoint.
