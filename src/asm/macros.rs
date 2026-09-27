//! Macro definitions and invocations.

use std::collections::HashMap;
use std::rc::Rc;

use super::scan::*;
use crate::source::SrcLine;

#[derive(Debug)]
pub struct Macro {
    pub name: String,
    /// Names of the parameters, in order.
    pub params: Rc<Vec<String>>,
    /// The macro takes the label of the invocation as `\*`.
    pub takes_label: bool,
    pub lines: Rc<Vec<SrcLine>>,
    /// Defined with MACROS: the body is a single line.
    pub short: bool,
}

/// State of a macro invocation.
#[derive(Debug)]
pub struct MacroCtx {
    pub args: Vec<String>,
    /// The argument list as written, for `\_`.
    pub all: String,
    /// Text after the dot of the invocation, for `\0`.
    pub size: String,
    /// Label of the invocation, for `\*`.
    pub label: String,
    pub params: Rc<Vec<String>>,
    /// Number of the invocation, for `\@`.
    pub unique: u32,
    /// Names declared with LOCAL and what they are renamed to.
    pub locals: HashMap<String, String>,
}

/// Parses the parameter names of a macro definition.
pub fn parse_params(text: &str) -> Result<(Vec<String>, bool), String> {
    let end = operand_end(text.as_bytes(), 0, false);
    let mut params = Vec::new();
    let mut takes_label = false;
    for (n, p) in text[..end].split(',').enumerate() {
        let p = p.trim();
        if p.is_empty() {
            if n == 0 && end == 0 {
                break;
            }
            return Err("Illegal name for macro parameter".into());
        }
        if n == 0 && p == "*" {
            takes_label = true;
            continue;
        }
        let p = p.strip_prefix('\\').unwrap_or(p);
        let b = p.as_bytes();
        if b.is_empty() || b[0].is_ascii_digit() || !b.iter().all(|&c| is_ident_char(c)) {
            return Err("Illegal name for macro parameter".into());
        }
        params.push(p.to_string());
    }
    Ok((params, takes_label))
}

/// Parses the arguments of a macro invocation. Returns the arguments and
/// the argument list as it was written.
pub fn parse_args(text: &str) -> Result<(Vec<String>, String), String> {
    let b = text.as_bytes();
    let mut args = Vec::new();
    let mut i = 0;
    if b.is_empty() || b[0] == b';' {
        return Ok((args, String::new()));
    }
    let mut last_end;
    loop {
        i = skip_space(b, i);
        if i < b.len() && b[i] == b'<' {
            // Angle brackets allow commas and spaces in an argument.
            let close = b[i..].iter().position(|&c| c == b'>').ok_or("Expected comma after < >")?;
            args.push(text[i + 1..i + close].to_string());
            i += close + 1;
            last_end = i;
            if i < b.len() && !is_space(b[i]) && b[i] != b',' && b[i] != b';' {
                return Err("Expected comma after < >".into());
            }
        } else {
            let start = i;
            while i < b.len() {
                match b[i] {
                    b'\'' | b'"' => match skip_string(b, i) {
                        Some(e) => i = e,
                        None => i = b.len(),
                    },
                    b',' | b' ' | b'\t' | b';' => break,
                    _ => i += 1,
                }
            }
            args.push(text[start..i].to_string());
            last_end = i;
        }
        if i < b.len() && b[i] == b',' {
            i += 1;
        } else {
            break;
        }
    }
    Ok((args, text[..last_end].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        parse_args(s).unwrap().0
    }

    #[test]
    fn arguments_split_at_commas() {
        assert_eq!(args("1,2"), ["1", "2"]);
        assert_eq!(args("3, 4"), ["3", "4"]);
        assert_eq!(args("4(a0,d0.w),d1"), ["4(a0", "d0.w)", "d1"]);
        assert_eq!(args("(a0)+, -(a1)"), ["(a0)+", "-(a1)"]);
        assert_eq!(args("#\"A,\",d0"), ["#\"A,\"", "d0"]);
        assert_eq!(args("\"a,b\",c"), ["\"a,b\"", "c"]);
    }

    #[test]
    fn arguments_end_at_white_space() {
        assert_eq!(args("a b,c"), ["a"]);
        assert_eq!(args("a ;comment"), ["a"]);
        assert_eq!(args("a\t; comment, with comma"), ["a"]);
        assert_eq!(args("\"a b\",c d"), ["\"a b\"", "c"]);
    }

    #[test]
    fn empty_arguments() {
        assert_eq!(args(""), Vec::<String>::new());
        assert_eq!(args("; comment"), Vec::<String>::new());
        assert_eq!(args(",c"), ["", "c"]);
        assert_eq!(args("a,,c"), ["a", "", "c"]);
        assert_eq!(args("a,b,"), ["a", "b", ""]);
    }

    #[test]
    fn angle_brackets() {
        assert_eq!(args("<a,b>,c"), ["a,b", "c"]);
        assert_eq!(args("<a b> c"), ["a b"]);
        assert_eq!(args("11,10,<Psy-Q, from Psygnosis>"), ["11", "10", "Psy-Q, from Psygnosis"]);
        assert!(parse_args("<a").is_err());
    }

    #[test]
    fn whole_argument_list() {
        assert_eq!(parse_args("18,19,20").unwrap().1, "18,19,20");
        assert_eq!(parse_args("21, 22 ; x").unwrap().1, "21, 22");
        assert_eq!(parse_args("").unwrap().1, "");
    }

    #[test]
    fn parameter_names() {
        assert_eq!(parse_params("").unwrap(), (vec![], false));
        assert_eq!(parse_params("a,b").unwrap(), (vec!["a".to_string(), "b".to_string()], false));
        assert_eq!(parse_params("flag, src ; comment").unwrap().0, ["flag", "src"]);
        assert_eq!(parse_params("*,a").unwrap(), (vec!["a".to_string()], true));
        assert_eq!(parse_params("\\a,\\b").unwrap().0, ["a", "b"]);
        assert!(parse_params("a,,b").is_err());
        assert!(parse_params("1a").is_err());
    }
}
