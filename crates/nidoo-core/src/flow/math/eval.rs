//! Tree-walking evaluation; every intermediate result must be a finite number.

use super::{BinOp, Expr, Func};

type Read<'a> = &'a dyn Fn(&str) -> Result<f64, String>;

const NOT_FINITE: &str = "result is not a finite number";

fn finite(v: f64) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(NOT_FINITE.into())
    }
}

fn truth(v: bool) -> f64 {
    if v { 1.0 } else { 0.0 }
}

pub(super) fn eval(expr: &Expr, read: Read) -> Result<f64, String> {
    match expr {
        Expr::Num(n) => Ok(*n),
        Expr::Var(name) => finite(read(name)?),
        Expr::Neg(a) => Ok(-eval(a, read)?),
        Expr::Not(a) => Ok(truth(eval(a, read)? == 0.0)),
        Expr::Bin(op, a, b) => binary(*op, a, b, read),
        Expr::Call(func, args) => {
            let values = args
                .iter()
                .map(|a| eval(a, read))
                .collect::<Result<Vec<_>, _>>()?;
            finite(call(*func, &values)?)
        }
        Expr::If(cond, then, otherwise) => {
            if eval(cond, read)? != 0.0 {
                eval(then, read)
            } else {
                eval(otherwise, read)
            }
        }
        Expr::Interp { x, xs, ys } => Ok(interp(eval(x, read)?, xs, ys)),
    }
}

fn binary(op: BinOp, a: &Expr, b: &Expr, read: Read) -> Result<f64, String> {
    let l = eval(a, read)?;
    match op {
        BinOp::And if l == 0.0 => return Ok(0.0),
        BinOp::Or if l != 0.0 => return Ok(1.0),
        _ => {}
    }
    let r = eval(b, read)?;
    let v = match op {
        BinOp::Add => l + r,
        BinOp::Sub => l - r,
        BinOp::Mul => l * r,
        BinOp::Div | BinOp::Rem if r == 0.0 => return Err("division by zero".into()),
        BinOp::Div => l / r,
        BinOp::Rem => l % r,
        BinOp::Lt => truth(l < r),
        BinOp::Le => truth(l <= r),
        BinOp::Gt => truth(l > r),
        BinOp::Ge => truth(l >= r),
        BinOp::Eq => truth(l == r),
        BinOp::Ne => truth(l != r),
        BinOp::And | BinOp::Or => truth(r != 0.0),
    };
    finite(v)
}

fn call(func: Func, v: &[f64]) -> Result<f64, String> {
    Ok(match func {
        Func::Min => v.iter().copied().fold(f64::INFINITY, f64::min),
        Func::Max => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Func::Clamp => {
            if v[1] > v[2] {
                return Err("clamp: low is above high".into());
            }
            v[0].max(v[1]).min(v[2])
        }
        Func::Abs => v[0].abs(),
        Func::Floor => v[0].floor(),
        Func::Ceil => v[0].ceil(),
        Func::Round => match v.get(1) {
            None => v[0].round(),
            Some(&n) if n.fract() == 0.0 && (0.0..=6.0).contains(&n) => {
                let scale = 10f64.powi(n as i32);
                (v[0] * scale).round() / scale
            }
            Some(_) => return Err("round: n must be 0 to 6".into()),
        },
    })
}

/// Piecewise-linear lookup, held at the first and last values outside the table.
fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let last = xs.len() - 1;
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[last] {
        return ys[last];
    }
    let i = xs.partition_point(|&p| p <= x) - 1;
    ys[i] + (x - xs[i]) / (xs[i + 1] - xs[i]) * (ys[i + 1] - ys[i])
}
