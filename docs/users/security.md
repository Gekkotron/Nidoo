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
- Flows are retained on the broker in clear, so the hub refuses a flow whose `command` node carries an alarm code. Arm and disarm from a flow only on a panel that needs none. See [Flows](flows.md).
