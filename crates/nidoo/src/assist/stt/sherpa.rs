//! The recognizer and detector on sherpa-onnx: Parakeet TDT 0.6B v3 (int8, NeMo transducer,
//! greedy search) and Silero VAD, both on the CPU.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};

use super::engine::{EngineLoader, Engines, SAMPLE_RATE, Transcriber, Vad, VadFactory};

/// Seconds of audio one detector buffers.
const VAD_BUFFER_S: f32 = 30.0;

/// Loads Parakeet and Silero from the model files.
pub struct SherpaLoader;

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl EngineLoader for SherpaLoader {
    fn built(&self) -> bool {
        true
    }

    fn load(&self, model_dir: &Path, vad: &Path, threads: u8) -> Result<Engines, String> {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(path(&model_dir.join("encoder.int8.onnx"))),
            decoder: Some(path(&model_dir.join("decoder.int8.onnx"))),
            joiner: Some(path(&model_dir.join("joiner.int8.onnx"))),
        };
        cfg.model_config.tokens = Some(path(&model_dir.join("tokens.txt")));
        cfg.model_config.num_threads = i32::from(threads);
        cfg.model_config.model_type = Some("nemo_transducer".into());
        cfg.model_config.provider = Some("cpu".into());
        cfg.decoding_method = Some("greedy_search".into());
        let recognizer = OfflineRecognizer::create(&cfg)
            .ok_or_else(|| format!("cannot load the model in {}", model_dir.display()))?;
        let vads = SileroVads {
            model: vad.to_owned(),
        };
        // One detector now, so a broken VAD file shows at load time.
        vads.create()
            .ok_or_else(|| format!("cannot load the VAD {}", vad.display()))?;
        Ok(Engines {
            transcriber: Arc::new(SherpaTranscriber(Mutex::new(recognizer))),
            vads: Arc::new(vads),
        })
    }
}

/// One phrase at a time: the recognizer already uses every thread it was given.
struct SherpaTranscriber(Mutex<OfflineRecognizer>);

impl Transcriber for SherpaTranscriber {
    fn transcribe(&self, samples: &[f32]) -> String {
        let recognizer = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let stream = recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, samples);
        recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default()
    }
}

struct SileroVads {
    model: PathBuf,
}

impl SileroVads {
    fn create(&self) -> Option<VoiceActivityDetector> {
        let cfg = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(path(&self.model)),
                threshold: 0.5,
                min_silence_duration: 0.5,
                min_speech_duration: 0.25,
                window_size: 512,
                max_speech_duration: 10.0,
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
            ..Default::default()
        };
        VoiceActivityDetector::create(&cfg, VAD_BUFFER_S)
    }
}

impl VadFactory for SileroVads {
    fn new_vad(&self) -> Box<dyn Vad> {
        match self.create() {
            Some(vad) => Box::new(SileroVad(vad)),
            None => {
                tracing::warn!("cannot create a VAD; the whole window becomes one phrase");
                Box::new(WholeWindow::default())
            }
        }
    }
}

struct SileroVad(VoiceActivityDetector);

impl Vad for SileroVad {
    fn accept(&mut self, samples: &[f32]) {
        self.0.accept_waveform(samples);
    }

    fn pop_segment(&mut self) -> Option<Vec<f32>> {
        let segment = self.0.front()?.samples().to_vec();
        self.0.pop();
        Some(segment)
    }

    fn flush(&mut self) {
        self.0.flush();
    }

    fn reset(&mut self) {
        self.0.clear();
        self.0.reset();
    }
}

/// Without a detector: everything heard, once the window ends.
#[derive(Default)]
struct WholeWindow {
    heard: Vec<f32>,
    done: Option<Vec<f32>>,
}

impl Vad for WholeWindow {
    fn accept(&mut self, samples: &[f32]) {
        self.heard.extend_from_slice(samples);
    }

    fn pop_segment(&mut self) -> Option<Vec<f32>> {
        self.done.take()
    }

    fn flush(&mut self) {
        if !self.heard.is_empty() {
            self.done = Some(std::mem::take(&mut self.heard));
        }
    }

    fn reset(&mut self) {
        *self = WholeWindow::default();
    }
}
