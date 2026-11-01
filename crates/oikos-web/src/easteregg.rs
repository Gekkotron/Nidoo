//! The logo's Easter egg: double-click the bird and it sings.
//!
//! A short songbird call, synthesized with Web Audio: two rising whistles ("tsee-tseet") then a
//! quick trill. Every note is one sine gliding between two pitches under its own envelope.
//! The context is created inside the gesture, so autoplay policies let it sound.

/// One note: a sine that glides from `from` to `to` Hz over `glide` seconds, starting `at`
/// seconds into the call, under an attack / hold / decay envelope peaking at `peak`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub at: f64,
    pub from: f32,
    pub to: f32,
    pub glide: f64,
    pub peak: f32,
    pub attack: f64,
    pub hold: f64,
    pub decay: f64,
}

impl Note {
    /// When the note has faded out, in seconds from the start of the call.
    pub fn end(&self) -> f64 {
        self.at + self.attack + self.hold + self.decay
    }
}

/// The two rising whistles, "tsee" then a slightly higher, longer "tseet".
pub const WHISTLES: [Note; 2] = [
    Note {
        at: 0.0,
        from: 2300.0,
        to: 3500.0,
        glide: 0.09,
        peak: 0.049,
        attack: 0.01,
        hold: 0.0,
        decay: 0.08,
    },
    Note {
        at: 0.17,
        from: 2500.0,
        to: 3900.0,
        glide: 0.11,
        peak: 0.056,
        attack: 0.01,
        hold: 0.02,
        decay: 0.09,
    },
];

/// The trill after the whistles: `TRILL_COUNT` short notes `TRILL_STEP` seconds apart from
/// `TRILL_START`, alternating `TRILL_HIGH` and `TRILL_LOW` (in Hz), each drooping to `TRILL_DROOP`
/// of its pitch, at `TRILL_PEAK`.
pub const TRILL_START: f64 = 0.45;
pub const TRILL_STEP: f64 = 0.055;
pub const TRILL_COUNT: usize = 7;
pub const TRILL_HIGH: f32 = 3800.0;
pub const TRILL_LOW: f32 = 3300.0;
pub const TRILL_DROOP: f32 = 0.92;
pub const TRILL_PEAK: f32 = 0.0385;

/// How long the whole call lasts, after which the audio context is closed.
pub const DURATION_MS: u32 = 1200;

/// Every note of the call, whistles first, then the trill in order.
pub fn notes() -> Vec<Note> {
    let trill = (0..TRILL_COUNT).map(|i| {
        let pitch = if i % 2 == 0 { TRILL_HIGH } else { TRILL_LOW };
        Note {
            at: TRILL_START + TRILL_STEP * i as f64,
            from: pitch,
            to: pitch * TRILL_DROOP,
            glide: 0.04,
            peak: TRILL_PEAK,
            attack: 0.004,
            hold: 0.0,
            decay: 0.045,
        }
    });
    WHISTLES.into_iter().chain(trill).collect()
}

/// Plays the call. Audio is a nicety: any failure is silent.
pub fn play() {
    #[cfg(target_arch = "wasm32")]
    let _ = imp::play();
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{DURATION_MS, notes};
    use wasm_bindgen::JsValue;
    use web_sys::{AudioContext, OscillatorType};

    /// The envelope starts and ends here: exponential ramps cannot reach zero.
    const SILENT: f32 = 0.0001;

    pub fn play() -> Result<(), JsValue> {
        let ctx = AudioContext::new()?;
        let t0 = ctx.current_time() + 0.02;
        for note in notes() {
            let at = t0 + note.at;
            let osc = ctx.create_oscillator()?;
            osc.set_type(OscillatorType::Sine);
            osc.frequency().set_value_at_time(note.from, at)?;
            osc.frequency()
                .exponential_ramp_to_value_at_time(note.to, at + note.glide)?;
            let env = ctx.create_gain()?;
            let sustain = at + note.attack + note.hold;
            env.gain().set_value_at_time(SILENT, at)?;
            env.gain()
                .exponential_ramp_to_value_at_time(note.peak, at + note.attack)?;
            env.gain().set_value_at_time(note.peak, sustain)?;
            env.gain()
                .exponential_ramp_to_value_at_time(SILENT, t0 + note.end())?;
            osc.connect_with_audio_node(&env)?;
            env.connect_with_audio_node(&ctx.destination())?;
            osc.start_with_when(at)?;
            osc.stop_with_when(t0 + note.end() + 0.05)?;
        }
        gloo_timers::callback::Timeout::new(DURATION_MS, move || {
            let _ = ctx.close();
        })
        .forget();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whistles_rise_and_the_second_climbs_higher() {
        let [tsee, tseet] = WHISTLES;
        assert!(tsee.to > tsee.from && tseet.to > tseet.from);
        assert!(tseet.to > tsee.to);
        assert!(tseet.at >= tsee.end(), "the whistles do not overlap");
    }

    #[test]
    fn the_trill_follows_the_whistles_and_alternates_high_and_low() {
        let all = notes();
        assert_eq!(all.len(), WHISTLES.len() + TRILL_COUNT);
        let trill = &all[WHISTLES.len()..];
        assert!(trill[0].at >= WHISTLES[1].end());
        for pair in trill.windows(2) {
            assert!(pair[1].at > pair[0].at);
            assert_ne!(pair[0].from, pair[1].from);
        }
    }

    #[test]
    fn the_call_is_a_quiet_bird_range_chirp_that_ends_before_the_context_closes() {
        let all = notes();
        // The start lead (0.02 s) and the oscillator stop margin (0.05 s) come on top.
        let last = all.iter().map(Note::end).fold(0.0, f64::max) + 0.02 + 0.05;
        assert!(last * 1000.0 < f64::from(DURATION_MS));
        for note in &all {
            assert!(note.peak > 0.0 && note.peak <= 0.06, "{note:?} is too loud");
            for hz in [note.from, note.to] {
                assert!(
                    (2000.0..=4000.0).contains(&hz),
                    "{note:?} leaves the bird range"
                );
            }
            assert!(note.glide <= note.end() - note.at);
        }
    }
}
