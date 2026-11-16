---
title: Push notifications
sidebar_position: 8
description: How the hub sends alerts to the Android app through Firebase Cloud Messaging, the topics it uses and its settings.
---

The hub sends the house's push notifications to the Android app through Firebase Cloud Messaging. Without a Firebase key it keeps the alert history and sends nothing, with one warning at start. The hub starts either way.

## Turn it on

Open **Settings → Notifications** (or the Notifications card in the Services step of the setup wizard), choose the Firebase service-account key file (`.json`) and save. The page reads the key's project id and shows it as **Project: …**, and refuses a file that has no `project_id`. The key is stored in the hub's database and is write-only: once saved, the page shows only that one is set, and nothing is copied onto the host. Saving restarts the hub. To turn push off, clear the key (`"service_account": null` through the API).

Without a key the hub keeps the alert history and sends nothing, with one warning at start. A key that is not valid stops the save with an error. The key is never logged or returned by the API.

## Configuration

The key is the only field with a screen. The topics and limits keep these defaults, and change through `PUT /api/v1/settings/push` or the `set_settings` MCP tool (send the whole section back; see the [Settings reference](../reference/configuration.md#push)):

| Field | Default | Meaning |
| --- | --- | --- |
| `prefix` | `"nidoo/push"` | `<prefix>/notification/<source>`, `/tokens/<installId>`, `/responses`, `/history`. |
| `retry_interval` | `60` | Seconds between two passes of the retry queue. |
| `max_retries` | `10` | Attempts before a queued alert is given up. |
| `history_size` | `100` | Alerts kept in the history. |
| `history_days` | `7` | Days an alert stays in the history. |

## How it works

Producers publish an alert (a v1 or legacy payload) on `nidoo/push/notification/<source>`; the hub normalises it, adds it to its history and sends it through FCM to every app install registered on `nidoo/push/tokens/<installId>`. Sends that fail for a transient reason are retried from memory every `retry_interval` seconds, up to `max_retries` times; a dead token is cleared on the broker. Parts of the hub that raise alerts themselves hand them to the same path in process, without the broker. Camera alerts from Frigate are one such part; see [Cameras (Frigate)](cameras.md).

| Topic | Direction | Retained | Content |
| --- | --- | --- | --- |
| `nidoo/push/notification/<source>` | producer → hub | no | v1 or legacy payload |
| `nidoo/push/tokens/<installId>` | app → hub | yes | `{token, device_name, lang, updated_at}`; an empty payload unregisters |
| `nidoo/push/responses` | app / web → producer | no | `{id, action, install_id, ts}` |
| `nidoo/push/history` | hub → app, web | yes | JSON array of the normalised alerts, newest first |

After each (re)connect the hub waits for its subscription to be acknowledged, then 2 s more for the retained tokens, before it sends; alerts arriving meanwhile wait in the queue. The history is the retained `nidoo/push/history` itself: the hub adopts it at its first sync.

## Service outage pushes

Weather and the Solar forecast can each send a `system` alert when their outside service has been unreachable for 30 minutes ("Weather unreachable", `warning`), and one more when it answers again ("Weather reachable again", `info`). Each is a switch, off by default, in that service's settings. See [Weather](weather.md#tell-me-when-open-meteo-is-unreachable) and [Solar forecast](solar-forecast.md#tell-me-when-solcast-is-unreachable).

The hub's own services use the same alerts with a shorter wait: the broker, Zigbee2MQTT and the database each send the "unreachable" alert after 5 minutes without an answer, and the "reachable again" alert when they answer. The service is named in the alert (`service` is `broker`, `zigbee2mqtt` or `database`), and `minutes` is sent as a string for every service. Their switch is **Notify me** on [System health](system-health.md#notify-me).

## Hub restart pushes

When the hub starts after an unexpected stop (a crash, a power cut, a kill), it sends one `system` alert, "Hub restarted", `warning`, with the minutes it was down when that is known. Its localisation key is `hub.restarted_unexpectedly`, with the argument `minutes` as a string; the argument is left out when the downtime is unknown. A clean restart, such as `docker compose restart`, sends nothing. The switch is **Notify me** on [System health](system-health.md#notify-me), off by default.

## Sensor supervisor pushes

The [sensor supervisor](system-health.md#sensor-supervisor) sends on the `system` channel, from source `system`, with a time to live of 24 hours. An opened problem is `warning` and a recovered one is `info`.

| Alert | Title (English fallback) | Key |
| --- | --- | --- |
| Stuck sensor | "<name> looks stuck" | `supervisor.stuck` |
| Stopped reporting | "<name> stopped reporting" | `supervisor.silent` |
| Impossible value | "<name> reads an impossible value" | `supervisor.impossible` |
| Back to normal | "<name> is back to normal" | `supervisor.recovered` |
| Several sensors wrong | "N sensors look wrong" | `supervisor.group`, `state` `wrong` |
| Several back to normal | "N sensors are back to normal" | `supervisor.group`, `state` `recovered` |

The body gives the evidence, for example "Same value (15.0 °C) for 14 h, usually changes within 3 h. Check the sensor." A grouped body lists the first three names, then "and N more".

The keys `supervisor.stuck`, `supervisor.silent`, `supervisor.impossible` and `supervisor.recovered` take these arguments:

| Argument | Meaning |
| --- | --- |
| `name` | The sensor's name. |
| `check` | `stuck`, `silent` or `impossible`. |
| `value` | The value, to one decimal, when known. |
| `unit` | The unit, when known. |
| `hours` | The hours shown in the alert text. |
| `usual` | The usual hours between two changes. Only when the check used the usual gap. Absent for the offline trigger and for impossible values. |

`supervisor.group` takes `state` (`wrong` or `recovered`), `n` (the number of sensors), `names` (the first three, comma-separated) and `more` (how many names are left out).

A single alert carries the sensor's `entity_id`, so the app opens the sensor. An opened single alert has one button, **Mute this sensor** (action `mute`). Tapping it publishes `{"id": "<alert id>", "action": "mute"}` on `nidoo/push/responses`. The sensor is added to the ignored list and its problems close without a push. Group alerts have no button.

Alert ids are `supervisor-<check>-<entity_id>-<opened_at>` for an opened problem, `supervisor-recovered-<check>-<entity_id>-<opened_at>` for a recovered one, and `supervisor-group-<check or "problem" or "recovered">-<time>` for a group.

## Alarm pushes

The native alarm sends on its own channel, `alarm`, from source `alarm`. A triggered alarm is a critical alert with a **Disarm** button: tapping it sends the reply on `nidoo/push/responses`, and the hub disarms the alarm. The hub listens on the reply topic at all times. Arming and disarming send an info push that names the mode and who did it, with any sensor left open at arm. See [Alarm](alarm.md#phone-push).
