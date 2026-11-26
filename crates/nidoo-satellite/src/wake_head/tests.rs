use super::*;

const QUIET: f32 = -60.0;
const LOUD: f32 = -20.0;

/// Features carrying the step's time in their first value.
fn feat(t: f32) -> Vec<f32> {
    let mut x = vec![0.0; FEATURES];
    x[0] = t;
    x
}

/// Drives `e` through `steps` levels; returns the last progress and the next time.
fn feed(e: &mut Enrolment, t0: f32, levels: &[f32]) -> (TrainProgress, f32) {
    let mut t = t0;
    let mut last = None;
    for &l in levels {
        t += HOP_S;
        last = Some(e.push(l, &feat(t)));
    }
    (last.expect("at least one step"), t)
}

fn steps(secs: f32, level: f32) -> Vec<f32> {
    vec![level; (secs / HOP_S).round() as usize]
}

/// `n` tries of 0.56 s, each after 1.52 s of quiet, ending when the last one is
/// heard (0.48 s of quiet closes a burst).
fn tries(n: usize) -> Vec<f32> {
    let mut v = Vec::new();
    for _ in 0..n {
        v.extend(steps(1.52, QUIET));
        v.extend(steps(0.56, LOUD));
    }
    v.extend(steps(0.48, QUIET));
    v
}

/// When try `k` of `tries()` ends.
fn try_end(k: usize) -> f32 {
    (26 + 26 * k) as f32 * HOP_S
}

#[test]
fn segments_tries_by_level() {
    let mut e = Enrolment::new(15, 30, 120);
    let (p, t) = feed(&mut e, 0.0, &tries(3));
    assert_eq!(p, TrainProgress::Saying { heard: 3, of: 15 });
    assert_eq!(p.status(), "Say Hey Jarvis — 3 of 15");
    feed(&mut e, t, &steps(2.0, QUIET));
    let ends: Vec<f32> = (0..3).map(try_end).collect();
    let pos = e.positive_times();
    assert!(!pos.is_empty());
    for t in &pos {
        assert!(
            ends.iter()
                .any(|end| *t >= end + 0.16 - 1e-3 && *t <= end + 0.8 + 1e-3),
            "positive at {t}"
        );
    }
    // Every try gave positives.
    for &end in &ends {
        assert!(
            pos.iter()
                .any(|t| *t >= end + 0.16 - 1e-3 && *t <= end + 0.8 + 1e-3)
        );
    }
    // Negatives keep away from the tries.
    for t in e.negative_times() {
        assert!(
            ends.iter().all(|end| t < end - 0.56 - 0.5 || t > end + 1.6),
            "negative at {t}"
        );
    }
}

#[test]
fn ignores_too_short_or_too_long_bursts() {
    let mut e = Enrolment::new(15, 30, 120);
    let mut v = steps(1.0, QUIET);
    v.extend(steps(0.08, LOUD)); // a click
    v.extend(steps(2.0, QUIET));
    v.extend(steps(2.0, LOUD)); // a sentence
    v.extend(steps(2.0, QUIET));
    let (p, _) = feed(&mut e, 0.0, &v);
    assert_eq!(p, TrainProgress::Saying { heard: 0, of: 15 });
    assert!(e.positive_times().is_empty());
}

#[test]
fn gaps_up_to_0_4_s_are_one_try() {
    let mut e = Enrolment::new(15, 30, 120);
    let mut v = steps(1.0, QUIET);
    v.extend(steps(0.32, LOUD)); // "Hey"
    v.extend(steps(0.32, QUIET));
    v.extend(steps(0.4, LOUD)); // "Jarvis"
    v.extend(steps(2.0, QUIET));
    let (p, _) = feed(&mut e, 0.0, &v);
    assert_eq!(p, TrainProgress::Saying { heard: 1, of: 15 });
}

#[test]
fn moves_to_talking_then_training() {
    let mut e = Enrolment::new(15, 30, 120);
    let (p, t) = feed(&mut e, 0.0, &tries(15));
    assert_eq!(p, TrainProgress::Talking { secs_left: 30 });
    let before = e.negative_times().len();
    let (p, t) = feed(&mut e, t, &steps(7.0, LOUD));
    assert_eq!(p, TrainProgress::Talking { secs_left: 23 });
    assert_eq!(p.status(), "Now talk normally — 23 s");
    let (p, _) = feed(&mut e, t, &steps(23.0, LOUD));
    assert_eq!(p, TrainProgress::Training);
    assert_eq!(p.status(), "Training…");
    assert!(
        e.negative_times().len() >= before + 360,
        "talking is all negatives"
    );
}

#[test]
fn too_few_tries_fails() {
    let mut e = Enrolment::new(15, 30, 120);
    let mut v = tries(4);
    let elapsed = v.len() as f32 * HOP_S;
    v.extend(steps(120.0 - elapsed, QUIET));
    let (p, _) = feed(&mut e, 0.0, &v);
    let want = TrainProgress::Failed("Heard only 4 of 15 — try again".into());
    assert_eq!(p, want);
    assert_eq!(p.status(), "Heard only 4 of 15 — try again");
}

#[test]
fn finish_trains_a_head() {
    let mut e = Enrolment::new(2, 2, 120);
    let (p, t) = feed(&mut e, 0.0, &tries(2));
    assert_eq!(p, TrainProgress::Talking { secs_left: 2 });
    let (p, _) = feed(&mut e, t, &steps(2.0, LOUD));
    assert_eq!(p, TrainProgress::Training);
    e.add_negatives(vec![feat(-1.0); 3]);
    assert!(matches!(e.finish(), TrainProgress::Done(_)));
    assert_eq!(
        TrainProgress::Done(Head::zero()).status(),
        "Hey Jarvis trained"
    );
}

/// Deterministic noise in [-0.5, 0.5).
fn noise(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 40) as f32 / (1u64 << 24) as f32) - 0.5
}

fn cluster(center: f32, n: usize, seed: &mut u64) -> Vec<Vec<f32>> {
    (0..n)
        .map(|_| (0..FEATURES).map(|_| center + noise(seed)).collect())
        .collect()
}

#[test]
fn train_separates_two_clusters() {
    let mut seed = 7;
    let head = train(&cluster(0.3, 10, &mut seed), &cluster(-0.3, 30, &mut seed));
    for x in cluster(0.3, 5, &mut seed) {
        assert!(head.prob(&x) > 0.9, "{}", head.prob(&x));
    }
    for x in cluster(-0.3, 5, &mut seed) {
        assert!(head.prob(&x) < 0.1, "{}", head.prob(&x));
    }
}

#[test]
fn head_bytes_roundtrip_and_wrong_size_rejected() {
    let mut seed = 3;
    let head = train(&cluster(0.3, 4, &mut seed), &cluster(-0.3, 4, &mut seed));
    let bytes = head.to_bytes();
    assert_eq!(bytes.len(), (3 * FEATURES + 1) * 4);
    assert_eq!(Head::from_bytes(&bytes).unwrap(), head);
    assert!(Head::from_bytes(&bytes[1..]).is_err());
    assert!(Head::from_bytes(&[]).is_err());
}

#[test]
fn head_file_written_atomically() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    save_head(dir.path(), &Head::zero()).unwrap();
    let file = dir.path().join("head.bin");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!dir.path().join("head.bin.tmp").exists());
    assert_eq!(load_head(dir.path()).unwrap(), Some(Head::zero()));
    assert_eq!(
        load_head(tempfile::tempdir().unwrap().path()).unwrap(),
        None
    );
}
