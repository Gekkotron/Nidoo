# Climate: room-thermometer correction and room presence setback — design

Status: draft for the user's review (2026-10-29). Extends the climate engine
([2026-10-22-climate-engine-design.md](2026-10-22-climate-engine-design.md), "the engine spec" below).

## 1. Intent

The goal is to **use less energy** with heat pumps / ACs, without making the rooms
uncomfortable.

Two things waste energy today:

1. **The AC regulates on its own sensor.** The engine sends the scheduled setpoint (say 19 °C) and
   the unit decides when it has reached it, using a sensor inside the unit. A real thermometer in
   the room can read well above that (21.6 °C has been seen with the AC set to 19 °C). The room
   thermometer is only used for safety limits and the season today (engine spec §5.1, §5.4).
2. **Empty rooms stay heated.** "Nobody home" eco is house-wide (phones). A room nobody is in, while
   someone is elsewhere in the house, keeps its full block setpoint.

Two features fix them, built in this order (each its own PR):

- **A. Room-thermometer correction:** nudge the AC's setpoint in 0.5 °C steps, using the room
  thermometer, so the room settles near the target instead of overshooting.
- **B. Room presence setback:** lower a room's scheduled temperature by 2 °C after 30 minutes with
  nobody detected in it.

How the user judges success: the AC's daily consumption in the existing **Energy** view drops for
similar weather, and the room holds around its target (19–19.5 °C for a 19 °C block is fine).
No new consumption screen.

## 2. Goals, non-goals, anti-goals

Goals
- Rooms stop overshooting the scheduled temperature by more than about 0.5 °C when the room
  thermometer says so.
- An empty room drops by a configurable amount after a configurable time, and comes back when
  someone has actually stayed.
- Every adjustment is visible: the room's **Recent decisions** list says what was done and why.
- Both features are off by default and switched on per room.

Non-goals
- Learning schedules from presence; presence-based schedule proposals.
- A new consumption or savings screen (the Energy view already shows the AC's consumption).
- Changing the safety rules (frost, overheat, overcool, stale) or the house-wide "nobody home" eco.
- The Android app (web, hub, REST and MCP only).
- Correcting during a whole-zone **override** (an explicit "set 21 °C until 18:00" is the user's
  intent, kept as is). *Claude's call.*

Anti-goals (a failure even if it "works")
- A room that ends up cold: never more than 3 °C below the target, never below the rules' `base`,
  never below `frost`.
- An AC whose setpoint changes every minute (oscillation, wear, noise).
- The learner slowly lowering the approved schedule because it sees corrected or setback setpoints
  as the user's habits.
- Someone walking through a room re-heating it.
- Acting on device readings (the AC's own sensor) for the correction: that is the sensor being
  corrected.

## 3. Decided with the user

- Correction **stays on and lowers the setpoint step by step**, 0.5 °C a step, step configurable.
  It does not switch the AC off.
- Hysteresis: nothing changes while the room is within ±0.5 °C of the target. A room reaching
  target + 0.5 °C (19.5 °C for 19 °C) is acceptable.
- In winter the scheduled setpoint is sometimes not enough to reach the target: the correction may
  go **up to 0.5 °C above** the schedule, no more.
- The user's room thermometers report **only on change** (a Tuya TS0201-type sensor can be silent
  for 3 h). Silence means "unchanged", not "broken", for the correction.
- Presence: a room is empty after **30 min** with no sensor detecting anyone; the scheduled
  temperature then drops by **2 °C**.
- Coming back needs **a few minutes of presence** (default 5 min), so passing through a room does
  not end the setback.
- A room may have several presence sensors (three in one room); **any one** detecting counts.
  A false "empty" is acceptable (the user grabs a blanket).
- The room's **Recent decisions** list (web, room panel) is where the user follows what happens.

## 4. Definitions

- **Scheduled**: the setpoint of the approved rules at this moment (`ModeRules::at`).
- **Target**: what the room should be. Equal to *scheduled*, or to the setback value when feature B
  is in effect.
- **Offset**: the correction (feature A) currently added to the target, a multiple of the step,
  possibly negative.
- **Commanded**: what is sent to each device: `fit(target + offset, device caps)`.
- **Room thermometer**: the zone's dedicated sensor only (`settings.sensor`, else the area
  thermometer the view reports as `room_sensor`, engine spec §5.4). Never the devices' own
  readings.

## 5. Where it sits in the decision tree

Both features act only when the tree reaches the **Schedule** row (engine spec §5.1, row 5) in an
`active` zone, with a temperature setpoint (not `off`). Safety, overrides, holiday, room eco and
house away are unchanged and win as today. Per-device hand changes (§7) still hold their device
whatever the correction or the setback does.

Order inside the Schedule row: scheduled → setback (B) gives the target → correction (A) gives
the offset → commanded.

## 6. Feature A — room-thermometer correction

### 6.1 Behaviour (heat)

Each evaluation, with a usable room-thermometer value `r` (§6.3):

- `r > target + band` → the offset jumps to `−(r − target)` rounded to the nearest `step`, at
  once, whatever the interval, when that is deeper than the offset held. Otherwise, with at least
  `interval` since the last step → offset −= step.
- `r < target − band`: during the warm-up, a negative offset goes to 0 at once, whatever the
  interval, and a zero or positive one stays. Once the warm-up is over, with at least `interval`
  since the last step → offset += step. The warm-up is over once the room has reached the band
  (`r ≥ target − band`) since the target began, or `warmup` after the target began at the latest.
- Otherwise the offset stays.

*Changed 2026-11-05:* the saving side used to move one `step` per `interval` too, so a room 2 °C
over its target took 45 min to reach −2.0 while the AC kept heating. It now jumps by the gap
(21.2 °C on a 20 °C target → −1.0, 21.9 °C → −2.0) and keeps stepping if the room stays above.
The warm-up used to be a fixed `warmup` (60 min) after the target began; it now ends as soon as
the room first reaches the band, with `warmup` kept as the cap, so the change can only shorten
it. Recovery after a jump stays one `step` per `interval` (*Claude's call*). No new setting.

*Changed 2026-11-09:* the warm-up used to hold every raise, so a cut made under another target
stayed. An empty-room setback (18 °C, room 19.8 °C) cut −2.0; someone came back 11 min later, the
target went to 20 °C with the room at 19.1 °C, and the AC stayed at 18 °C for the whole 60-min
warm-up before stepping back up (20 °C only after ~1 h 45). The warm-up only exists to stop a push
past the target while the room heats up, so it now holds raises above 0 only; a negative offset
unwinds one `step` per `interval` from the start (20 °C ~50 min after the return). Cool mirrors it.

*Changed 2026-11-10:* unwinding one `step` per `interval` was still too slow for a short target. A
1-hour boost to 21 °C started with a −2.5 cut and the room at 20.4 °C: the AC got 18.5 °C, and five
steps of 15 min would have reached 21 °C only after the boost ended. During the warm-up a cut
always comes from another target, and a room under the band contradicts it, so it now drops to 0
at once. After the warm-up, recovery under the same target stays one `step` per `interval`, so a
room swinging around its target does not swing the setpoint with it. Cool mirrors it. No new
setting.

Limits, applied to the result (heat):
- lowest: `target + offset ≥ max(target − max_below, base of the rules, frost)`;
- highest: `target + offset ≤ target + max_above`.

Both limits are relative to the **target**. *Claude's call:* with a setback active the room is
empty, so the floor can sit up to `max_below` under the setback target; the rules' `base` still
bounds it (a 19 °C block over a 15 °C base never commands below 15 °C), and the upper limit means
the correction never undoes a setback.

*Claude's call:* the warm-up (at most `warmup`, default 60 min) stops a boost at every block start:
at 09:00 the room is cold because the block just began, not because the setpoint is too low.

**Cool** mirrors it. Above `target + band` the setpoint goes down one step (more cooling) once the
warm-up is over (reached means `r ≤ target + band`); below `target − band` it jumps up by the gap
rounded to `step` (saving), then steps once per interval, as heat does above the band. Limits in cool:
- lowest (extra cooling): `target + offset ≥ target − max_above`;
- highest (saving): `target + offset ≤ min(target + max_below, base of the rules)` when the base
  is a temperature, else `target + max_below`. `frost` plays no part in cool.

**Zone with several devices.** The offset is one value per zone. Its step is the largest
`temp_step` among the zone's unheld devices, and never less than the configured step; `fit()` then
rounds per device. *Claude's call.*

**Held devices.** A device held by a hand change (engine spec §7) keeps its held value and is left
out of the correction. The other devices keep following target + offset. When **every** device of
the zone is held, the offset is frozen: no steps, since nothing the engine commands can move the
room. When a hold ends, the device simply gets target + offset. *Claude's call.*

### 6.2 State and resets

The engine is stateless per pass today; the offset lives in the hub's climate runtime per zone:
`{ offset, last_step_at, scheduled_at_start, block_started_at }`, plus `reached` (the room has
reached the band since the target began; *added 2026-11-05*).

The offset goes back to 0, and `last_step_at` and the warm-up restart, when:
- the Schedule row stops applying (safety, override, eco, away, holiday, idle);
- the zone's correction settings change, or the feature is switched off.

When the **target** changes (a new block or the base, or a presence setback starting or ending)
the offset carries over, held within the new target's limits: it measures how far the unit reads
from the room, whatever the target. Only the warm-up restarts (`since` and `reached` reset; a
reading already in the new target's band counts as reached at once). *Changed 2026-10-30 (PR #113):*
resetting it threw away the Salon's -2 °C at every presence toggle.

A **hub restart** picks the offset up from the zone's retained decision (`adjust`), when that
decision was a Schedule or Empty temperature and the correction is still on. The warm-up and the
step clock start at the decision's `at`: a decision is republished only when its outcome changes,
so that is about when the offset last stepped. The retained offset is used whatever its age.
`reached` starts false, with the `warmup` cap counted from that `at`: a restart during a heat-up
must not raise early (*Claude's call, 2026-11-05*).
*Changed 2026-10-30:* a reset sent the plain target after each restart and the room overshot
until the correction stepped back down.

### 6.3 Quiet thermometers

- The correction uses the room thermometer's **last value** while it is younger than
  `quiet_limit` (default 6 h), even when the engine's safety staleness (`stale_after_min`, 60 by
  default) already treats it as stale and falls back to the devices' readings.
- Older than `quiet_limit`, or never seen: no correction (offset back to 0), the plain schedule
  goes to the device, and the reason says so once.
- The safety rules keep their own staleness exactly as today.
- **Inputs:** `Reading` only carries the dedicated sensor's value while it is fresh, and `Stale`
  has no age. The hub therefore passes the room thermometer's own last value and age to the
  engine separately (a new field on `ZoneInputs`), whatever `Reading` says.
- **No room thermometer at all** (no `sensor`, no area thermometer): enabling the correction is
  accepted; it stays inactive and the reason says "no room thermometer". *Claude's call:* a
  thermometer added later starts working without touching the settings.
- *Builder note:* staleness today counts "state seen" including unchanged echoes; this sensor class
  sends nothing when unchanged, which is why the decision log often says "the configured sensor is
  stale". Nothing to change in safety; the correction just has its own limit.

### 6.4 Decision record

- `source` stays `schedule`; `setpoint` is the **target**; `entities.<id>.setpoint` is the
  **commanded** value.
- New field `adjust` (the offset, °C; absent or 0 when none).
- The reason is extended, for example:
  "Schedule block 1: 19.0 °C; room 20.6 °C (2 h ago) above 19.5 °C: AC 18.5 °C (−0.5)",
  "…; room 18.2 °C below 18.5 °C: AC 19.5 °C (+0.5)",
  "…; room thermometer silent for 6 h: no correction".
- The reading age shows in the room panel next to the reading ("20.2 °C · 2 h ago").

### 6.5 What must not break

- **Hand changes** (engine spec §7): a commanded value changed by the correction is the engine's
  own command and goes through the ledger like any other; a corrected setpoint must never look
  like a hand change, and a real hand change still holds the device.
- **Command rate** (engine spec §5.5): steps are at most one per `interval` (default 15 min), except
  the saving-side jump (§6.1), which only ever makes the offset deeper; commands
  stay at most once per entity per 60 s, as today.
- **Device step**: the effective step is at least the device's `temp_step` (a device with 1 °C steps
  moves 1 °C at a time). *Claude's call.*
- **Learner** (engine spec §6.2): the device history will show corrected targets. A slot where the
  device holds what the engine commanded with a correction must count as the **target** (the
  decision's `setpoint`), not the corrected value. This needs the decision log
  (`climate_decision`) to keep both the target and the commanded value: the `setpoint` column
  holds the target, and a new `adjust` column holds the offset.
- **Ledger after a restart**: the ledger (`Ledger::from_decisions`) must keep rebuilding what was
  commanded from `entities.<id>.setpoint` (the commanded value), never from the decision's
  `setpoint`, or every corrected device would look like a hand change after a restart.
- **Older rows in the web log** (`DecisionRow.setpoint`) show the target; the reason text already
  says what the AC got.

## 7. Feature B — room presence setback

### 7.1 Presence

- Per zone, a list of presence sensors: binary sensors (occupancy, motion, presence classes) of
  the area, picked by the user. Any one `on` means someone is detected.
- **Empty**: no sensor has been `on` for `empty_after` (default 30 min).
- **Occupied again**: presence has been detected for `back_after` (default 5 min). A single short
  pass (a motion pulse of a minute and nothing after) must not end the setback; someone staying
  must end it within about `back_after`. Motion sensors pulse `on` for about a minute and radar
  sensors stay `on` while someone is there, so the rule has to work for both. *Builder's call on the
  exact rule;* a suggestion: the first detection opens a window of `back_after`; the room is
  occupied if some sensor is `on`, or has turned `on` again, in the last minutes of that window.
- Unavailable sensors are ignored. All of them unavailable, or none configured: the room counts as
  occupied (no setback).
- After a hub restart the room counts as occupied until `empty_after` passes with no detection
  (comfort first). *Claude's call.*

### 7.2 Effect

- Only during a **block** (scheduled differs from the rules' base). Outside blocks the room is
  already at base.
- Heat: target = `max(scheduled − setback, base)`. Cool: target = `min(scheduled + setback, base)`.
- The decision's `source` is a **new source** (name is the builder's; e.g. `empty`), so the learner
  excludes those slots like `away` and `eco` (engine spec §6.2) and the room panel shows it as its
  own state.
- Reason: "No one in the room for 30 min: 17.0 °C (19.0 − 2.0)".
- The correction (A) applies on top, against the setback target.
- House-wide "nobody home" eco, room eco, holiday, overrides and safety win over the setback, as
  today's rows do.

## 8. Settings

New optional fields on the **zone settings** document (engine spec §4.2), parsed with defaults so
existing documents keep working; validated (positive durations, steps > 0, limits ≥ 0):

```json
"feedback": { "enabled": false, "band": 0.5, "step": 0.5, "interval_min": 15,
              "warmup_min": 60, "max_below": 3.0, "max_above": 0.5, "quiet_limit_min": 360 },
"presence": { "enabled": false, "sensors": [], "empty_after_min": 30,
              "back_after_min": 5, "setback": 2.0 }
```

- Web: the room's climate settings get two sections, **Thermometer correction** and **Presence**,
  in English and French like the rest of the climate screens. The presence section lists the
  area's binary sensors to pick from.
- REST and MCP: the existing zone-settings endpoints and tools carry the new fields; the read view
  (`climate_status`, `get_climate_zone`) shows the live state: offset, last step, presence
  (occupied / empty since …).
- User docs: `docs/users/climate.md` gets a short section for each feature.

## 9. Tests the builder should include

Pure engine (in `oikos-climate`, like the existing decide/learn tests):
- overshoot steps down once per interval and stops at `max_below`; never below base or frost;
- undershoot steps up only after `warmup`, at most `max_above`;
- within the band nothing changes (no oscillation around a reading at the band edge);
- a reading 3 h old still corrects; one older than `quiet_limit` resets to the schedule;
- offset resets on a new block, on override/eco/away/safety, and when switched off;
- empty after 30 min lowers a block, not the base; a one-minute pass does not end it; 5 min of
  presence does; unavailable sensors keep the room occupied;
- setback plus correction: the correction never goes above the setback target + `max_above`, and
  the offset resets when the setback starts and ends;
- cool mirror: the explicit cool limits of §6.1;
- several devices with different steps; one device held keeps the others stepping; all held
  freezes the offset;
- after a restart, the ledger rebuilt from decisions does not turn corrected setpoints into hand
  changes;
- learner: corrected and setback slots do not change what is learned.

Hub: a corrected command is not recorded as a hand change; a real hand change during a correction
still holds the device.

## 10. Left to the builder

- Names (source, fields, types), where the runtime state lives, how the decision log stores target
  vs commanded.
- The exact "occupied again" rule (§7.1), within the stated requirements.
- How the web picks presence sensors (filtering by device class and area).

## 11. Claude's calls (for the user to check)

1. No correction during a whole-zone override (§2).
2. Both correction limits are relative to the target: in an empty room the AC may go up to 3 °C
   under the setback value, bounded by the rules' base (§6.1).
3. The upward limit is target + 0.5 °C, so the correction never cancels a presence setback (§6.1).
4. A 60-minute warm-up after a block starts before the correction may raise the setpoint (§6.1).
5. The offset carries over when the target changes, including the setback starting or ending, and
   a hub restart picks it up from the retained decision (§6.2).
6. The step is the largest device step in the zone; held devices are left out; all held freezes
   the offset (§6.1).
7. Correction on a room without a thermometer is accepted and stays inactive (§6.3).
8. After a hub restart a room counts as occupied until 30 min pass without anyone (§7.1).
9. Correction first, presence second, each its own PR.
