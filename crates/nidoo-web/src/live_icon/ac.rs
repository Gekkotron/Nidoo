//! The AC icon's maths: the tone of each mode, the airflow's pace from the fan mode, the vane's
//! swing and the drops of dry mode. All positions are in the icon's 24-unit drawing.

use std::f64::consts::TAU;

/// The colour family the AC's air is drawn in; each maps to a theme token through its class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcTone {
    Cool,
    Heat,
    Auto,
    Plain,
}

impl AcTone {
    /// The class on the icon's `<svg>` that sets `--live-tone`.
    pub fn class(self) -> &'static str {
        match self {
            AcTone::Cool => "live-icon--cool",
            AcTone::Heat => "live-icon--heat",
            AcTone::Auto => "live-icon--auto",
            AcTone::Plain => "live-icon--plain",
        }
    }
}

/// The tone of a running AC, `None` when it is off or unreachable (a stale mode on an offline
/// unit must not blow air).
pub fn ac_tone(mode: Option<&str>, available: bool) -> Option<AcTone> {
    if !available {
        return None;
    }
    let mode = mode?.to_ascii_lowercase();
    Some(match mode.as_str() {
        "off" => return None,
        "cool" | "dry" => AcTone::Cool,
        "heat" => AcTone::Heat,
        "fan_only" => AcTone::Plain,
        _ => AcTone::Auto,
    })
}

/// Whether the AC is drying: it drips instead of blowing.
pub fn is_dry(mode: Option<&str>) -> bool {
    mode.is_some_and(|m| m.eq_ignore_ascii_case("dry"))
}

/// One airflow loop, in ms, for a fan mode: the faster the fan, the shorter the loop.
pub fn airflow_period_ms(fan_mode: Option<&str>) -> f64 {
    let fan = fan_mode.map(str::to_ascii_lowercase);
    match fan.as_deref() {
        Some("silent" | "silence" | "quiet" | "low" | "1") => 2400.0,
        Some("2") => 2000.0,
        Some("medium" | "med" | "3") => 1600.0,
        Some("4") => 1300.0,
        Some("high" | "5") => 1000.0,
        _ => 1800.0,
    }
}

/// One full swing of the vane.
pub const VANE_PERIOD_MS: f64 = 3200.0;
/// One drop's fall in dry mode.
pub const DROPS_PERIOD_MS: f64 = 1400.0;
/// The vane's resting line, below the unit's outline.
pub const VANE_REST_Y: f64 = 14.4;
/// How far the vane swings either side of its rest.
pub const VANE_SWING: f64 = 0.35;

/// The vane's line at `phase` (0..1) of its swing.
pub fn vane_y(phase: f64) -> f64 {
    VANE_REST_Y + VANE_SWING * (TAU * phase).sin()
}

/// The airflow's dash offset at `phase`: two dash cycles (`2.6 2.4`) downward per loop.
pub fn flow_dash_offset(phase: f64) -> f64 {
    -10.0 * phase
}

/// A drop at `phase` of its fall: how far it has fallen and its opacity. It fades in over the
/// first 15 %, then falls faster and faster as it fades out.
pub fn drop_fall(phase: f64) -> (f64, f64) {
    let opacity = if phase < 0.15 {
        phase / 0.15
    } else {
        (1.0 - phase) / 0.85
    };
    (7.0 * phase * phase, opacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_map_to_tones() {
        assert_eq!(ac_tone(Some("cool"), true), Some(AcTone::Cool));
        assert_eq!(ac_tone(Some("dry"), true), Some(AcTone::Cool));
        assert_eq!(ac_tone(Some("heat"), true), Some(AcTone::Heat));
        assert_eq!(ac_tone(Some("fan_only"), true), Some(AcTone::Plain));
        assert_eq!(ac_tone(Some("heat_cool"), true), Some(AcTone::Auto));
        assert_eq!(ac_tone(Some("auto"), true), Some(AcTone::Auto));
        assert_eq!(ac_tone(Some("off"), true), None);
        assert_eq!(ac_tone(None, true), None);
    }

    #[test]
    fn offline_is_off() {
        assert_eq!(ac_tone(Some("cool"), false), None);
    }

    #[test]
    fn airflow_periods_follow_the_fan() {
        for (fan, ms) in [
            ("quiet", 2400.0),
            ("low", 2400.0),
            ("1", 2400.0),
            ("2", 2000.0),
            ("medium", 1600.0),
            ("med", 1600.0),
            ("3", 1600.0),
            ("4", 1300.0),
            ("high", 1000.0),
            ("5", 1000.0),
            ("auto", 1800.0),
        ] {
            assert_eq!(airflow_period_ms(Some(fan)), ms, "{fan}");
        }
        assert_eq!(airflow_period_ms(None), 1800.0);
    }

    #[test]
    fn periods_ignore_case() {
        assert_eq!(airflow_period_ms(Some("HIGH")), 1000.0);
        assert_eq!(airflow_period_ms(Some("Silent")), 2400.0);
    }

    #[test]
    fn vane_swings_below_the_unit() {
        assert!((vane_y(0.0) - 14.4).abs() < 1e-9);
        assert!((vane_y(0.25) - 14.75).abs() < 1e-9);
        assert!((vane_y(0.75) - 14.05).abs() < 1e-9);
        // The top edge of the 1.3 vane stays clear of the unit's bottom edge (12.4 + 1.75 / 2).
        let top = (0..100)
            .map(|i| vane_y(f64::from(i) / 100.0))
            .fold(f64::MAX, f64::min)
            - 0.65;
        assert!(top > 12.4 + 0.875, "{top}");
    }

    #[test]
    fn flow_moves_down_one_dash_cycle_per_loop() {
        assert_eq!(flow_dash_offset(0.0), 0.0);
        assert!((flow_dash_offset(0.5) + 5.0).abs() < 1e-9);
    }

    #[test]
    fn drops_fade_in_fall_and_fade_out() {
        assert_eq!(drop_fall(0.0), (0.0, 0.0));
        let (_, o) = drop_fall(0.15);
        assert!((o - 1.0).abs() < 1e-9);
        let (dy, o) = drop_fall(1.0);
        assert!((dy - 7.0).abs() < 1e-9 && o.abs() < 1e-9);
    }

    #[test]
    fn tones_have_their_classes() {
        assert_eq!(AcTone::Heat.class(), "live-icon--heat");
    }
}
