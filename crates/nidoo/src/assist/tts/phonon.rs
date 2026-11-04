//! The Phonon speech engine (`ptts`) behind the [`Speaker`] trait.

use std::path::Path;

use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::{Lang, Normalize};
use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};

use super::engine::{Speaker, SpeakerLoader};
use super::store::quant_of;

/// Loads a Phonon model from a folder, on the CPU. Text is normalised per request, in the
/// request's language, so one loaded model serves every language it speaks.
pub struct PhononLoader;

impl SpeakerLoader for PhononLoader {
    fn load(&self, dir: &Path, weights: &str) -> Result<Box<dyn Speaker>, String> {
        let quant = match quant_of(weights) {
            Some("q8") => Quant::Q80,
            Some("q4k") => Quant::Q4k,
            Some("f32") => Quant::F32,
            _ => return Err(format!("unknown weight file: {weights}")),
        };
        let checkpoint = Checkpoint::resolve(
            dir,
            ResolveOptions {
                quant,
                weights: Some(weights),
            },
        )
        .map_err(|e| e.to_string())?;
        let mut synth = checkpoint
            .builder(Normalize::OFF)
            .device(DeviceKind::Cpu)
            .build()
            .map_err(|e| e.to_string())?;
        checkpoint.register_voices(&mut synth);
        Ok(Box::new(PhononSpeaker { synth }))
    }
}

struct PhononSpeaker {
    synth: Synth,
}

impl Speaker for PhononSpeaker {
    fn sample_rate(&self) -> u32 {
        self.synth.sample_rate()
    }

    fn languages(&self) -> Vec<String> {
        self.synth
            .languages()
            .iter()
            .map(|l| l.to_string())
            .collect()
    }

    fn voices(&self) -> Vec<String> {
        self.synth.voices().iter().map(|v| v.to_string()).collect()
    }

    fn add_voice(&mut self, name: &str, path: &Path) -> Result<(), String> {
        self.synth
            .add_voice_file(name, path)
            .map_err(|e| e.to_string())
    }

    fn speak(
        &mut self,
        text: &str,
        language: Option<&str>,
        voice: Option<&str>,
        chunk: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        // `pt-br` normalises as `pt`; a language ptts does not know speaks the text as written.
        let lang = language
            .and_then(|l| l.split('-').next())
            .and_then(|l| l.parse::<Lang>().ok());
        let text = match lang {
            Some(l) => Normalize::for_lang(l).apply(text),
            None => std::borrow::Cow::Borrowed(text),
        };
        let options = match voice {
            Some(v) => SpeechOptions::default().voice(v),
            None => SpeechOptions::default(),
        };
        let stream = self
            .synth
            .stream_with(&text, &options)
            .map_err(|e| e.to_string())?;
        for piece in stream {
            let samples = piece.map_err(|e| e.to_string())?;
            if !chunk(&samples) {
                break;
            }
        }
        Ok(())
    }
}
