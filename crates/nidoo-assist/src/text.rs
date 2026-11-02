//! Normalising a sentence: folded lowercase words and numbers.

use nidoo_api::search::fold;

/// One word of a normalised sentence.
#[derive(Clone, Debug, PartialEq)]
pub enum Word {
    /// Lowercase, accents folded to ASCII. Also `?`, `%` and `°`, which carry meaning.
    Text(String),
    /// From digits (`19`, `19,5`, `+1`, `-0,5`) or French number words (`dix-neuf`).
    Number(f64),
}

impl Word {
    pub fn text(&self) -> Option<&str> {
        match self {
            Word::Text(t) => Some(t),
            Word::Number(_) => None,
        }
    }

    pub fn is(&self, text: &str) -> bool {
        self.text() == Some(text)
    }
}

/// Splits `sentence` into folded words and numbers. Apostrophe elisions split (`d'un` is `d`
/// then 1), hyphens split (`fait-il`), punctuation is dropped except `?`, `%` and `°`.
pub fn words(sentence: &str) -> Vec<Word> {
    merge_numbers(tokens(&fold(sentence)))
}

fn tokens(folded: &str) -> Vec<Word> {
    let chars: Vec<char> = folded.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts_word = i == 0 || chars[i - 1].is_whitespace();
        let signed = (c == '+' || c == '-')
            && starts_word
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() || signed {
            let mut text = String::new();
            if signed {
                text.push(c);
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                text.push(chars[i]);
                i += 1;
            }
            if i + 1 < chars.len()
                && (chars[i] == ',' || chars[i] == '.')
                && chars[i + 1].is_ascii_digit()
            {
                text.push('.');
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    text.push(chars[i]);
                    i += 1;
                }
            }
            if let Ok(n) = text.parse::<f64>() {
                out.push(Word::Number(n));
            }
        } else if c.is_alphabetic() {
            let mut text = String::new();
            while i < chars.len() && chars[i].is_alphanumeric() {
                text.push(chars[i]);
                i += 1;
            }
            out.push(Word::Text(text));
        } else {
            match c {
                '?' | '%' => out.push(Word::Text(c.to_string())),
                '°' => {
                    out.push(Word::Text("°".into()));
                    // "°C" is one sign.
                    if chars.get(i + 1) == Some(&'c')
                        && !chars.get(i + 2).is_some_and(|n| n.is_alphanumeric())
                    {
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    out
}

/// The value of a single French number word.
fn unit(word: &str) -> Option<f64> {
    let n = match word {
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        "treize" => 13,
        "quatorze" => 14,
        "quinze" => 15,
        "seize" => 16,
        "vingt" => 20,
        "trente" => 30,
        _ => return None,
    };
    Some(f64::from(n))
}

/// Joins number words into one number: `dix neuf` (19), `vingt et un` (21), `vingt cinq` (25).
fn merge_numbers(tokens: Vec<Word>) -> Vec<Word> {
    let text = |i: usize| tokens.get(i).and_then(Word::text);
    let mut out: Vec<Word> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let Some(mut n) = text(i).and_then(unit) else {
            out.push(tokens[i].clone());
            i += 1;
            continue;
        };
        i += 1;
        if n == 10.0 {
            if let Some(u) = text(i).and_then(unit).filter(|u| (7.0..=9.0).contains(u)) {
                n += u;
                i += 1;
            }
        } else if n == 20.0 || n == 30.0 {
            if text(i) == Some("et") && matches!(text(i + 1), Some("un" | "une")) {
                n += 1.0;
                i += 2;
            } else if let Some(u) = text(i).and_then(unit).filter(|u| (2.0..=9.0).contains(u)) {
                n += u;
                i += 1;
            }
        }
        out.push(Word::Number(n));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Word {
        Word::Text(s.into())
    }

    #[test]
    fn folds_accents_and_case() {
        assert_eq!(
            words("Éteins la LUMIÈRE"),
            vec![t("eteins"), t("la"), t("lumiere")]
        );
    }

    #[test]
    fn reads_decimals_with_comma_and_dot() {
        assert_eq!(words("à 19,5"), vec![t("a"), Word::Number(19.5)]);
        assert_eq!(words("à 19.5"), vec![t("a"), Word::Number(19.5)]);
    }

    #[test]
    fn reads_signed_numbers() {
        assert_eq!(words("+1"), vec![Word::Number(1.0)]);
        assert_eq!(words("-0,5"), vec![Word::Number(-0.5)]);
    }

    #[test]
    fn reads_french_number_words() {
        assert_eq!(words("dix-neuf"), vec![Word::Number(19.0)]);
        assert_eq!(words("vingt et un"), vec![Word::Number(21.0)]);
        assert_eq!(words("vingt-cinq"), vec![Word::Number(25.0)]);
        assert_eq!(words("trente"), vec![Word::Number(30.0)]);
        assert_eq!(
            words("d'un degré"),
            vec![t("d"), Word::Number(1.0), t("degre")]
        );
        assert_eq!(words("l'entrée"), vec![t("l"), t("entree")]);
    }

    #[test]
    fn keeps_question_mark() {
        let w = words("il fait combien ?");
        assert_eq!(w.last(), Some(&t("?")));
        let w = words("il fait combien dehors?");
        assert_eq!(
            w,
            vec![t("il"), t("fait"), t("combien"), t("dehors"), t("?")]
        );
    }

    #[test]
    fn drops_other_punctuation() {
        assert_eq!(words("Oui !"), vec![t("oui")]);
        assert_eq!(words("salon, bureau."), vec![t("salon"), t("bureau")]);
    }
}
