//! openWakeWord's front end, streaming exactly as openWakeWord does (spike
//! `satellite/mac/spike/src/bin/oww.rs`): per 1280 new samples (80 ms), a
//! melspectrogram of the last 1760 samples (int16 values as f32), frames scaled
//! `x/10 + 2`, one embedding from the last 76 mel frames; the 16 latest embeddings
//! are the 16×96 feature window the classifier (or the personal head) scores.

use std::collections::VecDeque;
use std::path::Path;

use anyhow::Context;
use ort::session::Session;
use ort::value::Tensor;

pub const STEP: usize = 1280;
const CONTEXT: usize = 480;
const MEL_WINDOW: usize = 76;
pub const EMB_WINDOW: usize = 16;

pub fn session(path: &Path) -> anyhow::Result<Session> {
    Session::builder()?
        .with_intra_threads(1)?
        .commit_from_file(path)
        .with_context(|| path.display().to_string())
}

/// One 80 ms step.
pub struct Step {
    /// The step's level in dBFS.
    pub level_db: f32,
    /// The 16×96 window ending with this step, once 16 embeddings exist.
    pub features: Option<Vec<f32>>,
}

pub struct Frontend {
    mel: Session,
    emb: Session,
    raw: VecDeque<f32>,
    pending: usize,
    sq: f64,
    mels: VecDeque<[f32; 32]>,
    embs: VecDeque<Vec<f32>>,
}

impl Frontend {
    /// Loads `melspectrogram.onnx` and `embedding_model.onnx` from `models`.
    pub fn new(models: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            mel: session(&models.join("melspectrogram.onnx"))?,
            emb: session(&models.join("embedding_model.onnx"))?,
            raw: VecDeque::with_capacity(STEP + CONTEXT + 1),
            pending: 0,
            sq: 0.0,
            mels: std::iter::repeat_n([1.0; 32], MEL_WINDOW).collect(),
            embs: VecDeque::new(),
        })
    }

    /// Forgets the embeddings (after a voice window), so old words cannot score again.
    pub fn reset(&mut self) {
        self.embs.clear();
    }

    /// Forgets everything (a new clip).
    pub fn clear(&mut self) {
        self.raw.clear();
        self.pending = 0;
        self.sq = 0.0;
        self.mels = std::iter::repeat_n([1.0; 32], MEL_WINDOW).collect();
        self.embs.clear();
    }

    pub fn push(&mut self, samples: &[i16]) -> anyhow::Result<Vec<Step>> {
        let mut steps = Vec::new();
        for &s in samples {
            let x = f32::from(s);
            self.raw.push_back(x);
            if self.raw.len() > STEP + CONTEXT {
                self.raw.pop_front();
            }
            self.sq += f64::from(x / 32768.0).powi(2);
            self.pending += 1;
            if self.pending == STEP {
                let level_db = (10.0 * (self.sq / STEP as f64).max(1e-12).log10()) as f32;
                self.pending = 0;
                self.sq = 0.0;
                let features = self.step()?;
                steps.push(Step { level_db, features });
            }
        }
        Ok(steps)
    }

    fn step(&mut self) -> anyhow::Result<Option<Vec<f32>>> {
        let input: Vec<f32> = self.raw.iter().copied().collect();
        let n = input.len();
        let name = self.mel.inputs[0].name.clone();
        let out = self
            .mel
            .run(ort::inputs![name => Tensor::from_array(([1usize, n], input))?])?;
        let (_, mel) = out[0].try_extract_tensor::<f32>()?;
        for f in mel.as_chunks::<32>().0 {
            let mut frame = [0.0; 32];
            for (d, s) in frame.iter_mut().zip(f) {
                *d = s / 10.0 + 2.0;
            }
            self.mels.push_back(frame);
        }
        while self.mels.len() > MEL_WINDOW {
            self.mels.pop_front();
        }
        let window: Vec<f32> = self.mels.iter().flatten().copied().collect();
        let name = self.emb.inputs[0].name.clone();
        let out = self.emb.run(
            ort::inputs![name => Tensor::from_array(([1usize, MEL_WINDOW, 32, 1], window))?],
        )?;
        let (_, e) = out[0].try_extract_tensor::<f32>()?;
        self.embs.push_back(e.to_vec());
        while self.embs.len() > EMB_WINDOW {
            self.embs.pop_front();
        }
        if self.embs.len() < EMB_WINDOW {
            return Ok(None);
        }
        Ok(Some(self.embs.iter().flatten().copied().collect()))
    }
}
