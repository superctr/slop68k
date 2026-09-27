//! ELF relocatable object files, as used by GCC and the GNU linker.
//!
//! Each section of the program becomes a section of the object file. Values
//! that depend on where sections end up, or on symbols defined elsewhere,
//! become relocations. Labels are written to the symbol table; those made
//! known with XDEF, GLOBAL or PUBLIC are global, the rest are local to the
//! file.

use std::collections::HashMap;

use crate::asm::{Assembly, OutSymbol};
use crate::m68k::fixup::FixKind;
use crate::symbol::SymKind;
use crate::value::Base;

const EM_68K: u16 = 4;
/// The code is for the 68000 rather than for the 68020 and up.
const EF_M68K_M68000: u32 = 0x0100_0000;

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_RELA: u32 = 4;
const SHT_NOBITS: u32 = 8;

const SHF_WRITE: u32 = 1;
const SHF_ALLOC: u32 = 2;
const SHF_EXECINSTR: u32 = 4;
const SHF_INFO_LINK: u32 = 0x40;

const SHN_ABS: u16 = 0xfff1;

const STB_LOCAL: u8 = 0;
const STB_GLOBAL: u8 = 1;
const STT_NOTYPE: u8 = 0;
const STT_SECTION: u8 = 3;

const R_68K_32: u32 = 1;
const R_68K_16: u32 = 2;
const R_68K_8: u32 = 3;
const R_68K_PC16: u32 = 5;
const R_68K_PC8: u32 = 6;

/// The relocation type for a kind of value.
pub fn reloc_type(kind: FixKind) -> Option<u32> {
    Some(match kind {
        FixKind::Data32 => R_68K_32,
        FixKind::Data16 | FixKind::Disp16 | FixKind::Abs16 => R_68K_16,
        FixKind::Data8 | FixKind::Disp8 => R_68K_8,
        FixKind::Branch16 | FixKind::PcDisp16 => R_68K_PC16,
        FixKind::Branch8 | FixKind::PcDisp8 => R_68K_PC8,
        FixKind::Moveq | FixKind::Quick | FixKind::Trap => return None,
    })
}

/// Flags of a section, which follow from its name as they do for the
/// sections GCC produces.
fn section_flags(name: &str, bss: bool) -> (u32, u32) {
    let name = name.to_ascii_lowercase();
    let name = name.trim_start_matches('.');
    if bss || name.starts_with("bss") {
        (SHT_NOBITS, SHF_ALLOC | SHF_WRITE)
    } else if name.starts_with("rodata") {
        (SHT_PROGBITS, SHF_ALLOC)
    } else if name.starts_with("data") {
        (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE)
    } else {
        (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR)
    }
}

#[derive(Default)]
struct StringTable {
    data: Vec<u8>,
    index: HashMap<String, u32>,
}

impl StringTable {
    fn new() -> StringTable {
        StringTable { data: vec![0], index: HashMap::new() }
    }

    fn add(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        if let Some(&n) = self.index.get(s) {
            return n;
        }
        let n = self.data.len() as u32;
        self.data.extend(crate::source::encode(s));
        self.data.push(0);
        self.index.insert(s.to_string(), n);
        n
    }
}

struct Header {
    name: u32,
    kind: u32,
    flags: u32,
    offset: u32,
    size: u32,
    link: u32,
    info: u32,
    align: u32,
    entsize: u32,
}

struct Symbol {
    name: u32,
    value: u32,
    info: u8,
    shndx: u16,
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Pads to a multiple of `align`, which is a power of two.
fn pad(out: &mut Vec<u8>, align: usize) {
    while out.len() & (align - 1) != 0 {
        out.push(0);
    }
}

/// Writes the assembly as an ELF object file.
pub fn write(asm: &Assembly) -> Vec<u8> {
    // The first section of the assembly is the one for code placed with
    // ORG, which relocatable programs do not have.
    let sections = &asm.sections[1..];
    let shndx = |section: usize| section as u16;

    let mut shstrtab = StringTable::new();
    let mut strtab = StringTable::new();

    // Symbols: a section symbol for each section, then the local labels,
    // then the global ones.
    let mut symbols = vec![Symbol { name: 0, value: 0, info: 0, shndx: 0 }];
    for n in 0..sections.len() {
        symbols.push(Symbol {
            name: 0,
            value: 0,
            info: STB_LOCAL << 4 | STT_SECTION,
            shndx: shndx(n + 1),
        });
    }
    let is_global = |s: &OutSymbol| s.exported || s.kind == SymKind::Extern;
    let mut extern_index: HashMap<u32, u32> = HashMap::new();
    for global in [false, true] {
        for s in asm.symbols.iter().filter(|s| is_global(s) == global) {
            if s.local || (!global && s.section.is_none()) {
                // Local labels and constants are of no use to the linker.
                continue;
            }
            let shndx = match (s.kind, s.section) {
                (SymKind::Extern, _) => 0,
                (_, Some(n)) => shndx(n),
                (_, None) => SHN_ABS,
            };
            if s.kind == SymKind::Extern {
                extern_index.insert(s.id, symbols.len() as u32);
            }
            let binding = if global { STB_GLOBAL } else { STB_LOCAL };
            symbols.push(Symbol {
                name: strtab.add(&s.name),
                value: s.value,
                info: binding << 4 | STT_NOTYPE,
                shndx,
            });
        }
    }
    let first_global = symbols
        .iter()
        .position(|s| s.info >> 4 == STB_GLOBAL)
        .unwrap_or(symbols.len()) as u32;

    // Relocations, by section.
    let mut relocs: Vec<Vec<u8>> = vec![Vec::new(); sections.len()];
    let mut sorted: Vec<_> = asm.relocs.iter().collect();
    sorted.sort_by_key(|r| (r.section, r.offset));
    for r in sorted {
        let Some(kind) = reloc_type(r.kind) else { continue };
        let sym = match r.base {
            Base::Section(n) => n,
            Base::Extern(id) => extern_index.get(&id).copied().unwrap_or(0),
        };
        let out = &mut relocs[r.section - 1];
        put32(out, r.offset);
        put32(out, sym << 8 | kind);
        put32(out, r.addend as u32);
    }

    // Lay out the file: header, section contents, tables, section headers.
    let mut out = vec![0u8; 52];
    let mut headers = vec![Header { name: 0, kind: 0, flags: 0, offset: 0, size: 0, link: 0, info: 0, align: 0, entsize: 0 }];
    for s in sections {
        let (kind, flags) = section_flags(&s.name, s.bss);
        pad(&mut out, 4);
        let offset = out.len() as u32;
        let mut size = s.size;
        if kind != SHT_NOBITS {
            for c in &s.chunks {
                out.extend_from_slice(&c.data);
            }
            size = out.len() as u32 - offset;
        }
        headers.push(Header {
            name: shstrtab.add(&s.name),
            kind,
            flags,
            offset,
            size,
            link: 0,
            info: 0,
            align: s.align.max(1),
            entsize: 0,
        });
    }
    let symtab_index = (1 + sections.len() + relocs.iter().filter(|r| !r.is_empty()).count()) as u32;
    for (n, r) in relocs.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
        pad(&mut out, 4);
        let offset = out.len() as u32;
        out.extend_from_slice(r);
        headers.push(Header {
            name: shstrtab.add(&format!(".rela{}", sections[n].name)),
            kind: SHT_RELA,
            flags: SHF_INFO_LINK,
            offset,
            size: r.len() as u32,
            link: symtab_index,
            info: n as u32 + 1,
            align: 4,
            entsize: 12,
        });
    }
    pad(&mut out, 4);
    let offset = out.len() as u32;
    for s in &symbols {
        put32(&mut out, s.name);
        put32(&mut out, s.value);
        put32(&mut out, 0);
        out.push(s.info);
        out.push(0);
        put16(&mut out, s.shndx);
    }
    headers.push(Header {
        name: shstrtab.add(".symtab"),
        kind: SHT_SYMTAB,
        flags: 0,
        offset,
        size: symbols.len() as u32 * 16,
        link: symtab_index + 1,
        info: first_global,
        align: 4,
        entsize: 16,
    });
    let offset = out.len() as u32;
    out.extend_from_slice(&strtab.data);
    headers.push(Header {
        name: shstrtab.add(".strtab"),
        kind: SHT_STRTAB,
        flags: 0,
        offset,
        size: strtab.data.len() as u32,
        link: 0,
        info: 0,
        align: 1,
        entsize: 0,
    });
    let name = shstrtab.add(".shstrtab");
    let offset = out.len() as u32;
    out.extend_from_slice(&shstrtab.data);
    headers.push(Header {
        name,
        kind: SHT_STRTAB,
        flags: 0,
        offset,
        size: shstrtab.data.len() as u32,
        link: 0,
        info: 0,
        align: 1,
        entsize: 0,
    });

    pad(&mut out, 4);
    let shoff = out.len() as u32;
    for h in &headers {
        put32(&mut out, h.name);
        put32(&mut out, h.kind);
        put32(&mut out, h.flags);
        put32(&mut out, 0);
        put32(&mut out, h.offset);
        put32(&mut out, h.size);
        put32(&mut out, h.link);
        put32(&mut out, h.info);
        put32(&mut out, h.align);
        put32(&mut out, h.entsize);
    }

    let mut header = Vec::with_capacity(52);
    header.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    put16(&mut header, 1); // relocatable
    put16(&mut header, EM_68K);
    put32(&mut header, 1);
    put32(&mut header, 0); // entry
    put32(&mut header, 0); // program headers
    put32(&mut header, shoff);
    put32(&mut header, EF_M68K_M68000);
    put16(&mut header, 52);
    put16(&mut header, 0);
    put16(&mut header, 0);
    put16(&mut header, 40);
    put16(&mut header, headers.len() as u16);
    put16(&mut header, headers.len() as u16 - 1);
    out[..52].copy_from_slice(&header);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_follow_names() {
        assert_eq!(section_flags(".text", false), (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR));
        assert_eq!(section_flags("code", false), (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR));
        assert_eq!(section_flags(".data", false), (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE));
        assert_eq!(section_flags(".rodata.str", false), (SHT_PROGBITS, SHF_ALLOC));
        assert_eq!(section_flags(".bss", false), (SHT_NOBITS, SHF_ALLOC | SHF_WRITE));
        assert_eq!(section_flags("ram", true), (SHT_NOBITS, SHF_ALLOC | SHF_WRITE));
    }

    #[test]
    fn relocation_types() {
        assert_eq!(reloc_type(FixKind::Data32), Some(R_68K_32));
        assert_eq!(reloc_type(FixKind::Abs16), Some(R_68K_16));
        assert_eq!(reloc_type(FixKind::Branch16), Some(R_68K_PC16));
        assert_eq!(reloc_type(FixKind::Branch8), Some(R_68K_PC8));
        assert_eq!(reloc_type(FixKind::Moveq), None);
    }

    #[test]
    fn string_table_shares_strings() {
        let mut t = StringTable::new();
        assert_eq!(t.add(""), 0);
        assert_eq!(t.add("abc"), 1);
        assert_eq!(t.add("de"), 5);
        assert_eq!(t.add("abc"), 1);
        assert_eq!(t.data, b"\0abc\0de\0");
    }
}
