//! Room-thermometer correction (spec §6.1–§6.4): a small offset on the schedule's target that
//! nudges the device setpoint until the room reaches the target.
//!
//! Pure and deterministic: no I/O, no clock. `now` (Unix seconds) comes in through the input.

use crate::doc::{Decision, FeedbackSettings, Mode, Setpoint, Source};
use crate::schedule::ReadingSource;

/// Two targets within this distance are the same target.
const SAME_TARGET: f64 = 0.01;
/// Age given to a thermometer that has never reported: older than any quiet limit.
pub const UNKNOWN_AGE_S: u64 = u64::MAX / 4;

/// What the correction remembers between two ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackState {
    /// The schedule target this state belongs to.
    pub target: f64,
    /// The offset applied on top of the target, in degrees.
    pub offset: f64,
    /// When this target began (Unix seconds).
    pub since: u64,
    /// The last offset change (Unix seconds).
    pub last_step_at: Option<u64>,
}

/// Everything [`correct`] reads.
pub struct CorrectionInput<'a> {
    pub settings: &'a FeedbackSettings,
    pub mode: Mode,
    /// The schedule's target temperature.
    pub target: f64,
    /// The zone's lowest (heat) or highest (cool) useful setpoint, when set.
    pub base: Option<f64>,
    /// The frost floor: heat never goes below it.
    pub frost: f64,
    /// The room thermometer, if it has ever reported.
    pub thermometer: Option<ReadingSource>,
    /// The amount one correction moves the offset, in degrees.
    pub step: f64,
    /// A device is held by hand or in safety: keep the offset, do not step.
    pub frozen: bool,
    pub previous: Option<&'a FeedbackState>,
    pub now: u64,
}

/// The offset to apply and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    pub offset: f64,
    pub note: Option<String>,
}

/// Work out the offset for this tick.
pub fn correct(input: &CorrectionInput) -> Correction {
    let s = input.settings;
    let target = input.target;
    // A new target keeps the offset: it measures how far the unit reads from the room, whatever
    // the target. Only the warm-up starts over.
    let (start, since, last_step_at) = match input.previous {
        Some(p) if (p.target - target).abs() <= SAME_TARGET => (p.offset, p.since, p.last_step_at),
        Some(p) => (p.offset, input.now, p.last_step_at),
        None => (0.0, input.now, None),
    };

    let Some(room) = input.thermometer else {
        return silent(Some("no room thermometer".into()));
    };
    if room.age_s >= UNKNOWN_AGE_S {
        return silent(Some(
            "room thermometer has not reported: no correction".into(),
        ));
    }
    if room.age_s > u64::from(s.quiet_limit_min) * 60 {
        return silent(Some("room thermometer silent: no correction".into()));
    }
    if input.frozen {
        let (lo, hi) = limits(input);
        return Correction {
            offset: start.clamp(lo, hi),
            note: Some("all units held: correction paused".into()),
        };
    }

    let r = room.value;
    let ready =
        last_step_at.is_none_or(|t| input.now.saturating_sub(t) >= u64::from(s.interval_min) * 60);
    let warm = input.now.saturating_sub(since) >= u64::from(s.warmup_min) * 60;
    let above = r > target + s.band;
    let below = r < target - s.band;

    let mut offset = start;
    if !input.frozen {
        match input.mode {
            Mode::Heat => {
                if above && ready {
                    offset -= input.step;
                } else if below && ready && warm {
                    offset += input.step;
                }
            }
            Mode::Cool => {
                if above && ready && warm {
                    offset -= input.step;
                } else if below && ready {
                    offset += input.step;
                }
            }
        }
    }
    let (lo, hi) = limits(input);
    let offset = offset.clamp(lo, hi);

    // No live reading or age here: the reason must only change with the situation or the action.
    let note = (above || below || offset != 0.0).then(|| {
        let (word, edge) = if above {
            ("above", target + s.band)
        } else if below {
            ("below", target - s.band)
        } else {
            ("near", target)
        };
        let ac = target + offset;
        if offset == 0.0 {
            format!("room {word} {edge:.1} °C: AC {ac:.1} °C")
        } else {
            format!("room {word} {edge:.1} °C: AC {ac:.1} °C ({offset:+.1})")
        }
    });
    Correction { offset, note }
}

fn silent(note: Option<String>) -> Correction {
    Correction { offset: 0.0, note }
}

/// The lowest and highest offset allowed, in whole steps toward zero so that a device that moves
/// in steps never gets more than the limits.
fn limits(input: &CorrectionInput) -> (f64, f64) {
    let (lo, hi) = raw_limits(input);
    (whole_steps(lo, input.step), whole_steps(hi, input.step))
}

fn whole_steps(limit: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return limit;
    }
    ((limit / step).abs() + 1e-9).floor() * step * limit.signum()
}

fn raw_limits(input: &CorrectionInput) -> (f64, f64) {
    let s = input.settings;
    let target = input.target;
    match input.mode {
        Mode::Heat => {
            let mut floor = (target - s.max_below).max(input.frost);
            if let Some(base) = input.base {
                floor = floor.max(base);
            }
            ((floor - target).min(0.0), s.max_above)
        }
        Mode::Cool => {
            let hi = match input.base {
                Some(base) => s.max_below.min(base - target).max(0.0),
                None => s.max_below,
            };
            (-s.max_above, hi)
        }
    }
}

impl FeedbackState {
    /// The state a hub restart picks up from the zone's retained decision: the offset it last
    /// applied, its warm-up and step clock started at that decision. A decision is republished
    /// only when its outcome changes, so its time is about when the offset last stepped. `None`
    /// when that decision carried no correction.
    pub fn restored(decision: &Decision, enabled: bool) -> Option<FeedbackState> {
        if !enabled || !matches!(decision.source, Source::Schedule | Source::Empty) {
            return None;
        }
        let Some(Setpoint::Temp(target)) = decision.setpoint else {
            return None;
        };
        let offset = decision.adjust.filter(|a| a.abs() > f64::EPSILON)?;
        Some(FeedbackState {
            target,
            offset,
            since: decision.at,
            last_step_at: Some(decision.at),
        })
    }

    /// The state to keep after a decision: `Some` only for an enabled, scheduled (or set back)
    /// temperature.
    pub fn after(
        previous: Option<&FeedbackState>,
        decision: &Decision,
        enabled: bool,
        now: u64,
    ) -> Option<FeedbackState> {
        if !enabled || !matches!(decision.source, Source::Schedule | Source::Empty) {
            return None;
        }
        let Some(Setpoint::Temp(target)) = decision.setpoint else {
            return None;
        };
        let offset = decision.adjust.unwrap_or(0.0);
        let state = match previous {
            Some(p) => FeedbackState {
                target,
                offset,
                // A new target restarts the warm-up; the offset and the step clock carry over.
                since: if (p.target - target).abs() <= SAME_TARGET {
                    p.since
                } else {
                    now
                },
                last_step_at: if (offset - p.offset).abs() > f64::EPSILON {
                    Some(now)
                } else {
                    p.last_step_at
                },
            },
            None => FeedbackState {
                target,
                offset,
                since: now,
                // A first tick that already steps counts as a step, or the next would follow
                // at once.
                last_step_at: (offset.abs() > f64::EPSILON).then_some(now),
            },
        };
        Some(state)
    }
}

#[cfg(test)]
mod tests {
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
        }
    }

    #[test]
    fn overshoot_steps_down_once_per_interval() {
        assert_eq!(heat(21.6, None).offset, -0.5);
        let p = state(19.0, -0.5, NOW - 7200, Some(NOW - 600));
        assert_eq!(heat(21.6, Some(&p)).offset, -0.5);
        let p = state(19.0, -0.5, NOW - 7200, Some(NOW - 900));
        assert_eq!(heat(21.6, Some(&p)).offset, -1.0);
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
        let p = state(19.0, -1.0, NOW - 7200, Some(NOW - 60));
        let c = run(Mode::Heat, 17.0, Some(15.0), 21.6, 0, Some(&p), false);
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
        let c = run(Mode::Heat, 19.0, Some(15.0), 21.6, 3 * 3600, None, false);
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
        assert_eq!(cool(24.5, Some(&old(0.0))).offset, 0.5);
        assert_eq!(cool(24.5, Some(&old(2.0))).offset, 2.0);
        assert_eq!(cool(27.0, Some(&old(-0.5))).offset, -0.5);
    }

    #[test]
    fn note_text() {
        let c = run(Mode::Heat, 19.0, Some(15.0), 20.6, 7200, None, false);
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
            applied: false,
            at: NOW,
            entities: BTreeMap::new(),
            eco_restore: BTreeMap::new(),
            adjust,
        }
    }

    #[test]
    fn after_tracks_since_and_steps() {
        let prev = state(19.0, 0.0, 500, None);
        let d = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-0.5));
        let s = FeedbackState::after(Some(&prev), &d, true, NOW).unwrap();
        assert_eq!(s, state(19.0, -0.5, 500, Some(NOW)));
        // no change keeps the last step
        let prev = state(19.0, -0.5, 500, Some(900));
        let s = FeedbackState::after(Some(&prev), &d, true, NOW).unwrap();
        assert_eq!(s.last_step_at, Some(900));
        // a new target restarts the warm-up and keeps the step clock while the offset holds
        let d2 = decision(Source::Schedule, Some(Setpoint::Temp(17.0)), Some(-0.5));
        let s = FeedbackState::after(Some(&prev), &d2, true, NOW).unwrap();
        assert_eq!(s, state(17.0, -0.5, NOW, Some(900)));
        // and records a step when the offset moved
        let d2 = decision(Source::Schedule, Some(Setpoint::Temp(17.0)), Some(-1.0));
        let s = FeedbackState::after(Some(&prev), &d2, true, NOW).unwrap();
        assert_eq!(s, state(17.0, -1.0, NOW, Some(NOW)));
        // a first tick that already steps records the step
        let d3 = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-0.5));
        let s = FeedbackState::after(None, &d3, true, NOW).unwrap();
        assert_eq!(s, state(19.0, -0.5, NOW, Some(NOW)));
        let o = decision(Source::Override, Some(Setpoint::Temp(19.0)), None);
        assert_eq!(FeedbackState::after(Some(&prev), &o, true, NOW), None);
        assert_eq!(FeedbackState::after(Some(&prev), &d, false, NOW), None);
    }

    #[test]
    fn after_keeps_state_for_empty() {
        let d = decision(Source::Empty, Some(Setpoint::Temp(17.0)), None);
        let s = FeedbackState::after(None, &d, true, NOW).unwrap();
        assert_eq!(s.target, 17.0);
        // The setback starting moves the target: the offset carries over, the warm-up restarts.
        let prev = state(19.0, -0.5, 500, Some(900));
        let d = decision(Source::Empty, Some(Setpoint::Temp(17.0)), Some(-0.5));
        let s = FeedbackState::after(Some(&prev), &d, true, NOW).unwrap();
        assert_eq!(s, state(17.0, -0.5, NOW, Some(900)));
        // And so does it ending.
        let back = decision(Source::Schedule, Some(Setpoint::Temp(19.0)), Some(-1.5));
        let s = FeedbackState::after(Some(&state(17.0, -1.5, 500, Some(900))), &back, true, NOW);
        assert_eq!(s, Some(state(19.0, -1.5, NOW, Some(900))));
    }

    #[test]
    fn restored_picks_up_the_retained_offset() {
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
}
