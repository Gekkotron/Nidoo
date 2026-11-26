use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::assist::tts::tone::{BlockingLoader, RecordingLoader, ToneLoader};

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
fn the_language_reaches_the_speaker() {
    let dir = tempfile::tempdir().unwrap();
    let loader = RecordingLoader::default();
    let seen = loader.seen.clone();
    let engine = start(Arc::new(loader));
    engine.load("m", dir.path().into(), "model.q8.gguf".into());
    wait_for(&engine, &ready("m"));
    for language in [Some("en".to_string()), None] {
        let (sink, rx) = channel_sink();
        engine.speak("un".into(), language, None, sink).unwrap();
        while rx.recv_timeout(Duration::from_secs(2)).unwrap().is_some() {}
    }
    assert_eq!(*seen.lock().unwrap(), [Some("en".to_string()), None]);
}

#[test]
fn two_engines_speak_independently() {
    let good = tempfile::tempdir().unwrap();
    let bad = tempfile::tempdir().unwrap();
    std::fs::write(bad.path().join("fail"), "").unwrap();
    let (a, b) = (tone_engine(), tone_engine());
    a.load("a", good.path().into(), "model.q8.gguf".into());
    b.load("b", bad.path().into(), "model.q8.gguf".into());
    wait_for(&a, &ready("a"));
    assert!(matches!(
        {
            wait_for(
                &b,
                &EngineState::Error {
                    model: Some("b".into()),
                    message: "broken model".into(),
                },
            );
            b.state()
        },
        EngineState::Error { .. }
    ));
    let (sink, rx) = channel_sink();
    a.speak("un".into(), Some("fr".into()), None, sink).unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Some(Ok(_))
    ));
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
    engine.speak("un deux".into(), None, None, sink).unwrap();
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
    let err = engine.speak("un".into(), None, None, sink).unwrap_err();
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
    engine.speak("un".into(), None, None, sink).unwrap();
    assert!(release.wait_started(1), "the first speech never started");
    for _ in 0..QUEUE_LEN {
        let (sink, _rx) = channel_sink();
        engine.speak("un".into(), None, None, sink).unwrap();
    }
    let (sink, _rx) = channel_sink();
    let err = engine.speak("un".into(), None, None, sink).unwrap_err();
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
            None,
            Box::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                false
            }),
        )
        .unwrap();
    // Speeches run in order: once this one ends, the first is over.
    let (sink, rx) = channel_sink();
    engine.speak("un".into(), None, None, sink).unwrap();
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
        _language: Option<&str>,
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
    let err = engine.speak("un".into(), None, None, sink).unwrap_err();
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
    let err = engine.speak("un".into(), None, None, sink).unwrap_err();
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
    let err = engine.speak("un".into(), None, None, sink).unwrap_err();
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
