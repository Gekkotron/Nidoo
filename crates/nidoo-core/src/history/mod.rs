//! Pure recorder model: dedupe cache, bounded pending queue, value/availability rows.
//!
//! The hub feeds the recorder with each `Change::StateChanged` as an [`Incoming`]. The
//! recorder's job is purely functional here: compare the incoming state against the last
//! written one (held in [`LastWritten`]), produce [`Rows`] with at most one value row and
//! one availability row, and never let a non-finite number or an `Unknown` disturb the
//! cache.

pub mod buckets;
pub mod grid_day;

use std::collections::VecDeque;

use crate::model::{Availability, EntityState, StateValue};

/// Microseconds since the Unix epoch, as the hub's clock reads them.
pub type Micros = i64;

/// One hour in microseconds.
pub const HOUR: Micros = 3_600_000_000;

/// The chartable number for a state value, or `None` when it is not a number
/// (text, alarm, vacuum, media player) or when the number is not finite.
pub fn num_of(value: &StateValue) -> Option<f64> {
    let n = match value {
        StateValue::Number(x) => *x,
        StateValue::OnOff(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        StateValue::Light(l) => {
            if l.on {
                1.0
            } else {
                0.0
            }
        }
        StateValue::Cover(c) => c.position.map(f64::from)?,
        StateValue::Climate(c) => c.current?,
        StateValue::WaterHeater(w) => w.current?,
        StateValue::Text(_)
        | StateValue::Alarm(_)
        | StateValue::Vacuum(_)
        | StateValue::MediaPlayer(_)
        | StateValue::Generic(_)
        | StateValue::Unknown => return None,
    };
    n.is_finite().then_some(n)
}

/// A state received by the recorder.
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming {
    pub at: Micros,
    pub id: String,
    pub state: EntityState,
    /// Precomputed by the hub: `types::history_num` for a generic state, else [`num_of`].
    pub num: Option<f64>,
}

/// A row to insert in `entity_state`.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueRow {
    pub at: Micros,
    pub id: String,
    pub value: StateValue,
    pub num: Option<f64>,
}

/// A row to insert in `entity_availability`.
#[derive(Clone, Debug, PartialEq)]
pub struct AvailabilityRow {
    pub at: Micros,
    pub id: String,
    pub availability: Availability,
}

/// What one `diff` call produced: at most one value row and one availability row per
/// incoming, but [`Rows`] accumulates across many incoming states in a batch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    pub values: Vec<ValueRow>,
    pub availability: Vec<AvailabilityRow>,
}

impl Rows {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.availability.is_empty()
    }
}

/// Per-entity "last written" cache. Seeded from the database on startup, then updated by
/// `diff` as rows are produced; a dropped or skipped state never touches it.
#[derive(Clone, Debug, Default)]
pub struct LastWritten {
    values: std::collections::HashMap<String, StateValue>,
    availability: std::collections::HashMap<String, Availability>,
}

impl LastWritten {
    pub fn seed_value(&mut self, id: String, value: StateValue) {
        if !matches!(value, StateValue::Unknown) {
            self.values.insert(id, value);
        }
    }

    pub fn seed_availability(&mut self, id: String, a: Availability) {
        if !matches!(a, Availability::Unknown) {
            self.availability.insert(id, a);
        }
    }

    /// Produces at most one value row and one availability row into `rows`, following the
    /// recorder rules: skip `Unknown`, skip non-finite numbers, and compare the full
    /// `StateValue` with `PartialEq` so a brightness change on the same `on` is a row.
    pub fn diff(&mut self, incoming: &Incoming, rows: &mut Rows) {
        let incoming_value = &incoming.state.value;
        let value_writable = !matches!(incoming_value, StateValue::Unknown)
            && !matches!(incoming_value, StateValue::Number(n) if !n.is_finite());
        if value_writable {
            let differs = self
                .values
                .get(&incoming.id)
                .is_none_or(|cached| cached != incoming_value);
            if differs {
                rows.values.push(ValueRow {
                    at: incoming.at,
                    id: incoming.id.clone(),
                    value: incoming_value.clone(),
                    num: incoming.num,
                });
                self.values
                    .insert(incoming.id.clone(), incoming_value.clone());
            }
        }

        let a = incoming.state.availability;
        if !matches!(a, Availability::Unknown) {
            let differs = self
                .availability
                .get(&incoming.id)
                .is_none_or(|cached| *cached != a);
            if differs {
                rows.availability.push(AvailabilityRow {
                    at: incoming.at,
                    id: incoming.id.clone(),
                    availability: a,
                });
                self.availability.insert(incoming.id.clone(), a);
            }
        }
    }
}

/// A bounded FIFO of pending [`Incoming`]s, used by the writer to hold what it cannot
/// flush yet. When the queue is full, the oldest is dropped and `push` returns `false`.
#[derive(Debug)]
pub struct Pending {
    inner: VecDeque<Incoming>,
    capacity: usize,
}

impl Pending {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// Pushes to the back; returns `false` iff it had to drop the oldest to make room.
    pub fn push(&mut self, i: Incoming) -> bool {
        if self.inner.len() >= self.capacity {
            self.inner.pop_front();
            self.inner.push_back(i);
            false
        } else {
            self.inner.push_back(i);
            true
        }
    }

    pub fn take(&mut self, n: usize) -> Vec<Incoming> {
        let k = n.min(self.inner.len());
        self.inner.drain(..k).collect()
    }

    /// Puts a batch back at the front, keeping its order (used when a flush failed).
    pub fn requeue_front(&mut self, batch: Vec<Incoming>) {
        for i in batch.into_iter().rev() {
            self.inner.push_front(i);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests;
