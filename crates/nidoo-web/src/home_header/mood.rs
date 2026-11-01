//! The header's words: greeting band, activity, temperature band and the detail fragments.

use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};
use nidoo_api::{FlowsConfigView, HomeSettings, WeatherView};
use nidoo_core::flow::sun::{is_dark_at, sun_events};

use super::summary::{HouseSummary, Opening};
use crate::screens::flow_canvas::sun::house_place;

/// °C below which the house reads cold when the settings leave it unset.
pub const COLD_DEFAULT: f64 = 15.0;
/// °C above which the house reads warm when the settings leave it unset.
pub const HOT_DEFAULT: f64 = 26.0;

/// mm of rain in the next hour above which the detail line mentions it.
pub(crate) const RAIN_MIN_MM: f64 = 0.1;
/// Local hour from which a freezing night is worth a mention.
const FROST_FROM_HOUR: u32 = 15;
/// How long before sunset the detail line starts naming it.
const SUNSET_LEAD: Duration = Duration::hours(2);
/// Most fragments the detail line shows.
const MAX_DETAILS: usize = 2;

/// The greeting for the hour of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Greeting {
    Morning,
    Afternoon,
    Evening,
}

/// What the house is doing: the first half of the mood sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    Attention,
    Open(Opening),
    OpenMany(usize),
    Asleep,
    Quiet,
    Awake,
}

/// How the indoor temperature reads: the second half of the mood sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TempBand {
    Cold,
    Comfortable,
    Warm,
}

/// One fragment of the detail line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    RainSoon,
    FrostTonight,
    Sunset(NaiveTime),
    /// The house has no coordinates, so no sunset time can be given.
    SunsetFallback,
}

/// The greeting for a local hour (0..=23).
pub fn greeting(hour: u32) -> Greeting {
    match hour {
        5..=11 => Greeting::Morning,
        12..=17 => Greeting::Afternoon,
        _ => Greeting::Evening,
    }
}

/// What the house is doing, first matching row of the activity table.
pub fn activity(s: &HouseSummary, dark: bool, hour: u32) -> Activity {
    if s.unseen_alerts > 0 {
        return Activity::Attention;
    }
    match s.openings.as_slice() {
        [] => {}
        [one] => return Activity::Open(one.clone()),
        many => return Activity::OpenMany(many.len()),
    }
    if s.lights_on > 0 {
        return Activity::Awake;
    }
    if dark && !(6..22).contains(&hour) {
        Activity::Asleep
    } else {
        Activity::Quiet
    }
}

/// Cold below the lower threshold, warm above the upper one; both edges are comfortable.
pub fn temp_band(t: f64, settings: &HomeSettings) -> TempBand {
    if t < settings.cold_below.unwrap_or(COLD_DEFAULT) {
        TempBand::Cold
    } else if t > settings.hot_above.unwrap_or(HOT_DEFAULT) {
        TempBand::Warm
    } else {
        TempBand::Comfortable
    }
}

/// The detail fragments in display order (rain, frost, sunset), at most two.
pub fn details(
    weather: Option<&WeatherView>,
    sunset: Option<NaiveTime>,
    fallback: bool,
    now: NaiveTime,
    card_visible: bool,
) -> Vec<Detail> {
    let mut out = Vec::new();
    if weather.is_some_and(|w| w.rain_next_hour.is_some_and(|mm| mm > RAIN_MIN_MM)) {
        out.push(Detail::RainSoon);
    }
    if now.hour() >= FROST_FROM_HOUR
        && weather.is_some_and(|w| w.tonight_min.is_some_and(|c| c <= 0.0))
    {
        out.push(Detail::FrostTonight);
    }
    let near = sunset.is_none_or(|at| {
        let from = i64::from(at.num_seconds_from_midnight()) - SUNSET_LEAD.num_seconds();
        (from..=i64::from(at.num_seconds_from_midnight()))
            .contains(&i64::from(now.num_seconds_from_midnight()))
    });
    if near || !card_visible {
        match sunset {
            _ if fallback => out.push(Detail::SunsetFallback),
            Some(at) => out.push(Detail::Sunset(at)),
            None => {}
        }
    }
    out.truncate(MAX_DETAILS);
    out
}

/// Sunrise and sunset in house local time, whether it is dark now, and whether the house has no
/// coordinates (the Paris fallback is in use). While the flows config is not fetched yet (`cfg` is
/// `None`) the sun times and the fallback hint are unknown, so none is reported.
pub fn sun_today(
    cfg: Option<&FlowsConfigView>,
    now: DateTime<Utc>,
) -> (Option<NaiveTime>, Option<NaiveTime>, bool, bool) {
    let ((lat, lon), zone) = house_place(cfg);
    let local_date = now.with_timezone(&zone).date_naive();
    let events = sun_events(lat, lon, local_date);
    let local_time = |at: Option<DateTime<Utc>>| at.map(|at| at.with_timezone(&zone).time());
    let known = cfg.is_some();
    (
        local_time(events.sunrise).filter(|_| known),
        local_time(events.sunset).filter(|_| known),
        is_dark_at(lat, lon, now),
        cfg.is_some_and(|c| c.fallback_active),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn quiet() -> HouseSummary {
        HouseSummary {
            indoor: None,
            lights_on: 0,
            openings: vec![],
            unseen_alerts: 0,
            heating: None,
            mode: None,
        }
    }

    fn opening(title: &str) -> Opening {
        Opening {
            title: title.into(),
            room: None,
        }
    }

    fn weather(rain: Option<f64>, min: Option<f64>) -> WeatherView {
        let mut v: WeatherView = serde_json::from_value(serde_json::json!({
            "place": {"name": "X"},
            "fetched_at": 0, "stale": false,
            "current": {}, "today": {},
            "rain_next_hour": null, "tonight_min": null, "days": []
        }))
        .expect("weather fixture");
        v.rain_next_hour = rain;
        v.tonight_min = min;
        v
    }

    #[test]
    fn greeting_bands() {
        assert_eq!(greeting(4), Greeting::Evening);
        assert_eq!(greeting(5), Greeting::Morning);
        assert_eq!(greeting(11), Greeting::Morning);
        assert_eq!(greeting(12), Greeting::Afternoon);
        assert_eq!(greeting(17), Greeting::Afternoon);
        assert_eq!(greeting(18), Greeting::Evening);
    }

    #[test]
    fn activity_order() {
        let mut s = quiet();
        s.unseen_alerts = 1;
        s.openings = vec![opening("Window")];
        assert_eq!(activity(&s, false, 12), Activity::Attention);
        s.unseen_alerts = 0;
        assert_eq!(activity(&s, false, 12), Activity::Open(opening("Window")));
        s.openings.push(opening("Door"));
        assert_eq!(activity(&s, false, 12), Activity::OpenMany(2));
        let s = quiet();
        assert_eq!(activity(&s, true, 23), Activity::Asleep);
        assert_eq!(activity(&s, true, 21), Activity::Quiet);
        assert_eq!(activity(&s, false, 23), Activity::Quiet);
        let mut s = quiet();
        s.lights_on = 1;
        assert_eq!(activity(&s, true, 23), Activity::Awake);
    }

    #[test]
    fn temp_band_edges() {
        let d = HomeSettings::default();
        assert_eq!(temp_band(14.9, &d), TempBand::Cold);
        assert_eq!(temp_band(15.0, &d), TempBand::Comfortable);
        assert_eq!(temp_band(26.0, &d), TempBand::Comfortable);
        assert_eq!(temp_band(26.1, &d), TempBand::Warm);
        let s = HomeSettings {
            cold_below: Some(18.0),
            hot_above: Some(24.0),
            ..HomeSettings::default()
        };
        assert_eq!(temp_band(17.9, &s), TempBand::Cold);
        assert_eq!(temp_band(18.0, &s), TempBand::Comfortable);
        assert_eq!(temp_band(24.1, &s), TempBand::Warm);
    }

    #[test]
    fn details_order_and_cap() {
        let w = weather(Some(0.5), Some(-1.0));
        assert_eq!(
            details(Some(&w), Some(t(17, 30)), false, t(16, 0), true),
            vec![Detail::RainSoon, Detail::FrostTonight]
        );
    }

    #[test]
    fn rain_needs_more_than_a_tenth() {
        let w = weather(Some(0.1), None);
        assert!(details(Some(&w), None, false, t(10, 0), true).is_empty());
    }

    #[test]
    fn frost_only_from_15h() {
        let w = weather(None, Some(0.0));
        assert!(details(Some(&w), None, false, t(14, 59), true).is_empty());
        assert_eq!(
            details(Some(&w), None, false, t(15, 0), true),
            vec![Detail::FrostTonight]
        );
        let warm = weather(None, Some(0.1));
        assert!(details(Some(&warm), None, false, t(20, 0), true).is_empty());
    }

    #[test]
    fn sunset_two_hours_before() {
        let sunset = Some(t(18, 42));
        assert!(details(None, sunset, false, t(16, 41), true).is_empty());
        assert_eq!(
            details(None, sunset, false, t(16, 42), true),
            vec![Detail::Sunset(t(18, 42))]
        );
        assert_eq!(
            details(None, sunset, false, t(18, 42), true),
            vec![Detail::Sunset(t(18, 42))]
        );
        assert!(details(None, sunset, false, t(18, 43), true).is_empty());
    }

    #[test]
    fn sunset_always_without_card() {
        let d = details(None, Some(t(18, 42)), false, t(10, 0), false);
        assert!(d.contains(&Detail::Sunset(t(18, 42))));
    }

    #[test]
    fn fallback_sunset_hint() {
        let d = details(None, Some(t(18, 42)), true, t(17, 0), true);
        assert_eq!(d, vec![Detail::SunsetFallback]);
        let d = details(None, Some(t(18, 42)), true, t(10, 0), false);
        assert_eq!(d, vec![Detail::SunsetFallback]);
    }

    #[test]
    fn sun_today_in_a_configured_house() {
        use chrono::TimeZone;
        let cfg = FlowsConfigView {
            latitude: Some(48.85),
            longitude: Some(2.35),
            timezone: "Europe/Paris".into(),
            fallback_active: false,
        };
        let noon = Utc.with_ymd_and_hms(2026, 6, 21, 10, 0, 0).unwrap();
        let (rise, set, dark, fallback) = sun_today(Some(&cfg), noon);
        assert!(!dark && !fallback);
        assert!(rise.unwrap() < t(7, 0) && rise.unwrap() > t(5, 0));
        assert!(set.unwrap() > t(21, 0) && set.unwrap() < t(22, 30));
        let midnight = Utc.with_ymd_and_hms(2026, 6, 21, 23, 30, 0).unwrap();
        assert!(sun_today(Some(&cfg), midnight).2);
        assert!(sun_today(Some(&cfg), noon).1.is_some());
    }

    #[test]
    fn sun_today_is_unknown_before_the_config_arrives() {
        use chrono::TimeZone;
        let noon = Utc.with_ymd_and_hms(2026, 6, 21, 10, 0, 0).unwrap();
        let (rise, set, _, fallback) = sun_today(None, noon);
        assert!(rise.is_none() && set.is_none() && !fallback);
        let unset = FlowsConfigView {
            latitude: None,
            longitude: None,
            timezone: "Europe/Paris".into(),
            fallback_active: true,
        };
        assert!(sun_today(Some(&unset), noon).3);
    }
}
