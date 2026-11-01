//! The flow math formula: a small arithmetic language with comparisons, `and`/`or`/`not`, a few
//! functions and `interp` lookup tables. Pure and bounded (length, nesting, table size), so the
//! hub, the API and the wasm editor share one parser and one evaluator.
//!
//! Precedence, low to high: `or`, `and`, `not`, comparisons (not chainable), `+ -`, `* / %`,
//! unary `-`, calls and primaries. Booleans are 1 and 0; any non-zero number is true.

mod eval;
mod parse;

pub use parse::parse_formula;

/// The longest formula, in characters.
pub const MAX_FORMULA_CHARS: usize = 500;
/// The deepest nesting of parentheses, calls and unary operators.
pub const MAX_DEPTH: usize = 32;
/// The most distinct names (inputs) one math card reads.
pub const MAX_INPUTS: usize = 10;
/// The most points in an `interp` table.
pub const MAX_TABLE_POINTS: usize = 50;

/// A parsed formula; opaque, built by [`parse_formula`].
#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    expr: Expr,
}

/// Why a formula does not parse; `at` is the byte offset of the offending character.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{reason}")]
pub struct FormulaError {
    pub reason: String,
    pub at: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Func {
    Min,
    Max,
    Clamp,
    Abs,
    Round,
    Floor,
    Ceil,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Interp {
        x: Box<Expr>,
        xs: Vec<f64>,
        ys: Vec<f64>,
    },
}

impl Formula {
    /// The distinct names the formula reads (function names excluded), in first-seen order.
    pub fn names(&self) -> Vec<&str> {
        fn walk<'a>(expr: &'a Expr, out: &mut Vec<&'a str>) {
            match expr {
                Expr::Num(_) => {}
                Expr::Var(name) => {
                    if !out.contains(&name.as_str()) {
                        out.push(name);
                    }
                }
                Expr::Neg(a) | Expr::Not(a) => walk(a, out),
                Expr::Bin(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
                Expr::If(c, a, b) => {
                    walk(c, out);
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Interp { x, .. } => walk(x, out),
            }
        }
        let mut out = Vec::new();
        walk(&self.expr, &mut out);
        out
    }

    /// Evaluates the formula; `read` gives a name's value or why it has none. Fails, never
    /// yields 0, when no finite number comes out.
    pub fn eval(&self, read: &dyn Fn(&str) -> Result<f64, String>) -> Result<f64, String> {
        let value = eval::eval(&self.expr, read)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err("result is not a finite number".into())
        }
    }
}

/// One formula function, for the editor's picker.
pub struct Function {
    pub name: &'static str,
    pub signature: &'static str,
    pub min_args: usize,
    pub max_args: Option<usize>,
}

const fn function(
    name: &'static str,
    signature: &'static str,
    min_args: usize,
    max_args: Option<usize>,
) -> Function {
    Function {
        name,
        signature,
        min_args,
        max_args,
    }
}

/// Every function a formula may call.
pub const FUNCTIONS: [Function; 9] = [
    function("min", "min(a, b, ...)", 1, None),
    function("max", "max(a, b, ...)", 1, None),
    function("clamp", "clamp(x, low, high)", 3, Some(3)),
    function("abs", "abs(x)", 1, Some(1)),
    function("round", "round(x, n)", 1, Some(2)),
    function("floor", "floor(x)", 1, Some(1)),
    function("ceil", "ceil(x)", 1, Some(1)),
    function("if", "if(condition, then, else)", 3, Some(3)),
    function("interp", "interp(x, [xs], [ys])", 3, Some(3)),
];

/// A name a formula can read: `[a-z][a-z0-9_]{0,31}`, not a function or keyword.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let shape = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    shape
        && !FUNCTIONS.iter().any(|f| f.name == name)
        && !matches!(name, "and" | "or" | "not" | "trigger")
}

/// Fits a computed value to an entity: rounds to `step` (counted from `min` when known), clamps
/// into `min..=max`, then rounds to the step's decimals to drop float noise. The flag is true
/// when clamping changed the value.
pub fn fit_to_entity(
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
) -> (f64, bool) {
    let step = step.filter(|s| s.is_finite() && *s > 0.0);
    let mut v = value;
    if let Some(s) = step {
        let base = min.unwrap_or(0.0);
        v = base + ((v - base) / s).round() * s;
    }
    let before = v;
    if let Some(lo) = min {
        v = v.max(lo);
    }
    if let Some(hi) = max {
        v = v.min(hi);
    }
    let clamped = v != before;
    if let Some(s) = step {
        let decimals = decimals_of(s).max(min.map_or(0, decimals_of));
        let scale = 10f64.powi(decimals as i32);
        v = (v * scale).round() / scale;
    }
    (v, clamped)
}

/// How many decimals a number is written with (at most 10).
fn decimals_of(n: f64) -> usize {
    format!("{n}")
        .split_once('.')
        .map_or(0, |(_, d)| d.len())
        .min(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Result<f64, String> {
        Err("no names here".into())
    }

    fn run(text: &str) -> f64 {
        parse_formula(text).unwrap().eval(&none).unwrap()
    }

    fn parse_err(text: &str) -> FormulaError {
        parse_formula(text).unwrap_err()
    }

    fn reason(text: &str) -> String {
        parse_err(text).reason
    }

    #[test]
    fn precedence_and_unary() {
        assert_eq!(run("2 + 3 * 4 - -1"), 15.0);
        assert_eq!(run("(2 + 3) * 4"), 20.0);
        assert_eq!(run("7 % 4"), 3.0);
    }

    #[test]
    fn comparisons_and_logic() {
        assert_eq!(run("3 > 2"), 1.0);
        assert_eq!(run("3 <= 2"), 0.0);
        assert_eq!(run("1 and 0"), 0.0);
        assert_eq!(run("0 or 2"), 1.0);
        assert_eq!(run("not 0"), 1.0);
        assert_eq!(run("2 == 2 and 1 != 2"), 1.0);
    }

    #[test]
    fn functions() {
        assert_eq!(run("min(3, 1, 2)"), 1.0);
        assert_eq!(run("max(4)"), 4.0);
        assert_eq!(run("clamp(-5, 0, 10)"), 0.0);
        assert_eq!(run("abs(-2.5)"), 2.5);
        assert_eq!(run("round(2.5)"), 3.0);
        assert_eq!(run("round(-2.5)"), -3.0);
        assert_eq!(run("round(1.2345, 2)"), 1.23);
        assert_eq!(run("floor(-1.5)"), -2.0);
        assert_eq!(run("ceil(1.1)"), 2.0);
    }

    #[test]
    fn if_is_lazy() {
        let f = parse_formula("if(b != 0, 1 / b, 7)").unwrap();
        assert_eq!(f.eval(&|_| Ok(0.0)), Ok(7.0));
    }

    #[test]
    fn interp_table() {
        let text = "interp(x, [0, 38, 81, 160, 274, 438, 621, 843, 1062, 1270, 1450, 1650], \
                    [0, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60])";
        let f = parse_formula(text).unwrap();
        let at = |x: f64| f.eval(&|_| Ok(x)).unwrap();
        assert!((at(1132.0) - (45.0 + 70.0 / 208.0 * 5.0)).abs() < 1e-9);
        assert_eq!(at(-10.0), 0.0);
        assert_eq!(at(5000.0), 60.0);
        assert_eq!(at(38.0), 10.0);
    }

    #[test]
    fn interp_overflow_is_an_error() {
        // Exponent notation is not in the grammar and a 500-character formula cannot spell two
        // 1e308 literals, so build the tree directly.
        let f = Formula {
            expr: Expr::Interp {
                x: Box::new(Expr::Num(0.5)),
                xs: vec![0.0, 1.0],
                ys: vec![-1e308, 1e308],
            },
        };
        assert_eq!(f.eval(&none), Err("result is not a finite number".into()));
    }

    #[test]
    fn names_in_order() {
        let f = parse_formula("clamp(heater - 100 - grid, 0, max(grid, trigger))").unwrap();
        assert_eq!(f.names(), ["heater", "grid", "trigger"]);
    }

    #[test]
    fn run_time_errors() {
        let eval = |text: &str, read: &dyn Fn(&str) -> Result<f64, String>| {
            parse_formula(text).unwrap().eval(read)
        };
        assert_eq!(eval("1 / 0", &none), Err("division by zero".into()));
        assert_eq!(eval("5 % 0", &none), Err("division by zero".into()));
        assert_eq!(
            eval("x * x", &|_| Ok(1e300)),
            Err("result is not a finite number".into())
        );
        let hi_lo = |name: &str| Ok(if name == "hi" { 5.0 } else { 0.0 });
        assert_eq!(
            eval("clamp(1, hi, lo)", &hi_lo),
            Err("clamp: low is above high".into())
        );
        assert_eq!(
            eval("grid + 1", &|_| Err("grid is unknown".into())),
            Err("grid is unknown".into())
        );
    }

    #[test]
    fn parse_errors() {
        assert_eq!(reason(""), "empty formula");
        assert_eq!(reason("(1 + 2"), "missing )");
        assert_eq!(reason("1 +"), "unexpected end");
        let comma = parse_err("0,5 * grid");
        assert_eq!((comma.reason.as_str(), comma.at), ("unexpected ,", Some(1)));
        let minus = parse_err("2 \u{2212} 1");
        assert_eq!(
            (minus.reason.as_str(), minus.at),
            ("unexpected \u{2212}", Some(2))
        );
        assert_eq!(reason("foo(1)"), "unknown function foo");
        assert_eq!(reason("clamp(1, 2)"), "clamp takes 3 arguments");
        assert_eq!(reason("min()"), "min takes at least 1 argument");
        assert_eq!(reason("[1, 2]"), "a list is only allowed in interp");
        let size = "interp needs two lists of 2 to 50 numbers of the same length";
        assert_eq!(reason("interp(x, [0, 1], [0])"), size);
        assert_eq!(
            reason("interp(x, [0, 2, 1], [0, 1, 2])"),
            "interp: x values must increase"
        );
        assert_eq!(
            reason("interp(x, xs, ys)"),
            "interp tables must be written as lists"
        );
        assert_eq!(reason("clamp(x, 5, 0)"), "clamp: low is above high");
        assert_eq!(reason("round(x, 7)"), "round: n must be 0 to 6");
        assert_eq!(
            reason("clamp(\u{25a2}, 0, 1)"),
            "fill in the \u{25a2} slots"
        );
    }

    #[test]
    fn limits() {
        assert_eq!(
            reason(&"1".repeat(501)),
            "formula is longer than 500 characters"
        );
        let deep = format!("{}1{}", "(".repeat(33), ")".repeat(33));
        assert_eq!(reason(&deep), "formula nests deeper than 32");
        let ok = format!("{}1{}", "(".repeat(32), ")".repeat(32));
        assert_eq!(run(&ok), 1.0);
        let points = |n: usize| {
            let list = (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            format!("interp(x, [{list}], [{list}])")
        };
        assert_eq!(
            reason(&points(51)),
            "interp needs two lists of 2 to 50 numbers of the same length"
        );
        assert!(parse_formula(&points(50)).is_ok());
        assert_eq!(reason(&"-".repeat(40)), "formula nests deeper than 32");
    }

    #[test]
    fn valid_names() {
        assert!(is_valid_name("grid"));
        assert!(is_valid_name("heater_2"));
        for bad in [
            "Grid",
            "2x",
            "_x",
            "clamp",
            "trigger",
            "and",
            &"a".repeat(33),
        ] {
            assert!(!is_valid_name(bad), "{bad}");
        }
        assert!(is_valid_name(&"a".repeat(32)));
    }

    #[test]
    fn fit_to_entity_rounds_and_clamps() {
        let fit = fit_to_entity;
        assert_eq!(fit(47.6, Some(0.0), Some(100.0), Some(1.0)), (48.0, false));
        assert_eq!(fit(-5.0, Some(0.0), Some(100.0), Some(1.0)), (0.0, true));
        assert_eq!(
            fit(0.30000000000000004, None, None, Some(0.1)),
            (0.3, false)
        );
        assert_eq!(fit(12.34, None, None, None), (12.34, false));
        assert_eq!(fit(19.3, Some(10.0), Some(30.0), Some(0.5)), (19.5, false));
        assert_eq!(fit(1.6, Some(0.5), None, Some(1.0)), (1.5, false));
    }
}
