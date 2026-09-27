//! Splitting of source lines into label, opcode and operand fields.

use super::scan::*;

#[derive(Debug, PartialEq, Eq)]
pub struct Statement<'a> {
    pub label: Option<&'a str>,
    /// The opcode field as written, empty if there is none.
    pub opcode: &'a str,
    /// Offset of the operand field within the line.
    pub operand_at: usize,
}

impl<'a> Statement<'a> {
    /// The opcode without its size suffix.
    pub fn mnemonic(&self) -> &'a str {
        match self.opcode.find('.') {
            Some(0) | None => self.opcode,
            Some(n) => &self.opcode[..n],
        }
    }

    /// The text that follows the first dot of the opcode.
    pub fn size(&self) -> &'a str {
        match self.opcode.find('.') {
            Some(0) | None => "",
            Some(n) => &self.opcode[n + 1..],
        }
    }
}

/// Whether the line consists of a comment or nothing at all.
pub fn is_blank(line: &str) -> bool {
    let b = line.as_bytes();
    if b.first() == Some(&b'*') {
        return true;
    }
    let i = skip_space(b, 0);
    i == b.len() || b[i] == b';' || b[i] == b'*'
}

/// Splits a line into its fields.
pub fn parse(line: &str, local: char) -> Statement<'_> {
    let b = line.as_bytes();
    let mut i = 0;
    let mut label = None;
    if b.is_empty() {
        return Statement { label, opcode: "", operand_at: 0 };
    }

    if !is_space(b[0]) {
        // Anything in the first column is a label.
        let end = label_end(b, 0, local);
        if end > 0 {
            label = Some(&line[..end]);
        }
        i = end;
        while i < b.len() && b[i] == b':' {
            i += 1;
        }
    } else {
        i = skip_space(b, i);
        // Elsewhere a label has to end with a colon.
        if i < b.len() && is_ident_start(b[i], local) {
            let end = label_end(b, i, local);
            if b.get(end) == Some(&b':') && end > i {
                label = Some(&line[i..end]);
                i = end;
                while i < b.len() && b[i] == b':' {
                    i += 1;
                }
            }
        }
    }

    i = skip_space(b, i);
    let start = i;
    if i < b.len() && b[i] == b'=' {
        // Assignment does not need white space around it.
        i += 1;
        if b.get(i) == Some(&b'=') {
            i += 1;
        }
    } else {
        while i < b.len() && !is_space(b[i]) && b[i] != b';' {
            if b[i] == b'=' && label.is_some() && i > start {
                break;
            }
            i += 1;
        }
    }
    let opcode = &line[start..i];
    let operand_at = skip_space(b, i);
    Statement { label, opcode, operand_at }
}

/// End of the label that starts at `pos`. Unlike names in expressions,
/// labels include anything that looks like a size suffix.
fn label_end(b: &[u8], pos: usize, local: char) -> usize {
    let mut end = pos;
    while end < b.len() && (is_ident_char(b[end]) || b[end] as char == local) {
        end += 1;
    }
    end
}

/// Puts a line in the form in which the lines of macros and loops are
/// kept and listed: fields are separated by single tabs, and comments and
/// white space between operands are left out. `is_macro` tells whether an
/// opcode is known not to be an instruction or directive, in which case
/// its operands are the arguments of a macro. Returns `None` for lines
/// with nothing but a comment on them.
pub fn tidy(text: &str, local: char, white_space: bool, is_macro: impl Fn(&str) -> bool) -> Option<String> {
    let _ = local;
    if is_blank(text) {
        return None;
    }
    // The line may hold parameters yet to be replaced, so fields are told
    // apart by the white space between them and nothing else.
    let b = text.as_bytes();
    let token_end = |from: usize| {
        let mut i = from;
        while i < b.len() && !is_space(b[i]) && b[i] != b';' {
            i += 1;
        }
        i
    };
    let in_first_column = !is_space(b[0]);
    let first_at = skip_space(b, 0);
    let first_end = token_end(first_at);
    let first = &text[first_at..first_end];
    let has_label = in_first_column || first.ends_with(':');

    let mut out = String::with_capacity(text.len());
    if !in_first_column {
        out.push('\t');
    }
    out.push_str(first);
    let (opcode, operand_at) = if has_label {
        let at = skip_space(b, first_end);
        let end = token_end(at);
        if end == at {
            return Some(out);
        }
        out.push('\t');
        out.push_str(&text[at..end]);
        (&text[at..end], skip_space(b, end))
    } else {
        (first, skip_space(b, first_end))
    };

    let mnemonic = opcode.split('.').next().unwrap_or("");
    let is_macro = !opcode.contains('\\') && !opcode.starts_with('=') && is_macro(mnemonic);
    let end = if is_macro {
        argument_end(b, operand_at)
    } else {
        operand_end(b, operand_at, white_space)
    };
    if end > operand_at {
        // After a label that is not in the first column the statement is
        // kept as one field.
        out.push(if has_label && !in_first_column { ' ' } else { '\t' });
        let mut i = operand_at;
        while i < end {
            match b[i] {
                b'\'' | b'"' => {
                    let e = skip_string(b, i).unwrap_or(end).min(end);
                    out.push_str(&text[i..e]);
                    i = e;
                }
                b'<' if is_macro => {
                    let e = b[i..end].iter().position(|&c| c == b'>').map_or(end, |n| i + n + 1);
                    out.push_str(&text[i..e]);
                    i = e;
                }
                b',' => {
                    out.push(',');
                    i = skip_space(b, i + 1);
                }
                b' ' | b'\t' if white_space => i += 1,
                _ => {
                    let e = (i + 1..=end).find(|&e| text.is_char_boundary(e)).unwrap_or(end);
                    out.push_str(&text[i..e]);
                    i = e;
                }
            }
        }
    }
    Some(out)
}

/// Finds the end of the arguments of a macro invocation, which may hold
/// white space between angle brackets.
fn argument_end(s: &[u8], start: usize) -> usize {
    let mut i = start;
    let mut after_comma = true;
    while i < s.len() {
        match s[i] {
            b'\'' | b'"' => {
                i = skip_string(s, i).unwrap_or(s.len());
                after_comma = false;
                continue;
            }
            b'<' if after_comma => {
                i = s[i..].iter().position(|&c| c == b'>').map_or(s.len(), |n| i + n);
                after_comma = false;
            }
            b';' => break,
            b' ' | b'\t' => {
                if !after_comma {
                    break;
                }
            }
            b',' => after_comma = true,
            _ => after_comma = false,
        }
        i += 1;
    }
    // A bracket that is not closed takes in the rest of the line.
    let mut i = i.min(s.len());
    while i > start && is_space(s[i - 1]) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> (Option<&str>, &str, &str) {
        let s = parse(line, '@');
        (s.label, s.opcode, &line[s.operand_at..])
    }

    #[test]
    fn labels() {
        assert_eq!(p("start:"), (Some("start"), "", ""));
        assert_eq!(p("start"), (Some("start"), "", ""));
        assert_eq!(p("start\tmoveq\t#1,d0"), (Some("start"), "moveq", "#1,d0"));
        assert_eq!(p("start: moveq #1,d0 ; x"), (Some("start"), "moveq", "#1,d0 ; x"));
        assert_eq!(p("  start:\tnop"), (Some("start"), "nop", ""));
        assert_eq!(p("\tlab4: dc.b 4"), (Some("lab4"), "dc.b", "4"));
        assert_eq!(p("@loop\tnop"), (Some("@loop"), "nop", ""));
        assert_eq!(p("\t@loop:"), (Some("@loop"), "", ""));
        assert_eq!(p("stop_z80_1:"), (Some("stop_z80_1"), "", ""));
        assert_eq!(p("a.b\tnop"), (Some("a.b"), "nop", ""));
    }

    #[test]
    fn opcodes() {
        assert_eq!(p("\tnop"), (None, "nop", ""));
        assert_eq!(p("\tmove.w\td0,d1\tcomment"), (None, "move.w", "d0,d1\tcomment"));
        assert_eq!(p("\trts;comment"), (None, "rts", ";comment"));
        assert_eq!(p("\t@ch_mapping\t#0,foo"), (None, "@ch_mapping", "#0,foo"));
        let s = parse("\tmove.w\td0,d1", '@');
        assert_eq!((s.mnemonic(), s.size()), ("move", "w"));
        let s = parse("\tzed.nop.x", '@');
        assert_eq!((s.mnemonic(), s.size()), ("zed", "nop.x"));
        let s = parse("\trts", '@');
        assert_eq!((s.mnemonic(), s.size()), ("rts", ""));
    }

    #[test]
    fn assignment() {
        assert_eq!(p("x\t=\t5"), (Some("x"), "=", "5"));
        assert_eq!(p("x = 5"), (Some("x"), "=", "5"));
        assert_eq!(p("x=5"), (Some("x"), "=", "5"));
        assert_eq!(p("x =5"), (Some("x"), "=", "5"));
        assert_eq!(p("x: equ 5"), (Some("x"), "equ", "5"));
    }

    fn t(text: &str) -> String {
        tidy(text, '@', false, |m| m == "mac").unwrap()
    }

    #[test]
    fn tidies_lines() {
        assert_eq!(t("\tmove.w\td0,d1\tcomment here"), "\tmove.w\td0,d1");
        assert_eq!(t("  move.w  d0,  d1   ; two spaces"), "\tmove.w\td0,d1");
        assert_eq!(t("lab_1: move.w d0, d1 comment"), "lab_1:\tmove.w\td0,d1");
        assert_eq!(t("lab2_1\tmove.w\td0,d1"), "lab2_1\tmove.w\td0,d1");
        assert_eq!(t("\tlab3_1:\tmove.w\td0,d1"), "\tlab3_1:\tmove.w d0,d1");
        assert_eq!(t("\tlab4_1:  dc.b  1, 2,  3   ; x"), "\tlab4_1:\tdc.b 1,2,3");
        assert_eq!(t("lab5_1:dc.b\t1"), "lab5_1:dc.b\t1");
        assert_eq!(t("v_1\t=\t1"), "v_1\t=\t1");
        assert_eq!(t("\tv2_1:\t= 2"), "\tv2_1:\t= 2");
        assert_eq!(t("\tv3_1: equ 1 + 2"), "\tv3_1:\tequ 1");
        assert_eq!(t("    dc.b    'a, b',  \"c  d\"   ; four"), "\tdc.b\t'a, b',\"c  d\"");
        assert_eq!(t("\tnop"), "\tnop");
        assert_eq!(t("glab_1:"), "glab_1:");
        assert_eq!(t("\t.end:"), "\t.end:");
        assert_eq!(t("\trept 129"), "\trept\t129");
        assert_eq!(tidy("; comment", '@', false, |_| false), None);
        assert_eq!(tidy("", '@', false, |_| false), None);
        assert_eq!(tidy("  \t", '@', false, |_| false), None);
    }

    #[test]
    fn tidies_lines_with_white_space() {
        let t = |text: &str| tidy(text, '@', true, |_| false).unwrap();
        assert_eq!(t("\tdc.b\t1 + 2 ,  3\t; comment"), "\tdc.b\t1+2,3");
        assert_eq!(t("lab_1: dc.b 1 +  2 , 5 ; c"), "lab_1:\tdc.b\t1+2,5");
        assert_eq!(t("\tlab3_1:\tdc.b\t1 +\t2 , 6 ; c"), "\tlab3_1:\tdc.b 1+2,6");
        assert_eq!(t("\tlab4_1:  dc.b  'a  b' , 2,  3   ; x"), "\tlab4_1:\tdc.b 'a  b',2,3");
        assert_eq!(t("\tv2_1:\t= 2   +  2"), "\tv2_1:\t= 2+2");
        assert_eq!(t("\tmove.w\t( a0 ) , d1"), "\tmove.w\t(a0),d1");
    }

    #[test]
    fn tidies_macro_arguments() {
        assert_eq!(t("\tmac\t<a b>, c ; x"), "\tmac\t<a b>,c");
        assert_eq!(t("\tmac\t1, <x, y> rest"), "\tmac\t1,<x, y>");
        assert_eq!(t("\tmac\ta b"), "\tmac\ta");
        assert_eq!(t("\tmac\t<a b"), "\tmac\t<a b");
        assert_eq!(t("\tmac\t1,<"), "\tmac\t1,<");
    }

    #[test]
    fn tidies_lines_with_parameters() {
        assert_eq!(t("lbl\\@\tdc.b\t\\1 ; x"), "lbl\\@\tdc.b\t\\1");
        assert_eq!(t("stop\\@:"), "stop\\@:");
        assert_eq!(t("\tdc.\\0\t\\1, \\2"), "\tdc.\\0\t\\1,\\2");
        assert_eq!(t("\t\\0"), "\t\\0");
        assert_eq!(t("\\*_tab\tdc.b\t\\val"), "\\*_tab\tdc.b\t\\val");
        assert_eq!(t("x = 5"), "x\t=\t5");
        assert_eq!(t("x =5"), "x\t=5");
        assert_eq!(t("x=5"), "x=5");
    }

    #[test]
    fn blank_lines() {
        assert!(is_blank(""));
        assert!(is_blank("   \t"));
        assert!(is_blank("; comment"));
        assert!(is_blank("\t; comment"));
        assert!(is_blank("* comment"));
        assert!(is_blank("*****"));
        assert!(is_blank("\t* comment"));
        assert!(!is_blank("\tnop"));
        assert!(!is_blank("label"));
    }
}
