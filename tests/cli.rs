//! Runs the assembler as a program.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory for the files of a test, removed when the test is done.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("slop68k-test-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn write(&self, name: &str, contents: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn read(&self, name: &str) -> Vec<u8> {
        fs::read(self.0.join(name)).unwrap_or_else(|_| panic!("{} was not written", name))
    }

    fn exists(&self, name: &str) -> bool {
        self.0.join(name).exists()
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_slop68k"));
        cmd.current_dir(&self.0).args(args).env_remove("SOURCE_DATE_EPOCH");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn asm68k_command_line() {
    let d = Dir::new("compat");
    d.write("src/main.68k", "\torg\t$100\nstart:\tmoveq\t#1,d0\n\tinclude\t\"src/sub.inc\"\n\tbra.s\tstart\n");
    d.write("src/sub.inc", "value\tequ\t2\n\taddq.w\t#value,d0\n");
    fs::create_dir_all(d.0.join("out")).unwrap();

    // As in a build script written for asm68k, DOS paths and all.
    let o = d.run(&["/k", "/p", "/o", "ae-", "src\\main.68k,", "main.bin", ",", "out\\main.sym,", "out\\main.lst"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert!(stdout(&o).contains("0 error(s) from 6 lines"), "{}", stdout(&o));

    assert_eq!(d.read("main.bin"), [0x70, 0x01, 0x54, 0x40, 0x60, 0xfa]);
    assert_eq!(d.read("out/main.sym"), b"MND\x01\0\0\0\0\x00\x01\0\0\x02\x05start");
    let listing = String::from_utf8(d.read("out/main.lst")).unwrap();
    assert!(listing.contains("00000100 7001                       start:\tmoveq\t#1,d0\r\n"), "{}", listing);
    assert!(listing.contains("00000102 =00000002                  value\tequ\t2\r\n"), "{}", listing);
}

#[test]
fn errors_fail_the_build() {
    let d = Dir::new("errors");
    d.write("bad.68k", "\tmoveq\t#1,d0\n\tfrob\td0\n\tbra.s\tnowhere\n");
    let o = d.run(&["/p", "bad.68k,bad.bin,,bad.lst"]);
    assert!(!o.status.success());
    let out = stdout(&o);
    assert!(out.contains("bad.68k(2) : Error : Op-code not recognised\n\tfrob\td0\n"), "{}", out);
    assert!(out.contains("1 error(s) from 3 lines"), "{}", out);
    // No object code is written, but the listing is there to look at.
    assert!(!d.exists("bad.bin"));
    assert!(d.exists("bad.lst"));
}

#[test]
fn undefined_symbols_are_errors() {
    let d = Dir::new("undefined");
    d.write("bad.68k", "\tbra.s\tnowhere\n\tdc.l\tnothing+1\n");
    let o = d.run(&["/p", "bad.68k,bad.bin"]);
    assert!(!o.status.success());
    let out = stdout(&o);
    assert!(out.contains("bad.68k(1) : Error : Symbol 'nowhere' not defined"), "{}", out);
    assert!(out.contains("bad.68k(2) : Error : Symbol 'nothing' not defined"), "{}", out);
    assert!(!d.exists("bad.bin"));
}

#[test]
fn missing_source() {
    let d = Dir::new("missing");
    let o = d.run(&["/p", "nothing.68k,out.bin"]);
    assert!(!o.status.success());
    assert!(stdout(&o).contains("Could not open file 'nothing.68k'"), "{}", stdout(&o));
    let o = d.run(&[]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn source_extension_is_optional() {
    let d = Dir::new("extension");
    d.write("prog.68k", "\trts\n");
    let o = d.run(&["/p", "prog,prog.bin"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("prog.bin"), [0x4e, 0x75]);
}

#[test]
fn file_names_are_found_whatever_their_case() {
    if cfg!(windows) || cfg!(target_os = "macos") {
        return;
    }
    let d = Dir::new("case");
    d.write("Main.68k", "\tinclude\t'INC\\DEFS.INC'\n\tdc.b\tvalue\n\tincbin\t\"data\\Blob.BIN\"\n");
    d.write("inc/defs.inc", "value\tequ\t7\n");
    d.write("Data/blob.bin", "xyz");
    let o = d.run(&["/p", "main.68k,out.bin"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("out.bin"), b"\x07xyz");
}

#[test]
fn symbols_from_the_command_line() {
    let d = Dir::new("defines");
    d.write("prog.68k", "\tdc.w\tVERSION,DEBUG\n\tif\tdef(EXTRA)\n\tdc.b\t1\n\tendif\n");
    let o = d.run(&["/p", "/e", "VERSION=$102", "/e", "DEBUG=0", "prog.68k,prog.bin"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("prog.bin"), [0x01, 0x02, 0x00, 0x00]);
    let o = d.run(&["-D", "VERSION=1", "-DDEBUG=1", "-D", "EXTRA", "-o", "b.bin", "prog.68k"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("b.bin"), [0x00, 0x01, 0x00, 0x01, 0x01]);
}

#[test]
fn include_path() {
    let d = Dir::new("include");
    d.write("prog.68k", "\tinclude\t\"defs.inc\"\n\tdc.b\tvalue\n");
    d.write("lib/defs.inc", "value\tequ\t9\n");
    let o = d.run(&["/p", "prog.68k,prog.bin"]);
    assert!(!o.status.success());
    let o = d.run(&["/p", "/j", "lib", "prog.68k,prog.bin"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("prog.bin"), [9]);
    let o = d.run(&["-I", "lib", "-o", "prog2.bin", "prog.68k"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("prog2.bin"), [9]);
}

#[test]
fn time_of_assembly() {
    let d = Dir::new("time");
    d.write(
        "prog.68k",
        "year\tequ\t_year+1900\n\tdc.b\t\"\\#year/\\#_month/\\#_day \\#_hours:\\#_minutes:\\#_seconds \\#_weekday\"\n",
    );
    // 2026-09-27 15:26:40, a Sunday
    let o = d.run_with(&["/p", "prog.68k,prog.bin"], &[("SOURCE_DATE_EPOCH", "1790522800")]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(String::from_utf8(d.read("prog.bin")).unwrap(), "2026/9/27 15:26:40 0");
    // A time that is set is not moved to another time zone.
    let o = d.run_with(
        &["/p", "--local-time", "prog.68k,local.bin"],
        &[("SOURCE_DATE_EPOCH", "1790522800"), ("TZ", "Asia/Tokyo")],
    );
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("local.bin"), d.read("prog.bin"));
}

#[test]
fn time_zone() {
    let d = Dir::new("zone");
    d.write("prog.68k", "\tdc.b\t_hours,_minutes\n");
    // Fifteen hours apart, so that one reading of the clock cannot pass
    // for the other whatever the time is.
    let utc = |hours: u8, zone: u8| (hours + 24 - zone) % 24;
    let read = |args: &[&str], tz: &str| {
        let o = d.run_with(args, &[("TZ", tz)]);
        assert!(o.status.success(), "{}", stdout(&o));
        (stdout(&o), d.read("prog.bin"))
    };

    // The time is UTC wherever the program is run.
    let (_, a) = read(&["-q", "-o", "prog.bin", "prog.68k"], "Etc/GMT-9");
    let (_, b) = read(&["-q", "-o", "prog.bin", "prog.68k"], "Etc/GMT+6");
    let minutes = |t: &[u8]| t[0] as i32 * 60 + t[1] as i32;
    let apart = |a: &[u8], b: &[u8]| (minutes(b) - minutes(a)).rem_euclid(24 * 60);
    assert!(apart(&a, &b) <= 1, "{:?} {:?}", a, b);

    // Local time is there for the asking, on systems where it is known.
    let (out, east) = read(&["-q", "--local-time", "-o", "prog.bin", "prog.68k"], "Etc/GMT-9");
    if cfg!(target_os = "linux") {
        assert_eq!(out, "");
        let (_, west) = read(&["-q", "--local-time", "-o", "prog.bin", "prog.68k"], "Etc/GMT+6");
        assert!(apart(&[utc(east[0], 9), east[1]], &b) <= 2, "{:?} {:?}", east, b);
        assert!(apart(&[utc(west[0], 18), west[1]], &b) <= 2, "{:?} {:?}", west, b);
    } else {
        assert!(out.contains("Warning : Local time is not available"), "{}", out);
        assert!(apart(&east, &b) <= 2);
    }
}

#[test]
fn messages_of_inform() {
    let d = Dir::new("inform");
    d.write(
        "prog.68k",
        "size\tequ\t$1234\n\tinform\t0,\"Size is %d bytes (%h) of %s\",size,size,\"ROM\"\n\tinform\t1,\"careful\"\n\tdc.b\t1\n",
    );
    let o = d.run(&["/p", "prog.68k,prog.bin"]);
    assert!(o.status.success(), "{}", stdout(&o));
    let out = stdout(&o);
    assert!(out.contains("Size is 4660 bytes (1234) of ROM\n"), "{}", out);
    assert!(out.contains("prog.68k(3) : Warning : careful\n"), "{}", out);
    // Nothing but warnings and errors is printed when asked to be quiet.
    let o = d.run(&["/p", "--quiet", "prog.68k,prog.bin"]);
    assert_eq!(stdout(&o).trim(), "prog.68k(3) : Warning : careful\n\tinform\t1,\"careful\"");
}

#[test]
fn other_formats() {
    let d = Dir::new("formats");
    d.write("prog.68k", "\torg\t$400\n\tnop\n\trts\n");
    let o = d.run(&["/ps", "prog.68k,prog.s37"]);
    assert!(o.status.success(), "{}", stdout(&o));
    let srec = String::from_utf8(d.read("prog.s37")).unwrap();
    assert!(srec.contains("S309000004004E714E7570\r\n"), "{}", srec);

    d.write("obj.68k", "\txdef\tentry\nentry:\tnop\n\trts\n");
    let o = d.run(&["/l", "obj.68k,obj.o"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(&d.read("obj.o")[..4], b"\x7fELF");

    // The name of the output is enough to say what it is to be.
    let o = d.run(&["-o", "obj2.o", "-l", "obj.lst", "-s", "obj.sym", "obj.68k"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("obj2.o"), d.read("obj.o"));
    assert!(d.exists("obj.lst"));
    let o = d.run(&["-o", "prog.srec", "prog.68k"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(d.read("prog.srec"), d.read("prog.s37"));

    // Programs to be linked have no say in where they end up.
    d.write("org.68k", "\torg\t$400\n\tnop\n");
    let o = d.run(&["/l", "org.68k,org.o"]);
    assert!(!o.status.success());
    assert!(stdout(&o).contains("ORG cannot be used when producing linkable output"), "{}", stdout(&o));
}

#[test]
fn help() {
    let d = Dir::new("help");
    let o = d.run(&["--help"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("Usage: slop68k"));
    let o = d.run(&["--version"]);
    assert!(stdout(&o).starts_with("slop68k "));
    let _ = Path::new("");
}
