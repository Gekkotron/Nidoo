//! Says one sentence at a time over the music, on the shared clock.
//!
//! A sentence owns a *window* on the timeline: `FADE_FRAMES` where the music
//! fades down to [`DUCK_GAIN`], the speech itself, then `FADE_FRAMES` where
//! the music comes back. Any 20 ms chunk that overlaps the window is rebuilt
//! for the sentence's satellites as `music × gain + speech`; the others keep
//! the shared chunk. Pure: no clock, no I/O; the group decides when.

use std::collections::VecDeque;

use bytes::Bytes;

use crate::pacer::{CHUNK_FRAMES, FRAME_BYTES};
use crate::speech::Speech;

/// Music gain under speech: −15 dB.
pub const DUCK_GAIN: f32 = 0.177_83;
/// 150 ms at 48 kHz, into and out of the duck.
pub const FADE_FRAMES: usize = 7_200;
/// Sentences that may wait behind the one playing.
pub const MAX_QUEUED: usize = 4;

/// The sentence playing and the ones waiting.
#[derive(Debug, Default)]
pub struct SpeechMixer {
    queue: VecDeque<Speech>,
    current: Option<(Speech, i64)>,
}

impl SpeechMixer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a sentence; `false` when [`MAX_QUEUED`] already wait (dropped).
    pub fn push(&mut self, speech: Speech) -> bool {
        if self.queue.len() >= MAX_QUEUED {
            return false;
        }
        self.queue.push_back(speech);
        true
    }

    pub fn is_playing(&self) -> bool {
        self.current.is_some()
    }

    pub fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }

    /// When nothing plays, starts the next sentence with its window at
    /// `start_us` and returns its satellites.
    pub fn start_next(&mut self, start_us: i64) -> Option<&[String]> {
        if self.current.is_some() {
            return None;
        }
        let next = self.queue.pop_front()?;
        self.current = Some((next, start_us));
        Some(self.targets())
    }

    /// The playing sentence's satellites (empty when idle).
    pub fn targets(&self) -> &[String] {
        self.current
            .as_ref()
            .map_or(&[], |(s, _)| s.players.as_slice())
    }

    /// End of the playing sentence's window.
    pub fn end_us(&self) -> Option<i64> {
        let (speech, start) = self.current.as_ref()?;
        let frames = window_frames(speech) as i64;
        Some(start + (frames * 1_000_000 + 47_999) / 48_000)
    }

    /// The chunk at `timestamp_us` for the sentence's satellites, `None` when
    /// it does not overlap the window. `music` absent is silence.
    pub fn mix_chunk(&self, timestamp_us: i64, music: Option<&[u8]>) -> Option<Bytes> {
        let (speech, start) = self.current.as_ref()?;
        let window = window_frames(speech) as i64;
        // Frame of the window where this chunk begins (48 frames per ms).
        let first = ((timestamp_us - start) as f64 * 0.048).round() as i64;
        if first >= window || first + CHUNK_FRAMES as i64 <= 0 {
            return None;
        }
        let speech_frames = (speech.pcm.len() / FRAME_BYTES) as i64;
        let read = |bytes: &[u8], frame: usize, ch: usize| {
            let at = frame * FRAME_BYTES + ch * 2;
            bytes
                .get(at..at + 2)
                .map_or(0.0, |b| f32::from(i16::from_le_bytes([b[0], b[1]])))
        };
        let mut out = Vec::with_capacity(CHUNK_FRAMES * FRAME_BYTES);
        for i in 0..CHUNK_FRAMES {
            let f = first + i as i64;
            let gain = music_gain(f, window);
            let k = f - FADE_FRAMES as i64;
            for ch in 0..2 {
                let m = music.map_or(0.0, |m| read(m, i, ch));
                let sp = if (0..speech_frames).contains(&k) {
                    read(&speech.pcm, k as usize, ch)
                } else {
                    0.0
                };
                let v = (m * gain + sp)
                    .round()
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Some(Bytes::from(out))
    }

    /// Ends the playing sentence once everything up to `sent_until_us` went
    /// out past its window; `true` when it did.
    pub fn finish_if_done(&mut self, sent_until_us: i64) -> bool {
        match self.end_us() {
            Some(end) if sent_until_us >= end => {
                self.current = None;
                true
            }
            _ => false,
        }
    }
}

/// `chunk` with the music coming back: frame `i` at gain `min(1, (from_frame + i) / FADE_FRAMES)`.
pub fn fade_in(chunk: &[u8], from_frame: usize) -> Bytes {
    let mut out = Vec::with_capacity(chunk.len());
    for (i, frame) in chunk.as_chunks::<FRAME_BYTES>().0.iter().enumerate() {
        let gain = ((from_frame + i) as f32 / FADE_FRAMES as f32).min(1.0);
        for sample in frame.as_chunks::<2>().0 {
            let v = f32::from(i16::from_le_bytes([sample[0], sample[1]])) * gain;
            out.extend_from_slice(&(v.round() as i16).to_le_bytes());
        }
    }
    Bytes::from(out)
}

/// Fade in, the speech, fade out.
fn window_frames(speech: &Speech) -> usize {
    2 * FADE_FRAMES + speech.pcm.len() / FRAME_BYTES
}

/// The music's gain at window frame `f`: 1 outside, ramping to [`DUCK_GAIN`]
/// over the first `FADE_FRAMES` and back over the last.
fn music_gain(f: i64, window: i64) -> f32 {
    let fade = FADE_FRAMES as i64;
    let depth = 1.0 - DUCK_GAIN;
    if f < 0 || f >= window {
        1.0
    } else if f < fade {
        1.0 - depth * f as f32 / fade as f32
    } else if f >= window - fade {
        DUCK_GAIN + depth * (f - (window - fade)) as f32 / fade as f32
    } else {
        DUCK_GAIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pacer::{CHUNK_BYTES, CHUNK_US};

    const START: i64 = 1_000_000;

    fn constant(value: i16, frames: usize) -> Vec<u8> {
        std::iter::repeat_n(value.to_le_bytes(), frames * 2)
            .flatten()
            .collect()
    }

    fn samples(chunk: &[u8]) -> Vec<i16> {
        chunk
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect()
    }

    /// One chunk of speech at 8 000, for satellite "a".
    fn sentence() -> Speech {
        Speech {
            players: vec!["a".into()],
            pcm: Bytes::from(constant(8_000, CHUNK_FRAMES)),
        }
    }

    fn playing() -> SpeechMixer {
        let mut m = SpeechMixer::new();
        assert!(m.push(sentence()));
        assert_eq!(m.start_next(START), Some(&["a".to_owned()][..]));
        m
    }

    /// Speech starts after the fade-in: 150 ms into the window.
    const SPEECH_AT: i64 = START + 150_000;

    #[test]
    fn before_the_window_music_is_untouched() {
        let music = constant(10_000, CHUNK_FRAMES);
        assert_eq!(playing().mix_chunk(START - CHUNK_US, Some(&music)), None);
    }

    #[test]
    fn under_speech_music_is_ducked_and_speech_added() {
        let music = constant(10_000, CHUNK_FRAMES);
        let chunk = playing().mix_chunk(SPEECH_AT, Some(&music)).unwrap();
        assert_eq!(chunk.len(), CHUNK_BYTES);
        assert!(
            samples(&chunk).iter().all(|&s| s == 9_778),
            "{:?}",
            &samples(&chunk)[..4]
        );
    }

    #[test]
    fn fade_starts_at_full_music() {
        let music = constant(10_000, CHUNK_FRAMES);
        let chunk = playing().mix_chunk(START, Some(&music)).unwrap();
        assert_eq!(&samples(&chunk)[..2], [10_000, 10_000]);
        // ...and is going down by the end of that first chunk.
        assert!(samples(&chunk)[CHUNK_FRAMES * 2 - 1] < 10_000);
    }

    #[test]
    fn after_the_window_nothing() {
        let mut m = playing();
        let end = m.end_us().unwrap();
        assert_eq!(end, START + 320_000);
        assert_eq!(m.mix_chunk(end, None), None);
        assert!(!m.finish_if_done(end - 1));
        assert!(m.finish_if_done(end));
        assert!(!m.is_playing());
        assert!(m.targets().is_empty());
    }

    #[test]
    fn speech_alone_without_music() {
        let chunk = playing().mix_chunk(SPEECH_AT, None).unwrap();
        assert!(samples(&chunk).iter().all(|&s| s == 8_000));
    }

    #[test]
    fn loud_mix_saturates() {
        let mut m = SpeechMixer::new();
        m.push(Speech {
            players: vec!["a".into()],
            pcm: Bytes::from(constant(30_000, CHUNK_FRAMES)),
        });
        m.start_next(START);
        let music = constant(32_000, CHUNK_FRAMES);
        // 32 000 × 0.17783 + 30 000 > i16::MAX.
        let chunk = m.mix_chunk(SPEECH_AT, Some(&music)).unwrap();
        assert!(samples(&chunk).iter().all(|&s| s == i16::MAX));
    }

    #[test]
    fn queue_is_bounded() {
        let mut m = playing();
        for _ in 0..MAX_QUEUED {
            assert!(m.push(sentence()));
        }
        assert!(!m.push(sentence()));
        assert!(m.has_queued());
    }

    #[test]
    fn next_sentence_waits() {
        let mut m = playing();
        m.push(sentence());
        assert_eq!(m.start_next(START + 10_000), None);
        let end = m.end_us().unwrap();
        assert!(m.finish_if_done(end));
        assert!(m.start_next(end).is_some());
        assert!(!m.has_queued());
    }

    #[test]
    fn an_idle_mixer_mixes_nothing() {
        let m = SpeechMixer::new();
        assert!(!m.is_playing());
        assert_eq!(m.end_us(), None);
        assert_eq!(m.mix_chunk(START, None), None);
        assert_eq!(FRAME_BYTES, 4);
    }

    #[test]
    fn fade_in_ramps_from_silence() {
        let chunk: Vec<u8> = std::iter::repeat_n(10_000i16.to_le_bytes(), CHUNK_FRAMES * 2)
            .flatten()
            .collect();
        let sample = |b: &[u8], frame: usize| i16::from_le_bytes([b[frame * 4], b[frame * 4 + 1]]);
        let start = fade_in(&chunk, 0);
        assert_eq!(sample(&start, 0), 0);
        assert!(sample(&start, CHUNK_FRAMES - 1) < 10_000);
        let half = fade_in(&chunk, FADE_FRAMES / 2);
        assert!(
            (sample(&half, 0) - 5_000).abs() <= 1,
            "{}",
            sample(&half, 0)
        );
        assert_eq!(fade_in(&chunk, FADE_FRAMES), Bytes::from(chunk.clone()));
    }
}
