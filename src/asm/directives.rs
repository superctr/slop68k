//! Assembler directives.

use std::collections::HashMap;
use std::rc::Rc;

use super::macros::{self, Macro, MacroCtx};
use super::scan::*;
use super::section::{Group, Section};
use super::statement::{self, Statement};
use super::{Assembler, Capture, Capturing, Cond, CondKind, FrameKind, LoopKind, ObjState, R};
use crate::expr::Expr;
use crate::m68k::operand::{Reg, Size};
use crate::source::SrcLine;
use crate::symbol::SymKind;
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cmp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dir {
    Equ,
    Set,
    Equs,
    Equr,
    Reg,
    Rs,
    Rsset,
    Rsreset,
    Dc,
    Dcb,
    Ds,
    Hex,
    Datasize,
    Data,
    Ieee32,
    Ieee64,
    Org,
    Even,
    Cnop,
    Obj,
    Objend,
    Include,
    Incbin,
    End,
    If,
    /// IFEQ and friends compare their operand with zero.
    IfCmp(Cmp),
    /// IFD (true) and IFND (false).
    IfDef(bool),
    /// IFC (true) and IFNC (false).
    IfStr(bool),
    Else,
    Elseif,
    Endif,
    Case,
    Endcase,
    Rept,
    Endr,
    While,
    Endw,
    Do,
    Until,
    Macro,
    Macros,
    Endm,
    Mexit,
    Shift,
    Local,
    Purge,
    Pushp,
    Popp,
    Substr,
    Opt,
    Pusho,
    Popo,
    List,
    Nolist,
    Inform,
    Fail,
    Xdef,
    Xref,
    Public,
    Global,
    Section,
    Group,
    Pushs,
    Pops,
    Module,
    Modend,
    Radix,
    Alias,
    Disable,
    /// Directives for the target hardware, which have no effect here.
    Ignored,
}

pub(crate) fn lookup(name: &str) -> Option<Dir> {
    Some(match name {
        "equ" => Dir::Equ,
        "set" | "=" | "==" => Dir::Set,
        "equs" => Dir::Equs,
        "equr" => Dir::Equr,
        "reg" => Dir::Reg,
        "rs" => Dir::Rs,
        "rsset" => Dir::Rsset,
        "rsreset" => Dir::Rsreset,
        "dc" => Dir::Dc,
        "dcb" => Dir::Dcb,
        "ds" => Dir::Ds,
        "hex" => Dir::Hex,
        "datasize" => Dir::Datasize,
        "data" => Dir::Data,
        "ieee32" => Dir::Ieee32,
        "ieee64" => Dir::Ieee64,
        "org" => Dir::Org,
        "even" => Dir::Even,
        "cnop" => Dir::Cnop,
        "obj" => Dir::Obj,
        "objend" => Dir::Objend,
        "include" => Dir::Include,
        "incbin" => Dir::Incbin,
        "end" => Dir::End,
        "if" => Dir::If,
        "ifeq" => Dir::IfCmp(Cmp::Eq),
        "ifne" => Dir::IfCmp(Cmp::Ne),
        "ifgt" => Dir::IfCmp(Cmp::Gt),
        "ifge" => Dir::IfCmp(Cmp::Ge),
        "iflt" => Dir::IfCmp(Cmp::Lt),
        "ifle" => Dir::IfCmp(Cmp::Le),
        "ifd" => Dir::IfDef(true),
        "ifnd" => Dir::IfDef(false),
        "ifc" => Dir::IfStr(true),
        "ifnc" => Dir::IfStr(false),
        "else" => Dir::Else,
        "elseif" => Dir::Elseif,
        "endif" | "endc" => Dir::Endif,
        "case" => Dir::Case,
        "endcase" => Dir::Endcase,
        "rept" => Dir::Rept,
        "endr" => Dir::Endr,
        "while" => Dir::While,
        "endw" => Dir::Endw,
        "do" => Dir::Do,
        "until" => Dir::Until,
        "macro" => Dir::Macro,
        "macros" => Dir::Macros,
        "endm" => Dir::Endm,
        "mexit" => Dir::Mexit,
        "shift" => Dir::Shift,
        "local" => Dir::Local,
        "purge" => Dir::Purge,
        "pushp" => Dir::Pushp,
        "popp" => Dir::Popp,
        "substr" => Dir::Substr,
        "opt" => Dir::Opt,
        "pusho" => Dir::Pusho,
        "popo" => Dir::Popo,
        "list" => Dir::List,
        "nolist" => Dir::Nolist,
        "inform" => Dir::Inform,
        "fail" => Dir::Fail,
        "xdef" => Dir::Xdef,
        "xref" => Dir::Xref,
        "public" => Dir::Public,
        "global" => Dir::Global,
        "section" => Dir::Section,
        "group" => Dir::Group,
        "pushs" => Dir::Pushs,
        "pops" => Dir::Pops,
        "module" => Dir::Module,
        "modend" => Dir::Modend,
        "radix" => Dir::Radix,
        "alias" => Dir::Alias,
        "disable" => Dir::Disable,
        "regs" | "unit" | "workspace" => Dir::Ignored,
        _ => return None,
    })
}

/// Whether a directive opens a conditional block.
fn opens_conditional(dir: Dir) -> bool {
    matches!(dir, Dir::If | Dir::IfCmp(_) | Dir::IfDef(_) | Dir::IfStr(_) | Dir::Case)
}

/// Parses the size suffix of a data directive; the default is word.
fn data_size(st: &Statement) -> R<Size> {
    match st.size() {
        "" => Ok(Size::Word),
        s => match Size::parse(s) {
            Some(Size::Short) | None => Err("Bad size on opcode".into()),
            Some(s) => Ok(s),
        },
    }
}

impl Assembler<'_> {
    fn label_of<'a>(&self, st: &Statement<'a>) -> R<&'a str> {
        st.label.ok_or_else(|| "Statement must have a label".to_string())
    }

    /// Defines the label of a statement, if there is one, at the current
    /// location.
    pub(crate) fn place_label(&mut self, st: &Statement) -> R<()> {
        self.line_pc = self.pc_value();
        if let Some(l) = &mut self.list {
            l.set_addr(self.pc);
        }
        match st.label {
            Some(label) => self.define_label(label),
            None => Ok(()),
        }
    }

    /// Carries out a directive. Returns whether the line is to be listed.
    pub(crate) fn directive(&mut self, dir: Dir, st: &Statement, text: &str) -> R<bool> {
        // Directives that have no use for a label of their own leave it to
        // be defined as the current location.
        let takes_label = matches!(
            dir,
            Dir::Equ
                | Dir::Set
                | Dir::Equs
                | Dir::Equr
                | Dir::Reg
                | Dir::Rs
                | Dir::Macro
                | Dir::Macros
                | Dir::Substr
                | Dir::Group
                | Dir::Section
                | Dir::Alias
                | Dir::Module
        );
        let places_label = matches!(
            dir,
            Dir::Dc
                | Dir::Dcb
                | Dir::Ds
                | Dir::Hex
                | Dir::Datasize
                | Dir::Data
                | Dir::Ieee32
                | Dir::Ieee64
                | Dir::Org
                | Dir::Even
                | Dir::Cnop
                | Dir::Obj
                | Dir::Objend
                | Dir::Include
                | Dir::Incbin
                | Dir::End
                | Dir::Rept
                | Dir::While
                | Dir::Do
                | Dir::Opt
                | Dir::Inform
                | Dir::Rsset
                | Dir::Rsreset
        );
        if !takes_label && !places_label && st.label.is_some() {
            self.place_label(st)?;
        }
        match dir {
            Dir::If | Dir::IfCmp(_) | Dir::IfDef(_) | Dir::IfStr(_) => {
                let r = self.condition(dir, st, text);
                let v = *r.as_ref().unwrap_or(&false);
                self.conds.push(Cond {
                    kind: CondKind::If,
                    parent_active: true,
                    active: v,
                    taken: v,
                    value: 0,
                });
                r?;
                return Ok(dir != Dir::If);
            }
            Dir::Else | Dir::Elseif => {
                self.check_cond(CondKind::If)?;
                // The branch before this one was assembled, so this one and
                // those that follow are not.
                if let Some(c) = self.conds.last_mut() {
                    c.active = false;
                    c.taken = true;
                }
                return Ok(false);
            }
            Dir::Endif => {
                self.check_cond(CondKind::If)?;
                self.conds.pop();
                return Ok(false);
            }
            Dir::Case => {
                let field = self.operand_field(text, st.operand_at)?;
                let r = self.eval_now(&field);
                self.conds.push(Cond {
                    kind: CondKind::Case,
                    parent_active: true,
                    active: false,
                    taken: r.is_err(),
                    value: *r.as_ref().unwrap_or(&0),
                });
                if r.is_err() {
                    return Err("Case choice expression cannot be evaluated".into());
                }
                return Ok(false);
            }
            Dir::Endcase => {
                self.check_cond(CondKind::Case)?;
                self.conds.pop();
                return Ok(false);
            }
            _ => {}
        }

        match dir {
            Dir::Equ => {
                let label = self.label_of(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let e = self.parse_expr(&field)?;
                match e {
                    Expr::Const(v) => self.define(label, Some(v), None, SymKind::Equ)?,
                    e => self.define(label, None, Some(e), SymKind::Equ)?,
                }
            }
            Dir::Set => {
                let label = self.label_of(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let e = self.parse_expr(&field)?;
                let v = match e {
                    Expr::Const(v) => v,
                    e => Value::abs(self.const_value(&e)?),
                };
                self.define(label, Some(v), None, SymKind::Set)?;
            }
            Dir::Equs => {
                let label = self.label_of(st)?;
                let field = self.raw_field(text, st.operand_at);
                let value = self.string_operand(field)?;
                self.define_equs(label, &value)?;
            }
            Dir::Equr => {
                let label = self.label_of(st)?;
                let field = self.raw_field(text, st.operand_at);
                let reg = match self.lookup_reg(field.trim()) {
                    Some(r @ (Reg::Data(_) | Reg::Addr(_))) => r,
                    _ => return Err("Illegal addressing mode".into()),
                };
                if label.contains('.') {
                    return Err("Illegal label".into());
                }
                let n = reg.number().unwrap_or(0) as i32;
                self.define(label, Some(Value::abs(n)), None, SymKind::Equr)?;
                self.equr_count += 1;
                if let Some(l) = &mut self.list {
                    l.clear_value();
                }
            }
            Dir::Reg => {
                let label = self.label_of(st)?;
                let field = self.raw_field(text, st.operand_at);
                let mask = self.parse_reglist(field).ok_or("Error in register list")?;
                self.define(label, Some(Value::abs(mask as i32)), None, SymKind::RegList)?;
                self.reg_count += 1;
                if let Some(l) = &mut self.list {
                    l.clear_value();
                }
            }
            Dir::Rs => {
                let size = data_size(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let count = self.eval_now(&field)?;
                if size != Size::Byte && self.opts.auto_even && self.rs & 1 != 0 {
                    self.rs = self.rs.wrapping_add(1);
                }
                let at = self.rs;
                self.rs = self.rs.wrapping_add(count.wrapping_mul(size.bytes() as i32));
                if let Some(label) = st.label {
                    self.define(label, Some(Value::abs(at)), None, SymKind::Equ)?;
                }
            }
            Dir::Rsset => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                self.rs = self.eval_now(&field)?;
            }
            Dir::Rsreset => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                self.rs = if field.trim().is_empty() { 0 } else { self.eval_now(&field)? };
            }
            Dir::Dc => self.dir_dc(data_size(st)?, st, text)?,
            Dir::Dcb => self.dir_dcb(data_size(st)?, st, text)?,
            Dir::Ds => self.dir_ds(data_size(st)?, st, text)?,
            Dir::Hex => self.dir_hex(st, text)?,
            Dir::Datasize => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let n = self.eval_now(&field)?;
                if !(1..=256).contains(&n) {
                    return Err("Datasize value must be in range 1 to 256".into());
                }
                self.datasize = Some(n as usize);
            }
            Dir::Data => self.dir_data(st, text)?,
            Dir::Ieee32 => self.dir_ieee(false, st, text)?,
            Dir::Ieee64 => self.dir_ieee(true, st, text)?,
            Dir::Org => {
                let field = self.operand_field(text, st.operand_at)?;
                if self.cfg.relocatable {
                    return Err("ORG cannot be used when producing linkable output".into());
                }
                if self.sections.len() > 1 {
                    return Err("ORG cannot be used after SECTION directive".into());
                }
                let expr = field.split(',').next().unwrap_or("").trim();
                let expr = expr.strip_prefix('?').unwrap_or(expr);
                let addr = self.eval_now(expr)?;
                self.set_org(addr as u32);
                if let Some(label) = st.label {
                    self.define_label(label)?;
                }
            }
            Dir::Even => {
                self.align_even()?;
                self.place_label(st)?;
            }
            Dir::Cnop => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let parts = split_operands(&field);
                if parts.len() != 2 {
                    return Err("Expected comma between operands".into());
                }
                let offset = self.eval_now(parts[0])?;
                let bound = self.eval_now(parts[1])?;
                if bound <= 0 {
                    return Err(format!("Illegal value ({}) for boundary in cnop", bound));
                }
                if offset < 0 {
                    return Err(format!("Illegal value ({}) for offset in cnop", offset));
                }
                let bound = bound as u32;
                let sec = &self.sections[self.cur_section];
                if !sec.absolute && bound > sec.align {
                    self.warn("Alignment cannot be guaranteed");
                }
                // The next address that is the offset past a boundary.
                let offset = offset as u32;
                let rem = offset % bound;
                let pad = (rem + bound - self.pc % bound) % bound;
                self.reserve(pad + (offset - rem))?;
            }
            Dir::Obj => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let addr = self.eval_now(&field)? as u32;
                self.obj.push(ObjState { resume: self.pc, start: addr });
                self.pc = addr;
            }
            Dir::Objend => {
                self.place_label(st)?;
                match self.obj.pop() {
                    // Carry on from where the block was opened, past the
                    // code that was assembled in it.
                    Some(o) => self.pc = o.resume.wrapping_add(self.pc.wrapping_sub(o.start)),
                    None => return Err("OBJEND without OBJ".into()),
                }
            }
            Dir::Include => {
                self.place_label(st)?;
                let field = text[st.operand_at..].trim();
                let name = self.file_name(field)?.0;
                let path = self.locate(&name).ok_or_else(|| format!("Could not open file '{}'", name))?;
                // The line is listed before those of the file it names.
                self.list_now();
                self.include(&name, &path)?;
            }
            Dir::Incbin => self.dir_incbin(st, text)?,
            Dir::End => {
                self.place_label(st)?;
                self.ended = true;
                return Ok(false);
            }
            Dir::Rept => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?;
                let count = self.eval_now(&field)?;
                self.start_capture(Capture::Loop { kind: LoopKind::Rept(count.max(0) as u32), depth: 0 });
            }
            Dir::While => {
                self.place_label(st)?;
                let field = self.operand_field(text, st.operand_at)?.into_owned();
                self.start_capture(Capture::Loop { kind: LoopKind::While(field), depth: 0 });
            }
            Dir::Do => {
                self.place_label(st)?;
                self.start_capture(Capture::Loop { kind: LoopKind::Do, depth: 0 });
            }
            Dir::Endr => return Err("No corresponding rept".into()),
            Dir::Endw => return Err("No corresponding while".into()),
            Dir::Until => return Err("No corresponding do".into()),
            Dir::Macro | Dir::Macros => {
                let label = self.label_of(st)?;
                self.check_name(label)?;
                let (params, takes_label) = macros::parse_params(&text[st.operand_at..])?;
                let key = self.sym_key(label);
                if self.syms.find(&key).is_some_and(|id| self.syms.get(id).is_defined()) {
                    return Err(format!("Label '{}' multiply defined", label));
                }
                self.start_capture(Capture::Macro {
                    name: label.to_string(),
                    key,
                    params,
                    takes_label,
                    short: dir == Dir::Macros,
                    depth: 0,
                });
            }
            Dir::Endm => return Err("endm is illegal outside a macro definition".into()),
            Dir::Mexit => {
                if !self.in_macro() {
                    return Err("'mexit' illegal outside of macros".into());
                }
                while let Some(f) = self.frames.last() {
                    let done = matches!(f.kind, FrameKind::Macro(_));
                    self.pop_frame();
                    if done {
                        break;
                    }
                }
            }
            Dir::Shift => {
                let m = self.macro_ctx_mut().ok_or("Shift can only be used inside a macro")?;
                if !m.args.is_empty() {
                    m.args.remove(0);
                }
            }
            Dir::Local => {
                let field = self.raw_field(text, st.operand_at).to_string();
                let local_char = self.opts.local_char;
                let m = self.macro_ctx_mut().ok_or("Local can only be used inside a macro")?;
                for name in field.split(',') {
                    let name = name.trim();
                    if name.is_empty() {
                        return Err("Illegal name in locals list".into());
                    }
                    if name.starts_with(local_char) {
                        return Err(format!("Names declared with local must not start with '{}'", local_char));
                    }
                    let renamed = format!("{}#{}", name, m.unique);
                    m.locals.insert(name.to_ascii_lowercase(), renamed);
                }
            }
            Dir::Purge => {
                let field = self.raw_field(text, st.operand_at);
                let key = self.sym_key(field.trim());
                if self.macros.remove(&key).is_none() {
                    return Err(if self.syms.find(&key).is_some() {
                        format!("Old version of {} cannot be purged", field.trim())
                    } else {
                        "Cannot purge - name was never defined".to_string()
                    });
                }
            }
            Dir::Pushp => {
                let field = self.raw_field(text, st.operand_at);
                let value = self.string_operand(field).unwrap_or_else(|_| field.to_string());
                self.param_stack.push(value);
            }
            Dir::Popp => {
                let field = self.raw_field(text, st.operand_at).trim().to_string();
                if field.starts_with(self.opts.local_char) {
                    return Err("Cannot POPP to a local label".into());
                }
                let value = self.param_stack.pop().ok_or("Parameter stack is empty")?;
                self.define_equs(&field, &value).map_err(|_| "POPP must specify a string or undefined name")?;
            }
            Dir::Substr => {
                let label = self.label_of(st)?;
                let field = self.raw_field(text, st.operand_at);
                let parts = split_operands(field);
                if parts.len() != 3 {
                    return Err("Expecting comma between operands in 'substr'".into());
                }
                let s: Vec<char> = self.string_operand(parts[2])?.chars().collect();
                let start = if parts[0].is_empty() { 1 } else { self.eval_now(parts[0])? };
                let end = if parts[1].is_empty() { s.len() as i32 } else { self.eval_now(parts[1])? };
                if start < 1 || end < 0 {
                    return Err("Illegal index value in SUBSTR".into());
                }
                // Positions past the end of the string give an empty result.
                let end = (end as usize).min(s.len());
                let start = (start as usize - 1).min(end);
                let sub: String = s[start..end].iter().collect();
                self.define_equs(label, &sub)?;
            }
            Dir::Opt => {
                self.place_label(st)?;
                let field = self.raw_field(text, st.operand_at);
                let was = self.opts.local_char;
                self.opts.apply(field)?;
                if self.opts.local_char != was {
                    // Names are looked up by their prefix from here on.
                }
            }
            Dir::Pusho => self.opt_stack.push(self.opts.clone()),
            Dir::Popo => self.opts = self.opt_stack.pop().ok_or("Option stack is empty")?,
            Dir::List => {
                let field = self.raw_field(text, st.operand_at).trim();
                if let Some(l) = &mut self.list {
                    match field {
                        "" => l.level = 0,
                        "+" => l.level += 1,
                        "-" => l.level -= 1,
                        _ => return Err("Expecting '+' or '-' after list command".into()),
                    }
                }
            }
            Dir::Nolist => {
                if let Some(l) = &mut self.list {
                    l.level = -1;
                }
            }
            Dir::Inform => self.dir_inform(st, text)?,
            Dir::Fail => {
                self.error("Assembly failed");
                self.fatal = true;
            }
            Dir::Xdef | Dir::Xref | Dir::Global => {
                let field = self.raw_field(text, st.operand_at).to_string();
                for name in field.split(',') {
                    let name = name.trim();
                    if name.is_empty() {
                        return Err("Missing comma in XDEF/XREF list".into());
                    }
                    if name.starts_with(self.opts.local_char) {
                        return Err("Local symbols cannot be XDEF'd/XREF'd".into());
                    }
                    self.check_name(name).map_err(|_| "Illegal name in XDEF/XREF list")?;
                    let key = self.sym_key(name);
                    let id = self.syms.intern(&key, name);
                    let sym = self.syms.get_mut(id);
                    match dir {
                        Dir::Xdef => sym.exported = true,
                        Dir::Xref => {
                            if sym.kind == SymKind::Undefined {
                                sym.kind = SymKind::Extern;
                            }
                        }
                        _ => {
                            sym.global = true;
                            if sym.is_defined() {
                                sym.exported = true;
                            } else {
                                sym.kind = SymKind::Extern;
                            }
                        }
                    }
                }
            }
            Dir::Public => {
                let field = self.raw_field(text, st.operand_at).trim();
                if field.eq_ignore_ascii_case("on") {
                    self.public = true;
                } else if field.eq_ignore_ascii_case("off") {
                    self.public = false;
                } else {
                    return Err("Expecting ON or OFF after directive".into());
                }
            }
            Dir::Section => self.dir_section(st, text)?,
            Dir::Group => {
                let label = self.label_of(st)?;
                let field = self.raw_field(text, st.operand_at).to_string();
                let mut group = Group { name: label.to_string(), ..Group::default() };
                self.group_attributes(&mut group, &field)?;
                let key = self.sym_key(label);
                if self.group_names.contains_key(&key) {
                    return Err(format!("Label '{}' multiply defined", label));
                }
                self.group_names.insert(key, self.groups.len());
                self.groups.push(group);
            }
            Dir::Pushs => self.section_stack.push(self.cur_section),
            Dir::Pops => {
                let s = self.section_stack.pop().ok_or("Section stack is empty")?;
                self.switch_section(s, None)?;
            }
            Dir::Module => {
                // The label of a module is not local to it.
                if let Some(label) = st.label {
                    self.define_label(label)?;
                } else {
                    self.descope();
                }
                self.modules.push(self.scope);
                self.scope = self.next_scope;
                self.next_scope += 1;
            }
            Dir::Modend => {
                let depth = self.frames.last().map_or(0, |f| f.module_depth);
                if self.modules.len() <= depth {
                    return Err(if self.modules.is_empty() {
                        "Unexpected modend encountered".to_string()
                    } else {
                        "Module may not end until macro/loop expansion is complete".to_string()
                    });
                }
                if let Some(s) = self.modules.pop() {
                    self.scope = s;
                }
            }
            Dir::Radix => {
                // The new base is always given in decimal.
                let field = self.operand_field(text, st.operand_at)?;
                let was = std::mem::replace(&mut self.radix, 10);
                let r = self.eval_now(&field);
                self.radix = was;
                let r = r?;
                if !(2..=16).contains(&r) {
                    return Err("Radix must be in range 2 to 16".into());
                }
                self.radix = r as u32;
            }
            Dir::Alias | Dir::Disable => {
                let field = self.raw_field(text, st.operand_at).trim().to_ascii_lowercase();
                if field.is_empty() {
                    return Err("Illegal name in command".into());
                }
                if dir == Dir::Alias {
                    let label = self.label_of(st)?;
                    self.aliases.insert(label.to_ascii_lowercase(), field);
                } else {
                    self.disabled.insert(field);
                }
            }
            Dir::Ignored => {}
            _ => unreachable!(),
        }
        Ok(true)
    }

    // ------------------------------------------------------------------
    // Conditional assembly

    /// Whether code is being assembled rather than skipped.
    pub(crate) fn is_active(&self) -> bool {
        self.conds.last().is_none_or(|c| c.active)
    }

    /// Checks that a directive ending or continuing a conditional has one
    /// of the right kind to refer to.
    fn check_cond(&self, kind: CondKind) -> R<()> {
        let floor = self.frames.last().map_or(0, |f| f.cond_depth);
        match self.conds.last() {
            Some(c) if self.conds.len() > floor => {
                if c.kind == kind {
                    Ok(())
                } else if c.kind == CondKind::Case {
                    Err("Expecting 'ENDCASE' at this point".into())
                } else {
                    Err("Expecting 'ENDIF' at this point".into())
                }
            }
            _ => Err(match kind {
                CondKind::If => "No corresponding 'if'".into(),
                CondKind::Case => "No matching case statement for endcase".into(),
            }),
        }
    }

    /// Evaluates the condition of an IF directive.
    fn condition(&mut self, dir: Dir, st: &Statement, text: &str) -> R<bool> {
        match dir {
            Dir::IfDef(want) => {
                let field = self.raw_field(text, st.operand_at);
                Ok(self.is_defined(field.trim()) == want)
            }
            Dir::IfStr(want) => {
                let field = self.raw_field(text, st.operand_at);
                let parts = split_operands(field);
                if parts.len() != 2 {
                    return Err("Expected comma between operands".into());
                }
                let a = self.string_operand(parts[0]).unwrap_or_else(|_| parts[0].to_string());
                let b = self.string_operand(parts[1]).unwrap_or_else(|_| parts[1].to_string());
                Ok((a == b) == want)
            }
            _ => {
                let field = self.operand_field(text, st.operand_at)?;
                let v = self.eval_now(&field)?;
                Ok(match dir {
                    Dir::IfCmp(Cmp::Eq) => v == 0,
                    Dir::IfCmp(Cmp::Ne) => v != 0,
                    Dir::IfCmp(Cmp::Gt) => v > 0,
                    Dir::IfCmp(Cmp::Ge) => v >= 0,
                    Dir::IfCmp(Cmp::Lt) => v < 0,
                    Dir::IfCmp(Cmp::Le) => v <= 0,
                    _ => v != 0,
                })
            }
        }
    }

    /// Looks at a line that is being skipped, to keep track of the
    /// conditionals it may contain.
    pub(crate) fn skip_line(&mut self, text: &str) {
        if let Some(options) = text.strip_prefix('=') {
            let options = options.to_string();
            let pending = matches!(
                self.conds.last(),
                Some(c) if c.kind == CondKind::Case && c.parent_active && !c.taken
            );
            if pending {
                let r = match self.substitute(&options) {
                    Ok(o) => self.case_option(&o),
                    Err(e) => Err(e),
                };
                if let Err(e) = r {
                    self.error(e);
                }
            }
            return;
        }
        let st = statement::parse(text, self.opts.local_char);
        let mut buf = [0u8; 16];
        let Some(dir) = super::lowercase(st.mnemonic(), &mut buf).and_then(lookup) else {
            return;
        };
        if opens_conditional(dir) {
            self.conds.push(Cond {
                kind: if dir == Dir::Case { CondKind::Case } else { CondKind::If },
                parent_active: false,
                active: false,
                taken: true,
                value: 0,
            });
            return;
        }
        let r = match dir {
            Dir::Else | Dir::Elseif => self.check_cond(CondKind::If).and_then(|_| {
                let pending = self.conds.last().is_some_and(|c| c.parent_active && !c.taken);
                if !pending {
                    return Ok(());
                }
                let v = if dir == Dir::Else { true } else { self.elseif_condition(text)? };
                if let Some(c) = self.conds.last_mut() {
                    c.active = v;
                    c.taken = v;
                }
                Ok(())
            }),
            Dir::Endif => self.check_cond(CondKind::If).map(|_| {
                self.conds.pop();
            }),
            Dir::Endcase => self.check_cond(CondKind::Case).map(|_| {
                self.conds.pop();
            }),
            _ => Ok(()),
        };
        if let Err(e) = r {
            self.error(e);
        }
    }

    /// Evaluates the condition of an ELSEIF; without one it acts as ELSE.
    fn elseif_condition(&mut self, text: &str) -> R<bool> {
        let text = if text.contains('\\') { self.substitute(text)? } else { text.to_string() };
        self.cur_text = text.as_str().into();
        let st = statement::parse(&text, self.opts.local_char);
        let field = self.operand_field(&text, st.operand_at)?;
        if field.trim().is_empty() {
            return Ok(true);
        }
        Ok(self.eval_now(&field)? != 0)
    }

    /// Handles a line that selects a case: `=value,value` or `=?`.
    pub(crate) fn case_option(&mut self, options: &str) -> R<()> {
        let floor = self.frames.last().map_or(0, |f| f.cond_depth);
        let (taken, parent, value) = match self.conds.last() {
            Some(c) if c.kind == CondKind::Case && self.conds.len() > floor => {
                (c.taken, c.parent_active, c.value)
            }
            _ => return Err("No matching CASE statement for case option".into()),
        };
        let mut hit = false;
        if parent && !taken {
            let end = operand_end(options.as_bytes(), 0, self.opts.white_space);
            let field = self.expand_names(&options[..end])?;
            if field.trim() == "?" {
                hit = true;
            } else {
                for part in split_operands(&field) {
                    if part.is_empty() {
                        return Err("Missing comma in list of case options".into());
                    }
                    if self.eval_now(part)? == value {
                        hit = true;
                    }
                }
            }
        }
        if let Some(c) = self.conds.last_mut() {
            c.active = hit;
            c.taken |= hit;
        }
        // Anything after the options on the same line is ignored, as the
        // options take the place of the label.
        Ok(())
    }

    // ------------------------------------------------------------------
    // Macros and loops

    fn start_capture(&mut self, what: Capture) {
        self.capture = Some(Capturing { what, lines: Vec::new(), site: self.site });
    }

    /// Adds a line to the macro or loop being defined.
    pub(crate) fn capture_line(&mut self, line: &SrcLine) {
        let text: &str = &line.text;
        let mut buf = [0u8; 16];
        let dir = if statement::is_blank(text) {
            None
        } else {
            let st = statement::parse(text, self.opts.local_char);
            super::lowercase(st.mnemonic(), &mut buf).and_then(lookup)
        };
        let Some(cap) = &mut self.capture else { return };
        let mut done = false;
        let mut short_body = false;
        let mut mismatch = None;
        match &mut cap.what {
            Capture::Macro { short, depth, .. } => {
                if *short {
                    short_body = true;
                    done = true;
                } else {
                    match dir {
                        Some(Dir::Macro) => *depth += 1,
                        Some(Dir::Endm) => {
                            if *depth == 0 {
                                done = true;
                            } else {
                                *depth -= 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Capture::Loop { kind, depth } => match dir {
                Some(Dir::Rept | Dir::While | Dir::Do) => *depth += 1,
                Some(d @ (Dir::Endr | Dir::Endw | Dir::Until)) => {
                    if *depth == 0 {
                        done = true;
                        let want = match kind {
                            LoopKind::Rept(_) => Dir::Endr,
                            LoopKind::While(_) => Dir::Endw,
                            LoopKind::Do => Dir::Until,
                        };
                        if d != want {
                            mismatch = Some(match want {
                                Dir::Endr => "ENDR",
                                Dir::Endw => "ENDW",
                                _ => "UNTIL",
                            });
                        }
                    } else {
                        *depth -= 1;
                    }
                }
                _ => {}
            },
        }
        if !done {
            // Lines are kept without their comments.
            let tidy = statement::tidy(text, self.opts.local_char, self.opts.white_space, super::is_macro_name);
            if let Some(tidy) = tidy {
                cap.lines.push(SrcLine { text: tidy.into(), file: line.file, line: line.line });
            }
            return;
        }
        let Some(mut cap) = self.capture.take() else { return };
        if short_body {
            cap.lines.push(line.clone());
        }
        if let Some(want) = mismatch {
            self.error(format!("Expecting '{}' at this point", want));
            return;
        }
        let lines = Rc::new(cap.lines);
        let r = match cap.what {
            Capture::Macro { name, key, params, takes_label, short, .. } => {
                self.macros.insert(
                    key,
                    Rc::new(Macro { name, params: Rc::new(params), takes_label, lines, short }),
                );
                Ok(())
            }
            Capture::Loop { kind, .. } => match kind {
                LoopKind::Rept(0) => Ok(()),
                LoopKind::Rept(n) => self.push_frame(lines, FrameKind::Rept { remaining: n }),
                LoopKind::While(cond) => match self.eval_now(&cond) {
                    Ok(0) => Ok(()),
                    Ok(_) => self.push_frame(lines, FrameKind::While { cond }),
                    Err(e) => Err(e),
                },
                LoopKind::Do => self.push_frame(lines, FrameKind::Do { until: line.clone() }),
            },
        };
        if let Err(e) = r {
            self.error(e);
        }
    }

    /// Starts the expansion of a macro.
    pub(crate) fn invoke(&mut self, m: Rc<Macro>, st: &Statement, text: &str) -> R<()> {
        let (args, all) = macros::parse_args(&text[st.operand_at..])?;
        if !m.takes_label {
            self.place_label(st)?;
        }
        self.macro_count += 1;
        let ctx = MacroCtx {
            args,
            all,
            size: st.size().to_string(),
            label: st.label.unwrap_or("").to_string(),
            params: m.params.clone(),
            unique: self.macro_count,
            locals: HashMap::new(),
        };
        self.push_frame(m.lines.clone(), FrameKind::Macro(ctx))
    }

    // ------------------------------------------------------------------
    // Strings

    /// Parses an operand that is either a quoted string or the name of a
    /// string equate.
    pub(crate) fn string_operand(&self, field: &str) -> R<String> {
        let field = field.trim();
        let b = field.as_bytes();
        match b.first() {
            Some(b'\'' | b'"') => match skip_string(b, 0) {
                Some(end) if end == b.len() => Ok(unquote(field)),
                Some(_) => Err("Expecting quoted string as operand".into()),
                None => Err("No closing quote on string".into()),
            },
            Some(_) => match self.text_of(field) {
                Some(t) => Ok(t.into_owned()),
                None => Err("One string equate can only be equated to another".into()),
            },
            None => Err("Expecting quoted string as operand".into()),
        }
    }

    /// Defines or redefines a string equate.
    pub(crate) fn define_equs(&mut self, name: &str, text: &str) -> R<()> {
        self.check_name(name)?;
        if name.starts_with(self.opts.local_char) {
            return Err("Local labels may not be strings".into());
        }
        let key = self.sym_key(name);
        let id = self.syms.intern(&key, name);
        let site = self.site;
        let sym = self.syms.get_mut(id);
        match sym.kind {
            SymKind::Undefined => self.equs_count += 1,
            SymKind::Equs => {}
            _ => return Err(format!("Label '{}' multiply defined", name)),
        }
        let sym = self.syms.get_mut(id);
        sym.kind = SymKind::Equs;
        sym.text = Some(text.into());
        sym.defined_at = Some(site);
        Ok(())
    }

    /// Parses a file name, which may be quoted. Returns the name and what
    /// follows it.
    pub(crate) fn file_name<'a>(&self, field: &'a str) -> R<(String, &'a str)> {
        let b = field.as_bytes();
        match b.first() {
            Some(b'\'' | b'"') => {
                let quote = b[0];
                let end = b[1..].iter().position(|&c| c == quote).ok_or("No closing quote on string")?;
                Ok((field[1..end + 1].to_string(), &field[end + 2..]))
            }
            Some(_) => {
                let end = b
                    .iter()
                    .position(|&c| c == b',' || c == b';' || is_space(c))
                    .unwrap_or(b.len());
                Ok((field[..end].to_string(), &field[end..]))
            }
            None => Err("File name must be quoted".into()),
        }
    }

    fn dir_inform(&mut self, st: &Statement, text: &str) -> R<()> {
        self.place_label(st)?;
        let field = self.operand_field(text, st.operand_at)?;
        let parts = split_operands(&field);
        if parts.len() < 2 {
            return Err("Expected comma between operands".into());
        }
        let severity = self.eval_now(parts[0])?;
        let format = self.string_operand(parts[1])?;
        let mut args = parts[2..].iter();
        let mut out = String::new();
        let mut chars = format.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.peek().map(|c| c.to_ascii_lowercase()) {
                Some(kind @ ('d' | 'h' | 's')) => {
                    chars.next();
                    let arg = args.next().ok_or("Expected comma between operands")?;
                    match kind {
                        'd' => out.push_str(&self.eval_now(arg)?.to_string()),
                        'h' => out.push_str(&format!("{:X}", self.eval_now(arg)?)),
                        _ => out.push_str(&self.string_operand(arg)?),
                    }
                }
                _ => out.push('%'),
            }
        }
        match severity {
            0 => self.info(out),
            1 => self.warn(out),
            2 => self.error(out),
            3 => self.fatal(out),
            n => return Err(format!("Illegal value ({})", n)),
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Sections

    /// Makes a section the current one. Code is aligned to `align`, or to
    /// the alignment of the section if that is not given.
    pub(crate) fn switch_section(&mut self, idx: usize, align: Option<u32>) -> R<()> {
        let pc = self.pc;
        if self.obj.is_empty() {
            self.sections[self.cur_section].pc = pc;
        }
        self.cur_section = idx;
        self.pc = self.sections[idx].pc;
        self.obj.clear();
        if let Some(l) = &mut self.list {
            l.retarget(idx, &self.sections);
        }
        let sec = &mut self.sections[idx];
        if sec.absolute {
            return Ok(());
        }
        let align = align.unwrap_or(sec.align).max(1);
        sec.align = sec.align.max(align);
        let pad = (align - self.pc % align) % align;
        if pad > 0 {
            self.reserve(pad)?;
        }
        Ok(())
    }

    fn group_attributes(&mut self, group: &mut Group, field: &str) -> R<()> {
        for attr in split_operands(field) {
            if attr.is_empty() {
                continue;
            }
            let (name, arg) = match attr.find('(') {
                Some(n) => {
                    let inner = attr[n + 1..].strip_suffix(')').ok_or("Missing ')' after function parameter(s)")?;
                    (attr[..n].trim(), Some(inner.trim()))
                }
                None => (attr, None),
            };
            let linkable = self.cfg.relocatable;
            match (name.to_ascii_lowercase().as_str(), arg) {
                ("word", None) => group.word = true,
                ("bss", None) => group.bss = true,
                ("org", Some(a)) => {
                    if linkable {
                        return Err("ORG address cannot be specified when producing linkable output".into());
                    }
                    group.org = Some(self.eval_now(a)? as u32);
                }
                ("obj", Some(a)) => {
                    if linkable {
                        return Err("OBJ cannot be specified when producing linkable output".into());
                    }
                    group.obj = if a.is_empty() { group.org } else { Some(self.eval_now(a)? as u32) };
                }
                ("size", Some(a)) => group.size = Some(self.eval_now(a)? as u32),
                ("file", Some(a)) => group.file = Some(self.string_operand(a).map_err(|_| "File name must be quoted")?),
                ("over", Some(a)) => {
                    if linkable {
                        return Err("Overlay cannot be specified when producing linkable output".into());
                    }
                    let key = self.sym_key(a);
                    let g = self.group_names.get(&key).ok_or("Overlay must specify a previously defined group name")?;
                    group.over = Some(*g);
                }
                _ => return Err(format!("Unknown group attribute '{}'", attr)),
            }
        }
        Ok(())
    }

    fn dir_section(&mut self, st: &Statement, text: &str) -> R<()> {
        let align = match st.size() {
            "" => 2,
            s => match Size::parse(s) {
                Some(Size::Byte) => 1,
                Some(Size::Word) => 2,
                Some(Size::Long) => 4,
                _ => return Err("Bad size on opcode".into()),
            },
        };
        let field = self.raw_field(text, st.operand_at).to_string();
        let (name, group) = match st.label {
            // A section with attributes gets a group of its own.
            Some(label) => {
                let key = self.sym_key(label);
                if !self.section_names.contains_key(&key) {
                    let mut group = Group { name: format!("~{}", label), ..Group::default() };
                    self.group_attributes(&mut group, &field)?;
                    self.groups.push(group);
                    (label.to_string(), Some(self.groups.len() - 1))
                } else {
                    (label.to_string(), None)
                }
            }
            None => {
                let parts = split_operands(&field);
                let name = parts.first().copied().unwrap_or("");
                if name.is_empty() {
                    return Err("Illegal section name".into());
                }
                let group = match parts.get(1) {
                    Some(g) => {
                        let key = self.sym_key(g);
                        Some(*self.group_names.get(&key).ok_or_else(|| format!("'{}' is not a group", g))?)
                    }
                    None => None,
                };
                (name.to_string(), group)
            }
        };
        self.check_name(&name).map_err(|_| "Illegal section name")?;
        let key = self.sym_key(&name);
        let idx = match self.section_names.get(&key) {
            Some(&idx) => {
                if let Some(g) = group {
                    if self.sections[idx].group.is_some_and(|old| old != g) {
                        return Err(format!("Section '{}' is already in another group", name));
                    }
                    self.sections[idx].group = Some(g);
                }
                idx
            }
            None => {
                let mut sec = Section::new(&name, false);
                sec.group = group;
                sec.bss = group.is_some_and(|g| self.groups[g].bss);
                if self.cfg.relocatable && name.trim_start_matches('.').to_ascii_lowercase().starts_with("bss") {
                    sec.bss = true;
                }
                sec.align = align;
                if sec.bss {
                    sec.chunks.clear();
                }
                self.sections.push(sec);
                let idx = self.sections.len() - 1;
                self.section_names.insert(key, idx);
                idx
            }
        };
        self.switch_section(idx, Some(align))?;
        self.line_pc = self.pc_value();
        self.descope();
        Ok(())
    }

    /// The functions that report the size or end of a section or group.
    pub(crate) fn section_function(&mut self, func: &str, name: &str) -> R<Expr> {
        let key = self.sym_key(name);
        let sections: Vec<usize> = if func.starts_with("sect") {
            let idx = *self.section_names.get(&key).ok_or_else(|| format!("'{}' is not a section", name))?;
            vec![idx]
        } else {
            let g = *self.group_names.get(&key).ok_or_else(|| format!("'{}' is not a group", name))?;
            (0..self.sections.len()).filter(|&i| self.sections[i].group == Some(g)).collect()
        };
        let size = |i: usize| if i == self.cur_section { self.sections[i].size.max(self.pc) } else { self.sections[i].size };
        let total: u32 = sections.iter().map(|&i| size(i)).sum();
        if func.ends_with("size") {
            return Ok(Expr::num(total as i32));
        }
        match sections.last() {
            Some(&i) => Ok(Expr::Const(Value::rel(crate::value::Base::Section(i as u32), size(i) as i32))),
            None => Ok(Expr::num(0)),
        }
    }
}
