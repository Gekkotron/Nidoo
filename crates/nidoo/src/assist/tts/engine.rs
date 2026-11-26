//! The speech engine thread: loads one model at a time and speaks one request at a time.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};

use nidoo_api::EngineState;

use super::TtsError;

/// Speech requests that may wait while one runs; one more is [`TtsError::Busy`].
pub const QUEUE_LEN: usize = 4;

/// A loaded speech model. It lives on the engine thread, so it need not be `Send`.
pub trait Speaker {
    /// Samples per second of the audio it makes.
    fn sample_rate(&self) -> u32;
    /// The languages it speaks.
    fn languages(&self) -> Vec<String>;
    /// The voices it knows.
    fn voices(&self) -> Vec<String>;
    /// Register a voice file under a name.
    fn add_voice(&mut self, name: &str, path: &Path) -> Result<(), String>;
    /// Speak `text` in `language` (a normalised code; `None` speaks the text as written),
    /// handing each audio chunk to `chunk`; `chunk` returning `false` stops early.
    fn speak(
        &mut self,
        text: &str,
        language: Option<&str>,
        voice: Option<&str>,
        chunk: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<(), String>;
}

/// Builds a [`Speaker`] from a model folder. Called on the engine thread.
pub trait SpeakerLoader: Send + Sync {
    /// Load the model whose weight file is `weights` inside `dir`.
    fn load(&self, dir: &Path, weights: &str) -> Result<Box<dyn Speaker>, String>;
}

/// The loader of a hub built without speech.
pub struct NoSpeechLoader;

impl SpeakerLoader for NoSpeechLoader {
    fn load(&self, _dir: &Path, _weights: &str) -> Result<Box<dyn Speaker>, String> {
        Err("this hub was built without speech".into())
    }
}

/// Receives a speech: `Some(Ok(chunk))`s, then `None` at the end or `Some(Err(_))`.
/// Returning `false` stops the speech.
pub type Sink = Box<dyn FnMut(SpeechItem) -> bool + Send>;

/// One step of a speech: a chunk, a failure, or `None` at the end.
pub type SpeechItem = Option<Result<Vec<f32>, String>>;

enum Msg {
    Load {
        generation: u64,
        model: String,
        dir: PathBuf,
        weights: String,
    },
    Unload,
    AddVoice {
        generation: u64,
        name: String,
        path: PathBuf,
    },
    Speak {
        text: String,
        language: Option<String>,
        voice: Option<String>,
        sink: Sink,
    },
}

struct Shared {
    state: EngineState,
    generation: u64,
    voices: Vec<String>,
    languages: Vec<String>,
    sample_rate: Option<u32>,
}

/// Handle on the engine thread.
pub struct Engine {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Shared>>,
    waiting: Arc<AtomicUsize>,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// The message shown when the engine thread is gone.
const THREAD_STOPPED: &str = "the speech thread stopped";

/// The model named by a state, if any.
fn model_of(state: &EngineState) -> Option<String> {
    match state {
        EngineState::NoModel => None,
        EngineState::Loading { model } | EngineState::Ready { model } => Some(model.clone()),
        EngineState::Error { model, .. } => model.clone(),
    }
}

/// Marks the engine as stopped: nothing is loaded and speaking is refused.
fn mark_stopped(shared: &Mutex<Shared>, model: Option<String>) {
    let mut s = lock(shared);
    s.generation += 1;
    s.voices.clear();
    s.languages.clear();
    s.sample_rate = None;
    s.state = EngineState::Error {
        model: model.or_else(|| model_of(&s.state)),
        message: THREAD_STOPPED.into(),
    };
}

/// Turns a panic that ends the engine thread into an error state.
struct ThreadGuard {
    shared: Arc<Mutex<Shared>>,
}

impl Drop for ThreadGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            mark_stopped(&self.shared, None);
        }
    }
}

/// Start the engine thread with no model loaded.
pub fn start(loader: Arc<dyn SpeakerLoader>) -> Engine {
    let (tx, rx) = channel();
    let shared = Arc::new(Mutex::new(Shared {
        state: EngineState::NoModel,
        generation: 0,
        voices: Vec::new(),
        languages: Vec::new(),
        sample_rate: None,
    }));
    let waiting = Arc::new(AtomicUsize::new(0));
    let (s, w) = (shared.clone(), waiting.clone());
    if let Err(e) = std::thread::Builder::new()
        .name("tts".into())
        .spawn(move || {
            let _guard = ThreadGuard { shared: s.clone() };
            run(loader, rx, s, w)
        })
    {
        lock(&shared).state = EngineState::Error {
            model: None,
            message: format!("could not start the speech thread: {e}"),
        };
    }
    Engine {
        tx,
        shared,
        waiting,
    }
}

impl Engine {
    /// Where the engine stands.
    pub fn state(&self) -> EngineState {
        lock(&self.shared).state.clone()
    }

    /// Load a model, replacing the current one; a newer load queued behind wins.
    pub fn load(&self, model: &str, dir: PathBuf, weights: String) {
        let generation = {
            let mut s = lock(&self.shared);
            s.generation += 1;
            s.state = EngineState::Loading {
                model: model.to_string(),
            };
            s.voices.clear();
            s.languages.clear();
            s.sample_rate = None;
            s.generation
        };
        let sent = self.tx.send(Msg::Load {
            generation,
            model: model.to_string(),
            dir,
            weights,
        });
        if sent.is_err() {
            mark_stopped(&self.shared, Some(model.to_string()));
        }
    }

    /// Drop the loaded model.
    pub fn unload(&self) {
        {
            let mut s = lock(&self.shared);
            s.generation += 1;
            s.state = EngineState::NoModel;
            s.voices.clear();
            s.languages.clear();
            s.sample_rate = None;
        }
        if self.tx.send(Msg::Unload).is_err() {
            mark_stopped(&self.shared, None);
        }
    }

    /// Register a voice file with the loaded model (ignored when none is loaded).
    pub fn add_voice(&self, name: String, path: PathBuf) {
        let generation = lock(&self.shared).generation;
        let msg = Msg::AddVoice {
            generation,
            name,
            path,
        };
        if self.tx.send(msg).is_err() {
            mark_stopped(&self.shared, None);
        }
    }

    /// The languages of the loaded model.
    pub fn languages(&self) -> Vec<String> {
        lock(&self.shared).languages.clone()
    }

    /// The voices of the loaded model.
    pub fn voices(&self) -> Vec<String> {
        lock(&self.shared).voices.clone()
    }

    /// The sample rate of the loaded model.
    pub fn sample_rate(&self) -> Option<u32> {
        lock(&self.shared).sample_rate
    }

    /// Queue a speech in `language`. The sink is called on the engine thread.
    pub fn speak(
        &self,
        text: String,
        language: Option<String>,
        voice: Option<String>,
        sink: Sink,
    ) -> Result<(), TtsError> {
        match self.state() {
            EngineState::Ready { .. } => {}
            EngineState::NoModel => return Err(TtsError::Conflict("no model is active".into())),
            EngineState::Loading { .. } => {
                return Err(TtsError::Conflict("the model is still loading".into()));
            }
            EngineState::Error { message, .. } => {
                return Err(TtsError::Conflict(format!(
                    "the model failed to load: {message}"
                )));
            }
        }
        let reserved = self
            .waiting
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < QUEUE_LEN).then_some(n + 1)
            });
        if reserved.is_err() {
            return Err(TtsError::Busy);
        }
        let msg = Msg::Speak {
            text,
            language,
            voice,
            sink,
        };
        if self.tx.send(msg).is_err() {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return Err(TtsError::Conflict(
                "the speech thread is not running".into(),
            ));
        }
        Ok(())
    }
}

fn run(
    loader: Arc<dyn SpeakerLoader>,
    rx: Receiver<Msg>,
    shared: Arc<Mutex<Shared>>,
    waiting: Arc<AtomicUsize>,
) {
    let mut speaker: Option<Box<dyn Speaker>> = None;
    let mut pending: VecDeque<Msg> = VecDeque::new();
    loop {
        let msg = match pending.pop_front() {
            Some(m) => m,
            None => match rx.recv() {
                Ok(m) => m,
                Err(_) => return,
            },
        };
        match msg {
            Msg::Load {
                generation,
                model,
                dir,
                weights,
            } => {
                // Last load wins: skip this one when a newer load is already queued.
                pending.extend(rx.try_iter());
                if pending.iter().any(|m| matches!(m, Msg::Load { .. })) {
                    continue;
                }
                speaker = None;
                let result = catch_unwind(AssertUnwindSafe(|| loader.load(&dir, &weights)))
                    .unwrap_or_else(|_| Err("the speech model crashed while loading".into()));
                let mut s = lock(&shared);
                if s.generation != generation {
                    // Unloaded or reloaded meanwhile: the result is stale.
                    continue;
                }
                match result {
                    Ok(sp) => {
                        let info = catch_unwind(AssertUnwindSafe(|| {
                            (sp.voices(), sp.languages(), sp.sample_rate())
                        }));
                        match info {
                            Ok((voices, languages, rate)) => {
                                s.voices = voices;
                                s.languages = languages;
                                s.sample_rate = Some(rate);
                                s.state = EngineState::Ready { model };
                                speaker = Some(sp);
                            }
                            Err(_) => {
                                s.state = EngineState::Error {
                                    model: Some(model),
                                    message: "the speech model crashed while loading".into(),
                                };
                                // Dropping a broken model may panic too.
                                let _ = catch_unwind(AssertUnwindSafe(|| drop(sp)));
                            }
                        }
                    }
                    Err(message) => {
                        s.voices.clear();
                        s.languages.clear();
                        s.sample_rate = None;
                        s.state = EngineState::Error {
                            model: Some(model),
                            message,
                        };
                    }
                }
            }
            Msg::Unload => speaker = None,
            Msg::AddVoice {
                generation,
                name,
                path,
            } => {
                if let Some(sp) = speaker.as_mut() {
                    let added = catch_unwind(AssertUnwindSafe(|| {
                        sp.add_voice(&name, &path).map(|()| sp.voices())
                    }));
                    match added {
                        Ok(Ok(voices)) => {
                            let mut s = lock(&shared);
                            if s.generation == generation {
                                s.voices = voices;
                            }
                        }
                        Ok(Err(e)) => tracing::warn!("could not add the voice {name}: {e}"),
                        Err(_) => {
                            // The model is in an unknown state: drop it and say so.
                            if let Some(broken) = speaker.take() {
                                let _ = catch_unwind(AssertUnwindSafe(|| drop(broken)));
                            }
                            let mut s = lock(&shared);
                            if s.generation == generation {
                                s.generation += 1;
                                s.voices.clear();
                                s.languages.clear();
                                s.sample_rate = None;
                                s.state = EngineState::Error {
                                    model: model_of(&s.state),
                                    message: "the speech model crashed".into(),
                                };
                            }
                        }
                    }
                }
            }
            Msg::Speak {
                text,
                language,
                voice,
                mut sink,
            } => {
                waiting.fetch_sub(1, Ordering::SeqCst);
                let Some(sp) = speaker.as_mut() else {
                    sink(Some(Err("no model is loaded".into())));
                    continue;
                };
                let mut stopped = false;
                let result = catch_unwind(AssertUnwindSafe(|| {
                    sp.speak(&text, language.as_deref(), voice.as_deref(), &mut |c| {
                        let go = sink(Some(Ok(c.to_vec())));
                        stopped |= !go;
                        go
                    })
                }))
                .unwrap_or_else(|_| Err("the speech model crashed".into()));
                match result {
                    Ok(()) if !stopped => {
                        sink(None);
                    }
                    Ok(()) => {}
                    Err(e) => {
                        sink(Some(Err(e)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
