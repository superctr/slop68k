//! Low level scanning of source lines.
//!
//! Source text is scanned as bytes. All syntax is ASCII, so positions found
//! here always fall on character boundaries.

/// Whether `c` can appear in a name.
pub fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'?' || c == b'.' || c >= 0x80
}

/// Whether `c` can start a name.
pub fn is_ident_start(c: u8, local: char) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'.' || c >= 0x80 || c as char == local
}

pub fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

pub fn skip_space(s: &[u8], mut pos: usize) -> usize {
    while pos < s.len() && is_space(s[pos]) {
        pos += 1;
    }
    pos
}

/// Finds the end of the name starting at `pos`. Dots are part of names, so
/// `label.w` is a name in its own right; sizes are taken off operands
/// before they are looked at as expressions.
pub fn scan_ident(s: &[u8], pos: usize) -> usize {
    let mut end = pos;
    if end < s.len() {
        // The first character may be the local label prefix.
        end += 1;
    }
    while end < s.len() && is_ident_char(s[end]) {
        end += 1;
    }
    end
}

/// Whether a size suffix (`.b`, `.w`, `.l` or `.s`) starts at `pos` and is
/// followed by something that is not part of a name.
pub fn is_size_suffix(s: &[u8], pos: usize) -> bool {
    s.get(pos) == Some(&b'.')
        && matches!(s.get(pos + 1).map(|c| c.to_ascii_lowercase()), Some(b'b' | b'w' | b'l' | b's'))
        && !s.get(pos + 2).is_some_and(|&c| is_ident_char(c))
}

/// Finds the end of the quoted string that starts at `pos`. Quotes are
/// doubled to include them in the string. Returns the position after the
/// closing quote, or `None` if the string is not closed.
pub fn skip_string(s: &[u8], pos: usize) -> Option<usize> {
    let quote = s[pos];
    let mut i = pos + 1;
    while i < s.len() {
        if s[i] == quote {
            if s.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

/// The contents of a quoted string, with doubled quotes reduced.
pub fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() < 2 {
        return String::new();
    }
    let quote = b[0] as char;
    let inner = &s[1..s.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == quote && chars.peek() == Some(&quote) {
            chars.next();
        }
        out.push(c);
    }
    out
}

/// Finds the end of the operand field that starts at `start`. The field
/// ends at a comment, or at white space that does not follow a comma and is
/// not inside quotes. With `white_space` set only a comment ends the field.
pub fn operand_end(s: &[u8], start: usize, white_space: bool) -> usize {
    let mut i = start;
    let mut after_comma = false;
    while i < s.len() {
        let c = s[i];
        match c {
            b'\'' | b'"' => {
                i = skip_string(s, i).unwrap_or(s.len());
                after_comma = false;
                continue;
            }
            b';' => break,
            b' ' | b'\t' => {
                if !white_space && !after_comma {
                    // White space is allowed in front of a comma only when
                    // it is allowed everywhere.
                    break;
                }
            }
            b',' => after_comma = true,
            _ => after_comma = false,
        }
        i += 1;
    }
    // Leave out trailing white space.
    while i > start && is_space(s[i - 1]) {
        i -= 1;
    }
    i
}

/// Splits an operand field at its top level commas, that is those outside
/// of quotes and parentheses. Parts are trimmed of white space.
pub fn split_operands(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut parts = Vec::new();
    if s.trim().is_empty() {
        return parts;
    }
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                i = skip_string(b, i).unwrap_or(b.len());
                continue;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth <= 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(s[start..].trim());
    parts
}

/// Finds the parenthesis that matches the one closing at the end of `s`.
pub fn matching_open(s: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = 0;
    let mut open_at_depth1 = None;
    while i < s.len() {
        match s[i] {
            b'\'' | b'"' => {
                i = skip_string(s, i)?;
                continue;
            }
            b'(' => {
                depth += 1;
                if depth == 1 {
                    open_at_depth1 = Some(i);
                }
            }
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
                if depth == 0 && i + 1 == s.len() {
                    return open_at_depth1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> &str {
        &s[..operand_end(s.as_bytes(), 0, false)]
    }

    #[test]
    fn operand_field_ends_at_white_space() {
        assert_eq!(field("d0,d1 comment"), "d0,d1");
        assert_eq!(field("d0,d1\t; comment"), "d0,d1");
        assert_eq!(field("#$100, (addr) rest"), "#$100, (addr)");
        assert_eq!(field("1,2 , 3"), "1,2");
        assert_eq!(field("' ', \"a b\", ' ' x"), "' ', \"a b\", ' '");
        assert_eq!(field("';', \"a;b\" ; real"), "';', \"a;b\"");
        assert_eq!(field("10 % 3"), "10");
        assert_eq!(field("'it''s' x"), "'it''s'");
        assert_eq!(field("1;x"), "1");
        assert_eq!(field(""), "");
    }

    #[test]
    fn white_space_option() {
        let s = "1 + 2 ; comment";
        assert_eq!(&s[..operand_end(s.as_bytes(), 0, true)], "1 + 2");
    }

    #[test]
    fn splits_at_top_level_commas() {
        assert_eq!(split_operands("4(a0,d0.w),d1"), ["4(a0,d0.w)", "d1"]);
        assert_eq!(split_operands("#',',d0"), ["#','", "d0"]);
        assert_eq!(split_operands("(a0)+, -(a1)"), ["(a0)+", "-(a1)"]);
        assert_eq!(split_operands("1,,2"), ["1", "", "2"]);
        assert_eq!(split_operands("").len(), 0);
        assert_eq!(split_operands("\"a,b\",c"), ["\"a,b\"", "c"]);
    }

    #[test]
    fn names_and_suffixes() {
        let end = |s: &str| scan_ident(s.as_bytes(), 0);
        assert_eq!(end("label+1"), 5);
        assert_eq!(end("print_begin.w"), 13);
        assert_eq!(end("foo.bar"), 7);
        assert_eq!(end("foo.w)"), 5);
        assert_eq!(end("@loop-x"), 5);
        assert!(is_size_suffix(b"foo.w", 3));
        assert!(is_size_suffix(b"(foo).l", 5));
        assert!(!is_size_suffix(b"foo.west", 3));
        assert!(!is_size_suffix(b"foo.x", 3));
    }

    #[test]
    fn strings() {
        assert_eq!(skip_string(b"'abc' x", 0), Some(5));
        assert_eq!(skip_string(b"'it''s'", 0), Some(7));
        assert_eq!(skip_string(b"'abc", 0), None);
        assert_eq!(unquote("'it''s'"), "it's");
        assert_eq!(unquote("\"say \"\"hi\"\"\""), "say \"hi\"");
        assert_eq!(unquote("\"it''s\""), "it''s");
        assert_eq!(unquote("''"), "");
    }

    #[test]
    fn matching_parentheses() {
        assert_eq!(matching_open(b"4(a0,d0.w)"), Some(1));
        assert_eq!(matching_open(b"(1+2)*(3)"), Some(6));
        assert_eq!(matching_open(b"((4))"), Some(0));
        assert_eq!(matching_open(b"(a0)+"), None);
        assert_eq!(matching_open(b"foo"), None);
    }
}
