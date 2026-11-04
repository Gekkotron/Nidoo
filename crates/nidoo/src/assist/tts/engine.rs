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
    /// Speak `text`, handing each audio chunk to `chunk`; `chunk` returning `false` stops early.
    fn speak(
        &mut self,
        text: &str,
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

    /// Queue a speech. The sink is called on the engine thread.
    pub fn speak(&self, text: String, voice: Option<String>, sink: Sink) -> Result<(), TtsError> {
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
        if self.tx.send(Msg::Speak { text, voice, sink }).is_err() {
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
                    sp.speak(&text, voice.as_deref(), &mut |c| {
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
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::assist::tts::tone::{BlockingLoader, ToneLoader};

    fn wait_for(engine: &Engine, want: &EngineState) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if &engine.state() == want {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("state is {:?}, wanted {want:?}", engine.state());
    }

    fn ready(model: &str) -> EngineState {
        EngineState::Ready {
            model: model.into(),
        }
    }

    fn channel_sink() -> (Sink, mpsc::Receiver<SpeechItem>) {
        let (tx, rx) = mpsc::channel();
        (
            Box::new(move |item| {
                let _ = tx.send(item);
                true
            }),
            rx,
        )
    }

    fn tone_engine() -> Engine {
        start(Arc::new(ToneLoader))
    }

    #[test]
    fn loads_then_ready() {
        let dir = tempfile::tempdir().unwrap();
        let engine = tone_engine();
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        // Loading is visible straight away (or already Ready on a fast thread).
        assert!(matches!(
            engine.state(),
            EngineState::Loading { .. } | EngineState::Ready { .. }
        ));
        wait_for(&engine, &ready("m"));
        assert_eq!(engine.sample_rate(), Some(24_000));
        assert_eq!(engine.voices(), vec!["default".to_string()]);
        assert_eq!(engine.languages(), vec!["fr".to_string()]);
    }

    #[test]
    fn a_broken_model_is_an_error_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fail"), "").unwrap();
        let engine = tone_engine();
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(
            &engine,
            &EngineState::Error {
                model: Some("m".into()),
                message: "broken model".into(),
            },
        );
    }

    #[test]
    fn last_load_wins() {
        let dir = tempfile::tempdir().unwrap();
        let engine = tone_engine();
        engine.load("a", dir.path().into(), "model.q8.gguf".into());
        engine.load("b", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("b"));
    }

    #[test]
    fn unload_returns_to_no_model() {
        let dir = tempfile::tempdir().unwrap();
        let engine = tone_engine();
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("m"));
        engine.unload();
        wait_for(&engine, &EngineState::NoModel);
        assert_eq!(engine.sample_rate(), None);
        assert!(engine.voices().is_empty());
    }

    #[test]
    fn speak_streams_chunks_then_ends() {
        let dir = tempfile::tempdir().unwrap();
        let engine = tone_engine();
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("m"));
        let (sink, rx) = channel_sink();
        engine.speak("un deux".into(), None, sink).unwrap();
        let mut chunks = 0;
        let mut samples = 0;
        loop {
            match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                Some(Ok(c)) => {
                    chunks += 1;
                    samples += c.len();
                }
                Some(Err(e)) => panic!("{e}"),
                None => break,
            }
        }
        assert_eq!(chunks, 4);
        assert_eq!(samples, 2 * 2_400);
    }

    #[test]
    fn speak_before_ready_is_a_conflict() {
        let engine = tone_engine();
        let (sink, _rx) = channel_sink();
        let err = engine.speak("un".into(), None, sink).unwrap_err();
        assert!(matches!(&err, TtsError::Conflict(m) if m == "no model is active"));
    }

    #[test]
    fn a_full_queue_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let (loader, release) = BlockingLoader::new();
        let engine = start(Arc::new(loader));
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("m"));
        let (sink, rx) = channel_sink();
        engine.speak("un".into(), None, sink).unwrap();
        assert!(release.wait_started(1), "the first speech never started");
        for _ in 0..QUEUE_LEN {
            let (sink, _rx) = channel_sink();
            engine.speak("un".into(), None, sink).unwrap();
        }
        let (sink, _rx) = channel_sink();
        let err = engine.speak("un".into(), None, sink).unwrap_err();
        assert!(matches!(err, TtsError::Busy));
        release.release_all();
        // The first speech ends once released.
        loop {
            if rx.recv_timeout(Duration::from_secs(2)).unwrap().is_none() {
                break;
            }
        }
    }

    #[test]
    fn a_sink_returning_false_stops_the_speech() {
        let dir = tempfile::tempdir().unwrap();
        let engine = tone_engine();
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("m"));
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        engine
            .speak(
                "un deux trois".into(),
                None,
                Box::new(move |_| {
                    seen.fetch_add(1, Ordering::SeqCst);
                    false
                }),
            )
            .unwrap();
        // Speeches run in order: once this one ends, the first is over.
        let (sink, rx) = channel_sink();
        engine.speak("un".into(), None, sink).unwrap();
        while rx.recv_timeout(Duration::from_secs(2)).unwrap().is_some() {}
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A speaker that panics where the test says: `voices()` or `add_voice`.
    struct Crashy {
        on_voices: bool,
    }

    impl Speaker for Crashy {
        fn sample_rate(&self) -> u32 {
            24_000
        }
        fn languages(&self) -> Vec<String> {
            vec!["fr".into()]
        }
        fn voices(&self) -> Vec<String> {
            if self.on_voices {
                panic!("voices broke");
            }
            vec!["default".into()]
        }
        fn add_voice(&mut self, _name: &str, _path: &Path) -> Result<(), String> {
            panic!("add_voice broke");
        }
        fn speak(
            &mut self,
            _text: &str,
            _voice: Option<&str>,
            _chunk: &mut dyn FnMut(&[f32]) -> bool,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct CrashyLoader {
        on_voices: bool,
    }

    impl SpeakerLoader for CrashyLoader {
        fn load(&self, _dir: &Path, _weights: &str) -> Result<Box<dyn Speaker>, String> {
            Ok(Box::new(Crashy {
                on_voices: self.on_voices,
            }))
        }
    }

    fn wait_error(engine: &Engine) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(engine.state(), EngineState::Error { .. }) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("state is {:?}, wanted an error", engine.state());
    }

    #[test]
    fn a_model_whose_voices_panic_is_an_error_state() {
        let dir = tempfile::tempdir().unwrap();
        let engine = start(Arc::new(CrashyLoader { on_voices: true }));
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_error(&engine);
        let (sink, _rx) = channel_sink();
        let err = engine.speak("un".into(), None, sink).unwrap_err();
        assert!(matches!(err, TtsError::Conflict(_)));
    }

    #[test]
    fn a_model_whose_add_voice_panics_is_an_error_state() {
        let dir = tempfile::tempdir().unwrap();
        let engine = start(Arc::new(CrashyLoader { on_voices: false }));
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("m"));
        engine.add_voice("v".into(), dir.path().join("v.safetensors"));
        wait_error(&engine);
        let (sink, _rx) = channel_sink();
        let err = engine.speak("un".into(), None, sink).unwrap_err();
        assert!(matches!(err, TtsError::Conflict(_)));
    }

    #[test]
    fn a_dead_thread_is_an_error_state_and_drops_queued_sinks() {
        struct Flag(mpsc::Sender<()>);
        impl Drop for Flag {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let (loader, release) = BlockingLoader::new();
        let engine = start(Arc::new(loader));
        engine.load("m", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("m"));
        // The first sink panics at the end of its speech, outside the guarded call.
        engine
            .speak(
                "un".into(),
                None,
                Box::new(|item| {
                    if item.is_none() {
                        panic!("sink broke");
                    }
                    true
                }),
            )
            .unwrap();
        assert!(release.wait_started(1));
        let (flag_tx, flag_rx) = mpsc::channel();
        let flag = Flag(flag_tx);
        engine
            .speak(
                "deux".into(),
                None,
                Box::new(move |_| {
                    let _keep = &flag;
                    true
                }),
            )
            .unwrap();
        release.release_all();
        wait_for(
            &engine,
            &EngineState::Error {
                model: Some("m".into()),
                message: "the speech thread stopped".into(),
            },
        );
        flag_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the queued sink was never dropped");
        let (sink, _rx) = channel_sink();
        let err = engine.speak("un".into(), None, sink).unwrap_err();
        assert!(matches!(err, TtsError::Conflict(_)));
        // Loading on a dead thread stays an error instead of hanging in Loading.
        engine.load("n", dir.path().into(), "model.q8.gguf".into());
        assert!(matches!(engine.state(), EngineState::Error { .. }));
    }

    #[test]
    fn load_clears_the_previous_models_facts() {
        let dir = tempfile::tempdir().unwrap();
        let (loader, _release) = BlockingLoader::new();
        let engine = start(Arc::new(loader));
        engine.load("a", dir.path().into(), "model.q8.gguf".into());
        wait_for(&engine, &ready("a"));
        std::fs::write(dir.path().join("fail"), "").unwrap();
        engine.load("b", dir.path().into(), "model.q8.gguf".into());
        assert!(engine.voices().is_empty());
        assert_eq!(engine.sample_rate(), None);
        wait_error(&engine);
        assert!(engine.voices().is_empty());
        assert_eq!(engine.sample_rate(), None);
    }
}
