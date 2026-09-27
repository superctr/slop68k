//! The symbol table.

use std::collections::HashMap;
use std::rc::Rc;

use crate::expr::{EvalError, Expr, SymId, SymbolResolver};
use crate::value::{Base, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymKind {
    /// Referenced but not (yet) defined.
    Undefined,
    Label,
    Equ,
    Set,
    /// String equate; the text lives in `Symbol::text`.
    Equs,
    /// Register equate; the register number (0-15) is the value.
    Equr,
    /// Register list equate; the MOVEM mask is the value.
    RegList,
    Section,
    Group,
    /// Declared with XREF and defined in another module.
    Extern,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    /// Name as written in the source.
    pub name: String,
    pub kind: SymKind,
    pub value: Option<Value>,
    /// Expression of an EQU that could not be evaluated when it was defined.
    pub expr: Option<Rc<Expr>>,
    pub text: Option<Rc<str>>,
    pub referenced: bool,
    pub exported: bool,
    /// Declared GLOBAL: exported if defined, imported otherwise.
    pub global: bool,
    pub local: bool,
    /// Source position of the definition (file id, line number).
    pub defined_at: Option<(u32, u32)>,
}

impl Symbol {
    fn new(name: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymKind::Undefined,
            value: None,
            expr: None,
            text: None,
            referenced: false,
            exported: false,
            global: false,
            local: false,
            defined_at: None,
        }
    }

    pub fn is_defined(&self) -> bool {
        self.kind != SymKind::Undefined
    }
}

#[derive(Default)]
pub struct SymbolTable {
    syms: Vec<Symbol>,
    index: HashMap<String, SymId>,
    /// Base addresses of sections, available once the layout is final.
    pub section_bases: Option<Vec<u32>>,
}

impl SymbolTable {
    pub fn new() -> SymbolTable {
        SymbolTable::default()
    }

    pub fn len(&self) -> usize {
        self.syms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.syms.is_empty()
    }

    pub fn get(&self, id: SymId) -> &Symbol {
        &self.syms[id as usize]
    }

    pub fn get_mut(&mut self, id: SymId) -> &mut Symbol {
        &mut self.syms[id as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = (SymId, &Symbol)> {
        self.syms.iter().enumerate().map(|(i, s)| (i as SymId, s))
    }

    /// Looks up a symbol by its table key.
    pub fn find(&self, key: &str) -> Option<SymId> {
        self.index.get(key).copied()
    }

    /// Finds the symbol with the given key, creating an undefined one if needed.
    pub fn intern(&mut self, key: &str, name: &str) -> SymId {
        if let Some(&id) = self.index.get(key) {
            return id;
        }
        let id = self.syms.len() as SymId;
        self.syms.push(Symbol::new(name));
        self.index.insert(key.to_string(), id);
        id
    }

    /// Removes the key so that the name can be defined afresh. The old symbol
    /// stays in the table for the benefit of expressions that refer to it.
    pub fn forget(&mut self, key: &str) {
        self.index.remove(key);
    }

    /// Makes `key` refer to an existing symbol.
    pub fn alias(&mut self, key: &str, id: SymId) {
        self.index.insert(key.to_string(), id);
    }

}

impl SymbolResolver for SymbolTable {
    /// Converts a value to an absolute one if its section has an address.
    fn absolute(&self, v: Value) -> Value {
        if let (Some(Base::Section(s)), Some(bases)) = (v.base, &self.section_bases) {
            if let Some(&b) = bases.get(s as usize) {
                return Value::abs(v.n.wrapping_add(b as i32));
            }
        }
        v
    }

    fn resolve(&self, id: SymId, depth: u32) -> Result<Value, EvalError> {
        let sym = self.get(id);
        if let Some(v) = sym.value {
            return Ok(self.absolute(v));
        }
        if let Some(e) = &sym.expr {
            return e.eval_depth(self, depth + 1).map(|v| self.absolute(v));
        }
        if sym.kind == SymKind::Extern {
            return Ok(Value::rel(Base::Extern(id), 0));
        }
        Err(EvalError::Undefined(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::BinOp;

    #[test]
    fn deferred_equates_resolve_later() {
        let mut t = SymbolTable::new();
        let a = t.intern("a", "a");
        let b = t.intern("b", "b");
        // a equ b+1, with b defined afterwards
        let e = Expr::binary(BinOp::Add, Expr::Sym(b), Expr::num(1)).unwrap();
        t.get_mut(a).kind = SymKind::Equ;
        t.get_mut(a).expr = Some(Rc::new(e));
        assert_eq!(t.resolve(a, 0), Err(EvalError::Undefined(b)));
        t.get_mut(b).kind = SymKind::Label;
        t.get_mut(b).value = Some(Value::abs(9));
        assert_eq!(t.resolve(a, 0), Ok(Value::abs(10)));
    }

    #[test]
    fn recursive_equates_are_caught() {
        let mut t = SymbolTable::new();
        let a = t.intern("a", "a");
        t.get_mut(a).kind = SymKind::Equ;
        t.get_mut(a).expr = Some(Rc::new(Expr::Sym(a)));
        assert_eq!(t.resolve(a, 0), Err(EvalError::Recursion));
    }
}
