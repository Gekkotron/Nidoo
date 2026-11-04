//! A test tone (s16le, 48 kHz, stereo) for the hardware spike.

use std::f64::consts::TAU;

/// Frames in one 20 ms chunk at 48 kHz.
pub const CHUNK_FRAMES: u64 = 960;
const SAMPLE_RATE: f64 = 48_000.0;

/// One 20 ms chunk of a sine at `freq` Hz starting at frame `start`;
/// `amplitude` is a fraction of full scale.
pub fn tone_chunk(freq: f64, start: u64, amplitude: f64) -> Vec<u8> {
    tone_chunk_frames(freq, start, CHUNK_FRAMES, amplitude)
}

/// `frames` frames of the sine, starting at frame `start`.
pub fn tone_chunk_frames(freq: f64, start: u64, frames: u64, amplitude: f64) -> Vec<u8> {
    let scale = amplitude.clamp(0.0, 1.0) * f64::from(i16::MAX);
    let mut out = Vec::with_capacity(frames as usize * 4);
    for n in start..start + frames {
        let v = ((TAU * freq * n as f64 / SAMPLE_RATE).sin() * scale).round() as i16;
        let b = v.to_le_bytes();
        out.extend_from_slice(&[b[0], b[1], b[0], b[1]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_chunk_is_20ms_of_stereo_s16le() {
        let c = tone_chunk(440.0, 0, 0.5);
        assert_eq!(c.len(), 3840);
        // Frame 0 is silent (sin 0); left == right everywhere.
        assert_eq!(&c[0..4], &[0, 0, 0, 0]);
        let (frames, rest) = c.as_chunks::<4>();
        assert!(rest.is_empty());
        for f in frames {
            assert_eq!(f[0..2], f[2..4]);
        }
        // Peak close to half scale.
        let peak = frames
            .iter()
            .map(|f| i16::from_le_bytes([f[0], f[1]]).unsigned_abs())
            .max()
            .unwrap();
        assert!((16_000..=16_384).contains(&peak), "peak {peak}");
    }

    #[test]
    fn tone_chunks_are_continuous() {
        let a = tone_chunk(440.0, 0, 0.5);
        let b = tone_chunk(440.0, 960, 0.5);
        let both = tone_chunk_frames(440.0, 0, 1920, 0.5);
        assert_eq!([a, b].concat(), both);
    }
}
