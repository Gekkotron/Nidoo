//! The real recognizer on the archive's French test clip. Needs the models (about 650 MB), so
//! it is ignored by default:
//!
//! ```text
//! NIDOO_STT_MODELS=<folder> cargo test -p nidoo --test stt_models -- --ignored --nocapture
//! ```
//!
//! `<folder>` holds the Parakeet files, `silero_vad.onnx` and `test_wavs/fr.wav`, as the
//! sherpa-onnx release unpacks them.
#![cfg(feature = "stt")]

use std::path::PathBuf;
use std::time::Instant;

use nidoo::assist::stt::engine::EngineLoader;
use nidoo::assist::stt::sherpa::SherpaLoader;

/// The samples of a 16-bit mono WAV file, as -1.0 to 1.0.
fn read_wav(bytes: &[u8]) -> (u32, Vec<f32>) {
    assert_eq!(&bytes[..4], b"RIFF");
    let mut at = 12;
    let mut rate = 0;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " {
            assert_eq!(u16::from_le_bytes([body[2], body[3]]), 1, "mono");
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            assert_eq!(u16::from_le_bytes([body[14], body[15]]), 16, "16-bit");
        } else if id == b"data" {
            let samples = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0)
                .collect();
            return (rate, samples);
        }
        at += 8 + size + size % 2;
    }
    panic!("no data chunk")
}

/// Linear resampling to 16 kHz (the archive's clips are not all at 16 kHz).
fn to_16k(rate: u32, samples: &[f32]) -> Vec<f32> {
    if rate == 16_000 {
        return samples.to_vec();
    }
    let n = (samples.len() as u64 * 16_000 / u64::from(rate)) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * f64::from(rate) / 16_000.0;
            let (k, frac) = (pos as usize, (pos.fract()) as f32);
            let a = samples[k.min(samples.len() - 1)];
            let b = samples[(k + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

#[test]
#[ignore = "needs the speech models: set NIDOO_STT_MODELS"]
fn parakeet_reads_the_french_test_wav() {
    let dir = PathBuf::from(std::env::var("NIDOO_STT_MODELS").expect("NIDOO_STT_MODELS"));
    let started = Instant::now();
    let engines = SherpaLoader
        .load(&dir, &dir.join("silero_vad.onnx"), 4)
        .expect("the models load");
    println!("loaded in {} ms", started.elapsed().as_millis());
    let (rate, samples) = read_wav(&std::fs::read(dir.join("test_wavs/fr.wav")).unwrap());
    let samples = to_16k(rate, &samples);
    let mut vad = engines.vads.new_vad();
    for chunk in samples.chunks(640) {
        vad.accept(chunk);
    }
    vad.flush();
    let mut phrase = Vec::new();
    while let Some(segment) = vad.pop_segment() {
        phrase.extend(segment);
    }
    assert!(!phrase.is_empty(), "the VAD heard speech");
    let started = Instant::now();
    let text = engines.transcriber.transcribe(&phrase);
    println!(
        "{:.1} s of speech in {} ms: {text}",
        phrase.len() as f32 / 16_000.0,
        started.elapsed().as_millis()
    );
    assert!(text.split_whitespace().count() >= 3, "{text}");
}
