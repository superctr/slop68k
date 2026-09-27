//! Values produced by expression evaluation.
//!
//! A value is a 32-bit integer which may be relative to a base (a section
//! start or an external symbol). Absolute programs only ever see values with
//! no base; relocatable output keeps the base around so that relocations can
//! be emitted for it.

/// What a relocatable value is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    /// Start of the section with the given index.
    Section(u32),
    /// An externally defined symbol (symbol id).
    Extern(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value {
    pub n: i32,
    pub base: Option<Base>,
}

impl Value {
    pub const fn abs(n: i32) -> Value {
        Value { n, base: None }
    }

    pub const fn rel(base: Base, n: i32) -> Value {
        Value { n, base: Some(base) }
    }

    pub fn is_abs(&self) -> bool {
        self.base.is_none()
    }

    pub fn from_bool(b: bool) -> Value {
        Value::abs(if b { -1 } else { 0 })
    }
}
