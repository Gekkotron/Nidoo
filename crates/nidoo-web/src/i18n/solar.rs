//! FR/EN text: Solar forecast card and page.

use chrono::NaiveTime;

use super::*;

/// A fixed word of the solar card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolarText {
    CardLabel,
    Tomorrow,
    Kwh,
    Kw,
    Today,
    Peak,
    BestHours,
    Gap,
    GapSub,
    Yesterday,
    Accuracy,
    Roofs,
    Roof,
    Month,
    NextDays,
    Unavailable,
    EnableHint,
    Credit,
    LegendRange,
    LegendForecast,
    LegendMeasured,
    LegendSoFar,
    LegendNow,
}

/// A fixed word of the solar card.
pub fn solar_text(lang: Lang, k: SolarText) -> &'static str {
    use SolarText::*;
    let (en, fr) = match k {
        CardLabel => ("Solar forecast", "Prévisions solaires"),
        Tomorrow => ("Tomorrow", "Demain"),
        Kwh => ("kWh", "kWh"),
        Kw => ("kW", "kW"),
        Today => ("Today", "Aujourd'hui"),
        Peak => ("Forecast peak", "Pic prévu"),
        BestHours => ("Best hours", "Meilleures heures"),
        Gap => ("Gap", "Écart"),
        GapSub => (
            "measured vs forecast so far",
            "mesuré vs prévu à cette heure",
        ),
        Yesterday => ("Yesterday", "Hier"),
        Accuracy => ("7-day accuracy", "Précision sur 7 jours"),
        Roofs => ("Roofs", "Toits"),
        Roof => ("Roof", "Toit"),
        Month => ("This month", "Ce mois-ci"),
        NextDays => ("Next days", "Prochains jours"),
        Unavailable => (
            "The solar forecast is unavailable.",
            "Les prévisions solaires sont indisponibles.",
        ),
        EnableHint => (
            "Enable [solcast] in oikos.toml and put the API key in secrets/solcast.key.",
            "Activez [solcast] dans oikos.toml et placez la clé d'API dans secrets/solcast.key.",
        ),
        Credit => ("Forecasts by Solcast", "Prévisions Solcast"),
        LegendRange => ("Likely range", "Fourchette probable"),
        LegendForecast => ("Forecast", "Prévision"),
        LegendMeasured => ("Measured", "Mesuré"),
        LegendSoFar => ("Estimated so far", "Estimé jusqu'ici"),
        LegendNow => ("Now", "Maintenant"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// Binds the words of one part of a line with no-break spaces, so a narrow box only wraps
/// between parts.
fn no_break(part: &str) -> String {
    part.replace(' ', "\u{a0}")
}

/// The solar card's and page's headline: "5.2 kWh produced · 3.8 kWh forecast", or the forecast
/// so far with "≈" while nothing is measured. The forecast part is left out without a total.
pub fn solar_title(lang: Lang, measured: Option<f64>, so_far: f64, total: Option<f64>) -> String {
    let unit = solar_text(lang, SolarText::Kwh);
    let done = match (measured, lang) {
        (Some(m), Lang::En) => format!("{} {unit} produced", solar_kwh(lang, m)),
        (Some(m), Lang::Fr) => format!("{} {unit} produits", solar_kwh(lang, m)),
        (None, Lang::En) => format!("≈ {} {unit} so far", solar_kwh(lang, so_far)),
        (None, Lang::Fr) => format!("≈ {} {unit} jusqu'ici", solar_kwh(lang, so_far)),
    };
    let forecast = total.map(|t| match lang {
        Lang::En => format!("{} {unit} forecast", solar_kwh(lang, t)),
        Lang::Fr => format!("prévu {} {unit}", solar_kwh(lang, t)),
    });
    [Some(done), forecast]
        .into_iter()
        .flatten()
        .map(|p| no_break(&p))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A whole hour as a short axis label: "12 h" in French, "12:00" in English.
pub fn solar_hour(lang: Lang, hour: u32) -> String {
    match lang {
        Lang::Fr => format!("{hour} h"),
        Lang::En => format!("{:02}:00", hour % 24),
    }
}

/// "Range 9.8–14.1 kWh".
pub fn solar_range(lang: Lang, low: f64, high: f64) -> String {
    let word = match lang {
        Lang::En => "Range",
        Lang::Fr => "Fourchette",
    };
    format!(
        "{word} {}–{} {}",
        solar_kwh(lang, low),
        solar_kwh(lang, high),
        solar_text(lang, SolarText::Kwh)
    )
}

/// A kW figure with one decimal: "0.8 kW".
pub fn solar_kw(lang: Lang, v: f64) -> String {
    format!(
        "{} {}",
        home_number(lang, v + 0.0, 1),
        solar_text(lang, SolarText::Kw)
    )
}

/// "Now 0.8 kW · forecast 0.7 kW"; the measured part is left out without a measure.
pub fn solar_power_line(lang: Lang, measured: Option<f64>, forecast: Option<f64>) -> String {
    let (now, fc) = match lang {
        Lang::En => ("Now", "forecast"),
        Lang::Fr => ("Maintenant", "prévu"),
    };
    match (measured, forecast) {
        (Some(m), Some(f)) => format!("{now} {} · {fc} {}", solar_kw(lang, m), solar_kw(lang, f)),
        (Some(m), None) => format!("{now} {}", solar_kw(lang, m)),
        (None, Some(f)) => format!("{now} {fc} {}", solar_kw(lang, f)),
        (None, None) => String::new(),
    }
}

/// "above 0.5 kW".
pub fn solar_above(lang: Lang, kw: f64) -> String {
    match lang {
        Lang::En => format!("above {}", solar_kw(lang, kw)),
        Lang::Fr => format!("au-dessus de {}", solar_kw(lang, kw)),
    }
}

/// A signed whole percent: "−11 %", "+4 %", "0 %".
pub fn solar_percent(lang: Lang, p: i32) -> String {
    let sign = match p {
        p if p < 0 => "−",
        0 => "",
        _ => "+",
    };
    format!("{sign}{} %", home_number(lang, f64::from(p.abs()), 0))
}

/// The sub-line of yesterday's tile: "forecast 11.6 · −6 %", or, for a forecast made late,
/// "forecast at 09:42 11.6".
pub fn solar_yesterday_sub(
    lang: Lang,
    forecast: f64,
    gap: Option<i32>,
    made_late_at: Option<NaiveTime>,
) -> String {
    let word = match lang {
        Lang::En => "forecast",
        Lang::Fr => "prévu",
    };
    let f = solar_kwh(lang, forecast);
    if let Some(at) = made_late_at {
        let at_word = match lang {
            Lang::En => "at",
            Lang::Fr => "à",
        };
        return format!("{word} {at_word} {} {f}", home_time(lang, at));
    }
    match gap {
        Some(g) => format!("{word} {f} · {}", solar_percent(lang, g)),
        None => format!("{word} {f}"),
    }
}

/// "Roof 1 5.1 · Roof 2 7.3" from `(position, kWh)` pairs.
pub fn solar_roofs(lang: Lang, roofs: &[(u32, Option<f64>)]) -> String {
    roofs
        .iter()
        .map(|(n, kwh)| {
            format!(
                "{} {n} {}",
                solar_text(lang, SolarText::Roof),
                kwh.map_or_else(|| "–".to_owned(), |k| solar_kwh(lang, k))
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The page footer: "Updated at 21:02 · next around 22:00 · Solcast calls 3 / 10 today";
/// the "next" part is left out when no call is planned.
pub fn solar_footer(
    lang: Lang,
    updated: Option<NaiveTime>,
    next: Option<NaiveTime>,
    calls: u32,
    limit: u32,
) -> String {
    let (next_word, calls_fmt) = match lang {
        Lang::En => (
            "next around",
            format!("Solcast calls {calls} / {limit} today"),
        ),
        Lang::Fr => (
            "prochaine vers",
            format!("appels Solcast {calls} / {limit} aujourd'hui"),
        ),
    };
    [
        updated.map(|at| updated_at(lang, at)),
        next.map(|at| format!("{next_word} {}", home_time(lang, at))),
        Some(calls_fmt),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// kWh with one decimal and the language's separator, without the unit.
pub fn solar_kwh(lang: Lang, v: f64) -> String {
    home_number(lang, v + 0.0, 1)
}

/// The solar card's second line: the end-of-day projection (measured plus the forecast still to
/// come; "more expected" while nothing is measured) and tomorrow's range; a part without data is
/// left out, and so is the projection once less than 0.05 kWh is left. Tomorrow is
/// `(estimate, low, high)` in kWh.
pub fn solar_bottom(
    lang: Lang,
    remaining: Option<f64>,
    measured: Option<f64>,
    tomorrow: Option<(f64, f64, f64)>,
) -> String {
    let unit = solar_text(lang, SolarText::Kwh);
    let projection = remaining
        .filter(|r| *r >= 0.05)
        .map(|r| match (measured, lang) {
            (Some(m), Lang::En) => format!("≈ {} {unit} by tonight", solar_kwh(lang, m + r)),
            (Some(m), Lang::Fr) => format!("fin de journée ≈ {} {unit}", solar_kwh(lang, m + r)),
            (None, Lang::En) => format!("≈ {} {unit} more expected", solar_kwh(lang, r)),
            (None, Lang::Fr) => format!("encore ≈ {} {unit} prévus", solar_kwh(lang, r)),
        });
    let next = tomorrow.map(|(t, lo, hi)| {
        format!(
            "{} {} {unit} ({}–{})",
            solar_text(lang, SolarText::Tomorrow),
            solar_kwh(lang, t),
            home_number(lang, lo.round() + 0.0, 0),
            home_number(lang, hi.round() + 0.0, 0),
        )
    });
    [projection, next]
        .into_iter()
        .flatten()
        .map(|p| no_break(&p))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solar_copy() {
        for lang in [Lang::En, Lang::Fr] {
            for k in [
                SolarText::CardLabel,
                SolarText::Tomorrow,
                SolarText::Kwh,
                SolarText::Kw,
                SolarText::Today,
                SolarText::Peak,
                SolarText::BestHours,
                SolarText::Gap,
                SolarText::GapSub,
                SolarText::Yesterday,
                SolarText::Accuracy,
                SolarText::Roofs,
                SolarText::Roof,
                SolarText::Month,
                SolarText::NextDays,
                SolarText::Unavailable,
                SolarText::EnableHint,
                SolarText::Credit,
            ] {
                assert!(!solar_text(lang, k).is_empty(), "{k:?}");
            }
        }
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let nb = |s: &str| s.replace('~', "\u{a0}");
        assert_eq!(
            solar_title(Lang::En, Some(5.2), 0.0, Some(3.8)),
            nb("5.2~kWh~produced · 3.8~kWh~forecast")
        );
        assert_eq!(
            solar_title(Lang::Fr, Some(5.2), 0.0, Some(3.8)),
            nb("5,2~kWh~produits · prévu~3,8~kWh")
        );
        assert_eq!(
            solar_title(Lang::En, None, 1.2, Some(3.8)),
            nb("≈~1.2~kWh~so~far · 3.8~kWh~forecast")
        );
        assert_eq!(
            solar_title(Lang::Fr, None, 1.2, Some(3.8)),
            nb("≈~1,2~kWh~jusqu'ici · prévu~3,8~kWh")
        );
        assert_eq!(
            solar_title(Lang::En, Some(5.2), 0.0, None),
            nb("5.2~kWh~produced")
        );
        assert_eq!(solar_hour(Lang::Fr, 12), "12 h");
        assert_eq!(solar_hour(Lang::En, 12), "12:00");
        assert_eq!(solar_hour(Lang::En, 9), "09:00");
        assert_eq!(solar_hour(Lang::En, 0), "00:00");
        assert_eq!(solar_hour(Lang::En, 18), "18:00");
        assert_eq!(solar_range(Lang::En, 9.8, 14.1), "Range 9.8–14.1 kWh");
        assert_eq!(solar_range(Lang::Fr, 9.8, 14.1), "Fourchette 9,8–14,1 kWh");
        assert_eq!(
            solar_power_line(Lang::En, Some(0.8), Some(0.7)),
            "Now 0.8 kW · forecast 0.7 kW"
        );
        assert_eq!(
            solar_power_line(Lang::Fr, Some(0.8), Some(0.7)),
            "Maintenant 0,8 kW · prévu 0,7 kW"
        );
        assert_eq!(
            solar_power_line(Lang::En, None, Some(0.7)),
            "Now forecast 0.7 kW"
        );
        assert_eq!(solar_above(Lang::Fr, 0.5), "au-dessus de 0,5 kW");
        assert_eq!(solar_percent(Lang::En, -11), "−11 %");
        assert_eq!(solar_percent(Lang::En, 4), "+4 %");
        assert_eq!(solar_percent(Lang::En, 0), "0 %");
        assert_eq!(
            solar_yesterday_sub(Lang::En, 11.6, Some(-6), None),
            "forecast 11.6 · −6 %"
        );
        assert_eq!(
            solar_yesterday_sub(Lang::En, 11.6, Some(-6), Some(t(9, 42))),
            "forecast at 09:42 11.6"
        );
        assert_eq!(
            solar_roofs(Lang::Fr, &[(1, Some(5.1)), (2, Some(7.3))]),
            "Toit 1 5,1 · Toit 2 7,3"
        );
        assert_eq!(
            solar_footer(Lang::En, Some(t(21, 2)), Some(t(22, 0)), 3, 10),
            "Updated at 21:02 · next around 22:00 · Solcast calls 3 / 10 today"
        );
        assert_eq!(
            solar_footer(Lang::Fr, Some(t(21, 2)), None, 3, 10),
            "Mis à jour à 21 h 02 · appels Solcast 3 / 10 aujourd'hui"
        );
        assert_eq!(solar_kwh(Lang::Fr, 6.44), "6,4");
        assert_eq!(solar_kwh(Lang::En, 6.44), "6.4");
        let t = Some((1.9, 1.2, 2.6));
        assert_eq!(
            solar_bottom(Lang::En, Some(1.6), Some(4.0), t),
            nb("≈~5.6~kWh~by~tonight · Tomorrow~1.9~kWh~(1–3)")
        );
        assert_eq!(
            solar_bottom(Lang::Fr, Some(1.6), Some(4.0), t),
            nb("fin~de~journée~≈~5,6~kWh · Demain~1,9~kWh~(1–3)")
        );
        assert_eq!(
            solar_bottom(Lang::En, Some(1.6), None, t),
            nb("≈~1.6~kWh~more~expected · Tomorrow~1.9~kWh~(1–3)")
        );
        assert_eq!(
            solar_bottom(Lang::Fr, Some(1.6), None, None),
            nb("encore~≈~1,6~kWh~prévus")
        );
        assert_eq!(
            solar_bottom(Lang::En, Some(0.04), Some(4.0), t),
            nb("Tomorrow~1.9~kWh~(1–3)")
        );
        assert_eq!(solar_bottom(Lang::En, None, Some(4.0), None), "");
    }
    #[test]
    fn every_solar_text_has_two_languages() {
        const ALL: &[SolarText] = &[
            SolarText::CardLabel,
            SolarText::Tomorrow,
            SolarText::Kwh,
            SolarText::Kw,
            SolarText::Today,
            SolarText::Peak,
            SolarText::BestHours,
            SolarText::Gap,
            SolarText::GapSub,
            SolarText::Yesterday,
            SolarText::Accuracy,
            SolarText::Roofs,
            SolarText::Roof,
            SolarText::Month,
            SolarText::NextDays,
            SolarText::Unavailable,
            SolarText::EnableHint,
            SolarText::Credit,
        ];
        crate::i18n::assert_table(ALL, solar_text, &[SolarText::Kwh, SolarText::Kw]);
    }
}
