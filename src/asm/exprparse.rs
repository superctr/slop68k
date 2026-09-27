//! Parsing of expressions.
//!
//! Operator precedence follows asm68k rather than C. From lowest to
//! highest: equality, relational, additive, multiplicative, logical, shift,
//! unary.

use super::scan::*;
use super::{Assembler, R};
use crate::expr::{BinOp, EvalError, Expr, UnOp};
use crate::symbol::SymKind;
use crate::value::Value;

const LEVELS: usize = 6;

pub(crate) struct Cursor<'a> {
    pub text: &'a str,
    pub s: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(text: &'a str) -> Cursor<'a> {
        Cursor { text, s: text.as_bytes(), pos: 0 }
    }

    pub fn skip_ws(&mut self) {
        self.pos = skip_space(self.s, self.pos);
    }

    pub fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    pub fn peek_at(&self, n: usize) -> Option<u8> {
        self.s.get(self.pos + n).copied()
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.s.len()
    }

    pub fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// The binary operator of the given precedence level at the cursor.
    fn binop(&self, level: usize) -> Option<(BinOp, usize)> {
        let c = self.peek()?;
        let n = self.peek_at(1);
        match level {
            0 => match (c, n) {
                (b'=', Some(b'=')) => Some((BinOp::Eq, 2)),
                (b'=', _) => Some((BinOp::Eq, 1)),
                (b'<', Some(b'>')) => Some((BinOp::Ne, 2)),
                _ => None,
            },
            1 => match (c, n) {
                (b'<', Some(b'=')) => Some((BinOp::Le, 2)),
                (b'>', Some(b'=')) => Some((BinOp::Ge, 2)),
                (b'<', Some(b'<' | b'>')) | (b'>', Some(b'>')) => None,
                (b'<', _) => Some((BinOp::Lt, 1)),
                (b'>', _) => Some((BinOp::Gt, 1)),
                _ => None,
            },
            2 => match c {
                b'+' => Some((BinOp::Add, 1)),
                b'-' => Some((BinOp::Sub, 1)),
                _ => None,
            },
            3 => match c {
                b'*' => Some((BinOp::Mul, 1)),
                b'/' => Some((BinOp::Div, 1)),
                b'%' => Some((BinOp::Mod, 1)),
                _ => None,
            },
            4 => match c {
                b'&' => Some((BinOp::And, 1)),
                b'!' | b'|' => Some((BinOp::Or, 1)),
                b'^' => Some((BinOp::Xor, 1)),
                _ => None,
            },
            5 => match (c, n) {
                (b'<', Some(b'<')) => Some((BinOp::Shl, 2)),
                (b'>', Some(b'>')) => Some((BinOp::Shr, 2)),
                _ => None,
            },
            _ => None,
        }
    }
}

fn eval_error(e: EvalError) -> String {
    match e {
        EvalError::DivZero => "Division by zero".into(),
        EvalError::Recursion => "Out of stack space, possibly due to recursive equates".into(),
        EvalError::NotAbsolute | EvalError::Undefined(_) => "Error in expression".into(),
    }
}

fn is_function(name: &str) -> bool {
    matches!(
        name,
        "def"
            | "ref"
            | "type"
            | "sqrt"
            | "strlen"
            | "strcmp"
            | "instr"
            | "filesize"
            | "sect"
            | "offset"
            | "sectoff"
            | "group"
            | "groupoff"
            | "groupsize"
            | "grouporg"
            | "groupend"
            | "sectend"
            | "sectsize"
            | "alignment"
    )
}

impl Assembler<'_> {
    /// Parses an expression that makes up the whole of `text`.
    pub(crate) fn parse_expr(&mut self, text: &str) -> R<Expr> {
        let mut c = Cursor::new(text);
        let e = self.parse_expr_at(&mut c)?;
        c.skip_ws();
        if !c.at_end() {
            return Err("Unexpected characters at end of line".into());
        }
        Ok(e)
    }

    /// Parses an expression that starts at the cursor.
    pub(crate) fn parse_expr_at(&mut self, c: &mut Cursor) -> R<Expr> {
        self.expr_depth += 1;
        if self.expr_depth > 64 {
            self.expr_depth -= 1;
            return Err("Possible infinite loop in string substitution".into());
        }
        let r = self.binary(c, 0);
        self.expr_depth -= 1;
        r
    }

    /// Parses an expression whose value has to be known at this point.
    pub(crate) fn eval_now(&mut self, text: &str) -> R<i32> {
        if text.trim().is_empty() {
            return Err("Error in expression".into());
        }
        let e = self.parse_expr(text)?;
        self.const_value(&e)
    }

    /// The value of an expression that has to be a known absolute number.
    pub(crate) fn const_value(&self, e: &Expr) -> R<i32> {
        match e {
            Expr::Const(v) if v.is_abs() => Ok(v.n),
            _ => Err("Expression must evaluate".into()),
        }
    }

    fn binary(&mut self, c: &mut Cursor, level: usize) -> R<Expr> {
        if level >= LEVELS {
            return self.unary(c);
        }
        let mut left = self.binary(c, level + 1)?;
        loop {
            c.skip_ws();
            let Some((op, len)) = c.binop(level) else { break };
            c.pos += len;
            c.skip_ws();
            let right = self.binary(c, level + 1)?;
            left = Expr::binary(op, left, right).map_err(eval_error)?;
        }
        Ok(left)
    }

    fn unary(&mut self, c: &mut Cursor) -> R<Expr> {
        c.skip_ws();
        let op = match c.peek() {
            Some(b'-') => UnOp::Neg,
            Some(b'~') => UnOp::Not,
            Some(b'!') => UnOp::LogNot,
            Some(b'+') => {
                c.pos += 1;
                return self.unary(c);
            }
            _ => return self.primary(c),
        };
        c.pos += 1;
        let e = self.unary(c)?;
        Expr::unary(op, e).map_err(eval_error)
    }

    fn primary(&mut self, c: &mut Cursor) -> R<Expr> {
        c.skip_ws();
        let Some(ch) = c.peek() else {
            return Err("Syntax error in expression".into());
        };
        let local = self.opts.local_char;
        match ch {
            b'(' => {
                c.pos += 1;
                let e = self.binary(c, 0)?;
                c.skip_ws();
                if !c.eat(b')') {
                    return Err("Missing closing bracket in expression".into());
                }
                Ok(e)
            }
            b'0'..=b'9' => self.number(c),
            b'$' => {
                c.pos += 1;
                let start = c.pos;
                let mut n: u32 = 0;
                while let Some(d) = c.peek().and_then(|d| (d as char).to_digit(16)) {
                    n = n.wrapping_shl(4) | d;
                    c.pos += 1;
                }
                if c.pos == start {
                    return Err("Non-hexadecimal character starting number".into());
                }
                Ok(Expr::num(n as i32))
            }
            b'%' => {
                c.pos += 1;
                let start = c.pos;
                let mut n: u32 = 0;
                while let Some(d @ (b'0' | b'1')) = c.peek() {
                    n = n.wrapping_shl(1) | (d - b'0') as u32;
                    c.pos += 1;
                }
                if c.pos == start {
                    return Err("Non-binary character following %".into());
                }
                Ok(Expr::num(n as i32))
            }
            b'\'' | b'"' => {
                let end = skip_string(c.s, c.pos).ok_or("No closing quote on string")?;
                let text = unquote(&c.text[c.pos..end]);
                c.pos = end;
                // Up to four characters make up a number.
                if text.chars().count() > 4 {
                    return Err("Too many characters in character constant".into());
                }
                let mut n: u32 = 0;
                for ch in text.chars() {
                    n = n.wrapping_shl(8) | (ch as u32 & 0xff);
                }
                Ok(Expr::num(n as i32))
            }
            b'*' => {
                c.pos += 1;
                Ok(Expr::Const(self.line_pc))
            }
            _ if ch as char == local && !c.peek_at(1).is_some_and(is_ident_char) => {
                // On its own the local label prefix is the address at which
                // the value being defined will be stored.
                c.pos += 1;
                Ok(Expr::Const(self.pc_value()))
            }
            _ if is_ident_start(ch, local) => self.name(c),
            // A macro parameter that was not given leaves nothing in front
            // of an operator, which asm68k takes to be zero.
            b'<' | b'>' | b'=' | b'&' | b'|' | b'^' if c.pos + 1 < c.s.len() => Ok(Expr::num(0)),
            _ => Err("Syntax error in expression".into()),
        }
    }

    fn number(&mut self, c: &mut Cursor) -> R<Expr> {
        let start = c.pos;
        if c.peek() == Some(b'0') && matches!(c.peek_at(1), Some(b'x' | b'X')) {
            // Hexadecimal as written in C.
            c.pos += 2;
            let digits = c.pos;
            let mut n: u32 = 0;
            while let Some(d) = c.peek().and_then(|d| (d as char).to_digit(16)) {
                n = n.wrapping_shl(4) | d;
                c.pos += 1;
            }
            if c.pos == digits {
                return Err("Non-hexadecimal character starting number".into());
            }
            return Ok(Expr::num(n as i32));
        }
        let mut end = start;
        while end < c.s.len() && c.s[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let word = &c.text[start..end];
        let mut radix = self.radix;
        let mut digits = word;
        if self.opts.alt_numeric && self.radix <= 10 {
            let (body, suffix) = word.split_at(word.len() - 1);
            let r = match suffix {
                "h" | "H" => 16,
                "b" | "B" => 2,
                "d" | "D" => 10,
                _ => 0,
            };
            if r != 0 && !body.is_empty() {
                radix = r;
                digits = body;
            }
        }
        // Without the alternate syntax a number ends at the first
        // character that is not a digit in the current radix.
        let mut n: u32 = 0;
        let mut used = 0;
        for ch in digits.chars() {
            match ch.to_digit(radix) {
                Some(d) => n = n.wrapping_mul(radix).wrapping_add(d),
                None => break,
            }
            used += 1;
        }
        if digits.len() != word.len() {
            if used != digits.len() {
                return Err(match radix {
                    16 => "Illegal digit in suffixed hexadecimal number",
                    2 => "Illegal digit in suffixed binary number",
                    _ => "Illegal digit in suffixed decimal number",
                }
                .into());
            }
            c.pos = end;
        } else {
            if used == 0 {
                return Err("Decimal number illegal in this radix".into());
            }
            c.pos = start + used;
        }
        Ok(Expr::num(n as i32))
    }

    fn name(&mut self, c: &mut Cursor) -> R<Expr> {
        let start = c.pos;
        let end = scan_ident(c.s, start);
        let name = &c.text[start..end];
        c.pos = end;

        if name.len() <= 9 && name.as_bytes()[0] != self.opts.local_char as u8 {
            let lower = name.to_ascii_lowercase();
            if is_function(&lower) {
                let mut probe = end;
                probe = skip_space(c.s, probe);
                if c.s.get(probe) == Some(&b'(') && !self.disabled.contains(&lower) {
                    c.pos = probe + 1;
                    let v = self.function(&lower, c)?;
                    c.skip_ws();
                    if !c.eat(b')') {
                        return Err("Missing ')' after function parameter(s)".into());
                    }
                    return Ok(v);
                }
            }
            if let Some(v) = self.builtin_constant(&lower)? {
                return Ok(Expr::num(v));
            }
        }
        self.symbol_ref(name)
    }

    /// Value of the pre-defined constants.
    fn builtin_constant(&mut self, lower: &str) -> R<Option<i32>> {
        if self.disabled.contains(lower) {
            return Ok(None);
        }
        let t = &self.time;
        Ok(Some(match lower {
            "narg" => match self.macro_ctx() {
                Some(m) => m.args.len() as i32,
                None => return Err("Narg can only be used inside a macro".into()),
            },
            "__rs" => self.rs,
            "_year" => t.year - 1900,
            "_month" => t.month,
            "_day" => t.day,
            "_weekday" => t.weekday,
            "_hours" => t.hours,
            "_minutes" => t.minutes,
            "_seconds" => t.seconds,
            _ => return Ok(None),
        }))
    }

    /// Turns a reference to a symbol into an expression.
    pub(crate) fn symbol_ref(&mut self, name: &str) -> R<Expr> {
        if name.as_bytes()[0].is_ascii_digit() {
            return Err("Error in expression".into());
        }
        let key = self.sym_key(name);
        let id = self.syms.intern(&key, name);
        self.syms.get_mut(id).referenced = true;
        let sym = self.syms.get(id);
        match sym.kind {
            SymKind::Equs => {
                let text = sym.text.clone().unwrap_or_else(|| "".into());
                let mut sub = Cursor::new(&text);
                let e = self.parse_expr_at(&mut sub)?;
                return Ok(e);
            }
            SymKind::Equr | SymKind::RegList => {
                return Err("Illegal use of register equate".into());
            }
            SymKind::Section | SymKind::Group => {
                return Err(format!("'{}' cannot be used in an expression", name));
            }
            SymKind::Undefined if self.find_macro(name).is_some() => {
                return Err(format!("'{}' cannot be used in an expression", name));
            }
            _ => {}
        }
        let sym = self.syms.get(id);
        Ok(match sym.value {
            Some(v) => Expr::Const(v),
            None => Expr::Sym(id),
        })
    }

    /// Parses a quoted string argument of a function.
    fn string_arg(&mut self, func: &str, c: &mut Cursor) -> R<String> {
        c.skip_ws();
        match c.peek() {
            Some(b'\'' | b'"') => {
                let end = skip_string(c.s, c.pos).ok_or("No closing quote on string")?;
                let s = unquote(&c.text[c.pos..end]);
                c.pos = end;
                Ok(s)
            }
            _ => Err(format!("{}'s operand must be a quoted string", func)),
        }
    }

    /// Parses a name given as the argument of a function.
    fn name_arg<'a>(&mut self, c: &mut Cursor<'a>) -> Option<&'a str> {
        c.skip_ws();
        let ch = c.peek()?;
        if !is_ident_start(ch, self.opts.local_char) {
            return None;
        }
        let end = scan_ident(c.s, c.pos);
        let name = &c.text[c.pos..end];
        c.pos = end;
        Some(name)
    }

    fn comma(&mut self, c: &mut Cursor, func: &str) -> R<()> {
        c.skip_ws();
        if c.eat(b',') {
            Ok(())
        } else {
            Err(format!("Expecting comma between operands in '{}'", func))
        }
    }

    fn function(&mut self, func: &str, c: &mut Cursor) -> R<Expr> {
        let n = match func {
            "def" => {
                let name = self.name_arg(c).ok_or("DEF's parameter must be a name")?;
                Value::from_bool(self.is_defined(name)).n
            }
            "ref" => {
                let name = self.name_arg(c).ok_or("REF's parameter must be a name")?;
                let key = self.sym_key(name);
                let r = self.syms.find(&key).is_some_and(|id| self.syms.get(id).referenced);
                Value::from_bool(r).n
            }
            "type" => {
                let name = self.name_arg(c).ok_or("TYPE's parameter must be a name")?;
                self.symbol_type(name)
            }
            "sqrt" => {
                let e = self.binary(c, 0)?;
                let v = self.const_value(&e)?;
                if v < 0 {
                    return Err(format!("Illegal value ({})", v));
                }
                (v as f64).sqrt().floor() as i32
            }
            "strlen" => self.string_arg(func, c)?.chars().count() as i32,
            "strcmp" => {
                let a = self.string_arg(func, c)?;
                self.comma(c, func)?;
                let b = self.string_arg(func, c)?;
                Value::from_bool(a == b).n
            }
            "instr" => {
                c.skip_ws();
                let mut from = 1;
                if !matches!(c.peek(), Some(b'\'' | b'"')) {
                    let e = self.binary(c, 0)?;
                    from = self.const_value(&e)?;
                    self.comma(c, func)?;
                }
                let a = self.string_arg(func, c)?;
                self.comma(c, func)?;
                let b = self.string_arg(func, c)?;
                if from < 1 {
                    return Err("Illegal value for base in INSTR".into());
                }
                let hay: Vec<char> = a.chars().collect();
                let needle: Vec<char> = b.chars().collect();
                let from = from as usize - 1;
                let mut found = 0;
                if !needle.is_empty() && from < hay.len() {
                    if let Some(i) = hay[from..].windows(needle.len()).position(|w| w == needle.as_slice()) {
                        found = (from + i + 1) as i32;
                    }
                }
                found
            }
            "filesize" => {
                let name = self.string_arg(func, c)?;
                match self.locate(&name) {
                    Some(p) => std::fs::metadata(p).map(|m| m.len() as i32).unwrap_or(-1),
                    None => -1,
                }
            }
            "offset" | "sectoff" | "groupoff" => {
                let e = self.binary(c, 0)?;
                match e {
                    Expr::Const(v) => v.n,
                    _ => return Err(format!("{}'s parameter must be a defined name", func.to_uppercase())),
                }
            }
            "sect" | "group" | "grouporg" => {
                let e = self.binary(c, 0)?;
                return match e {
                    Expr::Const(v) => Ok(Expr::Const(Value { n: 0, base: v.base })),
                    _ => Err(format!("{}'s parameter must be a defined name", func.to_uppercase())),
                };
            }
            "sectsize" | "groupsize" | "sectend" | "groupend" => {
                let name = self.name_arg(c).ok_or("Illegal section name")?;
                return self.section_function(func, name);
            }
            "alignment" => {
                let e = self.binary(c, 0)?;
                match e {
                    Expr::Const(v) => v.n & 3,
                    _ => return Err("Alignment's parameter must be a defined name".into()),
                }
            }
            _ => return Err("Error in expression".into()),
        };
        Ok(Expr::num(n))
    }

    /// Whether a name has been defined as a symbol or macro.
    pub(crate) fn is_defined(&self, name: &str) -> bool {
        let key = self.sym_key(name);
        self.syms.find(&key).is_some_and(|id| self.syms.get(id).is_defined())
            || self.macros.contains_key(&key)
    }

    /// The value of the TYPE function.
    fn symbol_type(&self, name: &str) -> i32 {
        let key = self.sym_key(name);
        let mut t = 0;
        if let Some(m) = self.macros.get(&key) {
            t |= 1 << 3;
            if m.short {
                t |= 1 << 11;
            }
        }
        if let Some(m) = self.macro_ctx() {
            if m.params.iter().any(|p| p.eq_ignore_ascii_case(name)) {
                t |= 1 << 10;
            }
        }
        let Some(id) = self.syms.find(&key) else { return t };
        let sym = self.syms.get(id);
        let abs = sym.value.is_none_or(|v| v.is_abs());
        match sym.kind {
            SymKind::Undefined => {}
            SymKind::Label => t |= if abs { 1 } else { 2 },
            SymKind::Equ => t |= 1 << 5 | if abs { 1 } else { 2 },
            SymKind::Set => t |= 1 << 2 | 1 << 5,
            SymKind::Equs => t |= 1 << 4,
            SymKind::Equr => t |= 1 << 14,
            SymKind::RegList => t |= 1 << 15,
            SymKind::Section => t |= 1 << 12,
            SymKind::Group => t |= 1 << 9,
            SymKind::Extern => t |= 1 << 6,
        }
        if sym.exported {
            t |= 1 << 7;
        }
        t
    }
}
