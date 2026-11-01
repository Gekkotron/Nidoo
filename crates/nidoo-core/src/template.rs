//! Home Assistant style Jinja templates (`value_template`, `command_template`, …)
//! evaluated with minijinja.
//!
//! Every render sees `value` (the raw payload string) and `value_json` (the payload parsed
//! as JSON when it is an object or an array; otherwise the name stays undefined). Undefined
//! values are strict: printing or testing one is an error, so a template whose payload lacks
//! the key it reads fails instead of rendering an empty string.

use minijinja::value::{Kwargs, Rest, Serde, ValueKind};
use minijinja::{Environment, Error as JinjaError, ErrorKind, UndefinedBehavior, Value};
use serde_json::Map;

/// Why a template could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// The template does not parse, or names a filter, test or function that does not
    /// exist: a permanent authoring error, independent of the payload.
    #[error("template syntax error: {0}")]
    Syntax(String),
    /// The template read an undefined variable or attribute, or converted a value that
    /// has no numeric meaning without a default.
    #[error("undefined value: {0}")]
    Undefined(String),
    /// Any other evaluation failure (invalid operation, bad argument, …).
    #[error("template error: {0}")]
    Other(String),
}

impl From<JinjaError> for TemplateError {
    fn from(err: JinjaError) -> Self {
        let msg = err.to_string();
        match err.kind() {
            ErrorKind::UndefinedError => Self::Undefined(msg),
            ErrorKind::SyntaxError
            | ErrorKind::BadInclude
            | ErrorKind::UnknownFilter
            | ErrorKind::UnknownTest
            | ErrorKind::UnknownFunction => Self::Syntax(msg),
            _ => Self::Other(msg),
        }
    }
}

/// The template engine: one shared minijinja environment with the Home Assistant filters.
///
/// Cheap to share (`Send + Sync`); build it once and render per message.
#[derive(Debug)]
pub struct Templates {
    env: Environment<'static>,
}

impl Default for Templates {
    fn default() -> Self {
        Self::new()
    }
}

impl Templates {
    /// Builds the environment with strict undefined handling and the HA filters.
    pub fn new() -> Self {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.add_filter("float", filters::float);
        env.add_filter("int", filters::int);
        env.add_filter("is_defined", filters::is_defined);
        env.add_filter("is_number", filters::is_number);
        env.add_filter("iif", filters::iif);
        env.add_filter("truncate", filters::truncate);
        env.add_test("is_number", filters::is_number);
        // Z2M templates call Python's `dict.get(key, default)`.
        env.set_unknown_method_callback(|_, value, method, args| match (method, args) {
            ("get", [key]) | ("get", [key, _]) if value.kind() == ValueKind::Map => {
                let found = value.get_item(key).unwrap_or(Value::UNDEFINED);
                if found.is_undefined() {
                    Ok(args.get(1).cloned().unwrap_or(Value::from(())))
                } else {
                    Ok(found)
                }
            }
            _ => Err(JinjaError::from(ErrorKind::UnknownMethod)),
        });
        Self { env }
    }

    /// Renders `template` against `payload` with the standard `value` / `value_json` context.
    pub fn render(&self, template: &str, payload: &str) -> Result<String, TemplateError> {
        self.render_with(template, payload, &[])
    }

    /// Like [`render`](Self::render) with extra context variables; an extra named `value` or
    /// `value_json` replaces the payload-derived one.
    pub fn render_with(
        &self,
        template: &str,
        payload: &str,
        extra: &[(&str, serde_json::Value)],
    ) -> Result<String, TemplateError> {
        let mut ctx = Map::new();
        ctx.insert("value".to_owned(), serde_json::Value::from(payload));
        if let Some(json) = parse_payload(payload) {
            ctx.insert("value_json".to_owned(), json);
        }
        for (name, value) in extra {
            ctx.insert((*name).to_owned(), value.clone());
        }
        Ok(self.env.render_str(template, Serde(&ctx))?)
    }

    /// Compiles `template` without rendering it; `Err(Syntax)` when it does not parse.
    pub fn check(&self, template: &str) -> Result<(), TemplateError> {
        self.env.template_from_str(template)?;
        Ok(())
    }
}

/// The payload as JSON when it is an object or an array; scalars are not exposed as
/// `value_json` (Home Assistant only sets it for those two).
fn parse_payload(payload: &str) -> Option<serde_json::Value> {
    let trimmed = payload.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    serde_json::from_str(payload)
        .ok()
        .filter(|v: &serde_json::Value| v.is_object() || v.is_array())
}

/// Home Assistant filters layered over minijinja's built-ins.
mod filters {
    use super::*;

    fn undefined(msg: String) -> JinjaError {
        JinjaError::new(ErrorKind::UndefinedError, msg)
    }

    /// `default` from either the first positional argument or the `default=` keyword.
    fn default_arg(
        positional: Option<Value>,
        kwargs: &Kwargs,
    ) -> Result<Option<Value>, JinjaError> {
        let default = match positional {
            Some(v) => Some(v),
            None => kwargs.get::<Option<Value>>("default")?,
        };
        kwargs.assert_all_used()?;
        Ok(default)
    }

    /// The value as `f64` when it is a number, a bool or a numeric string.
    fn to_f64(value: &Value) -> Option<f64> {
        match value.kind() {
            ValueKind::Undefined | ValueKind::None => None,
            ValueKind::Bool => Some(if value.is_true() { 1.0 } else { 0.0 }),
            ValueKind::String => value.as_str()?.trim().parse::<f64>().ok(),
            _ => f64::try_from(value.clone()).ok(),
        }
    }

    /// HA `float`: numeric strings and numbers convert; anything else yields the default
    /// or an undefined-class error.
    pub fn float(
        value: &Value,
        default: Option<Value>,
        kwargs: Kwargs,
    ) -> Result<Value, JinjaError> {
        let default = default_arg(default, &kwargs)?;
        match to_f64(value).filter(|f| f.is_finite()) {
            Some(f) => Ok(Value::from(f)),
            None => default.ok_or_else(|| undefined(format!("float got invalid input {value:?}"))),
        }
    }

    /// HA `int`: like [`float`] but truncates toward zero.
    pub fn int(value: &Value, default: Option<Value>, kwargs: Kwargs) -> Result<Value, JinjaError> {
        let default = default_arg(default, &kwargs)?;
        match to_f64(value).filter(|f| f.is_finite()) {
            Some(f) => Ok(Value::from(f.trunc() as i64)),
            None => default.ok_or_else(|| undefined(format!("int got invalid input {value:?}"))),
        }
    }

    /// HA `is_defined`: passes the value through, fails on undefined.
    pub fn is_defined(value: Value) -> Result<Value, JinjaError> {
        if value.is_undefined() {
            Err(undefined("is_defined: value is undefined".to_owned()))
        } else {
            Ok(value)
        }
    }

    /// HA `is_number`: true for ints, floats and bools that are finite numbers; numeric
    /// strings count too, as in Home Assistant.
    pub fn is_number(value: &Value) -> bool {
        to_f64(value).is_some_and(f64::is_finite)
    }

    /// HA `iif(if_true, if_false, if_none=None)`: `if_none` applies only to `none` and
    /// only when given; otherwise truthiness decides. Undefined input is an error (Strict
    /// mode does not intercept filter arguments, and a missing key must not become `off`).
    pub fn iif(
        value: &Value,
        if_true: Value,
        if_false: Option<Value>,
        if_none: Option<Value>,
        kwargs: Kwargs,
    ) -> Result<Value, JinjaError> {
        let if_none = match if_none {
            Some(v) => Some(v),
            None => kwargs.get::<Option<Value>>("if_none")?,
        };
        kwargs.assert_all_used()?;
        if value.is_undefined() {
            return Err(undefined("iif: value is undefined".to_owned()));
        }
        if let (true, Some(v)) = (value.is_none(), if_none) {
            return Ok(v);
        }
        if value.is_true() {
            Ok(if_true)
        } else {
            Ok(if_false.unwrap_or_else(|| Value::from(false)))
        }
    }

    /// Jinja2 `truncate(length=255, killwords=false, end="...", leeway=5)`: text longer than
    /// `length + leeway` characters is cut to `length` including `end`, at the last space
    /// unless `killwords`. Zigbee2MQTT text sensors use `truncate(254, True, '', 0)`.
    pub fn truncate(value: &str, args: Rest<Value>, kwargs: Kwargs) -> Result<String, JinjaError> {
        if args.len() > 4 {
            return Err(JinjaError::new(
                ErrorKind::TooManyArguments,
                "truncate takes at most 4 positional arguments",
            ));
        }
        let length = match args.first() {
            Some(v) => usize::try_from(v.clone())?,
            None => kwargs.get::<Option<usize>>("length")?.unwrap_or(255),
        };
        let killwords = match args.get(1) {
            Some(v) => v.is_true(),
            None => kwargs.get::<Option<bool>>("killwords")?.unwrap_or(false),
        };
        let end = match args.get(2) {
            Some(v) => v.as_str().map_or_else(|| v.to_string(), str::to_owned),
            None => kwargs
                .get::<Option<String>>("end")?
                .unwrap_or_else(|| "...".to_owned()),
        };
        let leeway = match args.get(3) {
            Some(v) => usize::try_from(v.clone())?,
            None => kwargs.get::<Option<usize>>("leeway")?.unwrap_or(5),
        };
        kwargs.assert_all_used()?;
        let end_len = end.chars().count();
        if length < end_len {
            return Err(JinjaError::new(
                ErrorKind::InvalidOperation,
                "truncate: expected length >= the length of end",
            ));
        }
        let chars: Vec<char> = value.chars().collect();
        if chars.len() <= length.saturating_add(leeway) {
            return Ok(value.to_owned());
        }
        let keep: String = chars[..length - end_len].iter().collect();
        let kept = if killwords {
            keep
        } else {
            match keep.rfind(' ') {
                Some(space) => keep[..space].to_owned(),
                None => keep,
            }
        };
        Ok(kept + &end)
    }
}
