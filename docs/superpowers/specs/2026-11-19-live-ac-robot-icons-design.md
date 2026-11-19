# Live AC and robot vacuum icons in the web UI

## Intent

The Android app (Nidoo-app, PR #49) now draws the AC and robot vacuum tiles as small animations that
say what the device is doing: an AC unit blowing air in its mode's colour, a robot cleaning its room
in rows, heading to its dock and charging there. The hub's web UI should show the same living icons
on its cards, so both clients read the same at a glance.

## Goals

- The climate and vacuum card tiles show a live icon driven by the entity's state:
  - **AC:** a wall unit with a swinging vane and three wavy airflow lines in the mode's colour. Dry
    mode shows falling drops instead of airflow. When the AC is off or unavailable, the vane hangs
    closed and nothing flows.
  - **Robot:** a room outline. Cleaning drives the robot along back-and-forth rows, leaving a trail.
    Returning drives it into a dock in the corner. Docked shows it in a mint dock with a pulsing
    bolt. Idle, paused, error and unknown show it still at the start of the room.
- **Pace follows the fan:**
  - AC airflow uses the app's table: silent/quiet/low/"1" → 2400 ms; "2" → 2000; medium/"3" → 1600;
    "4" → 1300; high/"5" → 1000; anything else → 1800.
  - Robot rows use: quiet → 11000 ms; turbo → 6000; max → 4500; anything else → 8000.
  - A new fan speed changes the pace at once, carrying on from the current position.
- **Animations survive card rebuilds.** Room, search and sheet screens rebuild a card on every state
  update, so a battery report must not send the robot back to its first row.
- **Reduced motion:** with `prefers-reduced-motion: reduce`, the icons show still poses and no loop
  runs.

## Non-goals

- The vacuum card's big robot picture (`components/robot.rs`) stays as it is. Only the tile icon
  changes.
- The animated icons appear only in card tiles. Sheet headers, the entity picker, the dashboard
  library and group lists keep static icons.
- No change to the hub's API or state model.

## Anti-goals

- A loop that keeps running while no live icon is on screen, or one loop per icon.
- Per-frame reactive work that re-renders a card. Only the icon's own SVG attributes change each
  frame.

## Approaches

- **A. Fixed SVG per state, animated in CSS.** This is the least code and the way the hub already
  animates its big robot. Its pace comes in coarse steps, though, and a pace change makes the
  animation jump.
- **B. A reactive Leptos icon component with a requestAnimationFrame loop (chosen by the user).**
  It's a port of the app's logic: the positions are computed in plain Rust and native-tested, and
  the pace changes smoothly. It costs more Rust, but the hub already does this for its energy dots
  (`energy_dots.rs`).

## Design (B)

- **New module `src/live_icon/`**, split into pure, natively tested maths and wasm-only drawing:
  - `ac.rs`:
    - mode → tone mapping: cool → cool; heat → heat; dry → cool; fan_only → plain; anything else
      on → auto. Off or unavailable → none.
    - airflow period from the fan mode.
    - vane offset over time, swinging ±0.4 around its rest line below the unit every 3.2 s.
    - drop positions for dry mode, on a 1.4 s cycle.
  - `robot.rs`:
    - the row path `(5,5.5)→(19,5.5)→(19,10)→(5,10)→(5,14.5)→(19,14.5)→(19,19)→(5,19)`, length 69.5.
    - `pose_at(t)`, giving centre and heading, and `trail_to(t)`, copied from the app's
      `RobotPath`.
    - row period from the fan speed.
    - the return glide from (6.5,8.5) to the dock at (19.5,19): heading 39°, eased, arriving at
      2.25 s of a 3 s cycle.
    - the bolt's breathing: opacity 0.4–0.9 over 1.8 s.
  - `clock.rs`:
    - one shared requestAnimationFrame loop for every live icon. It starts when the first icon
      mounts and stops when the last one unmounts.
    - a phase store keyed by entity id and animation (`"<id>/flow"`, `"<id>/rows"`, …). Each
      frame adds `dt / period` to each phase, so a pace change carries on from where it was. A
      rebuilt card picks up its stored phase.
    - `dt` is clamped to 0.1 s, as in `energy_dots`, so a hidden tab doesn't jump on return.
  - `view.rs`: the `<LiveIcon entity>` component.
    - It draws the 24-unit SVG in the hub's line style (stroke 1.75, round caps, `currentColor`),
      with light fills only where the app had them.
    - It sets its few changing attributes (vane `y`, flow dash offset, robot transform, trail dash,
      bolt opacity) from per-frame signals, so nothing else re-renders.
- **Colours** use the hub's existing tokens, so both themes work:
  - cool and dry: `--ring-cool`
  - heat: `--ring-heat`, which is amber in the hub (the app uses rose; we follow the hub)
  - auto: `--ring-other`
  - fan only: plain text colour
  - a moving robot: `--warning`
  - the dock and bolt when docked: `--connected`
- **Wiring:** `CardFrame` renders `<LiveIcon>` in place of the static glyph when the entity is a
  climate or vacuum (groups included) and no explicit `icon` was passed. Other places keep
  `entity_icon`. The static `CLIMATE` and `VACUUM` glyphs are redrawn as the still poses (AC off,
  robot idle), so a sheet header matches its card.

## Tests

- **Native unit tests:**
  - mode → tone mapping
  - both period tables, including unknown values
  - `pose_at` and `trail_to` at their ends, corners and clamping
  - the vane's range
  - phase advance, a pace change carrying on, and the phase surviving a rebuild through the store
- **wasm tests** (`tests/`, like `rich_cards.rs`):
  - a climate card's tile holds the live icon, with the mode's tone and no airflow when off
  - each vacuum state draws its scene: trail and robot, dock, bolt, or still
  - a sheet header still shows the static glyph
- **Existing test:** `icons.rs` `every_kind_has_a_distinct_svg` keeps passing.

## Left to the builder

- The exact SVG element structure and class names.
- Whether per-frame values are signals or direct attribute writes, as long as no card re-renders.
- How the reduced-motion query is read and cached.
