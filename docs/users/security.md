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

The hub can run the alarm itself, optional and off by default (see [Alarm](alarm.md)). It has **no codes or PINs**: the panel asks for none to arm or disarm, because the hub has no authentication to protect a code with. Anyone who can reach the hub on your LAN can disarm it. Anything that can publish on the broker can disarm it too: the panel's command topic `<alarm prefix>/panel/set` takes `DISARM` from any login allowed to publish there (the phone app's alarm buttons use it, so its login keeps that right), and a remote's button press is just a message on the broker. The hub cannot tell who sent such a message. The push **Disarm** button is checked: the hub acts only on the reply to a trigger alert it sent and that is still open, so a reply with a made-up id, or one to an alert already cleared, does nothing. Keep the broker private (no anonymous access, a login per client), keep the alarm's sirens and the hub on your own network, and read the caveat above before you rely on it.

## Settings and secrets

The setup and settings routes have no login either: anyone on the LAN can read or change the hub's settings, and can reach the wizard of a hub that has not finished setup. Secrets are write-only through the API (a read says only `{"set": true}`; the one exception is the [phone setup code](#the-phone-setup-code)), the MQTT, database and Frigate passwords must be entered again when their address (host and port) changes, and no log line or error carries one. They are stored in the hub's database (and in `data/settings-cache.json` and `data/hub.json`, both readable by the hub's user only). Back up and protect `./data/` and the database like any other secret store. See the [Settings reference](../reference/configuration.md#secrets).

### The phone setup code

One read is the exception: `GET /api/v1/setup/mobile-code`, the text of the QR code under **Settings → Mobile app**, carries the broker login, the Frigate login and the gateway key in clear, because the Android app reads them from it. The hub refuses it (`403`) to a browser page of another origin, so a web page open on the LAN cannot `fetch` it even though the rest of the API answers any origin: a browser that sends `Sec-Fetch-Site` gets it only from the hub's own page or the address bar, and an older one only when its `Origin` names the address the request came to.

That guard is for browsers only. A client that is not a browser (curl, a script, a device on the LAN) sends neither header and still gets the code: without authentication, anyone who can reach the hub can read the broker login. A page that uses DNS rebinding (a name of its own pointed at the hub's address) also looks like the hub's own page to the browser and still gets through; refusing it needs a list of the names the hub is reached by, planned with authentication. The broker login opens the whole broker, so treat the LAN as trusted, as above.

## What else to know

- `get_entity` and the `bindings` it returns hand out the raw discovery config; for a bridge other than Zigbee2MQTT that config could carry credentials in topics or templates.
- With a Frigate address set (**Settings → Cameras**) the hub also fetches alert media from Frigate with the configured login. It only ever requests an event's `snapshot.jpg`, `clip.mp4` or `thumbnail.jpg` on that origin and follows no redirect (plus, with live view on, a camera's `latest.jpg` and MJPEG stream, and `/api/config` for the camera list, which it never passes on), but any LAN client can request those, so give it a Frigate *viewer* account, not the admin. See [Cameras (Frigate)](cameras.md).
- A camera or image entity whose picture is a URL (`url_topic`, written by any broker login) makes the hub fetch that URL when the picture is asked for, with no login. The fetch refuses loopback, link-local (cloud metadata included), unspecified, multicast and broadcast addresses, unless the URL is on the Frigate address; it resolves the host once and connects to the address it checked, follows no redirect, and serves only an `image/*` reply (anything else answers `502`), under a sandbox policy so an SVG cannot run a script. Private addresses stay allowed so LAN cameras keep working; they include the hub host's Docker bridge networks (`172.17.0.0/16` and up) and any VPN range it routes, so a broker login can still make the hub request a page there (a blind GET: only an image comes back).
- Discovery templates (`value_template` and the like) come from retained broker messages, so any broker login can supply one. The hub bounds each render: a template longer than 16 KiB is refused, a render stops after 50 000 instructions, and output past 64 KiB is an error. A runaway template fails instead of tying up the hub.
- Flows are retained on the broker in clear, so the hub refuses a flow whose `command` node carries an alarm code. Arm and disarm from a flow only on a panel that needs none. See [Flows](flows.md).

## Sound

`nidoo-sound` has no login either. Its ports are limited so that a device on the LAN cannot take over music that is already playing:

- The music input (TCP 4953) takes raw PCM from anyone, because the casting app cannot send a key. While music plays, only the device that is playing can reconnect; another one waits until the music stops. When nothing plays, any device on the LAN can start music in every room.
- The HTTP API (4954) that sets volume and mute listens on the hub host only (`127.0.0.1`). Set `http_bind = "0.0.0.0"` to open it to the LAN, with no login.
- A Mac satellite trusts the first `nidoo-sound` that connects to it and refuses any other server after that (**Forget server** resets it). A connection that has not finished the handshake no longer cuts the music. See [A Mac as a satellite](mac-satellite.md).
- The ESP32 satellites use Sendspin's unpaired key: any device on the LAN can still connect to one as its server, play on it or set its volume. Keep them on your own network.

## The broker

The broker that comes with Nidoo refuses anonymous clients, and its login travels in clear on port 1883 (no TLS yet). The hub, Zigbee2MQTT, the satellites and nidoo-sound share the `nidoo` login. The phone app has its own, `nidoo-app`, because the setup QR carries it to every phone (see [Mobile app](mobile-app.md#the-apps-own-broker-login)).

The broker's ACL, `data/mosquitto/config/acl`, keeps every login's access as it was, except that `nidoo-app` cannot:

- publish or read alerts on `nidoo/push/notification/#`, so a leaked app login cannot push a fake alert to every phone;
- read or change the alarm settings, `nidoo/alarm/settings`;
- read or change the flow label catalog, `nidoo/flow_labels`;
- read the broker's own topics, `$SYS/#`.

What it does not cover yet: the app login can still add or replace flows on `nidoo/automation/#`, send push replies and register push tokens, and the shared `nidoo` login can do anything. These have their own fixes on the backlog. Until then the hub bounds the damage on the push topics: it registers at most 32 installs and rate-limits alerts per source (see [Push notifications](push-notifications.md#limits)).

`init-bundled` writes the ACL once and never overwrites it, and on an install from before it, adds the `nidoo-app` login and an `acl_file` line to `mosquitto.conf` (unless that file already names one). If you changed the push, alarm or flow label prefixes in the advanced settings, edit the ACL to match, then reload the broker with `docker compose kill -s HUP mosquitto`; `tools/update.sh` does that reload for you. With your own broker, nothing changes: the file is a sample you can copy from `crates/nidoo/assets/mosquitto.acl`. With the [ACL watchdog](watchdogs.md#broker-acl) on, each denial becomes an alert.
