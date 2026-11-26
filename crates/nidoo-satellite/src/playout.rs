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
    /// The next frame of the wake chime while it rings.
    chime_at: Option<usize>,
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
            chime_at: None,
        }
    }
}

/// The wake chime: 200 ms, a soft bell C6 then G6, like the board's.
const CHIME_FRAMES: usize = (RATE / 5) as usize;

/// The chime's sample at frame `i`: two decaying notes, the second from 80 ms,
/// a 5 ms fade-in on each and a 10 ms fade-out at the end, so nothing clicks.
fn chime_sample(i: usize) -> f32 {
    use std::f32::consts::TAU;
    const LEVEL: f32 = 0.3;
    const TAU_S: f32 = 0.03;
    let note = |t: f32, hz: f32| {
        if t < 0.0 {
            return 0.0;
        }
        let attack = (t / 0.005).min(1.0);
        attack * (-t / TAU_S).exp() * (TAU * hz * t).sin()
    };
    let t = i as f32 / RATE as f32;
    let end = CHIME_FRAMES as f32 / RATE as f32;
    let fade = ((end - t) / 0.01).clamp(0.0, 1.0);
    LEVEL * fade * (note(t, 1046.5) + note(t - 0.08, 1568.0))
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

    /// Rings the wake chime from the next buffer on. It is heard over Nidoo's mute
    /// and the wake mute: it says the wake word was heard.
    pub fn chime(&mut self) {
        self.chime_at = Some(0);
    }

    /// Adds the chime's next frames to `out`, on the first two channels.
    fn mix_chime(&mut self, out: &mut [f32], channels: usize) {
        let Some(mut at) = self.chime_at else {
            return;
        };
        for frame in out.chunks_mut(channels) {
            if at >= CHIME_FRAMES {
                break;
            }
            let c = chime_sample(at);
            for o in frame.iter_mut().take(2) {
                *o = (*o + c).clamp(-1.0, 1.0);
            }
            at += 1;
        }
        self.chime_at = (at < CHIME_FRAMES).then_some(at);
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
        self.mix_chime(out, channels);
        self.played_until = self.played_until.max(last);
        self.cursor = Some(last);
    }
}

#[cfg(test)]
mod tests;
