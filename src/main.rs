use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use slop68k::cli::{self, Format, Parsed};
use slop68k::diag::Severity;
use slop68k::output;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match cli::parse(&args) {
        Ok(Parsed::Run(cmd)) => cmd,
        Ok(Parsed::Help) => {
            println!("slop68k {}\n\n{}", env!("CARGO_PKG_VERSION"), cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Version) => {
            println!("slop68k {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("slop68k: {}\n\n{}", e, cli::USAGE);
            return ExitCode::from(2);
        }
    };

    let start = Instant::now();
    let asm = slop68k::assemble(&cmd.config);

    // Messages go to standard output, where build scripts written for
    // asm68k expect to find them.
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for m in &asm.messages {
        if cmd.quiet && m.severity == Severity::Info {
            continue;
        }
        let _ = writeln!(out, "{}", m);
    }

    let mut failed = !asm.ok();
    let mut save = |path: &Path, data: &[u8], what: &str| {
        if let Err(e) = std::fs::write(path, data) {
            let _ = writeln!(out, "Error creating {} '{}': {}", what, path.display(), e);
            failed = true;
        }
    };

    if let Some(path) = &cmd.listing {
        if let Some(listing) = &asm.listing {
            save(path, listing, "list file");
        }
    }
    if asm.ok() {
        if let Some(path) = &cmd.object {
            let data = match cmd.format {
                Format::Binary => asm.image().1,
                Format::SRecord => {
                    let (base, image) = asm.image();
                    output::srec::write(base, &image, base)
                }
                Format::Elf => output::elf::write(&asm),
            };
            save(path, &data, "output file");
        }
        if let Some(path) = &cmd.symbols {
            let opts = &cmd.config.options;
            let data = output::symfile::write(&asm.symbols, opts.case_sensitive, opts.locals_in_symfile);
            save(path, &data, "symbol file");
        }
    }

    if !cmd.quiet || failed {
        let _ = writeln!(out, "\nAssembly completed.");
        let _ = writeln!(
            out,
            "{} error(s) from {} lines in {:.1} seconds",
            asm.errors,
            asm.lines,
            start.elapsed().as_secs_f64()
        );
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
