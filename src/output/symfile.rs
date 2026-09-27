//! Symbol files in the Psy-Q format, as read by debuggers and emulators.
//!
//! The file starts with the signature `MND`, a version byte and a 32-bit
//! field that is zero. Each symbol follows as its value (32 bits, little
//! endian), a type byte, the length of its name and the name itself.

use crate::asm::OutSymbol;
use crate::symbol::SymKind;

const TYPE_LABEL: u8 = 2;

/// Writes the labels of a program. Names are folded to lower case unless
/// the program was assembled with case sensitive names.
pub fn write(symbols: &[OutSymbol], case_sensitive: bool, with_locals: bool) -> Vec<u8> {
    let mut out = b"MND\x01\0\0\0\0".to_vec();
    let mut labels: Vec<&OutSymbol> = symbols
        .iter()
        .filter(|s| s.kind == SymKind::Label && s.section.is_none() && (!s.local || with_locals))
        .collect();
    // In order of address, and of definition where addresses are equal.
    labels.sort_by_key(|s| s.value);
    for s in labels {
        let name = if case_sensitive { s.name.clone() } else { s.name.to_ascii_lowercase() };
        let name = crate::source::encode(&name);
        let name = &name[..name.len().min(255)];
        out.extend_from_slice(&s.value.to_le_bytes());
        out.push(TYPE_LABEL);
        out.push(name.len() as u8);
        out.extend_from_slice(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, value: u32, kind: SymKind, local: bool) -> OutSymbol {
        OutSymbol { name: name.into(), value, section: None, exported: false, local, kind, id: 0 }
    }

    #[test]
    fn writes_labels_only() {
        let syms = [
            sym("@loop", 0x204, SymKind::Label, true),
            sym("Start", 0x200, SymKind::Label, false),
            sym("size", 4, SymKind::Equ, false),
        ];
        let out = write(&syms, false, false);
        assert_eq!(out, b"MND\x01\0\0\0\0\x00\x02\0\0\x02\x05start");
        let out = write(&syms, true, true);
        assert_eq!(out, b"MND\x01\0\0\0\0\x00\x02\0\0\x02\x05Start\x04\x02\0\0\x02\x05@loop");
    }
}
