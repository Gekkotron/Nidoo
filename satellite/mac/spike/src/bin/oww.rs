//! Throwaway: openWakeWord `hey_jarvis` in Rust, streaming the way openWakeWord does.
//!
//! oww-spike <model dir> <threshold> <wav…>   prints detections per file
//! oww-spike <model dir> <threshold> --live [out.wav]   prints WAKE on the default mic
//!
//! Per 1280 new samples (80 ms): melspectrogram of the last 1760 samples
//! (int16 values as f32), frames scaled `x/10 + 2`, appended; one embedding
//! from the last 76 mel frames; the classifier scores the last 16 embeddings.

use std::collections::VecDeque;
use std::time::Instant;

use anyhow::{Context, bail};
use ort::session::Session;
use ort::value::Tensor;

const RATE: u32 = 16_000;
const STEP: usize = 1280;
const CONTEXT: usize = 480;
const MEL_WINDOW: usize = 76;
const EMB_WINDOW: usize = 16;

struct Oww {
    mel: Session,
    emb: Session,
    cls: Session,
    raw: VecDeque<f32>,
    pending: usize,
    mels: VecDeque<[f32; 32]>,
    embs: VecDeque<Vec<f32>>,
    threshold: f32,
    /// Predictions to skip after a reset (openWakeWord skips its first 5).
    warmup: usize,
    /// PERSONAL=head.bin: a personal head (mean, std, w over 16×96, then b) replaces the classifier.
    head: Option<Vec<f32>>,
}

fn session(path: String) -> anyhow::Result<Session> {
    Session::builder()?
        .with_intra_threads(1)?
        .commit_from_file(&path)
        .with_context(|| path)
}

impl Oww {
    fn new(dir: &str, threshold: f32) -> anyhow::Result<Self> {
        Ok(Self {
            mel: session(format!("{dir}/melspectrogram.onnx"))?,
            emb: session(format!("{dir}/embedding_model.onnx"))?,
            cls: session(format!("{dir}/hey_jarvis_v0.1.onnx"))?,
            raw: VecDeque::new(),
            pending: 0,
            mels: std::iter::repeat_n([1.0; 32], MEL_WINDOW).collect(),
            embs: VecDeque::new(),
            threshold,
            warmup: 5,
            head: match std::env::var("PERSONAL") {
                Ok(p) => {
                    let b = std::fs::read(&p).with_context(|| p.clone())?;
                    let v: Vec<f32> = b
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect();
                    if v.len() != 3 * EMB_WINDOW * 96 + 1 {
                        bail!("{p}: wrong size");
                    }
                    eprintln!("personal head: {p}");
                    Some(v)
                }
                Err(_) => None,
            },
        })
    }

    fn reset(&mut self) {
        self.embs.clear();
        self.warmup = 5;
    }

    /// Feeds 16 kHz samples (int16 range); returns the best score of each full step.
    fn push(&mut self, samples: &[f32]) -> anyhow::Result<Vec<f32>> {
        let mut scores = Vec::new();
        for &s in samples {
            self.raw.push_back(s);
            self.pending += 1;
            if self.raw.len() > STEP + CONTEXT {
                self.raw.pop_front();
            }
            if self.pending == STEP {
                self.pending = 0;
                if let Some(score) = self.step()? {
                    scores.push(score);
                }
            }
        }
        Ok(scores)
    }

    fn step(&mut self) -> anyhow::Result<Option<f32>> {
        let input: Vec<f32> = self.raw.iter().copied().collect();
        let n = input.len();
        let name = self.mel.inputs[0].name.clone();
        let out = self
            .mel
            .run(ort::inputs![name => Tensor::from_array(([1usize, n], input))?])?;
        let (_, mel) = out[0].try_extract_tensor::<f32>()?;
        for f in mel.chunks_exact(32) {
            let mut frame = [0.0; 32];
            for (d, s) in frame.iter_mut().zip(f) {
                *d = s / 10.0 + 2.0;
            }
            self.mels.push_back(frame);
        }
        while self.mels.len() > MEL_WINDOW {
            self.mels.pop_front();
        }
        let window: Vec<f32> = self.mels.iter().flatten().copied().collect();
        let name = self.emb.inputs[0].name.clone();
        let out = self.emb.run(
            ort::inputs![name => Tensor::from_array(([1usize, MEL_WINDOW, 32, 1], window))?],
        )?;
        let (_, e) = out[0].try_extract_tensor::<f32>()?;
        self.embs.push_back(e.to_vec());
        while self.embs.len() > EMB_WINDOW {
            self.embs.pop_front();
        }
        if self.embs.len() < EMB_WINDOW {
            return Ok(None);
        }
        let feats: Vec<f32> = self.embs.iter().flatten().copied().collect();
        if let Some(h) = &self.head {
            let d = feats.len();
            let mut z = h[3 * d];
            for i in 0..d {
                z += h[2 * d + i] * (feats[i] - h[i]) / h[d + i];
            }
            if self.warmup > 0 {
                self.warmup -= 1;
                return Ok(Some(0.0));
            }
            return Ok(Some(1.0 / (1.0 + (-z).exp())));
        }
        let name = self.cls.inputs[0].name.clone();
        let out = self.cls.run(
            ort::inputs![name => Tensor::from_array(([1usize, EMB_WINDOW, 96], feats))?],
        )?;
        let (_, p) = out[0].try_extract_tensor::<f32>()?;
        if self.warmup > 0 {
            self.warmup -= 1;
            return Ok(Some(0.0));
        }
        Ok(Some(p[0]))
    }
}

fn read_wav(path: &str) -> anyhow::Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let s = r.spec();
    if s.sample_rate != RATE || s.channels != 1 || s.bits_per_sample != 16 {
        bail!("{path}: need 16 kHz mono s16");
    }
    Ok(r.samples::<i16>()
        .map(|x| x.map(f32::from))
        .collect::<Result<_, _>>()?)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        bail!("usage: oww-spike <model dir> <threshold> <wav…|--live>");
    }
    let threshold: f32 = args[2].parse()?;
    let mut oww = Oww::new(&args[1], threshold)?;
    if args[3] == "--live" {
        return live(&mut oww, args.get(4).map(String::as_str));
    }
    let (mut audio_s, mut cpu_s) = (0.0f64, 0.0f64);
    for path in &args[3..] {
        let mut samples = read_wav(path)?;
        // 1 s of silence after, so a phrase at the very end is still scored.
        samples.extend(std::iter::repeat_n(0.0, RATE as usize));
        oww.raw.clear();
        oww.pending = 0;
        oww.reset();
        let t = Instant::now();
        let mut hits = Vec::new();
        let mut best = 0.0f32;
        let mut refractory = 0;
        let trace = std::env::var_os("TRACE").is_some();
        for (i, score) in oww.push(&samples)?.into_iter().enumerate() {
            best = best.max(score);
            if trace {
                // Score i covers audio up to (i + 16) steps: 15 to fill the window, +1.
                eprintln!("t {:5.2} score {score:.3}", (i + 16) as f32 * 0.08);
            }
            if refractory > 0 {
                refractory -= 1;
            } else if score >= threshold {
                hits.push(i as f32 * 0.08);
                refractory = 25; // 2 s
            }
        }
        cpu_s += t.elapsed().as_secs_f64();
        audio_s += samples.len() as f64 / f64::from(RATE);
        println!("{} {} best={best:.3} {:?}", hits.len(), path, hits);
    }
    println!("RTF {:.3}", cpu_s / audio_s);
    Ok(())
}

fn live(oww: &mut Oww, record: Option<&str>) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let dev = cpal::default_host()
        .default_input_device()
        .context("no input device")?;
    let cfg = dev.default_input_config()?;
    let (rate, ch) = (cfg.sample_rate().0 as usize, cfg.channels() as usize);
    eprintln!(
        "mic: {:?}, {rate} Hz, {ch} ch, {:?}",
        dev.name().unwrap_or_default(),
        cfg.sample_format()
    );
    if rate % RATE as usize != 0 {
        bail!("mic at {rate} Hz: this spike only decimates by an integer");
    }
    let k = rate / RATE as usize;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let stream = dev.build_input_stream(
        &cfg.into(),
        move |d: &[f32], _| {
            // Crude decimation (average of k frames, first channel): fine for a spike.
            let mono: Vec<f32> = d.chunks(ch).map(|f| f[0]).collect();
            let _ = tx.send(
                mono.chunks(k)
                    .map(|c| c.iter().sum::<f32>() / c.len() as f32 * 32768.0)
                    .collect(),
            );
        },
        |e| eprintln!("mic: {e}"),
        None,
    )?;
    stream.play()?;
    let mut wav = match record {
        Some(p) if std::path::Path::new(p).exists() => bail!("{p} exists: pick a new name"),
        Some(p) => Some(hound::WavWriter::create(
            p,
            hound::WavSpec {
                channels: 1,
                sample_rate: RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?),
        None => None,
    };
    eprintln!("listening… one line per second: level (dBFS) and best score");
    let mut refractory = 0;
    let (mut sq, mut n, mut best) = (0.0f64, 0usize, 0.0f32);
    for buf in rx {
        if let Some(w) = wav.as_mut() {
            for &s in &buf {
                w.write_sample(s.clamp(-32768.0, 32767.0) as i16)?;
            }
            w.flush()?;
        }
        for &s in &buf {
            sq += f64::from(s / 32768.0).powi(2);
        }
        n += buf.len();
        for score in oww.push(&buf)? {
            best = best.max(score);
            if refractory > 0 {
                refractory -= 1;
            } else if score >= oww.threshold {
                println!("WAKE {score:.2}");
                refractory = 25;
            }
        }
        if n >= RATE as usize {
            let db = 10.0 * (sq / n as f64).max(1e-12).log10();
            eprintln!("level {db:6.1} dBFS  best {best:.2}");
            (sq, n, best) = (0.0, 0, 0.0);
        }
    }
    Ok(())
}
