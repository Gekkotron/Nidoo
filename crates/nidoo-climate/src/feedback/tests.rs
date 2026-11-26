use super::*;
use std::collections::BTreeMap;

const NOW: u64 = 100_000;

fn settings() -> FeedbackSettings {
    FeedbackSettings::default()
}

fn run(
    mode: Mode,
    target: f64,
    base: Option<f64>,
    r: f64,
    age_s: u64,
    previous: Option<&FeedbackState>,
    frozen: bool,
) -> Correction {
    let s = settings();
    correct(&CorrectionInput {
        settings: &s,
        mode,
        target,
        base,
        frost: 7.0,
        thermometer: Some(ReadingSource { value: r, age_s }),
        step: 0.5,
        frozen,
        previous,
        now: NOW,
    })
}

fn heat(r: f64, previous: Option<&FeedbackState>) -> Correction {
    run(Mode::Heat, 19.0, Some(15.0), r, 0, previous, false)
}

fn state(target: f64, offset: f64, since: u64, last: Option<u64>) -> FeedbackState {
    FeedbackState {
        target,
        offset,
        since,
        last_step_at: last,
        reached: false,
    }
}

fn reached(s: FeedbackState) -> FeedbackState {
    FeedbackState { reached: true, ..s }
}

fn on() -> FeedbackSettings {
    FeedbackSettings {
        enabled: true,
        ..settings()
    }
}

fn off() -> FeedbackSettings {
    FeedbackSettings {
        enabled: false,
        ..settings()
    }
}

fn room(value: f64) -> Option<ReadingSource> {
    Some(ReadingSource { value, age_s: 0 })
}

#[test]
fn overshoot_jumps_by_the_gap() {
    let heat20 = |r: f64| run(Mode::Heat, 20.0, Some(15.0), r, 0, None, false).offset;
    assert_eq!(heat20(21.2), -1.0);
    assert_eq!(heat20(21.9), -2.0);
    // Just over the band: one step.
    assert_eq!(heat20(20.6), -0.5);
}

#[test]
fn jump_ignores_the_interval() {
    let p = state(20.0, -0.5, NOW - 7200, Some(NOW - 60));
    let c = run(Mode::Heat, 20.0, Some(15.0), 21.9, 0, Some(&p), false);
    assert_eq!(c.offset, -2.0);
    assert_eq!(
        c.note.as_deref(),
        Some("room above 20.5 °C: AC 18.0 °C (-2.0)")
    );
}

#[test]
fn jump_never_undoes_a_deeper_offset() {
    let p = state(20.0, -2.0, NOW - 7200, Some(NOW - 60));
    let c = run(Mode::Heat, 20.0, Some(15.0), 20.6, 0, Some(&p), false);
    assert_eq!(c.offset, -2.0);
}

#[test]
fn jump_stops_at_max_below_and_base() {
    let c = run(Mode::Heat, 20.0, Some(15.0), 24.0, 0, None, false);
    assert_eq!(c.offset, -3.0);
    let c = run(Mode::Heat, 20.0, Some(18.0), 24.0, 0, None, false);
    assert_eq!(c.offset, -2.0);
}

#[test]
fn steps_continue_once_per_interval_after_the_jump() {
    // Changed 2026-11-05: the first overshoot jumps by the gap (was one step). While the room
    // stays above the band, the offset still deepens one step per interval.
    let p = state(19.0, -2.5, NOW - 7200, Some(NOW - 600));
    assert_eq!(heat(21.6, Some(&p)).offset, -2.5);
    let p = state(19.0, -2.5, NOW - 7200, Some(NOW - 900));
    assert_eq!(heat(21.6, Some(&p)).offset, -3.0);
}

#[test]
fn recovery_after_a_jump_stays_one_step_per_interval() {
    let p = reached(state(20.0, -2.0, NOW - 7200, Some(NOW - 600)));
    let c = run(Mode::Heat, 20.0, Some(15.0), 19.0, 0, Some(&p), false);
    assert_eq!(c.offset, -2.0);
    let p = reached(state(20.0, -2.0, NOW - 7200, Some(NOW - 900)));
    let c = run(Mode::Heat, 20.0, Some(15.0), 19.0, 0, Some(&p), false);
    assert_eq!(c.offset, -1.5);
}

#[test]
fn warmup_never_holds_a_cut_below_target() {
    // The room emptied: 18 °C with the room at 19.8 cut -2. Someone comes back 11 min later:
    // 20 °C, room 19.1. The new target restarts the warm-up, but the warm-up only holds raises
    // above the target. The room under the band contradicts the cut, so it drops at once.
    // (Changed 2026-11-10: it unwound one step per interval, 50 min for -2.0.)
    let p = state(18.0, -2.0, NOW - 660, Some(NOW - 900));
    let c = run(Mode::Heat, 20.0, Some(15.0), 19.1, 0, Some(&p), false);
    assert_eq!(c.offset, 0.0);
    // A 1-hour boost to 21 °C with the room at 20.4 and a -2.5 cut stepped a minute ago:
    // the AC goes to 21 °C now, not 18.5 °C for 75 min.
    let p = state(18.0, -2.5, NOW - 3600, Some(NOW - 60));
    let c = run(Mode::Heat, 21.0, Some(15.0), 20.4, 0, Some(&p), false);
    assert_eq!(c.offset, 0.0);
    assert_eq!(c.note.as_deref(), Some("room below 20.5 °C: AC 21.0 °C"));
    // At zero, going above waits for the warm-up.
    let p = state(20.0, -0.5, NOW - 1800, Some(NOW - 900));
    assert_eq!(
        run(Mode::Heat, 20.0, Some(15.0), 19.1, 0, Some(&p), false).offset,
        0.0
    );
    let p = state(20.0, 0.0, NOW - 1800, Some(NOW - 900));
    assert_eq!(
        run(Mode::Heat, 20.0, Some(15.0), 19.1, 0, Some(&p), false).offset,
        0.0
    );
    // Cool, mirrored: a raise held above the target drops during the warm-up.
    let p = state(24.0, 1.5, NOW - 600, Some(NOW - 60));
    assert_eq!(
        run(Mode::Cool, 26.0, None, 27.0, 0, Some(&p), false).offset,
        0.0
    );
}

#[test]
fn warmup_ends_once_the_room_reached_the_band() {
    let p = reached(state(19.0, 0.0, NOW - 600, None));
    assert_eq!(heat(18.0, Some(&p)).offset, 0.5);
}

#[test]
fn warmup_cap_when_the_band_was_never_reached() {
    let p = state(19.0, 0.0, NOW - 3599, None);
    assert_eq!(heat(18.0, Some(&p)).offset, 0.0);
    let p = state(19.0, 0.0, NOW - 3600, None);
    assert_eq!(heat(18.0, Some(&p)).offset, 0.5);
}

#[test]
fn new_target_forgets_reached() {
    // Reached under 17 °C; a new 19 °C target starts its warm-up over.
    let p = reached(state(17.0, 0.0, NOW - 7200, None));
    assert_eq!(heat(18.0, Some(&p)).offset, 0.0);
}

#[test]
fn after_marks_the_band_reached() {
    let d = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), None);
    let fresh = state(19.0, 0.0, 500, None);
    let after = |p: &FeedbackState, r: Option<ReadingSource>| {
        FeedbackState::after(Some(p), &d, &on(), r, NOW).unwrap()
    };
    assert!(!after(&fresh, room(18.4)).reached);
    assert!(after(&fresh, room(18.5)).reached);
    // Once reached it stays reached for this target, even when the room cools again.
    assert!(after(&reached(fresh), room(17.0)).reached);
    assert!(after(&reached(fresh), None).reached);
    // A quiet or unknown thermometer does not count.
    let quiet = Some(ReadingSource {
        value: 19.0,
        age_s: 6 * 3600 + 1,
    });
    assert!(!after(&fresh, quiet).reached);
    // A new target starts not reached, unless the room already sits in its band.
    let old = reached(state(17.0, 0.0, 500, None));
    assert!(!after(&old, room(18.0)).reached);
    assert!(after(&old, room(18.6)).reached);
    // A first tick in the band counts too.
    let s = FeedbackState::after(None, &d, &on(), room(19.0), NOW).unwrap();
    assert!(s.reached);
}

#[test]
fn after_marks_the_band_reached_in_cool() {
    let mut d = decision(Source::Schedule, Some(Setpoint::Temp(26.0)), None);
    d.mode = Some(Mode::Cool);
    let fresh = state(26.0, 0.0, 500, None);
    let after = |r: f64| FeedbackState::after(Some(&fresh), &d, &on(), room(r), NOW).unwrap();
    assert!(!after(26.6).reached);
    assert!(after(26.5).reached);
}

#[test]
fn cool_jumps_up_by_the_gap_and_warms_up_the_same_way() {
    let cool = |r: f64, p: Option<&FeedbackState>| run(Mode::Cool, 26.0, None, r, 0, p, false);
    // Too cold: the setpoint rises by the gap at once, whatever the interval.
    let p = state(26.0, 0.0, NOW - 60, Some(NOW - 60));
    assert_eq!(cool(24.8, Some(&p)).offset, 1.0);
    // Too warm: extra cooling waits for the band or the warm-up cap.
    let p = state(26.0, 0.0, NOW - 600, None);
    assert_eq!(cool(27.0, Some(&p)).offset, 0.0);
    let p = reached(state(26.0, 0.0, NOW - 600, None));
    assert_eq!(cool(27.0, Some(&p)).offset, -0.5);
}

#[test]
fn stops_at_max_below_and_base() {
    let p = state(19.0, -3.0, NOW - 7200, Some(NOW - 7200));
    assert_eq!(heat(22.0, Some(&p)).offset, -3.0);
    let c = run(Mode::Heat, 19.0, Some(17.0), 22.0, 0, Some(&p), false);
    assert_eq!(c.offset, -2.0);
}

#[test]
fn band_edge_does_not_step() {
    for r in [19.5, 18.5] {
        let c = heat(r, None);
        assert_eq!(c.offset, 0.0);
        assert_eq!(c.note, None);
    }
}

#[test]
fn undershoot_waits_for_warmup_and_max_above() {
    let p = state(19.0, 0.0, NOW - 1800, None);
    assert_eq!(heat(18.0, Some(&p)).offset, 0.0);
    let p = state(19.0, 0.0, NOW - 3600, None);
    assert_eq!(heat(18.0, Some(&p)).offset, 0.5);
    let p = state(19.0, 0.5, NOW - 7200, Some(NOW - 7200));
    assert_eq!(heat(18.0, Some(&p)).offset, 0.5);
}

#[test]
fn new_target_keeps_offset_and_restarts_warmup() {
    // The step clock carries over: no extra step right after the change.
    // (18.0 is 1.0 over the new target: a jump to -1.0 is not deeper than what is held.)
    let p = state(19.0, -1.0, NOW - 7200, Some(NOW - 60));
    let c = run(Mode::Heat, 17.0, Some(15.0), 18.0, 0, Some(&p), false);
    assert_eq!(c.offset, -1.0);
    // The setback ending: 17 → 19 with the room on the band edge keeps -1.5.
    let p = state(17.0, -1.5, NOW - 7200, Some(NOW - 900));
    let c = heat(19.5, Some(&p));
    assert_eq!(c.offset, -1.5);
    assert_eq!(
        c.note.as_deref(),
        Some("room near 19.0 °C: AC 17.5 °C (-1.5)")
    );
    // The new target's limits still apply: base 15 under target 17 allows -2 at most.
    let p = state(19.0, -3.0, NOW - 7200, Some(NOW - 7200));
    let c = run(Mode::Heat, 17.0, Some(15.0), 17.0, 0, Some(&p), false);
    assert_eq!(c.offset, -2.0);
    // The warm-up starts over: too cold under a new target does not raise yet.
    let p = state(17.0, 0.0, NOW - 7200, None);
    assert_eq!(heat(18.0, Some(&p)).offset, 0.0);
}

#[test]
fn quiet_thermometer() {
    let c = run(Mode::Heat, 19.0, Some(15.0), 19.6, 3 * 3600, None, false);
    assert_eq!(c.offset, -0.5);
    let c = run(
        Mode::Heat,
        19.0,
        Some(15.0),
        21.6,
        6 * 3600 + 1,
        None,
        false,
    );
    assert_eq!(c.offset, 0.0);
    assert!(c.note.unwrap().contains("silent: no correction"));
}

#[test]
fn no_thermometer_note() {
    let s = settings();
    let c = correct(&CorrectionInput {
        settings: &s,
        mode: Mode::Heat,
        target: 19.0,
        base: None,
        frost: 7.0,
        thermometer: None,
        step: 0.5,
        frozen: false,
        previous: None,
        now: NOW,
    });
    assert_eq!(c.offset, 0.0);
    assert_eq!(c.note.as_deref(), Some("no room thermometer"));
}

#[test]
fn frozen_keeps_offset() {
    let p = state(19.0, -0.5, NOW - 7200, Some(NOW - 7200));
    let c = run(Mode::Heat, 19.0, Some(15.0), 22.0, 0, Some(&p), true);
    assert_eq!(c.offset, -0.5);
}

#[test]
fn cool_mirrors() {
    let cool =
        |r: f64, p: Option<&FeedbackState>| run(Mode::Cool, 26.0, Some(28.0), r, 0, p, false);
    let old = |off: f64| state(26.0, off, NOW - 7200, Some(NOW - 7200));
    assert_eq!(cool(27.0, Some(&old(0.0))).offset, -0.5);
    // Changed 2026-11-05: too cold jumps up by the gap (was one step).
    assert_eq!(cool(24.5, Some(&old(0.0))).offset, 1.5);
    assert_eq!(cool(24.5, Some(&old(2.0))).offset, 2.0);
    assert_eq!(cool(27.0, Some(&old(-0.5))).offset, -0.5);
}

#[test]
fn note_text() {
    let c = run(Mode::Heat, 19.0, Some(15.0), 19.6, 7200, None, false);
    assert_eq!(c.offset, -0.5);
    assert_eq!(
        c.note.as_deref(),
        Some("room above 19.5 °C: AC 18.5 °C (-0.5)")
    );
}

fn decision(source: Source, setpoint: Option<Setpoint>, adjust: Option<f64>) -> Decision {
    Decision {
        zone: "z".into(),
        mode: Some(Mode::Heat),
        setpoint,
        source,
        cause: None,
        reason: String::new(),
        reading: None,
        reading_age_s: None,
        outdoor_mean: None,
        home: true,
        holiday: false,
        applied: false,
        at: NOW,
        entities: BTreeMap::new(),
        eco_restore: BTreeMap::new(),
        adjust,
        heaters: BTreeMap::new(),
    }
}

#[test]
fn after_tracks_since_and_steps() {
    let prev = state(19.0, 0.0, 500, None);
    let d = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-0.5));
    let s = FeedbackState::after(Some(&prev), &d, &on(), None, NOW).unwrap();
    assert_eq!(s, state(19.0, -0.5, 500, Some(NOW)));
    // no change keeps the last step
    let prev = state(19.0, -0.5, 500, Some(900));
    let s = FeedbackState::after(Some(&prev), &d, &on(), None, NOW).unwrap();
    assert_eq!(s.last_step_at, Some(900));
    // a new target restarts the warm-up and keeps the step clock while the offset holds
    let d2 = decision(Source::Schedule, Some(Setpoint::Temp(17.0)), Some(-0.5));
    let s = FeedbackState::after(Some(&prev), &d2, &on(), None, NOW).unwrap();
    assert_eq!(s, state(17.0, -0.5, NOW, Some(900)));
    // and records a step when the offset moved
    let d2 = decision(Source::Schedule, Some(Setpoint::Temp(17.0)), Some(-1.0));
    let s = FeedbackState::after(Some(&prev), &d2, &on(), None, NOW).unwrap();
    assert_eq!(s, state(17.0, -1.0, NOW, Some(NOW)));
    // a first tick that already steps records the step
    let d3 = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-0.5));
    let s = FeedbackState::after(None, &d3, &on(), None, NOW).unwrap();
    assert_eq!(s, state(19.0, -0.5, NOW, Some(NOW)));
    let o = decision(Source::Override, Some(Setpoint::Temp(19.0)), None);
    assert_eq!(
        FeedbackState::after(Some(&prev), &o, &on(), None, NOW),
        None
    );
    assert_eq!(
        FeedbackState::after(Some(&prev), &d, &off(), None, NOW),
        None
    );
}

#[test]
fn after_keeps_state_for_empty() {
    let d = decision(Source::Empty, Some(Setpoint::Temp(17.0)), None);
    let s = FeedbackState::after(None, &d, &on(), None, NOW).unwrap();
    assert_eq!(s.target, 17.0);
    // The setback starting moves the target: the offset carries over, the warm-up restarts.
    let prev = state(19.0, -0.5, 500, Some(900));
    let d = decision(Source::Empty, Some(Setpoint::Temp(17.0)), Some(-0.5));
    let s = FeedbackState::after(Some(&prev), &d, &on(), None, NOW).unwrap();
    assert_eq!(s, state(17.0, -0.5, NOW, Some(900)));
    // And so does it ending.
    let back = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-1.5));
    let s = FeedbackState::after(
        Some(&state(17.0, -1.5, 500, Some(900))),
        &back,
        &on(),
        None,
        NOW,
    );
    assert_eq!(s, Some(state(19.0, -1.5, NOW, Some(900))));
}

#[test]
fn restored_picks_up_the_retained_offset() {
    // Not reached: a restart during heat-up must not raise early; the cap counts from `at`.
    let d = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-1.0));
    assert_eq!(
        FeedbackState::restored(&d, true),
        Some(state(19.0, -1.0, NOW, Some(NOW)))
    );
    let d = decision(Source::Empty, Some(Setpoint::Temp(17.0)), Some(-0.5));
    assert_eq!(
        FeedbackState::restored(&d, true),
        Some(state(17.0, -0.5, NOW, Some(NOW)))
    );
}

#[test]
fn after_tracks_boost() {
    let prev = state(19.0, 0.0, 500, None);
    let d = decision(Source::Boost, Some(Setpoint::Temp(22.0)), Some(-0.5));
    let s = FeedbackState::after(Some(&prev), &d, &on(), None, NOW).unwrap();
    assert_eq!(s, state(22.0, -0.5, NOW, Some(NOW)));
}

#[test]
fn restored_picks_up_boost_offset() {
    let d = decision(Source::Boost, Some(Setpoint::Temp(22.0)), Some(-1.0));
    assert_eq!(
        FeedbackState::restored(&d, true),
        Some(state(22.0, -1.0, NOW, Some(NOW)))
    );
}

#[test]
fn restored_skips_decisions_without_a_correction() {
    let d = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-1.0));
    assert_eq!(FeedbackState::restored(&d, false), None);
    let none = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), None);
    assert_eq!(FeedbackState::restored(&none, true), None);
    let eco = decision(Source::Eco, Some(Setpoint::Temp(14.0)), Some(-1.0));
    assert_eq!(FeedbackState::restored(&eco, true), None);
    let off = decision(Source::Schedule, Some(Setpoint::Off), Some(-1.0));
    assert_eq!(FeedbackState::restored(&off, true), None);
}

#[test]
fn restored_offset_waits_for_the_next_interval() {
    // Right after a restart the room still reads warm: the restored step is not repeated
    // until `interval_min` has passed since the retained decision.
    let p = FeedbackState::restored(
        &decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-1.0)),
        true,
    )
    .unwrap();
    let c = run(Mode::Heat, 19.0, Some(15.0), 19.8, 0, Some(&p), false);
    assert_eq!(c.offset, -1.0);
}

#[test]
fn coarse_step_never_exceeds_max_above() {
    let s = settings();
    let c = correct(&CorrectionInput {
        settings: &s,
        mode: Mode::Heat,
        target: 19.0,
        base: Some(15.0),
        frost: 7.0,
        thermometer: Some(ReadingSource {
            value: 17.0,
            age_s: 0,
        }),
        step: 1.0,
        frozen: false,
        previous: Some(&state(19.0, 0.0, NOW - 7200, None)),
        now: NOW,
    });
    assert_eq!(c.offset, 0.0);
}

#[test]
fn never_reported_thermometer_has_its_own_note() {
    let c = run(Mode::Heat, 19.0, None, 21.6, UNKNOWN_AGE_S, None, false);
    assert_eq!(c.offset, 0.0);
    assert_eq!(
        c.note.as_deref(),
        Some("room thermometer has not reported: no correction")
    );
}

#[test]
fn frozen_note_claims_no_ac_value() {
    let p = state(19.0, -0.5, NOW - 7200, Some(NOW - 7200));
    let c = run(Mode::Heat, 19.0, Some(15.0), 22.0, 0, Some(&p), true);
    assert_eq!(c.note.as_deref(), Some("all units held: correction paused"));
}

#[test]
fn note_omits_zero_offset_and_names_near() {
    let c = run(Mode::Heat, 15.5, Some(15.0), 18.0, 0, None, false);
    assert_eq!(
        c.note.as_deref(),
        Some("room above 16.0 °C: AC 15.0 °C (-0.5)")
    );
    let p = state(19.0, -0.5, NOW - 7200, Some(NOW));
    let c = run(Mode::Heat, 19.0, Some(15.0), 19.0, 0, Some(&p), false);
    assert_eq!(
        c.note.as_deref(),
        Some("room near 19.0 °C: AC 18.5 °C (-0.5)")
    );
    let c = run(Mode::Heat, 15.0, Some(15.0), 16.0, 0, None, false);
    assert_eq!(c.note.as_deref(), Some("room above 15.5 °C: AC 15.0 °C"));
}
