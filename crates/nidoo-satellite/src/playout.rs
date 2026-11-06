//! The timed output buffer between Sendspin chunks and the speaker.
//!
//! Chunks arrive ahead of time with the local time their first frame is due (PCM s16le
//! stereo 48 kHz). The audio callback asks for the frames due from a given time on;
//! whatever has no chunk is silence, a chunk whose time has passed is dropped, and a
//! newer chunk wins where two overlap.
//!
//! The callback's clock reading wobbles by a few frames from one buffer to the next,
//! so the playout keeps its own cursor: each buffer starts where the last one ended,
//! slow drift moves it one frame at a time and only a large gap (a stall, a new
//! output stream) jumps it. Following every reading skipped or repeated a frame at
//! most buffer edges, which is audible as crackle.

use std::sync::{Arc, Mutex};

const RATE: i64 = 48_000;
/// The most audio held (10 s): `nidoo-sound` sends about 5 s ahead, so only a
/// playout nothing drains (no output device) ever reaches it.
const MAX_FRAMES: i64 = 10 * RATE;
/// Up to this far (10 ms) from the cursor, a reading is jitter or drift; beyond it
/// the cursor jumps to the reading.
const RESYNC_FRAMES: i64 = RATE / 100;
/// The smoothed gap that moves the cursor by one frame.
const NUDGE_FRAMES: f32 = 2.0;
/// Weight of each reading in the smoothed gap.
const SMOOTHING: f32 = 1.0 / 16.0;

/// A playout shared between the network task and the audio callback.
pub type SharedPlayout = Arc<Mutex<Playout>>;

type Frame = [i16; 2];

#[derive(Debug)]
pub struct Playout {
    /// (first frame, frames), sorted, never overlapping.
    chunks: Vec<(i64, Vec<Frame>)>,
    /// Frames before this one were already handed to the speaker.
    played_until: i64,
    /// The frame the next buffer starts at, and the smoothed gap between the
    /// callback's readings and it.
    cursor: Option<i64>,
    drift: f32,
    volume: u8,
    muted: bool,
    wake_muted: bool,
}

impl Default for Playout {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            played_until: i64::MIN,
            cursor: None,
            drift: 0.0,
            volume: 100,
            muted: false,
            wake_muted: false,
        }
    }
}

/// The 48 kHz frame due at `us` on the local clock.
fn frame_at(us: i64) -> i64 {
    (us * RATE).div_euclid(1_000_000)
}

impl Playout {
    /// Queues `pcm` (s16le stereo 48 kHz) to start at `local_play_at_us`.
    pub fn push(&mut self, local_play_at_us: i64, pcm: &[u8]) {
        let mut frames: Vec<Frame> = pcm
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                [
                    i16::from_le_bytes([b[0], b[1]]),
                    i16::from_le_bytes([b[2], b[3]]),
                ]
            })
            .collect();
        let mut start = frame_at(local_play_at_us);
        let end = start + frames.len() as i64;
        if end <= self.played_until || frames.is_empty() {
            return;
        }
        if start < self.played_until {
            frames.drain(..(self.played_until - start) as usize);
            start = self.played_until;
        }
        let mut kept = Vec::with_capacity(self.chunks.len() + 2);
        for (s, f) in self.chunks.drain(..) {
            let e = s + f.len() as i64;
            if e <= start || s >= end {
                kept.push((s, f));
                continue;
            }
            if s < start {
                kept.push((s, f[..(start - s) as usize].to_vec()));
            }
            if e > end {
                kept.push((end, f[(end - s) as usize..].to_vec()));
            }
        }
        kept.push((start, frames));
        kept.sort_by_key(|(s, _)| *s);
        self.chunks = kept;
        self.cap();
    }

    /// Drops the oldest audio beyond `MAX_FRAMES`.
    fn cap(&mut self) {
        let mut held: i64 = self.chunks.iter().map(|(_, f)| f.len() as i64).sum();
        while held > MAX_FRAMES {
            let (s, f) = &mut self.chunks[0];
            let over = held - MAX_FRAMES;
            if f.len() as i64 <= over {
                held -= f.len() as i64;
                self.chunks.remove(0);
            } else {
                f.drain(..over as usize);
                *s += over;
                held = MAX_FRAMES;
            }
        }
    }

    /// Drops everything queued (stream end, new stream, disconnect).
    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    /// Nidoo's volume, 0 to 100.
    pub fn set_volume(&mut self, v: u8) {
        self.volume = v.min(100);
    }

    /// Nidoo's mute.
    pub fn set_muted(&mut self, m: bool) {
        self.muted = m;
    }

    /// Our own mute while a voice window is open.
    pub fn set_wake_muted(&mut self, m: bool) {
        self.wake_muted = m;
    }

    fn gain(&self) -> f32 {
        if self.muted || self.wake_muted {
            return 0.0;
        }
        if self.volume == 0 {
            return 0.0;
        }
        // The ESP32 satellite's curve, shared with `nidoo-sound`: dB linear in
        // volume, -49 dB at 1 % up to 0 dB at 100 %.
        let v = f32::from(self.volume) / 100.0;
        10f32.powf(-49.0 * (1.0 - v) / 20.0)
    }

    /// The frame a buffer read as due at `wanted` starts at.
    fn next_frame(&mut self, wanted: i64) -> i64 {
        let Some(cursor) = self.cursor.filter(|c| (wanted - c).abs() <= RESYNC_FRAMES) else {
            self.drift = 0.0;
            return wanted;
        };
        self.drift += ((wanted - cursor) as f32 - self.drift) * SMOOTHING;
        if self.drift >= NUDGE_FRAMES {
            self.drift -= 1.0;
            cursor + 1
        } else if self.drift <= -NUDGE_FRAMES {
            self.drift += 1.0;
            cursor - 1
        } else {
            cursor
        }
    }

    /// Fills `out` (interleaved, `channels` per frame) with the frames due from
    /// `first_frame_at_us` on.
    pub fn fill(&mut self, out: &mut [f32], channels: usize, first_frame_at_us: i64) {
        out.fill(0.0);
        if channels == 0 {
            return;
        }
        let first = self.next_frame(frame_at(first_frame_at_us));
        let n = (out.len() / channels) as i64;
        let last = first + n;
        let gain = self.gain() / 32_768.0;
        self.chunks.retain(|(s, f)| s + f.len() as i64 > first);
        for (s, f) in &self.chunks {
            if *s >= last {
                break;
            }
            let from = (*s).max(first);
            let to = (s + f.len() as i64).min(last);
            for at in from..to {
                let [l, r] = f[(at - s) as usize];
                let (l, r) = (f32::from(l) * gain, f32::from(r) * gain);
                let o = &mut out[(at - first) as usize * channels..][..channels];
                if channels == 1 {
                    o[0] = (l + r) / 2.0;
                } else {
                    o[0] = l;
                    o[1] = r;
                }
            }
        }
        self.chunks.retain(|(s, f)| s + f.len() as i64 > last);
        self.played_until = self.played_until.max(last);
        self.cursor = Some(last);
    }
}

#[cfg(test)]
mod tests {
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
}
