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

An **Active** room shows the same card on **Home** too, at the top of its room, above the device cards. There its **⚙** opens the Climate screen. A Pending or Off room shows no card on Home.

The **⚙** on the card opens the room's settings:

- **Heaters** (see [Heaters](#heaters)): the room's on/off heaters, each with its drive and cool-down, then **Default heater** and the **The climate units take over while a heater cannot run** checkbox.
- **Room thermometer**: Automatic (the room's own temperature sensor when it has one, else the mean of its climate devices' readings) or a sensor you pick.
- **Thermometer correction**: the switch, a one-line summary of the step, tolerance and interval, and its seven numbers folded under **Advanced settings** (see [Thermometer correction](#thermometer-correction)).
- **Room presence** (see [Room presence](#room-presence)).
- **Learning**: where the learner stands, and **Learn now** to run it on demand.
- **Recent decisions**: the engine's last decisions for the room.

Below the card:

- **This week**: the approved schedule as a week calendar, with a base temperature and coloured blocks. **Create schedule** starts one when there is none, **Edit schedule** changes it, and you tap an empty spot to add a block. In a heat block, the **Heated by** row picks which heaters heat it (see [Heaters](#heaters)).
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

## Heaters

Some rooms heat with more than the climate unit: an electric radiator on a relay, or a stove on a smart plug. These are **on/off heaters**. Any switch can be one, from any area: the stove's plug can sit in the garage and still heat the living room. The engine switches them with the schedule.

Add them in the room's settings, under **Heaters**, above **Room thermometer**. **Add a heater…** lists the switches of the house. The room's climate units are listed above them, read only. Each heater has:

- A drive. **Thermostat** suits a radiator: the heater is on when the room is 0.3 °C or more under the target, and off once the target is reached. **Stays on** suits a stove: the heater is on for as long as it is chosen, whatever the temperature.
- **Cool-down**, in minutes from 0 to 240. Once the heater is off, it is not switched on again before the cool-down has passed. A stove might use 15.
- A switch to enable it. A disabled heater is left alone entirely.
- **✕** removes it.

**Default heater** chooses what heats the room when a schedule block, or the base temperature, does not choose. Its chips are the climate units and the heaters. A chip with ✓ is chosen, and any mix is allowed.

The checkbox **The climate units take over while a heater cannot run** (`fill_in`) covers a heater that is cooling down or unreachable. The room's climate units then heat to the same target until it can. They stop once the heater runs again. A **Stays on** heater never triggers this, however cold the room gets: the hub knows a switch's on or off state and whether it is reachable, but not whether a stove has gone out.

In a heat block, the **Heated by** row, after **Off**, picks the heaters for that block: **Room default**, or one chip per device. The row appears only in heat blocks, and only when the room has an on/off heater. The week view shows the names under the block's temperature.

What the hub does with on/off heaters:

- **Season and room state.** Outside the heating season, with the season off, or with the room off, the hub sends an on/off heater nothing at all, not even **off**. When a room stops heating, the hub sends one **off** to each heater it had switched on itself, and does not retry if that command is lost. A heater held by hand, or a disabled one, gets nothing.
- **Cool-down.** Switching off is never delayed. Switching on waits for the cool-down. This holds for the schedule, a boost, a hand hold ending, frost, and after a hub restart. A heater the hub has never seen counts as switched off when the hub starts.
- **Gaps.** If the next block that names a **Stays on** heater starts within its cool-down, the heater stays on through the gap.
- **Eco, away and holiday.** A **Stays on** heater is off. If the room's default heater is only **Stays on** heaters, the climate units hold the eco temperature instead.
- **Frost.** Frost protection switches **Thermostat** heaters on, after their cool-down. **Stays on** heaters are not used for frost. Outside the heating season, a **Stays on** heater gets nothing at all.
- **Overheat and no reading.** An overheating room, or a room with no usable temperature reading, switches every on/off heater off, even one switched on by hand.
- **Hand changes.** Switching a heater by hand holds it until the room's next schedule change, at most the room's hand-hold limit (`manual_hold_max_min`, 240 minutes by default). A hand **off** starts the cool-down. After a hub restart the switch's live state is taken as it is, and no hold is created.
- **Thermometer correction** does not move while only on/off heaters heat the room. It starts again from 0 when a climate unit heats.
- **Missing devices.** A heater whose device is gone or disabled is ignored. The room default applies, then the climate units.
- **Limits.** Heaters are for heating only: cooling is unchanged, and a room with no climate unit has no heaters. The thermostat band is 0.3 °C under the target and is not a setting. The hub cannot tell whether a heater really heats: it has no power or flame check.

For API clients, the zone settings take `heaters` (a list of `entity`, `drive` as `thermostat` or `hold`, `enabled`, `min_off_min` and `min_on_min`, the minimum run of a thermostat heater, 0 to 120 minutes, which safety ignores), `heated_by` (the room default, where `null` means the climate units) and `fill_in`. A schedule block takes `heated_by`, a list of entity ids; leave it out for the room default. It is valid only on a heat block, and an empty list is refused. The decision's `heaters` gives each heater's `off_since`, `on_since` and `wait_until`, and a switch in the decision's `entities` has mode `heat` (on) or `off`.

## How a zone is decided

The engine decides per zone from a priority tree, and every decision carries its `source` (`safety`, `override`, `boost`, `holiday`, `eco`, `away`, `empty`, `schedule` or `idle`) and a `reason`:

1. **Safety** comes first and always acts: frost, overheat, overcool, and a stale reading (no usable temperature, so no schedule, override or eco setpoint is sent to an active zone). A safety cause that holds stays until the reading is clearly back inside its limit.
2. **Pending**: a pending zone does nothing beyond safety.
3. **Zone override**: a whole-zone target and/or mode held for a number of minutes or until a time. One with a temperature is a **boost** (source `boost`): a room target, reached through the room-thermometer correction like the schedule's. Leaving the house (nobody home, or a holiday starting) ends a boost set before the leave; a boost set while away runs, to warm the room before you come back. A boost also ends the room's own eco (**Eco until…**), so when the boost is over the schedule applies, not eco. An app sets one with `<prefix>/zone/<zone>/hold/set` (below).
4. **Rules**: holiday, then eco (the zone's own eco, or the whole house's while nobody is home), then the schedule, set back while the room is empty (see [Room presence](#room-presence)).

Per-device overrides (hand changes) sit on top of the rules and hold their device whatever the zone does. An override the device cannot run (cool on a heat-only valve) holds it as it is, and the reason says "held as it is".

A temperature set by hand on a device holds until the zone's next scheduled change, at most `manual_hold_max_min` minutes (240 by default). Only an active zone watches for hand changes: what is set by hand while a zone is off is not held when it is switched back on, and the schedule applies.

**Who heats** depends on the source. The room's on/off heaters (see [Heaters](#heaters)) take part only in heat, and only in an active or pending zone:

- **Schedule block**: the block's **Heated by**, or the **Default heater** when the block has none.
- **Base temperature**: the **Default heater**.
- **Boost or zone override**: what the schedule would choose right now.
- **Room set back while empty**: the block it lowers.
- **Eco, away or holiday**: the **Default heater**, without any **Stays on** heater.
- **Frost**: every climate unit and every **Thermostat** heater.
- **Overheat**: no heater; every on/off heater is off.
- **Stale reading**: the climate units, as the safety rule above says; every on/off heater is off.
- **Pending zone**: every on/off heater is off.

A device that is not chosen is turned off.

The engine keeps deciding when history is off; only the learner pauses.

## Eco

Eco saves energy while a room, or the whole house, is empty. It is set once for the house, in the climate settings (`eco` in the house document):

- `heat` (16 °C by default) and `cool` (28 °C): what a zone holds in eco. Eco never uses more than the schedule: in heat the zone takes the lower of `heat` and its schedule, in cool the higher of `cool` and its schedule, a schedule `off` stays off, and a zone with no rules for the season stays idle.
- `actions`: extra settings for the zone's climate units while in eco. Today one kind exists, the fan speed: `[{"fan_mode": "low"}]`. Only units that list that fan mode get it; the others are left alone. When eco ends, each fan goes back to the speed it had, unless someone changed it in the meantime.

Eco starts two ways, and a zone is in eco while either one applies:

- **One room until a date** (`source: eco`). On the Climate screen, open the room and choose **Eco until…**, pick the end date and time, and start it. It ends by itself at that time, with **Cancel eco**, or when the room is boosted (a whole-room target from the − / + buttons, the assistant, MCP, an ambiance or `hold/set`). A hold on one unit, or a mode-only hold, leaves eco on. The decision reads "Eco until Tue 3 Nov 19:00: 14.0 °C". Over REST it is `PUT /api/v1/climate/zones/{id}/eco` with `{"until": <unix seconds>}`; over MCP, `set_climate_eco`. Not available on a zone that is `off`.
- **The whole house while nobody is home** (`source: away`). The hub's own [alarm](alarm.md) armed away always means nobody home, even with an empty **Presence** list and even when a phone in the list still says home. Disarmed, it leaves the decision to the list. Another alarm panel can be added to **Presence** in the climate settings: armed away (or vacation) means nobody home, also over the phones, and disarmed or armed home or night means someone is. While a panel is arming, in its entry delay or triggered, it keeps its last answer. Phones and presence sensors can sit in the same list: the house is home when one of them says so. The Presence field opens the [entity picker](web-ui.md#picking-an-entity) on the alarm panels, phones and presence sensors; tick each one, then Done. When the phones and sensors all say away, `away_grace_min` (15 min) passes before the house goes to eco. An alarm armed away needs no grace: eco starts on the next pass, so a schedule block that begins after you arm starts in eco, and disarming brings the schedule straight back.

The two are independent: a bedroom in eco until Sunday stays in eco when the house is disarmed on Saturday. Holiday and overrides still come first, and a temperature set by hand during eco holds as usual.

## Thermometer correction

A climate unit often reads its own surroundings, which can sit far from the room's temperature. With **Thermometer correction** on, the hub nudges the unit's target from the room thermometer's reading so the room, not the unit, reaches the schedule target. It is off by default and set per room, on the room's card on the Climate screen; it needs a room thermometer (pick one in the same card).

It acts only on the **Schedule** row of an active room with a temperature target: never under holiday, eco or away, nor under a whole-room hold (the hold buttons on the room card). A hand change on one unit leaves only that unit out; the others keep being corrected, and if every unit is held the offset stays where it is.

The room reading is checked on every pass. Inside the band (`band`, 0.5 °C either side of the target) nothing changes and the offset stays where it is. Outside it, the two directions behave differently:

- **The saving side acts at once.** In heat, a room above the band lowers the unit's target by the whole gap, rounded to `step` (0.5 °C). For example, 21.2 °C on a 20 °C target gives −1.0, and 21.9 °C gives −2.0. In cool, a room below the band raises it the same way. This jump does not wait for `interval_min`, and it only ever makes the offset deeper. If the room stays outside the band, the offset keeps moving one `step` every `interval_min` (15 min).
- **The other side waits for the warm-up.** Raising the unit's target in heat (room too cold), or extra cooling in cool (room too warm), moves one `step` every `interval_min`, and only once the warm-up is over. The warm-up ends as soon as the room first reaches the band after the current target began, or `warmup_min` (60 min) after it began at the latest. At a block start the room is cold because heating has just started, not because the unit is set too low, and the warm-up keeps the offset learned so far. Coming back up after a jump is on this side too: one step per interval. The warm-up only holds a push past the target, though: an offset still on the saving side (under the target in heat, over it in cool), for example a cut made while the room was empty or before a boost, goes straight back to 0 as soon as the room is too cold (too warm in cool), then waits for the warm-up to go further.

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

The learner never proposes **Heated by**, and when it revises a schedule it keeps the one you chose.

The approval routes and the rest of the climate API are in the [HTTP API](../reference/http-api.md); the MCP tools are in [MCP tools](../reference/mcp-tools.md).
