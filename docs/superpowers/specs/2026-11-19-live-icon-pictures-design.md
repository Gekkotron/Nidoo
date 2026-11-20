# Large live pictures on the vacuum and climate cards

Status: draft, 2026-11-19. Follows #217 (`2026-11-19-live-ac-robot-icons-design.md`), which made the card
tiles live and kept the vacuum card's big picture as it was. The layout was picked on a local mockup
(climate "A": a large AC on every climate card).

## Intent

Make the hub's two "living" devices read at a glance from across the room, as in the Android app
(Nidoo-app #49): the robot vacuum card's big picture shows the robot cleaning a room, and every climate card
shows a large wall unit blowing air above its dial.

- **Goals:** the vacuum card's big picture becomes #217's robot-in-a-room scene. Every climate card (Home,
  room, search, sheet) gets the AC scene above the dial. Both stay in step with the card's tile.
- **Non-goals:** no change to the tiles, state mapping, commands, pace tables or colours that #217 set. No
  new device kinds.
- **Anti-goals:** a second animation system next to #217's loop. Motion with reduced motion turned on. A
  climate card that breaks the dial or the step buttons on a phone-width grid.

## Approaches

1. **Reuse `LiveIcon`'s scenes at a large size** (picked). They share the frame loop, the phase store and the
   theme tones. Because the phase is keyed by entity and part, the tile and the picture move together.
2. CSS keyframes, as on the mockup page. This was dropped because it duplicates #217 and the two copies
   would drift apart.
3. Keep `components/robot.rs` and only add the AC. This was dropped because the robot's big picture would
   then contradict its tile (a spinning brush in the picture, a room in the tile).

## Design

- **Picture size:** `AcIcon` and `VacuumIcon` take an optional "picture" flag (or a wrapper class). It sets
  their size from CSS and thins the strokes so a 24-unit drawing at about 150 px does not look heavy. SVG ids
  (mask, gradient) get a suffix so the tile and the picture of one entity don't collide.
- **Vacuum card:** `<Robot>` in `vacuum__picture` is replaced by the vacuum scene at about 150 px. The
  caption stays. `components/robot.rs`, its `DUST` table and the `robot-*` keyframes in `cards.css` are
  removed.
- **Climate card:** the AC scene sits above the `Dial`, about 110 px tall and cropped to the unit and its
  airflow. It is shown in every card place. Off and offline show the closed vane, still.
- **Reduced motion:** the still poses #217 already draws.
- **Tests:** extend `tests/live_icon.rs`: the vacuum card shows the room scene and no longer the old robot;
  a climate card shows the large AC above the dial; the tile and the picture share one phase (same airflow
  offset in one frame). Native tests cover any new pure helpers.

## Left to the builder

The exact picture sizes and stroke widths, the crop `viewBox`, and whether the flag is a prop or a class.
