//! Checks the ELF object files the assembler writes, by reading them back.

use std::path::Path;

use slop68k::cli::{self, Parsed};
use slop68k::output::elf;

fn assemble(name: &str) -> Vec<u8> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("elf").join(name);
    let args = vec!["/l".to_string(), format!("{},out.o", source.display())];
    let Ok(Parsed::Run(cmd)) = cli::parse(&args) else { panic!("bad arguments") };
    assert!(cmd.config.relocatable);
    let asm = slop68k::assemble(&cmd.config);
    let messages: Vec<String> = asm.messages.iter().map(|m| m.to_string()).collect();
    assert!(asm.ok(), "{}", messages.join("\n"));
    elf::write(&asm)
}

struct Section {
    name: String,
    kind: u32,
    flags: u32,
    offset: usize,
    size: usize,
    link: u32,
    info: u32,
    align: u32,
}

#[derive(Debug, PartialEq)]
struct Symbol {
    name: String,
    value: u32,
    global: bool,
    /// Name of the section, "ABS" or "UND".
    place: String,
}

#[derive(Debug, PartialEq)]
struct Reloc {
    offset: u32,
    kind: &'static str,
    symbol: String,
    addend: i32,
}

struct Object {
    data: Vec<u8>,
    sections: Vec<Section>,
    symbols: Vec<Symbol>,
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([d[at], d[at + 1]])
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn string_at(d: &[u8], at: usize) -> String {
    let end = d[at..].iter().position(|&b| b == 0).unwrap();
    String::from_utf8(d[at..at + end].to_vec()).unwrap()
}

impl Object {
    fn parse(data: Vec<u8>) -> Object {
        let d = &data;
        assert_eq!(&d[..4], b"\x7fELF");
        assert_eq!(d[4], 1, "32 bit");
        assert_eq!(d[5], 2, "big endian");
        assert_eq!(u16_at(d, 16), 1, "relocatable");
        assert_eq!(u16_at(d, 18), 4, "68000");
        assert_eq!(u32_at(d, 36), 0x0100_0000, "flags");
        assert_eq!(u16_at(d, 46), 40, "size of section headers");
        let shoff = u32_at(d, 32) as usize;
        let shnum = u16_at(d, 48) as usize;
        let shstrndx = u16_at(d, 50) as usize;

        let header = |n: usize| shoff + n * 40;
        let strings = u32_at(d, header(shstrndx) + 16) as usize;
        let sections: Vec<Section> = (0..shnum)
            .map(|n| {
                let h = header(n);
                Section {
                    name: string_at(d, strings + u32_at(d, h) as usize),
                    kind: u32_at(d, h + 4),
                    flags: u32_at(d, h + 8),
                    offset: u32_at(d, h + 16) as usize,
                    size: u32_at(d, h + 20) as usize,
                    link: u32_at(d, h + 24),
                    info: u32_at(d, h + 28),
                    align: u32_at(d, h + 32),
                }
            })
            .collect();

        let symtab = sections.iter().find(|s| s.kind == 2).expect("a symbol table");
        let names = sections[symtab.link as usize].offset;
        let count = symtab.size / 16;
        let mut symbols = Vec::new();
        for n in 0..count {
            let at = symtab.offset + n * 16;
            let info = d[at + 12];
            let shndx = u16_at(d, at + 14);
            let place = match shndx {
                0 => "UND".to_string(),
                0xfff1 => "ABS".to_string(),
                n => sections[n as usize].name.clone(),
            };
            let mut name = string_at(d, names + u32_at(d, at) as usize);
            if info & 15 == 3 {
                name = place.clone();
            }
            let global = info >> 4 == 1;
            // Local symbols come before global ones.
            assert_eq!(global, n >= symtab.info as usize, "symbol {}", name);
            symbols.push(Symbol { name, value: u32_at(d, at + 4), global, place });
        }
        Object { data, sections, symbols }
    }

    fn section(&self, name: &str) -> &Section {
        self.sections.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no section {}", name))
    }

    fn contents(&self, name: &str) -> &[u8] {
        let s = self.section(name);
        &self.data[s.offset..s.offset + s.size]
    }

    fn symbol(&self, name: &str) -> &Symbol {
        self.symbols.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no symbol {}", name))
    }

    fn relocs(&self, name: &str) -> Vec<Reloc> {
        let s = self.section(&format!(".rela{}", name));
        assert_eq!(s.kind, 4);
        assert_eq!(self.sections[s.info as usize].name, name);
        assert_eq!(self.sections[s.link as usize].name, ".symtab");
        (0..s.size / 12)
            .map(|n| {
                let at = s.offset + n * 12;
                let info = u32_at(&self.data, at + 4);
                Reloc {
                    offset: u32_at(&self.data, at),
                    kind: match info & 0xff {
                        1 => "32",
                        2 => "16",
                        3 => "8",
                        4 => "PC32",
                        5 => "PC16",
                        6 => "PC8",
                        _ => "?",
                    },
                    symbol: self.symbols[(info >> 8) as usize].name.clone(),
                    addend: u32_at(&self.data, at + 8) as i32,
                }
            })
            .collect()
    }
}

fn reloc(offset: u32, kind: &'static str, symbol: &str, addend: i32) -> Reloc {
    Reloc { offset, kind, symbol: symbol.to_string(), addend }
}

#[test]
fn sections() {
    let o = Object::parse(assemble("object.68k"));
    let text = o.section(".text");
    assert_eq!((text.kind, text.flags, text.align), (1, 2 | 4, 2));
    let data = o.section(".data");
    assert_eq!((data.kind, data.flags), (1, 1 | 2));
    assert_eq!(data.size, 20);
    let bss = o.section(".bss");
    assert_eq!((bss.kind, bss.flags, bss.size), (8, 1 | 2, 16));

    // References within a section are resolved by the assembler.
    let code = o.contents(".text");
    assert_eq!(&code[..2], [0x70, 0x01]);
    assert_eq!(&code[0x28..0x2c], [0x61, 0x10, 0x61, 0x00], "bsr.s and bsr to a local label");
    assert_eq!(&code[0x2c..0x32], [0x00, 0x0e, 0x60, 0x00, 0x00, 0x0c]);
    // Fields that are relocated are left at zero.
    assert_eq!(&code[2..8], [0x41, 0xf9, 0, 0, 0, 0]);
    // The difference of two labels in a section is a constant.
    assert_eq!(&o.contents(".data")[16..], [0xff, 0xf0, 0x12, 0x34]);
}

#[test]
fn symbols() {
    let o = Object::parse(assemble("object.68k"));
    let symbol = |name: &str, value, global, place: &str| Symbol {
        name: name.to_string(),
        value,
        global,
        place: place.to_string(),
    };
    assert_eq!(*o.symbol("start"), symbol("start", 0, true, ".text"));
    assert_eq!(*o.symbol("table"), symbol("table", 0, true, ".data"));
    assert_eq!(*o.symbol("CONST"), symbol("CONST", 0x1234, true, "ABS"));
    assert_eq!(*o.symbol("printf"), symbol("printf", 0, true, "UND"));
    assert_eq!(*o.symbol("extvar"), symbol("extvar", 0, true, "UND"));
    assert_eq!(*o.symbol("local1"), symbol("local1", 0x3a, false, ".text"));
    assert_eq!(*o.symbol("helper"), symbol("helper", 0x3c, false, ".text"));
    assert_eq!(*o.symbol("buffer"), symbol("buffer", 0, false, ".bss"));
    // Labels with a scope of their own are left out.
    assert!(!o.symbols.iter().any(|s| s.name.contains("loop")));
    // Every section has a symbol for relocations to refer to.
    for name in [".text", ".data", ".bss"] {
        assert!(!o.symbol(name).global);
    }
}

#[test]
fn relocations() {
    let o = Object::parse(assemble("object.68k"));
    assert_eq!(
        o.relocs(".text"),
        [
            reloc(0x04, "32", ".data", 0),
            reloc(0x0a, "PC16", ".data", 0),
            reloc(0x0e, "32", ".bss", 0),
            reloc(0x14, "32", "extvar", 0),
            reloc(0x1a, "32", "extvar", 4),
            reloc(0x20, "32", "printf", 0),
            reloc(0x26, "PC16", "printf", 0),
            reloc(0x34, "16", "extvar", 0),
            reloc(0x3e, "32", ".bss", 0),
        ]
    );
    assert_eq!(
        o.relocs(".data"),
        [
            reloc(0x00, "32", ".text", 0),
            reloc(0x04, "32", ".text", 0x3a),
            reloc(0x08, "32", "printf", 0),
            reloc(0x0c, "32", ".bss", 8),
        ]
    );
}

#[test]
fn short_relocations() {
    let o = Object::parse(assemble("short.68k"));
    // Displacements are relative to the extension word, not to the byte
    // that holds them, which the addend makes up for.
    assert_eq!(
        o.relocs(".text"),
        [
            reloc(0x01, "PC8", "extfn", -1),
            reloc(0x04, "PC16", "extfn", 0),
            reloc(0x09, "PC8", "extfn", 1),
            reloc(0x0d, "8", "ext8", 0),
            reloc(0x10, "16", "ext8", 0),
            reloc(0x15, "8", "ext8", 0),
            reloc(0x16, "8", "ext8", 0),
        ]
    );
    assert_eq!(o.symbol("entry").place, ".text");
    assert!(o.symbol("entry").global);
}
