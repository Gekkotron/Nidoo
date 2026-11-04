//! Turns the phone's PCM byte stream into timestamped 20 ms chunks on the
//! server's clock.
//!
//! The timeline starts `lead` ahead of the clock and steps 20 ms per chunk;
//! a chunk is due `lead` before its timestamp. A chunk with no PCM buffered is
//! filled with silence, so players never underrun on a Wi-Fi hiccup. When
//! more than [`MAX_QUEUED_CHUNKS`] chunks of PCM pile up (the phone's clock
//! running fast, or a burst after a gap), one frame is dropped per chunk until
//! the queue is back under the limit.

use bytes::{Buf, Bytes, BytesMut};

/// s16le stereo.
pub const FRAME_BYTES: usize = 4;
/// 20 ms at 48 kHz.
pub const CHUNK_FRAMES: usize = 960;
pub const CHUNK_BYTES: usize = CHUNK_FRAMES * FRAME_BYTES;
pub const CHUNK_US: i64 = 20_000;
/// PCM queued beyond the timeline before frames are dropped.
pub const MAX_QUEUED_CHUNKS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub timestamp_us: i64,
    /// Always [`CHUNK_BYTES`] of s16le 48 kHz stereo.
    pub data: Bytes,
}

#[derive(Debug)]
pub struct Pacer {
    lead_us: i64,
    /// Timestamp of the next chunk; `None` while stopped.
    next_ts: Option<i64>,
    queue: BytesMut,
    /// Bytes of an incomplete frame, carried to the next push.
    partial: Vec<u8>,
}

impl Pacer {
    pub fn new(lead_us: i64) -> Self {
        Self {
            lead_us,
            next_ts: None,
            queue: BytesMut::new(),
            partial: Vec::new(),
        }
    }

    /// Starts a new timeline: the first chunk plays at `now + lead`.
    pub fn start(&mut self, now_us: i64) {
        self.next_ts = Some(now_us + self.lead_us);
    }

    pub fn stop(&mut self) {
        self.next_ts = None;
        self.queue.clear();
        self.partial.clear();
    }

    pub fn is_running(&self) -> bool {
        self.next_ts.is_some()
    }

    /// Whole-frame PCM bytes waiting to be sent.
    pub fn queued_bytes(&self) -> usize {
        self.queue.len()
    }

    /// Queues PCM; never splits a frame.
    pub fn push(&mut self, pcm: &[u8]) {
        if self.next_ts.is_none() {
            return;
        }
        self.partial.extend_from_slice(pcm);
        let whole = self.partial.len() - self.partial.len() % FRAME_BYTES;
        self.queue.extend_from_slice(&self.partial[..whole]);
        self.partial.drain(..whole);
    }

    /// Chunks whose send time (`timestamp - lead`) has come, in order.
    pub fn due_chunks(&mut self, now_us: i64) -> Vec<Chunk> {
        let mut out = Vec::new();
        while let Some(ts) = self.next_ts {
            if ts - self.lead_us > now_us {
                break;
            }
            if self.queue.len() > MAX_QUEUED_CHUNKS * CHUNK_BYTES {
                self.queue.advance(FRAME_BYTES);
            }
            let take = self.queue.len().min(CHUNK_BYTES);
            let mut data = self.queue.split_to(take);
            data.resize(CHUNK_BYTES, 0);
            out.push(Chunk {
                timestamp_us: ts,
                data: data.freeze(),
            });
            self.next_ts = Some(ts + CHUNK_US);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(bytes: usize, value: u8) -> Vec<u8> {
        vec![value; bytes]
    }

    #[test]
    fn timeline_starts_lead_ahead_and_steps_20ms() {
        let mut p = Pacer::new(500_000);
        p.start(1_000_000);
        p.push(&pcm(7680, 1));
        let c = p.due_chunks(1_000_000);
        assert_eq!(
            c.iter().map(|c| c.timestamp_us).collect::<Vec<_>>(),
            [1_500_000]
        );
        assert_eq!(c[0].data.len(), CHUNK_BYTES);
        assert!(c[0].data.iter().all(|&b| b == 1));
        assert!(p.due_chunks(1_010_000).is_empty());
        let c = p.due_chunks(1_020_000);
        assert_eq!(
            c.iter().map(|c| c.timestamp_us).collect::<Vec<_>>(),
            [1_520_000]
        );
    }

    #[test]
    fn late_polls_catch_up_in_order() {
        let mut p = Pacer::new(500_000);
        p.start(0);
        let c = p.due_chunks(60_000);
        assert_eq!(
            c.iter().map(|c| c.timestamp_us).collect::<Vec<_>>(),
            [500_000, 520_000, 540_000, 560_000]
        );
    }

    #[test]
    fn short_gap_is_silence_filled() {
        let mut p = Pacer::new(500_000);
        p.start(0);
        p.push(&pcm(CHUNK_BYTES, 7));
        let c = p.due_chunks(40_000);
        assert_eq!(c.len(), 3);
        assert!(c[0].data.iter().all(|&b| b == 7));
        assert!(c[1].data.iter().all(|&b| b == 0) && c[1].data.len() == CHUNK_BYTES);
        assert!(c[2].data.iter().all(|&b| b == 0));
    }

    #[test]
    fn partial_chunk_is_padded_with_silence() {
        let mut p = Pacer::new(0);
        p.start(0);
        p.push(&pcm(400, 9));
        let c = p.due_chunks(0);
        assert_eq!(&c[0].data[..400], &pcm(400, 9)[..]);
        assert!(c[0].data[400..].iter().all(|&b| b == 0));
    }

    #[test]
    fn excess_buffer_drops_frames() {
        let mut p = Pacer::new(0);
        p.start(0);
        p.push(&pcm(10 * CHUNK_BYTES, 1));
        let mut now = 0;
        let mut queued = p.queued_bytes();
        while queued > MAX_QUEUED_CHUNKS * CHUNK_BYTES {
            let c = p.due_chunks(now);
            assert_eq!(c.len(), 1);
            assert_eq!(c[0].data.len(), CHUNK_BYTES);
            assert_eq!(p.queued_bytes(), queued - CHUNK_BYTES - FRAME_BYTES);
            queued = p.queued_bytes();
            now += CHUNK_US;
        }
        // At or under the limit, nothing more is dropped.
        let c = p.due_chunks(now);
        assert_eq!(c.len(), 1);
        assert_eq!(p.queued_bytes(), queued - CHUNK_BYTES);
    }

    #[test]
    fn partial_frames_carry_over() {
        let mut p = Pacer::new(0);
        p.start(0);
        p.push(&[1, 2, 3]);
        assert_eq!(p.queued_bytes(), 0);
        p.push(&[4]);
        assert_eq!(p.queued_bytes(), 4);
        let c = p.due_chunks(0);
        assert_eq!(&c[0].data[..4], &[1, 2, 3, 4]);
    }

    #[test]
    fn stopped_pacer_emits_nothing_and_restart_starts_a_new_timeline() {
        let mut p = Pacer::new(500_000);
        p.start(0);
        p.push(&pcm(CHUNK_BYTES, 1));
        p.stop();
        assert!(p.due_chunks(1_000_000).is_empty());
        assert_eq!(p.queued_bytes(), 0);
        p.start(2_000_000);
        assert_eq!(p.due_chunks(2_000_000)[0].timestamp_us, 2_500_000);
    }
}
