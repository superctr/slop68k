//! The assembler proper.
//!
//! The source is read once. Code is generated as it is read, and any value
//! that depends on symbols defined further on is patched in when the end of
//! the source has been reached. This is how asm68k works, and it is what
//! makes the size of every instruction known at the time it is assembled.

mod data;
mod directives;
mod exprparse;
pub mod listing;
pub mod macros;
mod operands;
pub mod scan;
pub mod section;
pub mod statement;
mod subst;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::diag::{Message, Severity};
use crate::expr::{apply_binary, BinOp, EvalError, Expr, SymbolResolver};
use crate::m68k::encode::{self, EncodeCtx, Op};
use crate::m68k::fixup::{self, FixKind, Fixup};
use crate::m68k::operand::Size;
use crate::options::Options;
use crate::source::{self, SrcLine};
use crate::symbol::{SymKind, SymbolTable};
use crate::timestamp::Timestamp;
use crate::value::{Base, Value};

use listing::Listing;
use macros::{Macro, MacroCtx};
use section::{Chunk, Group, Section};
use statement::Statement;

pub(crate) type R<T> = Result<T, String>;

/// Most lines that may be processed before assembly is abandoned, as a
/// guard against loops that never end.
const MAX_LINES: u64 = 20_000_000;
/// Most code that may be generated, and the largest a program may be from
/// its first address to its last.
const MAX_SIZE: u64 = 1 << 30;
const MAX_FRAMES: usize = 256;
const MAX_ERRORS: u32 = 64;

/// What to assemble and how.
#[derive(Clone, Debug)]
pub struct Config {
    /// Name of the source file, as given by the user.
    pub source: String,
    pub include_dirs: Vec<PathBuf>,
    /// Symbols defined on the command line.
    pub defines: Vec<(String, i32)>,
    pub options: Options,
    /// Produce relocatable output: code outside of sections is not allowed
    /// and sections are not given addresses.
    pub relocatable: bool,
    /// Produce a listing.
    pub listing: bool,
    /// List the lines of macro expansions.
    pub list_macros: bool,
    /// List lines skipped by conditional assembly.
    pub list_skipped: bool,
    /// Time of assembly; the current time if not given.
    pub timestamp: Option<Timestamp>,
    /// Take the current time to be local time rather than UTC.
    pub local_time: bool,
}

impl Config {
    pub fn new(source: &str) -> Config {
        Config {
            source: source.to_string(),
            include_dirs: Vec::new(),
            defines: Vec::new(),
            options: Options::default(),
            relocatable: false,
            listing: false,
            list_macros: false,
            list_skipped: false,
            timestamp: None,
            local_time: false,
        }
    }
}

/// A relocation in relocatable output.
#[derive(Clone, Debug)]
pub struct Reloc {
    pub section: usize,
    pub offset: u32,
    pub kind: FixKind,
    /// What the value is relative to.
    pub base: Base,
    pub addend: i32,
}

/// A symbol as presented to the output writers.
#[derive(Clone, Debug)]
pub struct OutSymbol {
    pub name: String,
    pub value: u32,
    /// Section the symbol is relative to, if it is not absolute.
    pub section: Option<usize>,
    pub exported: bool,
    pub local: bool,
    pub kind: SymKind,
    /// Id in the symbol table, as used by relocations against externals.
    pub id: u32,
}

/// The result of an assembly.
pub struct Assembly {
    pub messages: Vec<Message>,
    pub errors: u32,
    pub warnings: u32,
    /// Number of source lines processed.
    pub lines: u64,
    pub sections: Vec<Section>,
    pub groups: Vec<Group>,
    pub relocs: Vec<Reloc>,
    pub symbols: Vec<OutSymbol>,
    pub listing: Option<Vec<u8>>,
}

impl Assembly {
    pub fn ok(&self) -> bool {
        self.errors == 0
    }

    /// The memory image of the program: its load address and contents.
    pub fn image(&self) -> (u32, Vec<u8>) {
        let mut chunks: Vec<Chunk> = Vec::new();
        for s in &self.sections {
            if s.bss {
                continue;
            }
            for c in &s.chunks {
                chunks.push(Chunk { addr: c.addr.wrapping_add(s.base), data: c.data.clone() });
            }
        }
        section::flatten(chunks.iter())
    }
}

struct SourceFile {
    name: String,
    dir: Option<PathBuf>,
}

enum FrameKind {
    File,
    Macro(MacroCtx),
    Rept { remaining: u32 },
    While { cond: String },
    Do { until: SrcLine },
}

struct Frame {
    lines: Rc<Vec<SrcLine>>,
    pos: usize,
    kind: FrameKind,
    /// Depth of conditional assembly when the frame was entered.
    cond_depth: usize,
    /// Depth of modules when the frame was entered.
    module_depth: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CondKind {
    If,
    Case,
}

struct Cond {
    kind: CondKind,
    /// Whether the code around the conditional is being assembled.
    parent_active: bool,
    /// Whether the current branch is being assembled.
    active: bool,
    /// Whether any branch has been assembled.
    taken: bool,
    /// Value being matched by a CASE.
    value: i32,
}

enum LoopKind {
    Rept(u32),
    While(String),
    Do,
}

enum Capture {
    Macro { name: String, key: String, params: Vec<String>, takes_label: bool, short: bool, depth: u32 },
    Loop { kind: LoopKind, depth: u32 },
}

struct Capturing {
    what: Capture,
    lines: Vec<SrcLine>,
    /// Where the construct started, for error messages.
    site: (u32, u32),
}

struct PendingFix {
    section: usize,
    chunk: usize,
    offset: usize,
    fix: Fixup,
    site: (u32, u32),
    text: Rc<str>,
}

struct ObjState {
    /// Location counter at the point the block was opened.
    resume: u32,
    /// Address the block is assembled for.
    start: u32,
}

pub struct Assembler<'c> {
    pub(crate) cfg: &'c Config,
    pub(crate) opts: Options,
    opt_stack: Vec<Options>,
    pub(crate) time: Timestamp,
    files: Vec<SourceFile>,
    frames: Vec<Frame>,
    pub(crate) syms: SymbolTable,
    pub(crate) macros: HashMap<String, Rc<Macro>>,
    conds: Vec<Cond>,
    capture: Option<Capturing>,

    /// Scope of local labels: changes with every non-local label.
    scope: u32,
    next_scope: u32,
    modules: Vec<u32>,

    pub(crate) rs: i32,
    pub(crate) radix: u32,
    macro_count: u32,
    pub(crate) equs_count: u32,
    pub(crate) equr_count: u32,
    pub(crate) reg_count: u32,
    pub(crate) expr_depth: u32,
    pub(crate) disabled: HashSet<String>,
    pub(crate) aliases: HashMap<String, String>,
    param_stack: Vec<String>,
    datasize: Option<usize>,
    public: bool,

    pub(crate) sections: Vec<Section>,
    pub(crate) groups: Vec<Group>,
    section_names: HashMap<String, usize>,
    group_names: HashMap<String, usize>,
    section_stack: Vec<usize>,
    pub(crate) cur_section: usize,
    /// Location counter.
    pub(crate) pc: u32,
    /// Location counter at the start of the statement, the value of `*`.
    pub(crate) line_pc: Value,
    /// OBJ blocks that are open, innermost last.
    obj: Vec<ObjState>,
    /// An ORG has been seen.
    org_seen: bool,
    fixups: Vec<PendingFix>,
    relocs: Vec<Reloc>,
    /// Number of bytes of code generated.
    emitted: u64,
    /// Number of times loops have gone round.
    turns: u64,

    messages: Vec<Message>,
    errors: u32,
    warnings: u32,
    lines: u64,
    /// Position in the source file being read (file id, line number).
    site: (u32, u32),
    /// Text of the line being assembled.
    cur_text: Rc<str>,
    ended: bool,
    fatal: bool,

    pub(crate) list: Option<Listing>,
}

/// Assembles the source named by the configuration.
pub fn assemble(cfg: &Config) -> Assembly {
    let mut asm = Assembler::new(cfg);
    asm.run();
    asm.finish()
}

impl<'c> Assembler<'c> {
    pub fn new(cfg: &'c Config) -> Assembler<'c> {
        let mut asm = Assembler::with_config(cfg);
        if cfg.timestamp.is_none() && cfg.local_time {
            match Timestamp::now_local() {
                Some(t) => asm.time = t,
                None => {
                    asm.warnings += 1;
                    asm.messages.push(Message {
                        severity: Severity::Warning,
                        location: None,
                        text: "Local time is not available on this system, using UTC".into(),
                        source: None,
                    });
                }
            }
        }
        if cfg.relocatable {
            // Code that is not put in a section goes where compilers put
            // theirs.
            asm.sections.push(Section::new(".text", false));
            asm.section_names.insert(asm.sym_key(".text"), 1);
            asm.cur_section = 1;
        }
        asm
    }

    fn with_config(cfg: &'c Config) -> Assembler<'c> {
        Assembler {
            cfg,
            opts: cfg.options.clone(),
            opt_stack: Vec::new(),
            time: cfg.timestamp.unwrap_or_else(Timestamp::now),
            files: Vec::new(),
            frames: Vec::new(),
            syms: SymbolTable::new(),
            macros: HashMap::new(),
            conds: Vec::new(),
            capture: None,
            scope: 0,
            next_scope: 1,
            modules: Vec::new(),
            rs: 0,
            radix: 10,
            macro_count: 0,
            equs_count: 0,
            equr_count: 0,
            reg_count: 0,
            expr_depth: 0,
            disabled: HashSet::new(),
            aliases: HashMap::new(),
            param_stack: Vec::new(),
            datasize: None,
            public: false,
            sections: vec![Section::new("", true)],
            groups: Vec::new(),
            section_names: HashMap::new(),
            group_names: HashMap::new(),
            section_stack: Vec::new(),
            cur_section: 0,
            pc: 0,
            line_pc: Value::abs(0),
            obj: Vec::new(),
            org_seen: false,
            fixups: Vec::new(),
            relocs: Vec::new(),
            emitted: 0,
            turns: 0,
            messages: Vec::new(),
            errors: 0,
            warnings: 0,
            lines: 0,
            site: (0, 0),
            cur_text: "".into(),
            ended: false,
            fatal: false,
            list: if cfg.listing { Some(Listing::new(cfg.list_macros)) } else { None },
        }
    }

    // ------------------------------------------------------------------
    // Messages

    fn message(&mut self, severity: Severity, text: String, site: Option<(u32, u32)>, source: Option<&str>) {
        let location = site.and_then(|(f, l)| self.files.get(f as usize).map(|f| (f.name.clone(), l)));
        let source = match severity {
            Severity::Info => None,
            _ if self.opts.print_error_lines => source.map(|s| s.to_string()),
            _ => None,
        };
        self.messages.push(Message { severity, location, text, source });
    }

    pub(crate) fn error(&mut self, text: impl Into<String>) {
        let src = self.cur_text.clone();
        self.error_at(text.into(), self.site, &src);
    }

    fn error_at(&mut self, text: String, site: (u32, u32), source: &str) {
        self.errors += 1;
        self.message(Severity::Error, text, Some(site), Some(source));
        if self.errors >= MAX_ERRORS && !self.fatal {
            self.fatal = true;
            self.messages.push(Message {
                severity: Severity::Info,
                location: None,
                text: "Too many errors, assembler aborting...".into(),
                source: None,
            });
        }
    }

    pub(crate) fn fatal(&mut self, text: impl Into<String>) {
        self.errors += 1;
        self.fatal = true;
        let src = self.cur_text.clone();
        self.message(Severity::Fatal, text.into(), Some(self.site), Some(&src));
    }

    pub(crate) fn warn(&mut self, text: impl Into<String>) {
        let src = self.cur_text.clone();
        self.warn_at(text.into(), self.site, &src);
    }

    fn warn_at(&mut self, text: String, site: (u32, u32), source: &str) {
        if !self.opts.warnings {
            return;
        }
        self.warnings += 1;
        self.message(Severity::Warning, text, Some(site), Some(source));
    }

    pub(crate) fn info(&mut self, text: impl Into<String>) {
        self.message(Severity::Info, text.into(), None, None);
    }

    // ------------------------------------------------------------------
    // Source files and the input stack

    /// Finds a file named in the source.
    pub(crate) fn locate(&self, name: &str) -> Option<PathBuf> {
        let dir = self
            .frames
            .iter()
            .rev()
            .find(|f| matches!(f.kind, FrameKind::File))
            .and_then(|f| f.lines.first())
            .and_then(|l| self.files.get(l.file as usize))
            .and_then(|f| f.dir.clone());
        source::locate(name, &self.cfg.include_dirs, dir.as_deref())
    }

    /// Reads a source file and makes it the current input.
    pub(crate) fn include(&mut self, name: &str, path: &Path) -> R<()> {
        let bytes = std::fs::read(path).map_err(|_| format!("Could not open file '{}'", name))?;
        let id = self.files.len() as u32;
        let lines = Rc::new(source::split_lines(&bytes, id));
        self.files.push(SourceFile { name: name.to_string(), dir: path.parent().map(|p| p.to_path_buf()) });
        self.push_frame(lines, FrameKind::File)
    }

    fn push_frame(&mut self, lines: Rc<Vec<SrcLine>>, kind: FrameKind) -> R<()> {
        if self.frames.len() >= MAX_FRAMES {
            self.fatal("Out of stack space, macros or includes are nested too deeply");
            return Ok(());
        }
        self.frames.push(Frame {
            lines,
            pos: 0,
            kind,
            cond_depth: self.conds.len(),
            module_depth: self.modules.len(),
        });
        Ok(())
    }

    /// The invocation of the macro being expanded, if any.
    pub(crate) fn macro_ctx(&self) -> Option<&MacroCtx> {
        self.frames.iter().rev().find_map(|f| match &f.kind {
            FrameKind::Macro(m) => Some(m),
            _ => None,
        })
    }

    pub(crate) fn macro_ctx_mut(&mut self) -> Option<&mut MacroCtx> {
        self.frames.iter_mut().rev().find_map(|f| match &mut f.kind {
            FrameKind::Macro(m) => Some(m),
            _ => None,
        })
    }

    fn in_macro(&self) -> bool {
        self.frames.iter().any(|f| matches!(f.kind, FrameKind::Macro(_)))
    }

    /// Evaluates the condition of a loop at the end of an iteration.
    fn loop_again(&mut self) -> bool {
        // A loop may go round without there being any lines in it.
        self.turns += 1;
        if self.turns > MAX_LINES {
            self.fatal("Too many lines, possibly due to a loop that does not end");
            return false;
        }
        enum Test {
            Count,
            While(String),
            Until(SrcLine),
        }
        let test = match self.frames.last_mut().map(|f| &mut f.kind) {
            Some(FrameKind::Rept { remaining }) => {
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    return false;
                }
                Test::Count
            }
            Some(FrameKind::While { cond }) => Test::While(cond.clone()),
            Some(FrameKind::Do { until }) => Test::Until(until.clone()),
            _ => return false,
        };
        let again = match test {
            Test::Count => true,
            Test::While(cond) => match self.eval_now(&cond) {
                Ok(v) => v != 0,
                Err(e) => {
                    self.error(e);
                    false
                }
            },
            Test::Until(line) => {
                self.cur_text = line.text.clone();
                match self.until_condition(&line.text) {
                    Ok(v) => v == 0,
                    Err(e) => {
                        self.error(e);
                        false
                    }
                }
            }
        };
        if again {
            // Conditionals left open by the body do not carry over.
            if let Some(f) = self.frames.last_mut() {
                f.pos = 0;
                let depth = f.cond_depth;
                self.conds.truncate(depth);
            }
        }
        again
    }

    fn until_condition(&mut self, line: &str) -> R<i32> {
        let text = if line.contains('\\') { Cow::Owned(self.substitute(line)?) } else { Cow::Borrowed(line) };
        let st = statement::parse(&text, self.opts.local_char);
        let field = self.operand_field(&text, st.operand_at)?;
        self.eval_now(&field)
    }

    /// Leaves the current input frame.
    fn pop_frame(&mut self) {
        let Some(frame) = self.frames.pop() else { return };
        if matches!(frame.kind, FrameKind::File) && !self.frames.is_empty() && !self.cfg.list_macros {
            if let Some(l) = &mut self.list {
                l.repeat_last(&self.sections);
            }
        }
        if let Some(cap) = self.capture.take() {
            let what = match cap.what {
                Capture::Macro { .. } => "MACRO",
                Capture::Loop { kind: LoopKind::Rept(_), .. } => "REPT",
                Capture::Loop { kind: LoopKind::While(_), .. } => "WHILE",
                Capture::Loop { kind: LoopKind::Do, .. } => "DO",
            };
            let text = format!("End of file reached without completion of {} construct", what);
            self.error_at(text, cap.site, "");
        }
        if self.conds.len() > frame.cond_depth {
            if let FrameKind::File = frame.kind {
                let what = match self.conds.last().map(|c| c.kind) {
                    Some(CondKind::Case) => "CASE does not have matching ENDCASE",
                    _ => "IF does not have matching ENDIF/ENDC",
                };
                self.error_at(what.into(), self.site, "");
            }
            self.conds.truncate(frame.cond_depth);
        }
        if self.modules.len() > frame.module_depth && !matches!(frame.kind, FrameKind::File) {
            self.warn("Module must end before end of macro/loop expansion - MODEND inserted");
            while self.modules.len() > frame.module_depth {
                if let Some(s) = self.modules.pop() {
                    self.scope = s;
                }
            }
        }
    }

    /// Fetches the next line of input, joining continuation lines.
    fn next_line(&mut self) -> Option<(SrcLine, bool, Option<Rc<str>>)> {
        loop {
            let frame = self.frames.last_mut()?;
            if frame.pos >= frame.lines.len() {
                if self.capture.is_none() && self.loop_again() {
                    continue;
                }
                self.pop_frame();
                continue;
            }
            let mut line = frame.lines[frame.pos].clone();
            frame.pos += 1;
            let from_file = matches!(frame.kind, FrameKind::File);

            // An ampersand at the end of a line continues it on the next.
            let mut first = None;
            if line.text.ends_with('&') && self.is_continued(&line.text) {
                first = Some(line.text.clone());
                let mut text = line.text.to_string();
                while text.ends_with('&') && self.is_continued(&text) {
                    let frame = self.frames.last_mut()?;
                    text.pop();
                    if frame.pos >= frame.lines.len() {
                        break;
                    }
                    let next = &frame.lines[frame.pos];
                    frame.pos += 1;
                    self.lines += 1;
                    text.push_str(next.text.trim_start());
                }
                line.text = text.into();
            }
            return Some((line, from_file, first));
        }
    }

    /// Whether the ampersand that ends a line is part of its operand
    /// field, rather than of a comment.
    fn is_continued(&self, text: &str) -> bool {
        if statement::is_blank(text) {
            return false;
        }
        let st = statement::parse(text, self.opts.local_char);
        scan::operand_end(text.as_bytes(), st.operand_at, self.opts.white_space) == text.len()
    }

    // ------------------------------------------------------------------
    // The main loop

    pub fn run(&mut self) {
        for (name, value) in self.cfg.defines.clone() {
            if let Err(e) = self.define(&name, Some(Value::abs(value)), None, SymKind::Equ) {
                self.error(e);
            }
        }
        let name = self.cfg.source.clone();
        let path = match source::locate(&name, &[], None) {
            Some(p) => p,
            None => {
                // Try the default extensions.
                match [".68k", ".s", ".asm"].iter().find_map(|ext| source::locate(&format!("{}{}", name, ext), &[], None)) {
                    Some(p) => p,
                    None => {
                        self.errors += 1;
                        self.fatal = true;
                        self.messages.push(Message {
                            severity: Severity::Fatal,
                            location: None,
                            text: format!("Could not open file '{}'", name),
                            source: None,
                        });
                        return;
                    }
                }
            }
        };
        if let Err(e) = self.include(&name, &path) {
            self.fatal(e);
            return;
        }

        while !self.ended && !self.fatal {
            let Some((line, from_file, first)) = self.next_line() else { break };
            self.lines += 1;
            if self.lines > MAX_LINES {
                self.fatal("Too many lines, possibly due to a loop that does not end");
                break;
            }
            if from_file {
                self.site = (line.file, line.line);
            }
            self.process_line(line, from_file, first);
        }

        if !self.fatal {
            if self.ended {
                // END leaves constructs open without complaint.
                self.capture = None;
                self.frames.clear();
                self.conds.clear();
            }
            if !self.modules.is_empty() {
                self.error_at("MODULE has no corresponding MODEND".into(), self.site, "");
            }
        }
        // The listing shows the code as it was before values were patched.
        if let Some(l) = &mut self.list {
            l.flush(&self.sections);
        }
        if self.errors > 0 {
            // What is wrong with the rest is likely to follow from the
            // errors so far.
            self.info("Errors during pass 1 - pass 2 aborted");
            return;
        }
        self.resolve();
    }

    /// Assembles a line and lists it. `first` is the first of the lines
    /// the line was joined from, if it was continued.
    fn process_line(&mut self, line: SrcLine, from_file: bool, first: Option<Rc<str>>) {
        self.cur_text = line.text.clone();
        let expansion = !from_file;
        if let Some(l) = &mut self.list {
            l.begin(first.as_ref().unwrap_or(&line.text), expansion, self.pc, self.cur_section, &self.sections);
        }

        let capturing = self.capture.is_some();
        let mut listed = self.process(&line);

        if let Some(l) = &mut self.list {
            if expansion {
                // Lines of expansions are listed as they were assembled,
                // less their comments.
                let text = &self.cur_text;
                if statement::is_blank(text) || capturing {
                    // Comments and the bodies of loops are not part of the
                    // expansion, and do not affect the listing of the line
                    // that caused it.
                    if !l.is_hidden() {
                        l.end(false, None);
                    }
                } else if l.is_hidden() {
                    l.end(listed, None);
                } else {
                    let tidy: Rc<str> = statement::tidy(text, self.opts.local_char, self.opts.white_space, is_macro_name)
                        .unwrap_or_default()
                        .into();
                    l.end(listed, Some(&tidy));
                }
            } else {
                // Empty lines are left out of macros and loops.
                if capturing && line.text.is_empty() {
                    listed = false;
                }
                l.end(listed, None);
            }
        }
    }

    /// Assembles a line. Returns whether the line belongs in the listing.
    fn process(&mut self, line: &SrcLine) -> bool {
        let text: &str = &line.text;

        if self.capture.is_some() {
            self.capture_line(line);
            return true;
        }
        if statement::is_blank(text) {
            return self.is_active() || self.cfg.list_skipped;
        }
        if !self.is_active() {
            self.skip_line(text);
            return self.cfg.list_skipped;
        }

        let text = if text.contains('\\') {
            match self.substitute(text) {
                Ok(t) => {
                    self.cur_text = t.as_str().into();
                    if statement::is_blank(&t) {
                        // Nothing is left of the line.
                        return true;
                    }
                    Cow::Owned(t)
                }
                Err(e) => {
                    self.error(e);
                    return true;
                }
            }
        } else {
            Cow::Borrowed(text)
        };

        if let Some(options) = text.strip_prefix('=') {
            if let Err(e) = self.case_option(options) {
                self.error(e);
            }
            return false;
        }

        let st = statement::parse(&text, self.opts.local_char);
        match self.execute(&st, &text) {
            Ok(listed) => listed,
            Err(e) => {
                self.error(e);
                true
            }
        }
    }

    /// Carries out a statement. Returns whether it belongs in the listing.
    fn execute(&mut self, st: &Statement, text: &str) -> R<bool> {
        self.line_pc = self.pc_value();
        let mnemonic = st.mnemonic();
        if mnemonic.is_empty() {
            if let Some(label) = st.label {
                self.define_label(label)?;
            }
            return Ok(true);
        }

        let mut buf = [0u8; 16];
        let lower = lowercase(mnemonic, &mut buf);
        let alias;
        let lower = match lower {
            Some(l) if !self.aliases.is_empty() && self.aliases.contains_key(l) => {
                alias = self.aliases[l].clone();
                Some(alias.as_str())
            }
            l => l,
        };

        if let Some(lower) = lower {
            if !self.disabled.is_empty() && self.disabled.contains(lower) {
                // Fall through to the macros.
            } else if let Some(dir) = directives::lookup(lower) {
                return self.directive(dir, st, text);
            } else if let Some(op) = encode::lookup(lower) {
                self.instruction(op, st, text)?;
                return Ok(true);
            }
        }
        if let Some(m) = self.find_macro(mnemonic) {
            self.invoke(m, st, text)?;
            return Ok(true);
        }
        Err("Op-code not recognised".into())
    }

    fn instruction(&mut self, op: Op, st: &Statement, text: &str) -> R<()> {
        if self.pc & 1 != 0 {
            self.warn("Instruction has been word aligned");
            self.emit(&[0], Vec::new())?;
        }
        self.line_pc = self.pc_value();
        if let Some(l) = &mut self.list {
            l.set_addr(self.pc);
        }
        if let Some(label) = st.label {
            self.define_label(label)?;
        }
        let size = match st.size() {
            "" => None,
            s => Some(Size::parse(s).ok_or("Bad size on opcode")?),
        };
        let field = self.operand_field(text, st.operand_at)?;
        let mut ops = Vec::with_capacity(2);
        for part in scan::split_operands(&field) {
            match self.parse_operand(part) {
                Ok(op) => ops.push(op),
                Err(_) if op == Op::Movem && part.contains(['-', '/']) => {
                    return Err("Error in register list".into());
                }
                Err(e) => return Err(e),
            }
        }
        let ctx = EncodeCtx { opts: &self.opts, pc: self.pc_value() };
        let enc = encode::encode(op, size, ops, &ctx)?;
        for w in enc.warnings {
            self.warn(w);
        }
        self.emit(&enc.bytes, enc.fixups)
    }

    /// The operand field of a statement, with names of string equates and
    /// macro parameters replaced.
    pub(crate) fn operand_field<'a>(&self, text: &'a str, at: usize) -> R<Cow<'a, str>> {
        let raw = self.raw_field(text, at);
        self.expand_names(raw)
    }

    /// The operand field of a statement as written.
    pub(crate) fn raw_field<'a>(&self, text: &'a str, at: usize) -> &'a str {
        let end = scan::operand_end(text.as_bytes(), at, self.opts.white_space);
        &text[at..end]
    }

    /// Lists the current line ahead of the lines it brings in.
    pub(crate) fn list_now(&mut self) {
        if let Some(l) = &mut self.list {
            l.end(true, None);
            l.flush(&self.sections);
        }
    }

    // ------------------------------------------------------------------
    // Symbols

    /// The key under which a name is kept in the symbol table.
    pub(crate) fn sym_key(&self, name: &str) -> String {
        let name = match self.macro_ctx() {
            Some(m) if !m.locals.is_empty() => match m.locals.get(&name.to_ascii_lowercase()) {
                Some(renamed) => renamed.as_str(),
                None => name,
            },
            _ => name,
        };
        let mut key = String::with_capacity(name.len() + 8);
        if name.starts_with(self.opts.local_char) {
            key.push_str(&self.scope.to_string());
            key.push(':');
        }
        if self.opts.case_sensitive {
            key.push_str(name);
        } else {
            key.extend(name.chars().map(|c| c.to_ascii_lowercase()));
        }
        key
    }

    fn is_local(&self, name: &str) -> bool {
        name.starts_with(self.opts.local_char)
    }

    pub(crate) fn find_macro(&self, name: &str) -> Option<Rc<Macro>> {
        if self.macros.is_empty() {
            return None;
        }
        self.macros.get(&self.sym_key(name)).cloned()
    }

    pub(crate) fn check_name(&self, name: &str) -> R<()> {
        let b = name.as_bytes();
        let ok = !b.is_empty()
            && scan::is_ident_start(b[0], self.opts.local_char)
            && b[1..].iter().all(|&c| scan::is_ident_char(c) || c as char == self.opts.local_char);
        if ok {
            Ok(())
        } else {
            Err("Illegal label".into())
        }
    }

    /// Defines a label at the current location.
    pub(crate) fn define_label(&mut self, name: &str) -> R<()> {
        let v = self.pc_value();
        self.define(name, Some(v), None, SymKind::Label)
    }

    /// Starts a new scope for local labels.
    fn descope(&mut self) {
        if self.modules.is_empty() {
            self.scope = self.next_scope;
            self.next_scope += 1;
        }
    }

    /// Defines a symbol.
    pub(crate) fn define(&mut self, name: &str, value: Option<Value>, expr: Option<Expr>, kind: SymKind) -> R<()> {
        self.check_name(name)?;
        let local = self.is_local(name);
        if !local {
            let descopes = match kind {
                SymKind::Label => true,
                SymKind::Equ | SymKind::Set => self.opts.descope_equ,
                _ => false,
            };
            if descopes {
                self.descope();
            }
        }
        let key = self.sym_key(name);
        if self.macros.contains_key(&key) && kind != SymKind::Set {
            return Err(format!("Label '{}' multiply defined", name));
        }
        let id = self.syms.intern(&key, name);
        let site = self.site;
        let public = self.public;
        let sym = self.syms.get_mut(id);
        match sym.kind {
            SymKind::Undefined => {}
            SymKind::Set if kind == SymKind::Set => {}
            SymKind::Equ if kind == SymKind::Equ && value.is_some() && sym.value == value => {}
            SymKind::Extern if kind == SymKind::Label && sym.global => {}
            _ => return Err(format!("Label '{}' multiply defined", name)),
        }
        let sym = self.syms.get_mut(id);
        sym.name = name.to_string();
        sym.kind = kind;
        sym.value = value;
        sym.expr = expr.map(Rc::new);
        sym.local = local;
        sym.defined_at = Some(site);
        if (public || sym.global) && !local && matches!(kind, SymKind::Label | SymKind::Equ | SymKind::Set) {
            sym.exported = true;
        }
        if let (Some(l), Some(v)) = (&mut self.list, value) {
            if kind != SymKind::Label {
                l.set_value(v.n);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Code generation

    /// The location counter as a value.
    pub(crate) fn pc_value(&self) -> Value {
        if self.sections[self.cur_section].absolute {
            Value::abs(self.pc as i32)
        } else {
            Value::rel(Base::Section(self.cur_section as u32), self.pc as i32)
        }
    }

    /// Appends bytes to the current section.
    pub(crate) fn emit(&mut self, bytes: &[u8], fixups: Vec<Fixup>) -> R<()> {
        let idx = self.cur_section;
        if idx == 0 && self.cfg.relocatable {
            return Err("Code generated before first section directive".into());
        }
        let sec = &mut self.sections[idx];
        if sec.bss {
            if bytes.iter().any(|&b| b != 0) || !fixups.is_empty() {
                return Err("Initialised data in BSS section".into());
            }
            self.pc = self.pc.wrapping_add(bytes.len() as u32);
            sec.size = sec.size.max(self.pc);
            return Ok(());
        }
        let chunk_idx = sec.chunks.len() - 1;
        let chunk = &mut sec.chunks[chunk_idx];
        let base = chunk.data.len();
        self.emitted += bytes.len() as u64;
        if self.emitted > MAX_SIZE {
            self.fatal("Out of memory, Assembler aborting");
            return Ok(());
        }
        chunk.data.extend_from_slice(bytes);
        self.pc = self.pc.wrapping_add(bytes.len() as u32);
        if !sec.absolute {
            sec.size = sec.size.max(chunk.end());
        }
        for mut fix in fixups {
            fix.offset += base;
            self.fixups.push(PendingFix {
                section: idx,
                chunk: chunk_idx,
                offset: fix.offset,
                fix,
                site: self.site,
                text: self.cur_text.clone(),
            });
        }
        Ok(())
    }

    /// Reserves space without defining its contents.
    pub(crate) fn reserve(&mut self, count: u32) -> R<()> {
        if self.sections[self.cur_section].bss {
            self.pc = self.pc.wrapping_add(count);
            let sec = &mut self.sections[self.cur_section];
            sec.size = sec.size.max(self.pc);
            return Ok(());
        }
        if count > 0x4000_0000 {
            return Err(format!("Illegal value ({})", count as i32));
        }
        self.emit(&vec![0; count as usize], Vec::new())
    }

    /// Emits a value of the given kind, to be patched later if need be.
    pub(crate) fn emit_value(&mut self, kind: FixKind, e: &Expr) -> R<()> {
        let mut bytes = [0u8; 4];
        let width = kind.width();
        let mut fixups = Vec::new();
        match e.as_abs() {
            Some(n) => {
                let mut warnings = Vec::new();
                fixup::apply(kind, n, &mut bytes, &mut warnings)?;
                for w in warnings {
                    self.warn(w);
                }
            }
            None => fixups.push(Fixup { offset: 0, kind, expr: e.clone(), pc: None }),
        }
        self.emit(&bytes[..width], fixups)
    }

    /// Moves the location counter to an even address.
    pub(crate) fn align_even(&mut self) -> R<()> {
        if self.pc & 1 != 0 {
            self.reserve(1)?;
        }
        Ok(())
    }

    /// Starts assembling at a new address.
    pub(crate) fn set_org(&mut self, addr: u32) {
        let sec = &mut self.sections[0];
        self.cur_section = 0;
        match sec.chunks.last_mut() {
            Some(c) if c.data.is_empty() => c.addr = addr,
            _ => sec.chunks.push(Chunk { addr, data: Vec::new() }),
        }
        self.pc = addr;
        self.obj.clear();
        self.org_seen = true;
    }

    // ------------------------------------------------------------------
    // Completion

    /// Gives relocatable sections their addresses.
    fn layout(&mut self) {
        if self.cfg.relocatable {
            return;
        }
        // Sections follow the code placed with ORG, in the order of their
        // groups and then of their definition.
        let mut addr = match section::extent(self.sections[0].chunks.iter()) {
            Some((_, end)) => end,
            None => self.sections[0].chunks.last().map_or(0, |c| c.addr),
        };
        let mut order: Vec<usize> = (1..self.sections.len()).collect();
        order.sort_by_key(|&i| (self.sections[i].group.unwrap_or(usize::MAX), i));
        let mut last_group = None;
        let mut group_start: HashMap<usize, u32> = HashMap::new();
        for i in order {
            let group = self.sections[i].group;
            if group != last_group {
                if let Some(g) = group {
                    if let Some(org) = self.groups[g].org {
                        addr = org;
                    } else if let Some(over) = self.groups[g].over {
                        addr = group_start.get(&over).copied().unwrap_or(addr);
                    }
                    group_start.insert(g, addr);
                }
                last_group = group;
            }
            let sec = &mut self.sections[i];
            let align = sec.align.max(1);
            addr = addr.wrapping_add(align - 1) / align * align;
            sec.base = addr;
            addr = addr.wrapping_add(sec.size);
            if let Some(g) = group {
                if let Some(limit) = self.groups[g].size {
                    let used = addr.wrapping_sub(group_start[&g]);
                    if used > limit {
                        let name = self.groups[g].name.clone();
                        self.error_at(format!("Group '{}' is too large ({} bytes)", name, used), self.site, "");
                    }
                }
            }
        }
        self.syms.section_bases = Some(self.sections.iter().map(|s| s.base).collect());
    }

    /// Patches in the values that were not known when the code using them
    /// was assembled.
    fn resolve(&mut self) {
        self.layout();
        if !self.cfg.relocatable {
            let chunks = self.sections.iter().filter(|s| !s.bss).flat_map(|s| {
                s.chunks.iter().filter(|c| !c.data.is_empty()).map(|c| (s.base as u64 + c.addr as u64, c.data.len() as u64))
            });
            let (mut lo, mut hi) = (u64::MAX, 0);
            for (addr, len) in chunks {
                lo = lo.min(addr);
                hi = hi.max(addr + len);
            }
            if hi > lo && (hi - lo > MAX_SIZE || hi > 1 << 32) {
                self.errors += 1;
                let text = format!("Program is too large: it goes from ${:X} to ${:X}", lo, hi);
                self.message(Severity::Error, text, None, None);
                return;
            }
        }
        let undefined: Vec<String> = self
            .syms
            .iter()
            .filter(|(_, s)| s.exported && !s.is_defined())
            .map(|(_, s)| s.name.clone())
            .collect();
        for name in undefined {
            self.errors += 1;
            let text = format!("Could not evaluate XDEF'd symbol '{}'", name);
            self.message(Severity::Error, text, None, None);
        }
        let fixups = std::mem::take(&mut self.fixups);
        let mut warnings = Vec::new();
        for f in &fixups {
            let v = match f.fix.expr.eval(&self.syms) {
                Ok(v) => v,
                Err(e) => {
                    let text = match e {
                        EvalError::Undefined(id) => {
                            let text = format!("Symbol '{}' not defined", self.syms.get(id).name);
                            self.errors += 1;
                            self.message(Severity::Error, text, Some(f.site), None);
                            continue;
                        }
                        EvalError::DivZero => "Division by zero".into(),
                        EvalError::Recursion => {
                            "Out of stack space, possibly due to recursive equates".into()
                        }
                        EvalError::NotAbsolute => "Error in expression".into(),
                    };
                    self.error_at(text, f.site, &f.text);
                    continue;
                }
            };
            let v = match f.fix.pc {
                Some(pc) => {
                    let pc = self.syms.absolute(pc);
                    match apply_binary(BinOp::Sub, v, pc) {
                        Ok(d) if d.is_abs() => d,
                        _ => {
                            if let Err(e) = self.relocate(f, v, Some(pc)) {
                                self.error_at(e, f.site, &f.text);
                            }
                            continue;
                        }
                    }
                }
                None => v,
            };
            if !v.is_abs() {
                if let Err(e) = self.relocate(f, v, None) {
                    self.error_at(e, f.site, &f.text);
                }
                continue;
            }
            let data = &mut self.sections[f.section].chunks[f.chunk].data;
            warnings.clear();
            match fixup::apply(f.fix.kind, v.n, &mut data[f.offset..], &mut warnings) {
                Ok(()) => {
                    for w in warnings.drain(..) {
                        self.warn_at(w, f.site, &f.text);
                    }
                }
                Err(_) if f.fix.kind == FixKind::Branch8 && v.n != 0 => {
                    let text = format!("Branch ({} bytes) is out of range", v.n);
                    self.errors += 1;
                    self.message(Severity::Error, text, Some(f.site), None);
                }
                Err(e) => {
                    self.errors += 1;
                    self.message(Severity::Error, e, Some(f.site), None);
                }
            }
        }
    }

    /// Records a relocation for a value that only the linker can know.
    fn relocate(&mut self, f: &PendingFix, v: Value, pc: Option<Value>) -> R<()> {
        if !self.cfg.relocatable {
            return Err("Error in expression".into());
        }
        let Some(base) = v.base else { return Err("Error in expression".into()) };
        let offset = self.sections[f.section].chunks[f.chunk].addr.wrapping_add(f.offset as u32);
        let mut addend = v.n;
        if let Some(pc) = pc {
            // Displacements are relative to the section the code is in; the
            // linker supplies the distance between the two sections.
            match pc.base {
                Some(Base::Section(s)) if s as usize == f.section => {
                    addend = addend.wrapping_sub(pc.n).wrapping_add(offset as i32);
                }
                _ => return Err("Error in expression".into()),
            }
        }
        match f.fix.kind {
            FixKind::Moveq | FixKind::Quick | FixKind::Trap => {
                return Err("Expression must evaluate".into());
            }
            _ => {}
        }
        self.relocs.push(Reloc { section: f.section, offset, kind: f.fix.kind, base, addend });
        Ok(())
    }

    pub fn finish(mut self) -> Assembly {
        let mut symbols = Vec::new();
        for (id, s) in self.syms.iter() {
            let value = match (s.value, &s.expr) {
                (Some(v), _) => Some(v),
                (None, Some(e)) => e.eval(&self.syms).ok(),
                _ => None,
            };
            let (value, section) = match value {
                Some(v) => match v.base {
                    None => (v.n as u32, None),
                    Some(Base::Section(n)) => {
                        let v = self.syms.absolute(v);
                        if v.is_abs() {
                            (v.n as u32, None)
                        } else {
                            (v.n as u32, Some(n as usize))
                        }
                    }
                    Some(Base::Extern(_)) => continue,
                },
                None if s.kind == SymKind::Extern || (s.kind == SymKind::Undefined && s.global) => (0, None),
                None => continue,
            };
            let kind = if s.kind == SymKind::Undefined { SymKind::Extern } else { s.kind };
            if !matches!(kind, SymKind::Label | SymKind::Equ | SymKind::Set | SymKind::Extern) {
                continue;
            }
            symbols.push(OutSymbol {
                name: s.name.clone(),
                value,
                section,
                exported: s.exported,
                local: s.local,
                kind,
                id,
            });
        }
        let listing = self.list.take().map(|l| l.finish(&self.sections));
        Assembly {
            messages: self.messages,
            errors: self.errors,
            warnings: self.warnings,
            lines: self.lines,
            sections: self.sections,
            groups: self.groups,
            relocs: self.relocs,
            symbols,
            listing,
        }
    }
}

/// Whether an opcode is that of a macro, as far as can be told without
/// knowing which macros there are: it is not an instruction or directive.
pub(crate) fn is_macro_name(mnemonic: &str) -> bool {
    let mut buf = [0u8; 16];
    match lowercase(mnemonic, &mut buf) {
        Some(l) => directives::lookup(l).is_none() && encode::lookup(l).is_none(),
        None => true,
    }
}

/// Lower case copy of a short ASCII word, without allocating.
fn lowercase<'a>(s: &str, buf: &'a mut [u8; 16]) -> Option<&'a str> {
    let b = s.as_bytes();
    if b.len() > buf.len() {
        return None;
    }
    for (d, c) in buf.iter_mut().zip(b) {
        if !c.is_ascii() {
            return None;
        }
        *d = c.to_ascii_lowercase();
    }
    std::str::from_utf8(&buf[..b.len()]).ok()
}
