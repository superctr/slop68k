//! The command line.
//!
//! There are two ways of writing it, which can be mixed. The command line
//! of asm68k is accepted as it is, so that the assembler can take its place
//! in existing build scripts:
//!
//! ```text
//! slop68k /k /p /o ae- source,object,symbols,listing
//! ```
//!
//! Switches that start with a slash are those of asm68k. Options that start
//! with a dash are of the kind other assemblers and compilers take, which
//! is of more use in a makefile:
//!
//! ```text
//! slop68k -I include -D DEBUG=1 -o build/main.o src/main.68k
//! ```

use std::path::{Path, PathBuf};

use crate::asm::Config;
use crate::options::Options;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Memory image.
    Binary,
    /// Motorola S-records.
    SRecord,
    /// ELF relocatable object file.
    Elf,
}

impl Format {
    fn from_name(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "bin" | "binary" => Some(Format::Binary),
            "srec" | "s19" | "s28" | "s37" => Some(Format::SRecord),
            "elf" | "o" => Some(Format::Elf),
            _ => None,
        }
    }

    /// The format that goes with the name of an output file.
    fn from_path(path: &Path) -> Format {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        match ext.as_deref() {
            Some("o" | "obj" | "elf") => Format::Elf,
            Some("s19" | "s28" | "s37" | "srec" | "mot") => Format::SRecord,
            _ => Format::Binary,
        }
    }
}

#[derive(Debug)]
pub struct Command {
    pub config: Config,
    pub format: Format,
    pub object: Option<PathBuf>,
    pub symbols: Option<PathBuf>,
    pub listing: Option<PathBuf>,
    /// Print nothing but warnings and errors.
    pub quiet: bool,
}

#[derive(Debug)]
pub enum Parsed<T> {
    Run(T),
    Help,
    Version,
}

pub const USAGE: &str = "\
Usage: slop68k [options] source
       slop68k [switches] source,object,symbols,listing

Options:
  -o, --output FILE     Write the object code to FILE
  -f, --format FORMAT   Format of the object code: bin, srec or elf.
                        The default goes by the name of the output file:
                        elf for .o, srec for .s19, .s28, .s37 and .srec,
                        bin for anything else
  -l, --listing FILE    Write a listing to FILE
  -s, --symbols FILE    Write a symbol file to FILE
  -I, --include DIR     Add a directory to search for INCLUDE and INCBIN
  -D, --define N[=X]    Define the symbol N with the value X, or 1
  -O, --options LIST    Set assembler options, as with OPT: -O ae-,c+
  -m, --list-macros     List the lines of macro expansions
      --list-skipped    List code skipped by conditional assembly
      --local-time      Use local time, not UTC, for the date and time
  -q, --quiet           Print nothing but warnings and errors
  -h, --help            Print this
  -V, --version         Print the version

Switches of asm68k:
  /c            List code skipped by conditional assembly
  /e n=x        Define the symbol n with the value x
  /j path       Add a directory to search for INCLUDE and INCBIN files
  /k            Accept IFEQ, IFND and the like (always on)
  /l            Produce a relocatable object file (ELF)
  /m            List the lines of macro expansions
  /o options    Set assembler options, as with OPT: /o ae-,ow+
  /p            Produce a pure binary file
  /ps           Produce Motorola S-records

The environment variable SOURCE_DATE_EPOCH sets the time of assembly, in
seconds since 1970, for builds that have to be reproducible.";

/// The name of a switch of asm68k, if the argument is one. Paths may start
/// with a slash too, so only known switches count.
fn switch_name(arg: &str) -> Option<&str> {
    let name = arg.strip_prefix('/')?;
    let lower = name.to_ascii_lowercase();
    let known = ["c", "d", "e", "g", "j", "k", "l", "m", "o", "p", "ps", "w", "z", "zd", "?"];
    if known.contains(&lower.as_str()) {
        return Some(name);
    }
    // Switches that take their argument without a space: /oae-, /eN=1.
    let first = lower.chars().next()?;
    if matches!(first, 'o' | 'e') && name.contains(['+', '-', '=']) && !name.contains(['/', '\\']) {
        return Some(name);
    }
    None
}

#[derive(Default)]
struct Args {
    opts: Options,
    include_dirs: Vec<PathBuf>,
    defines: Vec<(String, i32)>,
    format: Option<Format>,
    list_macros: bool,
    list_skipped: bool,
    local_time: bool,
    quiet: bool,
    object: Option<PathBuf>,
    symbols: Option<PathBuf>,
    listing: Option<PathBuf>,
    files: Vec<String>,
}

impl Args {
    fn options(&mut self, list: &str) -> Result<(), String> {
        self.opts.apply(list).map_err(|e| format!("{} in '{}'", e, list))
    }

    /// Handles a switch of asm68k.
    fn switch(&mut self, arg: &str, name: &str, value: &mut dyn FnMut(&str) -> Result<String, String>) -> Result<Option<Parsed<Command>>, String> {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "?" => return Ok(Some(Parsed::Help)),
            "c" => self.list_skipped = true,
            "m" => self.list_macros = true,
            "p" => self.format = Some(Format::Binary),
            "ps" => self.format = Some(Format::SRecord),
            "l" => self.format = Some(Format::Elf),
            // Switches that concern the debugger and the Psy-Q linker.
            "d" | "g" | "k" | "w" | "z" | "zd" => {}
            "o" => self.options(&value("options")?)?,
            "j" => self.include_dirs.push(crate::source::native_path(&value("a path")?)),
            "e" => self.defines.push(parse_define(&value("a symbol")?)?),
            _ => {
                let (switch, rest) = name.split_at(1);
                match switch.to_ascii_lowercase().as_str() {
                    "o" => self.options(rest)?,
                    "e" => self.defines.push(parse_define(rest)?),
                    _ => return Err(format!("Unknown switch '{}'", arg)),
                }
            }
        }
        Ok(None)
    }

    /// Handles an option, given its name and the value that is part of the
    /// same argument, if there is one.
    fn option(
        &mut self,
        arg: &str,
        name: &str,
        attached: Option<&str>,
        next: &mut dyn FnMut(&str) -> Result<String, String>,
    ) -> Result<Option<Parsed<Command>>, String> {
        let mut value = |what: &str| match attached {
            Some(v) => Ok(v.to_string()),
            None => next(what),
        };
        let flag = |attached: Option<&str>| match attached {
            Some(_) => Err(format!("Option '{}' does not take a value", name)),
            None => Ok(()),
        };
        match name {
            "o" | "output" => self.object = Some(PathBuf::from(value("a file name")?)),
            "l" | "listing" => self.listing = Some(PathBuf::from(value("a file name")?)),
            "s" | "symbols" => self.symbols = Some(PathBuf::from(value("a file name")?)),
            "f" | "format" => {
                let v = value("a format")?;
                self.format = Some(Format::from_name(&v).ok_or_else(|| format!("Unknown output format '{}'", v))?);
            }
            "I" | "include" => self.include_dirs.push(PathBuf::from(value("a directory")?)),
            "D" | "define" => self.defines.push(parse_define(&value("a symbol")?)?),
            "O" | "options" => self.options(&value("options")?)?,
            "m" | "list-macros" => {
                flag(attached)?;
                self.list_macros = true;
            }
            "list-skipped" => {
                flag(attached)?;
                self.list_skipped = true;
            }
            "local-time" => {
                flag(attached)?;
                self.local_time = true;
            }
            "q" | "quiet" => {
                flag(attached)?;
                self.quiet = true;
            }
            "h" | "help" => return Ok(Some(Parsed::Help)),
            "V" | "version" => return Ok(Some(Parsed::Version)),
            _ => return Err(format!("Unknown option '{}'", arg)),
        }
        Ok(None)
    }
}

pub fn parse(args: &[String]) -> Result<Parsed<Command>, String> {
    let mut a = Args::default();
    let mut it = args.iter();
    let mut only_files = false;
    while let Some(arg) = it.next() {
        let mut next = |what: &str| -> Result<String, String> {
            it.next().cloned().ok_or_else(|| format!("Expecting {} after {}", what, arg))
        };
        let done = if only_files {
            a.files.push(arg.clone());
            None
        } else if arg == "--" {
            only_files = true;
            None
        } else if let Some(long) = arg.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (long, None),
            };
            a.option(arg, name, attached, &mut next)?
        } else if arg.len() > 1 && arg.starts_with('-') {
            // The value may follow the letter directly, as in -Iinclude.
            let name = &arg[1..2];
            let attached = Some(&arg[2..]).filter(|v| !v.is_empty());
            a.option(arg, name, attached, &mut next)?
        } else if let Some(name) = switch_name(arg) {
            a.switch(arg, name, &mut next)?
        } else {
            a.files.push(arg.clone());
            None
        };
        if let Some(done) = done {
            return Ok(done);
        }
    }

    // What is left names the files, as asm68k has them: the source, the
    // object code, the symbols, the listing and a temporary file, with
    // commas in between and with or without spaces.
    if a.files.len() > 1 && !a.files.iter().any(|f| f.contains(',')) {
        return Err(format!("More than one source file: '{}'", a.files.join("' '")));
    }
    let files = a.files.join(" ");
    let mut specs = files.split(',').map(|s| s.trim());
    let source = specs.next().unwrap_or("");
    if source.is_empty() {
        return Err("No source file specified".into());
    }
    let mut spec = |slot: &mut Option<PathBuf>| {
        if let Some(name) = specs.next().filter(|s| !s.is_empty()) {
            if slot.is_none() {
                *slot = Some(crate::source::native_path(name));
            }
        }
    };
    spec(&mut a.object);
    spec(&mut a.symbols);
    spec(&mut a.listing);

    let format = match (a.format, &a.object) {
        (Some(f), _) => f,
        (None, Some(path)) => Format::from_path(path),
        (None, None) => Format::Binary,
    };
    let mut config = Config::new(source);
    config.include_dirs = a.include_dirs;
    config.defines = a.defines;
    config.options = a.opts;
    config.relocatable = format == Format::Elf;
    config.listing = a.listing.is_some();
    config.list_macros = a.list_macros;
    config.list_skipped = a.list_skipped;
    config.local_time = a.local_time;
    Ok(Parsed::Run(Command {
        config,
        format,
        object: a.object,
        symbols: a.symbols,
        listing: a.listing,
        quiet: a.quiet,
    }))
}

fn parse_define(s: &str) -> Result<(String, i32), String> {
    let (name, value) = match s.split_once('=') {
        Some((n, v)) => (n.trim(), v.trim()),
        None => (s.trim(), "1"),
    };
    if name.is_empty() {
        return Err(format!("Bad symbol definition '{}'", s));
    }
    let (digits, radix) = match value.strip_prefix('$').or_else(|| value.strip_prefix("0x")) {
        Some(h) => (h, 16),
        None => (value, 10),
    };
    let (digits, negative) = match digits.strip_prefix('-') {
        Some(d) => (d, true),
        None => (digits, false),
    };
    let n = i64::from_str_radix(digits, radix).map_err(|_| format!("Bad value in symbol definition '{}'", s))?;
    let n = if negative { -n } else { n };
    Ok((name.to_string(), n as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn run(a: &[&str]) -> Command {
        match parse(&args(a)).unwrap() {
            Parsed::Run(c) => c,
            p => panic!("unexpected {:?}", p),
        }
    }

    #[test]
    fn asm68k_command_line() {
        // As the shell passes: asm68k /k /p /o ae- src\main.68k, main.bin , , out\main.lst
        let c = run(&["/k", "/p", "/o", "ae-", "src\\main.68k,", "main.bin", ",", ",", "out\\main.lst"]);
        assert_eq!(c.config.source, "src\\main.68k");
        assert_eq!(c.object, Some(crate::source::native_path("main.bin")));
        assert_eq!(c.symbols, None);
        assert_eq!(c.listing, Some(crate::source::native_path("out\\main.lst")));
        assert!(!c.config.options.auto_even);
        assert!(c.config.listing);
        assert_eq!(c.format, Format::Binary);
    }

    #[test]
    fn single_argument_file_list() {
        let c = run(&["/p", "build.68k,main.bin,out/main.sym,out/build.lst"]);
        assert_eq!(c.config.source, "build.68k");
        assert_eq!(c.symbols, Some(PathBuf::from("out/main.sym")));
        assert_eq!(c.listing, Some(PathBuf::from("out/build.lst")));
    }

    #[test]
    fn asm68k_switches() {
        let c = run(&["/m", "/c", "/e", "DEBUG=1", "/eTEST=2", "/j", "inc", "/oow+,oz+", "a.68k"]);
        assert!(c.config.list_macros);
        assert!(c.config.list_skipped);
        assert_eq!(c.config.defines, [("DEBUG".to_string(), 1), ("TEST".to_string(), 2)]);
        assert_eq!(c.config.include_dirs, [PathBuf::from("inc")]);
        assert!(c.config.options.opt_abs_word);
        assert!(c.config.options.opt_zero_disp);
        assert_eq!(c.object, None);
    }

    #[test]
    fn options() {
        let c = run(&["-I", "include", "-D", "DEBUG=1", "-O", "ae-,c+", "-o", "build/main.o", "src/main.68k"]);
        assert_eq!(c.config.source, "src/main.68k");
        assert_eq!(c.object, Some(PathBuf::from("build/main.o")));
        assert_eq!(c.config.include_dirs, [PathBuf::from("include")]);
        assert_eq!(c.config.defines, [("DEBUG".to_string(), 1)]);
        assert!(c.config.options.case_sensitive);
        assert!(!c.config.options.auto_even);
        assert_eq!(c.format, Format::Elf);
        assert!(c.config.relocatable);
    }

    #[test]
    fn options_in_other_forms() {
        // Values attached to their options, and options after the source.
        let c = run(&["main.68k", "-Iinc", "-DNAME", "-DX=$10", "-omain.bin", "-lmain.lst", "-m", "-q"]);
        assert_eq!(c.config.source, "main.68k");
        assert_eq!(c.config.include_dirs, [PathBuf::from("inc")]);
        assert_eq!(c.config.defines, [("NAME".to_string(), 1), ("X".to_string(), 16)]);
        assert_eq!(c.object, Some(PathBuf::from("main.bin")));
        assert_eq!(c.listing, Some(PathBuf::from("main.lst")));
        assert!(c.config.listing);
        assert!(c.config.list_macros);
        assert!(c.quiet);
        assert_eq!(c.format, Format::Binary);

        let c = run(&["--output=a.bin", "--symbols", "a.sym", "--listing=a.lst", "--include=inc", "--define", "N=2", "--options=ow+", "--list-skipped", "a.68k"]);
        assert_eq!(c.object, Some(PathBuf::from("a.bin")));
        assert_eq!(c.symbols, Some(PathBuf::from("a.sym")));
        assert_eq!(c.listing, Some(PathBuf::from("a.lst")));
        assert_eq!(c.config.include_dirs, [PathBuf::from("inc")]);
        assert_eq!(c.config.defines, [("N".to_string(), 2)]);
        assert!(c.config.options.opt_abs_word);
        assert!(c.config.list_skipped);
    }

    #[test]
    fn time() {
        assert!(!run(&["a.68k"]).config.local_time);
        assert!(run(&["--local-time", "a.68k"]).config.local_time);
        assert!(run(&["/k", "/p", "--local-time", "a.68k,a.bin"]).config.local_time);
        assert!(parse(&args(&["--local-time=yes", "a.68k"])).is_err());
    }

    #[test]
    fn both_kinds_together() {
        let c = run(&["/k", "/o", "ae-", "-o", "out.bin", "-I", "inc", "main.68k"]);
        assert_eq!(c.config.source, "main.68k");
        assert_eq!(c.object, Some(PathBuf::from("out.bin")));
        assert!(!c.config.options.auto_even);
        // An option takes precedence over the list of files.
        let c = run(&["-o", "out.bin", "main.68k,other.bin,,main.lst"]);
        assert_eq!(c.object, Some(PathBuf::from("out.bin")));
        assert_eq!(c.listing, Some(PathBuf::from("main.lst")));
    }

    #[test]
    fn absolute_paths_are_not_switches() {
        let c = run(&["/p", "/home/user/src/main.68k,/tmp/out.bin"]);
        assert_eq!(c.config.source, "/home/user/src/main.68k");
        assert_eq!(c.object, Some(PathBuf::from("/tmp/out.bin")));
        let c = run(&["-o", "/tmp/out.bin", "/home/user/src/main.68k"]);
        assert_eq!(c.config.source, "/home/user/src/main.68k");
    }

    #[test]
    fn names_that_look_like_options() {
        let c = run(&["-o", "out.bin", "--", "-odd.68k"]);
        assert_eq!(c.config.source, "-odd.68k");
        let c = run(&["my file.68k"]);
        assert_eq!(c.config.source, "my file.68k");
    }

    #[test]
    fn formats() {
        assert_eq!(run(&["/ps", "a.68k"]).format, Format::SRecord);
        assert_eq!(run(&["/l", "a.68k"]).format, Format::Elf);
        assert!(run(&["/l", "a.68k"]).config.relocatable);
        assert_eq!(run(&["--format", "elf", "a.68k"]).format, Format::Elf);
        assert_eq!(run(&["-f", "srec", "a.68k"]).format, Format::SRecord);
        assert_eq!(run(&["-felf", "-o", "a.bin", "a.68k"]).format, Format::Elf);
        // Without a format, the name of the output decides.
        assert_eq!(run(&["-o", "a.o", "a.68k"]).format, Format::Elf);
        assert_eq!(run(&["-o", "a.S37", "a.68k"]).format, Format::SRecord);
        assert_eq!(run(&["-o", "a.bin", "a.68k"]).format, Format::Binary);
        assert_eq!(run(&["-o", "a.md", "a.68k"]).format, Format::Binary);
        assert_eq!(run(&["a.68k,a.o"]).format, Format::Elf);
        assert_eq!(run(&["/p", "a.68k,a.o"]).format, Format::Binary);
        assert_eq!(run(&["a.68k"]).format, Format::Binary);
    }

    #[test]
    fn defines() {
        assert_eq!(parse_define("x=5").unwrap(), ("x".to_string(), 5));
        assert_eq!(parse_define("x=$ff").unwrap(), ("x".to_string(), 255));
        assert_eq!(parse_define("x=0x10").unwrap(), ("x".to_string(), 16));
        assert_eq!(parse_define("x=-2").unwrap(), ("x".to_string(), -2));
        assert_eq!(parse_define("x").unwrap(), ("x".to_string(), 1));
        assert!(parse_define("=1").is_err());
        assert!(parse_define("x=y").is_err());
    }

    #[test]
    fn errors_and_help() {
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["/p"])).is_err());
        assert!(parse(&args(&["-o", "a.bin"])).is_err());
        assert!(parse(&args(&["a.68k", "-o"])).is_err());
        assert!(parse(&args(&["a.68k", "b.68k"])).is_err());
        assert!(parse(&args(&["-x", "a.68k"])).is_err());
        assert!(parse(&args(&["--frob", "a.68k"])).is_err());
        assert!(parse(&args(&["--quiet=yes", "a.68k"])).is_err());
        assert!(parse(&args(&["-f", "coff", "a.68k"])).is_err());
        assert!(parse(&args(&["/o", "zz+", "a.68k"])).is_err());
        assert!(parse(&args(&["-O", "zz+", "a.68k"])).is_err());
        assert!(matches!(parse(&args(&["--help"])), Ok(Parsed::Help)));
        assert!(matches!(parse(&args(&["-h"])), Ok(Parsed::Help)));
        assert!(matches!(parse(&args(&["/?"])), Ok(Parsed::Help)));
        assert!(matches!(parse(&args(&["--version"])), Ok(Parsed::Version)));
        assert!(matches!(parse(&args(&["-V"])), Ok(Parsed::Version)));
    }
}
