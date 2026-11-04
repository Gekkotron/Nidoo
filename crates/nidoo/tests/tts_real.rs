//! Speed spike for the Phonon text-to-speech engine.
//!
//! Needs a real model folder, so it is `#[ignore]`d: run it by hand with
//! `NIDOO_TTS_MODEL_DIR=<folder> NIDOO_TTS_WEIGHTS=model.q8.gguf \
//!  cargo test --release -p nidoo --test tts_real -- --ignored --nocapture`.
#![cfg(feature = "tts")]

use std::time::Instant;

use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, SpeechOptions};

const SENTENCES: [&str; 2] = [
    "Salon allumé (3 lumières).",
    "Il fait 17,5 °C dans le salon.",
];

#[test]
#[ignore = "needs a Phonon model folder in NIDOO_TTS_MODEL_DIR"]
fn speaks_and_prints_timings() {
    let Ok(dir) = std::env::var("NIDOO_TTS_MODEL_DIR") else {
        eprintln!("NIDOO_TTS_MODEL_DIR is not set: skipping");
        return;
    };
    // `model.q8.gguf` by default; `model.q4k.gguf` selects the 4-bit weights.
    let weights = std::env::var("NIDOO_TTS_WEIGHTS").unwrap_or_else(|_| "model.q8.gguf".into());
    let quant = if weights.contains("q4k") {
        Quant::Q4k
    } else {
        Quant::Q80
    };
    let lang = match std::env::var("NIDOO_TTS_LANG").as_deref() {
        Ok("en") => Lang::En,
        _ => Lang::Fr,
    };
    let sentences: Vec<&str> = if matches!(lang, Lang::En) {
        vec![
            "The living room light is on.",
            "It is 17.5 degrees in the living room.",
        ]
    } else {
        SENTENCES.to_vec()
    };

    let t = Instant::now();
    let checkpoint = Checkpoint::resolve(
        &dir,
        ResolveOptions {
            quant,
            weights: Some(&weights),
        },
    )
    .unwrap();
    let mut tts = checkpoint
        .builder(lang)
        .device(DeviceKind::Cpu)
        .build()
        .unwrap();
    checkpoint.register_voices(&mut tts);
    println!("load ({weights}): {:.2?}", t.elapsed());
    let rate = tts.sample_rate() as f32;
    println!("sample rate {rate} Hz, voices {:?}", tts.voices());

    // Two rounds: the first is cold, the second shows the warm cost.
    for round in 1..=2 {
        for text in &sentences {
            let start = Instant::now();
            let mut first = None;
            let mut samples = 0usize;
            for chunk in tts.stream_with(text, &SpeechOptions::default()).unwrap() {
                let chunk = chunk.unwrap();
                first.get_or_insert_with(|| start.elapsed());
                samples += chunk.len();
            }
            let total = start.elapsed();
            let audio = samples as f32 / rate;
            println!(
                "round {round} {text:?}: first chunk {:.2?}, total {total:.2?}, audio {audio:.2}s, RTF {:.2}",
                first.unwrap_or_default(),
                total.as_secs_f32() / audio.max(f32::EPSILON)
            );
            assert!(samples > 0, "no audio produced");
        }
    }
}

#[test]
#[ignore = "needs a Phonon model folder in NIDOO_TTS_MODEL_DIR"]
fn the_engine_loads_and_speaks_with_phonon() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use nidoo::assist::tts::engine::start;
    use nidoo::assist::tts::phonon::PhononLoader;
    use nidoo_api::EngineState;

    let Ok(dir) = std::env::var("NIDOO_TTS_MODEL_DIR") else {
        eprintln!("NIDOO_TTS_MODEL_DIR is not set: skipping");
        return;
    };
    let weights = std::env::var("NIDOO_TTS_WEIGHTS").unwrap_or_else(|_| "model.q8.gguf".into());
    let engine = start(Arc::new(PhononLoader));
    engine.load("real", dir.into(), weights);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !matches!(engine.state(), EngineState::Ready { .. }) {
        assert!(
            !matches!(engine.state(), EngineState::Error { .. }),
            "{:?}",
            engine.state()
        );
        assert!(Instant::now() < deadline, "the model never became ready");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(engine.sample_rate(), Some(24_000));
    let (tx, rx) = std::sync::mpsc::channel();
    engine
        .speak(
            SENTENCES[1].into(),
            None,
            Box::new(move |item| tx.send(item).is_ok()),
        )
        .unwrap();
    let mut samples = 0;
    loop {
        match rx.recv_timeout(Duration::from_secs(30)).unwrap() {
            Some(Ok(chunk)) => samples += chunk.len(),
            Some(Err(e)) => panic!("{e}"),
            None => break,
        }
    }
    assert!(samples > 0, "no audio produced");
}

#[tokio::test]
#[ignore = "needs a Phonon model folder in NIDOO_TTS_MODEL_DIR"]
async fn the_speech_address_serves_a_real_wav() {
    use std::sync::Arc;
    use std::time::Duration;

    use nidoo::assist::tts::phonon::PhononLoader;
    use nidoo::assist::tts::{Tts, TtsConfig};
    use nidoo::config::Config;
    use nidoo::http::router;
    use nidoo::hub::Hub;
    use nidoo_api::EngineState;

    let Ok(dir) = std::env::var("NIDOO_TTS_MODEL_DIR") else {
        eprintln!("NIDOO_TTS_MODEL_DIR is not set: skipping");
        return;
    };
    let weights = std::env::var("NIDOO_TTS_WEIGHTS").unwrap_or_else(|_| "model.q8.gguf".into());
    let root = tempfile::tempdir().unwrap();
    let tts = Tts::start(TtsConfig {
        root: root.path().join("tts"),
        hf_base: "http://127.0.0.1:1".into(),
        loader: Arc::new(PhononLoader),
    });
    tts.engine().load("real", dir.into(), weights);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !matches!(tts.engine().state(), EngineState::Ready { .. }) {
        assert!(Instant::now() < deadline, "{:?}", tts.engine().state());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let hub = Hub::new(&Config::default(), tx);
    hub.set_tts(tts);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router(hub, nidoo::setup::SetupCtx::inert())).into_future());
    let client = reqwest::Client::new();
    let ticket: serde_json::Value = client
        .post(format!("http://{addr}/api/v1/assist/speech"))
        .json(&serde_json::json!({"text": SENTENCES_EN}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let url = ticket["url"].as_str().unwrap();
    let resp = client
        .get(format!("http://{addr}{url}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.headers()["content-type"], "audio/wav");
    let body = resp.bytes().await.unwrap();
    println!("header: {:02x?}", &body[..44]);
    let seconds = (body.len() - 44) as f32 / 2.0 / 24_000.0;
    println!("{} bytes, {seconds:.2} s of audio", body.len());
    assert!(seconds > 0.5);
    let out = std::env::var("NIDOO_TTS_WAV_OUT").unwrap_or_else(|_| "speech.wav".into());
    std::fs::write(out, &body).unwrap();
}

const SENTENCES_EN: &str = "The living room light is on.";
