//! The recognizer and the end-of-phrase detector, behind traits so the exchange logic runs in
//! tests without the 650 MB model. The sherpa-onnx versions are in `sherpa.rs` (feature `stt`).

use std::path::Path;
use std::sync::Arc;

/// Samples per second of every input.
pub const SAMPLE_RATE: u32 = 16_000;

/// Turns one phrase (16 kHz mono, -1.0 to 1.0) into text.
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, samples: &[f32]) -> String;
}

/// Finds phrases in a stream of samples, one detector per exchange.
pub trait Vad: Send + Sync {
    fn accept(&mut self, samples: &[f32]);
    /// The next finished phrase (speech followed by enough silence), if any.
    fn pop_segment(&mut self) -> Option<Vec<f32>>;
    /// Ends the stream: what is still being heard becomes a phrase.
    fn flush(&mut self);
    /// Forgets everything (a new window).
    fn reset(&mut self);
}

/// Makes a fresh [`Vad`].
pub trait VadFactory: Send + Sync {
    fn new_vad(&self) -> Box<dyn Vad>;
}

/// A loaded recognizer and its detector factory.
#[derive(Clone)]
pub struct Engines {
    pub transcriber: Arc<dyn Transcriber>,
    pub vads: Arc<dyn VadFactory>,
}

/// Loads the engines from the model files.
pub trait EngineLoader: Send + Sync {
    /// This hub can recognise speech at all.
    fn built(&self) -> bool;
    fn load(&self, model_dir: &Path, vad: &Path, threads: u8) -> Result<Engines, String>;
}

/// The loader of a hub built without speech input.
pub struct NoSttLoader;

impl EngineLoader for NoSttLoader {
    fn built(&self) -> bool {
        false
    }

    fn load(&self, _: &Path, _: &Path, _: u8) -> Result<Engines, String> {
        Err("built without speech input".to_owned())
    }
}

/// The loader for this build: sherpa-onnx with the `stt` feature, nothing without it.
pub fn default_loader() -> Arc<dyn EngineLoader> {
    #[cfg(feature = "stt")]
    return Arc::new(super::sherpa::SherpaLoader);
    #[cfg(not(feature = "stt"))]
    return Arc::new(NoSttLoader);
}

/// The phrase without the wake word the recognizer sometimes keeps ("Jarvis, allume…").
pub fn strip_wake_word(text: &str) -> String {
    let text = text.trim();
    let after_greeting = ["hey", "hé", "he", "eh", "et"]
        .iter()
        .find_map(|w| strip_word(text, w))
        .map_or(text, skip_separators);
    match strip_word(after_greeting, "jarvis") {
        Some(rest) => skip_separators(rest).trim_end().to_owned(),
        None => text.to_owned(),
    }
}

/// `s` after `word` (any case) when `s` starts with that whole word.
fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let mut chars = s.char_indices();
    for w in word.chars() {
        let (_, c) = chars.next()?;
        if !c.to_lowercase().eq(w.to_lowercase()) {
            return None;
        }
    }
    let rest = chars.next().map_or("", |(i, _)| &s[i..]);
    (!rest.starts_with(char::is_alphanumeric)).then_some(rest)
}

fn skip_separators(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_whitespace() || ",.!?;:…-".contains(c))
}

/// Fakes for the exchange and service tests.
#[cfg(test)]
pub mod fakes {
    use super::*;

    /// Always hears `.0`.
    pub struct FakeTranscriber(pub String);

    impl Transcriber for FakeTranscriber {
        fn transcribe(&self, _: &[f32]) -> String {
            self.0.clone()
        }
    }

    /// Quiet below this level.
    const QUIET: f32 = 0.01;
    /// 0.5 s of quiet ends a phrase.
    const END_SILENCE: usize = 8_000;

    /// Speech is any sample louder than [`QUIET`]; 0.5 s of quiet after speech ends a phrase.
    #[derive(Default)]
    pub struct FakeVad {
        speech: Vec<f32>,
        quiet: usize,
        done: std::collections::VecDeque<Vec<f32>>,
    }

    impl Vad for FakeVad {
        fn accept(&mut self, samples: &[f32]) {
            for &s in samples {
                if s.abs() > QUIET {
                    self.speech.push(s);
                    self.quiet = 0;
                } else if !self.speech.is_empty() {
                    self.quiet += 1;
                    if self.quiet >= END_SILENCE {
                        self.done.push_back(std::mem::take(&mut self.speech));
                        self.quiet = 0;
                    }
                }
            }
        }

        fn pop_segment(&mut self) -> Option<Vec<f32>> {
            self.done.pop_front()
        }

        fn flush(&mut self) {
            if !self.speech.is_empty() {
                self.done.push_back(std::mem::take(&mut self.speech));
            }
            self.quiet = 0;
        }

        fn reset(&mut self) {
            *self = FakeVad::default();
        }
    }

    pub struct FakeVads;

    impl VadFactory for FakeVads {
        fn new_vad(&self) -> Box<dyn Vad> {
            Box::new(FakeVad::default())
        }
    }

    pub fn engines(heard: &str) -> Engines {
        Engines {
            transcriber: Arc::new(FakeTranscriber(heard.to_owned())),
            vads: Arc::new(FakeVads),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_wake_word_drops_a_leading_jarvis() {
        assert_eq!(
            strip_wake_word("Jarvis, allume la lumière."),
            "allume la lumière."
        );
        assert_eq!(strip_wake_word("hey jarvis allume"), "allume");
        assert_eq!(strip_wake_word("Hé Jarvis ! Éteins tout"), "Éteins tout");
        assert_eq!(strip_wake_word("  Et Jarvis... allume"), "allume");
        assert_eq!(strip_wake_word("Jarvis."), "");
    }

    #[test]
    fn strip_wake_word_keeps_everything_else() {
        assert_eq!(strip_wake_word("Allume Jarvis"), "Allume Jarvis");
        assert_eq!(strip_wake_word("Jarvisien"), "Jarvisien");
        assert_eq!(
            strip_wake_word(" Allume la lumière. "),
            "Allume la lumière."
        );
        assert_eq!(strip_wake_word("Hey"), "Hey");
    }

    #[test]
    fn fake_vad_finds_a_phrase_after_half_a_second_of_quiet() {
        let mut vad = fakes::FakeVad::default();
        vad.accept(&[0.5; 1_600]);
        vad.accept(&[0.0; 7_999]);
        assert!(vad.pop_segment().is_none());
        vad.accept(&[0.0; 1]);
        assert_eq!(vad.pop_segment().map(|s| s.len()), Some(1_600));
    }

    #[test]
    fn the_no_stt_loader_reports_not_built() {
        let loader = NoSttLoader;
        assert!(!loader.built());
        assert!(loader.load(Path::new("x"), Path::new("y"), 4).is_err());
        assert_eq!(default_loader().built(), cfg!(feature = "stt"));
    }
}
