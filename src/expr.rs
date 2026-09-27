//! Expression trees and their evaluation.
//!
//! Expressions are folded to constants as they are parsed wherever possible.
//! Anything that refers to a symbol which is not yet known is kept as a tree
//! and evaluated again once the whole source has been read.

use crate::value::Value;

pub type SymId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    LogNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    And,
    Or,
    Xor,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Const(Value),
    Sym(SymId),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    Undefined(SymId),
    DivZero,
    /// The operation cannot be applied to relocatable operands.
    NotAbsolute,
    Recursion,
}

/// Resolves symbol references during evaluation.
pub trait SymbolResolver {
    fn resolve(&self, id: SymId, depth: u32) -> Result<Value, EvalError>;

    /// Converts a relocatable value to an absolute one, if the address of
    /// what it is relative to is known.
    fn absolute(&self, v: Value) -> Value {
        v
    }
}

impl Expr {
    pub fn num(n: i32) -> Expr {
        Expr::Const(Value::abs(n))
    }

    pub fn as_const(&self) -> Option<Value> {
        match self {
            Expr::Const(v) => Some(*v),
            _ => None,
        }
    }

    /// The value if this is an absolute constant.
    pub fn as_abs(&self) -> Option<i32> {
        match self {
            Expr::Const(v) if v.is_abs() => Some(v.n),
            _ => None,
        }
    }

    pub fn is_const(&self) -> bool {
        matches!(self, Expr::Const(_))
    }

    /// Builds a unary node, folding if the operand is constant.
    pub fn unary(op: UnOp, e: Expr) -> Result<Expr, EvalError> {
        if let Expr::Const(v) = &e {
            match apply_unary(op, *v) {
                Ok(v) => return Ok(Expr::Const(v)),
                Err(EvalError::NotAbsolute) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Expr::Un(op, Box::new(e)))
    }

    /// Builds a binary node, folding if both operands are constant.
    pub fn binary(op: BinOp, a: Expr, b: Expr) -> Result<Expr, EvalError> {
        if let (Expr::Const(x), Expr::Const(y)) = (&a, &b) {
            match apply_binary(op, *x, *y) {
                Ok(v) => return Ok(Expr::Const(v)),
                // May still become computable once sections have addresses.
                Err(EvalError::NotAbsolute) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Expr::Bin(op, Box::new(a), Box::new(b)))
    }

    pub fn eval(&self, syms: &dyn SymbolResolver) -> Result<Value, EvalError> {
        self.eval_depth(syms, 0)
    }

    pub fn eval_depth(&self, syms: &dyn SymbolResolver, depth: u32) -> Result<Value, EvalError> {
        if depth > 64 {
            return Err(EvalError::Recursion);
        }
        match self {
            Expr::Const(v) => Ok(syms.absolute(*v)),
            Expr::Sym(id) => syms.resolve(*id, depth + 1),
            Expr::Un(op, e) => apply_unary(*op, e.eval_depth(syms, depth + 1)?),
            Expr::Bin(op, a, b) => {
                let x = a.eval_depth(syms, depth + 1)?;
                let y = b.eval_depth(syms, depth + 1)?;
                apply_binary(*op, x, y)
            }
        }
    }

    /// First symbol referenced by the expression, used for error messages.
    pub fn first_symbol(&self) -> Option<SymId> {
        match self {
            Expr::Const(_) => None,
            Expr::Sym(id) => Some(*id),
            Expr::Un(_, e) => e.first_symbol(),
            Expr::Bin(_, a, b) => a.first_symbol().or_else(|| b.first_symbol()),
        }
    }
}

pub fn apply_unary(op: UnOp, v: Value) -> Result<Value, EvalError> {
    if !v.is_abs() {
        return Err(EvalError::NotAbsolute);
    }
    Ok(Value::abs(match op {
        UnOp::Neg => v.n.wrapping_neg(),
        UnOp::Not => !v.n,
        UnOp::LogNot => {
            if v.n == 0 {
                -1
            } else {
                0
            }
        }
    }))
}

pub fn apply_binary(op: BinOp, a: Value, b: Value) -> Result<Value, EvalError> {
    match (a.base, b.base) {
        (None, None) => apply_abs(op, a.n, b.n).map(Value::abs),
        (Some(_), None) => match op {
            BinOp::Add => Ok(Value { n: a.n.wrapping_add(b.n), base: a.base }),
            BinOp::Sub => Ok(Value { n: a.n.wrapping_sub(b.n), base: a.base }),
            _ => Err(EvalError::NotAbsolute),
        },
        (None, Some(_)) => match op {
            BinOp::Add => Ok(Value { n: a.n.wrapping_add(b.n), base: b.base }),
            _ => Err(EvalError::NotAbsolute),
        },
        (Some(x), Some(y)) => {
            if x != y {
                return Err(EvalError::NotAbsolute);
            }
            // The difference or ordering of two values sharing a base is absolute.
            match op {
                BinOp::Sub
                | BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge => apply_abs(op, a.n, b.n).map(Value::abs),
                _ => Err(EvalError::NotAbsolute),
            }
        }
    }
}

fn apply_abs(op: BinOp, a: i32, b: i32) -> Result<i32, EvalError> {
    let t = |c: bool| if c { -1 } else { 0 };
    Ok(match op {
        BinOp::Add => a.wrapping_add(b),
        BinOp::Sub => a.wrapping_sub(b),
        BinOp::Mul => a.wrapping_mul(b),
        BinOp::Div => {
            if b == 0 {
                return Err(EvalError::DivZero);
            }
            a.wrapping_div(b)
        }
        BinOp::Mod => {
            if b == 0 {
                return Err(EvalError::DivZero);
            }
            a.wrapping_rem(b)
        }
        // Shift counts wrap at 32 and right shifts are logical, as in asm68k.
        BinOp::Shl => ((a as u32) << (b as u32 & 31)) as i32,
        BinOp::Shr => ((a as u32) >> (b as u32 & 31)) as i32,
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        BinOp::Eq => t(a == b),
        BinOp::Ne => t(a != b),
        BinOp::Lt => t(a < b),
        BinOp::Gt => t(a > b),
        BinOp::Le => t(a <= b),
        BinOp::Ge => t(a >= b),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoSyms;
    impl SymbolResolver for NoSyms {
        fn resolve(&self, id: SymId, _depth: u32) -> Result<Value, EvalError> {
            Err(EvalError::Undefined(id))
        }
    }

    fn bin(op: BinOp, a: i32, b: i32) -> i32 {
        apply_binary(op, Value::abs(a), Value::abs(b)).unwrap().n
    }

    #[test]
    fn shifts_are_logical_and_wrap() {
        assert_eq!(bin(BinOp::Shr, -8, 1), 0x7fff_fffc);
        assert_eq!(bin(BinOp::Shl, 1, 31), i32::MIN);
        assert_eq!(bin(BinOp::Shl, 1, 32), 1);
        assert_eq!(bin(BinOp::Shl, 1, 33), 2);
    }

    #[test]
    fn division_truncates_toward_zero() {
        assert_eq!(bin(BinOp::Div, -7, 2), -3);
        assert_eq!(bin(BinOp::Mod, -7, 2), -1);
        assert_eq!(bin(BinOp::Div, 7, -2), -3);
        assert_eq!(
            apply_binary(BinOp::Div, Value::abs(1), Value::abs(0)),
            Err(EvalError::DivZero)
        );
    }

    #[test]
    fn comparisons_are_signed_and_return_minus_one() {
        assert_eq!(bin(BinOp::Gt, -1, 2), 0);
        assert_eq!(bin(BinOp::Eq, 5, 5), -1);
        assert_eq!(bin(BinOp::Ne, 5, 5), 0);
    }

    #[test]
    fn relocatable_arithmetic() {
        use crate::value::Base;
        let a = Value::rel(Base::Section(1), 10);
        let b = Value::rel(Base::Section(1), 4);
        let c = Value::rel(Base::Section(2), 4);
        assert_eq!(apply_binary(BinOp::Sub, a, b), Ok(Value::abs(6)));
        assert_eq!(apply_binary(BinOp::Add, a, Value::abs(2)), Ok(Value::rel(Base::Section(1), 12)));
        assert_eq!(apply_binary(BinOp::Sub, a, c), Err(EvalError::NotAbsolute));
        assert_eq!(apply_binary(BinOp::Mul, a, Value::abs(2)), Err(EvalError::NotAbsolute));
    }

    #[test]
    fn folding_keeps_unknowns() {
        let e = Expr::binary(BinOp::Add, Expr::Sym(3), Expr::num(1)).unwrap();
        assert!(!e.is_const());
        assert_eq!(e.eval(&NoSyms), Err(EvalError::Undefined(3)));
        let e = Expr::binary(BinOp::Add, Expr::num(2), Expr::num(1)).unwrap();
        assert_eq!(e.as_abs(), Some(3));
    }
}
