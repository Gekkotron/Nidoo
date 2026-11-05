//! Throwaway: spots "HEY JARVIS" with the sherpa-onnx keyword spotter.
//!
//! wake-spike <model dir> <threshold> <score> <wav…>   prints detections per file
//! wake-spike <model dir> <threshold> <score> --live   prints WAKE on the default mic

use std::time::Instant;

use anyhow::{Context, bail};
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig};

const KEYWORD: &str = "▁HE Y ▁JA R VI S @HEY_JARVIS";
const RATE: i32 = 16_000;

fn spotter(dir: &str, threshold: f32, score: f32) -> anyhow::Result<KeywordSpotter> {
    let mut c = KeywordSpotterConfig::default();
    let m = |n: &str| Some(format!("{dir}/{n}-epoch-12-avg-2-chunk-16-left-64.int8.onnx"));
    c.model_config.transducer.encoder = m("encoder");
    c.model_config.transducer.decoder = m("decoder");
    c.model_config.transducer.joiner = m("joiner");
    c.model_config.tokens = Some(format!("{dir}/tokens.txt"));
    c.model_config.num_threads = 1;
    c.keywords_threshold = threshold;
    c.keywords_score = score;
    // KWS_FILE: one tokenized spelling per line, all tagged @HEY_JARVIS.
    c.keywords_buf = Some(match std::env::var("KWS_FILE") {
        Ok(p) => std::fs::read_to_string(p)?,
        Err(_) => format!("{KEYWORD}\n"),
    });
    KeywordSpotter::create(&c).context("keyword spotter")
}

fn read_wav(path: &str) -> anyhow::Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let s = r.spec();
    if s.sample_rate != RATE as u32 || s.channels != 1 || s.bits_per_sample != 16 {
        bail!("{path}: need 16 kHz mono s16");
    }
    Ok(r.samples::<i16>()
        .map(|x| x.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()?)
}

/// Feeds `samples` in 100 ms pieces; returns the detection times in seconds.
fn spot(kws: &KeywordSpotter, samples: &[f32]) -> Vec<f32> {
    let stream = kws.create_stream();
    let mut hits = Vec::new();
    for (i, chunk) in samples.chunks(1600).enumerate() {
        stream.accept_waveform(RATE, chunk);
        while kws.is_ready(&stream) {
            kws.decode(&stream);
            if let Some(r) = kws.get_result(&stream)
                && !r.keyword.is_empty()
            {
                hits.push(i as f32 * 0.1);
                kws.reset(&stream);
            }
        }
    }
    hits
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        bail!("usage: wake-spike <model dir> <threshold> <score> <wav…|--live>");
    }
    let kws = spotter(&args[1], args[2].parse()?, args[3].parse()?)?;
    if args[4] == "--live" {
        return live(&kws);
    }
    let (mut audio_s, mut cpu_s) = (0.0f64, 0.0f64);
    for path in &args[4..] {
        let samples = read_wav(path)?;
        let t = Instant::now();
        let hits = spot(&kws, &samples);
        cpu_s += t.elapsed().as_secs_f64();
        audio_s += samples.len() as f64 / RATE as f64;
        println!("{} {} {:?}", hits.len(), path, hits);
    }
    println!("RTF {:.3}", cpu_s / audio_s);
    Ok(())
}

fn live(kws: &KeywordSpotter) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let dev = cpal::default_host()
        .default_input_device()
        .context("no input device")?;
    let cfg = dev.default_input_config()?;
    let (rate, ch) = (cfg.sample_rate().0, cfg.channels() as usize);
    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let stream = dev.build_input_stream(
        &cfg.into(),
        move |d: &[f32], _| {
            let _ = tx.send(d.chunks(ch).map(|f| f[0]).collect());
        },
        |e| eprintln!("mic: {e}"),
        None,
    )?;
    stream.play()?;
    eprintln!("listening at {rate} Hz…");
    let s = kws.create_stream();
    for buf in rx {
        s.accept_waveform(rate as i32, &buf);
        while kws.is_ready(&s) {
            kws.decode(&s);
            if let Some(r) = kws.get_result(&s)
                && !r.keyword.is_empty()
            {
                println!("WAKE {}", r.keyword);
                kws.reset(&s);
            }
        }
    }
    Ok(())
}
