//! Filling a parsed template with values: maths, units, rounding and translated words.

use super::words::{format_number, on_off_word, text_word, with_unit};
use super::{BinOp, Expr, Lang, Part, Template, TemplateError, parse};
use crate::flow::Scalar;
use crate::model::StateValue;
use crate::types::type_def;

/// An entity's value as a placeholder reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub value: Scalar,
    pub unit: Option<String>,
    /// Picks the on/off words (`door` → open/closed, `motion` → detected/clear).
    pub device_class: Option<String>,
}

impl Reading {
    /// The headline value of an entity state, `None` when it has none.
    pub fn of(
        component: &str,
        device_class: Option<&str>,
        unit: Option<&str>,
        state: &StateValue,
    ) -> Option<Reading> {
        let value = match state {
            StateValue::Generic(generic) => Scalar::of_generic(type_def(component)?, generic)?,
            other => Scalar::of_state(other)?,
        };
        Some(Reading {
            value,
            unit: unit.map(Into::into),
            device_class: device_class.map(Into::into),
        })
    }
}

/// Parses and renders `text` in one go.
pub fn render(
    text: &str,
    lang: Lang,
    lookup: &dyn Fn(&str) -> Option<Reading>,
    trigger: Option<&Reading>,
) -> Result<String, TemplateError> {
    Ok(parse(text)?.render(lang, lookup, trigger))
}

/// What a placeholder prints when its value is missing.
const MISSING: &str = "?";

impl Template {
    /// The text with every placeholder filled; a value that cannot be had prints `?`.
    pub fn render(
        &self,
        lang: Lang,
        lookup: &dyn Fn(&str) -> Option<Reading>,
        trigger: Option<&Reading>,
    ) -> String {
        let read = |expr: &Expr| match expr {
            Expr::Entity(id) => lookup(id),
            Expr::Trigger => trigger.cloned(),
            _ => None,
        };
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Value {
                    expr: expr @ (Expr::Entity(_) | Expr::Trigger),
                    decimals,
                } => {
                    out.push_str(&match read(expr) {
                        None => MISSING.to_owned(),
                        Some(Reading {
                            value: Scalar::Bool(on),
                            device_class,
                            ..
                        }) => on_off_word(on, device_class.as_deref(), lang).to_owned(),
                        Some(Reading {
                            value: Scalar::Text(text),
                            ..
                        }) => text_word(&text, lang),
                        Some(Reading {
                            value: Scalar::Number(n),
                            unit,
                            ..
                        }) => number_text(Some((n, unit)), *decimals, lang),
                    });
                }
                Part::Value { expr, decimals } => {
                    out.push_str(&number_text(eval(expr, &read), *decimals, lang));
                }
            }
        }
        out
    }

    /// The template as typed, with each entity id replaced by `name(id)` (the id when `None`).
    pub fn label(&self, name: &dyn Fn(&str) -> Option<String>) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(&text.replace('{', "{{").replace('}', "}}")),
                Part::Value { expr, decimals } => {
                    out.push('{');
                    label_expr(expr, name, &mut out);
                    if let Some(n) = decimals {
                        out.push_str(&format!(":{n}"));
                    }
                    out.push('}');
                }
            }
        }
        out
    }
}

fn number_text(value: Option<(f64, Option<String>)>, decimals: Option<u8>, lang: Lang) -> String {
    match value.filter(|(n, _)| n.is_finite()) {
        Some((n, unit)) => with_unit(format_number(n, decimals, lang), unit.as_deref(), lang),
        None => MISSING.to_owned(),
    }
}

/// A number and its unit; `None` for anything that is not a number, or a division by zero.
fn eval(expr: &Expr, read: &dyn Fn(&Expr) -> Option<Reading>) -> Option<(f64, Option<String>)> {
    match expr {
        Expr::Number(n) => Some((*n, None)),
        Expr::Entity(_) | Expr::Trigger => match read(expr)? {
            Reading {
                value: Scalar::Number(n),
                unit,
                ..
            } => Some((n, unit)),
            _ => None,
        },
        Expr::Bin(left, op, right) => {
            let (a, unit_a) = eval(left, read)?;
            let (b, unit_b) = eval(right, read)?;
            let value = match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div if b == 0.0 => return None,
                BinOp::Div => a / b,
            };
            let unit = match (op, unit_a, unit_b) {
                (_, None, unit) | (_, unit, None) => unit,
                (BinOp::Add | BinOp::Sub, Some(a), Some(b)) if a == b => Some(a),
                _ => None,
            };
            value.is_finite().then_some((value, unit))
        }
    }
}

fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Add | BinOp::Sub => 1,
        BinOp::Mul | BinOp::Div => 2,
    }
}

fn label_expr(expr: &Expr, name: &dyn Fn(&str) -> Option<String>, out: &mut String) {
    match expr {
        Expr::Number(n) => out.push_str(&n.to_string()),
        Expr::Trigger => out.push_str("trigger"),
        Expr::Entity(id) => out.push_str(&name(id).unwrap_or_else(|| id.clone())),
        Expr::Bin(left, op, right) => {
            label_operand(left, *op, false, name, out);
            out.push_str(match op {
                BinOp::Add => " + ",
                BinOp::Sub => " - ",
                BinOp::Mul => " * ",
                BinOp::Div => " / ",
            });
            label_operand(right, *op, true, name, out);
        }
    }
}

/// An operand of `op`, in parentheses when it binds looser (or, on the right of a
/// left-associative operator, the same).
fn label_operand(
    expr: &Expr,
    op: BinOp,
    right: bool,
    name: &dyn Fn(&str) -> Option<String>,
    out: &mut String,
) {
    let parens = match expr {
        Expr::Bin(_, inner, _) => {
            precedence(*inner) < precedence(op) || (right && precedence(*inner) == precedence(op))
        }
        _ => false,
    };
    if parens {
        out.push('(');
    }
    label_expr(expr, name, out);
    if parens {
        out.push(')');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::template::parse;
    use crate::model::StateValue;

    fn r(v: Scalar, unit: Option<&str>, dc: Option<&str>) -> Reading {
        Reading {
            value: v,
            unit: unit.map(Into::into),
            device_class: dc.map(Into::into),
        }
    }

    fn house(id: &str) -> Option<Reading> {
        match id {
            "t" => Some(r(Scalar::Number(10.1), Some("°C"), None)),
            "max" => Some(r(Scalar::Number(12.0), Some("°C"), None)),
            "min" => Some(r(Scalar::Number(6.6), Some("°C"), None)),
            "rain" => Some(r(Scalar::Number(0.0), Some("mm"), None)),
            "hum" => Some(r(Scalar::Number(77.0), Some("%"), None)),
            "cond" => Some(r(Scalar::Text("partly_cloudy".into()), None, None)),
            "door" => Some(r(Scalar::Bool(true), None, Some("door"))),
            "mode" => Some(r(Scalar::Text("eco_mode".into()), None, None)),
            _ => None,
        }
    }

    fn en(text: &str) -> String {
        render(text, Lang::En, &house, None).unwrap()
    }

    fn fr(text: &str) -> String {
        render(text, Lang::Fr, &house, None).unwrap()
    }

    #[test]
    fn values_carry_units_and_decimal_marks() {
        assert_eq!(en("Now {t}"), "Now 10.1\u{a0}°C");
        assert_eq!(fr("Now {t}"), "Now 10,1\u{a0}°C");
        assert_eq!(en("{t:0}"), "10\u{a0}°C");
        assert_eq!(en("{hum}"), "77%");
        assert_eq!(fr("{hum}"), "77\u{a0}%");
        assert_eq!(en("{rain}"), "0\u{a0}mm");
    }

    #[test]
    fn maths_rounds_and_keeps_shared_units() {
        assert_eq!(en("{max - min:1}"), "5.4\u{a0}°C");
        assert_eq!(en("{max - min}"), "5.4\u{a0}°C");
        assert_eq!(en("{rain * 2}"), "0\u{a0}mm");
        assert_eq!(en("{t * hum}"), "777.7");
        assert_eq!(en("{(max + min) / 2:1}"), "9.3\u{a0}°C");
        assert_eq!(en("{1 / 3}"), "0.33");
        assert_eq!(en("{2.5:0}"), "3");
    }

    #[test]
    fn misses_print_a_question_mark() {
        assert_eq!(en("{gone}"), "?");
        assert_eq!(en("{t / 0}"), "?");
        assert_eq!(en("{cond + 1}"), "?");
        assert_eq!(en("{trigger}"), "?");
    }

    #[test]
    fn text_and_on_off_are_translated() {
        assert_eq!(en("{cond}"), "Partly cloudy");
        assert_eq!(fr("{cond}"), "Éclaircies");
        assert_eq!(fr("{door}"), "Ouvert");
        assert_eq!(en("{mode}"), "Eco mode");
    }

    #[test]
    fn trigger_uses_the_given_reading() {
        let trig = r(Scalar::Number(3.0), Some("mm"), None);
        assert_eq!(
            render("{trigger}", Lang::Fr, &house, Some(&trig)).unwrap(),
            "3\u{a0}mm"
        );
    }

    #[test]
    fn escaped_braces_render_literally() {
        assert_eq!(en("{{t}} {t:0}"), "{t} 10\u{a0}°C");
    }

    #[test]
    fn label_names_entities() {
        let name = |id: &str| (id == "max").then(|| "Today's maximum".to_owned());
        assert_eq!(
            parse("Spread {max - min:1}").unwrap().label(&name),
            "Spread {Today's maximum - min:1}"
        );
        let same = |_: &str| None;
        for text in [
            "{(a + b) / 2}",
            "{a - (b - c)}",
            "{a - b - c} {{x}}",
            "{trigger * -0.5:2}",
        ] {
            assert_eq!(parse(text).unwrap().label(&same), text);
        }
    }

    #[test]
    fn reading_of_a_number_state() {
        let got = Reading::of(
            "sensor",
            Some("temperature"),
            Some("°C"),
            &StateValue::Number(10.1),
        )
        .unwrap();
        assert_eq!(got.value, Scalar::Number(10.1));
        assert_eq!(got.unit.as_deref(), Some("°C"));
        assert!(Reading::of("sensor", None, None, &StateValue::Unknown).is_none());
    }
}
