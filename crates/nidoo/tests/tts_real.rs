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
