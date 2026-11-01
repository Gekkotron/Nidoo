//! Tokenizer and recursive-descent parser. Length, nesting depth and table sizes are bounded, so
//! parsing, evaluating and dropping a formula never overflow the stack.

use super::{
    BinOp, Expr, FUNCTIONS, Formula, FormulaError, Func, MAX_DEPTH, MAX_FORMULA_CHARS,
    MAX_TABLE_POINTS, is_valid_name,
};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Num,
    Ident,
    And,
    Or,
    Not,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
}

#[derive(Clone, Copy)]
struct Tok {
    kind: Kind,
    at: usize,
    end: usize,
}

fn error(reason: impl Into<String>, at: usize) -> FormulaError {
    FormulaError {
        reason: reason.into(),
        at: Some(at),
    }
}

/// Parses a formula.
pub fn parse_formula(text: &str) -> Result<Formula, FormulaError> {
    if text.trim().is_empty() {
        return Err(FormulaError {
            reason: "empty formula".into(),
            at: None,
        });
    }
    if text.chars().count() > MAX_FORMULA_CHARS {
        return Err(FormulaError {
            reason: format!("formula is longer than {MAX_FORMULA_CHARS} characters"),
            at: None,
        });
    }
    if let Some(at) = text.find('\u{25a2}') {
        return Err(error("fill in the \u{25a2} slots", at));
    }
    let mut parser = Parser {
        src: text,
        toks: tokenize(text)?,
        pos: 0,
        depth: 0,
    };
    let expr = parser.or()?;
    if parser.pos < parser.toks.len() {
        return Err(parser.unexpected());
    }
    Ok(Formula { expr })
}

fn tokenize(src: &str) -> Result<Vec<Tok>, FormulaError> {
    let mut toks = Vec::new();
    let mut chars = src.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let mut end = at + c.len_utf8();
        let mut take = |chars: &mut std::iter::Peekable<std::str::CharIndices>, want: char| {
            chars
                .next_if(|&(_, n)| n == want)
                .map(|(i, n)| end = i + n.len_utf8())
                .is_some()
        };
        let kind = match c {
            c if c.is_whitespace() => continue,
            '+' => Kind::Plus,
            '-' => Kind::Minus,
            '*' => Kind::Star,
            '/' => Kind::Slash,
            '%' => Kind::Percent,
            '(' => Kind::LParen,
            ')' => Kind::RParen,
            '[' => Kind::LBracket,
            ']' => Kind::RBracket,
            ',' => Kind::Comma,
            '<' if take(&mut chars, '=') => Kind::Le,
            '<' => Kind::Lt,
            '>' if take(&mut chars, '=') => Kind::Ge,
            '>' => Kind::Gt,
            '=' if take(&mut chars, '=') => Kind::Eq,
            '!' if take(&mut chars, '=') => Kind::Ne,
            '0'..='9' => {
                while let Some(&(i, n)) = chars.peek() {
                    if !n.is_ascii_digit() {
                        break;
                    }
                    end = i + 1;
                    chars.next();
                }
                // A fraction needs a digit after the dot.
                let mut ahead = chars.clone();
                if ahead.next().is_some_and(|(_, d)| d == '.')
                    && ahead.peek().is_some_and(|&(_, d)| d.is_ascii_digit())
                {
                    chars.next();
                    while let Some(&(i, n)) = chars.peek() {
                        if !n.is_ascii_digit() {
                            break;
                        }
                        end = i + 1;
                        chars.next();
                    }
                }
                Kind::Num
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                while let Some(&(i, n)) = chars.peek() {
                    if !(n.is_ascii_alphanumeric() || n == '_') {
                        break;
                    }
                    end = i + 1;
                    chars.next();
                }
                match &src[at..end] {
                    "and" => Kind::And,
                    "or" => Kind::Or,
                    "not" => Kind::Not,
                    _ => Kind::Ident,
                }
            }
            other => return Err(error(format!("unexpected {other}"), at)),
        };
        toks.push(Tok { kind, at, end });
    }
    Ok(toks)
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Kind> {
        self.toks.get(self.pos).map(|t| t.kind)
    }

    fn text(&self, tok: Tok) -> &str {
        &self.src[tok.at..tok.end]
    }

    fn eat(&mut self, kind: Kind) -> bool {
        let hit = self.peek() == Some(kind);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn unexpected(&self) -> FormulaError {
        match self.toks.get(self.pos) {
            Some(&tok) => error(format!("unexpected {}", self.text(tok)), tok.at),
            None => error("unexpected end", self.src.len()),
        }
    }

    /// The error for a missing closer: at the end of the text it reads `missing )`.
    fn closer(&self, closer: char) -> FormulaError {
        if self.pos >= self.toks.len() {
            error(format!("missing {closer}"), self.src.len())
        } else {
            self.unexpected()
        }
    }

    fn enter(&mut self) -> Result<(), FormulaError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let at = self.toks.get(self.pos).map_or(self.src.len(), |t| t.at);
            return Err(error(format!("formula nests deeper than {MAX_DEPTH}"), at));
        }
        Ok(())
    }

    fn or(&mut self) -> Result<Expr, FormulaError> {
        let mut left = self.and()?;
        while self.eat(Kind::Or) {
            let right = self.and()?;
            left = Expr::Bin(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, FormulaError> {
        let mut left = self.not()?;
        while self.eat(Kind::And) {
            let right = self.not()?;
            left = Expr::Bin(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr, FormulaError> {
        if self.peek() != Some(Kind::Not) {
            return self.comparison();
        }
        self.enter()?;
        self.pos += 1;
        let inner = self.not()?;
        self.depth -= 1;
        Ok(Expr::Not(Box::new(inner)))
    }

    fn comparison(&mut self) -> Result<Expr, FormulaError> {
        let left = self.additive()?;
        let Some(op) = self.peek().and_then(compare_op) else {
            return Ok(left);
        };
        self.pos += 1;
        let right = self.additive()?;
        if self.peek().and_then(compare_op).is_some() {
            let at = self.toks[self.pos].at;
            return Err(error("comparisons cannot be chained", at));
        }
        Ok(Expr::Bin(op, Box::new(left), Box::new(right)))
    }

    fn additive(&mut self) -> Result<Expr, FormulaError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Kind::Plus) => BinOp::Add,
                Some(Kind::Minus) => BinOp::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.multiplicative()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, FormulaError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Kind::Star) => BinOp::Mul,
                Some(Kind::Slash) => BinOp::Div,
                Some(Kind::Percent) => BinOp::Rem,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.unary()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, FormulaError> {
        if self.peek() != Some(Kind::Minus) {
            return self.primary();
        }
        self.enter()?;
        self.pos += 1;
        let inner = self.unary()?;
        self.depth -= 1;
        Ok(Expr::Neg(Box::new(inner)))
    }

    fn primary(&mut self) -> Result<Expr, FormulaError> {
        let Some(&tok) = self.toks.get(self.pos) else {
            return Err(self.unexpected());
        };
        match tok.kind {
            Kind::Num => {
                self.pos += 1;
                match self.text(tok).parse::<f64>() {
                    Ok(n) if n.is_finite() => Ok(Expr::Num(n)),
                    _ => Err(error("number is too large", tok.at)),
                }
            }
            Kind::Ident => {
                self.pos += 1;
                let name = self.text(tok).to_string();
                if self.peek() == Some(Kind::LParen) {
                    self.call(&name, tok)
                } else if FUNCTIONS.iter().any(|f| f.name == name) {
                    Err(error(format!("{name} needs arguments in ( )"), tok.at))
                } else if name == "trigger" || is_valid_name(&name) {
                    Ok(Expr::Var(name))
                } else {
                    Err(error(format!("{name} is not a valid name"), tok.at))
                }
            }
            Kind::LParen => {
                self.enter()?;
                self.pos += 1;
                let inner = self.or()?;
                if !self.eat(Kind::RParen) {
                    return Err(self.closer(')'));
                }
                self.depth -= 1;
                Ok(inner)
            }
            Kind::LBracket => Err(error("a list is only allowed in interp", tok.at)),
            _ => Err(self.unexpected()),
        }
    }

    fn call(&mut self, name: &str, name_tok: Tok) -> Result<Expr, FormulaError> {
        let Some(def) = FUNCTIONS.iter().find(|f| f.name == name) else {
            return Err(error(format!("unknown function {name}"), name_tok.at));
        };
        self.enter()?;
        self.pos += 1; // (
        let interp = name == "interp";
        let mut args: Vec<Expr> = Vec::new();
        let mut lists: Vec<Vec<f64>> = Vec::new();
        if !self.eat(Kind::RParen) {
            loop {
                if interp && (args.len() + lists.len() == 1 || args.len() + lists.len() == 2) {
                    lists.push(self.list()?);
                } else {
                    args.push(self.or()?);
                }
                if self.eat(Kind::Comma) {
                    continue;
                }
                if self.eat(Kind::RParen) {
                    break;
                }
                return Err(self.closer(')'));
            }
        }
        self.depth -= 1;
        let count = args.len() + lists.len();
        if count < def.min_args || def.max_args.is_some_and(|max| count > max) {
            return Err(error(arity_message(def), name_tok.at));
        }
        build(name, args, lists, name_tok)
    }

    /// An `interp` table: `[n, n, ...]` of plain (optionally negative) numbers.
    fn list(&mut self) -> Result<Vec<f64>, FormulaError> {
        if self.peek() != Some(Kind::LBracket) {
            let at = self.toks.get(self.pos).map_or(self.src.len(), |t| t.at);
            return Err(error("interp tables must be written as lists", at));
        }
        self.pos += 1;
        let mut values = Vec::new();
        if self.eat(Kind::RBracket) {
            return Ok(values);
        }
        loop {
            let negative = self.eat(Kind::Minus);
            match self.toks.get(self.pos).copied() {
                Some(tok) if tok.kind == Kind::Num => {
                    self.pos += 1;
                    match self.text(tok).parse::<f64>() {
                        Ok(n) if n.is_finite() => values.push(if negative { -n } else { n }),
                        _ => return Err(error("number is too large", tok.at)),
                    }
                }
                _ => return Err(self.unexpected()),
            }
            if self.eat(Kind::Comma) {
                continue;
            }
            if self.eat(Kind::RBracket) {
                return Ok(values);
            }
            return Err(self.closer(']'));
        }
    }
}

fn compare_op(kind: Kind) -> Option<BinOp> {
    Some(match kind {
        Kind::Lt => BinOp::Lt,
        Kind::Le => BinOp::Le,
        Kind::Gt => BinOp::Gt,
        Kind::Ge => BinOp::Ge,
        Kind::Eq => BinOp::Eq,
        Kind::Ne => BinOp::Ne,
        _ => return None,
    })
}

fn arity_message(def: &super::Function) -> String {
    let plural = |n: usize| if n == 1 { "argument" } else { "arguments" };
    match (def.min_args, def.max_args) {
        (min, None) => format!("{} takes at least {min} {}", def.name, plural(min)),
        (min, Some(max)) if min == max => format!("{} takes {min} {}", def.name, plural(min)),
        (min, Some(max)) => format!("{} takes {min} or {max} arguments", def.name),
    }
}

/// A plain number written in the source: `5` or `-5`.
fn literal(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Num(n) => Some(*n),
        Expr::Neg(inner) => match **inner {
            Expr::Num(n) => Some(-n),
            _ => None,
        },
        _ => None,
    }
}

fn build(
    name: &str,
    mut args: Vec<Expr>,
    mut lists: Vec<Vec<f64>>,
    tok: Tok,
) -> Result<Expr, FormulaError> {
    let func = match name {
        "if" => {
            let otherwise = args.pop().expect("arity checked");
            let then = args.pop().expect("arity checked");
            let cond = args.pop().expect("arity checked");
            return Ok(Expr::If(
                Box::new(cond),
                Box::new(then),
                Box::new(otherwise),
            ));
        }
        "interp" => {
            let ys = lists.pop().expect("arity checked");
            let xs = lists.pop().expect("arity checked");
            let x = args.pop().expect("arity checked");
            if xs.len() != ys.len() || !(2..=MAX_TABLE_POINTS).contains(&xs.len()) {
                return Err(error(
                    format!(
                        "interp needs two lists of 2 to {MAX_TABLE_POINTS} numbers of the same length"
                    ),
                    tok.at,
                ));
            }
            if xs.windows(2).any(|w| w[0] >= w[1]) {
                return Err(error("interp: x values must increase", tok.at));
            }
            return Ok(Expr::Interp {
                x: Box::new(x),
                xs,
                ys,
            });
        }
        "min" => Func::Min,
        "max" => Func::Max,
        "clamp" => {
            if let (Some(low), Some(high)) = (literal(&args[1]), literal(&args[2]))
                && low > high
            {
                return Err(error("clamp: low is above high", tok.at));
            }
            Func::Clamp
        }
        "abs" => Func::Abs,
        "round" => {
            if let Some(n) = args.get(1).and_then(literal)
                && !(n.fract() == 0.0 && (0.0..=6.0).contains(&n))
            {
                return Err(error("round: n must be 0 to 6", tok.at));
            }
            Func::Round
        }
        "floor" => Func::Floor,
        _ => Func::Ceil,
    };
    Ok(Expr::Call(func, args))
}
