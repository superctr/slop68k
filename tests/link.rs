//! Links object files from the assembler with code compiled by GCC.
//!
//! The same program is written for this assembler and for the GNU
//! assembler. Both are linked with the same C code, and what comes out has
//! to be the same.
//!
//! This needs a GCC toolchain for `m68k-elf`. It is looked for in the
//! `PATH` and in `/opt/toolchains/m68k-elf/bin`; set `M68K_ELF` to the
//! prefix of its programs, such as `/usr/bin/m68k-elf-`, if it is somewhere
//! else. Without a toolchain the tests pass without doing anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The prefix of the programs of the toolchain, if there is one.
fn toolchain() -> Option<String> {
    let works = |prefix: &str| {
        Command::new(format!("{}gcc", prefix)).arg("--version").output().is_ok_and(|o| o.status.success())
    };
    if let Ok(prefix) = std::env::var("M68K_ELF") {
        assert!(works(&prefix), "M68K_ELF is set to '{}', but {}gcc does not run", prefix, prefix);
        return Some(prefix);
    }
    ["m68k-elf-", "/opt/toolchains/m68k-elf/bin/m68k-elf-"]
        .into_iter()
        .find(|p| works(p))
        .map(String::from)
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap_or_else(|e| panic!("{:?}: {}", cmd, e));
    assert!(
        out.status.success(),
        "{:?}\n{}{}",
        cmd,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Build {
    prefix: String,
    src: PathBuf,
    out: PathBuf,
}

impl Build {
    fn new(name: &str) -> Option<Build> {
        let Some(prefix) = toolchain() else {
            eprintln!("no m68k-elf toolchain, skipping");
            return None;
        };
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("link");
        let out = std::env::temp_dir().join(format!("slop68k-link-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&out);
        fs::create_dir_all(&out).unwrap();
        Some(Build { prefix, src, out })
    }

    fn tool(&self, name: &str) -> Command {
        Command::new(format!("{}{}", self.prefix, name))
    }

    fn compile(&self, name: &str) -> PathBuf {
        let object = self.out.join(format!("{}.o", name));
        run(self
            .tool("gcc")
            .args(["-m68000", "-O1", "-ffreestanding", "-fno-builtin", "-c"])
            .arg(self.src.join(format!("{}.c", name)))
            .arg("-o")
            .arg(&object));
        object
    }

    /// Assembles with the assembler under test.
    fn assemble(&self, name: &str) -> PathBuf {
        let object = self.out.join(format!("{}.o", name));
        run(Command::new(env!("CARGO_BIN_EXE_slop68k"))
            .arg("-o")
            .arg(&object)
            .arg(self.src.join(format!("{}.68k", name))));
        object
    }

    /// Assembles with the GNU assembler.
    fn assemble_gnu(&self, name: &str) -> PathBuf {
        let object = self.out.join(format!("{}.gnu.o", name));
        run(self.tool("as").arg("-m68000").arg(self.src.join(format!("{}.s", name))).arg("-o").arg(&object));
        object
    }

    /// Links objects and returns the program as a memory image.
    fn link(&self, name: &str, objects: &[&PathBuf]) -> Vec<u8> {
        let elf = self.out.join(format!("{}.elf", name));
        let bin = self.out.join(format!("{}.bin", name));
        run(self
            .tool("ld")
            .arg("-T")
            .arg(self.src.join("link.ld"))
            .args(["--defsym", "small_const=0x42", "-o"])
            .arg(&elf)
            .args(objects));
        run(self.tool("objcopy").args(["-O", "binary"]).arg(&elf).arg(&bin));
        fs::read(bin).unwrap()
    }

    /// Code and relocations of an object file, as a disassembly.
    fn disassemble(&self, object: &Path) -> String {
        let out = self.tool("objdump").arg("-dr").arg(object).output().unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        // Leave out the name of the file, and the names of labels, of
        // which the GNU assembler keeps more.
        text.lines()
            .skip(3)
            .filter(|l| !l.is_empty() && !l.ends_with(">:"))
            .map(|l| match l.find('<') {
                Some(n) if !l.contains("R_68K") => l[..n].trim_end(),
                _ => l,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for Build {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.out);
    }
}

#[test]
fn links_with_c() {
    let Some(b) = Build::new("c") else { return };
    let c = b.compile("main");
    let ours = b.assemble("mod");
    let gnu = b.assemble_gnu("mod");
    assert_eq!(b.disassemble(&ours), b.disassemble(&gnu));

    let ours = b.link("ours", &[&ours, &c]);
    let gnu = b.link("gnu", &[&gnu, &c]);
    assert!(!ours.is_empty());
    assert_eq!(ours, gnu);
    // The program starts by setting the stack pointer to the top of the
    // stack, at the end of the memory the program uses.
    assert_eq!(&ours[..2], [0x4f, 0xf9]);
    assert_eq!(&ours[2..4], [0x00, 0xff]);
}

#[test]
fn links_short_references() {
    let Some(b) = Build::new("short") else { return };
    let c = b.compile("main");
    let (mod_ours, mod_gnu) = (b.assemble("mod"), b.assemble_gnu("mod"));
    let (ours, gnu) = (b.assemble("short"), b.assemble_gnu("short"));
    assert_eq!(b.disassemble(&ours), b.disassemble(&gnu));

    let ours = b.link("ours", &[&mod_ours, &ours, &c]);
    let gnu = b.link("gnu", &[&mod_gnu, &gnu, &c]);
    assert_eq!(ours, gnu);
}
