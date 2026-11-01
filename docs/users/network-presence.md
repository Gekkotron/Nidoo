---
title: Network presence
sidebar_position: 17
description: Let the hub ping a device or a host and use "is it on the network" in flows, the web UI and the app.
---

The hub can ping a device or any host on your network and publish the answer as an entity. A phone at home, a TV, a NAS or a vacuum then has an on/off "network" entity that flows can read and the web UI and the Android app can show.

Each watch is one host, pinged on its own interval. Its entity is `network_<id>`, a connectivity binary sensor: **on** while the host answers, **off** once it has stopped answering.

## Switch it on from a device

Open the device's sheet and find the **Network** panel, between **Settings** and **Device & diagnostics**.

1. Turn on **Watch on network**.
2. Type the device's **IP or hostname**, for example `192.168.1.42` or `nas.local`.
3. Pick **Ping every**: 10 s to 10 min, 30 s by default.

Each change saves as you make it. Once the first ping is answered the panel shows the **State** (connected or disconnected, and for how long) and the **Last reply** with its round-trip time.

The panel appears on devices the hub knows from outside Zigbee2MQTT, such as a vacuum or a TV. A Zigbee device, a group, a virtual entity or a network entity has no Network panel.

The watch is named "<device name> network", which is how a flow's entity picker lists it.

## Add a host

For something the hub has no device for, open **Settings › Network presence**. Under **Add host**, give a name, the IP or hostname, the interval and optionally a room, then press **Add**. Each watched host is a row you can **Edit**.

## Pause or remove

- **Pause** (turn **Watch on network** off): the hub stops pinging but keeps the host and the interval. The entity stays, shown as unavailable, and the State reads **paused**.
- **Remove** (on a host's edit form in Settings › Network presence): the watch and its entity are deleted, and what the broker retained for it is cleared. A flow that used the entity then has nothing to read.

## When it counts as disconnected

The hub declares a host **disconnected after 3 missed pings in a row**: at the default 30 s that is 90 s of silence. A single lost reply never flips it. The first reply makes it **connected** again at once.

Until the first verdict, nothing is published, and the State reads **waiting for the first reply**.

## Use it in a flow

A **When** card on `network_<id>` with the value `off` fires when the host disconnects:

```json
{
  "id": "nas_down", "name": "NAS offline", "enabled": true, "mode": "restart",
  "nodes": [
    { "id": "gone", "type": "trigger.state", "params": { "entity": "network_nas", "to": "off" } },
    { "id": "alert", "type": "notify", "params": { "title": "NAS is off the network", "severity": "warning" } }
  ],
  "edges": [
    { "from": "gone", "to": "alert" }
  ]
}
```

See [Flows](flows.md) for the cards. The same watches are available over REST and MCP; see the [HTTP API](../reference/http-api.md#network-presence) and the [MCP tools](../reference/mcp-tools.md#network-presence).

## The hub cannot send pings

If the State reads **the hub cannot send pings**, the hub's container is not allowed to open the ICMP socket it needs. Allow it on the host (not in the container) with:

```sh
sudo sysctl net.ipv4.ping_group_range="0 2147483647"
```

The hub runs with host networking, so a `sysctls` entry on the container does not apply: the setting has to be the host's. To keep it after a reboot, put the line `net.ipv4.ping_group_range = 0 2147483647` in a file under `/etc/sysctl.d/`. The hub retries by itself, so the State recovers without a restart.

## In the Android app

The Android app sees the entity like any other, a connectivity sensor in its room.
