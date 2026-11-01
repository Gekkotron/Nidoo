//! The schedule picker's pure half: the five friendly choices, their mapping to and from a
//! five-field cron expression, and a plain-language reading of any expression.

use chrono::Weekday;

use crate::i18n::Lang;
use crate::i18n::flows::{
    DaysText, custom_schedule, days_at, days_text, every_hour_at, every_minute, every_minutes,
};
use crate::i18n::format::weekday_short;

/// What the picker shows. `days` are 0 = Sunday … 6 = Saturday, sorted and without duplicates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedulePick {
    /// Every N minutes (1 is every minute).
    EveryMinutes(u8),
    /// Every hour at `:minute`.
    Hourly { minute: u8 },
    /// Every day at `hour:minute`.
    Daily { hour: u8, minute: u8 },
    /// On the listed weekdays at `hour:minute`.
    Weekdays { days: Vec<u8>, hour: u8, minute: u8 },
    /// Any other expression, edited as raw text.
    Advanced(String),
}

/// The weekday of a cron day index (0 = Sunday … 6 = Saturday).
fn weekday(day: u8) -> Weekday {
    [
        Weekday::Sun,
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
    ][usize::from(day % 7)]
}

/// The weekday names, Monday first, as `(day index, name)` for the picker's chips.
pub fn week_chips(lang: Lang) -> [(u8, &'static str); 7] {
    [1, 2, 3, 4, 5, 6, 0].map(|d| (d, weekday_short(lang, weekday(d))))
}

/// The cron expression of a pick.
pub fn to_cron(pick: &SchedulePick) -> String {
    match pick {
        SchedulePick::EveryMinutes(1) | SchedulePick::EveryMinutes(0) => "* * * * *".into(),
        SchedulePick::EveryMinutes(n) => format!("*/{n} * * * *"),
        SchedulePick::Hourly { minute } => format!("{minute} * * * *"),
        SchedulePick::Daily { hour, minute } => format!("{minute} {hour} * * *"),
        SchedulePick::Weekdays { days, hour, minute } => {
            format!("{minute} {hour} * * {}", days_field(days))
        }
        SchedulePick::Advanced(cron) => cron.clone(),
    }
}

/// `1-5` for three or more consecutive days, else a comma list.
fn days_field(days: &[u8]) -> String {
    let mut sorted: Vec<u8> = days.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() >= 3 && sorted.windows(2).all(|w| w[1] == w[0] + 1) {
        return format!("{}-{}", sorted[0], sorted[sorted.len() - 1]);
    }
    sorted
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn number(field: &str, max: u8) -> Option<u8> {
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) || field.len() > 2 {
        return None;
    }
    field.parse::<u8>().ok().filter(|n| *n <= max)
}

/// The days of a day-of-week field made of numbers, `a-b` ranges and commas.
fn parse_days(field: &str) -> Option<Vec<u8>> {
    let mut days = Vec::new();
    for item in field.split(',') {
        match item.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (number(a, 6)?, number(b, 6)?);
                if a > b {
                    return None;
                }
                days.extend(a..=b);
            }
            None => days.push(number(item, 6)?),
        }
    }
    days.sort_unstable();
    days.dedup();
    (!days.is_empty()).then_some(days)
}

/// The pick a cron expression falls under; anything the friendly choices cannot say is
/// `Advanced` with the text untouched.
pub fn from_cron(cron: &str) -> SchedulePick {
    let advanced = || SchedulePick::Advanced(cron.to_owned());
    let fields: Vec<&str> = cron.split_whitespace().collect();
    let [minute, hour, dom, month, dow] = fields[..] else {
        return advanced();
    };
    if dom != "*" || month != "*" {
        return advanced();
    }
    if (minute, hour, dow) == ("*", "*", "*") {
        return SchedulePick::EveryMinutes(1);
    }
    if let Some(step) = minute.strip_prefix("*/") {
        return match number(step, 59) {
            Some(n) if n >= 2 && (hour, dow) == ("*", "*") => SchedulePick::EveryMinutes(n),
            _ => advanced(),
        };
    }
    let Some(minute) = number(minute, 59) else {
        return advanced();
    };
    if hour == "*" && dow == "*" {
        return SchedulePick::Hourly { minute };
    }
    let Some(hour) = number(hour, 23) else {
        return advanced();
    };
    if dow == "*" {
        return SchedulePick::Daily { hour, minute };
    }
    match parse_days(dow) {
        Some(days) => SchedulePick::Weekdays { days, hour, minute },
        None => advanced(),
    }
}

/// A plain-language reading of a cron expression.
pub fn describe(lang: Lang, cron: &str) -> String {
    match from_cron(cron) {
        SchedulePick::EveryMinutes(1) | SchedulePick::EveryMinutes(0) => every_minute(lang),
        SchedulePick::EveryMinutes(n) => every_minutes(lang, n),
        SchedulePick::Hourly { minute } => every_hour_at(lang, minute),
        SchedulePick::Daily { hour, minute } => days_at(
            lang,
            days_text(lang, DaysText::EveryDay),
            &format!("{hour:02}:{minute:02}"),
        ),
        SchedulePick::Weekdays { days, hour, minute } => {
            let who = match days[..] {
                [1, 2, 3, 4, 5] => days_text(lang, DaysText::Weekdays).to_owned(),
                [0, 6] => days_text(lang, DaysText::Weekends).to_owned(),
                [0, 1, 2, 3, 4, 5, 6] => days_text(lang, DaysText::EveryDay).to_owned(),
                _ => days
                    .iter()
                    .map(|d| weekday_short(lang, weekday(*d)))
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            days_at(lang, &who, &format!("{hour:02}:{minute:02}"))
        }
        SchedulePick::Advanced(text) => custom_schedule(lang, &text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_map_to_cron_and_back() {
        for (pick, cron) in [
            (SchedulePick::EveryMinutes(10), "*/10 * * * *"),
            (SchedulePick::EveryMinutes(1), "* * * * *"),
            (SchedulePick::Hourly { minute: 15 }, "15 * * * *"),
            (
                SchedulePick::Daily {
                    hour: 7,
                    minute: 30,
                },
                "30 7 * * *",
            ),
            (
                SchedulePick::Weekdays {
                    days: vec![1, 2, 3, 4, 5],
                    hour: 7,
                    minute: 30,
                },
                "30 7 * * 1-5",
            ),
            (
                SchedulePick::Weekdays {
                    days: vec![0, 6],
                    hour: 9,
                    minute: 0,
                },
                "0 9 * * 0,6",
            ),
        ] {
            assert_eq!(to_cron(&pick), cron);
            assert_eq!(from_cron(cron), pick);
        }
        assert_eq!(
            from_cron("30 7 * * 1,2,3,4,5"),
            SchedulePick::Weekdays {
                days: vec![1, 2, 3, 4, 5],
                hour: 7,
                minute: 30
            }
        );
        assert_eq!(
            from_cron("0 */2 1 * *"),
            SchedulePick::Advanced("0 */2 1 * *".into())
        );
    }

    #[test]
    fn descriptions_read_as_english() {
        let d = |c: &str| describe(Lang::En, c);
        assert_eq!(d("*/10 * * * *"), "Every 10 minutes");
        assert_eq!(d("* * * * *"), "Every minute");
        assert_eq!(d("15 * * * *"), "Every hour at hh:15");
        assert_eq!(d("30 7 * * *"), "Every day at 07:30");
        assert_eq!(d("30 7 * * 1-5"), "Weekdays at 07:30");
        assert_eq!(d("0 9 * * 0,6"), "Weekends at 09:00");
        assert_eq!(d("0 9 * * 1,3"), "Mon, Wed at 09:00");
        assert_eq!(d("0 */2 1 * *"), "Custom: 0 */2 1 * *");
    }

    #[test]
    fn descriptions_read_as_french() {
        let d = |c: &str| describe(Lang::Fr, c);
        assert_eq!(d("*/10 * * * *"), "Toutes les 10 minutes");
        assert_eq!(d("* * * * *"), "Toutes les minutes");
        assert_eq!(d("15 * * * *"), "Toutes les heures à hh:15");
        assert_eq!(d("0 7 * * *"), "Tous les jours à 07:00");
        assert_eq!(d("0 7 * * 1-5"), "Du lundi au vendredi à 07:00");
        assert_eq!(d("0 9 * * 0,6"), "Le week-end à 09:00");
        assert_eq!(d("0 9 * * 1,3"), "Lun., Mer. à 09:00");
        assert_eq!(d("0 */2 1 * *"), "Personnalisé\u{A0}: 0 */2 1 * *");
    }

    #[test]
    fn week_chips_start_on_monday() {
        let chips = week_chips(Lang::Fr);
        assert_eq!(chips[0], (1, "Lun."));
        assert_eq!(chips[6], (0, "Dim."));
        assert_eq!(week_chips(Lang::En)[5], (6, "Sat"));
    }
}
