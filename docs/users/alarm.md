---
title: Alarm
sidebar_position: 16
description: "The optional native alarm: modes, sensors, entry delays, open windows, sirens, remotes, presence, the push with Disarm, and moving off another controller."
---

:::caution Beta
The native alarm is new. It drives real sirens, so test it with the sirens switched off before you rely on it, and keep your current alarm in place until it has passed the checks below.
:::

The native alarm is an **optional** alarm that runs in the hub. It watches the door, window and motion sensors you pick, rings the sirens you list when one of them trips, and sends a critical push with a **Disarm** button to your phones. It also arms and disarms itself from your phones' presence, and from remote controls.

Nothing changes for a house that leaves it off. It is off by default, and a house that already has an alarm panel from another controller keeps seeing that panel on the [Security](security.md) page.

:::danger No codes, no authentication
The alarm has no code or PIN. The hub has no authentication yet (see [Security](security.md)), so anyone on your LAN who can reach the hub can disarm it. Keep the hub off the internet.
:::

## Turn it on

The alarm is switched on in its own settings, not in the service settings. Open **Settings → Alarm** in the [web UI](web-ui.md), or use the [MCP tools](../reference/mcp-tools.md#alarm) or the [HTTP API](../reference/http-api.md#alarm). The only other setting is the topic prefix, which you rarely change (see the [Settings reference](../reference/configuration.md#alarm)).

The page has an **Enable the alarm** switch and the sections **Sensors**, **Sirens**, **Remotes**, **Presence** and **Timing**. Nothing is applied until you press **Save**, and a toast says **Saved**. If the hub refuses the settings, the page shows the reason and the previous settings stay in force. A line at the top lists chosen entities the house no longer has.

Turning it off stops everything at once: no entity is published, nothing is watched, and any ringing siren stops. The settings are kept, so turning it back on restores them. Turning it off while the alarm is armed or ringing also resolves the open alarm alerts.

The panel itself is published to Home Assistant discovery as one `alarm_control_panel` entity, `oikos_alarm_house`, named **Alarm**. The web Security page shows it like any other panel.

## Modes

| Mode | What it watches | Typical use |
| --- | --- | --- |
| **Away** | Every sensor that ticks away | Nobody is home |
| **Night** | Doors and windows only | Going to bed |
| **Home** | The openings you tick for home | Staying in, with the outside doors armed |

Each sensor row has its own **away**, **home** and **night** ticks, so any house can set its own mix. The defaults are:

- A new **opening** (door or window) watches away and night, instant.
- A new **motion** sensor watches away only.
- For home, the usual choice is the outside doors only. Tick home on those, and leave the interior ones off.

The panel states are:

- **Disarmed**: nothing is watched. The SOS remote still rings the sirens.
- **Arming**: the exit delay after a manual arm. Nothing is watched yet, so you can leave.
- **Armed** (away, home or night): the mode's sensors are watched.
- **Pending** (shown as **Entry delay** on the settings page): a delayed door opened, and the entry delay is counting down.
- **Triggered**: the sirens are ringing.

## Instant and delayed openings

An opening is **instant** or **delayed**, per sensor, with two buttons on its row (the delayed one shows the entry delay, such as "delayed (30 s)"):

- An **instant** opening trips the alarm at once. Use it for windows and any door you do not come through while armed.
- A **delayed** opening starts the entry delay (**Entry delay (s)**, 30 s by default, 0 to 300). Disarm in time and nothing happens. Use it for the door you come in through.

While the entry delay runs, other delayed openings and motion are ignored, because the person walking in would trip them. An instant opening still trips the alarm at once.

Motion is always counted, never instant or delayed. The alarm trips when **two motion detections** (the default) fall within **120 seconds**. Any two motion sensors count, or the same sensor twice. A single stray detection never trips it. The count (**Motions to trigger**, 1 to 5) and the window (**Within (s)**, 10 to 900) are both editable on the settings page.

Only a change to *open* counts. A sensor that the hub reads again after a restart or a reconnect is not a new event, and neither is the first report of a sensor that was open before the hub started (see [Restarts](#restarts)).

## Windows and doors left open

The alarm arms even when something is open, so a window left ajar doesn't keep the whole house unarmed when the geofence fires. An open sensor is never ignored, though. When the exit delay ends, every watched opening that is still open is **watched for closing**, and the alarm reacts to its change to closed as it would to an opening. Once closed, the sensor is watched as usual, so opening it again is an event too. The push tells you which sensors are watched this way.

This matters most for **tilt-and-turn windows**. Left tilted, the contact reads open. To open the window fully, someone has to close it first and then turn it. That close is the first sign of tampering, so it trips the alarm.

:::caution The limit
A window or door that is ajar and can be pushed wider without closing (a plain casement or a door) shows no change on a single contact. In away mode, motion still covers it. A second contact, or a tilt sensor, closes the gap.
:::

A sensor that goes unavailable while the alarm is armed sends one warning push ("Alarm sensor offline"). It does not trip the alarm, and the warning clears when the sensor comes back.

## Sirens

In Settings › Alarm, each **Add** button (sensors, sirens, remotes, trackers) opens the [entity picker](web-ui.md#picking-an-entity) on the entities that list accepts. Tick several in one go, then **Done**; ticking a chosen one again takes it out.

A siren is one of two kinds:

- **Entity**: a `switch`, or a siren with a `power` control, that the hub already has. On means ring, off means stop. You can add an optional chirp length for it.
- **MQTT**: raw payloads published to a topic, for a siren that Zigbee2MQTT does not expose as a command entity. You give the topic, the **on** payload, the **off** payload and an optional **chirp** payload. The topic must not contain `+` or `#`.

For example, a generic MQTT siren might take `on` = `{"trigger": 6500000}` (a long trigger), `off` = `{"trigger": 100}` (a short pulse that replaces the long one) and `chirp` = `{"trigger": 100}`. Use the payloads your own device documents.

When the alarm trips, every siren rings. After the **siren period** (**Siren rings for (s)**, 180 s by default, 10 to 900) every siren stops and the panel goes back to the armed mode it was in. A new detection rings the sirens again for another full period, and so on, until someone disarms. Disarming stops every siren at once.

A siren that does not confirm within about 2 seconds is listed as failed in the trigger alert. The other sirens still ring. Test every siren before you rely on it: use the **Test chirp** button where the settings page offers it, and for an entity siren, switch it on and off from its own device.

## Chirp on arm

When the alarm enters an armed state, each siren that has a chirp sounds it once. This is the confirmation that the house is armed. Sirens without a chirp stay silent. The phones also get an *armed* push (see [Phone push](#phone-push)).

## Remotes

A remote is a `device_automation.*` or `event.*` entity that sends button presses. Each remote maps its signal names to an action: **away**, **home**, **night**, **disarm** or **SOS**. Known names are filled in when you add the remote:

| Signal | Action |
| --- | --- |
| `arm_all_zones` | away |
| `arm_day_zones` | home |
| `arm_night_zones` | night |
| `disarm` | disarm |
| `emergency`, `panic` | SOS |

You can add or change the names in the builder.

**SOS** trips the alarm from any state, including disarmed, so a panic button works even when the alarm is disarmed. It does nothing while the alarm is switched off in the settings. Disarming stops it.

A remote only shows up in the picker once Zigbee2MQTT has seen it press a button. If the picker is empty, press a button on the remote first.

### When a remote does not work

- **Press it once first.** A remote that has not sent anything yet has no entity.
- **Change the battery.** A low battery makes presses unreliable.
- **Re-pair the remote close to the coordinator.** A weak link drops presses.
- **Re-pair any device whose IEEE address reads `0x0000000000000000`.** Zigbee2MQTT has not learned its real address, so its entity cannot be trusted.
- **Enrol IAS remotes.** IAS remotes (the ones with the Heiman and IAS signal names) need enrolment, which pairing does again.

## Presence

The alarm can follow your phones. List the trackers (the `device_tracker` entities of your phones, for example the one the Nidoo Android app publishes), then choose what presence may do:

- **Arm away** (**Arm when everyone is away**) when **every** tracker has read *away* for 120 s (**Arm after (s)**, the default, 0 to 3600). The 120 s absorbs a geofence flapping at the edge. An unknown tracker blocks the auto-arm, and so does an offline one: a phone that is switched off or out of battery counts as unknown, whatever it last said, so it never arms the house by accident.
- **Disarm** (**Disarm when someone comes home**) when **any** tracker changes to *home*. Only a real homecoming counts: the phone must have read *away* before. A phone that is already home when someone arms the house by hand does not disarm it, and neither does one that drops offline and comes back while still at home.

Presence works on edges, not levels. It arms once for each time the house empties. After any disarm, from anywhere (the panel, a remote, the push button, or turning the alarm on), presence won't arm again until a tracker reads home (a phone back online at home counts) and the house empties again (or one is already home when you disarm, in which case it arms as soon as the house empties). So if you disarm to let someone in while you are out, the alarm stays disarmed until you come back and leave again.

A presence arm skips the exit delay, because you have already left. A manual arm waits out the exit delay (**Exit delay (s)**, 60 s by default, 0 to 300).

The hub reads the trackers' state at start-up too. A phone that came home while the hub was down disarms the alarm once the hub is back, and a house that emptied while it was down starts the 120 s timer.

## From a flow

A [flow](flows.md) can arm or disarm the alarm. In the editor's card library, search for the alarm: the THEN group offers **Disarm** and one **Arm** card per mode the panel offers (**Arm Alarm (away)**, and so on). As JSON, the `command` is `{"type": "disarm"}` or `{"type": "arm", "mode": "away"}` (`home`, `away` or `night`). A flow arm counts as a manual arm, so it waits out the exit delay.

## Phone push

The alarm sends its pushes on the `alarm` channel. Three kinds of push can arrive:

- **Triggered**: a critical alert, titled *Alarm*, that names what tripped it: the door or window, the motions and their rooms, or an SOS. It has a **Disarm** button. Tapping it disarms the alarm and stops the sirens. Each new trip sends a new alert, and the earlier one is cleared when the alarm is disarmed.
- **Armed and disarmed**: a plain alert with the normal sound, sent when the alarm becomes armed (at the end of the exit delay) and when it is disarmed. It names the mode and who did it: *Armed night*, *Armed away (everyone left)*, *Armed home by remote*, *Disarmed (someone came home)*. When something is open at arm, the same push names the sensors watched for closing: *Armed away. Office window open, watched for closing*. Disarming an alarm that is already disarmed sends nothing.
- **Sensor offline**: a warning when a watched sensor goes unavailable while armed.

See [Push notifications](push-notifications.md) for the channels and the Disarm action.

## Restarts

The alarm keeps its place across a restart of the hub. It stores its state, mode, source, timers, motion hits, the sensors watched for closing and the open alert, so a running entry delay keeps counting down, and a ringing siren rings for the time left.

After a restart the hub does not count sensor states that the broker replays as events. A door that is open on the broker when the hub starts does not trip the alarm by itself. The open-at-arm rule still applies to it.

A sensor that has not reported since the restart is the same case: Zigbee2MQTT does not retain device states by default, so a window that was already open reports *open* for the first time only after the restart. That first report is not an opening. If the alarm is armed, the window is watched for closing, as if it had been open at arm, but quietly: there is no open-sensors push.

## Moving off another controller

If another controller, such as Jeedom, runs your alarm today, move over in this order. Do not run both alarms with live sirens at the same time, because every trip would ring twice.

1. Enable the native alarm here, with the same sensors, sirens and modes as the other controller. Leave the other controller's scenarios that ring the sirens as they are for now.
2. Test it with the sirens disconnected or on a test siren: open each sensor, press each remote, check the pushes and the Disarm button, and check the arm chirp.
3. Switch over: disarm the other controller's alarm and remove its alarm logic, its scenarios that ring the sirens, and its discovery topic. Its `alarm_control_panel` config is retained on the broker, so publish an empty retained message to that discovery `config` topic too. Otherwise the old panel stays on the Security page next to the native one.

Then the Security page shows only `oikos_alarm_house`.
