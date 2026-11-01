//! FR/EN text: the Energy screen, its diagram, cards and grid day chart.

use super::format::{decimal, with_unit};
use super::*;

/// Fixed words of the energy area. Sentences with values are the helper functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnergyText {
    // Node names.
    NodeGrid,
    NodeWater,
    NodeConsumer,
    Unavailable,
    NoEnergyMeter,
    // Screen.
    CheckSign,
    EmptyHint,
    GoHome,
    AllConsumers,
    // Grid day card.
    DayCard,
    Today,
    Yesterday,
    PrevDay,
    NextDay,
    HistoryOff,
    SetItUp,
    GiveGridRole,
    NoData,
    OffPeak,
    Peak,
    Bought,
    Sold,
    SolarExport,
    // Flux (Sankey) view.
    Export,
    BatteryCharging,
    Unitemised,
    ChipConsumed,
    ChipSolar,
    ChipFromGrid,
    ChipToGrid,
    ChipSolarShare,
    NoLivePower,
    // Inside the big circles.
    CircleProduction,
    CircleImport,
    CircleExport,
    CircleCharging,
    CircleDischarging,
    // Orbite view.
    ComingIn,
    GoingOut,
    InTheHouse,
}

/// A fixed word of the energy area.
pub fn energy_text(lang: Lang, k: EnergyText) -> &'static str {
    use EnergyText::*;
    let (en, fr) = match k {
        NodeGrid => ("Grid", "Réseau"),
        NodeWater => ("Water", "Eau"),
        NodeConsumer => ("Consumer", "Consommateur"),
        Unavailable => ("Unavailable", "Indisponible"),
        NoEnergyMeter => ("No energy meter", "Aucun compteur d'énergie"),
        CheckSign => (
            "Check the grid or battery sign",
            "Vérifiez le signe du réseau ou de la batterie",
        ),
        EmptyHint => (
            "Long-press a power sensor on Home and choose an energy role.",
            "Appuyez longuement sur un capteur de puissance dans Maison et choisissez un rôle énergétique.",
        ),
        GoHome => ("Go to Home", "Aller à l'accueil"),
        AllConsumers => ("All consumers", "Consommation"),
        DayCard => (
            "Grid exchange by half hour",
            "Échanges avec le réseau par demi-heure",
        ),
        Today => ("Today", "Aujourd'hui"),
        Yesterday => ("Yesterday", "Hier"),
        PrevDay => ("Previous day", "Jour précédent"),
        NextDay => ("Next day", "Jour suivant"),
        HistoryOff => (
            "History is off or unreachable.",
            "L'historique est désactivé ou injoignable.",
        ),
        SetItUp => ("Set it up", "Le configurer"),
        GiveGridRole => (
            "Give your grid meter the Grid role on its device page.",
            "Donnez le rôle Réseau à votre compteur sur la page de l'appareil.",
        ),
        NoData => ("No data for this day", "Aucune donnée pour ce jour"),
        OffPeak => ("Off-peak", "Heures creuses"),
        Peak => ("Peak", "Heures pleines"),
        Bought => ("Bought", "Acheté"),
        Sold => ("Sold", "Vendu"),
        SolarExport => ("Solar export", "Injection solaire"),
        Export => ("To the grid", "Vers le réseau"),
        BatteryCharging => ("Battery charging", "Charge batterie"),
        Unitemised => ("Not itemised", "Non détaillé"),
        ChipConsumed => ("consumed now", "consommés maintenant"),
        ChipSolar => ("solar production", "production solaire"),
        ChipFromGrid => ("from the grid", "tirés du réseau"),
        ChipToGrid => ("sent to the grid", "renvoyés au réseau"),
        ChipSolarShare => ("solar share", "part solaire"),
        NoLivePower => (
            "No live power reading yet",
            "Pas encore de mesure de puissance",
        ),
        CircleProduction => ("production", "production"),
        CircleImport => ("import", "import"),
        CircleExport => ("export", "export"),
        CircleCharging => ("charging", "charge"),
        CircleDischarging => ("discharging", "d\u{e9}charge"),
        ComingIn => ("Coming in", "Entrant"),
        GoingOut => ("Going out", "Sortant"),
        InTheHouse => ("in the house", "dans la maison"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The part names of a tooltip and a totals line: `(off-peak, peak, bought, sold)`, lower case.
pub fn part_names(lang: Lang) -> (&'static str, &'static str, &'static str, &'static str) {
    match lang {
        Lang::En => ("off-peak", "peak", "bought", "sold"),
        Lang::Fr => ("heures creuses", "heures pleines", "acheté", "vendu"),
    }
}

/// A power read aloud: "3.2 kilowatts", "400 watts" (the same words in both languages).
pub fn spoken_power(number: String, kilo: bool) -> String {
    format!("{number}\u{A0}{}", if kilo { "kilowatts" } else { "watts" })
}

/// A failed history call as a line: a translated lead, then the hub's own text as received.
pub fn history_error(lang: Lang, err: &crate::api::history::HistoryApiError) -> String {
    use crate::api::history::HistoryApiError as E;
    match err {
        E::NotRecorded => match lang {
            Lang::En => "Not recorded yet".into(),
            Lang::Fr => "Pas encore enregistré".into(),
        },
        E::Unavailable => energy_text(lang, EnergyText::HistoryOff).into(),
        E::NoGridMeter => energy_text(lang, EnergyText::GiveGridRole).into(),
        E::BadRequest(m) | E::Other(m) => match lang {
            Lang::En => format!("History error: {m}"),
            Lang::Fr => format!("Erreur d'historique\u{A0}: {m}"),
        },
    }
}

/// `s` with its first letter in lower case, to run a label into a sentence.
pub fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// "Live" or "Live · self-consumption 62 %".
pub fn live_subtitle(lang: Lang, self_consumption: Option<impl std::fmt::Display>) -> String {
    let live = text(lang, Key::Live);
    match self_consumption {
        Some(p) => {
            let word = match lang {
                Lang::En => "self-consumption",
                Lang::Fr => "autoconsommation",
            };
            format!("{live} · {word} {}", with_unit(p.to_string(), Some("%")))
        }
        None => live.to_string(),
    }
}

/// "Show all 5 consumers"; French takes the singular for 0 and 1.
pub fn show_all_consumers(lang: Lang, n: usize) -> String {
    let one = match lang {
        Lang::En => n == 1,
        Lang::Fr => n <= 1,
    };
    match (lang, one) {
        (Lang::En, true) => format!("Show all {n} consumer"),
        (Lang::En, false) => format!("Show all {n} consumers"),
        (Lang::Fr, true) => format!("Afficher {n} consommateur"),
        (Lang::Fr, false) => format!("Afficher les {n} consommateurs"),
    }
}

/// "72% solar" / "72 % solaire" (narrow no-break space before the percent sign in French).
pub fn solar_share_line(lang: Lang, pct: u8) -> String {
    match lang {
        Lang::En => format!("{pct}% solar"),
        Lang::Fr => format!("{pct}\u{202F}% solaire"),
    }
}

/// "+3 others".
pub fn others(lang: Lang, n: usize) -> String {
    match lang {
        Lang::En => format!("+{n} others"),
        Lang::Fr => format!("+{n} autres"),
    }
}

/// "Meter 601.359 m³".
pub fn meter_line(lang: Lang, value: &str) -> String {
    let word = match lang {
        Lang::En => "Meter",
        Lang::Fr => "Compteur",
    };
    format!("{word} {value}")
}

/// "12.4 kWh today".
pub fn kwh_today(lang: Lang, kwh: f64) -> String {
    let value = with_unit(decimal(lang, kwh, 1), Some("kWh"));
    match lang {
        Lang::En => format!("{value} today"),
        Lang::Fr => format!("{value} aujourd'hui"),
    }
}

/// A diagram sentence for screen readers, one per node kind and direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AriaPart {
    Solar,
    House,
    Importing,
    Exporting,
    BatterySupplies,
    BatteryCharging,
    Water,
}

/// "Solar 3.2 kilowatts to the house." from the spoken `value` ("3.2 kilowatts", "5 L").
pub fn aria_part(lang: Lang, part: AriaPart, value: &str) -> String {
    use AriaPart::*;
    match (lang, part) {
        (Lang::En, Solar) => format!("Solar {value} to the house."),
        (Lang::En, House) => format!("House uses {value}."),
        (Lang::En, Importing) => format!("Importing {value} from the grid."),
        (Lang::En, Exporting) => format!("Exporting {value} to the grid."),
        (Lang::En, BatterySupplies) => format!("Battery supplies {value}."),
        (Lang::En, BatteryCharging) => format!("Battery charging at {value}."),
        (Lang::En, Water) => format!("Water {value}."),
        (Lang::Fr, Solar) => format!("Solaire {value} vers la maison."),
        (Lang::Fr, House) => format!("La maison consomme {value}."),
        (Lang::Fr, Importing) => format!("Importation de {value} depuis le réseau."),
        (Lang::Fr, Exporting) => format!("Exportation de {value} vers le réseau."),
        (Lang::Fr, BatterySupplies) => format!("La batterie fournit {value}."),
        (Lang::Fr, BatteryCharging) => format!("La batterie se charge à {value}."),
        (Lang::Fr, Water) => format!("Eau {value}."),
    }
}

/// "Grid on Yesterday: bought 17.1 kWh, …." from the day label and the lower-case parts.
pub fn grid_summary(lang: Lang, day: &str, parts: &str) -> String {
    match lang {
        Lang::En => format!("Grid on {day}: {parts}."),
        Lang::Fr => format!("Réseau, {}\u{A0}: {parts}.", lower_first(day)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_energy_text_has_two_languages() {
        use EnergyText::*;
        const ALL: &[EnergyText] = &[
            NodeGrid,
            NodeWater,
            NodeConsumer,
            Unavailable,
            NoEnergyMeter,
            CheckSign,
            EmptyHint,
            GoHome,
            AllConsumers,
            DayCard,
            Today,
            Yesterday,
            PrevDay,
            NextDay,
            HistoryOff,
            SetItUp,
            GiveGridRole,
            NoData,
            OffPeak,
            Peak,
            Bought,
            Sold,
            SolarExport,
            Export,
            BatteryCharging,
            Unitemised,
            ChipConsumed,
            ChipSolar,
            ChipFromGrid,
            ChipToGrid,
            ChipSolarShare,
            NoLivePower,
            CircleProduction,
            CircleImport,
            CircleExport,
            CircleCharging,
            CircleDischarging,
            ComingIn,
            GoingOut,
            InTheHouse,
        ];
        crate::i18n::assert_table(
            ALL,
            energy_text,
            &[CircleProduction, CircleImport, CircleExport],
        );
    }

    #[test]
    fn history_errors_are_led_in_french() {
        use crate::api::history::HistoryApiError as E;
        assert_eq!(history_error(Lang::En, &E::NotRecorded), "Not recorded yet");
        assert_eq!(
            history_error(Lang::Fr, &E::NotRecorded),
            "Pas encore enregistré"
        );
        assert_eq!(
            history_error(Lang::Fr, &E::Other("boom".into())),
            "Erreur d'historique\u{A0}: boom"
        );
        assert_eq!(
            history_error(Lang::En, &E::BadRequest("bad".into())),
            "History error: bad"
        );
    }

    #[test]
    fn solar_share_line_in_both_languages() {
        assert_eq!(solar_share_line(Lang::En, 72), "72% solar");
        assert_eq!(solar_share_line(Lang::Fr, 72), "72\u{202F}% solaire");
    }

    #[test]
    fn others_in_both_languages() {
        assert_eq!(others(Lang::En, 3), "+3 others");
        assert_eq!(others(Lang::Fr, 3), "+3 autres");
    }

    #[test]
    fn spoken_power_keeps_number_and_unit_together() {
        assert_eq!(spoken_power("3,2".into(), true), "3,2\u{A0}kilowatts");
        assert_eq!(spoken_power("400".into(), false), "400\u{A0}watts");
    }

    #[test]
    fn consumer_counts_at_0_1_and_2() {
        assert_eq!(show_all_consumers(Lang::En, 1), "Show all 1 consumer");
        assert_eq!(show_all_consumers(Lang::Fr, 0), "Afficher 0 consommateur");
        assert_eq!(show_all_consumers(Lang::Fr, 1), "Afficher 1 consommateur");
        assert_eq!(
            show_all_consumers(Lang::Fr, 2),
            "Afficher les 2 consommateurs"
        );
    }

    #[test]
    fn subtitle_and_today_in_both_languages() {
        assert_eq!(live_subtitle(Lang::En, None::<u8>), "Live");
        assert_eq!(
            live_subtitle(Lang::Fr, Some(62)),
            "En direct · autoconsommation 62\u{A0}%"
        );
        assert_eq!(kwh_today(Lang::Fr, 12.4), "12,4\u{A0}kWh aujourd'hui");
        assert_eq!(kwh_today(Lang::En, 12.4), "12.4\u{A0}kWh today");
    }
}
