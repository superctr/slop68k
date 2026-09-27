//! 68000 operands and registers.

use crate::expr::Expr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Byte,
    Word,
    Long,
    /// Short branch (`.s`).
    Short,
}

impl Size {
    pub fn parse(s: &str) -> Option<Size> {
        match s {
            "b" | "B" => Some(Size::Byte),
            "w" | "W" => Some(Size::Word),
            "l" | "L" => Some(Size::Long),
            "s" | "S" => Some(Size::Short),
            _ => None,
        }
    }

    pub fn bytes(self) -> u32 {
        match self {
            Size::Byte | Size::Short => 1,
            Size::Word => 2,
            Size::Long => 4,
        }
    }
}

/// A register as it can appear on its own in an operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reg {
    Data(u8),
    Addr(u8),
    Pc,
    Sr,
    Ccr,
    Usp,
}

impl Reg {
    /// Register number in the 0-15 numbering used by MOVEM masks and index
    /// registers, for data and address registers.
    pub fn number(self) -> Option<u8> {
        match self {
            Reg::Data(n) => Some(n),
            Reg::Addr(n) => Some(n + 8),
            _ => None,
        }
    }

    pub fn from_number(n: u8) -> Reg {
        if n < 8 {
            Reg::Data(n)
        } else {
            Reg::Addr(n - 8)
        }
    }

    /// Recognises the built-in register names.
    pub fn parse(name: &str) -> Option<Reg> {
        let b = name.as_bytes();
        match b.len() {
            2 => {
                let c = b[0].to_ascii_lowercase();
                let d = b[1];
                if (b'0'..=b'7').contains(&d) {
                    if c == b'd' {
                        return Some(Reg::Data(d - b'0'));
                    }
                    if c == b'a' {
                        return Some(Reg::Addr(d - b'0'));
                    }
                }
                if name.eq_ignore_ascii_case("sp") {
                    Some(Reg::Addr(7))
                } else if name.eq_ignore_ascii_case("pc") {
                    Some(Reg::Pc)
                } else if name.eq_ignore_ascii_case("sr") {
                    Some(Reg::Sr)
                } else {
                    None
                }
            }
            3 => {
                if name.eq_ignore_ascii_case("ccr") {
                    Some(Reg::Ccr)
                } else if name.eq_ignore_ascii_case("usp") {
                    Some(Reg::Usp)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Index register of the `d8(An,Xn)` addressing modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexReg {
    /// 0-7 for data registers, 8-15 for address registers.
    pub reg: u8,
    pub long: bool,
}

#[derive(Clone, Debug)]
pub enum Operand {
    DataReg(u8),
    AddrReg(u8),
    Indirect(u8),
    PostInc(u8),
    PreDec(u8),
    Disp(u8, Expr),
    Index(u8, Expr, IndexReg),
    /// Absolute address without an explicit size.
    Abs(Expr),
    AbsShort(Expr),
    AbsLong(Expr),
    PcDisp(Expr),
    PcIndex(Expr, IndexReg),
    Imm(Expr),
    Sr,
    Ccr,
    Usp,
    /// Register list for MOVEM, as a mask with bit 0 = d0 and bit 15 = a7.
    RegList(u16),
}

// Addressing mode classes, as bit masks.
pub const M_DN: u16 = 1 << 0;
pub const M_AN: u16 = 1 << 1;
pub const M_IND: u16 = 1 << 2;
pub const M_POSTINC: u16 = 1 << 3;
pub const M_PREDEC: u16 = 1 << 4;
pub const M_DISP: u16 = 1 << 5;
pub const M_INDEX: u16 = 1 << 6;
pub const M_ABSW: u16 = 1 << 7;
pub const M_ABSL: u16 = 1 << 8;
pub const M_PCDISP: u16 = 1 << 9;
pub const M_PCINDEX: u16 = 1 << 10;
pub const M_IMM: u16 = 1 << 11;

pub const M_ALL: u16 = 0x0fff;
pub const M_DATA: u16 = M_ALL & !M_AN;
pub const M_MEMORY: u16 = M_ALL & !(M_DN | M_AN);
pub const M_CONTROL: u16 = M_IND | M_DISP | M_INDEX | M_ABSW | M_ABSL | M_PCDISP | M_PCINDEX;
pub const M_ALTERABLE: u16 =
    M_DN | M_AN | M_IND | M_POSTINC | M_PREDEC | M_DISP | M_INDEX | M_ABSW | M_ABSL;
pub const M_DATA_ALT: u16 = M_ALTERABLE & !M_AN;
pub const M_MEM_ALT: u16 = M_ALTERABLE & !(M_DN | M_AN);
pub const M_CONTROL_ALT: u16 = M_CONTROL & !(M_PCDISP | M_PCINDEX);

impl Operand {
    /// The addressing mode class bit, or 0 for operands that are not
    /// effective addresses. Unsized absolute addresses count as long.
    pub fn mode_mask(&self) -> u16 {
        match self {
            Operand::DataReg(_) => M_DN,
            Operand::AddrReg(_) => M_AN,
            Operand::Indirect(_) => M_IND,
            Operand::PostInc(_) => M_POSTINC,
            Operand::PreDec(_) => M_PREDEC,
            Operand::Disp(..) => M_DISP,
            Operand::Index(..) => M_INDEX,
            Operand::AbsShort(_) => M_ABSW,
            Operand::Abs(_) | Operand::AbsLong(_) => M_ABSL,
            Operand::PcDisp(_) => M_PCDISP,
            Operand::PcIndex(..) => M_PCINDEX,
            Operand::Imm(_) => M_IMM,
            Operand::Sr | Operand::Ccr | Operand::Usp | Operand::RegList(_) => 0,
        }
    }

    /// Mode and register fields of the effective address.
    pub fn ea_bits(&self) -> Option<(u16, u16)> {
        Some(match self {
            Operand::DataReg(n) => (0, *n as u16),
            Operand::AddrReg(n) => (1, *n as u16),
            Operand::Indirect(n) => (2, *n as u16),
            Operand::PostInc(n) => (3, *n as u16),
            Operand::PreDec(n) => (4, *n as u16),
            Operand::Disp(n, _) => (5, *n as u16),
            Operand::Index(n, ..) => (6, *n as u16),
            Operand::AbsShort(_) => (7, 0),
            Operand::Abs(_) | Operand::AbsLong(_) => (7, 1),
            Operand::PcDisp(_) => (7, 2),
            Operand::PcIndex(..) => (7, 3),
            Operand::Imm(_) => (7, 4),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_names() {
        assert_eq!(Reg::parse("d0"), Some(Reg::Data(0)));
        assert_eq!(Reg::parse("A7"), Some(Reg::Addr(7)));
        assert_eq!(Reg::parse("SP"), Some(Reg::Addr(7)));
        assert_eq!(Reg::parse("ccr"), Some(Reg::Ccr));
        assert_eq!(Reg::parse("d8"), None);
        assert_eq!(Reg::parse("da"), None);
        assert_eq!(Reg::parse("foo"), None);
    }
}
