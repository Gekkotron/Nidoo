# Alarm: no disarm from a forged push reply

Closes the P1 item of the 2026-11-17 security audit: "A forged push reply disarms the alarm".

## Why

`on_alarm_reply` accepted a `disarm` reply when its id was one of the open trigger alerts, **or**
when the id merely started with `alarm-` while the alarm was armed. Any broker client that can
publish `nidoo/push/responses` (the phone app's login must, to send real replies) could disarm with
a made-up id. The same rule let an old notification's Disarm turn off an alarm armed again since.

## Goals / non-goals

- Goal: only the Disarm of a trigger alert the hub sent and that is still open disarms.
- Non-goal: codes or PINs (the alarm has none by design, see `docs/users/security.md`).
- Non-goal: a source check on `<alarm prefix>/panel/set`. MQTT gives the hub no publisher
  identity, and the phone app drives its alarm buttons through that topic (its `nidoo-app` login
  keeps publish rights in the bundled ACL).

## Approaches (panel command side)

- **A. Document it (picked).** State in the security, alarm-topic and push docs that any login
  allowed to publish there can disarm, and that the broker must stay private.
- B. Deny the app login on `panel/set` in the ACL: breaks the app's alarm buttons until the app
  moves to another path; a two-repo change.
- C. Hub-checked disarm (hub login only, or a signed command): a new protocol, out of scope.

## Design

- Reply side: drop the `starts_with("alarm-") && armed` fallback; accept a `disarm` reply only when
  its id is in `Runtime::alert_ids`. Those ids are in the retained runtime document, so a restart
  keeps them; disarming clears them (and cancels the alerts), so a stale id is refused.
- Tests (`crates/nidoo/tests/alarm_hub.rs`): a forged id, an info-note id and an offline-warning id
  do nothing while armed or triggered; a resolved alert's id does not disarm a re-armed alarm. The
  existing `push_disarm_reply_disarms` keeps the real path.
- Docs: `security.md`, `alarm.md`, `push-notifications.md`, `reference/mqtt-topics.md`.
