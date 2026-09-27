//! Text substitution: macro parameters, string equates and the special
//! backslash sequences.

use std::borrow::Cow;

use super::scan::*;
use super::{Assembler, R};
use crate::symbol::SymKind;

/// Whether `c` can be part of a name that follows a backslash. Dots are
/// left out so that `\1.w` and `\name.l` work as expected.
fn is_subst_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'?' || c >= 0x80
}

impl Assembler<'_> {
    /// Replaces the backslash sequences of a line.
    pub(crate) fn substitute(&mut self, line: &str) -> R<String> {
        let b = line.as_bytes();
        let mut out = String::with_capacity(line.len() + 16);
        let mut i = 0;
        let mut copied = 0;
        while let Some(n) = b[i..].iter().position(|&c| c == b'\\') {
            let at = i + n;
            let Some(&c) = b.get(at + 1) else { break };
            out.push_str(&line[copied..at]);
            let mut end = at + 2;
            let mut terminated = true;
            match c {
                b'0'..=b'9' => {
                    while end < b.len() && b[end].is_ascii_digit() {
                        end += 1;
                    }
                    let n: usize = line[at + 1..end].parse().unwrap_or(usize::MAX);
                    match self.macro_ctx() {
                        Some(m) if n == 0 => out.push_str(&m.size),
                        Some(m) => {
                            if n > 32 {
                                return Err("Illegal parameter number".into());
                            }
                            if let Some(a) = m.args.get(n - 1) {
                                out.push_str(a);
                            }
                        }
                        None => out.push_str(&line[at..end]),
                    }
                    terminated = false;
                }
                b'@' => {
                    match self.macro_ctx() {
                        Some(m) => {
                            out.push('_');
                            out.push_str(&m.unique.to_string());
                        }
                        None => out.push_str("\\@"),
                    }
                    terminated = false;
                }
                b'#' | b'$' => {
                    let start = at + 2;
                    end = start;
                    if end < b.len() && b[end] as char == self.opts.local_char {
                        end += 1;
                    }
                    while end < b.len() && is_subst_char(b[end]) {
                        end += 1;
                    }
                    let name = &line[start..end];
                    if name.is_empty() {
                        out.push_str(&line[at..end]);
                    } else {
                        let v = self.eval_now(name)?;
                        if c == b'#' {
                            out.push_str(&v.to_string());
                        } else {
                            out.push_str(&format!("{:x}", v));
                        }
                    }
                }
                b'_' if !b.get(at + 2).is_some_and(|&c| is_subst_char(c)) && self.macro_ctx().is_some() => {
                    if let Some(m) = self.macro_ctx() {
                        out.push_str(&m.all);
                    }
                    terminated = false;
                }
                b'*' => match self.macro_ctx() {
                    Some(m) => out.push_str(&m.label),
                    None => out.push_str("\\*"),
                },
                b'^' => {
                    match b.get(at + 2) {
                        Some(&x) if x.is_ascii() => {
                            out.push((x & 0x1f) as char);
                            end = at + 3;
                        }
                        _ => out.push_str("\\^"),
                    }
                    terminated = false;
                }
                b'\\' => {
                    out.push('\\');
                    terminated = false;
                }
                _ if is_subst_char(c) || c as char == self.opts.local_char => {
                    // Characters outside of ASCII take more than one byte,
                    // all of which count as part of a name.
                    end = at + 1;
                    if c as char == self.opts.local_char {
                        end += 1;
                    }
                    while end < b.len() && is_subst_char(b[end]) {
                        end += 1;
                    }
                    let name = &line[at + 1..end];
                    match self.text_of(name) {
                        Some(text) => out.push_str(&text),
                        None => {
                            // Not something to substitute: leave it alone.
                            out.push_str(&line[at..end]);
                            terminated = false;
                        }
                    }
                }
                _ => {
                    out.push('\\');
                    end = at + 1;
                    terminated = false;
                }
            }
            // A backslash may end the name to separate it from what follows.
            if terminated && b.get(end) == Some(&b'\\') {
                end += 1;
            }
            copied = end;
            i = end;
            if i >= b.len() {
                break;
            }
        }
        out.push_str(&line[copied..]);
        Ok(out)
    }

    /// The text a name stands for: a macro argument or a string equate.
    pub(crate) fn text_of(&self, name: &str) -> Option<Cow<'_, str>> {
        if let Some(m) = self.macro_ctx() {
            if let Some(n) = m.params.iter().position(|p| p.eq_ignore_ascii_case(name)) {
                return Some(Cow::Borrowed(m.args.get(n).map_or("", |a| a.as_str())));
            }
        }
        if name.eq_ignore_ascii_case("_filename") {
            return Some(Cow::Borrowed(self.cfg.source.as_str()));
        }
        if self.equs_count == 0 {
            return None;
        }
        let id = self.syms.find(&self.sym_key(name))?;
        let sym = self.syms.get(id);
        if sym.kind == SymKind::Equs {
            return sym.text.as_deref().map(Cow::Borrowed);
        }
        None
    }

    /// Whether names in operands may need replacing at all.
    fn has_text_names(&self) -> bool {
        self.equs_count > 0 || self.macro_ctx().is_some_and(|m| !m.params.is_empty())
    }

    /// Replaces names of string equates and macro parameters that appear
    /// in an operand field without a backslash.
    pub(crate) fn expand_names<'a>(&self, field: &'a str) -> R<Cow<'a, str>> {
        if !self.has_text_names() {
            return Ok(Cow::Borrowed(field));
        }
        let mut text = Cow::Borrowed(field);
        for _ in 0..32 {
            match self.expand_names_once(&text) {
                Some(t) => text = Cow::Owned(t),
                None => return Ok(text),
            }
        }
        Err("Possible infinite loop in string substitution".into())
    }

    fn expand_names_once(&self, field: &str) -> Option<String> {
        let b = field.as_bytes();
        let local = self.opts.local_char;
        let mut out: Option<String> = None;
        let mut copied = 0;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if c == b'\'' || c == b'"' {
                i = skip_string(b, i).unwrap_or(b.len());
            } else if c == b'$' || c.is_ascii_digit() {
                // Numbers may contain letters.
                i += 1;
                while i < b.len() && b[i].is_ascii_alphanumeric() {
                    i += 1;
                }
            } else if is_ident_start(c, local) {
                let end = scan_ident(b, i);
                let name = &field[i..end];
                if !self.is_name_argument(b, i) {
                    if let Some(text) = self.text_of(name) {
                        let out = out.get_or_insert_with(|| String::with_capacity(field.len() + 16));
                        out.push_str(&field[copied..i]);
                        out.push_str(&text);
                        copied = end;
                    }
                }
                i = end;
            } else {
                i += 1;
            }
        }
        out.map(|mut o| {
            o.push_str(&field[copied..]);
            o
        })
    }

    /// Whether the name at `pos` is the argument of a function that takes a
    /// name rather than a value.
    fn is_name_argument(&self, b: &[u8], pos: usize) -> bool {
        if pos == 0 || b[pos - 1] != b'(' {
            return false;
        }
        let end = pos - 1;
        let mut start = end;
        while start > 0 && b[start - 1].is_ascii_alphabetic() {
            start -= 1;
        }
        let func = &b[start..end];
        func.eq_ignore_ascii_case(b"def") || func.eq_ignore_ascii_case(b"ref") || func.eq_ignore_ascii_case(b"type")
    }
}
