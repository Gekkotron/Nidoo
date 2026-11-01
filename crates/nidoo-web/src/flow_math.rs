//! Pure helpers of the math card's settings drawer: the live preview of the formula and the
//! name proposed for a new input.

use std::cell::RefCell;

use nidoo_core::flow::{
    FUNCTIONS, Flow, FlowError, Function, MathParams, Scalar, check_math_card, is_valid_name,
    parse_formula,
};

use crate::flow_edit::fold_accents;

/// What the drawer shows under the formula.
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    /// The formula's value with the readings as they are now. `averaged_now` is true when an
    /// input averages over a window: the preview reads its current value, not the average.
    Value { value: f64, averaged_now: bool },
    /// The preview cannot be computed here: the name of the reading it lacks (an input with no
    /// value, another card's result, `trigger`).
    Unknown(String),
    /// Why the card would be refused on save.
    Error(String),
    /// Why the formula fails with the readings as they are now (`heater / grid` while `grid` is
    /// 0). The hub accepts the card, so this shows without blocking Save.
    Fails(String),
}

/// The preview of math card `node_id` (with `params`, a draft of it) in `flow`. Errors come from
/// [`check_math_card`], the rules the hub applies on save, so the editor and the hub agree.
pub fn preview(
    node_id: &str,
    params: &MathParams,
    flow: &Flow,
    value: &dyn Fn(&str) -> Option<Scalar>,
) -> Preview {
    if let Err(e) = check_math_card(node_id, params, flow) {
        return Preview::Error(match e {
            FlowError::BadMath { reason, .. } => reason,
            other => other.to_string(),
        });
    }
    let formula = match parse_formula(&params.formula) {
        Ok(f) => f,
        Err(e) => return Preview::Error(e.reason),
    };
    let lacking = RefCell::new(None::<String>);
    let read =
        |name: &str| -> Result<f64, String> {
            let number = params.inputs.iter().find(|i| i.name == name).and_then(|i| {
                match value(&i.entity) {
                    Some(Scalar::Number(n)) => Some(n),
                    Some(Scalar::Bool(b)) => Some(f64::from(u8::from(b))),
                    _ => None,
                }
            });
            number.ok_or_else(|| {
                // The reason is dropped: a lacking reading shows as `Unknown(name)`.
                lacking.borrow_mut().get_or_insert_with(|| name.to_owned());
                name.to_owned()
            })
        };
    let result = formula.eval(&read);
    if let Some(name) = lacking.into_inner() {
        return Preview::Unknown(name);
    }
    match result {
        Ok(value) => Preview::Value {
            value,
            averaged_now: params.inputs.iter().any(|i| i.average.is_some()),
        },
        Err(reason) => Preview::Fails(reason),
    }
}

/// Whether `name` is one a new input row proposes before an entity is chosen (`input`,
/// `input2`…), so choosing the entity may replace it with a name drawn from the entity.
pub fn is_placeholder_name(name: &str) -> bool {
    name.strip_prefix("input")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// The name proposed for an input reading an entity called `label`: lowercase, accents folded,
/// other characters as `_`, at most 32 characters, starting with a letter (`v_` otherwise), and
/// numbered `2`, `3`… until it is valid and not in `taken`.
pub fn input_name_for(label: &str, taken: &[&str]) -> String {
    let mut slug = String::new();
    for c in fold_accents(label).chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_');
    let base = if slug.starts_with(|c: char| c.is_ascii_lowercase()) {
        slug.to_owned()
    } else {
        format!("v_{slug}")
    };
    let base: String = base.chars().take(32).collect();
    let free = |name: &str| is_valid_name(name) && !taken.contains(&name);
    if free(&base) {
        return base;
    }
    (2..)
        .map(|n| {
            let suffix = n.to_string();
            let keep: String = base.chars().take(32 - suffix.len()).collect();
            format!("{keep}{suffix}")
        })
        .find(|name| free(name))
        .expect("an unbounded range always yields a free name")
}

/// The slot a function snippet leaves for the user to fill; the formula is refused until none
/// is left.
pub const SLOT: char = '▢';

/// The text inserted for function `f`: its name and one slot per required argument (at least
/// two for `min` and `max`), `interp` with its two lists.
pub fn function_snippet(f: &Function) -> String {
    let slots = |n: usize| vec![SLOT.to_string(); n].join(", ");
    match f.name {
        "min" | "max" => format!("{}({})", f.name, slots(2)),
        "interp" => format!("{}({SLOT}, [{SLOT}], [{SLOT}])", f.name),
        _ => format!("{}({})", f.name, slots(f.min_args)),
    }
}

/// `text` with `snippet` inserted at char offset `cursor`, and the selection (char offsets) over
/// the snippet's first slot, or a caret after the snippet when it has none.
pub fn insert_at(text: &str, cursor: usize, snippet: &str) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut out: String = chars[..cursor].iter().collect();
    out.push_str(snippet);
    out.extend(&chars[cursor..]);
    match snippet.chars().position(|c| c == SLOT) {
        Some(i) => (out, cursor + i, cursor + i + 1),
        None => {
            let end = cursor + snippet.chars().count();
            (out, end, end)
        }
    }
}

/// One proposal while typing: what the list shows, what choosing it inserts, and whether it is
/// a function.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub is_function: bool,
}

/// The `[a-z0-9_]` word ending at char offset `cursor`, with its start offset.
fn word_before(text: &str, cursor: usize) -> (usize, String) {
    let chars: Vec<char> = text.chars().collect();
    let end = cursor.min(chars.len());
    let mut start = end;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    (start, chars[start..end].iter().collect())
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
}

/// The functions and `names` that start with the word before `cursor` (at least two characters):
/// functions first, then names, each alphabetical.
pub fn suggestions(text: &str, cursor: usize, names: &[&str]) -> Vec<Suggestion> {
    let (_, word) = word_before(text, cursor);
    if word.chars().count() < 2 {
        return Vec::new();
    }
    let mut functions: Vec<Suggestion> = FUNCTIONS
        .iter()
        .filter(|f| f.name.starts_with(&word))
        .map(|f| Suggestion {
            label: f.name.to_owned(),
            insert: function_snippet(f),
            is_function: true,
        })
        .collect();
    functions.sort_by(|a, b| a.label.cmp(&b.label));
    let mut found: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.starts_with(&word))
        .collect();
    found.sort_unstable();
    found.dedup();
    functions.extend(found.into_iter().map(|n| Suggestion {
        label: n.to_owned(),
        insert: n.to_owned(),
        is_function: false,
    }));
    functions
}

/// `text` with the word before `cursor` replaced by suggestion `s`, and the selection as
/// [`insert_at`] gives it.
pub fn accept(text: &str, cursor: usize, s: &Suggestion) -> (String, usize, usize) {
    let (start, word) = word_before(text, cursor);
    let chars: Vec<char> = text.chars().collect();
    let end = start + word.chars().count();
    let mut kept: String = chars[..start].iter().collect();
    kept.extend(&chars[end..]);
    insert_at(&kept, start, &s.insert)
}

/// The char offset of UTF-16 offset `units` in `text` (the DOM's selection unit); past the end
/// gives the length.
pub fn char_offset(text: &str, units: u32) -> usize {
    let mut seen = 0usize;
    for (i, c) in text.chars().enumerate() {
        if seen >= units as usize {
            return i;
        }
        seen += c.len_utf16();
    }
    text.chars().count()
}

/// The UTF-16 offset of char offset `chars` in `text`.
pub fn utf16_offset(text: &str, chars: usize) -> u32 {
    text.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use nidoo_core::flow::{Flow, MathInput, MathParams, Node, NodeKind, Scalar};

    use super::*;
    use crate::i18n::Lang;
    use crate::i18n::flows::function_help;

    fn input(name: &str, entity: &str, average: Option<&str>) -> MathInput {
        MathInput {
            name: name.into(),
            entity: entity.into(),
            average: average.map(str::to_owned),
            extra: Default::default(),
        }
    }

    fn params(formula: &str, inputs: Vec<MathInput>) -> MathParams {
        MathParams {
            inputs,
            formula: formula.into(),
            name: "desired".into(),
            save_to: None,
            extra: Default::default(),
        }
    }

    /// A flow holding `params` as card `m`, and optionally a second card named `other`.
    fn flow_of(params: &MathParams, other: Option<&str>) -> Flow {
        let node = |id: &str, p: MathParams| Node {
            id: id.into(),
            name: None,
            position: None,
            kind: NodeKind::Math(p),
            extra: Default::default(),
        };
        let mut nodes = vec![node("m", params.clone())];
        if let Some(name) = other {
            nodes.push(node(
                "n",
                MathParams {
                    name: name.into(),
                    formula: "1".into(),
                    ..Default::default()
                },
            ));
        }
        Flow {
            id: "f".into(),
            name: "F".into(),
            nodes,
            ..Flow::default()
        }
    }

    fn values(name: &str) -> Option<Scalar> {
        match name {
            "sensor.grid" => Some(Scalar::Number(1330.0)),
            "sensor.heater" => Some(Scalar::Number(5.0)),
            _ => None,
        }
    }

    fn run(p: &MathParams, other: Option<&str>) -> Preview {
        preview("m", p, &flow_of(p, other), &values)
    }

    #[test]
    fn preview_computes_with_current_values() {
        let p = params(
            "clamp(heater - 100 - grid, 0, 1650)",
            vec![
                input("grid", "sensor.grid", None),
                input("heater", "sensor.heater", None),
            ],
        );
        assert_eq!(
            run(&p, None),
            Preview::Value {
                value: 0.0,
                averaged_now: false
            }
        );
    }

    #[test]
    fn preview_reports_errors_and_unknowns() {
        let grid = || input("grid", "sensor.grid", None);
        let p = params("gird + 1", vec![grid()]);
        assert_eq!(run(&p, None), Preview::Error("unknown name `gird`".into()));

        let p = params("desired2 + 1", vec![grid()]);
        assert_eq!(
            run(&p, Some("desired2")),
            Preview::Unknown("desired2".into())
        );

        let p = params("grid + 1", vec![input("grid", "sensor.missing", None)]);
        assert_eq!(run(&p, None), Preview::Unknown("grid".into()));

        // A run-time failure with today's readings is shown, not refused.
        let p = params("1 / 0", vec![]);
        assert!(matches!(run(&p, None), Preview::Fails(_)));
    }

    #[test]
    fn preview_marks_averages() {
        let p = params("grid", vec![input("grid", "sensor.grid", Some("1m"))]);
        assert_eq!(
            run(&p, None),
            Preview::Value {
                value: 1330.0,
                averaged_now: true
            }
        );
    }

    #[test]
    fn input_names() {
        assert_eq!(input_name_for("Réseau", &[]), "reseau");
        assert_eq!(input_name_for("Chauffe eau", &[]), "chauffe_eau");
        assert_eq!(input_name_for("2nd floor", &[]), "v_2nd_floor");
        assert_eq!(input_name_for("Réseau", &["reseau"]), "reseau2");
        assert_eq!(input_name_for("Min", &[]), "min2");
        assert_eq!(input_name_for("Réseau", &["reseau", "reseau2"]), "reseau3");
        assert_eq!(input_name_for("  !! ", &[]), "v_");
        let long = input_name_for(&"a".repeat(50), &[]);
        assert_eq!(long.len(), 32);
        assert!(is_placeholder_name("input") && is_placeholder_name("input12"));
        assert!(!is_placeholder_name("inputs") && !is_placeholder_name("grid"));
    }

    #[test]
    fn snippets() {
        let snippet =
            |name: &str| function_snippet(FUNCTIONS.iter().find(|f| f.name == name).unwrap());
        assert_eq!(snippet("clamp"), "clamp(▢, ▢, ▢)");
        assert_eq!(snippet("round"), "round(▢)");
        assert_eq!(snippet("min"), "min(▢, ▢)");
        assert_eq!(snippet("interp"), "interp(▢, [▢], [▢])");
        assert_eq!(snippet("if"), "if(▢, ▢, ▢)");
    }

    #[test]
    fn insert_selects_first_slot() {
        assert_eq!(
            insert_at("a + ", 4, "clamp(▢, ▢, ▢)"),
            ("a + clamp(▢, ▢, ▢)".to_owned(), 10, 11)
        );
        // Char offsets, not bytes: the accent and the slot are one char each.
        assert_eq!(insert_at("é▢", 1, "x"), ("éx▢".to_owned(), 2, 2));
    }

    #[test]
    fn insert_without_slot() {
        assert_eq!(insert_at("", 0, "grid"), ("grid".to_owned(), 4, 4));
    }

    #[test]
    fn suggest_after_two_letters() {
        let labels = |text: &str, names: &[&str]| -> Vec<String> {
            suggestions(text, text.chars().count(), names)
                .into_iter()
                .map(|s| s.label)
                .collect()
        };
        assert_eq!(labels("heater - cl", &["heater"]), ["clamp"]);
        assert_eq!(labels("he", &["heater"]), ["heater"]);
        assert!(labels("h", &["heater"]).is_empty());
        assert_eq!(labels("ma", &["max_w"]), ["max", "max_w"]);
        assert!(labels("a + ", &["heater"]).is_empty());
    }

    #[test]
    fn accept_replaces_word() {
        let s = suggestions("heater - cl", 11, &["heater"]).remove(0);
        assert_eq!(
            accept("heater - cl", 11, &s),
            ("heater - clamp(▢, ▢, ▢)".to_owned(), 15, 16)
        );
        // Text after the caret stays.
        let s = suggestions("cl + 1", 2, &[]).remove(0);
        assert_eq!(accept("cl + 1", 2, &s).0, "clamp(▢, ▢, ▢) + 1");
    }

    #[test]
    fn help_exists_for_every_function() {
        for f in FUNCTIONS.iter() {
            for lang in [Lang::En, Lang::Fr] {
                assert!(
                    !function_help(lang, f.name).is_empty(),
                    "{} {lang:?}",
                    f.name
                );
            }
        }
    }

    #[test]
    fn offsets_convert_between_chars_and_utf16() {
        assert_eq!(char_offset("é▢🌧x", 2), 2);
        assert_eq!(char_offset("é▢🌧x", 4), 3);
        assert_eq!(utf16_offset("é▢🌧x", 3), 4);
        assert_eq!(utf16_offset("ab", 5), 2);
    }
}
