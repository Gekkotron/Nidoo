//! "Hey Jarvis": openWakeWord's stock classifier at 0.5, or the personal head
//! (`head.bin`, from Train Hey Jarvis) at 0.9; both thresholds can be changed in
//! `settings.json` (`wake_threshold`, `personal_threshold`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nidoo_satellite::WakeDetector;
use nidoo_satellite::wake_head::{Head, load_head};
use ort::session::Session;
use ort::value::Tensor;

use crate::features::{EMB_WINDOW, Frontend, session};
use crate::models::{CLASSIFIER, Models};

/// Scores skipped after a reset (openWakeWord skips its first 5).
const WARMUP: usize = 5;
/// Steps ignored after a detection (2 s).
const REFRACTORY: usize = 25;

pub struct Thresholds {
    pub stock: f32,
    pub personal: f32,
}

struct Engine {
    front: Frontend,
    cls: Session,
    head: Option<Head>,
}

pub struct Detector {
    models: Arc<Models>,
    store_dir: PathBuf,
    thresholds: Thresholds,
    /// Set by the trainer when it saved a new head.
    head_changed: Arc<AtomicBool>,
    engine: Option<Engine>,
    load_failed: bool,
    warmup: usize,
    refractory: usize,
}

impl Detector {
    pub fn new(
        models: Arc<Models>,
        store_dir: PathBuf,
        thresholds: Thresholds,
        head_changed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            models,
            store_dir,
            thresholds,
            head_changed,
            engine: None,
            load_failed: false,
            warmup: WARMUP,
            refractory: 0,
        }
    }

    fn head(&self) -> Option<Head> {
        match load_head(&self.store_dir) {
            Ok(head) => head,
            Err(e) => {
                tracing::warn!(error = %e, "ignoring head.bin; using the stock model");
                None
            }
        }
    }

    /// Loads the engine once the models are there, and a new head when one was saved.
    fn prepare(&mut self) {
        if self.engine.is_none() && !self.load_failed && self.models.ready() {
            let built = Frontend::new(&self.models.dir)
                .and_then(|front| Ok((front, session(&self.models.dir.join(CLASSIFIER))?)));
            match built {
                Ok((front, cls)) => {
                    let head = self.head();
                    tracing::info!(personal = head.is_some(), "wake word ready");
                    self.engine = Some(Engine { front, cls, head });
                }
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "loading the wake word");
                    self.load_failed = true;
                }
            }
        }
        if self.head_changed.swap(false, Ordering::AcqRel) {
            let head = self.head();
            if let Some(engine) = self.engine.as_mut() {
                tracing::info!(personal = head.is_some(), "wake head reloaded");
                engine.head = head;
            }
        }
    }

    fn score(engine: &mut Engine, features: Vec<f32>) -> anyhow::Result<f32> {
        if let Some(head) = &engine.head {
            return Ok(head.prob(&features));
        }
        let name = engine.cls.inputs[0].name.clone();
        let out = engine
            .cls
            .run(ort::inputs![name => Tensor::from_array(([1usize, EMB_WINDOW, 96], features))?])?;
        let (_, p) = out[0].try_extract_tensor::<f32>()?;
        Ok(p[0])
    }
}

impl WakeDetector for Detector {
    fn push(&mut self, samples_16k: &[i16]) -> bool {
        self.prepare();
        let Some(engine) = self.engine.as_mut() else {
            return false;
        };
        let threshold = if engine.head.is_some() {
            self.thresholds.personal
        } else {
            self.thresholds.stock
        };
        let steps = match engine.front.push(samples_16k) {
            Ok(steps) => steps,
            Err(e) => {
                tracing::warn!(error = %e, "wake word");
                return false;
            }
        };
        let mut heard = false;
        for features in steps.into_iter().filter_map(|s| s.features) {
            let score = match Self::score(engine, features) {
                Ok(score) => score,
                Err(e) => {
                    tracing::warn!(error = %e, "wake word");
                    continue;
                }
            };
            if self.warmup > 0 {
                self.warmup -= 1;
            } else if self.refractory > 0 {
                self.refractory -= 1;
            } else if score >= threshold {
                tracing::info!(score, "wake word");
                self.refractory = REFRACTORY;
                heard = true;
            }
        }
        heard
    }

    fn reset(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.front.reset();
        }
        self.warmup = WARMUP;
        self.refractory = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::train::spoken;
    use crate::train::tests::{english_voice, models, scratch};

    fn hears(d: &mut Detector, clip: &[i16]) -> bool {
        let mut audio = vec![0i16; 16_000];
        audio.extend_from_slice(clip);
        audio.extend(std::iter::repeat_n(0, 16_000));
        audio.chunks(320).fold(false, |heard, c| d.push(c) | heard)
    }

    #[test]
    #[ignore = "macOS with the downloaded models"]
    fn stock_model_hears_an_english_hey_jarvis_and_not_talk() {
        let dir = scratch("wake");
        let mut d = Detector::new(
            models(),
            dir.clone(),
            Thresholds {
                stock: 0.5,
                personal: 0.9,
            },
            Arc::new(AtomicBool::new(false)),
        );
        let voice = english_voice();
        assert!(hears(
            &mut d,
            &spoken(&voice, "Hey Jarvis, turn on the kitchen light").unwrap()
        ));
        d.reset();
        assert!(!hears(
            &mut d,
            &spoken(&voice, "The meeting was moved to Thursday morning.").unwrap()
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
