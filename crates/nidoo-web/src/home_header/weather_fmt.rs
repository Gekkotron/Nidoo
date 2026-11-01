//! Weather formatting helpers: when the card shows, which icon, compass sector, UV band.

use nidoo_api::{WeatherCondition, WeatherView};

use crate::icons;

/// Age, in seconds, past which the weather card is hidden (6 h).
pub const WEATHER_MAX_AGE_S: u64 = 21_600;

/// A UV index band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvLevel {
    Low,
    Moderate,
    High,
    VeryHigh,
    Extreme,
}

/// Whether the weather card shows: there is data and it is at most 6 h old.
pub fn weather_card_visible(w: Option<&WeatherView>, now_s: u64) -> bool {
    w.is_some_and(|w| now_s.saturating_sub(w.fetched_at) <= WEATHER_MAX_AGE_S)
}

/// The icon for a condition; `daylight` picks the day or night variant where there is one.
pub fn condition_icon(c: WeatherCondition, daylight: bool) -> &'static str {
    match (c, daylight) {
        (WeatherCondition::Clear, true) => icons::SUN,
        (WeatherCondition::Clear, false) => icons::MOON,
        (WeatherCondition::PartlyCloudy, true) => icons::PARTLY_CLOUDY_DAY,
        (WeatherCondition::PartlyCloudy, false) => icons::PARTLY_CLOUDY_NIGHT,
        (WeatherCondition::Cloudy, _) => icons::CLOUDY,
        (WeatherCondition::Fog, _) => icons::FOG,
        (WeatherCondition::Drizzle, _) => icons::DRIZZLE,
        (WeatherCondition::Rain, _) => icons::RAIN,
        (WeatherCondition::FreezingRain, _) => icons::FREEZING_RAIN,
        (WeatherCondition::Snow, _) => icons::SNOW,
        (WeatherCondition::Showers, _) => icons::SHOWERS,
        (WeatherCondition::Thunderstorm, _) => icons::THUNDERSTORM,
    }
}

/// The compass point (0 = N, clockwise, 8 points) a bearing in degrees falls in.
pub fn compass_index(deg: f64) -> usize {
    ((deg.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8
}

/// The UV band of an index.
pub fn uv_level(uv: f64) -> UvLevel {
    match uv {
        v if v < 3.0 => UvLevel::Low,
        v if v < 6.0 => UvLevel::Moderate,
        v if v < 8.0 => UvLevel::High,
        v if v < 11.0 => UvLevel::VeryHigh,
        _ => UvLevel::Extreme,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, compass_word, condition_label, uv_word};

    fn weather(fetched_at: u64) -> WeatherView {
        serde_json::from_value(serde_json::json!({
            "place": {"name": "X"},
            "fetched_at": fetched_at, "stale": false,
            "current": {}, "today": {},
            "rain_next_hour": null, "tonight_min": null, "days": []
        }))
        .expect("weather fixture")
    }

    #[test]
    fn card_hidden_when_old_or_missing() {
        let now = 100_000;
        assert!(!weather_card_visible(None, now));
        assert!(weather_card_visible(Some(&weather(now)), now));
        assert!(weather_card_visible(Some(&weather(now - 21_600)), now));
        assert!(!weather_card_visible(Some(&weather(now - 21_601)), now));
    }

    #[test]
    fn compass_sectors() {
        assert_eq!(compass_index(0.0), 0);
        assert_eq!(compass_index(22.4), 0);
        assert_eq!(compass_index(22.5), 1);
        assert_eq!(compass_index(246.0), 5);
        assert_eq!(compass_index(350.0), 0);
    }

    #[test]
    fn uv_bands() {
        assert_eq!(uv_level(2.9), UvLevel::Low);
        assert_eq!(uv_level(3.0), UvLevel::Moderate);
        assert_eq!(uv_level(7.9), UvLevel::High);
        assert_eq!(uv_level(8.0), UvLevel::VeryHigh);
        assert_eq!(uv_level(11.0), UvLevel::Extreme);
    }

    #[test]
    fn every_condition_has_icon_and_labels() {
        for c in WeatherCondition::ALL {
            for daylight in [true, false] {
                assert!(condition_icon(c, daylight).starts_with("<svg"), "{c:?}");
            }
            for lang in [Lang::En, Lang::Fr] {
                assert!(!condition_label(lang, c).is_empty(), "{c:?}");
            }
        }
        for i in 0..8 {
            for lang in [Lang::En, Lang::Fr] {
                assert!(!compass_word(lang, i).is_empty());
            }
        }
        for lang in [Lang::En, Lang::Fr] {
            for l in [
                UvLevel::Low,
                UvLevel::Moderate,
                UvLevel::High,
                UvLevel::VeryHigh,
                UvLevel::Extreme,
            ] {
                assert!(!uv_word(lang, l).is_empty());
            }
        }
    }
}
