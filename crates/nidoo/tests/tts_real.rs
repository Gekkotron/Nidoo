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

/// The resident memory of this process, in KB (`ps`), or 0 when it cannot be read.
fn rss_kb() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// One loaded model serves several languages only if the hub can normalise the text itself:
/// a model built with `Normalize::OFF`, fed text the hub normalised as French, must speak
/// exactly as a model built for French. Also prints what a second loaded model costs.
#[test]
#[ignore = "needs a Phonon model folder in NIDOO_TTS_MODEL_DIR"]
fn normalising_outside_the_builder_matches() {
    use ptts::preprocess::Normalize;
    let Ok(dir) = std::env::var("NIDOO_TTS_MODEL_DIR") else {
        eprintln!("NIDOO_TTS_MODEL_DIR is not set: skipping");
        return;
    };
    let text = "Il fait 19,5 degrés le 3 novembre.";
    let checkpoint = Checkpoint::resolve(
        &dir,
        ResolveOptions {
            quant: Quant::Q80,
            weights: Some("model.q8.gguf"),
        },
    )
    .unwrap();
    let before = rss_kb();
    let fr = checkpoint
        .builder(Lang::Fr)
        .device(DeviceKind::Cpu)
        .build()
        .unwrap();
    let one = rss_kb();
    let off = checkpoint
        .builder(Normalize::OFF)
        .device(DeviceKind::Cpu)
        .build()
        .unwrap();
    let two = rss_kb();
    let speak = |synth: &ptts::synth::Synth, text: &str| -> Vec<f32> {
        let opts = SpeechOptions::default().seed(42);
        synth
            .stream_with(text, &opts)
            .unwrap()
            .flat_map(|piece| piece.unwrap())
            .collect()
    };
    let by_builder = speak(&fr, text);
    let normalised = Normalize::for_lang(Lang::Fr).apply(text).into_owned();
    let by_hub = speak(&off, &normalised);
    eprintln!("normalised text: {normalised:?}");
    eprintln!(
        "rss: before {before} KB, one model {one} KB, two models {two} KB (second costs {} KB)",
        two.saturating_sub(one)
    );
    eprintln!(
        "samples: builder {} hub {}",
        by_builder.len(),
        by_hub.len()
    );
    let again = speak(&fr, text);
    let max_diff = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0f32, f32::max)
    };
    let peak = by_builder.iter().fold(0f32, |m, x| m.max(x.abs()));
    eprintln!(
        "peak {peak}; max diff builder vs builder again {}; builder vs hub {}",
        max_diff(&by_builder, &again),
        max_diff(&by_builder, &by_hub)
    );
    assert_eq!(by_builder.len(), by_hub.len(), "same speech length");
    // Bit-exact only when the model itself is: compare against its own run-to-run noise.
    let noise = max_diff(&by_builder, &again);
    assert!(
        max_diff(&by_builder, &by_hub) <= noise.max(peak * 1e-4),
        "the hub-normalised speech differs from the model's own"
    );
}
