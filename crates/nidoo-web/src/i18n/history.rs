//! FR/EN text: the history chart and the history page.

use super::*;
use nidoo_api::HistoryRange;

const NBSP: char = '\u{A0}';

/// Fixed words of the history chart and page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryText {
    /// The last x-axis label.
    Now,
    NotRecorded,
    Unavailable,
    CouldNotLoad,
    Retry,
    UnknownEntity,
    LastHour,
    Last6h,
    /// Chart legend: "Last 24 hours".
    Last24h,
    Last7d,
    Last30d,
    LastYear,
    /// Legend words of `min_max`; the same in both languages.
    Min,
    Max,
    /// Chart key of the line on the bucketed ranges.
    Average,
    /// Chart key of the shaded band around the line on the bucketed ranges.
    Spread,
    /// Hover text of the chart key: what the line and the band are.
    SpreadHint,
}

pub fn history_text(lang: Lang, k: HistoryText) -> &'static str {
    use HistoryText as T;
    let (en, fr) = match k {
        T::Now => ("now", "maintenant"),
        T::NotRecorded => (
            "No data recorded for this entity yet. The hub starts recording from the moment the entity first appears.",
            "Aucune donnée enregistrée pour cette entité pour l'instant. Le hub commence à enregistrer dès l'apparition de l'entité.",
        ),
        T::Unavailable => (
            "History is unavailable right now. The database may be down.",
            "L'historique est indisponible pour le moment. La base de données est peut-être arrêtée.",
        ),
        T::CouldNotLoad => (
            "Could not load history",
            "Impossible de charger l'historique",
        ),
        T::Retry => ("Retry", "Réessayer"),
        T::UnknownEntity => ("Unknown entity", "Entité inconnue"),
        T::LastHour => ("Last hour", "Dernière heure"),
        T::Last6h => ("Last 6 hours", "6 dernières heures"),
        T::Last24h => ("Last 24 hours", "24 dernières heures"),
        T::Last7d => ("Last 7 days", "7 derniers jours"),
        T::Last30d => ("Last 30 days", "30 derniers jours"),
        T::LastYear => ("Last year", "12 derniers mois"),
        T::Min => ("min", "min"),
        T::Max => ("max", "max"),
        T::Average => ("Average", "Moyenne"),
        T::Spread => ("Min–max range", "Écart min–max"),
        T::SpreadHint => (
            "Each point sums up a slice of time: the line is its average, the shaded band spans its lowest to highest reading.",
            "Chaque point résume une tranche de temps : la ligne en est la moyenne, la bande ombrée va de sa valeur la plus basse à la plus haute.",
        ),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The legend name of a chart range: "Last 24 hours" / "24 dernières heures".
pub fn range_name(lang: Lang, r: HistoryRange) -> &'static str {
    history_text(
        lang,
        match r {
            HistoryRange::Hour => HistoryText::LastHour,
            HistoryRange::SixHours => HistoryText::Last6h,
            HistoryRange::Day => HistoryText::Last24h,
            HistoryRange::Week => HistoryText::Last7d,
            HistoryRange::Month => HistoryText::Last30d,
            HistoryRange::Year => HistoryText::LastYear,
        },
    )
}

/// The range chip of the history page: "1h" / "1 h", "6h" / "6 h", "24h" / "24 h", "7d" / "7 j", "30d" / "30 j", "1y" / "1 an".
pub fn range_chip(lang: Lang, r: HistoryRange) -> String {
    match (lang, r) {
        (Lang::En, HistoryRange::Hour) => "1h".into(),
        (Lang::En, HistoryRange::SixHours) => "6h".into(),
        (Lang::En, HistoryRange::Day) => "24h".into(),
        (Lang::En, HistoryRange::Week) => "7d".into(),
        (Lang::En, HistoryRange::Month) => "30d".into(),
        (Lang::En, HistoryRange::Year) => "1y".into(),
        (Lang::Fr, HistoryRange::Hour) => format!("1{NBSP}h"),
        (Lang::Fr, HistoryRange::SixHours) => format!("6{NBSP}h"),
        (Lang::Fr, HistoryRange::Day) => format!("24{NBSP}h"),
        (Lang::Fr, HistoryRange::Week) => format!("7{NBSP}j"),
        (Lang::Fr, HistoryRange::Month) => format!("30{NBSP}j"),
        (Lang::Fr, HistoryRange::Year) => format!("1{NBSP}an"),
    }
}

/// "Could not load history: boom" / "Impossible de charger l'historique : boom"; the reason is
/// the hub's own text.
pub fn load_error(lang: Lang, reason: &str) -> String {
    let lead = history_text(lang, HistoryText::CouldNotLoad);
    match lang {
        Lang::En => format!("{lead}: {reason}"),
        Lang::Fr => format!("{lead}{NBSP}: {reason}"),
    }
}

/// The legend's right side: "min 3.0 °C · max 21.5 °C" (French decimals and spacing).
pub fn min_max(lang: Lang, min: f64, max: f64, unit: Option<&str>) -> String {
    let cell = |v: f64| format::with_unit(format::decimal_grouped(lang, v, 1), unit);
    format!(
        "{} {} · {} {}",
        history_text(lang, HistoryText::Min),
        cell(min),
        history_text(lang, HistoryText::Max),
        cell(max)
    )
}

/// An energy amount with its unit: "4.2 kWh" / "4,2 kWh".
pub fn kwh(lang: Lang, v: f64) -> String {
    format::with_unit(format::decimal_grouped(lang, v, 1), Some("kWh"))
}

/// "Last 24 hours total: 4.2 kWh" / "24 dernières heures, total : 4,2 kWh".
pub fn total_line(lang: Lang, r: HistoryRange, total: f64) -> String {
    let value = kwh(lang, total);
    match lang {
        Lang::En => format!("{} total: {value}", range_name(lang, r)),
        Lang::Fr => format!("{}, total{NBSP}: {value}", range_name(lang, r)),
    }
}

/// "Last 24 hours · daily total" / "24 dernières heures · total quotidien".
pub fn counter_legend(lang: Lang, r: HistoryRange) -> String {
    let tail = match lang {
        Lang::En => "daily total",
        Lang::Fr => "total quotidien",
    };
    format!("{} · {tail}", range_name(lang, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_history_text_has_two_languages() {
        const ALL: &[HistoryText] = &[
            HistoryText::Now,
            HistoryText::NotRecorded,
            HistoryText::Unavailable,
            HistoryText::CouldNotLoad,
            HistoryText::Retry,
            HistoryText::UnknownEntity,
            HistoryText::LastHour,
            HistoryText::Last6h,
            HistoryText::Last24h,
            HistoryText::Last7d,
            HistoryText::Last30d,
            HistoryText::LastYear,
            HistoryText::Min,
            HistoryText::Max,
            HistoryText::Average,
            HistoryText::Spread,
            HistoryText::SpreadHint,
        ];
        crate::i18n::assert_table(ALL, history_text, &[HistoryText::Min, HistoryText::Max]);
    }

    #[test]
    fn legends_and_errors_follow_the_language() {
        assert_eq!(
            min_max(Lang::Fr, 3.0, 21.5, Some("°C")),
            "min 3,0\u{A0}°C · max 21,5\u{A0}°C"
        );
        assert_eq!(
            min_max(Lang::En, 3.0, 21.5, Some("°C")),
            "min 3.0\u{A0}°C · max 21.5\u{A0}°C"
        );
        assert_eq!(
            min_max(Lang::Fr, 0.0, 3450.0, Some("W")),
            "min 0,0\u{A0}W · max 3\u{202F}450,0\u{A0}W"
        );
        assert_eq!(range_name(Lang::Fr, HistoryRange::Year), "12 derniers mois");
        assert_eq!(
            total_line(Lang::Fr, HistoryRange::Day, 4321.7),
            "24 dernières heures, total\u{A0}: 4\u{202F}321,7\u{A0}kWh"
        );
        assert_eq!(
            total_line(Lang::En, HistoryRange::Day, 4.25),
            "Last 24 hours total: 4.2\u{A0}kWh"
        );
        assert_eq!(
            total_line(Lang::Fr, HistoryRange::Week, 4.5),
            "7 derniers jours, total\u{A0}: 4,5\u{A0}kWh"
        );
        assert_eq!(
            load_error(Lang::Fr, "boom"),
            "Impossible de charger l'historique\u{A0}: boom"
        );
        assert_eq!(load_error(Lang::En, "boom"), "Could not load history: boom");
        assert_eq!(range_chip(Lang::En, HistoryRange::Hour), "1h");
        assert_eq!(range_chip(Lang::Fr, HistoryRange::SixHours), "6\u{A0}h");
        assert_eq!(range_name(Lang::Fr, HistoryRange::Hour), "Dernière heure");
        assert_eq!(range_name(Lang::En, HistoryRange::SixHours), "Last 6 hours");
        assert_eq!(range_chip(Lang::En, HistoryRange::Week), "7d");
        assert_eq!(range_chip(Lang::Fr, HistoryRange::Week), "7\u{A0}j");
        assert_eq!(
            counter_legend(Lang::Fr, HistoryRange::Month),
            "30 derniers jours · total quotidien"
        );
    }
}
