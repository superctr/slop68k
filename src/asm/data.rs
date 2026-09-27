//! Directives that define data.

use super::scan::*;
use super::statement::Statement;
use super::{Assembler, R};
use crate::m68k::fixup::FixKind;
use crate::m68k::operand::Size;
use crate::source;

fn data_kind(size: Size) -> FixKind {
    match size {
        Size::Byte | Size::Short => FixKind::Data8,
        Size::Word => FixKind::Data16,
        Size::Long => FixKind::Data32,
    }
}

/// Whether the operand consists of a single quoted string.
fn is_string(part: &str) -> bool {
    let b = part.as_bytes();
    matches!(b.first(), Some(b'\'' | b'"')) && skip_string(b, 0) == Some(b.len())
}

impl Assembler<'_> {
    /// Aligns the location counter for word and long data, if automatic
    /// alignment is on, and defines the label of the statement.
    fn start_data(&mut self, size: Size, st: &Statement) -> R<()> {
        if size != Size::Byte && self.opts.auto_even {
            self.align_even()?;
        }
        self.place_label(st)
    }

    pub(crate) fn dir_dc(&mut self, size: Size, st: &Statement, text: &str) -> R<()> {
        self.start_data(size, st)?;
        let field = self.operand_field(text, st.operand_at)?;
        let parts = split_operands(&field);
        if parts.is_empty() {
            return Err("Error in expression".into());
        }
        let kind = data_kind(size);
        for part in parts {
            if size == Size::Byte && is_string(part) {
                let bytes = source::encode(&unquote(part));
                self.emit(&bytes, Vec::new())?;
                continue;
            }
            let e = self.parse_expr(part)?;
            self.emit_value(kind, &e)?;
        }
        Ok(())
    }

    pub(crate) fn dir_dcb(&mut self, size: Size, st: &Statement, text: &str) -> R<()> {
        self.start_data(size, st)?;
        let field = self.operand_field(text, st.operand_at)?;
        let parts = split_operands(&field);
        if parts.len() != 2 {
            return Err("Expected comma between operands".into());
        }
        let count = self.eval_now(parts[0])?;
        if count < 0 {
            return Err(format!("Illegal value ({})", count));
        }
        let e = self.parse_expr(parts[1])?;
        let kind = data_kind(size);
        self.list_block(count as u32 * size.bytes());
        match e.as_abs() {
            Some(n) => {
                let mut unit = [0u8; 4];
                let mut warnings = Vec::new();
                crate::m68k::fixup::apply(kind, n, &mut unit, &mut warnings)?;
                let unit = &unit[..kind.width()];
                let mut bytes = Vec::with_capacity(unit.len() * count as usize);
                for _ in 0..count {
                    bytes.extend_from_slice(unit);
                }
                self.emit(&bytes, Vec::new())
            }
            None => {
                for _ in 0..count {
                    self.emit_value(kind, &e)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) fn dir_ds(&mut self, size: Size, st: &Statement, text: &str) -> R<()> {
        self.start_data(size, st)?;
        let field = self.operand_field(text, st.operand_at)?;
        let count = self.eval_now(&field)?;
        if count < 0 {
            return Err(format!("Illegal value ({})", count));
        }
        self.list_block((count as u32).wrapping_mul(size.bytes()));
        self.reserve((count as u32).wrapping_mul(size.bytes()))
    }

    /// Large blocks of data are left out of the listing.
    fn list_block(&mut self, bytes: u32) {
        if bytes > 256 {
            if let Some(l) = &mut self.list {
                l.omit_code();
            }
        }
    }

    pub(crate) fn dir_hex(&mut self, st: &Statement, text: &str) -> R<()> {
        self.place_label(st)?;
        let field = self.raw_field(text, st.operand_at);
        let mut bytes = Vec::new();
        let mut high: Option<u8> = None;
        for c in field.chars() {
            let d = c
                .to_digit(16)
                .ok_or_else(|| format!("Non-hexadecimal character '{}' encountered", c))? as u8;
            match high.take() {
                Some(h) => bytes.push(h << 4 | d),
                None => high = Some(d),
            }
        }
        if high.is_some() {
            return Err("Odd number of nibbles specified".into());
        }
        self.emit(&bytes, Vec::new())
    }

    /// DATA: numbers of the size set with DATASIZE.
    pub(crate) fn dir_data(&mut self, st: &Statement, text: &str) -> R<()> {
        self.place_label(st)?;
        let size = self.datasize.ok_or("Datasize has not been specified")?;
        let field = self.raw_field(text, st.operand_at).to_string();
        for part in field.split(',') {
            let part = part.trim();
            let (negative, digits) = match part.strip_prefix('-') {
                Some(d) => (true, d),
                None => (false, part),
            };
            let (radix, digits) = match digits.strip_prefix('$') {
                Some(d) => (16, d),
                None => (10, digits),
            };
            if digits.is_empty() {
                return Err("Non-numeric value in DATA statement".into());
            }
            // Big-endian arithmetic on as many bytes as the value has.
            let mut value = vec![0u8; size];
            for c in digits.chars() {
                let d = c.to_digit(radix).ok_or("Non-numeric value in DATA statement")?;
                let mut carry = d;
                for b in value.iter_mut().rev() {
                    let v = *b as u32 * radix + carry;
                    *b = v as u8;
                    carry = v >> 8;
                }
                if carry != 0 {
                    return Err("Overflow in DATA value".into());
                }
            }
            if negative {
                let mut carry = 1u32;
                for b in value.iter_mut().rev() {
                    let v = (!*b) as u32 + carry;
                    *b = v as u8;
                    carry = v >> 8;
                }
            }
            self.emit(&value, Vec::new())?;
        }
        Ok(())
    }

    pub(crate) fn dir_ieee(&mut self, double: bool, st: &Statement, text: &str) -> R<()> {
        self.place_label(st)?;
        let field = self.raw_field(text, st.operand_at).to_string();
        for part in field.split(',') {
            let v: f64 = part.trim().parse().map_err(|_| "Error in floating point number")?;
            if double {
                self.emit(&v.to_be_bytes(), Vec::new())?;
            } else {
                self.emit(&(v as f32).to_be_bytes(), Vec::new())?;
            }
        }
        Ok(())
    }

    pub(crate) fn dir_incbin(&mut self, st: &Statement, text: &str) -> R<()> {
        self.place_label(st)?;
        let field = text[st.operand_at..].trim();
        let (name, rest) = self.file_name(field)?;
        let path = self.locate(&name).ok_or_else(|| format!("Could not open file '{}'", name))?;
        let data = std::fs::read(&path).map_err(|_| format!("Error reading file '{}'", name))?;

        // Optional start and length.
        let rest = rest.trim_start();
        let (mut start, mut len) = (0usize, None);
        if let Some(rest) = rest.strip_prefix(',') {
            let end = operand_end(rest.as_bytes(), 0, self.opts.white_space);
            let args = self.expand_names(&rest[..end])?;
            let parts = split_operands(&args);
            if let Some(p) = parts.first().filter(|p| !p.is_empty()) {
                start = usize::try_from(self.eval_now(p)?).map_err(|_| "Illegal start position/length in INCBIN")?;
            }
            if let Some(p) = parts.get(1).filter(|p| !p.is_empty()) {
                len = Some(usize::try_from(self.eval_now(p)?).map_err(|_| "Illegal start position/length in INCBIN")?);
            }
        }
        if start > data.len() {
            return Err("Illegal start position/length in INCBIN".into());
        }
        let end = match len {
            Some(n) => start.checked_add(n).filter(|&e| e <= data.len()).ok_or("Illegal start position/length in INCBIN")?,
            None => data.len(),
        };
        if start == end {
            self.warn("Zero length incbin");
        }
        if let Some(l) = &mut self.list {
            l.omit_code();
        }
        self.emit(&data[start..end], Vec::new())
    }
}
