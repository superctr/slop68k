//! 68000 instruction encoding.

use super::fixup::{self, FixKind, Fixup};
use super::operand::*;
use crate::expr::{apply_binary, BinOp, Expr};
use crate::options::Options;
use crate::value::Value;

pub const ERR_MODE: &str = "Illegal addressing mode";
pub const ERR_SIZE: &str = "Bad size on opcode";
pub const ERR_EXTRA: &str = "Unexpected characters at end of line";

type R<T> = Result<T, String>;

/// An instruction, as identified by its mnemonic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Move,
    Movea,
    Moveq,
    Movem,
    Movep,
    Lea,
    Pea,
    Exg,
    Swap,
    Ext,
    Link,
    Unlk,
    /// ADD (false) or SUB (true), and likewise for the variants below.
    AddSub(bool),
    AddSubA(bool),
    AddSubI(bool),
    AddSubQ(bool),
    AddSubX(bool),
    /// AND (false) or OR (true).
    AndOr(bool),
    Eor,
    /// ANDI, ORI or EORI, with the base opcode.
    LogicI(u16),
    Cmp,
    Cmpa,
    Cmpi,
    Cmpm,
    /// CLR, NEG, NEGX, NOT and TST, with the base opcode.
    Unary(u16),
    Nbcd,
    Tas,
    /// MULU, MULS, DIVU, DIVS and CHK, with the base opcode.
    MulDiv(u16),
    /// ABCD (false) or SBCD (true).
    Bcd(bool),
    /// Shift or rotate: kind (0 = AS, 1 = LS, 2 = ROX, 3 = RO) and direction.
    Shift(u8, bool),
    /// BTST, BCHG, BCLR, BSET as 0-3.
    Bit(u8),
    Bcc(u8),
    Dbcc(u8),
    Scc(u8),
    Jmp,
    Jsr,
    Trap,
    Stop,
    /// Instructions without operands, with their opcode.
    Simple(u16),
}

fn condition(s: &str) -> Option<u8> {
    Some(match s {
        "t" => 0,
        "f" => 1,
        "hi" => 2,
        "ls" => 3,
        "cc" | "hs" => 4,
        "cs" | "lo" => 5,
        "ne" => 6,
        "eq" => 7,
        "vc" => 8,
        "vs" => 9,
        "pl" => 10,
        "mi" => 11,
        "ge" => 12,
        "lt" => 13,
        "gt" => 14,
        "le" => 15,
        _ => return None,
    })
}

/// Looks up a (lower case) mnemonic.
pub fn lookup(mn: &str) -> Option<Op> {
    Some(match mn {
        "move" => Op::Move,
        "movea" => Op::Movea,
        "moveq" => Op::Moveq,
        "movem" => Op::Movem,
        "movep" => Op::Movep,
        "lea" => Op::Lea,
        "pea" => Op::Pea,
        "exg" => Op::Exg,
        "swap" => Op::Swap,
        "ext" => Op::Ext,
        "link" => Op::Link,
        "unlk" => Op::Unlk,
        "add" => Op::AddSub(false),
        "sub" => Op::AddSub(true),
        "adda" => Op::AddSubA(false),
        "suba" => Op::AddSubA(true),
        "addi" => Op::AddSubI(false),
        "subi" => Op::AddSubI(true),
        "addq" => Op::AddSubQ(false),
        "subq" => Op::AddSubQ(true),
        "addx" => Op::AddSubX(false),
        "subx" => Op::AddSubX(true),
        "and" => Op::AndOr(false),
        "or" => Op::AndOr(true),
        "eor" => Op::Eor,
        "andi" => Op::LogicI(0x0200),
        "ori" => Op::LogicI(0x0000),
        "eori" => Op::LogicI(0x0a00),
        "cmp" => Op::Cmp,
        "cmpa" => Op::Cmpa,
        "cmpi" => Op::Cmpi,
        "cmpm" => Op::Cmpm,
        "negx" => Op::Unary(0x4000),
        "clr" => Op::Unary(0x4200),
        "neg" => Op::Unary(0x4400),
        "not" => Op::Unary(0x4600),
        "tst" => Op::Unary(0x4a00),
        "nbcd" => Op::Nbcd,
        "tas" => Op::Tas,
        "mulu" => Op::MulDiv(0xc0c0),
        "muls" => Op::MulDiv(0xc1c0),
        "divu" => Op::MulDiv(0x80c0),
        "divs" => Op::MulDiv(0x81c0),
        "chk" => Op::MulDiv(0x4180),
        "abcd" => Op::Bcd(false),
        "sbcd" => Op::Bcd(true),
        "asl" => Op::Shift(0, true),
        "asr" => Op::Shift(0, false),
        "lsl" => Op::Shift(1, true),
        "lsr" => Op::Shift(1, false),
        "roxl" => Op::Shift(2, true),
        "roxr" => Op::Shift(2, false),
        "rol" => Op::Shift(3, true),
        "ror" => Op::Shift(3, false),
        "btst" => Op::Bit(0),
        "bchg" => Op::Bit(1),
        "bclr" => Op::Bit(2),
        "bset" => Op::Bit(3),
        "bra" => Op::Bcc(0),
        "bsr" => Op::Bcc(1),
        "jmp" => Op::Jmp,
        "jsr" => Op::Jsr,
        "trap" => Op::Trap,
        "stop" => Op::Stop,
        "nop" => Op::Simple(0x4e71),
        "rts" => Op::Simple(0x4e75),
        "rte" => Op::Simple(0x4e73),
        "rtr" => Op::Simple(0x4e77),
        "reset" => Op::Simple(0x4e70),
        "trapv" => Op::Simple(0x4e76),
        "illegal" => Op::Simple(0x4afc),
        "dbra" => Op::Dbcc(1),
        _ => {
            if let Some(cc) = mn.strip_prefix("db").and_then(condition) {
                Op::Dbcc(cc)
            } else if let Some(cc) = mn.strip_prefix('b').and_then(condition).filter(|&cc| cc >= 2) {
                // BT and BF are written BRA and BSR.
                Op::Bcc(cc)
            } else if mn == "bt" || mn == "bf" {
                return None;
            } else {
                Op::Scc(mn.strip_prefix('s').and_then(condition)?)
            }
        }
    })
}

pub struct EncodeCtx<'a> {
    pub opts: &'a Options,
    /// Address of the instruction.
    pub pc: Value,
}

#[derive(Default, Debug)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub fixups: Vec<Fixup>,
    pub warnings: Vec<String>,
}

struct Builder<'a> {
    ctx: &'a EncodeCtx<'a>,
    out: Encoded,
}

fn check(op: &Operand, allowed: u16) -> R<()> {
    if op.mode_mask() & allowed == 0 {
        Err(ERR_MODE.into())
    } else {
        Ok(())
    }
}

/// Size field in bits 6-7 of most opcodes.
fn size_bits(size: Size) -> R<u16> {
    match size {
        Size::Byte => Ok(0),
        Size::Word => Ok(1),
        Size::Long => Ok(2),
        Size::Short => Err(ERR_SIZE.into()),
    }
}

fn word_or_long(size: Option<Size>) -> R<Size> {
    match size {
        None | Some(Size::Word) => Ok(Size::Word),
        Some(Size::Long) => Ok(Size::Long),
        _ => Err(ERR_SIZE.into()),
    }
}

/// Requires the size to be absent or the given one.
fn only_size(size: Option<Size>, want: Size) -> R<()> {
    match size {
        None => Ok(()),
        Some(s) if s == want => Ok(()),
        _ => Err(ERR_SIZE.into()),
    }
}

fn two(ops: Vec<Operand>) -> R<(Operand, Operand)> {
    let mut it = ops.into_iter();
    match (it.next(), it.next(), it.next()) {
        (Some(a), Some(b), None) => Ok((a, b)),
        (Some(_), None, _) => Err("Expected comma after operand".into()),
        (None, ..) => Err(ERR_MODE.into()),
        _ => Err(ERR_EXTRA.into()),
    }
}

fn one(ops: Vec<Operand>) -> R<Operand> {
    let mut it = ops.into_iter();
    match (it.next(), it.next()) {
        (Some(a), None) => Ok(a),
        (Some(_), Some(_)) => Err(ERR_EXTRA.into()),
        _ => Err(ERR_MODE.into()),
    }
}

/// The expression of an operand that names a branch target.
fn target(op: Operand) -> R<Expr> {
    match op {
        Operand::Abs(e) | Operand::AbsShort(e) | Operand::AbsLong(e) => Ok(e),
        _ => Err(ERR_MODE.into()),
    }
}

impl<'a> Builder<'a> {
    fn word(&mut self, w: u16) {
        self.out.bytes.extend_from_slice(&w.to_be_bytes());
    }

    fn pos(&self) -> usize {
        self.out.bytes.len()
    }

    /// Address of the byte at `offset` within the instruction.
    fn addr(&self, offset: usize) -> Value {
        Value { n: self.ctx.pc.n.wrapping_add(offset as i32), base: self.ctx.pc.base }
    }

    /// Patches a value into the bytes emitted so far, or records a fixup if
    /// the value is not known yet.
    fn patch(&mut self, offset: usize, kind: FixKind, expr: &Expr, pc: Option<Value>) -> R<()> {
        if let Expr::Const(v) = expr {
            let known = match pc {
                None if v.is_abs() => Some(v.n),
                None => None,
                Some(pc) => match apply_binary(BinOp::Sub, *v, pc) {
                    Ok(d) if d.is_abs() => Some(d.n),
                    _ => None,
                },
            };
            if let Some(n) = known {
                return fixup::apply(kind, n, &mut self.out.bytes[offset..], &mut self.out.warnings);
            }
        }
        self.out.fixups.push(Fixup { offset, kind, expr: expr.clone(), pc });
        Ok(())
    }

    /// Emits an immediate operand of the given size.
    fn imm(&mut self, e: &Expr, size: Size) -> R<()> {
        let at = self.pos();
        match size {
            Size::Byte | Size::Short => {
                self.word(0);
                self.patch(at + 1, FixKind::Data8, e, None)
            }
            Size::Word => {
                self.word(0);
                self.patch(at, FixKind::Data16, e, None)
            }
            Size::Long => {
                self.word(0);
                self.word(0);
                self.patch(at, FixKind::Data32, e, None)
            }
        }
    }

    /// Emits the extension words of an effective address.
    fn ext(&mut self, op: &Operand, size: Size) -> R<()> {
        let at = self.pos();
        match op {
            Operand::Disp(_, e) => {
                self.word(0);
                self.patch(at, FixKind::Disp16, e, None)
            }
            Operand::Index(_, e, x) => {
                self.word((x.reg as u16) << 12 | if x.long { 0x0800 } else { 0 });
                self.patch(at + 1, FixKind::Disp8, e, None)
            }
            Operand::AbsShort(e) => {
                self.word(0);
                self.patch(at, FixKind::Abs16, e, None)
            }
            Operand::Abs(e) | Operand::AbsLong(e) => {
                self.word(0);
                self.word(0);
                self.patch(at, FixKind::Data32, e, None)
            }
            Operand::PcDisp(e) => {
                self.word(0);
                let pc = self.addr(at);
                self.patch(at, FixKind::PcDisp16, e, Some(pc))
            }
            Operand::PcIndex(e, x) => {
                self.word((x.reg as u16) << 12 | if x.long { 0x0800 } else { 0 });
                let pc = self.addr(at);
                self.patch(at + 1, FixKind::PcDisp8, e, Some(pc))
            }
            Operand::Imm(e) => self.imm(e, size),
            _ => Ok(()),
        }
    }

    /// Emits an opcode with an effective address in its low six bits.
    fn op_ea(&mut self, base: u16, ea: &Operand, size: Size) -> R<()> {
        let (m, r) = ea.ea_bits().ok_or(ERR_MODE)?;
        self.word(base | m << 3 | r);
        self.ext(ea, size)
    }

    /// Picks the form of unsized absolute addresses and applies the
    /// addressing mode optimisations. `ext_at` is the offset at which the
    /// extension words of the operand will be placed.
    fn optimise(&self, op: Operand, allowed: u16, ext_at: usize) -> Operand {
        let opts = self.ctx.opts;
        match op {
            Operand::Abs(e) => {
                if let Expr::Const(v) = &e {
                    if opts.opt_pc_rel && allowed & M_PCDISP != 0 {
                        if let Ok(d) = apply_binary(BinOp::Sub, *v, self.addr(ext_at)) {
                            if d.is_abs() && (-32768..=32767).contains(&d.n) {
                                return Operand::PcDisp(e);
                            }
                        }
                    }
                    if opts.opt_abs_word && v.is_abs() && (-32768..=32767).contains(&v.n) {
                        return Operand::AbsShort(e);
                    }
                }
                Operand::AbsLong(e)
            }
            Operand::Disp(r, e) => {
                if opts.opt_zero_disp && e.as_abs() == Some(0) {
                    Operand::Indirect(r)
                } else {
                    Operand::Disp(r, e)
                }
            }
            op => op,
        }
    }

    /// Optimises and validates an effective address operand.
    fn ea(&self, op: Operand, allowed: u16, ext_at: usize) -> R<Operand> {
        let op = self.optimise(op, allowed, ext_at);
        check(&op, allowed)?;
        Ok(op)
    }

    fn quick(&mut self, base: u16, count: &Expr) -> R<()> {
        let at = self.pos();
        self.word(base);
        self.patch(at, FixKind::Quick, count, None)
    }

    fn moveq(&mut self, e: &Expr, reg: u8) -> R<()> {
        let at = self.pos();
        self.word(0x7000 | (reg as u16) << 9);
        self.patch(at + 1, FixKind::Moveq, e, None)
    }

    fn encode(&mut self, op: Op, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        match op {
            Op::Simple(w) => {
                if !ops.is_empty() {
                    return Err(ERR_EXTRA.into());
                }
                self.word(w);
                Ok(())
            }
            Op::Move => self.enc_move(size, ops),
            Op::Movea => {
                let (src, dst) = two(ops)?;
                self.enc_movea(size, src, dst)
            }
            Op::Moveq => {
                only_size(size, Size::Long)?;
                match two(ops)? {
                    (Operand::Imm(e), Operand::DataReg(r)) => self.moveq(&e, r),
                    _ => Err(ERR_MODE.into()),
                }
            }
            Op::Movem => self.enc_movem(size, ops),
            Op::Movep => self.enc_movep(size, ops),
            Op::Lea => {
                only_size(size, Size::Long)?;
                let (src, dst) = two(ops)?;
                let Operand::AddrReg(r) = dst else { return Err(ERR_MODE.into()) };
                let src = self.ea(src, M_CONTROL, 2)?;
                self.op_ea(0x41c0 | (r as u16) << 9, &src, Size::Long)
            }
            Op::Pea => {
                only_size(size, Size::Long)?;
                let src = self.ea(one(ops)?, M_CONTROL, 2)?;
                self.op_ea(0x4840, &src, Size::Long)
            }
            Op::Jmp | Op::Jsr => {
                let src = self.ea(one(ops)?, M_CONTROL, 2)?;
                let base = if op == Op::Jmp { 0x4ec0 } else { 0x4e80 };
                self.op_ea(base, &src, Size::Long)
            }
            Op::Exg => {
                only_size(size, Size::Long)?;
                let w = match two(ops)? {
                    (Operand::DataReg(x), Operand::DataReg(y)) => 0xc140 | (x as u16) << 9 | y as u16,
                    (Operand::AddrReg(x), Operand::AddrReg(y)) => 0xc148 | (x as u16) << 9 | y as u16,
                    (Operand::DataReg(d), Operand::AddrReg(a))
                    | (Operand::AddrReg(a), Operand::DataReg(d)) => {
                        0xc188 | (d as u16) << 9 | a as u16
                    }
                    _ => return Err(ERR_MODE.into()),
                };
                self.word(w);
                Ok(())
            }
            Op::Swap => {
                only_size(size, Size::Word)?;
                let Operand::DataReg(r) = one(ops)? else { return Err(ERR_MODE.into()) };
                self.word(0x4840 | r as u16);
                Ok(())
            }
            Op::Ext => {
                let size = word_or_long(size)?;
                let Operand::DataReg(r) = one(ops)? else { return Err(ERR_MODE.into()) };
                self.word(if size == Size::Long { 0x48c0 } else { 0x4880 } | r as u16);
                Ok(())
            }
            Op::Link => match two(ops)? {
                (Operand::AddrReg(r), Operand::Imm(e)) => {
                    self.word(0x4e50 | r as u16);
                    self.imm(&e, Size::Word)
                }
                _ => Err(ERR_MODE.into()),
            },
            Op::Unlk => {
                let Operand::AddrReg(r) = one(ops)? else { return Err(ERR_MODE.into()) };
                self.word(0x4e58 | r as u16);
                Ok(())
            }
            Op::AddSub(sub) => self.enc_addsub(sub, size, ops),
            Op::AddSubA(sub) => {
                let (src, dst) = two(ops)?;
                self.enc_adda(sub, size, src, dst)
            }
            Op::AddSubI(sub) => {
                let (src, dst) = two(ops)?;
                self.enc_addi(sub, size, src, dst)
            }
            Op::AddSubQ(sub) => {
                let (src, dst) = two(ops)?;
                let Operand::Imm(e) = src else { return Err(ERR_MODE.into()) };
                self.enc_addq(sub, size, &e, dst)
            }
            Op::AddSubX(sub) => {
                let sz = size_bits(size.unwrap_or(Size::Word))?;
                let base = if sub { 0x9100 } else { 0xd100 };
                self.enc_extended(base | sz << 6, ops)
            }
            Op::Bcd(sub) => {
                only_size(size, Size::Byte)?;
                self.enc_extended(if sub { 0x8100 } else { 0xc100 }, ops)
            }
            Op::AndOr(or) => self.enc_andor(or, size, ops),
            Op::Eor => {
                let (src, dst) = two(ops)?;
                match src {
                    Operand::Imm(_) => self.enc_logic_imm(0x0a00, size, src, dst),
                    Operand::DataReg(r) => {
                        let size = size.unwrap_or(Size::Word);
                        let dst = self.ea(dst, M_DATA_ALT, 2)?;
                        self.op_ea(0xb100 | (r as u16) << 9 | size_bits(size)? << 6, &dst, size)
                    }
                    _ => Err(ERR_MODE.into()),
                }
            }
            Op::LogicI(base) => {
                let (src, dst) = two(ops)?;
                self.enc_logic_imm(base, size, src, dst)
            }
            Op::Cmp => self.enc_cmp(size, ops),
            Op::Cmpa => {
                let (src, dst) = two(ops)?;
                self.enc_cmpa(size, src, dst)
            }
            Op::Cmpi => {
                let (src, dst) = two(ops)?;
                self.enc_cmpi(size, src, dst)
            }
            Op::Cmpm => {
                let size = size.unwrap_or(Size::Word);
                match two(ops)? {
                    (Operand::PostInc(y), Operand::PostInc(x)) => {
                        self.word(0xb108 | (x as u16) << 9 | size_bits(size)? << 6 | y as u16);
                        Ok(())
                    }
                    _ => Err(ERR_MODE.into()),
                }
            }
            Op::Unary(base) => {
                let size = size.unwrap_or(Size::Word);
                let dst = self.ea(one(ops)?, M_DATA_ALT, 2)?;
                self.op_ea(base | size_bits(size)? << 6, &dst, size)
            }
            Op::Nbcd => {
                only_size(size, Size::Byte)?;
                let dst = self.ea(one(ops)?, M_DATA_ALT, 2)?;
                self.op_ea(0x4800, &dst, Size::Byte)
            }
            Op::Tas => {
                only_size(size, Size::Byte)?;
                let dst = self.ea(one(ops)?, M_DATA_ALT, 2)?;
                self.op_ea(0x4ac0, &dst, Size::Byte)
            }
            Op::Scc(cc) => {
                only_size(size, Size::Byte)?;
                let dst = self.ea(one(ops)?, M_DATA_ALT, 2)?;
                self.op_ea(0x50c0 | (cc as u16) << 8, &dst, Size::Byte)
            }
            Op::MulDiv(base) => {
                only_size(size, Size::Word)?;
                let (src, dst) = two(ops)?;
                let Operand::DataReg(r) = dst else { return Err(ERR_MODE.into()) };
                let src = self.ea(src, M_DATA, 2)?;
                self.op_ea(base | (r as u16) << 9, &src, Size::Word)
            }
            Op::Shift(kind, left) => self.enc_shift(kind, left, size, ops),
            Op::Bit(kind) => self.enc_bit(kind, ops),
            Op::Bcc(cc) => self.enc_branch(cc, size, ops),
            Op::Dbcc(cc) => {
                only_size(size, Size::Word)?;
                let (reg, dest) = two(ops)?;
                let Operand::DataReg(r) = reg else { return Err(ERR_MODE.into()) };
                let e = target(dest)?;
                self.word(0x50c8 | (cc as u16) << 8 | r as u16);
                self.word(0);
                let pc = self.addr(2);
                self.patch(2, FixKind::Branch16, &e, Some(pc))
            }
            Op::Trap => {
                let Operand::Imm(e) = one(ops)? else { return Err(ERR_MODE.into()) };
                self.word(0x4e40);
                self.patch(1, FixKind::Trap, &e, None)
            }
            Op::Stop => {
                let Operand::Imm(e) = one(ops)? else { return Err(ERR_MODE.into()) };
                self.word(0x4e72);
                self.imm(&e, Size::Word)
            }
        }
    }

    fn enc_move(&mut self, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let (src, dst) = two(ops)?;
        match (&src, &dst) {
            (Operand::Sr, _) => {
                only_size(size, Size::Word)?;
                let dst = self.ea(dst, M_DATA_ALT, 2)?;
                self.op_ea(0x40c0, &dst, Size::Word)
            }
            (_, Operand::Sr) => {
                only_size(size, Size::Word)?;
                let src = self.ea(src, M_DATA, 2)?;
                self.op_ea(0x46c0, &src, Size::Word)
            }
            (_, Operand::Ccr) => {
                if !matches!(size, None | Some(Size::Word) | Some(Size::Byte)) {
                    return Err(ERR_SIZE.into());
                }
                let src = self.ea(src, M_DATA, 2)?;
                self.op_ea(0x44c0, &src, Size::Word)
            }
            (Operand::Usp, Operand::AddrReg(r)) => {
                only_size(size, Size::Long)?;
                self.word(0x4e68 | *r as u16);
                Ok(())
            }
            (Operand::AddrReg(r), Operand::Usp) => {
                only_size(size, Size::Long)?;
                self.word(0x4e60 | *r as u16);
                Ok(())
            }
            (_, Operand::AddrReg(_)) => self.enc_movea(size, src, dst),
            _ => {
                let size = size.unwrap_or(Size::Word);
                if self.ctx.opts.opt_moveq && size == Size::Long {
                    if let (Operand::Imm(e), Operand::DataReg(r)) = (&src, &dst) {
                        if matches!(e.as_abs(), Some(-128..=127)) {
                            return self.moveq(e, *r);
                        }
                    }
                }
                let src_modes = if size == Size::Byte { M_DATA } else { M_ALL };
                let src = self.ea(src, src_modes, 2)?;
                let dst = self.optimise(dst, M_DATA_ALT, 2);
                check(&dst, M_DATA_ALT)?;
                let sz = match size {
                    Size::Byte => 1,
                    Size::Long => 2,
                    Size::Word => 3,
                    Size::Short => return Err(ERR_SIZE.into()),
                };
                let (sm, sr) = src.ea_bits().ok_or(ERR_MODE)?;
                let (dm, dr) = dst.ea_bits().ok_or(ERR_MODE)?;
                self.word(sz << 12 | dr << 9 | dm << 6 | sm << 3 | sr);
                self.ext(&src, size)?;
                self.ext(&dst, size)
            }
        }
    }

    fn enc_movea(&mut self, size: Option<Size>, src: Operand, dst: Operand) -> R<()> {
        if size == Some(Size::Byte) {
            return Err(ERR_MODE.into());
        }
        let size = word_or_long(size)?;
        let Operand::AddrReg(r) = dst else { return Err(ERR_MODE.into()) };
        let src = self.ea(src, M_ALL, 2)?;
        let sz = if size == Size::Long { 2 } else { 3 };
        self.op_ea(sz << 12 | (r as u16) << 9 | 1 << 6, &src, size)
    }

    fn enc_movem(&mut self, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let size = word_or_long(size)?;
        let sz = if size == Size::Long { 0x40 } else { 0 };
        let (a, b) = two(ops)?;
        let mask_of = |op: &Operand| match op {
            Operand::RegList(m) => Some(*m),
            Operand::DataReg(r) => Some(1u16 << r),
            Operand::AddrReg(r) => Some(1u16 << (r + 8)),
            _ => None,
        };
        // A lone register is a register list when the other operand is
        // memory, and the memory operand otherwise is an error.
        let a_is_list = matches!(a, Operand::RegList(_));
        let b_is_list = matches!(b, Operand::RegList(_));
        let to_memory = if a_is_list {
            true
        } else if b_is_list {
            false
        } else {
            mask_of(&a).is_some() && mask_of(&b).is_none()
        };
        if to_memory {
            let mask = mask_of(&a).ok_or("Error in register list")?;
            let ea = self.ea(b, M_CONTROL_ALT | M_PREDEC, 4)?;
            let mask = if matches!(ea, Operand::PreDec(_)) { mask.reverse_bits() } else { mask };
            let (m, r) = ea.ea_bits().ok_or(ERR_MODE)?;
            self.word(0x4880 | sz | m << 3 | r);
            self.word(mask);
            self.ext(&ea, size)
        } else {
            let mask = mask_of(&b).ok_or(ERR_MODE)?;
            let ea = self.ea(a, M_CONTROL | M_POSTINC, 4)?;
            let (m, r) = ea.ea_bits().ok_or(ERR_MODE)?;
            self.word(0x4c80 | sz | m << 3 | r);
            self.word(mask);
            self.ext(&ea, size)
        }
    }

    fn enc_movep(&mut self, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let size = word_or_long(size)?;
        let sz = if size == Size::Long { 0x40 } else { 0 };
        let mem = |op: Operand| match op {
            Operand::Disp(r, e) => Ok((r, e)),
            Operand::Indirect(r) => Ok((r, Expr::num(0))),
            _ => Err(ERR_MODE.to_string()),
        };
        let (a, b) = two(ops)?;
        let (w, e) = match (a, b) {
            (Operand::DataReg(d), m) => {
                let (r, e) = mem(m)?;
                (0x0188 | (d as u16) << 9 | sz | r as u16, e)
            }
            (m, Operand::DataReg(d)) => {
                let (r, e) = mem(m)?;
                (0x0108 | (d as u16) << 9 | sz | r as u16, e)
            }
            _ => return Err(ERR_MODE.into()),
        };
        self.word(w);
        self.word(0);
        self.patch(2, FixKind::Disp16, &e, None)
    }

    fn enc_addsub(&mut self, sub: bool, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let (src, dst) = two(ops)?;
        match (&src, &dst) {
            (_, Operand::AddrReg(_)) => self.enc_adda(sub, size, src, dst),
            (Operand::Imm(_), _) => self.enc_addi(sub, size, src, dst),
            (_, Operand::DataReg(r)) => {
                let size = size.unwrap_or(Size::Word);
                let modes = if size == Size::Byte { M_DATA } else { M_ALL };
                let src = self.ea(src, modes, 2)?;
                let base = if sub { 0x9000 } else { 0xd000 };
                self.op_ea(base | (*r as u16) << 9 | size_bits(size)? << 6, &src, size)
            }
            (Operand::DataReg(r), _) => {
                let size = size.unwrap_or(Size::Word);
                let r = *r;
                let dst = self.ea(dst, M_MEM_ALT, 2)?;
                let base = if sub { 0x9100 } else { 0xd100 };
                self.op_ea(base | (r as u16) << 9 | size_bits(size)? << 6, &dst, size)
            }
            _ => Err(ERR_MODE.into()),
        }
    }

    /// Whether an ADD or SUB of a constant may be turned into its quick form.
    fn can_quicken(&self, sub: bool, e: &Expr) -> bool {
        let enabled = if sub { self.ctx.opts.opt_subq } else { self.ctx.opts.opt_addq };
        enabled && matches!(e.as_abs(), Some(1..=8))
    }

    fn enc_adda(&mut self, sub: bool, size: Option<Size>, src: Operand, dst: Operand) -> R<()> {
        let size = word_or_long(size)?;
        let Operand::AddrReg(r) = dst else { return Err(ERR_MODE.into()) };
        if let Operand::Imm(e) = &src {
            if self.can_quicken(sub, e) {
                return self.enc_addq(sub, Some(size), e, dst);
            }
        }
        let src = self.ea(src, M_ALL, 2)?;
        let base = if sub { 0x90c0 } else { 0xd0c0 };
        let sz = if size == Size::Long { 0x100 } else { 0 };
        self.op_ea(base | sz | (r as u16) << 9, &src, size)
    }

    fn enc_addi(&mut self, sub: bool, size: Option<Size>, src: Operand, dst: Operand) -> R<()> {
        if matches!(dst, Operand::AddrReg(_)) && matches!(src, Operand::Imm(_)) {
            return self.enc_adda(sub, size, src, dst);
        }
        let Operand::Imm(e) = src else { return Err(ERR_MODE.into()) };
        if self.can_quicken(sub, &e) {
            return self.enc_addq(sub, size, &e, dst);
        }
        let size = size.unwrap_or(Size::Word);
        let imm_len = if size == Size::Long { 4 } else { 2 };
        let dst = self.ea(dst, M_DATA_ALT, 2 + imm_len)?;
        let (m, r) = dst.ea_bits().ok_or(ERR_MODE)?;
        let base = if sub { 0x0400 } else { 0x0600 };
        self.word(base | size_bits(size)? << 6 | m << 3 | r);
        self.imm(&e, size)?;
        self.ext(&dst, size)
    }

    fn enc_addq(&mut self, sub: bool, size: Option<Size>, e: &Expr, dst: Operand) -> R<()> {
        let size = size.unwrap_or(Size::Word);
        let modes = if size == Size::Byte { M_ALTERABLE & !M_AN } else { M_ALTERABLE };
        let dst = self.ea(dst, modes, 2)?;
        let (m, r) = dst.ea_bits().ok_or(ERR_MODE)?;
        let base = if sub { 0x5100 } else { 0x5000 };
        self.quick(base | size_bits(size)? << 6 | m << 3 | r, e)?;
        self.ext(&dst, size)
    }

    /// ADDX, SUBX, ABCD and SBCD.
    fn enc_extended(&mut self, base: u16, ops: Vec<Operand>) -> R<()> {
        let w = match two(ops)? {
            (Operand::DataReg(y), Operand::DataReg(x)) => base | (x as u16) << 9 | y as u16,
            (Operand::PreDec(y), Operand::PreDec(x)) => base | 8 | (x as u16) << 9 | y as u16,
            _ => return Err(ERR_MODE.into()),
        };
        self.word(w);
        Ok(())
    }

    fn enc_andor(&mut self, or: bool, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let (src, dst) = two(ops)?;
        match (&src, &dst) {
            (Operand::Imm(_), _) => self.enc_logic_imm(if or { 0x0000 } else { 0x0200 }, size, src, dst),
            (_, Operand::DataReg(r)) => {
                let size = size.unwrap_or(Size::Word);
                let src = self.ea(src, M_DATA, 2)?;
                let base = if or { 0x8000 } else { 0xc000 };
                self.op_ea(base | (*r as u16) << 9 | size_bits(size)? << 6, &src, size)
            }
            (Operand::DataReg(r), _) => {
                let size = size.unwrap_or(Size::Word);
                let r = *r;
                let dst = self.ea(dst, M_MEM_ALT, 2)?;
                let base = if or { 0x8100 } else { 0xc100 };
                self.op_ea(base | (r as u16) << 9 | size_bits(size)? << 6, &dst, size)
            }
            _ => Err(ERR_MODE.into()),
        }
    }

    /// ANDI, ORI and EORI, including the forms that operate on CCR and SR.
    fn enc_logic_imm(&mut self, base: u16, size: Option<Size>, src: Operand, dst: Operand) -> R<()> {
        let Operand::Imm(e) = src else { return Err(ERR_MODE.into()) };
        match dst {
            Operand::Ccr => {
                if !matches!(size, None | Some(Size::Byte) | Some(Size::Word)) {
                    return Err(ERR_SIZE.into());
                }
                self.word(base | 0x3c);
                self.imm(&e, Size::Byte)
            }
            Operand::Sr => {
                only_size(size, Size::Word)?;
                self.word(base | 0x7c);
                self.imm(&e, Size::Word)
            }
            dst => {
                let size = size.unwrap_or(Size::Word);
                let imm_len = if size == Size::Long { 4 } else { 2 };
                let dst = self.ea(dst, M_DATA_ALT, 2 + imm_len)?;
                let (m, r) = dst.ea_bits().ok_or(ERR_MODE)?;
                self.word(base | size_bits(size)? << 6 | m << 3 | r);
                self.imm(&e, size)?;
                self.ext(&dst, size)
            }
        }
    }

    fn enc_cmp(&mut self, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let (src, dst) = two(ops)?;
        match (&src, &dst) {
            (_, Operand::AddrReg(_)) => self.enc_cmpa(size, src, dst),
            (Operand::PostInc(y), Operand::PostInc(x)) => {
                let size = size.unwrap_or(Size::Word);
                self.word(0xb108 | (*x as u16) << 9 | size_bits(size)? << 6 | *y as u16);
                Ok(())
            }
            (Operand::Imm(_), _) => self.enc_cmpi(size, src, dst),
            (_, Operand::DataReg(r)) => {
                let size = size.unwrap_or(Size::Word);
                let modes = if size == Size::Byte { M_DATA } else { M_ALL };
                let src = self.ea(src, modes, 2)?;
                self.op_ea(0xb000 | (*r as u16) << 9 | size_bits(size)? << 6, &src, size)
            }
            _ => Err(ERR_MODE.into()),
        }
    }

    fn enc_cmpa(&mut self, size: Option<Size>, src: Operand, dst: Operand) -> R<()> {
        let size = word_or_long(size)?;
        let Operand::AddrReg(r) = dst else { return Err(ERR_MODE.into()) };
        let src = self.ea(src, M_ALL, 2)?;
        let sz = if size == Size::Long { 0x100 } else { 0 };
        self.op_ea(0xb0c0 | sz | (r as u16) << 9, &src, size)
    }

    fn enc_cmpi(&mut self, size: Option<Size>, src: Operand, dst: Operand) -> R<()> {
        if matches!(dst, Operand::AddrReg(_)) && matches!(src, Operand::Imm(_)) {
            return self.enc_cmpa(size, src, dst);
        }
        let Operand::Imm(e) = src else { return Err(ERR_MODE.into()) };
        let size = size.unwrap_or(Size::Word);
        let imm_len = if size == Size::Long { 4 } else { 2 };
        let dst = self.ea(dst, M_DATA_ALT, 2 + imm_len)?;
        let (m, r) = dst.ea_bits().ok_or(ERR_MODE)?;
        self.word(0x0c00 | size_bits(size)? << 6 | m << 3 | r);
        self.imm(&e, size)?;
        self.ext(&dst, size)
    }

    fn enc_shift(&mut self, kind: u8, left: bool, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let kind = kind as u16;
        let dir = if left { 0x100 } else { 0 };
        if ops.len() == 1 {
            let dst = one(ops)?;
            if let Operand::DataReg(r) = dst {
                // A lone data register is shifted by one.
                let sz = size_bits(size.unwrap_or(Size::Word))?;
                self.word(0xe000 | 1 << 9 | dir | sz << 6 | kind << 3 | r as u16);
                return Ok(());
            }
            only_size(size, Size::Word)?;
            let dst = self.ea(dst, M_MEM_ALT, 2)?;
            return self.op_ea(0xe0c0 | kind << 9 | dir, &dst, Size::Word);
        }
        let sz = size_bits(size.unwrap_or(Size::Word))?;
        match two(ops)? {
            (Operand::Imm(e), Operand::DataReg(r)) => {
                self.quick(0xe000 | dir | sz << 6 | kind << 3 | r as u16, &e)
            }
            (Operand::DataReg(c), Operand::DataReg(r)) => {
                self.word(0xe020 | (c as u16) << 9 | dir | sz << 6 | kind << 3 | r as u16);
                Ok(())
            }
            _ => Err(ERR_MODE.into()),
        }
    }

    fn enc_bit(&mut self, kind: u8, ops: Vec<Operand>) -> R<()> {
        let kind = kind as u16;
        let (src, dst) = two(ops)?;
        match src {
            Operand::DataReg(r) => {
                let modes = if kind == 0 { M_DATA } else { M_DATA_ALT };
                let dst = self.ea(dst, modes, 2)?;
                self.op_ea(0x0100 | (r as u16) << 9 | kind << 6, &dst, Size::Byte)
            }
            Operand::Imm(e) => {
                // asm68k lets BTST test an immediate, so this does too.
                let modes = if kind == 0 { M_DATA } else { M_DATA_ALT };
                let dst = self.ea(dst, modes, 4)?;
                let (m, r) = dst.ea_bits().ok_or(ERR_MODE)?;
                self.word(0x0800 | kind << 6 | m << 3 | r);
                self.word(0);
                // The bit number is not range checked beyond fitting a byte.
                match e.as_abs() {
                    Some(n) => self.out.bytes[3] = n as u8,
                    None => self.patch(3, FixKind::Data8, &e, None)?,
                }
                self.ext(&dst, Size::Byte)
            }
            _ => Err(ERR_MODE.into()),
        }
    }

    fn enc_branch(&mut self, cc: u8, size: Option<Size>, ops: Vec<Operand>) -> R<()> {
        let e = target(one(ops)?)?;
        let pc = self.addr(2);
        let short = match size {
            Some(Size::Short) | Some(Size::Byte) => true,
            Some(Size::Word) => false,
            Some(Size::Long) => return Err(ERR_SIZE.into()),
            None => {
                // Only targets that are already known are optimised.
                self.ctx.opts.opt_short_branch
                    && match &e {
                        Expr::Const(v) => match apply_binary(BinOp::Sub, *v, pc) {
                            Ok(d) => d.is_abs() && d.n != 0 && (-128..=127).contains(&d.n),
                            Err(_) => false,
                        },
                        _ => false,
                    }
            }
        };
        self.word(0x6000 | (cc as u16) << 8);
        if short {
            self.patch(1, FixKind::Branch8, &e, Some(pc))
        } else {
            self.word(0);
            self.patch(2, FixKind::Branch16, &e, Some(pc))
        }
    }
}

/// Encodes an instruction.
pub fn encode(op: Op, size: Option<Size>, ops: Vec<Operand>, ctx: &EncodeCtx) -> R<Encoded> {
    let mut b = Builder { ctx, out: Encoded::default() };
    b.encode(op, size, ops)?;
    Ok(b.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_with(opts: &Options, mn: &str, size: Option<Size>, ops: Vec<Operand>) -> R<Encoded> {
        let ctx = EncodeCtx { opts, pc: Value::abs(0x1000) };
        encode(lookup(mn).unwrap(), size, ops, &ctx)
    }

    fn enc(mn: &str, size: Option<Size>, ops: Vec<Operand>) -> Vec<u8> {
        enc_with(&Options::default(), mn, size, ops).unwrap().bytes
    }

    fn imm(n: i32) -> Operand {
        Operand::Imm(Expr::num(n))
    }

    fn abs(n: i32) -> Operand {
        Operand::Abs(Expr::num(n))
    }

    use Operand::*;
    use Size::*;

    #[test]
    fn mnemonic_lookup() {
        assert_eq!(lookup("bhs"), Some(Op::Bcc(4)));
        assert_eq!(lookup("blo"), Some(Op::Bcc(5)));
        assert_eq!(lookup("dbra"), Some(Op::Dbcc(1)));
        assert_eq!(lookup("dbf"), Some(Op::Dbcc(1)));
        assert_eq!(lookup("st"), Some(Op::Scc(0)));
        assert_eq!(lookup("bt"), None);
        assert_eq!(lookup("bset"), Some(Op::Bit(3)));
        assert_eq!(lookup("sbcd"), Some(Op::Bcd(true)));
        assert_eq!(lookup("frob"), None);
    }

    #[test]
    fn moves() {
        assert_eq!(enc("move", Some(Word), vec![DataReg(0), PostInc(0)]), [0x30, 0xc0]);
        assert_eq!(enc("move", Some(Long), vec![DataReg(0), AddrReg(0)]), [0x20, 0x40]);
        assert_eq!(enc("move", None, vec![Sr, DataReg(0)]), [0x40, 0xc0]);
        assert_eq!(enc("move", None, vec![DataReg(0), Ccr]), [0x44, 0xc0]);
        assert_eq!(enc("move", Some(Long), vec![AddrReg(0), Usp]), [0x4e, 0x60]);
        assert_eq!(
            enc("move", Some(Byte), vec![imm(-1), DataReg(0)]),
            [0x10, 0x3c, 0x00, 0xff]
        );
        assert_eq!(
            enc("move", Some(Word), vec![abs(0x1234), DataReg(0)]),
            [0x30, 0x39, 0x00, 0x00, 0x12, 0x34]
        );
        assert_eq!(
            enc("move", Some(Word), vec![AbsShort(Expr::num(-0x8000)), DataReg(0)]),
            [0x30, 0x38, 0x80, 0x00]
        );
        assert_eq!(enc("moveq", None, vec![imm(-1), DataReg(0)]), [0x70, 0xff]);
        assert_eq!(enc("movea", Some(Long), vec![AbsShort(Expr::num(4)), AddrReg(6)]), [0x2c, 0x78, 0, 4]);
    }

    #[test]
    fn movem_masks() {
        assert_eq!(
            enc("movem", Some(Long), vec![RegList(0x7fff), PreDec(7)]),
            [0x48, 0xe7, 0xff, 0xfe]
        );
        assert_eq!(
            enc("movem", Some(Long), vec![PostInc(7), RegList(0x7fff)]),
            [0x4c, 0xdf, 0x7f, 0xff]
        );
        assert_eq!(enc("movem", Some(Word), vec![DataReg(0), PreDec(7)]), [0x48, 0xa7, 0x80, 0x00]);
        assert_eq!(
            enc("movem", Some(Long), vec![RegList(0x0101), Disp(0, Expr::num(4))]),
            [0x48, 0xe8, 0x01, 0x01, 0x00, 0x04]
        );
    }

    #[test]
    fn arithmetic_picks_immediate_forms() {
        assert_eq!(enc("add", Some(Word), vec![imm(1), DataReg(0)]), [0x06, 0x40, 0, 1]);
        assert_eq!(enc("add", Some(Word), vec![imm(1), AddrReg(0)]), [0xd0, 0xfc, 0, 1]);
        assert_eq!(enc("sub", Some(Word), vec![imm(1), DataReg(0)]), [0x04, 0x40, 0, 1]);
        assert_eq!(enc("and", Some(Word), vec![imm(1), DataReg(0)]), [0x02, 0x40, 0, 1]);
        assert_eq!(enc("or", Some(Word), vec![imm(1), DataReg(0)]), [0x00, 0x40, 0, 1]);
        assert_eq!(enc("eor", Some(Word), vec![imm(1), DataReg(0)]), [0x0a, 0x40, 0, 1]);
        assert_eq!(enc("cmp", Some(Word), vec![imm(1), DataReg(0)]), [0x0c, 0x40, 0, 1]);
        assert_eq!(enc("cmp", Some(Word), vec![imm(1), AddrReg(0)]), [0xb0, 0xfc, 0, 1]);
        assert_eq!(enc("cmp", Some(Word), vec![PostInc(0), PostInc(1)]), [0xb3, 0x48]);
        assert_eq!(enc("add", None, vec![DataReg(0), DataReg(1)]), [0xd2, 0x40]);
        assert_eq!(enc("add", Some(Long), vec![DataReg(0), AddrReg(0)]), [0xd1, 0xc0]);
        assert_eq!(enc("and", None, vec![imm(0xfe), Ccr]), [0x02, 0x3c, 0x00, 0xfe]);
        // Immediate forms with an address register use the address forms.
        assert_eq!(enc("subi", Some(Word), vec![imm(0x20), AddrReg(5)]), [0x9a, 0xfc, 0, 0x20]);
        assert_eq!(enc("addi", Some(Long), vec![imm(0x20), AddrReg(5)]), [0xdb, 0xfc, 0, 0, 0, 0x20]);
        assert_eq!(enc("cmpi", Some(Word), vec![imm(0x20), AddrReg(5)]), [0xba, 0xfc, 0, 0x20]);
        assert_eq!(enc("or", None, vec![imm(0x700), Sr]), [0x00, 0x7c, 0x07, 0x00]);
    }

    #[test]
    fn quick_forms() {
        assert_eq!(enc("addq", None, vec![imm(8), DataReg(0)]), [0x50, 0x40]);
        assert_eq!(enc("addq", Some(Long), vec![imm(1), AddrReg(0)]), [0x52, 0x88]);
        assert_eq!(enc("subq", None, vec![imm(1), DataReg(0)]), [0x53, 0x40]);
        let o = Options { opt_addq: true, opt_moveq: true, ..Options::default() };
        assert_eq!(enc_with(&o, "add", Some(Word), vec![imm(1), DataReg(0)]).unwrap().bytes, [0x52, 0x40]);
        assert_eq!(enc_with(&o, "add", Some(Word), vec![imm(9), DataReg(0)]).unwrap().bytes, [0x06, 0x40, 0, 9]);
        assert_eq!(enc_with(&o, "add", Some(Long), vec![imm(1), AddrReg(0)]).unwrap().bytes, [0x52, 0x88]);
        assert_eq!(enc_with(&o, "move", Some(Long), vec![imm(-1), DataReg(0)]).unwrap().bytes, [0x70, 0xff]);
        assert_eq!(
            enc_with(&o, "move", Some(Long), vec![imm(0x80), DataReg(0)]).unwrap().bytes,
            [0x20, 0x3c, 0, 0, 0, 0x80]
        );
    }

    #[test]
    fn shifts_and_bits() {
        assert_eq!(enc("lsl", Some(Word), vec![imm(1), DataReg(0)]), [0xe3, 0x48]);
        assert_eq!(enc("lsl", Some(Word), vec![imm(8), DataReg(0)]), [0xe1, 0x48]);
        assert_eq!(enc("lsl", Some(Word), vec![DataReg(1), DataReg(0)]), [0xe3, 0x68]);
        assert_eq!(enc("lsl", None, vec![Indirect(0)]), [0xe3, 0xd0]);
        assert_eq!(enc("asl", Some(Word), vec![DataReg(0)]), [0xe3, 0x40]);
        assert_eq!(enc("btst", None, vec![imm(0), DataReg(0)]), [0x08, 0x00, 0, 0]);
        assert_eq!(enc("btst", None, vec![DataReg(1), Indirect(0)]), [0x03, 0x10]);
        assert_eq!(enc("bset", None, vec![imm(7), Indirect(0)]), [0x08, 0xd0, 0, 7]);
    }

    #[test]
    fn branches() {
        // Instruction is at $1000.
        assert_eq!(enc("bra", None, vec![abs(0x1000)]), [0x60, 0x00, 0xff, 0xfe]);
        assert_eq!(enc("bra", Some(Short), vec![abs(0x1010)]), [0x60, 0x0e]);
        assert_eq!(enc("dbra", None, vec![DataReg(0), abs(0x1000)]), [0x51, 0xc8, 0xff, 0xfe]);
        assert_eq!(enc("bhs", Some(Short), vec![abs(0x1004)]), [0x64, 0x02]);
        assert!(enc_with(&Options::default(), "bra", Some(Short), vec![abs(0x1002)]).is_err());
        assert!(enc_with(&Options::default(), "bra", Some(Long), vec![abs(0x1002)]).is_err());
        let o = Options { opt_short_branch: true, ..Options::default() };
        assert_eq!(enc_with(&o, "bra", None, vec![abs(0x0ff0)]).unwrap().bytes, [0x60, 0xee]);
        assert_eq!(enc_with(&o, "bra", Some(Word), vec![abs(0x0ff0)]).unwrap().bytes, [0x60, 0x00, 0xff, 0xee]);
    }

    #[test]
    fn unknown_values_become_fixups() {
        let e = enc_with(&Options::default(), "bra", None, vec![Abs(Expr::Sym(1))]).unwrap();
        assert_eq!(e.bytes, [0x60, 0x00, 0x00, 0x00]);
        assert_eq!(e.fixups.len(), 1);
        assert_eq!(e.fixups[0].kind, FixKind::Branch16);
        assert_eq!(e.fixups[0].offset, 2);
        assert_eq!(e.fixups[0].pc, Some(Value::abs(0x1002)));

        let e = enc_with(&Options::default(), "jmp", None, vec![Abs(Expr::Sym(1))]).unwrap();
        assert_eq!(e.bytes, [0x4e, 0xf9, 0, 0, 0, 0]);
        assert_eq!(e.fixups[0].kind, FixKind::Data32);
    }

    #[test]
    fn address_optimisations() {
        let o = Options { opt_abs_word: true, ..Options::default() };
        assert_eq!(enc_with(&o, "jmp", None, vec![abs(0x120)]).unwrap().bytes, [0x4e, 0xf8, 0x01, 0x20]);
        assert_eq!(
            enc_with(&o, "move", Some(Word), vec![abs(0x8000), DataReg(0)]).unwrap().bytes,
            [0x30, 0x39, 0, 0, 0x80, 0]
        );
        // Explicitly sized addresses are left alone.
        assert_eq!(
            enc_with(&o, "move", Some(Word), vec![AbsLong(Expr::num(0x120)), DataReg(0)]).unwrap().bytes,
            [0x30, 0x39, 0, 0, 0x01, 0x20]
        );
        let o = Options { opt_pc_rel: true, ..Options::default() };
        assert_eq!(enc_with(&o, "lea", None, vec![abs(0x1000), AddrReg(0)]).unwrap().bytes, [0x41, 0xfa, 0xff, 0xfe]);
        // Destinations cannot be PC relative.
        assert_eq!(
            enc_with(&o, "move", Some(Word), vec![DataReg(0), abs(0x1000)]).unwrap().bytes,
            [0x33, 0xc0, 0, 0, 0x10, 0]
        );
        let o = Options { opt_zero_disp: true, ..Options::default() };
        assert_eq!(
            enc_with(&o, "move", Some(Word), vec![Disp(0, Expr::num(0)), DataReg(0)]).unwrap().bytes,
            [0x30, 0x10]
        );
    }

    #[test]
    fn indexed_modes() {
        let x = IndexReg { reg: 1, long: false };
        assert_eq!(
            enc("move", Some(Word), vec![Index(0, Expr::num(4), x), DataReg(0)]),
            [0x30, 0x30, 0x10, 0x04]
        );
        let x = IndexReg { reg: 0, long: true };
        // lea $1000(pc,d0.l),a0 at $1000: displacement is relative to $1002
        assert_eq!(
            enc("lea", None, vec![PcIndex(Expr::num(0x1000), x), AddrReg(0)]),
            [0x41, 0xfb, 0x08, 0xfe]
        );
    }

    #[test]
    fn illegal_modes_are_rejected() {
        let o = Options::default();
        assert!(enc_with(&o, "move", Some(Byte), vec![DataReg(0), AddrReg(0)]).is_err());
        assert!(enc_with(&o, "move", Some(Byte), vec![AddrReg(0), DataReg(0)]).is_err());
        assert!(enc_with(&o, "addq", Some(Byte), vec![imm(1), AddrReg(0)]).is_err());
        assert!(enc_with(&o, "lea", None, vec![DataReg(0), AddrReg(0)]).is_err());
        assert!(enc_with(&o, "clr", None, vec![PcDisp(Expr::num(0))]).is_err());
        assert!(enc_with(&o, "move", None, vec![DataReg(0)]).is_err());
    }

    #[test]
    fn misc() {
        assert_eq!(enc("exg", None, vec![DataReg(0), AddrReg(1)]), [0xc1, 0x89]);
        assert_eq!(enc("exg", None, vec![AddrReg(1), DataReg(0)]), [0xc1, 0x89]);
        assert_eq!(enc("link", None, vec![AddrReg(6), imm(-4)]), [0x4e, 0x56, 0xff, 0xfc]);
        assert_eq!(enc("trap", None, vec![imm(15)]), [0x4e, 0x4f]);
        assert_eq!(enc("stop", None, vec![imm(0x2700)]), [0x4e, 0x72, 0x27, 0x00]);
        assert_eq!(enc("movep", Some(Long), vec![DataReg(0), Disp(0, Expr::num(4))]), [0x01, 0xc8, 0, 4]);
        assert_eq!(enc("movep", Some(Word), vec![Disp(0, Expr::num(0)), DataReg(0)]), [0x01, 0x08, 0, 0]);
        assert_eq!(enc("muls", Some(Word), vec![imm(5), DataReg(1)]), [0xc3, 0xfc, 0, 5]);
        assert_eq!(enc("chk", None, vec![DataReg(0), DataReg(1)]), [0x43, 0x80]);
        assert_eq!(enc("subx", Some(Long), vec![PreDec(0), PreDec(1)]), [0x93, 0x88]);
        assert_eq!(enc("tas", None, vec![DataReg(0)]), [0x4a, 0xc0]);
        assert_eq!(enc("st", None, vec![Indirect(0)]), [0x50, 0xd0]);
    }
}
