//! The assembly listing.
//!
//! The listing is written as the source is read, so values that depend on
//! symbols defined further on show as zero, as they do with asm68k.
//!
//! Each line read from a source file gets one line in the listing, along
//! with the code it generated. Unless the listing of macros was asked for,
//! that includes the code of any macros and loops the line set off.

use std::rc::Rc;

use super::section::Section;
use crate::source;

/// Column at which the source text starts.
const TEXT_COLUMN: usize = 36;
/// Most bytes of code shown for a line.
const MAX_BYTES: usize = 10;

/// A line that has been read but not yet listed.
struct Record {
    addr: u32,
    value: Option<i32>,
    text: Rc<str>,
    expansion: bool,
    visible: bool,
    /// Show the code the line generated.
    code: bool,
    /// Where the code of the line starts.
    section: usize,
    chunk: usize,
    len: usize,
}

pub struct Listing {
    out: Vec<u8>,
    /// Lines are listed while this is not negative.
    pub level: i32,
    list_macros: bool,
    pending: Option<Record>,
    /// The line being assembled is part of the pending record rather than
    /// the line the record is for.
    hidden: bool,
    /// Start and end of the last line written.
    last: Option<(usize, usize)>,
}

impl Listing {
    pub fn new(list_macros: bool) -> Listing {
        Listing { out: Vec::new(), level: 0, list_macros, pending: None, hidden: false, last: None }
    }

    /// Starts on a line that is about to be assembled.
    pub fn begin(&mut self, text: &Rc<str>, expansion: bool, pc: u32, section: usize, sections: &[Section]) {
        if expansion && !self.list_macros {
            // The code goes with the line that set off the expansion.
            self.hidden = true;
            return;
        }
        self.flush(sections);
        self.hidden = false;
        let sec = &sections[section];
        self.pending = Some(Record {
            addr: pc,
            value: None,
            text: text.clone(),
            expansion,
            visible: true,
            code: true,
            section,
            chunk: sec.chunks.len().saturating_sub(1),
            len: sec.chunks.last().map_or(0, |c| c.data.len()),
        });
    }

    /// Finishes with the line being assembled. `listed` is false for lines
    /// that are left out of the listing.
    pub fn end(&mut self, listed: bool, text: Option<&Rc<str>>) {
        let level = self.level;
        let hidden = self.hidden;
        if let Some(r) = &mut self.pending {
            if !listed {
                r.visible = false;
            }
            if !hidden {
                if level < 0 {
                    r.visible = false;
                }
                if let Some(t) = text {
                    r.text = t.clone();
                }
            }
        }
    }

    /// Whether the line being assembled is left out of the listing, its
    /// code going with the line that caused it to be assembled.
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Sets the address shown for the line.
    pub fn set_addr(&mut self, addr: u32) {
        if self.hidden {
            return;
        }
        if let Some(r) = &mut self.pending {
            r.addr = addr;
        }
    }

    /// Sets the value shown for a line that defines a symbol.
    pub fn set_value(&mut self, value: i32) {
        if self.hidden {
            return;
        }
        if let Some(r) = &mut self.pending {
            r.value = Some(value);
        }
    }

    /// Notes that the line went on to another section, where its code is
    /// to be found from now on.
    pub fn retarget(&mut self, section: usize, sections: &[Section]) {
        if self.hidden {
            return;
        }
        if let Some(r) = &mut self.pending {
            let sec = &sections[section];
            r.section = section;
            r.chunk = sec.chunks.len().saturating_sub(1);
            r.len = sec.chunks.last().map_or(0, |c| c.data.len());
        }
    }

    /// Leaves the code of the line out of the listing, as is done for
    /// binary files.
    pub fn omit_code(&mut self) {
        if self.hidden {
            return;
        }
        if let Some(r) = &mut self.pending {
            r.code = false;
        }
    }

    pub fn clear_value(&mut self) {
        if let Some(r) = &mut self.pending {
            r.value = None;
        }
    }

    /// Writes out the pending line.
    pub fn flush(&mut self, sections: &[Section]) {
        let Some(r) = self.pending.take() else { return };
        if !r.visible {
            self.last = None;
            return;
        }
        let mut bytes: Vec<u8> = Vec::new();
        let mut more = false;
        if let Some(sec) = sections.get(r.section).filter(|_| r.code) {
            let mut from = r.len;
            for c in sec.chunks.iter().skip(r.chunk) {
                let data = c.data.get(from..).unwrap_or(&[]);
                let room = MAX_BYTES - bytes.len();
                if data.len() > room {
                    more = true;
                }
                bytes.extend_from_slice(&data[..data.len().min(room)]);
                from = 0;
            }
        }
        let start = self.out.len();
        self.write(&r, &bytes, more);
        self.last = Some((start, self.out.len()));
    }

    /// Lists the last line once more. This is what asm68k does with the
    /// last line of an included file.
    pub fn repeat_last(&mut self, sections: &[Section]) {
        self.flush(sections);
        if let Some((start, end)) = self.last.take() {
            self.out.extend_from_within(start..end);
        }
    }

    fn write(&mut self, r: &Record, bytes: &[u8], more: bool) {
        let mut line = format!("{:08X} ", r.addr);
        if let Some(v) = r.value {
            line.push_str(&format!("={:08X}", v));
        } else {
            for (n, b) in bytes.iter().enumerate() {
                if n > 0 && n % 2 == 0 {
                    line.push(' ');
                }
                line.push_str(&format!("{:02X}", b));
            }
            if more {
                line.push('+');
            }
        }
        while line.len() < TEXT_COLUMN - 2 {
            line.push(' ');
        }
        line.push(if r.expansion { 'M' } else { ' ' });
        line.push(' ');
        self.out.extend_from_slice(line.as_bytes());
        self.out.extend_from_slice(&source::encode(&r.text));
        // asm68k leaves the carriage return off empty lines.
        self.out.extend_from_slice(if r.text.is_empty() { b"\n" } else { b"\r\n" });
    }

    pub fn finish(mut self, sections: &[Section]) -> Vec<u8> {
        self.flush(sections);
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        l: Listing,
        sections: Vec<Section>,
    }

    impl Fixture {
        fn new(list_macros: bool) -> Fixture {
            Fixture { l: Listing::new(list_macros), sections: vec![Section::new("", true)] }
        }

        fn line(&mut self, addr: u32, bytes: &[u8], text: &str, expansion: bool) {
            self.l.begin(&text.into(), expansion, addr, 0, &self.sections);
            self.sections[0].chunks[0].data.extend_from_slice(bytes);
            self.l.end(true, None);
        }

        fn text(mut self) -> String {
            self.l.flush(&self.sections);
            String::from_utf8(self.l.out).unwrap()
        }
    }

    #[test]
    fn formats_code() {
        let mut f = Fixture::new(true);
        f.line(0, &[0x70, 0x01], "\tmoveq\t#1,d0", false);
        f.line(0x1d, &[0x65, 0x6c, 0x65], "\tdc.b\t1", false);
        f.line(0, &[1, 2], "\tdc.b\t1,2", true);
        assert_eq!(
            f.text(),
            "00000000 7001                       \tmoveq\t#1,d0\r\n\
             0000001D 656C 65                    \tdc.b\t1\r\n\
             00000000 0102                     M \tdc.b\t1,2\r\n"
        );
    }

    #[test]
    fn long_data_is_cut_short() {
        let mut f = Fixture::new(false);
        f.line(4, &[0, 7, 0, 5, 0, 4, 0, 4, 0, 1], "x", false);
        f.line(4, &[0, 7, 0, 5, 0, 4, 0, 4, 0, 1, 2], "x", false);
        assert_eq!(
            f.text(),
            "00000004 0007 0005 0004 0004 0001   x\r\n\
             00000004 0007 0005 0004 0004 0001+  x\r\n"
        );
    }

    #[test]
    fn values() {
        let mut f = Fixture::new(false);
        f.l.begin(&"fwdequ\tequ\t6".into(), false, 0x186, 0, &f.sections);
        f.l.set_value(6);
        f.l.end(true, None);
        assert_eq!(f.text(), "00000186 =00000006                  fwdequ\tequ\t6\r\n");
    }

    #[test]
    fn hidden_expansions_add_to_the_line_that_caused_them() {
        let mut f = Fixture::new(false);
        f.line(8, &[], "\tm1\t$11", false);
        f.line(8, &[0x11], "\tdc.b\t\\1", true);
        f.l.set_value(5);
        f.l.set_addr(0x100);
        f.line(9, &[0, 0x11], "\tdc.w\t\\1", true);
        f.line(11, &[1], "\tdc.b\t1", false);
        assert_eq!(
            f.text(),
            "00000008 1100 11                    \tm1\t$11\r\n\
             0000000B 01                         \tdc.b\t1\r\n"
        );
    }

    #[test]
    fn unlisted_lines() {
        let mut f = Fixture::new(false);
        f.l.begin(&"\tm2".into(), false, 0, 0, &f.sections);
        f.l.end(true, None);
        // A conditional inside the macro takes the line out of the listing.
        f.l.begin(&"\tif 1".into(), true, 0, 0, &f.sections);
        f.l.end(false, None);
        f.line(0, &[1], "\tdc.b\t1", false);
        f.l.level = -1;
        f.line(1, &[2], "\tdc.b\t2", false);
        f.l.level = 0;
        f.line(2, &[3], "\tdc.b\t3", false);
        assert_eq!(
            f.text(),
            "00000000 01                         \tdc.b\t1\r\n\
             00000002 03                         \tdc.b\t3\r\n"
        );
    }

    #[test]
    fn last_line_of_include_is_repeated() {
        let mut f = Fixture::new(false);
        f.line(0, &[1], "\tdc.b\t1", false);
        f.l.repeat_last(&f.sections);
        f.line(1, &[2], "\tdc.b\t2", false);
        assert_eq!(
            f.text(),
            "00000000 01                         \tdc.b\t1\r\n\
             00000000 01                         \tdc.b\t1\r\n\
             00000001 02                         \tdc.b\t2\r\n"
        );
    }
}
