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

## Alarm pushes

The native alarm sends on its own channel, `alarm`, from source `alarm`. A triggered alarm is a critical alert with a **Disarm** button: tapping it sends the reply on `nidoo/push/responses`, and the hub disarms the alarm. The hub listens on the reply topic at all times. Arming and disarming send an info push that names the mode and who did it, with any sensor left open at arm. See [Alarm](alarm.md#phone-push).
