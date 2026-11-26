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
mod tests;
