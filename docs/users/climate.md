---
title: Climate
sidebar_position: 10
description: How the climate engine decides each zone's heating or cooling, what it learns from history, and how eco, proposals, overrides and hand changes work.
---

:::caution Beta
The climate engine is still being tested. Its rules, settings and decisions may change between releases, and it commands real heating and cooling: watch what it decides before you leave a zone on it.
:::

With climate on, the hub heats and cools by zone. The engine is off by default. Turn it on with the **Climate** card in the Services step of the setup wizard (**Settings → Run setup again** reopens it; nothing is wiped), where it is marked Beta. It is the card's **Enabled** switch. Everything else is edited on the Climate screen, over REST and over MCP.

The one low-level field, the topic `prefix` (`nidoo/climate` by default), has no screen: change it with `PUT /api/v1/settings/climate` or the `set_settings` MCP tool, which restarts the hub (see the [Settings reference](../reference/configuration.md#climate)).

Settings are retained documents under that prefix. While climate is off the REST routes answer `503` and the MCP tools answer `climate disabled`.

## The Climate screen

The **Climate** tab appears when climate is on. It lists the rooms with their state, the current reading and what the engine decided (the source and the reason). Open a room to see its panel.

The room card:

- **Now**: the current reading, with the room thermometer's own value and age when it is more than 15 minutes old.
- **Target**: the decided temperature with **−** and **+**. Each press moves it by 0.5 °C; once you stop, one hold is sent, until the next schedule change (two hours when there is none). Chips then offer the same target for **1 h**, **2 h** or **Until next change** for a few seconds.
- Under the figures, why the engine chose the target (its source and reason).
- Active holds, one chip each, with a **✕** that clears them.
- **Zone state**: Off, Pending or Active. **−** and **+** only work in an Active room.
- **Eco…** opens a small popup to start eco until a date and time; a running eco reads **Eco until …** with a **✕** that cancels it (see [Eco](#eco)).

The **⚙** on the card opens the room's settings:

- **Room thermometer**: Automatic (the room's own temperature sensor when it has one, else the mean of its climate devices' readings) or a sensor you pick.
- **Thermometer correction**: the switch, a one-line summary of the step, tolerance and interval, and its seven numbers folded under **Advanced settings** (see [Thermometer correction](#thermometer-correction)).
- **Room presence** (see [Room presence](#room-presence)).
- **Learning**: where the learner stands, and **Learn now** to run it on demand.
- **Recent decisions**: the engine's last decisions for the room.

Below the card:

- **This week**: the approved schedule as a week calendar, with a base temperature and coloured blocks. **Create schedule** starts one when there is none, **Edit schedule** changes it, and you tap an empty spot to add a block.
- **Proposed schedule**: what the learner found, with **Approve**, **Reject** and, after you edit it, **Approve edited**.

The **⚙** in the page header opens **Settings**, the house document (see below). Banners explain when learning is off or [History](history.md) is not set up.

## House settings

These are the house-wide settings, with their defaults:

- **Season**: `auto` (default), `heat`, `cool` or `off`. In `auto` the mean outdoor temperature picks heat when it is below **Heat below** (`heat_below`, 16 °C) and cool when it is above **Cool above** (`cool_above`, 24 °C); in between the previous mode holds. With no outdoor reading it heats. **Outdoor temperature** (`outdoor_entity`) names the sensor.
- **Holiday** (`holiday_entity`): an entity that, while on, puts every zone in holiday: frost temperature in heat, off in cool.
- Safety limits: **Frost** (`frost`, 7 °C), **Overheat** (`overheat`, 30 °C), **Overcool** (`overcool`, 16 °C), and **Stale after** (`stale_after_min`, 60 minutes without a reading). A room thermometer stays the room reading for as long as the room's `quiet_limit_min` (6 h) when that is longer: most thermometers only report a change, so a steady room can leave them silent for hours. Past that, the hub uses the devices' own readings and the reason says "the room thermometer has been silent for over 6 h".
- Learner: **Learning window** (`learn_days`, 28), **Minimum days** (`learn_min_days`, 10), **Agreement needed** (`learn_support`, 0.6) and **Learn at** (`learn_at`, 03:30).
- **Presence**, **Away grace** and the eco settings (see [Eco](#eco)).

## Zones

Each area with a climate device is a zone. A zone is in one of three states:

- `off`: the engine does not command its devices.
- `pending`: waiting for its first approved schedule. A pending zone acts only on safety.
- `active`: running its schedule and overrides.

## How a zone is decided

The engine decides per zone from a priority tree, and every decision carries its `source` (`safety`, `override`, `boost`, `holiday`, `eco`, `away`, `empty`, `schedule` or `idle`) and a `reason`:

1. **Safety** comes first and always acts: frost, overheat, overcool, and a stale reading (no usable temperature, so no schedule, override or eco setpoint is sent to an active zone). A safety cause that holds stays until the reading is clearly back inside its limit.
2. **Pending**: a pending zone does nothing beyond safety.
3. **Zone override**: a whole-zone target and/or mode held for a number of minutes or until a time. One with a temperature is a **boost** (source `boost`): a room target, reached through the room-thermometer correction like the schedule's. Leaving the house (nobody home, or a holiday starting) ends a boost set before the leave; a boost set while away runs, to warm the room before you come back. An app sets one with `<prefix>/zone/<zone>/hold/set` (below).
4. **Rules**: holiday, then eco (the zone's own eco, or the whole house's while nobody is home), then the schedule, set back while the room is empty (see [Room presence](#room-presence)).

Per-device overrides (hand changes) sit on top of the rules and hold their device whatever the zone does. An override the device cannot run (cool on a heat-only valve) holds it as it is, and the reason says "held as it is".

A temperature set by hand on a device holds until the zone's next scheduled change, at most `manual_hold_max_min` minutes (240 by default). Only an active zone watches for hand changes: what is set by hand while a zone is off is not held when it is switched back on, and the schedule applies.

The engine keeps deciding when history is off; only the learner pauses.

## Eco

Eco saves energy while a room, or the whole house, is empty. It is set once for the house, in the climate settings (`eco` in the house document):

- `heat` (16 °C by default) and `cool` (28 °C): what a zone holds in eco. Eco never uses more than the schedule: in heat the zone takes the lower of `heat` and its schedule, in cool the higher of `cool` and its schedule, a schedule `off` stays off, and a zone with no rules for the season stays idle.
- `actions`: extra settings for the zone's climate units while in eco. Today one kind exists, the fan speed: `[{"fan_mode": "low"}]`. Only units that list that fan mode get it; the others are left alone. When eco ends, each fan goes back to the speed it had, unless someone changed it in the meantime.

Eco starts two ways, and a zone is in eco while either one applies:

- **One room until a date** (`source: eco`). On the Climate screen, open the room and choose **Eco until…**, pick the end date and time, and start it. It ends by itself at that time, or with **Cancel eco**. The decision reads "Eco until Tue 3 Nov 19:00: 14.0 °C". Over REST it is `PUT /api/v1/climate/zones/{id}/eco` with `{"until": <unix seconds>}`; over MCP, `set_climate_eco`. Not available on a zone that is `off`.
- **The whole house while nobody is home** (`source: away`). The hub's own [alarm](alarm.md) armed away always means nobody home, even with an empty **Presence** list and even when a phone in the list still says home. Disarmed, it leaves the decision to the list. Another alarm panel can be added to **Presence** in the climate settings: armed away (or vacation) means nobody home, also over the phones, and disarmed or armed home or night means someone is. While a panel is arming, in its entry delay or triggered, it keeps its last answer. Phones and presence sensors can sit in the same list: the house is home when one of them says so. The Presence field opens the [entity picker](web-ui.md#picking-an-entity) on the alarm panels, phones and presence sensors; tick each one, then Done. When the phones and sensors all say away, `away_grace_min` (15 min) passes before the house goes to eco. An alarm armed away needs no grace: eco starts on the next pass, so a schedule block that begins after you arm starts in eco, and disarming brings the schedule straight back.

The two are independent: a bedroom in eco until Sunday stays in eco when the house is disarmed on Saturday. Holiday and overrides still come first, and a temperature set by hand during eco holds as usual.

## Thermometer correction

A climate unit often reads its own surroundings, which can sit far from the room's temperature. With **Thermometer correction** on, the hub nudges the unit's target from the room thermometer's reading so the room, not the unit, reaches the schedule target. It is off by default and set per room, on the room's card on the Climate screen; it needs a room thermometer (pick one in the same card).

It acts only on the **Schedule** row of an active room with a temperature target: never under holiday, eco or away, nor under a whole-room hold (the hold buttons on the room card). A hand change on one unit leaves only that unit out; the others keep being corrected, and if every unit is held the offset stays where it is.

The room reading is checked on every pass. Inside the band (`band`, 0.5 °C either side of the target) nothing changes and the offset stays where it is. Outside it, the two directions behave differently:

- **The saving side acts at once.** In heat, a room above the band lowers the unit's target by the whole gap, rounded to `step` (0.5 °C). For example, 21.2 °C on a 20 °C target gives −1.0, and 21.9 °C gives −2.0. In cool, a room below the band raises it the same way. This jump does not wait for `interval_min`, and it only ever makes the offset deeper. If the room stays outside the band, the offset keeps moving one `step` every `interval_min` (15 min).
- **The other side waits for the warm-up.** Raising the unit's target in heat (room too cold), or extra cooling in cool (room too warm), moves one `step` every `interval_min`, and only once the warm-up is over. The warm-up ends as soon as the room first reaches the band after the current target began, or `warmup_min` (60 min) after it began at the latest. At a block start the room is cold because heating has just started, not because the unit is set too low, and the warm-up keeps the offset learned so far. Coming back up after a jump is on this side too: one step per interval.

The offset never goes more than `max_below` (3 °C) under or `max_above` (0.5 °C) over the target. When the target changes (a new schedule block, an empty-room setback starting or ending), the offset carries over, since it measures how far the unit reads from the room rather than the target. It is only held within the new target's limits, and the warm-up starts again. It goes back to 0 when the Schedule row stops applying, the room's settings change, or the thermometer goes quiet. A hub restart keeps it: the hub picks up the offset from the room's last decision, and the next step still waits `interval_min` after that decision. After a restart the warm-up counts as not reached yet, with its `warmup_min` limit counted from that decision.

If the thermometer has not reported for `quiet_limit_min` (6 h), the correction is dropped and the unit follows the schedule target alone. Under the room's reading, the screen shows the thermometer's value and its age once it is 15 min old or more.

Each correction is written in the decision's reason, for example "room above 19.5 °C: AC 18.5 °C (-0.5)", and appears under **Recent decisions**. The settings live in the zone's `feedback` block (`enabled`, `band`, `step`, `interval_min`, `warmup_min`, `max_below`, `max_above`, `quiet_limit_min`).

## Room presence

Eco covers a house nobody is in. **Room presence** covers a room nobody is in while the rest of the house is lived in: after a while with no one detected in it, the room's schedule block is lowered (raised in cool) until someone stays again. It is off by default and set per room, on the room's **Room presence** card on the Climate screen, where you pick the room's presence sensors (its occupancy, motion and presence binary sensors).

- **Empty**: no sensor has detected anyone for `empty_after_min` (30 min). Any one sensor detecting counts, and a motion sensor's short pulse between two passes counts too.
- **Occupied again**: someone is still detected near the end of `back_after_min` (5 min) after the first detection. Walking through the room does not end the setback; staying does, within about those 5 min.
- Sensors that are unavailable are ignored. With none picked, or all unavailable, the room counts as occupied. After a hub restart the room counts as occupied until `empty_after_min` passes with nobody detected.

It acts only during a schedule **block** of an active room with a temperature target: outside blocks the room is already at its base. In heat the block is lowered by `setback` (2 °C), never under the rules' base or the frost limit; in cool it is raised by `setback`, never over the base. Safety, overrides, holiday, eco and the whole house's "nobody home" come first, and a temperature set by hand on a unit still holds that unit.

The decision's source is `empty`, shown as **Empty room** under **Recent decisions**, with a reason such as "No one in the room for 30 min: 17.0 °C (19.0 − 2.0)". The card says whether the room counts as occupied or since when it is empty. The [thermometer correction](#thermometer-correction) works on top, against the lowered target, and keeps its offset when the setback starts or ends. The learner ignores the time a room spent set back. The settings live in the zone's `presence` block (`enabled`, `sensors`, `empty_after_min`, `back_after_min`, `setback`); over MCP, `set_climate_zone` takes them and `get_climate_zone` shows the live state (`presence.empty_since`).

## The learner and proposals

The learner builds a weekly schedule from the devices' target history (time held in eco, away, a room set back as empty, holiday, safety or an override is not evidence) in the history database and proposes it with the evidence behind it. Once a schedule is approved, a part of it changes only where enough days (**Agreement needed**) held the same other value there: hand changes that vary from day to day keep the approved block, and days with no data keep theirs. Nothing applies until it is approved, through REST, MCP or the push button. The hub remembers a fingerprint of each rejected rule set. A room in eco is not learned until its eco ends (its learning state reads `eco`, "Learning paused while in eco"), so no new proposal arrives for it meanwhile; a proposal made before the eco stays. Requires [History](history.md).

The approval routes and the rest of the climate API are in the [HTTP API](../reference/http-api.md); the MCP tools are in [MCP tools](../reference/mcp-tools.md).
