//! One shared curve for a satellite's Music and Speech volumes.
//!
//! A satellite has a single hardware volume. The louder of the two settings
//! sets it; the quieter side is scaled down digitally by the difference, on
//! the same curve the device uses (49 dB across the 0–100 range).

/// Decibels across the full 0–100 range.
const RANGE_DB: f32 = 49.0;

/// What to send to the device and how to scale each source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    /// The satellite's hardware volume: the louder of the two settings.
    pub device: u8,
    /// Linear gain for music (1.0 when music is the louder side).
    pub music_gain: f32,
    /// Linear gain for speech (1.0 when speech is the louder side).
    pub speech_gain: f32,
}

/// Splits a Music and a Speech setting (0–100, clamped) into the device
/// volume and one gain per source. A side at 0 is silent.
pub fn split(music: u8, speech: u8) -> Levels {
    let (music, speech) = (music.min(100), speech.min(100));
    let device = music.max(speech);
    let gain = |own: u8| {
        if own == device {
            1.0
        } else if own == 0 {
            0.0
        } else {
            let db = -RANGE_DB * f32::from(device - own) / 100.0;
            10f32.powf(db / 20.0)
        }
    };
    Levels {
        device,
        music_gain: gain(music),
        speech_gain: gain(speech),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_settings_change_nothing() {
        assert_eq!(
            split(40, 40),
            Levels {
                device: 40,
                music_gain: 1.0,
                speech_gain: 1.0
            }
        );
    }

    #[test]
    fn speech_louder_scales_music() {
        let l = split(30, 80);
        assert_eq!(l.device, 80);
        assert_eq!(l.speech_gain, 1.0);
        assert!((l.music_gain - 10f32.powf(-24.5 / 20.0)).abs() < 1e-4);
    }

    #[test]
    fn music_louder_scales_speech() {
        let l = split(90, 40);
        assert_eq!(l.device, 90);
        assert_eq!(l.music_gain, 1.0);
        assert!((l.speech_gain - 10f32.powf(-24.5 / 20.0)).abs() < 1e-4);
    }

    #[test]
    fn zero_is_silence() {
        assert_eq!(split(0, 60).music_gain, 0.0);
        assert_eq!(split(60, 0).speech_gain, 0.0);
        assert_eq!(split(0, 0).device, 0);
    }

    #[test]
    fn over_100_is_clamped() {
        assert_eq!(split(150, 100), split(100, 100));
    }
}
