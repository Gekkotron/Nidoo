---
title: Cameras (Frigate)
sidebar_position: 9
description: Serve Frigate snapshots and clips to the browser through the hub, and turn Frigate detections into camera alerts.
---

Alert snapshots and clips are URLs on Frigate. The web UI shows them on an alert's page (the clip in a player, the snapshot as a picture). When they point at Frigate's login port (8971) the browser cannot load them, so give the hub the Frigate login and it serves them itself: media on that origin is rewritten to `/api/v1/media?url=…`, and the hub fetches it with Frigate's session cookie (logging in on demand, once more on a 401). Any other origin is refused. Leave the user and password empty for the unauthenticated port (5000).

Set it up in **Settings → Cameras** (or the Cameras card in the Services step of the setup wizard):

- **Frigate address**: the Frigate origin, for example `http://192.168.1.20:8971`. With it set, media on that origin is served by the hub. A blank address counts as unset, and a login written into the address is refused: use the two fields below.
- **User** and **Password**: the Frigate login. The password is write-only: once saved, the page shows only that one is set. Changing the address needs the password entered again, so nobody on the LAN can point the hub at their own machine and collect it.
- **Media URL for the phone**: the base URL the phone fetches snapshots and clips from; the Frigate address when empty. Use the unauthenticated port (5000), because the app sends no login.

**Test connection** checks the address and login before you save. Saving restarts the hub.

## The Frigate bridge

With **Alerts** on (and a Frigate address or a media URL) the hub reads `<topic_prefix>/events` (subscribed only then) and raises a `camera` alert for each tracked object that qualifies. The first qualifying message (`new`, or the `update` that enters a watched zone) rings with the snapshot; `end` with a clip re-sends the same alert id silently with the clip; an event that only qualifies at `end` gets one audible alert with its clip. The title is the camera, the body `Person detected`, `Person in Porch` or `Alice (person) detected`, and `loc` (`frigate.object`) lets the app write it in the phone's language.

The form has **Alerts** (the bridge switch), **Alert on** (the object labels, such as `person` or `car`; none selected means every label) and **Cameras** (none selected means every camera). The rest have no screen and keep these defaults unless you change them through `PUT /api/v1/settings/frigate` or the `set_settings` MCP tool (send the whole section back, see the [Settings reference](../reference/configuration.md#frigate)):

| Field | Default | Meaning |
| --- | --- | --- |
| `zones` | `[]` | Alert only once the object enters one of these zones. Empty: any. |
| `cooldown` | `0` | Seconds to ignore new events on a camera after an alert. |
| `severity` | `"warning"` | `info`, `warning` or `critical`. |
| `ttl` | `43200` | Time to live of the alerts, in seconds. |
| `topic_prefix` | `"frigate"` | Frigate's MQTT `topic_prefix`. |

The broker user needs read on `<topic_prefix>/events`. Frigate needs `snapshots.enabled: true`, and event clips retained (`record.enabled`) for the clip update.

The alerts are delivered as described in [Push notifications](push-notifications.md). Use a Frigate *viewer* account for `user` and `password`, not the admin; see [Security](security.md).

## Live view on dashboards

The dashboard **Camera** widget shows one to four Frigate cameras in one tile; its settings sheet picks the cameras and their order (**Add camera**, move up or down, **Remove camera**). The House overview also shows every camera in a **Cameras** section while **Live view** is on, one 16:9 tile each. The hub serves the frames, so they stay off until you turn **Live view** on in **Settings → Cameras**. Every device on the LAN can then read the live frames, and it needs a Frigate address.

- `GET /api/v1/cameras` lists Frigate's enabled cameras (read from Frigate's `/api/config`, which the hub never passes on, and kept for about a minute) and says whether live view is on.
- `GET /api/v1/cameras/{id}/latest.jpg?h=` serves one frame, `GET /api/v1/cameras/{id}/stream?h=` the MJPEG stream. `h` is the height in pixels, from 1 to 1080.
- `h` is 360 when left out. Both answer `403` while live view is off, `404` for a camera Frigate does not list and `502` when Frigate cannot be reached. They only ever fetch `/api/{id}/latest.jpg` and `/api/{id}` from the Frigate address, with the same login.

In the widget:

- One camera fills the tile. Two cameras stack on a 2×2 tile, three or four make a grid on it; on any other size the tile shows one camera at a time, with arrows, dots and a swipe to change.
- With live view on, each visible picture refreshes about once a second and carries a **Live** label. A camera Frigate does not list reads **Camera not found**, and one that does not answer reads **Camera unavailable**. Pictures stop refreshing while the tile is off screen or the tab is hidden.
- A badge with the alert text and how long ago it was sent appears on a camera that raised an alert in the last 24 hours. It opens that alert.
- Tap a picture to open it full screen. The viewer plays the MJPEG stream while live view is on, has arrows to step through the widget's cameras, and closes with its button, Escape or Back.

With live view off the widget shows each camera's latest alert snapshot with **Live view off** over it, or "No events yet" when the alert history holds none for that camera.
