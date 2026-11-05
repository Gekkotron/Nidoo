//! The timed output buffer between Sendspin chunks and the speaker.
//!
//! Chunks arrive ahead of time with the local time their first frame is due (PCM s16le
//! stereo 48 kHz). The audio callback asks for the frames due from a given time on;
//! whatever has no chunk is silence, a chunk whose time has passed is dropped, and a
//! newer chunk wins where two overlap.

use std::sync::{Arc, Mutex};

const RATE: i64 = 48_000;

/// A playout shared between the network task and the audio callback.
pub type SharedPlayout = Arc<Mutex<Playout>>;

type Frame = [i16; 2];

#[derive(Debug)]
pub struct Playout {
    /// (first frame, frames), sorted, never overlapping.
    chunks: Vec<(i64, Vec<Frame>)>,
    /// Frames before this one were already handed to the speaker.
    played_until: i64,
    volume: u8,
    muted: bool,
    wake_muted: bool,
}

impl Default for Playout {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            played_until: i64::MIN,
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
        let v = f32::from(self.volume) / 100.0;
        v * v
    }

    /// Fills `out` (interleaved, `channels` per frame) with the frames due from
    /// `first_frame_at_us` on.
    pub fn fill(&mut self, out: &mut [f32], channels: usize, first_frame_at_us: i64) {
        out.fill(0.0);
        if channels == 0 {
            return;
        }
        let first = frame_at(first_frame_at_us);
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
        assert!(fill(&mut p, 480, 2, 0).iter().all(|&s| s == 0.125));
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
}
