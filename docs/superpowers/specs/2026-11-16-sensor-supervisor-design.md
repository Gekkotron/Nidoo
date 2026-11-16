# Sensor supervisor — design

Date: 2026-11-16 · Status: draft for approval

## Why

The water heater's tank probe read exactly 15.0 °C from 2026-11-11 to 2026-11-16. The value looked plausible, the probe kept reporting, and nothing in Nidoo noticed: it took days and a cold shower to find that the ADC was saturating. The user wants the hub itself to notice data that looks wrong and send a push saying "check this sensor".

## Goals

- **Generic.** Works on every numeric sensor of the house with no per-sensor setup. No thresholds to type in.
- **Learns each sensor's normal** from the recorded history, so a sensor that is flat by nature (a battery at 100 %, a setpoint, a power meter at 0 W overnight) never alerts.
- **One push per problem.** A push when a sensor first looks wrong, nothing more about it while the problem lasts, one "back to normal" push when it recovers. Never a push every hour.
- **Can be turned off and tuned in Settings**, without a hub restart.

## Non-goals

- Cross-sensor reasoning ("the heater draws 1 kW but the tank does not warm"). That is not generic; it stays a flow.
- Binary sensors, switches, text and enum entities. Their changes are event-driven (a door opens when someone uses it), so "no change for a long time" says nothing. Only entities whose history has a numeric value (`entity_state.num`) are supervised.
- A daily digest. The user chose one push per problem.
- Fixing anything. The supervisor only reports.

## Anti-goals (failure even if it "works")

- **Crying wolf.** If it alerts on sensors that are fine, the user mutes it and it is worthless. When unsure, stay silent: a missed alert costs what we have today, a false one costs trust.
- **Repeats.** The same open problem must never push twice, including across a hub restart.
- **Loading the house.** The check must not slow the ingest path or hammer the database.

## Approaches considered

- **A. Learned baseline from history (chosen).** An hourly task reads each sensor's recent history and compares the last hours with its own normal. No thresholds; needs the database; a new sensor gets checks only once it has enough history.
- **B. Fixed rules per sensor kind, in memory.** "A temperature unchanged for 12 h is stuck." Works without the database and from day one, but one rule per kind misfires on sensors that are flat by nature, and the user would maintain an ignore list. Rejected: it fails "works for every sensor".
- **C. Shipped flow templates.** Per-sensor, not generic. Rejected.

The user picked A, and picked "one push per problem" over a daily summary.

## The three checks

All run hourly, for every supervised sensor that is not ignored.

### 1. Stuck — still reporting, value never changes

History holds one row per *change* (the writer only records changed states), so the gaps between rows are the gaps between changes.

- **Baseline:** over the last 14 days, the longest gap between two consecutive changes (`max_gap`). Needs at least 7 days of history and at least 20 changes in the 14 days; otherwise the sensor has no baseline and is skipped by this check.
  - *(my call)* Gaps that overlap a recorded problem of that sensor (open or closed in the last 14 days) are left out, so a past outage does not inflate the baseline and hide the next one.
  - *(my call)* The baseline is computed **once a day** (and at the first run after start) and kept in memory; the hourly run uses it with the live "last change" and "last report" times. This keeps the 14-day scan over compressed chunks to one run a day. A baseline up to a day old is fine for this purpose.
- **Fires when:** time since the last change > `max(max_gap × factor, 6 h)` **and** the hub received a report of that entity within the last `max(max_gap, 1 h)` (it is still talking — otherwise it is check 2).
- `factor` comes from the sensitivity setting (below). With the tank probe (dozens of changes a day, longest gap a few hours) this fires within roughly 6–12 h of going flat.

### 2. Silent — stopped reporting

Reports (messages, changed or not) are not in the history, so this check uses the live report times the hub already sees (`StateTouched`, the same source `trigger.silent` uses).

- **Fires when:** no report for > `max(max_gap × factor, 6 h)` (same baseline as check 1), **or** the entity has been `unavailable` for > 1 h.
- *(my call)* A **report** means a `StateTouched` that read a value for the entity (`read: true`). A message carrying `null` or no value does not count, so a sensor that only sends garbage ends up "silent", not "stuck".
- *(my call)* `unavailable` and since when come from the live registry, not the `entity_availability` table; after a restart the hour starts counting from the start (the rule just below).
- Clocks follow `trigger.silent`'s rules: they start when the hub starts or the broker link comes back, and are frozen while the link is down. Nothing fires for a sensor until the hub has been up and linked for at least the threshold, so a restart never makes everything look silent.

### 3. Impossible — physically out of range

A fixed table keyed by `device_class` (and unit where it matters), only for classes with hard physical bounds:

| device_class | valid range |
| --- | --- |
| `temperature` (°C) | −50 … 150 |
| `humidity`, `battery`, `moisture` (%) | 0 … 100 |
| `illuminance` | ≥ 0 |
| `power` | builder: ≥ 0 unless the entity is known to be signed (grid, battery); safest is to skip `power` |
| `pressure` (hPa) | 800 … 1200 |
| `voltage` (mains, V) | 0 … 300 |
| `co2` (ppm) | 0 … 10 000 |

- **Fires when:** the current value is outside the range. No baseline needed, so it works from day one.
- Sensors with an unknown or missing `device_class` are not range-checked.

## Problem lifecycle and pushes

A *problem* is (sensor, check). Its life:

1. **Open:** the check fires and no problem is open for that (sensor, check). Send one push.
2. **Still open:** later hourly runs that still fire send nothing.
3. **Closed:** only on positive evidence: the sensor changed value (stuck), reported again or came back available (silent), or read a value back in range (impossible). Send one "back to normal" push, `info`. *(my call)* A check that merely cannot run any more (the baseline dropped below 20 changes or 7 days because the sensor was stuck) keeps the problem open; it never closes it.

Open problems are persisted (database), so a restart neither re-sends an open problem's push nor forgets to send its "back to normal".

Pushes go through the existing push service, source `system`, like the watchdogs and outage alerts:

- **Open:** `warning`, title "Tank temperature looks stuck" / "… stopped reporting" / "… reads an impossible value", body with the evidence ("Same value (15.0 °C) for 14 h, usually changes every 3 h at most. Check the sensor."). Localisation keys `supervisor.stuck`, `supervisor.silent`, `supervisor.impossible`, `supervisor.recovered` with arguments, and an English `title`/`body` fallback (the hub's push already carries both). `entity_id` set so the app opens the sensor.
- **Action button "Mute this sensor"** (`id: supervisor_mute:<entity_id>` or equivalent): the reply on `nidoo/push/responses` adds the sensor to the ignore list and closes its problems silently.
- **Recovered:** `info`, "Tank temperature is back to normal".
- One push per problem; if one sensor trips two checks, two problems (rare in practice: stuck and silent exclude each other).
- *(my call)* **Grouping.** When one hourly run opens more than 3 problems at once (a Zigbee coordinator or a bridge dropped, so 30 sensors go silent together), they become **one** push: "12 sensors stopped reporting" with the first names in the body and no Mute button. Each problem is still recorded and listed on the page. The "back to normal" pushes of one run are grouped the same way. And while the hub's own Zigbee2MQTT or broker outage is open (System health), the silent check is paused, since that outage already has its own push.
- *(my call)* **Mute.** The `supervisor_mute:<entity_id>` reply goes through the existing `nidoo/push/responses` dispatcher (the one the alarm's Disarm uses). It adds the sensor to `ignored` and deletes its open problems without a recovered push. Un-muting is on the page only.

## Settings

A new live section (saved like the System page's "Notify me", **no hub restart**), shown as a card on **Settings → System** titled **Sensor supervisor**:

| Field | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | Run the checks at all. |
| `notify` | `true` | Send pushes. Off: problems are still listed on the page and in the API, nothing is sent. |
| `notify_recovered` | `true` | Send the "back to normal" push. |
| `sensitivity` | `normal` | `low` / `normal` / `high` → `factor` 3 / 2 / 1.5 in checks 1–2. |
| `ignored` | `[]` | Entity ids never checked. Edited on the page (entity picker) or by the Mute button. |

The card also lists the **open problems** (sensor, check, since when, evidence) with a Mute button each, and a line "Learning: N sensors still have less than 7 days of history" so the user knows why a new sensor is not covered yet.

Also exposed as:
- `GET /api/v1/supervisor` (settings + open problems) and `PUT /api/v1/supervisor` (settings).
- The open problems in the MCP `system_status` tool output (or a small `supervisor_status` tool — builder's call), so an assistant can answer "is any sensor misbehaving?".

*(my call)* With `notify` off, problems still open and close and are listed, but nothing is pushed. Turning `notify` back on does not send the problems that are already open; only problems opened afterwards push. Turning `enabled` off clears the open problems without pushes.

`enabled` defaults to `true` because the user asked for something that works for every sensor without setup; the anti-goal on false alarms is handled by the conservative rules, not by shipping it off.

## Where the code goes

- **Pure logic** (baseline stats → verdicts, the range table, the problem open/close diff, alert JSON): a new module in `nidoo-watchdog` (`supervisor.rs`), keeping that crate's pattern — no I/O, clock passed in, returns v1 alert JSON. Takes plain structs (entity id, name, device_class, unit, last change, max gap, change count, last report, availability, current value), so the crate keeps no workspace dependency.
- **Hub side** (`crates/nidoo/src/supervisor/`): the hourly task, the database queries (per-entity max gap / change count / last change over 14 days, ideally one grouped query using `lag()` over `entity_state` — keep it off the ingest path and cheap), the live report clocks, the persisted open problems, the settings section, the push-response handler, the HTTP route.
- **Web** (`crates/nidoo-web/src/screens/system_settings.rs`): the card.
- **Docs:** a "Sensor supervisor" section in `docs/users/system-health.md` (or its own page), the new fields in `docs/reference/configuration.md`, the route in `http-api.md`, the alert keys in `push-notifications.md`.

## Constraints

- No database → the supervisor does nothing (and the card says so). It does not fall back to approach B.
- Secret entities are never in the history, so they are never supervised.
- The first hourly run waits until the hub has been up for an hour and the history restore has settled.
- Tests: the pure module gets table-driven unit tests, including the tank-probe case (frequent changes, then flat 15.0 while reporting → one stuck push; next hour → nothing; value moves → one recovered push) and a flat-by-nature sensor (battery at 100 % for 14 days → never alerts).

## Left to the builder

- Exact SQL and whether to read `entity_state` or the hourly rollup for the baseline (the rollup's `min == max` buckets could give flatness cheaply).
- Storage of open problems (a small table is the expected shape).
- `power` in the range table (skip, or ≥ 0 only for non-signed meters).
- MCP: extend `system_status` or add `supervisor_status`.
- Exact wording of titles and bodies, English and French.
- Whether the Android app needs changes for the new localisation keys (check how it renders unknown keys; the English fallback must be readable either way).
