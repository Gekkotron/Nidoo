//! Reading a spoken answer: which choice was heard, and the first number said.
//! Pure text helpers, no I/O.

use super::model::AskAnswer;
use super::template::Lang;

/// What the satellite says when an answer matched no choice.
pub fn not_understood(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Je n'ai pas compris.",
        Lang::En => "I didn't understand.",
    }
}

/// Lowercases, folds accents and turns every other non-alphanumeric run into one space.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        };
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
    }
    out.truncate(out.trim_end().len());
    out
}

const YES: &[&str] = &[
    "oui",
    "ouais",
    "ouai",
    "yes",
    "yeah",
    "vas y",
    "d accord",
    "ok",
    "okay",
    "volontiers",
    "bien sur",
    "go",
];
const NO: &[&str] = &[
    "non",
    "no",
    "nope",
    "pas maintenant",
    "non merci",
    "laisse",
    "laisse tomber",
    "pas la peine",
    "not now",
    "no thanks",
    "pas d accord",
    "pas ok",
    "pas okay",
    "not ok",
    "not okay",
    "no go",
    "surtout pas",
];

/// The index of the answer that was heard: the one with the longest matching phrase
/// (its label, its extra words, and the usual yes/no phrasings), the first on a tie.
pub fn match_answer(answers: &[AskAnswer], heard: &str) -> Option<usize> {
    let heard = normalize(heard);
    let heard: Vec<&str> = heard.split(' ').filter(|t| !t.is_empty()).collect();
    let mut best: Option<(usize, usize)> = None; // (score, index)
    for (index, answer) in answers.iter().enumerate() {
        let label = normalize(&answer.label);
        let built_in: &[&str] = match label.as_str() {
            "oui" | "yes" => YES,
            "non" | "no" => NO,
            _ => &[],
        };
        let score = std::iter::once(label.clone())
            .chain(answer.words.iter().map(|w| normalize(w)))
            .chain(built_in.iter().map(|p| (*p).to_owned()))
            .filter_map(|phrase| {
                let tokens: Vec<&str> = phrase.split(' ').filter(|t| !t.is_empty()).collect();
                let found = !tokens.is_empty() && heard.windows(tokens.len()).any(|w| w == tokens);
                found.then_some(tokens.len())
            })
            .max();
        if let Some(score) = score
            && best.is_none_or(|(top, _)| score > top)
        {
            best = Some((score, index));
        }
    }
    best.map(|(_, index)| index)
}

/// Words that are a number on their own (folded, French and English).
const ATOMS: &[(&[&str], u32)] = &[
    (&["zero"], 0),
    (&["un"], 1),
    (&["une"], 1),
    (&["one"], 1),
    (&["deux"], 2),
    (&["two"], 2),
    (&["trois"], 3),
    (&["three"], 3),
    (&["quatre"], 4),
    (&["four"], 4),
    (&["cinq"], 5),
    (&["five"], 5),
    (&["six"], 6),
    (&["sept"], 7),
    (&["seven"], 7),
    (&["huit"], 8),
    (&["eight"], 8),
    (&["neuf"], 9),
    (&["nine"], 9),
    (&["dix"], 10),
    (&["ten"], 10),
    (&["onze"], 11),
    (&["eleven"], 11),
    (&["douze"], 12),
    (&["twelve"], 12),
    (&["treize"], 13),
    (&["thirteen"], 13),
    (&["quatorze"], 14),
    (&["fourteen"], 14),
    (&["quinze"], 15),
    (&["fifteen"], 15),
    (&["seize"], 16),
    (&["sixteen"], 16),
    (&["dix", "sept"], 17),
    (&["seventeen"], 17),
    (&["dix", "huit"], 18),
    (&["eighteen"], 18),
    (&["dix", "neuf"], 19),
    (&["nineteen"], 19),
    (&["cent"], 100),
    (&["hundred"], 100),
];

/// Tens, which may be followed by a unit (and by a teen after soixante / quatre-vingt).
const TENS: &[(&[&str], u32)] = &[
    (&["vingt"], 20),
    (&["twenty"], 20),
    (&["trente"], 30),
    (&["thirty"], 30),
    (&["quarante"], 40),
    (&["forty"], 40),
    (&["cinquante"], 50),
    (&["fifty"], 50),
    (&["soixante"], 60),
    (&["sixty"], 60),
    (&["seventy"], 70),
    (&["quatre", "vingt"], 80),
    (&["quatre", "vingts"], 80),
    (&["eighty"], 80),
    (&["ninety"], 90),
];

/// The longest table entry that starts `tokens`: its value and token count.
fn longest(table: &[(&[&str], u32)], tokens: &[String]) -> Option<(u32, usize)> {
    table
        .iter()
        .filter(|(words, _)| {
            tokens.len() >= words.len() && words.iter().zip(tokens).all(|(w, t)| w == t)
        })
        .map(|(words, value)| (*value, words.len()))
        .max_by_key(|&(_, len)| len)
}

/// A spoken number (0..=100) at the start of `tokens`: its value and token count.
fn spoken_number(tokens: &[String]) -> Option<(u32, usize)> {
    if let Some((base, len)) = longest(TENS, tokens) {
        let mut rest = &tokens[len..];
        let mut used = len;
        // "vingt et un", "soixante et onze": `et` only joins un / une / onze.
        if rest.first().is_some_and(|t| t == "et")
            && rest
                .get(1)
                .is_some_and(|t| matches!(t.as_str(), "un" | "une" | "onze"))
        {
            rest = &rest[1..];
            used += 1;
        }
        let teens_ok = matches!(base, 60 | 80);
        if let Some((unit, ulen)) = longest(ATOMS, rest)
            && ((1..=9).contains(&unit) || (teens_ok && (10..=19).contains(&unit)))
        {
            return Some((base + unit, used + ulen));
        }
        return Some((base, len));
    }
    let (value, len) = longest(ATOMS, tokens)?;
    // The article "un" / "une" / "one" alone is not a number.
    if value == 1 { None } else { Some((value, len)) }
}

/// Splits on anything but letters and digits, keeping `21,5` / `21.5` whole.
fn number_tokens(heard: &str) -> Vec<String> {
    let chars: Vec<char> = heard.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let decimal_point = matches!(c, ',' | '.')
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_alphanumeric() || decimal_point {
            current.push(c);
        } else if !current.is_empty() {
            tokens.push(fold_token(std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        tokens.push(fold_token(current));
    }
    tokens
}

/// Folds a word token; a token with digits (maybe a decimal point) is kept as is.
fn fold_token(token: String) -> String {
    if token.starts_with(|c: char| c.is_ascii_digit()) {
        token
    } else {
        normalize(&token)
    }
}

/// The first number in what was heard: digits (`21`, `21,5`) or French / English words up to 100.
///
/// A number right after `à` / `to` / `at` / `sur` wins ("oui dans dix minutes à 21" is 21);
/// otherwise the first number.
pub fn first_number(heard: &str) -> Option<f64> {
    let tokens = number_tokens(heard);
    let at = |i: usize| -> Option<f64> {
        let token = tokens.get(i)?;
        if token.starts_with(|c: char| c.is_ascii_digit()) {
            token.replace(',', ".").parse::<f64>().ok()
        } else {
            spoken_number(&tokens[i..]).map(|(value, _)| f64::from(value))
        }
    };
    let marked = (0..tokens.len())
        .filter(|&i| matches!(tokens[i].as_str(), "a" | "to" | "at" | "sur"))
        .find_map(|i| at(i + 1));
    marked.or_else(|| (0..tokens.len()).find_map(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(labels: &[&str]) -> Vec<AskAnswer> {
        labels
            .iter()
            .map(|l| AskAnswer {
                label: (*l).into(),
                words: vec![],
            })
            .collect()
    }

    #[test]
    fn oui_phrasings_match_oui() {
        let x = a(&["Oui", "Non"]);
        for h in ["oui oui vas-y", "Ouais", "d'accord", "OK"] {
            assert_eq!(match_answer(&x, h), Some(0), "{h}");
        }
    }
    #[test]
    fn non_never_matches_oui() {
        assert_eq!(match_answer(&a(&["Oui", "Non"]), "non"), Some(1));
        assert_eq!(match_answer(&a(&["Oui"]), "non"), None);
    }
    #[test]
    fn longest_phrase_wins() {
        assert_eq!(
            match_answer(&a(&["Oui", "Non"]), "oui mais non merci"),
            Some(1)
        );
    }
    #[test]
    fn custom_label_and_words() {
        let x = vec![
            AskAnswer {
                label: "Oui".into(),
                words: vec![],
            },
            AskAnswer {
                label: "Seulement en haut".into(),
                words: vec!["l'étage".into()],
            },
        ];
        assert_eq!(match_answer(&x, "seulement en-haut"), Some(1));
        assert_eq!(match_answer(&x, "juste l etage"), Some(1));
    }
    #[test]
    fn tie_goes_to_first() {
        assert_eq!(match_answer(&a(&["Oui", "Yes"]), "oui yes"), Some(0));
    }
    #[test]
    fn numbers() {
        assert_eq!(first_number("oui à 21"), Some(21.0));
        assert_eq!(first_number("21,5"), Some(21.5));
        assert_eq!(first_number("vingt-et-un"), Some(21.0));
        assert_eq!(first_number("vingt et un degrés"), Some(21.0));
        assert_eq!(first_number("twenty two"), Some(22.0));
        assert_eq!(first_number("quatre-vingt-dix"), Some(90.0));
        assert_eq!(first_number("oui"), None);
    }
    #[test]
    fn negated_yes_is_no() {
        for (labels, h) in [
            (["Oui", "Non"], "pas d'accord"),
            (["Oui", "Non"], "pas ok"),
            (["Oui", "Non"], "surtout pas"),
            (["Yes", "No"], "not okay"),
            (["Yes", "No"], "no go"),
        ] {
            assert_eq!(match_answer(&a(&labels), h), Some(1), "{h}");
        }
    }
    #[test]
    fn number_after_a_marker_wins() {
        assert_eq!(first_number("oui dans dix minutes à 21"), Some(21.0));
        assert_eq!(first_number("oui 21"), Some(21.0));
        assert_eq!(first_number("oui à vingt et un"), Some(21.0));
        assert_eq!(first_number("yes in ten minutes to 19"), Some(19.0));
    }
    #[test]
    fn article_un_is_not_a_number() {
        assert_eq!(first_number("oui, un peu"), None);
        assert_eq!(first_number("one more"), None);
    }
    #[test]
    fn more_numbers_and_messages() {
        assert_eq!(first_number("soixante-dix-sept"), Some(77.0));
        assert_eq!(first_number("quatre vingt un"), Some(81.0));
        assert_eq!(first_number("cent"), Some(100.0));
        assert_eq!(first_number("21.5"), Some(21.5));
        assert_eq!(normalize("  L’Étage-là, OK! "), "l etage la ok");
        assert_eq!(not_understood(Lang::Fr), "Je n'ai pas compris.");
        assert_eq!(not_understood(Lang::En), "I didn't understand.");
    }
}
