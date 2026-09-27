//! Compares the output of the assembler with that of the original asm68k.
//!
//! The cases are the source files in `tests/cases`. The object code,
//! listing and messages that asm68k produced for them are kept alongside,
//! and are regenerated with `tests/regen.sh`.

use std::fs;
use std::path::{Path, PathBuf};

use slop68k::cli::{self, Parsed};
use slop68k::diag::Severity;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("cases")
}

/// The messages of an assembly in the form they are kept in: those that
/// refer to a line, with the file named without its directory.
fn messages(asm: &slop68k::Assembly) -> String {
    let mut out = String::new();
    for m in &asm.messages {
        let kind = match m.severity {
            Severity::Info => continue,
            Severity::Warning => "Warning",
            Severity::Error => "Error",
            Severity::Fatal => "Fatal",
        };
        let Some((file, line)) = &m.location else { continue };
        let file = Path::new(file).file_name().unwrap().to_string_lossy().to_lowercase();
        out.push_str(&format!("{}({}) : {} : {}\n", file, line, kind, m.text));
    }
    out
}

/// Describes where two files differ, for the failure message.
fn first_difference(ours: &[u8], theirs: &[u8]) -> String {
    let at = ours.iter().zip(theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
    let line = ours[..at].iter().filter(|&&b| b == b'\n').count() + 1;
    let context = |data: &[u8]| {
        let start = data[..at.min(data.len())].iter().rposition(|&b| b == b'\n').map_or(0, |n| n + 1);
        let end = data[start..].iter().position(|&b| b == b'\n').map_or(data.len(), |n| start + n);
        String::from_utf8_lossy(&data[start..end]).trim_end().to_string()
    };
    format!(
        "sizes {} and {}, first difference at offset {:#x} (line {})\n  ours:   {}\n  asm68k: {}",
        ours.len(),
        theirs.len(),
        at,
        line,
        context(ours),
        context(theirs)
    )
}

fn run(name: &str) {
    let dir = cases_dir();
    let source = dir.join(format!("{}.68k", name));
    let text = fs::read_to_string(&source).unwrap();
    let switches = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("; switches: "))
        .unwrap_or_else(|| panic!("{}: no switches on the first line", name));

    let mut args: Vec<String> = switches.split_whitespace().map(String::from).collect();
    args.push(format!("{},out.bin,,out.lst", source.display()));
    let cmd = match cli::parse(&args).unwrap() {
        Parsed::Run(cmd) => cmd,
        _ => panic!("{}: bad switches", name),
    };
    let asm = slop68k::assemble(&cmd.config);

    let expected = |ext: &str| fs::read(dir.join(format!("{}.{}", name, ext))).ok();

    let theirs = expected("msg").map(|m| String::from_utf8(m).unwrap()).unwrap_or_default();
    assert_eq!(messages(&asm), theirs, "{}: messages differ", name);

    match expected("bin") {
        Some(theirs) => {
            assert!(asm.ok(), "{}: errors:\n{}", name, messages(&asm));
            let ours = asm.image().1;
            assert!(ours == theirs, "{}: object code differs: {}", name, first_difference(&ours, &theirs));
        }
        None => assert!(!asm.ok(), "{}: asm68k finds errors in this", name),
    }
    if let Some(theirs) = expected("lst") {
        let ours = asm.listing.expect("a listing");
        assert!(ours == theirs, "{}: listing differs: {}", name, first_difference(&ours, &theirs));
    }
}

macro_rules! cases {
    ($($name:ident)*) => {
        $(
            #[test]
            fn $name() {
                run(stringify!($name));
            }
        )*

        /// Every source file in the directory has to be listed here.
        #[test]
        fn all_cases_are_run() {
            let mut listed = vec![$(stringify!($name).to_string()),*];
            listed.sort();
            let mut found: Vec<String> = fs::read_dir(cases_dir())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "68k"))
                .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
                .collect();
            found.sort();
            assert_eq!(listed, found);
        }
    };
}

cases! {
    blocks
    conditionals
    directives
    errors
    expressions
    instructions
    listing
    listing_macros
    macros
    numbers
    optimise
    options
    scope
    sections
    structure
}
