//! FR/EN text: Weather card and page.

use chrono::{Datelike, NaiveDate, NaiveTime};

use nidoo_api::WeatherCondition;

use crate::home_header::weather_fmt::UvLevel;

use super::*;

/// "feels like 5°" after the condition, on the wide card.
pub fn feels_like(lang: Lang, deg: &str) -> String {
    match lang {
        Lang::Fr => format!("ressenti {deg}"),
        Lang::En => format!("feels like {deg}"),
    }
}

/// Today's low and high on the wide card: "Today 9° / 14°".
pub fn today_range(lang: Lang, range: &str) -> String {
    match lang {
        Lang::Fr => format!("Aujourd'hui {range}"),
        Lang::En => format!("Today {range}"),
    }
}

/// The sunset line of the card; `short` is "Sunset 18:42" / "Coucher 18 h 42".
pub fn card_sunset(lang: Lang, at: NaiveTime, short: bool) -> String {
    let time = home_time(lang, at);
    match (lang, short) {
        (Lang::Fr, false) => format!("Coucher du soleil {time}"),
        (Lang::Fr, true) => format!("Coucher {time}"),
        (Lang::En, _) => format!("Sunset {time}"),
    }
}

/// The muted line of a card whose data is late.
pub fn data_from(lang: Lang, at: NaiveTime) -> String {
    match lang {
        Lang::Fr => format!("Données de {}", home_time(lang, at)),
        Lang::En => format!("Data from {}", home_time(lang, at)),
    }
}

/// The label of a weather condition.
pub fn condition_label(lang: Lang, c: WeatherCondition) -> &'static str {
    use WeatherCondition::*;
    match (lang, c) {
        (Lang::Fr, Clear) => "Dégagé",
        (Lang::Fr, PartlyCloudy) => "Éclaircies",
        (Lang::Fr, Cloudy) => "Nuageux",
        (Lang::Fr, Fog) => "Brouillard",
        (Lang::Fr, Drizzle) => "Bruine",
        (Lang::Fr, Rain) => "Pluie",
        (Lang::Fr, FreezingRain) => "Pluie verglaçante",
        (Lang::Fr, Snow) => "Neige",
        (Lang::Fr, Showers) => "Averses",
        (Lang::Fr, Thunderstorm) => "Orage",
        (Lang::En, Clear) => "Clear",
        (Lang::En, PartlyCloudy) => "Partly cloudy",
        (Lang::En, Cloudy) => "Cloudy",
        (Lang::En, Fog) => "Fog",
        (Lang::En, Drizzle) => "Drizzle",
        (Lang::En, Rain) => "Rain",
        (Lang::En, FreezingRain) => "Freezing rain",
        (Lang::En, Snow) => "Snow",
        (Lang::En, Showers) => "Showers",
        (Lang::En, Thunderstorm) => "Thunderstorm",
    }
}

/// The compass point for an index from `compass_index` (0 = N, clockwise).
pub fn compass_word(lang: Lang, index: usize) -> &'static str {
    const EN: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    const FR: [&str; 8] = ["N", "NE", "E", "SE", "S", "SO", "O", "NO"];
    let table = if lang == Lang::Fr { FR } else { EN };
    table[index % 8]
}

/// The word for a UV band.
pub fn uv_word(lang: Lang, level: UvLevel) -> &'static str {
    match (lang, level) {
        (Lang::Fr, UvLevel::Low) => "Faible",
        (Lang::Fr, UvLevel::Moderate) => "Modéré",
        (Lang::Fr, UvLevel::High) => "Élevé",
        (Lang::Fr, UvLevel::VeryHigh) => "Très élevé",
        (Lang::Fr, UvLevel::Extreme) => "Extrême",
        (Lang::En, UvLevel::Low) => "Low",
        (Lang::En, UvLevel::Moderate) => "Moderate",
        (Lang::En, UvLevel::High) => "High",
        (Lang::En, UvLevel::VeryHigh) => "Very high",
        (Lang::En, UvLevel::Extreme) => "Extreme",
    }
}

/// The fixed words of the weather page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherText {
    Now,
    NextDays,
    NoRainSoon,
    UpdatedAt,
    Unavailable,
    EnableHint,
    Credit,
    TonightMin,
    Wind,
    Humidity,
    Pressure,
    Clouds,
    Rain,
    MaxGusts,
    UvIndex,
    Sun,
    Today,
    Tomorrow,
}

/// A fixed word of the weather page.
pub fn weather_text(lang: Lang, k: WeatherText) -> &'static str {
    use WeatherText::*;
    let (en, fr) = match k {
        Now => ("Now", "Maintenant"),
        NextDays => ("Forecast", "Prévisions"),
        NoRainSoon => ("No rain within the hour", "Pas de pluie dans l'heure"),
        UpdatedAt => ("Updated at", "Mis à jour à"),
        Unavailable => (
            "The weather is unavailable.",
            "La météo n'est pas disponible.",
        ),
        EnableHint => (
            "Enable [weather] in oikos.toml to see it here.",
            "Activez [weather] dans oikos.toml pour la voir ici.",
        ),
        Credit => (
            "Data Open-Meteo (CC BY 4.0)",
            "Données Open-Meteo (CC BY 4.0)",
        ),
        TonightMin => ("Tonight, minimum", "Cette nuit, minimum"),
        Wind => ("Wind", "Vent"),
        Humidity => ("Humidity", "Humidité"),
        Pressure => ("Pressure", "Pression"),
        Clouds => ("Clouds", "Nuages"),
        Rain => ("Rain", "Pluie"),
        MaxGusts => ("Max gusts", "Rafales max"),
        UvIndex => ("UV index", "Indice UV"),
        Sun => ("Sun", "Soleil"),
        Today => ("Today", "Aujourd'hui"),
        Tomorrow => ("Tomorrow", "Demain"),
    };
    if lang == Lang::Fr { fr } else { en }
}

const NBSP: char = '\u{A0}';

/// Rain in the next hour: "Rain within the hour: 1.2 mm".
pub fn rain_next_hour(lang: Lang, mm: &str) -> String {
    match lang {
        Lang::Fr => format!("Pluie dans l'heure{NBSP}: {mm}{NBSP}mm"),
        Lang::En => format!("Rain within the hour: {mm}{NBSP}mm"),
    }
}

/// The sub-line of the rain tile: "today · 93 % chance".
pub fn rain_sub(lang: Lang, chance: Option<&str>) -> String {
    match (lang, chance) {
        (Lang::Fr, Some(c)) => format!("aujourd'hui · risque {c}{NBSP}%"),
        (Lang::En, Some(c)) => format!("today · {c}{NBSP}% chance"),
        (Lang::Fr, None) => "aujourd'hui".to_owned(),
        (Lang::En, None) => "today".to_owned(),
    }
}

/// The sub-line of a forecast day's rain tile: "93 % chance" / "risque 93 %".
pub fn chance_sub(lang: Lang, chance: Option<&str>) -> String {
    match (lang, chance) {
        (Lang::Fr, Some(c)) => format!("risque {c}{NBSP}%"),
        (Lang::En, Some(c)) => format!("{c}{NBSP}% chance"),
        (_, None) => String::new(),
    }
}

/// The sub-line of the wind tile: "W · gusts 48".
pub fn wind_sub(lang: Lang, compass: Option<&str>, gusts: Option<&str>) -> String {
    let gust = gusts.map(|g| match lang {
        Lang::Fr => format!("rafales {g}"),
        Lang::En => format!("gusts {g}"),
    });
    [compass.map(str::to_owned), gust]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The sub-line of the sun tile: "sunrise 08:21".
pub fn sunrise_sub(lang: Lang, at: NaiveTime) -> String {
    match lang {
        Lang::Fr => format!("lever {}", home_time(lang, at)),
        Lang::En => format!("sunrise {}", home_time(lang, at)),
    }
}

/// A forecast day's heading: "Sam. 24" / "Sat 24".
pub fn short_weekday(lang: Lang, date: NaiveDate) -> String {
    format!(
        "{} {}",
        format::weekday_short(lang, date.weekday()),
        date.day()
    )
}

/// The name of a forecast day on the weather page: "Tomorrow" the day after `today`, else the
/// long weekday and the day of the month ("Thursday 29" / "Jeudi 29").
pub fn day_heading(lang: Lang, date: NaiveDate, today: NaiveDate) -> String {
    if today.succ_opt() == Some(date) {
        return weather_text(lang, WeatherText::Tomorrow).to_owned();
    }
    format!(
        "{} {}",
        format::weekday_long(lang, date.weekday()),
        date.day()
    )
}

/// "Updated at 21:02".
pub fn updated_at(lang: Lang, at: NaiveTime) -> String {
    format!(
        "{} {}",
        weather_text(lang, WeatherText::UpdatedAt),
        home_time(lang, at)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_labels_match_the_notify_words() {
        for c in WeatherCondition::ALL {
            for lang in [Lang::En, Lang::Fr] {
                assert_eq!(
                    condition_label(lang, c),
                    nidoo_core::flow::template::text_word(c.as_str(), lang.core())
                );
            }
        }
    }
    #[test]
    fn weather_page_copy() {
        let sat = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        assert_eq!(short_weekday(Lang::Fr, sat), "Sam. 24");
        assert_eq!(short_weekday(Lang::En, sat), "Sat 24");
        assert_eq!(wind_sub(Lang::En, Some("W"), Some("48")), "W · gusts 48");
        assert_eq!(wind_sub(Lang::Fr, None, None), "");
        for lang in [Lang::En, Lang::Fr] {
            assert!(!weather_text(lang, WeatherText::Credit).is_empty());
        }
    }
    #[test]
    fn a_forecast_day_is_named_by_weekday_or_tomorrow() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let thu = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        assert_eq!(day_heading(Lang::En, tomorrow, today), "Tomorrow");
        assert_eq!(day_heading(Lang::Fr, tomorrow, today), "Demain");
        assert_eq!(day_heading(Lang::En, thu, today), "Thursday 29");
        assert_eq!(day_heading(Lang::Fr, thu, today), "Jeudi 29");
        assert_eq!(chance_sub(Lang::En, Some("93")), "93\u{A0}% chance");
        assert_eq!(chance_sub(Lang::Fr, None), "");
    }
    #[test]
    fn units_and_colons_take_a_no_break_space() {
        assert_eq!(
            rain_next_hour(Lang::Fr, "1,2"),
            "Pluie dans l'heure\u{A0}: 1,2\u{A0}mm"
        );
        assert_eq!(
            rain_next_hour(Lang::En, "1.2"),
            "Rain within the hour: 1.2\u{A0}mm"
        );
        assert_eq!(
            rain_sub(Lang::Fr, Some("93")),
            "aujourd'hui · risque 93\u{A0}%"
        );
        assert_eq!(rain_sub(Lang::En, Some("93")), "today · 93\u{A0}% chance");
    }
    #[test]
    fn every_weather_text_has_two_languages() {
        const ALL: &[WeatherText] = &[
            WeatherText::Now,
            WeatherText::NextDays,
            WeatherText::NoRainSoon,
            WeatherText::UpdatedAt,
            WeatherText::Unavailable,
            WeatherText::EnableHint,
            WeatherText::Credit,
            WeatherText::TonightMin,
            WeatherText::Wind,
            WeatherText::Humidity,
            WeatherText::Pressure,
            WeatherText::Clouds,
            WeatherText::Rain,
            WeatherText::MaxGusts,
            WeatherText::UvIndex,
            WeatherText::Sun,
            WeatherText::Today,
            WeatherText::Tomorrow,
        ];
        crate::i18n::assert_table(ALL, weather_text, &[]);
    }
}
