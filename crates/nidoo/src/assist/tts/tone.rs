//! Fake synthesisers for tests: a tone per word, and one that blocks until released.

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use super::engine::{Speaker, SpeakerLoader};

const RATE: u32 = 24_000;
/// Samples in one chunk: each word is two chunks, 0.1 s in all.
const CHUNK: usize = 1_200;

fn tone() -> Vec<f32> {
    (0..CHUNK)
        .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin())
        .collect()
}

/// A loader whose speakers say each word as 0.1 s of a 440 Hz tone, in two chunks.
///
/// A model folder holding a file named `fail` does not load.
pub struct ToneLoader;

impl SpeakerLoader for ToneLoader {
    fn load(&self, dir: &Path, _weights: &str) -> Result<Box<dyn Speaker>, String> {
        if dir.join("fail").exists() {
            return Err("broken model".into());
        }
        Ok(Box::new(ToneSpeaker {
            voices: vec!["default".into()],
            gate: None,
        }))
    }
}

struct ToneSpeaker {
    voices: Vec<String>,
    gate: Option<Arc<Gate>>,
}

impl Speaker for ToneSpeaker {
    fn sample_rate(&self) -> u32 {
        RATE
    }

    fn languages(&self) -> Vec<String> {
        vec!["fr".into()]
    }

    fn voices(&self) -> Vec<String> {
        self.voices.clone()
    }

    fn add_voice(&mut self, name: &str, _path: &Path) -> Result<(), String> {
        self.voices.push(name.to_string());
        Ok(())
    }

    fn speak(
        &mut self,
        text: &str,
        _voice: Option<&str>,
        chunk: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        if let Some(gate) = &self.gate {
            gate.wait_for_release();
        }
        let samples = tone();
        for _ in text.split_whitespace() {
            for _ in 0..2 {
                if !chunk(&samples) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct GateState {
    started: usize,
    permits: usize,
    all: bool,
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    cond: Condvar,
}

impl Gate {
    fn wait_for_release(&self) {
        let mut s = self.state.lock().unwrap();
        s.started += 1;
        self.cond.notify_all();
        while !s.all && s.permits == 0 {
            s = self.cond.wait(s).unwrap();
        }
        if !s.all {
            s.permits -= 1;
        }
    }
}

/// A loader like [`ToneLoader`] whose `speak` blocks until [`Release`] lets it go.
pub struct BlockingLoader {
    gate: Arc<Gate>,
}

/// The test's handle on a [`BlockingLoader`].
#[derive(Clone)]
pub struct Release {
    gate: Arc<Gate>,
}

impl BlockingLoader {
    /// A blocking loader and the handle that releases its speeches.
    pub fn new() -> (BlockingLoader, Release) {
        let gate = Arc::new(Gate::default());
        (BlockingLoader { gate: gate.clone() }, Release { gate })
    }
}

impl SpeakerLoader for BlockingLoader {
    fn load(&self, dir: &Path, weights: &str) -> Result<Box<dyn Speaker>, String> {
        ToneLoader.load(dir, weights)?;
        Ok(Box::new(ToneSpeaker {
            voices: vec!["default".into()],
            gate: Some(self.gate.clone()),
        }))
    }
}

impl Release {
    /// Let one blocked (or the next) speech finish.
    pub fn release_one(&self) {
        let mut s = self.gate.state.lock().unwrap();
        s.permits += 1;
        self.gate.cond.notify_all();
    }

    /// Let every speech finish, now and later.
    pub fn release_all(&self) {
        let mut s = self.gate.state.lock().unwrap();
        s.all = true;
        self.gate.cond.notify_all();
    }

    /// Wait (up to 2 s) until `n` speeches have started; false on timeout.
    pub fn wait_started(&self, n: usize) -> bool {
        let deadline = std::time::Duration::from_secs(2);
        let s = self.gate.state.lock().unwrap();
        let (s, _) = self
            .gate
            .cond
            .wait_timeout_while(s, deadline, |s| s.started < n)
            .unwrap();
        s.started >= n
    }
}
