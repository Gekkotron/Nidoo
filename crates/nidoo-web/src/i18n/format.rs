//! Numbers, dates, times, relative times and counts in the UI language.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};

use super::Lang;

const DASH: &str = "—";
const NBSP: char = '\u{A0}';

/// Integers without a decimal, everything else with one; from 1000 up no decimals and grouped
/// thousands. French: "21,5", "1 250" (narrow no-break space); English: "21.5", "1 250" (thin
/// space). "—" when not finite.
pub fn number(lang: Lang, n: f64) -> String {
    if !n.is_finite() {
        return DASH.into();
    }
    if n.abs() >= 1000.0 {
        let sep = match lang {
            Lang::Fr => '\u{202F}',
            Lang::En => '\u{2009}',
        };
        let digits = format!("{:.0}", n.abs());
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(sep);
            }
            out.push(c);
        }
        return if n < 0.0 { format!("-{out}") } else { out };
    }
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        decimal(lang, n, 1)
    }
}

/// `v` with `decimals` places and the language's decimal separator.
pub fn decimal(lang: Lang, v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    match lang {
        Lang::Fr => s.replace('.', ","),
        Lang::En => s,
    }
}

/// Like [`decimal`], with thousands grouped from 1000 up: "1 234,5" (U+202F) / "1 234.5" (U+2009).
pub fn decimal_grouped(lang: Lang, v: f64, decimals: usize) -> String {
    let plain = decimal(lang, v, decimals);
    let sep = match lang {
        Lang::Fr => '\u{202F}',
        Lang::En => '\u{2009}',
    };
    let (sign, rest) = match plain.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", plain.as_str()),
    };
    let (int, frac) = rest.split_at(rest.find(['.', ',']).unwrap_or(rest.len()));
    if int.len() <= 3 || !int.bytes().all(|b| b.is_ascii_digit()) {
        return plain;
    }
    let mut out = String::from(sign);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out.push_str(frac);
    out
}

/// `value` then a no-break space and `unit`; `value` alone without a unit or when it is "—".
pub fn with_unit(value: String, unit: Option<&str>) -> String {
    match unit {
        Some(u) if !u.is_empty() && value != DASH => format!("{value}{NBSP}{u}"),
        _ => value,
    }
}

/// A number as it goes into an input: the shortest text that reads back as `v`, with the
/// language's decimal separator and no thousands grouping ("0,75", "20.25").
pub fn input_number(lang: Lang, v: f64) -> String {
    let s = format!("{v}");
    match lang {
        Lang::Fr => s.replace('.', ","),
        Lang::En => s,
    }
}

/// A number typed by hand: "21,5" or "21.5", thousands separated by spaces or not. Digits, one
/// separator and a leading minus only: no exponent, no "NaN" or "inf".
pub fn parse_decimal(s: &str) -> Option<f64> {
    let cleaned: String = s
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{A0}' | '\u{2009}' | '\u{202F}'))
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    let digits = cleaned.strip_prefix('-').unwrap_or(&cleaned);
    if !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    cleaned.parse().ok().filter(|v: &f64| v.is_finite())
}

/// "14:05" in both languages.
pub fn time(t: NaiveTime) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// "Sam." / "Sat".
pub fn weekday_short(lang: Lang, d: Weekday) -> &'static str {
    const EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const FR: [&str; 7] = ["Lun.", "Mar.", "Mer.", "Jeu.", "Ven.", "Sam.", "Dim."];
    let i = d.num_days_from_monday() as usize;
    match lang {
        Lang::Fr => FR[i],
        Lang::En => EN[i],
    }
}

/// "Samedi" / "Saturday".
pub fn weekday_long(lang: Lang, d: Weekday) -> &'static str {
    const EN: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    const FR: [&str; 7] = [
        "Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche",
    ];
    let i = d.num_days_from_monday() as usize;
    match lang {
        Lang::Fr => FR[i],
        Lang::En => EN[i],
    }
}

/// `month` 1–12 as "octobre" / "October"; "" out of range.
pub fn month_long(lang: Lang, month: u32) -> &'static str {
    const EN: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const FR: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    month_in(lang, month, &EN, &FR)
}

/// `month` 1–12 as "oct." / "Oct"; "" out of range.
pub fn month_short(lang: Lang, month: u32) -> &'static str {
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const FR: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    month_in(lang, month, &EN, &FR)
}

fn month_in(
    lang: Lang,
    month: u32,
    en: &[&'static str; 12],
    fr: &[&'static str; 12],
) -> &'static str {
    let table = match lang {
        Lang::Fr => fr,
        Lang::En => en,
    };
    month
        .checked_sub(1)
        .and_then(|i| table.get(i as usize))
        .copied()
        .unwrap_or("")
}

/// "24 oct." / "Oct 24".
pub fn date_short(lang: Lang, d: NaiveDate) -> String {
    let month = month_short(lang, d.month());
    match lang {
        Lang::Fr => format!("{} {month}", d.day()),
        Lang::En => format!("{month} {}", d.day()),
    }
}

/// "24 oct. 2026, 14:05" / "Oct 24 2026, 14:05".
pub fn date_time(lang: Lang, t: NaiveDateTime) -> String {
    format!(
        "{} {}, {}",
        date_short(lang, t.date()),
        t.year(),
        time(t.time())
    )
}

/// How long ago, from `secs` seconds: "à l'instant", "il y a 5 min", "il y a 3 h", "hier",
/// "il y a 4 j" / "just now", "5 min ago", "3 h ago", "yesterday", "4 d ago". Callers compute
/// `secs` as `now.saturating_sub(then)`, so a timestamp in the future reads "just now" instead of
/// underflowing.
pub fn ago(lang: Lang, secs: u64) -> String {
    let (n, fr_unit, en_unit) = match secs {
        0..60 => {
            return match lang {
                Lang::Fr => "à l'instant".into(),
                Lang::En => "just now".into(),
            };
        }
        60..3600 => (secs / 60, "min", "min"),
        3600..86_400 => (secs / 3600, "h", "h"),
        86_400..172_800 => {
            return match lang {
                Lang::Fr => "hier".into(),
                Lang::En => "yesterday".into(),
            };
        }
        _ => (secs / 86_400, "j", "d"),
    };
    match lang {
        Lang::Fr => format!("il y a {n}{NBSP}{fr_unit}"),
        Lang::En => format!("{n}{NBSP}{en_unit} ago"),
    }
}

/// "{n} {form}" where each form is `(en, fr)`. French takes `one` for 0 and 1, English for 1.
pub fn count(lang: Lang, n: usize, one: (&str, &str), other: (&str, &str)) -> String {
    let singular = match lang {
        Lang::Fr => n <= 1,
        Lang::En => n == 1,
    };
    let (en, fr) = if singular { one } else { other };
    let form = match lang {
        Lang::Fr => fr,
        Lang::En => en,
    };
    format!("{n} {form}")
}

/// `unix` seconds on the browser's clock; UTC off the browser.
pub fn local_date_time(unix: u64) -> NaiveDateTime {
    #[cfg(target_arch = "wasm32")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(unix as f64 * 1000.0));
        NaiveDate::from_ymd_opt(d.get_full_year() as i32, d.get_month() + 1, d.get_date())
            .and_then(|day| day.and_hms_opt(d.get_hours(), d.get_minutes(), d.get_seconds()))
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        i64::try_from(unix)
            .ok()
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            .map(|t| t.naive_utc())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn grouped_decimals_group_thousands_in_both_languages() {
        use super::*;
        assert_eq!(decimal_grouped(Lang::Fr, 1234.5, 1), "1\u{202F}234,5");
        assert_eq!(decimal_grouped(Lang::En, 1234.5, 1), "1\u{2009}234.5");
        assert_eq!(decimal_grouped(Lang::Fr, -4321.74, 1), "-4\u{202F}321,7");
        assert_eq!(decimal_grouped(Lang::Fr, 999.9, 1), "999,9");
        assert_eq!(decimal_grouped(Lang::Fr, 3.0, 1), "3,0");
    }

    #[test]
    fn input_numbers_keep_every_digit() {
        use super::{Lang, input_number, parse_decimal};
        assert_eq!(input_number(Lang::En, 0.75), "0.75");
        assert_eq!(input_number(Lang::Fr, 0.75), "0,75");
        assert_eq!(input_number(Lang::En, 20.25), "20.25");
        assert_eq!(input_number(Lang::Fr, 20.25), "20,25");
        assert_eq!(input_number(Lang::Fr, 1500.0), "1500");
        for v in [0.04, 20.25, 1234.5] {
            for lang in [Lang::En, Lang::Fr] {
                assert_eq!(parse_decimal(&input_number(lang, v)), Some(v));
            }
        }
    }

    use chrono::{NaiveDate, NaiveTime, Weekday};

    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn numbers_follow_the_language() {
        assert_eq!(number(Lang::Fr, 21.46), "21,5");
        assert_eq!(number(Lang::En, 21.46), "21.5");
        assert_eq!(number(Lang::Fr, 1250.0), "1\u{202F}250");
        assert_eq!(number(Lang::En, 1250.0), "1\u{2009}250");
        assert_eq!(number(Lang::Fr, -1250.0), "-1\u{202F}250");
        assert_eq!(number(Lang::Fr, 20.0), "20");
        assert_eq!(number(Lang::Fr, f64::NAN), "—");
        assert_eq!(decimal(Lang::Fr, 3.0, 2), "3,00");
        assert_eq!(with_unit(number(Lang::Fr, 9.5), Some("°C")), "9,5\u{A0}°C");
    }

    #[test]
    fn decimals_parse_with_either_separator() {
        assert_eq!(parse_decimal("21,5"), Some(21.5));
        assert_eq!(parse_decimal(" 21.5 "), Some(21.5));
        assert_eq!(parse_decimal("1\u{202F}250,5"), Some(1250.5));
        assert_eq!(parse_decimal("abc"), None);
        assert_eq!(parse_decimal(""), None);
        for bad in ["NaN", "inf", "-infinity", "1e3", "2,5e1"] {
            assert_eq!(parse_decimal(bad), None, "{bad}");
        }
        assert_eq!(parse_decimal("-3,5"), Some(-3.5));
    }

    #[test]
    fn dates_and_times_follow_the_language() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        let t = NaiveTime::from_hms_opt(14, 5, 0).unwrap();
        assert_eq!(time(t), "14:05");
        assert_eq!(date_short(Lang::Fr, d), "24 oct.");
        assert_eq!(date_short(Lang::En, d), "Oct 24");
        assert_eq!(date_time(Lang::Fr, d.and_time(t)), "24 oct. 2026, 14:05");
        assert_eq!(date_time(Lang::En, d.and_time(t)), "Oct 24 2026, 14:05");
        assert_eq!(month_short(Lang::Fr, 2), "févr.");
        assert_eq!(weekday_short(Lang::Fr, Weekday::Sat), "Sam.");
        assert_eq!(
            crate::i18n::home_time(Lang::En, NaiveTime::from_hms_opt(21, 2, 0).unwrap()),
            "21:02"
        );
    }

    #[test]
    fn relative_times() {
        assert_eq!(ago(Lang::Fr, 30), "à l'instant");
        assert_eq!(ago(Lang::Fr, 300), "il y a 5\u{A0}min");
        assert_eq!(ago(Lang::En, 300), "5\u{A0}min ago");
        assert_eq!(ago(Lang::Fr, 3 * 3600), "il y a 3\u{A0}h");
        assert_eq!(ago(Lang::En, 30 * 3600), "yesterday");
        assert_eq!(ago(Lang::Fr, 4 * 86_400), "il y a 4\u{A0}j");
    }

    #[test]
    fn counts_agree_at_zero_and_one() {
        let one = ("light on", "lumière allumée");
        let other = ("lights on", "lumières allumées");
        assert_eq!(count(Lang::Fr, 0, one, other), "0 lumière allumée");
        assert_eq!(count(Lang::Fr, 1, one, other), "1 lumière allumée");
        assert_eq!(count(Lang::Fr, 2, one, other), "2 lumières allumées");
        assert_eq!(count(Lang::En, 0, one, other), "0 lights on");
        assert_eq!(count(Lang::En, 1, one, other), "1 light on");
    }
}
