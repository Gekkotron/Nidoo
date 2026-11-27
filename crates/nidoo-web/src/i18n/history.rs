//! FR/EN text: the history chart and the history page.

use super::*;
use crate::history_period::{Span, Unit};
use chrono::{Datelike, NaiveDate};
use nidoo_api::{DailyPer, HistoryRange};

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
    /// Label of a headline value that is not a number: "State" / "État".
    State,
    /// Label of the headline number chip when the entity has no device class.
    Value,
    /// `aria-label` of the row of value chips.
    ValuesShown,
    /// Chart key of the line on the bucketed ranges.
    Average,
    /// Chart key of the shaded band around the line on the bucketed ranges.
    Spread,
    /// Hover text of the chart key: what the line and the band are.
    SpreadHint,
    // The period picker of the history page.
    UnitDay,
    UnitWeek,
    UnitMonth,
    UnitYear,
    UnitCustom,
    BackToNow,
    CompareWith,
    CompareOff,
    Today,
    ThisWeek,
    ThisMonth,
    ThisYear,
    PickTitle,
    PickFrom,
    PickTo,
    PickShow,
    PickCancel,
    ThisWinter,
    Last12Months,
    AllData,
    Difference,
    Mean,
    Total,
    SoFar,
    SameDays,
    PreviousPeriod,
    NextPeriod,
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
        T::State => ("State", "État"),
        T::Value => ("Value", "Valeur"),
        T::ValuesShown => ("Values shown", "Valeurs affichées"),
        T::Average => ("Average", "Moyenne"),
        T::Spread => ("Min–max range", "Écart min–max"),
        T::SpreadHint => (
            "Each point sums up a slice of time: the line is its average, the shaded band spans its lowest to highest reading.",
            "Chaque point résume une tranche de temps : la ligne en est la moyenne, la bande ombrée va de sa valeur la plus basse à la plus haute.",
        ),
        T::UnitDay => ("Day", "Jour"),
        T::UnitWeek => ("Week", "Semaine"),
        T::UnitMonth => ("Month", "Mois"),
        T::UnitYear => ("Year", "Année"),
        T::UnitCustom => ("Custom…", "Personnalisée…"),
        T::BackToNow => ("Back to now", "Revenir à maintenant"),
        T::CompareWith => ("Compare with", "Comparer avec"),
        T::CompareOff => ("Off", "Non"),
        T::Today => ("Today", "Aujourd'hui"),
        T::ThisWeek => ("This week", "Cette semaine"),
        T::ThisMonth => ("This month", "Ce mois-ci"),
        T::ThisYear => ("This year", "Cette année"),
        T::PickTitle => ("Show a period", "Afficher une période"),
        T::PickFrom => ("From", "Du"),
        T::PickTo => ("To", "Au"),
        T::PickShow => ("Show", "Afficher"),
        T::PickCancel => ("Cancel", "Annuler"),
        T::ThisWinter => ("This winter", "Cet hiver"),
        T::Last12Months => ("Last 12 months", "12 derniers mois"),
        T::AllData => ("All data", "Tout l'historique"),
        T::Difference => ("Difference", "Écart"),
        T::Mean => ("mean", "moyenne"),
        T::Total => ("total", "total"),
        T::SoFar => ("so far", "jusqu'ici"),
        T::SameDays => ("same days", "mêmes jours"),
        T::PreviousPeriod => ("Previous period", "Période précédente"),
        T::NextPeriod => ("Next period", "Période suivante"),
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
    total_text(lang, range_name(lang, r), total)
}

/// "{name} total: 4.2 kWh" / "{name}, total : 4,2 kWh".
fn total_text(lang: Lang, name: &str, total: f64) -> String {
    let value = kwh(lang, total);
    match lang {
        Lang::En => format!("{name} total: {value}"),
        Lang::Fr => format!("{name}, total{NBSP}: {value}"),
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

/// The legend of a counter over a named period: "November 2026 · daily total" /
/// "novembre 2026 · total quotidien"; hourly for a day, monthly for a year.
pub fn period_counter_legend(lang: Lang, name: &str, per: DailyPer) -> String {
    let tail = match (lang, per) {
        (Lang::En, DailyPer::Hour) => "hourly total",
        (Lang::En, DailyPer::Day) => "daily total",
        (Lang::En, DailyPer::Month) => "monthly total",
        (Lang::Fr, DailyPer::Hour) => "total horaire",
        (Lang::Fr, DailyPer::Day) => "total quotidien",
        (Lang::Fr, DailyPer::Month) => "total mensuel",
    };
    format!("{name} · {tail}")
}

/// "November 2026 total: 4.2 kWh" / "novembre 2026, total : 4,2 kWh".
pub fn period_total_line(lang: Lang, name: &str, total: f64) -> String {
    total_text(lang, name, total)
}

/// The picker's unit chip: the 1 h and 6 h chips keep the range chip text.
pub fn unit_chip(lang: Lang, u: Unit) -> String {
    use HistoryText as T;
    let word = |k| history_text(lang, k).to_string();
    match u {
        Unit::Hour1 => range_chip(lang, HistoryRange::Hour),
        Unit::Hour6 => range_chip(lang, HistoryRange::SixHours),
        Unit::Day => word(T::UnitDay),
        Unit::Week => word(T::UnitWeek),
        Unit::Month => word(T::UnitMonth),
        Unit::Year => word(T::UnitYear),
        Unit::Custom => word(T::UnitCustom),
    }
}

/// "26 Nov" / "26 nov." (day first in both languages).
fn day_month(lang: Lang, d: NaiveDate) -> String {
    format!("{} {}", d.day(), format::month_short(lang, d.month()))
}

/// "26 Nov 2026" / "26 nov. 2026".
fn day_month_year(lang: Lang, d: NaiveDate) -> String {
    format!("{} {}", day_month(lang, d), d.year())
}

/// The name of a shown period: "Thu 26 Nov 2026", "23 – 29 Nov 2026", "November 2026", "2026",
/// "1 Nov 2025 – 31 Mar 2026". The 1 h and 6 h units are named like a day.
pub fn period_name(lang: Lang, u: Unit, s: Span) -> String {
    let (from, to) = (s.from, s.to);
    match u {
        Unit::Hour1 | Unit::Hour6 | Unit::Day => {
            let wd = format::weekday_short(lang, from.weekday());
            let wd = match lang {
                Lang::En => wd.to_string(),
                Lang::Fr => wd.to_lowercase(),
            };
            format!("{wd} {}", day_month_year(lang, from))
        }
        Unit::Week => {
            if from.year() != to.year() {
                format!(
                    "{} \u{2013} {}",
                    day_month_year(lang, from),
                    day_month_year(lang, to)
                )
            } else if from.month() != to.month() {
                format!(
                    "{} \u{2013} {}",
                    day_month(lang, from),
                    day_month_year(lang, to)
                )
            } else {
                format!("{} \u{2013} {}", from.day(), day_month_year(lang, to))
            }
        }
        Unit::Month => format!("{} {}", format::month_long(lang, from.month()), from.year()),
        Unit::Year => from.year().to_string(),
        Unit::Custom => format!(
            "{} \u{2013} {}",
            day_month_year(lang, from),
            day_month_year(lang, to)
        ),
    }
}

/// "No data for November 2025" / "Aucune donnée pour novembre 2025".
pub fn no_data_for(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("No data for {name}"),
        Lang::Fr => format!("Aucune donnée pour {name}"),
    }
}

/// "1 day left out of the comparison" / "3 jours exclus de la comparaison".
pub fn left_out(lang: Lang, n: usize, per: DailyPer) -> String {
    let one = n == 1;
    match lang {
        Lang::En => {
            let unit = match (per, one) {
                (DailyPer::Hour, true) => "hour",
                (DailyPer::Hour, false) => "hours",
                (DailyPer::Day, true) => "day",
                (DailyPer::Day, false) => "days",
                (DailyPer::Month, true) => "month",
                (DailyPer::Month, false) => "months",
            };
            format!("{n} {unit} left out of the comparison")
        }
        Lang::Fr => {
            let (unit, left) = match (per, one) {
                (DailyPer::Hour, true) => ("heure", "exclue"),
                (DailyPer::Hour, false) => ("heures", "exclues"),
                (DailyPer::Day, true) => ("jour", "exclu"),
                (DailyPer::Day, false) => ("jours", "exclus"),
                (DailyPer::Month, true) => ("mois", "exclu"),
                (DailyPer::Month, false) => ("mois", "exclus"),
            };
            format!("{n} {unit} {left} de la comparaison")
        }
    }
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
            HistoryText::UnitDay,
            HistoryText::UnitWeek,
            HistoryText::UnitMonth,
            HistoryText::UnitYear,
            HistoryText::UnitCustom,
            HistoryText::BackToNow,
            HistoryText::CompareWith,
            HistoryText::CompareOff,
            HistoryText::Today,
            HistoryText::ThisWeek,
            HistoryText::ThisMonth,
            HistoryText::ThisYear,
            HistoryText::PickTitle,
            HistoryText::PickFrom,
            HistoryText::PickTo,
            HistoryText::PickShow,
            HistoryText::PickCancel,
            HistoryText::ThisWinter,
            HistoryText::Last12Months,
            HistoryText::AllData,
            HistoryText::Difference,
            HistoryText::Mean,
            HistoryText::Total,
            HistoryText::SoFar,
            HistoryText::SameDays,
            HistoryText::PreviousPeriod,
            HistoryText::NextPeriod,
        ];
        crate::i18n::assert_table(
            ALL,
            history_text,
            &[HistoryText::Min, HistoryText::Max, HistoryText::Total],
        );
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

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn span(a: NaiveDate, b: NaiveDate) -> Span {
        Span { from: a, to: b }
    }

    #[test]
    fn period_names_en_fr() {
        let day = span(d(2026, 11, 26), d(2026, 11, 26));
        let week = span(d(2026, 11, 23), d(2026, 11, 29));
        let month = span(d(2026, 11, 1), d(2026, 11, 30));
        let year = span(d(2026, 1, 1), d(2026, 12, 31));
        let custom = span(d(2025, 11, 1), d(2026, 3, 31));
        let cross_month = span(d(2026, 11, 30), d(2026, 12, 6));
        let cross_year = span(d(2026, 12, 28), d(2027, 1, 3));
        assert_eq!(period_name(Lang::En, Unit::Day, day), "Thu 26 Nov 2026");
        assert_eq!(period_name(Lang::Fr, Unit::Day, day), "jeu. 26 nov. 2026");
        assert_eq!(
            period_name(Lang::En, Unit::Week, week),
            "23 \u{2013} 29 Nov 2026"
        );
        assert_eq!(
            period_name(Lang::Fr, Unit::Week, week),
            "23 \u{2013} 29 nov. 2026"
        );
        assert_eq!(
            period_name(Lang::En, Unit::Week, cross_month),
            "30 Nov \u{2013} 6 Dec 2026"
        );
        assert_eq!(
            period_name(Lang::Fr, Unit::Week, cross_month),
            "30 nov. \u{2013} 6 d\u{e9}c. 2026"
        );
        assert_eq!(
            period_name(Lang::En, Unit::Week, cross_year),
            "28 Dec 2026 \u{2013} 3 Jan 2027"
        );
        assert_eq!(
            period_name(Lang::Fr, Unit::Week, cross_year),
            "28 d\u{e9}c. 2026 \u{2013} 3 janv. 2027"
        );
        assert_eq!(period_name(Lang::En, Unit::Month, month), "November 2026");
        assert_eq!(period_name(Lang::Fr, Unit::Month, month), "novembre 2026");
        assert_eq!(period_name(Lang::En, Unit::Year, year), "2026");
        assert_eq!(period_name(Lang::Fr, Unit::Year, year), "2026");
        assert_eq!(
            period_name(Lang::En, Unit::Custom, custom),
            "1 Nov 2025 \u{2013} 31 Mar 2026"
        );
        assert_eq!(
            period_name(Lang::Fr, Unit::Custom, custom),
            "1 nov. 2025 \u{2013} 31 mars 2026"
        );
        assert_eq!(period_name(Lang::En, Unit::Hour1, day), "Thu 26 Nov 2026");
        assert_eq!(unit_chip(Lang::En, Unit::Hour1), "1h");
        assert_eq!(unit_chip(Lang::Fr, Unit::Hour6), "6\u{A0}h");
        assert_eq!(unit_chip(Lang::Fr, Unit::Week), "Semaine");
        assert_eq!(unit_chip(Lang::En, Unit::Custom), "Custom\u{2026}");
        assert_eq!(
            unit_chip(Lang::Fr, Unit::Custom),
            "Personnalis\u{e9}e\u{2026}"
        );
        assert_eq!(
            no_data_for(Lang::En, "November 2025"),
            "No data for November 2025"
        );
        assert_eq!(
            no_data_for(Lang::Fr, "novembre 2025"),
            "Aucune donn\u{e9}e pour novembre 2025"
        );
    }

    #[test]
    fn left_out_plural() {
        assert_eq!(
            left_out(Lang::En, 1, DailyPer::Day),
            "1 day left out of the comparison"
        );
        assert_eq!(
            left_out(Lang::En, 3, DailyPer::Day),
            "3 days left out of the comparison"
        );
        assert_eq!(
            left_out(Lang::En, 3, DailyPer::Month),
            "3 months left out of the comparison"
        );
        assert_eq!(
            left_out(Lang::En, 1, DailyPer::Hour),
            "1 hour left out of the comparison"
        );
        assert_eq!(
            left_out(Lang::Fr, 1, DailyPer::Day),
            "1 jour exclu de la comparaison"
        );
        assert_eq!(
            left_out(Lang::Fr, 3, DailyPer::Day),
            "3 jours exclus de la comparaison"
        );
        assert_eq!(
            left_out(Lang::Fr, 1, DailyPer::Month),
            "1 mois exclu de la comparaison"
        );
        assert_eq!(
            left_out(Lang::Fr, 3, DailyPer::Month),
            "3 mois exclus de la comparaison"
        );
        assert_eq!(
            left_out(Lang::Fr, 1, DailyPer::Hour),
            "1 heure exclue de la comparaison"
        );
        assert_eq!(
            left_out(Lang::Fr, 3, DailyPer::Hour),
            "3 heures exclues de la comparaison"
        );
    }

    #[test]
    fn a_named_period_words_its_totals() {
        assert_eq!(
            period_counter_legend(Lang::En, "November 2026", DailyPer::Day),
            "November 2026 · daily total"
        );
        assert_eq!(
            period_counter_legend(Lang::En, "2026", DailyPer::Month),
            "2026 · monthly total"
        );
        assert_eq!(
            period_counter_legend(Lang::Fr, "jeu. 26 nov. 2026", DailyPer::Hour),
            "jeu. 26 nov. 2026 · total horaire"
        );
        assert_eq!(
            period_total_line(Lang::En, "2026", 4.2),
            "2026 total: 4.2\u{a0}kWh"
        );
        assert_eq!(
            total_line(Lang::En, HistoryRange::Day, 4.2),
            "Last 24 hours total: 4.2\u{a0}kWh"
        );
    }
}
