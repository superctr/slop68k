//! Patching of values into assembled code, with range checking.

use crate::expr::Expr;
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixKind {
    /// 8-bit data or immediate, -128 to 255.
    Data8,
    /// 16-bit data or immediate, -32768 to 65535.
    Data16,
    Data32,
    /// Signed 8-bit displacement of an indexed addressing mode.
    Disp8,
    /// Signed 16-bit displacement.
    Disp16,
    /// Absolute short address: must survive sign extension.
    Abs16,
    /// 8-bit displacement of a short branch.
    Branch8,
    /// 16-bit displacement of a branch or DBcc.
    Branch16,
    /// 16-bit PC relative displacement of an addressing mode.
    PcDisp16,
    /// 8-bit PC relative displacement of an indexed addressing mode.
    PcDisp8,
    /// MOVEQ data in the low byte of the opcode.
    Moveq,
    /// 1-8 in bits 9-11 of the opcode (ADDQ, SUBQ, shifts).
    Quick,
    /// TRAP vector in the low 4 bits of the opcode.
    Trap,
}

impl FixKind {
    pub fn is_pc_relative(self) -> bool {
        matches!(
            self,
            FixKind::Branch8 | FixKind::Branch16 | FixKind::PcDisp16 | FixKind::PcDisp8
        )
    }

    /// Number of bytes the patch touches.
    pub fn width(self) -> usize {
        match self {
            FixKind::Data8
            | FixKind::Disp8
            | FixKind::Branch8
            | FixKind::PcDisp8
            | FixKind::Moveq
            | FixKind::Trap => 1,
            FixKind::Data16
            | FixKind::Disp16
            | FixKind::Abs16
            | FixKind::Branch16
            | FixKind::PcDisp16
            | FixKind::Quick => 2,
            FixKind::Data32 => 4,
        }
    }
}

/// A value to be patched into the instruction being built.
#[derive(Clone, Debug)]
pub struct Fixup {
    /// Offset of the patch from the start of the instruction or data.
    pub offset: usize,
    pub kind: FixKind,
    pub expr: Expr,
    /// For PC relative kinds, the address displacements are relative to.
    pub pc: Option<Value>,
}

fn illegal(v: i32) -> String {
    format!("Illegal value ({})", v)
}

/// Writes `v` into `buf` (which starts at the patch position) as `kind`.
pub fn apply(kind: FixKind, v: i32, buf: &mut [u8], warnings: &mut Vec<String>) -> Result<(), String> {
    match kind {
        FixKind::Data8 => {
            if !(-128..=255).contains(&v) {
                return Err(illegal(v));
            }
            buf[0] = v as u8;
        }
        FixKind::Data16 => {
            if !(-32768..=65535).contains(&v) {
                return Err(illegal(v));
            }
            buf[..2].copy_from_slice(&(v as u16).to_be_bytes());
        }
        FixKind::Data32 => buf[..4].copy_from_slice(&(v as u32).to_be_bytes()),
        FixKind::Disp8 | FixKind::PcDisp8 => {
            if !(-128..=127).contains(&v) {
                return Err(illegal(v));
            }
            buf[0] = v as u8;
        }
        FixKind::Disp16 | FixKind::Abs16 | FixKind::PcDisp16 => {
            if !(-32768..=32767).contains(&v) {
                return Err(illegal(v));
            }
            buf[..2].copy_from_slice(&(v as u16).to_be_bytes());
        }
        FixKind::Branch8 => {
            if v == 0 {
                return Err("Illegal zero length short branch".into());
            }
            if !(-128..=127).contains(&v) {
                return Err(illegal(v));
            }
            if v & 1 != 0 {
                warnings.push("Branch to odd address".into());
            }
            buf[0] = v as u8;
        }
        FixKind::Branch16 => {
            if !(-32768..=32767).contains(&v) {
                return Err(format!("Branch ({} bytes) is out of range", v));
            }
            if v & 1 != 0 {
                warnings.push("Branch to odd address".into());
            }
            buf[..2].copy_from_slice(&(v as u16).to_be_bytes());
        }
        FixKind::Moveq => {
            if !(-128..=255).contains(&v) {
                return Err(illegal(v));
            }
            if v > 127 {
                warnings.push(format!("Operand value {} will be sign extended", v));
            }
            buf[0] = v as u8;
        }
        FixKind::Quick => {
            if !(1..=8).contains(&v) {
                return Err(illegal(v));
            }
            buf[0] = (buf[0] & 0xf1) | (((v & 7) as u8) << 1);
        }
        FixKind::Trap => {
            if !(0..=15).contains(&v) {
                return Err(illegal(v));
            }
            buf[0] = (buf[0] & 0xf0) | v as u8;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_apply(kind: FixKind, v: i32) -> Result<(Vec<u8>, Vec<String>), String> {
        let mut buf = vec![0u8; 4];
        let mut w = Vec::new();
        apply(kind, v, &mut buf, &mut w)?;
        Ok((buf, w))
    }

    #[test]
    fn data_ranges() {
        assert!(try_apply(FixKind::Data8, 255).is_ok());
        assert!(try_apply(FixKind::Data8, -128).is_ok());
        assert!(try_apply(FixKind::Data8, 256).is_err());
        assert!(try_apply(FixKind::Data8, -129).is_err());
        assert!(try_apply(FixKind::Data16, 65535).is_ok());
        assert!(try_apply(FixKind::Data16, 65536).is_err());
        assert!(try_apply(FixKind::Data16, -32769).is_err());
        assert_eq!(try_apply(FixKind::Data32, -2).unwrap().0, vec![0xff, 0xff, 0xff, 0xfe]);
    }

    #[test]
    fn displacement_ranges() {
        assert!(try_apply(FixKind::Disp16, 32767).is_ok());
        assert!(try_apply(FixKind::Disp16, 32768).is_err());
        assert!(try_apply(FixKind::Abs16, -32768).is_ok());
        assert!(try_apply(FixKind::Abs16, 0xffff).is_err());
        assert!(try_apply(FixKind::Disp8, 127).is_ok());
        assert!(try_apply(FixKind::Disp8, 128).is_err());
    }

    #[test]
    fn short_branches() {
        assert_eq!(
            try_apply(FixKind::Branch8, 0),
            Err("Illegal zero length short branch".to_string())
        );
        assert!(try_apply(FixKind::Branch8, 128).is_err());
        assert_eq!(try_apply(FixKind::Branch8, -126).unwrap().0[0], 0x82);
        assert_eq!(try_apply(FixKind::Branch8, 1).unwrap().1.len(), 1);
    }

    #[test]
    fn quick_fields() {
        let mut buf = [0x50u8, 0x40];
        let mut w = Vec::new();
        apply(FixKind::Quick, 8, &mut buf, &mut w).unwrap();
        assert_eq!(buf, [0x50, 0x40]);
        apply(FixKind::Quick, 1, &mut buf, &mut w).unwrap();
        assert_eq!(buf, [0x52, 0x40]);
        assert!(apply(FixKind::Quick, 0, &mut buf, &mut w).is_err());
        assert!(apply(FixKind::Quick, 9, &mut buf, &mut w).is_err());
        let (b, w) = try_apply(FixKind::Moveq, 255).unwrap();
        assert_eq!(b[0], 0xff);
        assert_eq!(w.len(), 1);
    }
}
