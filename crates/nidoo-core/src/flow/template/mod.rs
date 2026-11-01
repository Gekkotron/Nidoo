//! Notify templates: `{…}` placeholders in a notify card's title and body, filled with entity
//! values when the step runs. Pure: values come through a caller-supplied lookup, so the hub and
//! the web preview render the same text.

mod parse;
mod render;
mod words;

use serde::{Deserialize, Serialize};

pub use parse::{parse, placeholder_for};
pub use render::{Reading, render};
pub use words::{format_number, on_off_word, text_word, with_unit};

/// The language text values are written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Fr,
}

/// Why a template does not parse.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{reason}")]
pub struct TemplateError {
    pub reason: String,
}

impl TemplateError {
    fn new(reason: impl Into<String>) -> Self {
        TemplateError {
            reason: reason.into(),
        }
    }
}

/// A parsed template: literal text and placeholders.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Text(String),
    Value { expr: Expr, decimals: Option<u8> },
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Number(f64),
    Entity(String),
    Trigger,
    Bin(Box<Expr>, BinOp, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl Template {
    /// The distinct entity ids the placeholders read, in first-seen order (`trigger` excluded).
    pub fn entities(&self) -> Vec<&str> {
        let mut ids = Vec::new();
        for part in &self.parts {
            if let Part::Value { expr, .. } = part {
                expr.entities(&mut ids);
            }
        }
        ids
    }

    /// Whether the template has no placeholder.
    pub fn is_plain(&self) -> bool {
        self.parts.iter().all(|part| matches!(part, Part::Text(_)))
    }
}

impl Expr {
    fn entities<'a>(&'a self, ids: &mut Vec<&'a str>) {
        match self {
            Expr::Entity(id) if !ids.contains(&id.as_str()) => ids.push(id),
            Expr::Bin(left, _, right) => {
                left.entities(ids);
                right.entities(ids);
            }
            _ => {}
        }
    }
}
