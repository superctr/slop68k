//! Parsing of instruction operands.

use super::scan::*;
use super::{Assembler, R};
use crate::expr::Expr;
use crate::m68k::operand::{IndexReg, Operand, Reg};
use crate::symbol::SymKind;

impl Assembler<'_> {
    /// Looks up a register by name, including register equates.
    pub(crate) fn lookup_reg(&self, name: &str) -> Option<Reg> {
        if let Some(r) = Reg::parse(name) {
            return Some(r);
        }
        let b = name.as_bytes();
        if b.is_empty() || !is_ident_start(b[0], self.opts.local_char) || b.iter().any(|&c| !is_ident_char(c) && c as char != self.opts.local_char) {
            return None;
        }
        if self.equr_count == 0 {
            return None;
        }
        let id = self.syms.find(&self.sym_key(name))?;
        let sym = self.syms.get(id);
        if sym.kind == SymKind::Equr {
            return Some(Reg::from_number(sym.value?.n as u8));
        }
        None
    }

    /// Parses a register list such as `d0-d3/a0`, returning the MOVEM mask.
    pub(crate) fn parse_reglist(&self, s: &str) -> Option<u16> {
        let mut mask = 0u16;
        for part in s.split('/') {
            let part = part.trim();
            if let Some((a, b)) = part.split_once('-') {
                let a = self.lookup_reg(a.trim())?.number()?;
                let b = self.lookup_reg(b.trim())?.number()?;
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                for n in lo..=hi {
                    mask |= 1 << n;
                }
            } else if let Some(r) = self.lookup_reg(part) {
                mask |= 1 << r.number()?;
            } else {
                let id = self.syms.find(&self.sym_key(part))?;
                let sym = self.syms.get(id);
                if sym.kind != SymKind::RegList {
                    return None;
                }
                mask |= sym.value?.n as u16;
            }
        }
        Some(mask)
    }

    /// Parses an index register with its optional size.
    fn parse_index(&self, s: &str) -> Option<IndexReg> {
        let s = s.trim();
        let b = s.as_bytes();
        let (name, long) = if b.len() > 2 && is_size_suffix(b, b.len() - 2) {
            match b[b.len() - 1].to_ascii_lowercase() {
                b'w' => (&s[..s.len() - 2], false),
                b'l' => (&s[..s.len() - 2], true),
                _ => return None,
            }
        } else {
            (s, false)
        };
        let reg = self.lookup_reg(name)?.number()?;
        Some(IndexReg { reg, long })
    }

    fn addr_reg(&self, s: &str) -> Option<u8> {
        match self.lookup_reg(s.trim()) {
            Some(Reg::Addr(n)) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn parse_operand(&mut self, text: &str) -> R<Operand> {
        let s = text.trim();
        if s.is_empty() {
            return Err("Illegal addressing mode".into());
        }
        let b = s.as_bytes();

        if b[0] == b'#' {
            return Ok(Operand::Imm(self.parse_expr(&s[1..])?));
        }

        if let Some(r) = self.lookup_reg(s) {
            return Ok(match r {
                Reg::Data(n) => Operand::DataReg(n),
                Reg::Addr(n) => Operand::AddrReg(n),
                Reg::Sr => Operand::Sr,
                Reg::Ccr => Operand::Ccr,
                Reg::Usp => Operand::Usp,
                Reg::Pc => return Err("Illegal addressing mode".into()),
            });
        }

        if b[b.len() - 1] == b')' {
            if b.starts_with(b"-(") {
                if let Some(n) = self.addr_reg(&s[2..s.len() - 1]) {
                    return Ok(Operand::PreDec(n));
                }
            }
            if let Some(open) = matching_open(b) {
                if let Some(op) = self.parse_indirect(&s[..open], &s[open + 1..s.len() - 1])? {
                    return Ok(op);
                }
            }
        } else if b.ends_with(b")+") && b[0] == b'(' {
            if let Some(n) = self.addr_reg(&s[1..s.len() - 2]) {
                return Ok(Operand::PostInc(n));
            }
        }

        if s.contains(['/', '-']) || self.reg_count > 0 {
            if let Some(mask) = self.parse_reglist(s) {
                return Ok(Operand::RegList(mask));
            }
        }

        // What remains is an absolute address, possibly with a size.
        if b.len() > 2 && is_size_suffix(b, b.len() - 2) {
            let e = self.parse_expr(&s[..s.len() - 2])?;
            return match b[b.len() - 1].to_ascii_lowercase() {
                b'w' => Ok(Operand::AbsShort(e)),
                b'l' => Ok(Operand::AbsLong(e)),
                _ => Err("Illegal size specifier for absolute address".into()),
            };
        }
        // The older syntax for the same thing: `addr\w`.
        if b.len() > 2 && b[b.len() - 2] == b'\\' {
            let size = b[b.len() - 1].to_ascii_lowercase();
            if size == b'w' || size == b'l' {
                let e = self.parse_expr(&s[..s.len() - 2])?;
                return Ok(if size == b'w' { Operand::AbsShort(e) } else { Operand::AbsLong(e) });
            }
        }
        match self.parse_expr(s) {
            Ok(e) => Ok(Operand::Abs(e)),
            // Something in parentheses that is not a known addressing mode.
            Err(_) if b[b.len() - 1] == b')' && matching_open(b).is_some_and(|n| n > 0) => {
                Err("Illegal addressing mode".into())
            }
            Err(e) => Err(e),
        }
    }

    /// Parses the register indirect modes, given the text in front of and
    /// inside the parentheses. Returns `None` if the parentheses turn out to
    /// be part of an expression.
    fn parse_indirect(&mut self, prefix: &str, inner: &str) -> R<Option<Operand>> {
        let parts = split_operands(inner);
        let prefix = prefix.trim();

        // Find the base register: the first part that is an address
        // register or the program counter.
        let base_at = parts.iter().position(|p| {
            matches!(self.lookup_reg(p.trim()), Some(Reg::Addr(_)) | Some(Reg::Pc))
        });
        let Some(base_at) = base_at else {
            return Ok(None);
        };
        if base_at > 1 || (base_at == 1 && !prefix.is_empty()) {
            return Ok(None);
        }
        let base = self.lookup_reg(parts[base_at].trim());
        let rest = &parts[base_at + 1..];
        if rest.len() > 1 {
            return Err("Illegal addressing mode".into());
        }

        let disp_text = if base_at == 1 { parts[0] } else { prefix };
        let disp = if disp_text.is_empty() { None } else { Some(self.parse_expr(disp_text)?) };
        let index = match rest.first() {
            Some(x) => Some(self.parse_index(x).ok_or("Illegal index register")?),
            None => None,
        };

        Ok(Some(match (base, index) {
            (Some(Reg::Addr(n)), None) => match disp {
                Some(d) => Operand::Disp(n, d),
                None => Operand::Indirect(n),
            },
            (Some(Reg::Addr(n)), Some(x)) => Operand::Index(n, disp.unwrap_or(Expr::num(0)), x),
            (Some(Reg::Pc), None) => Operand::PcDisp(self.pc_target(disp)),
            (Some(Reg::Pc), Some(x)) => Operand::PcIndex(self.pc_target(disp), x),
            _ => return Ok(None),
        }))
    }

    /// The target of a PC relative operand. As with asm68k, leaving it out
    /// is the same as giving zero.
    fn pc_target(&self, disp: Option<Expr>) -> Expr {
        disp.unwrap_or(Expr::num(0))
    }
}
