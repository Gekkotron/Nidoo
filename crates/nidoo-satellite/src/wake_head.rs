//! The personal "Hey Jarvis" head: a logistic regression on openWakeWord's
//! 16 × 96 embedding window, trained on the owner's own tries (spike notes,
//! `satellite/mac/spike/src/bin/personal.rs`). The shell computes the features; this
//! module labels them live, trains, and keeps the result as `head.bin`.
//!
//! The recordings and features stay in memory and are dropped with the `Enrolment`.

use std::collections::VecDeque;
use std::io;
use std::path::Path;

use crate::store::write_private_atomic;

/// One feature window: 16 embeddings of 96 values.
pub const FEATURES: usize = 16 * 96;
/// Seconds between two feature windows.
pub const HOP_S: f32 = 0.08;
/// The trained head's file in the store directory.
pub const HEAD_FILE: &str = "head.bin";

/// Louder than this is speech.
const LOUD_DB: f32 = -35.0;
/// Quiet gaps this short stay inside one try.
const MERGE_GAP_S: f32 = 0.4;
const TRY_MIN_S: f32 = 0.3;
const TRY_MAX_S: f32 = 1.3;
/// Positive windows end this long after a try.
const POS_FROM_S: f32 = 0.16;
const POS_TO_S: f32 = 0.8;
/// No negatives this close around any burst.
const NEG_BEFORE_S: f32 = 0.5;
const NEG_AFTER_S: f32 = 1.6;
const EPS: f32 = 1e-3;

/// A trained head.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    mean: Vec<f32>,
    std: Vec<f32>,
    w: Vec<f32>,
    b: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HeadError {
    #[error("a head is {expected} bytes, this one is {got}")]
    WrongSize { expected: usize, got: usize },
}

impl Head {
    /// The probability that `x` (a feature window) ends a "Hey Jarvis".
    pub fn prob(&self, x: &[f32]) -> f32 {
        let z = self.b
            + x.iter()
                .zip(&self.mean)
                .zip(&self.std)
                .zip(&self.w)
                .map(|(((x, mean), std), w)| w * (x - mean) / std)
                .sum::<f32>();
        1.0 / (1.0 + (-z).exp())
    }

    /// mean, std, w, then b; f32 little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.mean
            .iter()
            .chain(&self.std)
            .chain(&self.w)
            .chain([&self.b])
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Head, HeadError> {
        let expected = (3 * FEATURES + 1) * 4;
        if bytes.len() != expected {
            return Err(HeadError::WrongSize {
                expected,
                got: bytes.len(),
            });
        }
        let v: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        Ok(Head {
            mean: v[..FEATURES].to_vec(),
            std: v[FEATURES..2 * FEATURES].to_vec(),
            w: v[2 * FEATURES..3 * FEATURES].to_vec(),
            b: v[3 * FEATURES],
        })
    }

    #[cfg(test)]
    pub(crate) fn zero() -> Head {
        Head {
            mean: vec![0.0; FEATURES],
            std: vec![1.0; FEATURES],
            w: vec![0.0; FEATURES],
            b: 0.0,
        }
    }
}

/// Standardized, class-balanced logistic regression with L2, full-batch gradient
/// descent (the spike's values).
pub fn train(pos: &[Vec<f32>], neg: &[Vec<f32>]) -> Head {
    let xs: Vec<&[f32]> = pos.iter().chain(neg).map(Vec::as_slice).collect();
    let n = xs.len() as f32;
    let mut mean = vec![0.0f32; FEATURES];
    let mut std = vec![0.0f32; FEATURES];
    for x in &xs {
        for (mean, x) in mean.iter_mut().zip(*x) {
            *mean += x / n;
        }
    }
    for x in &xs {
        for ((std, mean), x) in std.iter_mut().zip(&mean).zip(*x) {
            *std += (x - mean).powi(2) / n;
        }
    }
    for s in &mut std {
        *s = s.sqrt().max(1e-3);
    }
    let z: Vec<Vec<f32>> = xs
        .iter()
        .map(|x| {
            x.iter()
                .zip(&mean)
                .zip(&std)
                .map(|((x, mean), std)| (x - mean) / std)
                .collect()
        })
        .collect();
    let (wp, wn) = (0.5 / pos.len().max(1) as f32, 0.5 / neg.len().max(1) as f32);
    let (mut w, mut b) = (vec![0.0f32; FEATURES], 0.0f32);
    let (lr, l2) = (0.5, 1e-2);
    let mut gw = vec![0.0f32; FEATURES];
    for _ in 0..400 {
        gw.fill(0.0);
        let mut gb = 0.0;
        for (k, x) in z.iter().enumerate() {
            let positive = k < pos.len();
            let s = b + w.iter().zip(x).map(|(w, x)| w * x).sum::<f32>();
            let p = 1.0 / (1.0 + (-s).exp());
            let y = if positive { 1.0 } else { 0.0 };
            let g = (p - y) * if positive { wp } else { wn };
            gb += g;
            for (gw, x) in gw.iter_mut().zip(x) {
                *gw += g * x;
            }
        }
        for (w, gw) in w.iter_mut().zip(&gw) {
            *w -= lr * (gw + l2 * *w);
        }
        b -= lr * gb;
    }
    Head { mean, std, w, b }
}

/// Writes `head.bin` (mode 0600) through `head.bin.tmp` and a rename.
pub fn save_head(dir: &Path, head: &Head) -> io::Result<()> {
    write_private_atomic(&dir.join(HEAD_FILE), &head.to_bytes())
}

/// The trained head in `dir`, if there is one.
pub fn load_head(dir: &Path) -> io::Result<Option<Head>> {
    match std::fs::read(dir.join(HEAD_FILE)) {
        Ok(bytes) => Head::from_bytes(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Where training is, for the menu's status line.
#[derive(Debug, Clone, PartialEq)]
pub enum TrainProgress {
    Saying { heard: usize, of: usize },
    Talking { secs_left: u32 },
    Training,
    Done(Head),
    Failed(String),
}

impl TrainProgress {
    pub fn status(&self) -> String {
        match self {
            Self::Saying { heard, of } => format!("Say Hey Jarvis — {heard} of {of}"),
            Self::Talking { secs_left } => format!("Now talk normally — {secs_left} s"),
            Self::Training => "Training…".into(),
            Self::Done(_) => "Hey Jarvis trained".into(),
            Self::Failed(reason) => reason.clone(),
        }
    }
}

/// A loud stretch, in seconds.
#[derive(Debug, Clone, Copy)]
struct Burst {
    start: f32,
    end: f32,
    is_try: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Saying,
    /// Steps after `negatives_from` are all negatives.
    Talking {
        since_step: u64,
        negatives_from: f32,
    },
    Training,
    Failed(String),
}

/// One training session: labels the 80 ms steps as they come.
#[derive(Debug)]
pub struct Enrolment {
    tries: usize,
    talk_s: u32,
    max_saying_steps: u64,
    step: u64,
    phase: Phase,
    /// (first loud step, last loud step) of the burst going on.
    open: Option<(u64, u64)>,
    bursts: Vec<Burst>,
    heard: usize,
    /// Steps not labeled yet: (time, features).
    pending: VecDeque<(f32, Vec<f32>)>,
    pos: Vec<Vec<f32>>,
    neg: Vec<Vec<f32>>,
}

impl Enrolment {
    pub fn new(tries: usize, talk_s: u32, max_saying_s: u32) -> Self {
        Self {
            tries,
            talk_s,
            max_saying_steps: (max_saying_s as f32 / HOP_S).round() as u64,
            step: 0,
            phase: Phase::Saying,
            open: None,
            bursts: Vec::new(),
            heard: 0,
            pending: VecDeque::new(),
            pos: Vec::new(),
            neg: Vec::new(),
        }
    }

    fn time(step: u64) -> f32 {
        step as f32 * HOP_S
    }

    /// One 80 ms step: its level in dBFS and the feature window ending with it.
    pub fn push(&mut self, level_db: f32, features: &[f32]) -> TrainProgress {
        if matches!(self.phase, Phase::Training | Phase::Failed(_)) {
            return self.progress();
        }
        self.step += 1;
        let t = Self::time(self.step);
        self.pending.push_back((t, features.to_vec()));
        if self.phase == Phase::Saying {
            self.track_level(level_db);
        }
        self.label_ready();
        match self.phase {
            Phase::Saying if self.step >= self.max_saying_steps => {
                self.phase = Phase::Failed(format!(
                    "Heard only {} of {} — try again",
                    self.heard, self.tries
                ));
            }
            Phase::Talking { since_step, .. }
                if Self::time(self.step - since_step) >= self.talk_s as f32 - EPS =>
            {
                self.phase = Phase::Training;
            }
            _ => {}
        }
        self.progress()
    }

    fn track_level(&mut self, level_db: f32) {
        let step = self.step;
        if level_db > LOUD_DB {
            self.open = Some(match self.open {
                Some((first, _)) => (first, step),
                None => (step, step),
            });
            return;
        }
        let Some((first, last)) = self.open else {
            return;
        };
        if Self::time(step - last) <= MERGE_GAP_S + EPS {
            return;
        }
        self.open = None;
        let (start, end) = (Self::time(first - 1), Self::time(last));
        let length = end - start;
        let is_try = (TRY_MIN_S - EPS..=TRY_MAX_S + EPS).contains(&length);
        self.bursts.push(Burst { start, end, is_try });
        if is_try {
            self.heard += 1;
            if self.heard >= self.tries {
                self.phase = Phase::Talking {
                    since_step: step,
                    negatives_from: end + NEG_AFTER_S,
                };
            }
        }
    }

    /// Labels the pending steps no future burst can change.
    fn label_ready(&mut self) {
        let now = Self::time(self.step);
        while let Some((t, _)) = self.pending.front() {
            let t = *t;
            let ready = match self.phase {
                Phase::Saying => {
                    // A burst starting within 0.5 s after `t` has begun by now, and an open
                    // burst near `t` must end first.
                    now >= t + NEG_BEFORE_S + HOP_S - EPS
                        && self
                            .open
                            .is_none_or(|(first, _)| t < Self::time(first - 1) - NEG_BEFORE_S - EPS)
                }
                _ => true,
            };
            if !ready {
                break;
            }
            let (t, x) = self.pending.pop_front().expect("front exists");
            if let Phase::Talking { negatives_from, .. } = self.phase
                && t > negatives_from + EPS
            {
                self.neg.push(x);
                continue;
            }
            let positive = self
                .bursts
                .iter()
                .any(|b| b.is_try && t >= b.end + POS_FROM_S - EPS && t <= b.end + POS_TO_S + EPS);
            let near = self
                .bursts
                .iter()
                .any(|b| t >= b.start - NEG_BEFORE_S - EPS && t <= b.end + NEG_AFTER_S + EPS);
            if positive {
                self.pos.push(x);
            } else if !near {
                self.neg.push(x);
            }
        }
    }

    fn progress(&self) -> TrainProgress {
        match &self.phase {
            Phase::Saying => TrainProgress::Saying {
                heard: self.heard,
                of: self.tries,
            },
            Phase::Talking { since_step, .. } => {
                let left = self.talk_s as f32 - Self::time(self.step - since_step);
                TrainProgress::Talking {
                    secs_left: (left - EPS).ceil().max(0.0) as u32,
                }
            }
            Phase::Training => TrainProgress::Training,
            Phase::Failed(reason) => TrainProgress::Failed(reason.clone()),
        }
    }

    /// Adds negatives from elsewhere (the shell's synthetic talk).
    pub fn add_negatives(&mut self, windows: Vec<Vec<f32>>) {
        self.neg
            .extend(windows.into_iter().filter(|x| x.len() == FEATURES));
    }

    /// Trains on everything gathered: `Done(head)`, or `Failed` with the reason.
    pub fn finish(&mut self) -> TrainProgress {
        if let Phase::Failed(reason) = &self.phase {
            return TrainProgress::Failed(reason.clone());
        }
        // Steps still pending are labeled with what is known now.
        let pending: Vec<_> = self.pending.drain(..).collect();
        for (t, x) in pending {
            let positive = self
                .bursts
                .iter()
                .any(|b| b.is_try && t >= b.end + POS_FROM_S - EPS && t <= b.end + POS_TO_S + EPS);
            if positive {
                self.pos.push(x);
            }
        }
        if self.pos.is_empty() || self.neg.is_empty() {
            return TrainProgress::Failed("Not enough sound to learn from — try again".into());
        }
        TrainProgress::Done(train(&self.pos, &self.neg))
    }

    #[cfg(test)]
    fn positive_times(&self) -> Vec<f32> {
        self.pos.iter().map(|x| x[0]).collect()
    }

    #[cfg(test)]
    fn negative_times(&self) -> Vec<f32> {
        self.neg.iter().map(|x| x[0]).collect()
    }
}

#[cfg(test)]
mod tests;
