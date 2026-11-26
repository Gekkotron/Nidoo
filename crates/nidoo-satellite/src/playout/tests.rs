use super::*;

/// `frames` stereo frames, every sample `value`.
fn pcm(frames: usize, l: i16, r: i16) -> Vec<u8> {
    (0..frames)
        .flat_map(|_| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

fn fill(p: &mut Playout, frames: usize, channels: usize, at_us: i64) -> Vec<f32> {
    let mut out = vec![f32::NAN; frames * channels];
    p.fill(&mut out, channels, at_us);
    out
}

const HALF: i16 = 16_384; // 0.5 full scale

#[test]
fn plays_chunk_at_its_time() {
    let mut p = Playout::default();
    p.push(1_000_000, &pcm(960, HALF, -HALF));
    let out = fill(&mut p, 480 + 960 + 10, 2, 990_000);
    assert!(out[..480 * 2].iter().all(|&s| s == 0.0));
    assert!(out[480 * 2..1440 * 2].chunks(2).all(|f| f == [0.5, -0.5]));
    assert!(out[1440 * 2..].iter().all(|&s| s == 0.0));
}

#[test]
fn continues_across_fills() {
    let mut p = Playout::default();
    p.push(0, &pcm(960, HALF, HALF));
    let a = fill(&mut p, 480, 2, 0);
    let b = fill(&mut p, 480, 2, 10_000);
    assert!(a.iter().chain(&b).all(|&s| s == 0.5));
}

#[test]
fn gap_plays_silence() {
    let mut p = Playout::default();
    p.push(0, &pcm(480, HALF, HALF));
    p.push(20_000, &pcm(480, HALF, HALF));
    let out = fill(&mut p, 1440, 2, 0);
    assert!(out[..480 * 2].iter().all(|&s| s == 0.5));
    assert!(out[480 * 2..960 * 2].iter().all(|&s| s == 0.0));
    assert!(out[960 * 2..].iter().all(|&s| s == 0.5));
}

#[test]
fn late_chunks_are_dropped() {
    let mut p = Playout::default();
    p.push(0, &pcm(480, HALF, HALF));
    assert!(fill(&mut p, 480, 2, 20_000).iter().all(|&s| s == 0.0));
    assert!(
        fill(&mut p, 480, 2, 0).iter().all(|&s| s == 0.0),
        "never played later"
    );
    // A chunk pushed after its time has passed is not played either.
    p.push(5_000, &pcm(240, HALF, HALF));
    assert!(fill(&mut p, 960, 2, 0).iter().all(|&s| s == 0.0));
}

#[test]
fn overlapping_chunk_does_not_double() {
    let mut p = Playout::default();
    p.push(0, &pcm(960, 1_000, 1_000));
    p.push(10_000, &pcm(960, 2_000, 2_000));
    let out = fill(&mut p, 1440, 1, 0);
    let one = 1_000.0 / 32_768.0;
    let two = 2_000.0 / 32_768.0;
    assert!(out[..480].iter().all(|&s| s == one));
    assert!(out[480..].iter().all(|&s| s == two));
}

#[test]
fn stream_end_clears() {
    let mut p = Playout::default();
    p.push(0, &pcm(960, HALF, HALF));
    p.clear();
    assert!(fill(&mut p, 960, 2, 0).iter().all(|&s| s == 0.0));
}

#[test]
fn gain_volume_mute_and_wake_mute() {
    let mut p = Playout::default();
    p.push(0, &pcm(4 * 480, HALF, HALF));
    p.set_volume(50);
    let want = 0.5 * 10f32.powf(-24.5 / 20.0);
    assert!(
        fill(&mut p, 480, 2, 0)
            .iter()
            .all(|&s| (s - want).abs() < 1e-4)
    );
    p.set_volume(0);
    assert!(fill(&mut p, 480, 2, 10_000).iter().all(|&s| s == 0.0));
    p.set_volume(100);
    p.set_muted(true);
    assert!(fill(&mut p, 480, 2, 20_000).iter().all(|&s| s == 0.0));
    p.set_muted(false);
    p.set_wake_muted(true);
    assert!(fill(&mut p, 240, 2, 30_000).iter().all(|&s| s == 0.0));
    p.set_wake_muted(false);
    assert!(fill(&mut p, 240, 2, 35_000).iter().all(|&s| s == 0.5));
}

#[test]
fn mono_output_mixes_channels() {
    let mut p = Playout::default();
    p.push(0, &pcm(480, HALF, 0));
    assert!(fill(&mut p, 480, 1, 0).iter().all(|&s| s == 0.25));
}

#[test]
fn extra_output_channels_are_silent() {
    let mut p = Playout::default();
    p.push(0, &pcm(480, HALF, -HALF));
    assert!(
        fill(&mut p, 480, 4, 0)
            .chunks(4)
            .all(|f| f == [0.5, -0.5, 0.0, 0.0])
    );
}

/// A ramp, one step per frame, so a skipped or repeated frame shows.
fn ramp(frames: usize) -> Vec<u8> {
    (0..frames)
        .flat_map(|i| {
            let v = i16::try_from(i % 30_000).unwrap();
            [v.to_le_bytes(), v.to_le_bytes()]
        })
        .flatten()
        .collect()
}

#[test]
fn clock_jitter_neither_skips_nor_repeats_frames() {
    // The callback's clock reading wobbles by a few frames (measured ±3 on a
    // MacBook): every buffer must still follow the last one exactly.
    let mut p = Playout::default();
    p.push(0, &ramp(48_000));
    let jitter = [0i64, 1, -1, 2, -2, 3, -3, 0, 1, -1];
    let mut played = Vec::new();
    for (k, j) in jitter.iter().enumerate() {
        let at = (k as i64 * 512 + j) * 1_000_000 / 48_000;
        played.extend(fill(&mut p, 512, 1, at));
    }
    let step = 1.0 / 32_768.0;
    for (i, w) in played.windows(2).enumerate() {
        assert!(
            (w[1] - w[0] - step).abs() < 1e-7,
            "jump at frame {i}: {w:?}"
        );
    }
}

#[test]
fn slow_drift_is_followed_one_frame_at_a_time() {
    // The callback clock runs 10 frames ahead over 400 buffers: the cursor
    // catches up by single frames, never by a jump.
    let mut p = Playout::default();
    let mut jumps = 0;
    let mut last: Option<f32> = None;
    p.push(0, &ramp(400 * 512 + 48_000));
    for k in 0..400i64 {
        let at = (k * 512 + k / 40) * 1_000_000 / 48_000;
        for s in fill(&mut p, 512, 1, at) {
            if let Some(l) = last {
                let d = ((s - l) * 32_768.0).round() as i64;
                if d != 1 && d > -29_000 {
                    assert_eq!(d, 2, "only single-frame skips");
                    jumps += 1;
                }
            }
            last = Some(s);
        }
    }
    assert!(
        (5..=10).contains(&jumps),
        "{jumps} skips for 10 frames of drift"
    );
}

#[test]
fn large_clock_step_resyncs() {
    // The device stalled 100 ms (or a new output stream started): play what is
    // due now, not where the old buffer ended.
    let mut p = Playout::default();
    p.push(0, &ramp(48_000));
    fill(&mut p, 480, 1, 0);
    let out = fill(&mut p, 1, 1, 110_000);
    assert_eq!((out[0] * 32_768.0).round() as i64, 110 * 48);
}

#[test]
fn chime_plays_over_both_mutes_then_stops() {
    let mut p = Playout::default();
    p.set_muted(true);
    p.set_wake_muted(true);
    p.push(0, &pcm(CHIME_FRAMES * 2, HALF, HALF));
    p.chime();
    // Split over two buffers, like a speaker callback.
    let mut out = fill(&mut p, CHIME_FRAMES / 2, 2, 0);
    out.extend(fill(&mut p, CHIME_FRAMES / 2, 2, 100_000));
    assert!(out.chunks(2).all(|f| f[0] == f[1]), "same on both sides");
    let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!((0.1..=0.5).contains(&peak), "audible but soft: {peak}");
    let first_ms = out[..96].iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(first_ms < peak / 2.0, "fades in, no click");
    let tail = &out[out.len() - 96..];
    assert!(tail.iter().all(|s| s.abs() < 0.01), "fades out, no click");
    let after = fill(&mut p, 480, 2, 200_000);
    assert!(
        after.iter().all(|&s| s == 0.0),
        "once only, music still muted"
    );
}

#[test]
fn chime_mixes_over_music() {
    let mut p = Playout::default();
    p.push(0, &pcm(CHIME_FRAMES, HALF / 4, HALF / 4));
    p.chime();
    let out = fill(&mut p, CHIME_FRAMES, 1, 0);
    let music = 0.125;
    assert!(
        out.iter().any(|&s| (s - music).abs() > 0.05),
        "the bell is heard"
    );
    assert!(
        out.iter().all(|s| s.abs() <= 1.0),
        "never clips past full scale"
    );
}

#[test]
fn holds_at_most_the_cap_when_nothing_plays() {
    // No speaker drains it (the output device failed): 60 s of 20 ms chunks.
    let mut p = Playout::default();
    for i in 0..3_000 {
        p.push(1_000_000 + i * 20_000, &pcm(960, HALF, HALF));
    }
    let held: usize = p.chunks.iter().map(|(_, f)| f.len()).sum();
    assert!(held <= MAX_FRAMES as usize, "{held} frames held");
    // The newest audio is the one kept.
    let newest = 1_000_000 + 2_999 * 20_000;
    let out = fill(&mut p, 960, 2, newest);
    assert!(out.iter().all(|&s| s == 0.5), "the last chunk still plays");
}
