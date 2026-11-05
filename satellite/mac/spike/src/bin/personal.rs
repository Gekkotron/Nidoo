//! Throwaway: can a personal head on openWakeWord's embeddings learn one
//! speaker's "Hey Jarvis"? 5-fold cross-validation over labelled recordings.
//!
//! oww-personal <model dir> <labels.txt>
//! labels: `<wav> pos <start> <end> <group>` | `<wav> ign <start> <end>` | `<wav> neg`
//! (`neg` marks the file's remaining time as negative; files without it are
//! only used for their `pos` windows.)

use std::collections::{BTreeMap, VecDeque};

use anyhow::{Context, bail};
use ort::session::Session;
use ort::value::Tensor;

const STEP: usize = 1280;
const CONTEXT: usize = 480;
const MEL_WINDOW: usize = 76;
const EMB_WINDOW: usize = 16;
const DIM: usize = EMB_WINDOW * 96;
const HOP_S: f32 = 0.08;

fn session(path: String) -> anyhow::Result<Session> {
    Session::builder()?
        .with_intra_threads(1)?
        .commit_from_file(&path)
        .with_context(|| path)
}

/// (time in s of the last sample seen, 16×96 features) per step.
fn features(mel: &mut Session, emb: &mut Session, samples: &[f32]) -> anyhow::Result<Vec<(f32, Vec<f32>)>> {
    let mut raw: VecDeque<f32> = VecDeque::new();
    let mut mels: VecDeque<[f32; 32]> = std::iter::repeat_n([1.0; 32], MEL_WINDOW).collect();
    let mut embs: VecDeque<Vec<f32>> = VecDeque::new();
    let mut out = Vec::new();
    for (i, chunk) in samples.chunks_exact(STEP).enumerate() {
        raw.extend(chunk);
        while raw.len() > STEP + CONTEXT {
            raw.pop_front();
        }
        let input: Vec<f32> = raw.iter().copied().collect();
        let n = input.len();
        let name = mel.inputs[0].name.clone();
        let o = mel.run(ort::inputs![name => Tensor::from_array(([1usize, n], input))?])?;
        let (_, m) = o[0].try_extract_tensor::<f32>()?;
        for f in m.chunks_exact(32) {
            let mut frame = [0.0; 32];
            for (d, s) in frame.iter_mut().zip(f) {
                *d = s / 10.0 + 2.0;
            }
            mels.push_back(frame);
        }
        while mels.len() > MEL_WINDOW {
            mels.pop_front();
        }
        let window: Vec<f32> = mels.iter().flatten().copied().collect();
        let name = emb.inputs[0].name.clone();
        let o = emb.run(ort::inputs![name => Tensor::from_array(([1usize, MEL_WINDOW, 32, 1], window))?])?;
        let (_, e) = o[0].try_extract_tensor::<f32>()?;
        embs.push_back(e.to_vec());
        while embs.len() > EMB_WINDOW {
            embs.pop_front();
        }
        if embs.len() == EMB_WINDOW {
            out.push(((i + 1) as f32 * HOP_S, embs.iter().flatten().copied().collect()));
        }
    }
    Ok(out)
}

fn read_wav(path: &str) -> anyhow::Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let s = r.spec();
    if s.sample_rate != 16_000 || s.channels != 1 || s.bits_per_sample != 16 {
        bail!("{path}: need 16 kHz mono s16");
    }
    let mut v: Vec<f32> = r.samples::<i16>().map(|x| x.map(f32::from)).collect::<Result<_, _>>()?;
    v.extend(std::iter::repeat_n(0.0, 16_000));
    Ok(v)
}

struct Utt {
    file: usize,
    start: f32,
    end: f32,
    group: String,
    fold: usize,
}

struct Model {
    mean: Vec<f32>,
    std: Vec<f32>,
    w: Vec<f32>,
    b: f32,
}

impl Model {
    fn prob(&self, x: &[f32]) -> f32 {
        let mut z = self.b;
        for i in 0..DIM {
            z += self.w[i] * (x[i] - self.mean[i]) / self.std[i];
        }
        1.0 / (1.0 + (-z).exp())
    }
}

/// Weighted logistic regression, L2, full-batch gradient descent.
fn train(xs: &[&[f32]], ys: &[f32]) -> Model {
    let n = xs.len() as f32;
    let mut mean = vec![0.0; DIM];
    let mut std = vec![0.0; DIM];
    for x in xs {
        for i in 0..DIM {
            mean[i] += x[i] / n;
        }
    }
    for x in xs {
        for i in 0..DIM {
            std[i] += (x[i] - mean[i]).powi(2) / n;
        }
    }
    for s in &mut std {
        *s = s.sqrt().max(1e-3);
    }
    let z: Vec<Vec<f32>> = xs.iter().map(|x| (0..DIM).map(|i| (x[i] - mean[i]) / std[i]).collect()).collect();
    let npos = ys.iter().filter(|&&y| y > 0.5).count() as f32;
    let (wp, wn) = (0.5 / npos, 0.5 / (n - npos));
    let (mut w, mut b) = (vec![0.0f32; DIM], 0.0f32);
    let (lr, l2) = (0.5, 1e-2);
    for _ in 0..400 {
        let mut gw = vec![0.0f32; DIM];
        let mut gb = 0.0;
        for (x, &y) in z.iter().zip(ys) {
            let mut s = b;
            for i in 0..DIM {
                s += w[i] * x[i];
            }
            let p = 1.0 / (1.0 + (-s).exp());
            let g = (p - y) * if y > 0.5 { wp } else { wn };
            gb += g;
            for i in 0..DIM {
                gw[i] += g * x[i];
            }
        }
        for i in 0..DIM {
            w[i] -= lr * (gw[i] + l2 * w[i]);
        }
        b -= lr * gb;
    }
    Model { mean, std, w, b }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 && !(args.len() == 5 && args[3] == "--save") {
        bail!("usage: oww-personal <model dir> <labels.txt> [--save head.bin]");
    }
    let dir = &args[1];
    let mut mel = session(format!("{dir}/melspectrogram.onnx"))?;
    let mut emb = session(format!("{dir}/embedding_model.onnx"))?;
    const K: usize = 5;

    let mut files: Vec<String> = Vec::new();
    let mut neg_files: Vec<bool> = Vec::new();
    let mut utts: Vec<Utt> = Vec::new();
    let mut ign: Vec<(usize, f32, f32)> = Vec::new();
    let mut idx = |files: &mut Vec<String>, neg: &mut Vec<bool>, p: &str| {
        files.iter().position(|f| f == p).unwrap_or_else(|| {
            files.push(p.to_string());
            neg.push(false);
            files.len() - 1
        })
    };
    for line in std::fs::read_to_string(&args[2])?.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() < 2 {
            continue;
        }
        let f = idx(&mut files, &mut neg_files, p[0]);
        match p[1] {
            "neg" => neg_files[f] = true,
            "ign" => ign.push((f, p[2].parse()?, p[3].parse()?)),
            "pos" => {
                let fold = utts.len() % K;
                utts.push(Utt { file: f, start: p[2].parse()?, end: p[3].parse()?, group: p[4].into(), fold });
            }
            o => bail!("bad label {o}"),
        }
    }

    // Every step of every file, labelled: Some(utt) positive window, None negative, skipped if ignored.
    struct Win {
        x: Vec<f32>,
        pos: Option<usize>,
        eval_utt: Option<usize>,
        neg: bool,
        fold: usize,
        file: usize,
        t: f32,
    }
    let mut wins: Vec<Win> = Vec::new();
    let mut neg_seconds = [0.0f32; K];
    for (fi, path) in files.iter().enumerate() {
        let feats = features(&mut mel, &mut emb, &read_wav(path)?)?;
        for (t, x) in feats {
            let pos = utts.iter().position(|u| u.file == fi && t >= u.end + 0.16 && t <= u.end + 0.8);
            let eval_utt = utts.iter().position(|u| u.file == fi && t >= u.start && t <= u.end + 1.2);
            let near = utts.iter().any(|u| u.file == fi && t >= u.start - 0.5 && t <= u.end + 1.6)
                || ign.iter().any(|&(f, a, b)| f == fi && t >= a && t <= b + 1.6);
            let neg = !near && neg_files[fi];
            // Negatives fold by 10 s chunk, so held-out talk is unseen talk.
            let fold = match pos.or(eval_utt) {
                Some(u) => utts[u].fold,
                None => ((t / 10.0) as usize + fi) % K,
            };
            if neg {
                neg_seconds[fold] += HOP_S;
            }
            if pos.is_some() || eval_utt.is_some() || neg {
                wins.push(Win { x, pos, eval_utt, neg, fold, file: fi, t });
            }
        }
    }
    eprintln!(
        "{} files, {} utterances, {} windows, {:.0} s of negative audio",
        files.len(),
        utts.len(),
        wins.len(),
        neg_seconds.iter().sum::<f32>()
    );

    if args.len() == 5 {
        let all: Vec<&Win> = wins.iter().filter(|w| w.pos.is_some() || w.neg).collect();
        let xs: Vec<&[f32]> = all.iter().map(|w| w.x.as_slice()).collect();
        let ys: Vec<f32> = all.iter().map(|w| if w.pos.is_some() { 1.0 } else { 0.0 }).collect();
        let m = train(&xs, &ys);
        let mut bytes = Vec::new();
        for v in m.mean.iter().chain(&m.std).chain(&m.w).chain(std::iter::once(&m.b)) {
            bytes.extend(v.to_le_bytes());
        }
        std::fs::write(&args[4], bytes)?;
        println!("saved head ({} windows) to {}", all.len(), args[4]);
        return Ok(());
    }

    let thresholds = [0.5f32, 0.7, 0.8, 0.9, 0.95];
    let mut best_utt = vec![0.0f32; utts.len()];
    let mut false_trig = vec![0usize; thresholds.len()];
    let mut worst_neg: Vec<(f32, usize, f32)> = Vec::new();
    for k in 0..K {
        let train_set: Vec<&Win> = wins.iter().filter(|w| w.fold != k && (w.pos.is_some() || w.neg)).collect();
        let xs: Vec<&[f32]> = train_set.iter().map(|w| w.x.as_slice()).collect();
        let ys: Vec<f32> = train_set.iter().map(|w| if w.pos.is_some() { 1.0 } else { 0.0 }).collect();
        let m = train(&xs, &ys);
        for (ti, &th) in thresholds.iter().enumerate() {
            let mut last: Option<(usize, f32)> = None;
            for w in wins.iter().filter(|w| w.fold == k && w.neg) {
                if m.prob(&w.x) >= th {
                    let fresh = !matches!(last, Some((f, t)) if f == w.file && w.t - t < 2.0);
                    if fresh {
                        false_trig[ti] += 1;
                        last = Some((w.file, w.t));
                    }
                }
            }
        }
        for w in wins.iter().filter(|w| w.fold == k) {
            let p = m.prob(&w.x);
            if let Some(u) = w.eval_utt {
                best_utt[u] = best_utt[u].max(p);
            } else if w.neg {
                worst_neg.push((p, w.file, w.t));
            }
        }
    }

    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, u) in utts.iter().enumerate() {
        groups.entry(u.group.as_str()).or_default().push(i);
    }
    for (g, us) in &groups {
        let scores: Vec<String> = us.iter().map(|&u| format!("{:.2}", best_utt[u])).collect();
        println!("{g}: held-out scores {}", scores.join(" "));
    }
    let neg_h = neg_seconds.iter().sum::<f32>() / 3600.0;
    for (ti, th) in thresholds.iter().enumerate() {
        let hits: Vec<String> = groups
            .iter()
            .map(|(g, us)| format!("{g} {}/{}", us.iter().filter(|&&u| best_utt[u] >= *th).count(), us.len()))
            .collect();
        println!(
            "threshold {th:.2}: {}  false triggers {} in {:.1} min ({:.1}/h)",
            hits.join("  "),
            false_trig[ti],
            neg_h * 60.0,
            false_trig[ti] as f32 / neg_h
        );
    }
    worst_neg.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (p, f, t) in worst_neg.iter().take(5) {
        println!("worst negative {p:.3} at {}:{t:.2}", files[*f].rsplit('/').next().unwrap_or(""));
    }
    Ok(())
}
