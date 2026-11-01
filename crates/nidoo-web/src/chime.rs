//! The logo's Easter egg: double-click it and someone rings at the door.
//!
//! A two-tone "ding-dong" doorbell, synthesized with Web Audio: each strike is a tubular chime,
//! a fundamental plus a faster-decaying octave and a faint inharmonic partial for the metal.
//! The context is created inside the gesture, so autoplay policies let it sound.

/// The strikes, as (pitch in Hz, start offset in seconds): E5 then C5, the classic doorbell.
pub const STRIKES: [(f32, f64); 2] = [(659.25, 0.0), (523.25, 0.55)];

/// Partials of one strike, as (ratio to the pitch, peak gain, decay in seconds).
pub const PARTIALS: [(f32, f32, f64); 3] = [(1.0, 0.16, 1.6), (2.0, 0.05, 0.7), (2.76, 0.025, 0.4)];

/// How long the whole chime rings, after which the audio context is closed.
pub const DURATION_MS: u32 = 2400;

/// Rings the doorbell. Audio is a nicety: any failure is silent.
pub fn ring() {
    #[cfg(target_arch = "wasm32")]
    let _ = imp::ring();
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{DURATION_MS, PARTIALS, STRIKES};
    use wasm_bindgen::JsValue;
    use web_sys::{AudioContext, OscillatorType};

    pub fn ring() -> Result<(), JsValue> {
        let ctx = AudioContext::new()?;
        let t0 = ctx.current_time() + 0.02;
        for (pitch, offset) in STRIKES {
            for (ratio, peak, decay) in PARTIALS {
                let at = t0 + offset;
                let osc = ctx.create_oscillator()?;
                osc.set_type(OscillatorType::Sine);
                osc.frequency().set_value(pitch * ratio);
                let env = ctx.create_gain()?;
                env.gain().set_value_at_time(0.0001, at)?;
                env.gain()
                    .exponential_ramp_to_value_at_time(peak, at + 0.005)?;
                env.gain()
                    .exponential_ramp_to_value_at_time(0.0001, at + decay)?;
                osc.connect_with_audio_node(&env)?;
                env.connect_with_audio_node(&ctx.destination())?;
                osc.start_with_when(at)?;
                osc.stop_with_when(at + decay + 0.05)?;
            }
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
    fn the_chime_falls_a_major_third_and_ends_before_the_context_closes() {
        let (ding, dong) = (STRIKES[0], STRIKES[1]);
        assert!((ding.0 / dong.0 - 1.26).abs() < 0.01);
        let last = dong.1 + PARTIALS.iter().map(|p| p.2).fold(0.0, f64::max);
        assert!(last * 1000.0 < f64::from(DURATION_MS));
    }
}
