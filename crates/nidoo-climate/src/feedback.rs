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
    /// The room has reached the band since this target began: the warm-up is over.
    pub reached: bool,
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
    let (start, since, last_step_at, reached) = match input.previous {
        Some(p) if (p.target - target).abs() <= SAME_TARGET => {
            (p.offset, p.since, p.last_step_at, p.reached)
        }
        Some(p) => (p.offset, input.now, p.last_step_at, false),
        None => (0.0, input.now, None, false),
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
    // The warm-up ends once the room has reached the band under this target, or after
    // `warmup_min` at the latest.
    let warm = reached || input.now.saturating_sub(since) >= u64::from(s.warmup_min) * 60;
    let above = r > target + s.band;
    let below = r < target - s.band;
    // The saving side (heat: room above, cool: room below) jumps to the gap at once, whatever the
    // interval, when that is deeper than the offset held; otherwise it steps once per interval.
    let gap = nearest_step(r - target, input.step);
    // The warm-up only holds a push past the target (heat: above zero, cool: below). A cut still
    // held on the saving side during it was made under another target: the room on the far side
    // of the band contradicts it, so it drops to 0 at once, whatever the interval.
    let saving = |o: f64| match input.mode {
        Mode::Heat => o < -1e-9,
        Mode::Cool => o > 1e-9,
    };

    let mut offset = start;
    match input.mode {
        Mode::Heat => {
            if above {
                if -gap < offset - 1e-9 {
                    offset = -gap;
                } else if ready {
                    offset -= input.step;
                }
            } else if below {
                if !warm && saving(offset) {
                    offset = 0.0;
                } else if warm && ready {
                    offset += input.step;
                }
            }
        }
        Mode::Cool => {
            if above {
                if !warm && saving(offset) {
                    offset = 0.0;
                } else if warm && ready {
                    offset -= input.step;
                }
            } else if below {
                if -gap > offset + 1e-9 {
                    offset = -gap;
                } else if ready {
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

/// `d` rounded to the nearest whole number of steps.
fn nearest_step(d: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return d;
    }
    (d / step).round() * step
}

/// The room sits in the band or on its far side for `mode`: heat at or above `target - band`,
/// cool at or below `target + band`. A quiet thermometer does not count.
fn band_reached(
    settings: &FeedbackSettings,
    mode: Option<Mode>,
    target: f64,
    thermometer: Option<ReadingSource>,
) -> bool {
    let Some(room) = thermometer else {
        return false;
    };
    if room.age_s > u64::from(settings.quiet_limit_min) * 60 {
        return false;
    }
    match mode {
        Some(Mode::Heat) => room.value >= target - settings.band,
        Some(Mode::Cool) => room.value <= target + settings.band,
        None => false,
    }
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
    /// only when its outcome changes, so its time is about when the offset last stepped. The band
    /// counts as not reached yet, so a restart during a heat-up never raises early. `None` when
    /// that decision carried no correction.
    pub fn restored(decision: &Decision, enabled: bool) -> Option<FeedbackState> {
        if !enabled
            || !matches!(
                decision.source,
                Source::Schedule | Source::Empty | Source::Boost
            )
        {
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
            reached: false,
        })
    }

    /// The state to keep after a decision: `Some` only for an enabled, scheduled (or set back)
    /// temperature. `thermometer` is the room reading the decision used: once it sits in the band
    /// the warm-up is over for this target.
    pub fn after(
        previous: Option<&FeedbackState>,
        decision: &Decision,
        settings: &FeedbackSettings,
        thermometer: Option<ReadingSource>,
        now: u64,
    ) -> Option<FeedbackState> {
        if !settings.enabled
            || !matches!(
                decision.source,
                Source::Schedule | Source::Empty | Source::Boost
            )
        {
            return None;
        }
        let Some(Setpoint::Temp(target)) = decision.setpoint else {
            return None;
        };
        let offset = decision.adjust.unwrap_or(0.0);
        let in_band = band_reached(settings, decision.mode, target, thermometer);
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
                // Once reached, the warm-up stays over until the target changes.
                reached: in_band || ((p.target - target).abs() <= SAME_TARGET && p.reached),
            },
            None => FeedbackState {
                target,
                offset,
                since: now,
                // A first tick that already steps counts as a step, or the next would follow
                // at once.
                last_step_at: (offset.abs() > f64::EPSILON).then_some(now),
                reached: in_band,
            },
        };
        Some(state)
    }
}

#[cfg(test)]
mod tests;
