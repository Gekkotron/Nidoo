//! Train Hey Jarvis: the owner's tries and talk feed `Enrolment` step by step, while
//! a background thread makes extra negatives from the Mac's own French and English
//! voices (`say` into a temp file, never played aloud). Training runs on its own
//! thread; the head is saved only when it finishes and was not cancelled. Audio and
//! features are dropped afterwards and never leave the Mac.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use nidoo_satellite::Trainer;
use nidoo_satellite::wake_head::{Enrolment, TrainProgress, save_head};

use crate::features::Frontend;
use crate::models::Models;

const TRIES: usize = 15;
const TALK_S: u32 = 30;
const MAX_SAYING_S: u32 = 120;
/// Each sentence is spoken by this many voices of its language.
const VOICES_PER_SENTENCE: usize = 3;

const FRENCH: [&str; 10] = [
    "Je vais faire les courses cet après-midi, il manque du pain et du lait.",
    "Tu as vu la météo pour demain ? Il paraît qu'il va pleuvoir.",
    "On pourrait manger dehors ce soir si le vent se calme.",
    "J'ai oublié de rappeler le garage pour la voiture.",
    "Les enfants rentrent de l'école vers seize heures trente.",
    "Est-ce que quelqu'un a vu mes clés ? Elles étaient sur la table.",
    "Le film commence dans dix minutes, dépêche-toi.",
    "Il faudrait arroser les plantes du jardin avant de partir.",
    "Je prends un café, tu en veux un aussi ?",
    "La réunion a été déplacée à jeudi matin.",
];

const ENGLISH: [&str; 10] = [
    "I'm going to the shops later, we're out of bread and milk.",
    "Did you see the weather for tomorrow? It might rain all day.",
    "We could eat outside tonight if the wind dies down.",
    "I forgot to call the garage about the car again.",
    "The kids get back from school at half past four.",
    "Has anyone seen my keys? They were on the table a minute ago.",
    "The film starts in ten minutes, hurry up.",
    "Someone should water the garden before we leave.",
    "I'm making a coffee, do you want one too?",
    "The meeting was moved to Thursday morning.",
];

/// The synthetic-talk thread and its stop flag.
type Synth = (Arc<AtomicBool>, JoinHandle<Vec<Vec<f32>>>);

pub struct MacTrainer {
    models: Arc<Models>,
    store_dir: PathBuf,
    head_changed: Arc<AtomicBool>,
    front: Option<Frontend>,
    enrol: Option<Enrolment>,
    last: TrainProgress,
    synth: Option<Synth>,
    finishing: Option<JoinHandle<TrainProgress>>,
}

impl MacTrainer {
    pub fn new(models: Arc<Models>, store_dir: PathBuf, head_changed: Arc<AtomicBool>) -> Self {
        Self {
            models,
            store_dir,
            head_changed,
            front: None,
            enrol: None,
            last: TrainProgress::Training,
            synth: None,
            finishing: None,
        }
    }

    fn stop_synth(&mut self) {
        if let Some((stop, _)) = self.synth.take() {
            stop.store(true, Ordering::Release);
        }
    }

    fn finished(&mut self, progress: TrainProgress) -> TrainProgress {
        if let TrainProgress::Done(head) = &progress {
            if let Err(e) = save_head(&self.store_dir, head) {
                tracing::error!(error = %e, "saving head.bin");
                return TrainProgress::Failed("The trained model could not be saved".into());
            }
            self.head_changed.store(true, Ordering::Release);
        }
        progress
    }
}

impl Trainer for MacTrainer {
    fn start(&mut self) {
        self.cancel();
        if !self.models.ready() {
            self.last = TrainProgress::Failed(
                "The wake word model is still downloading — try again in a minute".into(),
            );
            return;
        }
        if self.front.is_none() {
            match Frontend::new(&self.models.dir) {
                Ok(front) => self.front = Some(front),
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "loading the wake word");
                    self.last = TrainProgress::Failed("The wake word model could not load".into());
                    return;
                }
            }
        }
        self.enrol = Some(Enrolment::new(TRIES, TALK_S, MAX_SAYING_S));
        self.last = TrainProgress::Saying {
            heard: 0,
            of: TRIES,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (dir, s) = (self.models.dir.clone(), stop.clone());
        match std::thread::Builder::new()
            .name("train-voices".into())
            .spawn(move || synthetic_negatives(&dir, &s))
        {
            Ok(handle) => self.synth = Some((stop, handle)),
            Err(e) => tracing::warn!(error = %e, "no synthetic talk"),
        }
    }

    fn push(&mut self, samples_16k: &[i16]) -> TrainProgress {
        if let Some(handle) = self.finishing.take_if(|h| h.is_finished()) {
            let progress = handle
                .join()
                .unwrap_or_else(|_| TrainProgress::Failed("Training failed — try again".into()));
            return self.finished(progress);
        }
        if self.finishing.is_some() {
            return TrainProgress::Training;
        }
        let (Some(enrol), Some(front)) = (self.enrol.as_mut(), self.front.as_mut()) else {
            return self.last.clone();
        };
        let steps = match front.push(samples_16k) {
            Ok(steps) => steps,
            Err(e) => {
                tracing::warn!(error = %e, "training features");
                return self.last.clone();
            }
        };
        for step in steps {
            let Some(features) = step.features else {
                continue;
            };
            self.last = enrol.push(step.level_db, &features);
            match &self.last {
                TrainProgress::Training => {
                    let mut enrol = self.enrol.take().expect("checked above");
                    let synth = self.synth.take();
                    self.finishing = std::thread::Builder::new()
                        .name("train".into())
                        .spawn(move || {
                            if let Some((_, handle)) = synth {
                                enrol.add_negatives(handle.join().unwrap_or_default());
                            }
                            enrol.finish()
                        })
                        .ok();
                    if self.finishing.is_none() {
                        return TrainProgress::Failed("Training failed — try again".into());
                    }
                    self.front.as_mut().map(Frontend::clear);
                    return TrainProgress::Training;
                }
                TrainProgress::Failed(_) => {
                    self.enrol = None;
                    self.stop_synth();
                    self.front.as_mut().map(Frontend::clear);
                    return self.last.clone();
                }
                _ => {}
            }
        }
        self.last.clone()
    }

    fn cancel(&mut self) {
        self.enrol = None;
        self.stop_synth();
        // A training thread still running is left to end; its head is never saved.
        self.finishing = None;
        if let Some(front) = self.front.as_mut() {
            front.clear();
        }
    }
}

/// `(name, locale)` of each installed voice, from `say -v '?'`.
pub(crate) fn voices() -> Vec<(String, String)> {
    let Ok(out) = Command::new("say").args(["-v", "?"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let left = line.split('#').next()?.trim_end();
            let (name, locale) = left.rsplit_once(char::is_whitespace)?;
            Some((name.trim().to_string(), locale.to_string()))
        })
        .collect()
}

fn synthetic_negatives(models: &Path, stop: &AtomicBool) -> Vec<Vec<f32>> {
    let mut front = match Frontend::new(models) {
        Ok(front) => front,
        Err(_) => return Vec::new(),
    };
    let all = voices();
    let mut windows = Vec::new();
    for (lang, sentences) in [("fr_", &FRENCH), ("en_", &ENGLISH)] {
        let names: Vec<&str> = all
            .iter()
            .filter(|(_, locale)| locale.starts_with(lang))
            .map(|(name, _)| name.as_str())
            .collect();
        if names.is_empty() {
            continue;
        }
        for (i, sentence) in sentences.iter().enumerate() {
            for k in 0..VOICES_PER_SENTENCE.min(names.len()) {
                if stop.load(Ordering::Acquire) {
                    return Vec::new();
                }
                let voice = names[(i * VOICES_PER_SENTENCE + k) % names.len()];
                match spoken(voice, sentence) {
                    Ok(mut samples) => {
                        samples.extend(std::iter::repeat_n(0, 16_000));
                        front.clear();
                        if let Ok(steps) = front.push(&samples) {
                            windows.extend(steps.into_iter().filter_map(|s| s.features));
                        }
                    }
                    Err(e) => tracing::debug!(voice, error = %e, "say"),
                }
            }
        }
    }
    tracing::info!(windows = windows.len(), "synthetic talk ready");
    windows
}

/// `sentence` in `voice`, 16 kHz mono, through a temp file deleted right away.
pub(crate) fn spoken(voice: &str, sentence: &str) -> anyhow::Result<Vec<i16>> {
    let path = std::env::temp_dir().join(format!(
        "nidoo-satellite-{}-{:x}.wav",
        std::process::id(),
        rand_suffix()
    ));
    let status = Command::new("say")
        .arg("-v")
        .arg(voice)
        .arg("-o")
        .arg(&path)
        .arg("--data-format=LEI16@16000")
        .arg(sentence)
        .status();
    let samples = (|| {
        anyhow::ensure!(status?.success(), "say failed");
        let mut r = hound::WavReader::open(&path)?;
        anyhow::ensure!(r.spec().channels == 1 && r.spec().sample_rate == 16_000);
        Ok(r.samples::<i16>().collect::<Result<Vec<_>, _>>()?)
    })();
    let _ = std::fs::remove_file(&path);
    samples
}

fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(std::time::SystemTime::now())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wake::{Detector, Thresholds};
    use nidoo_satellite::WakeDetector;

    /// The models the app downloaded (run it once first).
    pub(crate) fn models() -> Arc<Models> {
        let home = std::env::var_os("HOME").unwrap();
        Models::ready_at(
            PathBuf::from(home).join("Library/Application Support/Nidoo Satellite/models"),
        )
    }

    pub(crate) fn english_voice() -> String {
        voices()
            .into_iter()
            .find(|(n, l)| l == "en_US" && n == "Samantha")
            .or_else(|| voices().into_iter().find(|(_, l)| l == "en_US"))
            .expect("an en_US voice")
            .0
    }

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nidoo-sat-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn feed(t: &mut MacTrainer, samples: &[i16]) -> TrainProgress {
        let mut last = TrainProgress::Training;
        for chunk in samples.chunks(320) {
            last = t.push(chunk);
            if matches!(last, TrainProgress::Done(_) | TrainProgress::Failed(_)) {
                return last;
            }
        }
        last
    }

    #[test]
    #[ignore = "macOS with the downloaded models; slow"]
    fn trains_a_head_from_spoken_tries() {
        let dir = scratch("train");
        let flag = Arc::new(AtomicBool::new(false));
        let mut t = MacTrainer::new(models(), dir.clone(), flag.clone());
        t.start();
        let voice = english_voice();
        let tryout = spoken(&voice, "Hey Jarvis").unwrap();
        let silence = vec![0i16; 32_000];
        feed(&mut t, &silence);
        let mut p = TrainProgress::Training;
        for _ in 0..TRIES {
            feed(&mut t, &tryout);
            p = feed(&mut t, &silence);
        }
        assert!(matches!(p, TrainProgress::Talking { .. }), "{p:?}");
        let mut talk = Vec::new();
        for s in ENGLISH {
            talk.extend(spoken(&voice, s).unwrap());
        }
        while talk.len() < 16_000 * 32 {
            talk.extend_from_within(..);
        }
        let p = feed(&mut t, &talk[..16_000 * 32]);
        assert_eq!(p, TrainProgress::Training);
        let start = std::time::Instant::now();
        let p = loop {
            let p = t.push(&[0; 1280]);
            if !matches!(p, TrainProgress::Training) {
                break p;
            }
            assert!(start.elapsed().as_secs() < 300, "training never ended");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(matches!(p, TrainProgress::Done(_)), "{p:?}");
        assert!(dir.join("head.bin").exists());
        assert!(flag.load(Ordering::Acquire));

        // The detector now uses the personal head and hears a new try.
        let mut d = Detector::new(
            models(),
            dir.clone(),
            Thresholds {
                stock: 0.5,
                personal: 0.9,
            },
            flag,
        );
        let mut heard = false;
        for chunk in silence
            .iter()
            .chain(&tryout)
            .chain(&silence)
            .copied()
            .collect::<Vec<_>>()
            .chunks(320)
        {
            heard |= d.push(chunk);
        }
        assert!(heard, "personal head hears the try");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[ignore = "macOS with the downloaded models"]
    fn cancel_keeps_the_old_head() {
        let dir = scratch("cancel");
        let mut t = MacTrainer::new(models(), dir.clone(), Arc::new(AtomicBool::new(false)));
        t.start();
        feed(&mut t, &vec![0; 16_000]);
        t.cancel();
        assert_eq!(
            feed(&mut t, &vec![0; 16_000]),
            TrainProgress::Saying {
                heard: 0,
                of: TRIES
            }
        );
        assert!(!dir.join("head.bin").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
