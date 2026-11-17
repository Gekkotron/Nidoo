---
title: Security
sidebar_position: 13
description: What the hub does not protect in v1, what that means for your LAN and how to keep it off the internet.
---

There is no authentication yet, and that is deliberate for v1. The hub binds on `0.0.0.0` (the HTTP bind address, port 8081 by default) and serves the REST API, the WebSocket, the MCP endpoint and the web UI on that one listener, with no login on any of them. It answers any origin (CORS `*`) and its MCP endpoint accepts any `Host` header. Any device on the LAN, and any web page open in a browser on the LAN (through DNS rebinding), can therefore read the whole house and send commands, alarm arming and disarming included. Authentication and TLS arrive in a later phase.

:::danger Keep the hub off the internet
No port forwarding, and no exposure through a reverse proxy without authentication in front of it.
:::

## The native alarm

The hub can run the alarm itself, optional and off by default (see [Alarm](alarm.md)). It has **no codes or PINs**: the panel asks for none to arm or disarm, because the hub has no authentication to protect a code with. Anyone who can reach the hub on your LAN can disarm it, and the remote and push Disarm buttons work without any check. Keep the alarm's sirens and the hub on your own network, and read the caveat above before you rely on it.

## Settings and secrets

The setup and settings routes have no login either: anyone on the LAN can read or change the hub's settings, and can reach the wizard of a hub that has not finished setup. Secrets are write-only through the API (a read says only `{"set": true}`), the MQTT, database and Frigate passwords must be entered again when their address (host and port) changes, and no log line or error carries one. They are stored in the hub's database (and in `data/settings-cache.json` and `data/hub.json`, both readable by the hub's user only). Back up and protect `./data/` and the database like any other secret store. See the [Settings reference](../reference/configuration.md#secrets).

## What else to know

- `get_entity` and the `bindings` it returns hand out the raw discovery config; for a bridge other than Zigbee2MQTT that config could carry credentials in topics or templates.
- With a Frigate address set (**Settings → Cameras**) the hub also fetches alert media from Frigate with the configured login. It only ever requests an event's `snapshot.jpg`, `clip.mp4` or `thumbnail.jpg` on that origin and follows no redirect (plus, with live view on, a camera's `latest.jpg` and MJPEG stream, and `/api/config` for the camera list, which it never passes on), but any LAN client can request those, so give it a Frigate *viewer* account, not the admin. See [Cameras (Frigate)](cameras.md).
- Flows are retained on the broker in clear, so the hub refuses a flow whose `command` node carries an alarm code. Arm and disarm from a flow only on a panel that needs none. See [Flows](flows.md).

## The broker

The broker that comes with Nidoo refuses anonymous clients, and its login travels in clear on port 1883 (no TLS yet). The hub, Zigbee2MQTT, the satellites and nidoo-sound share the `nidoo` login. The phone app has its own, `nidoo-app`, because the setup QR carries it to every phone (see [Mobile app](mobile-app.md#the-apps-own-broker-login)).

The broker's ACL, `data/mosquitto/config/acl`, keeps every login's access as it was, except that `nidoo-app` cannot:

- publish or read alerts on `nidoo/push/notification/#`, so a leaked app login cannot push a fake alert to every phone;
- read or change the alarm settings, `nidoo/alarm/settings`;
- read or change the flow label catalog, `nidoo/flow_labels`;
- read the broker's own topics, `$SYS/#`.

What it does not cover yet: the app login can still add or replace flows on `nidoo/automation/#`, send push replies and register push tokens, and the shared `nidoo` login can do anything. These have their own fixes on the backlog.

`init-bundled` writes the ACL once and never overwrites it, and on an install from before it, adds the `nidoo-app` login and an `acl_file` line to `mosquitto.conf` (unless that file already names one). If you changed the push, alarm or flow label prefixes in the advanced settings, edit the ACL to match, then reload the broker with `docker compose kill -s HUP mosquitto`; `tools/update.sh` does that reload for you. With your own broker, nothing changes: the file is a sample you can copy from `crates/nidoo/assets/mosquitto.acl`. With the [ACL watchdog](watchdogs.md#broker-acl) on, each denial becomes an alert.
