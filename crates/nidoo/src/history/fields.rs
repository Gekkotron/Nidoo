//! Every value of one entity over a window: the headline and each field as a track, built from
//! the entity's recorded states in one pass.

use nidoo_api::{FieldTrack, TrackData};
use nidoo_core::flow::Scalar;
use nidoo_core::flow::{FieldInfo, field_scalar, headline_scalar};
use nidoo_core::history::Micros;
use nidoo_core::history::buckets::{Sample, fill, rollup, round3};
use nidoo_core::model::StateValue;
use nidoo_core::vocabulary::Reports;

/// What [`build_tracks`] reads: the entity's states around `[from, to)` and how to draw them.
pub struct TrackInput<'a> {
    /// Discovery component, for a type-table entity's headline.
    pub component: &'a str,
    /// The entity's fields, in the order their tracks come.
    pub fields: &'a [FieldInfo],
    /// The number the main chart draws for a state; the headline gets a track only when no
    /// state has one.
    pub headline_num: &'a dyn Fn(&StateValue) -> Option<f64>,
    /// The last state before `from`.
    pub carry: Option<&'a StateValue>,
    /// The states in `[from, to)`, oldest first.
    pub rows: &'a [(Micros, StateValue)],
    pub from: Micros,
    pub to: Micros,
    /// Width of a number track's windows.
    pub window: Micros,
}

enum Column {
    Number {
        carry: Option<f64>,
        samples: Vec<Sample>,
    },
    Steps(Vec<(Micros, Option<Scalar>)>),
}

/// Appends a change unless it repeats the held value; a change at the instant of the last
/// entry replaces it.
fn push_step(steps: &mut Vec<(Micros, Option<Scalar>)>, at: Micros, value: Option<Scalar>) {
    if let Some(last) = steps.last_mut() {
        if last.0 == at {
            last.1 = value;
            let n = steps.len();
            if n >= 2 && steps[n - 2].1 == steps[n - 1].1 {
                steps.pop();
            }
            return;
        }
        if last.1 == value {
            return;
        }
    }
    steps.push((at, value));
}

fn finite(scalar: Option<Scalar>) -> Option<f64> {
    match scalar {
        Some(Scalar::Number(n)) if n.is_finite() => Some(n),
        _ => None,
    }
}

/// The headline track (when the main chart does not draw it) then one track per field.
pub fn build_tracks(input: &TrackInput) -> Vec<FieldTrack> {
    let states = || {
        input
            .carry
            .into_iter()
            .chain(input.rows.iter().map(|(_, v)| v))
    };
    let headline = !states().any(|v| (input.headline_num)(v).is_some())
        && states().any(|v| headline_scalar(input.component, v).is_some());

    let mut specs: Vec<(Option<&FieldInfo>, Column)> = Vec::new();
    let steps_from = |scalar: Option<Scalar>| vec![(input.from, scalar)];
    if headline {
        let held = input
            .carry
            .and_then(|v| headline_scalar(input.component, v));
        specs.push((None, Column::Steps(steps_from(held))));
    }
    // A `recorded` field is the entity's history number, which the main chart already draws.
    for info in input.fields.iter().filter(|info| !info.recorded) {
        let column = match info.reports {
            Reports::Number { .. } => Column::Number {
                carry: input
                    .carry
                    .and_then(|v| finite(field_scalar(v, &info.name))),
                samples: Vec::new(),
            },
            _ => Column::Steps(steps_from(
                input.carry.and_then(|v| field_scalar(v, &info.name)),
            )),
        };
        specs.push((Some(info), column));
    }

    for (at, value) in input.rows {
        for (info, column) in &mut specs {
            match (*info, column) {
                (None, Column::Steps(steps)) => {
                    push_step(steps, *at, headline_scalar(input.component, value));
                }
                (Some(info), Column::Steps(steps)) => {
                    push_step(steps, *at, field_scalar(value, &info.name));
                }
                (Some(info), Column::Number { samples, .. }) => {
                    if let Some(num) = finite(field_scalar(value, &info.name)) {
                        samples.push(Sample { at: *at, num });
                    }
                }
                (None, Column::Number { .. }) => {}
            }
        }
    }

    specs
        .into_iter()
        .map(|(info, column)| {
            let (field, unit) = match info {
                None => (None, None),
                Some(info) => (
                    Some(info.name.clone()),
                    match &info.reports {
                        Reports::Number { unit, .. } => unit.clone(),
                        _ => None,
                    },
                ),
            };
            let data = match column {
                Column::Steps(changes) => TrackData::Steps {
                    changes: changes
                        .into_iter()
                        .map(|(at, v)| (at / 1_000_000, v))
                        .collect(),
                },
                Column::Number { carry, samples } => {
                    let raw = rollup(carry, &samples, input.from, input.to, input.window);
                    let buckets = fill(carry, &raw, input.from, input.to, input.window);
                    TrackData::Number {
                        window: input.window / 1_000_000,
                        points: buckets
                            .iter()
                            .map(|b| {
                                (
                                    b.start / 1_000_000,
                                    round3(b.mean()),
                                    round3(b.min),
                                    round3(b.max),
                                )
                            })
                            .collect(),
                    }
                }
            };
            FieldTrack { field, unit, data }
        })
        .collect()
}

#[cfg(test)]
mod tests;
