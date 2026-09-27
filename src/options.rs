//! Assembler options, as set by the OPT directive or the /o switch.

#[derive(Clone, Debug)]
pub struct Options {
    /// AE - align word and long data directives to even addresses.
    pub auto_even: bool,
    /// AN - allow Intel/Zilog style numbers (0FFh, 101b, 99d).
    pub alt_numeric: bool,
    /// C - symbol names are case sensitive.
    pub case_sensitive: bool,
    /// D - EQU and SET descope local labels.
    pub descope_equ: bool,
    /// E - print the source line along with errors.
    pub print_error_lines: bool,
    /// L - character that introduces a local label.
    pub local_char: char,
    /// S - treat equated names as labels.
    pub equ_as_label: bool,
    /// V - write local labels to the symbol file.
    pub locals_in_symfile: bool,
    /// W - print warnings.
    pub warnings: bool,
    /// WS - operands may contain white space.
    pub white_space: bool,
    /// X - assume XREFs are in the section they are declared in.
    pub xref_in_section: bool,
    /// OP - optimise absolute long to PC relative.
    pub opt_pc_rel: bool,
    /// OS - optimise backward branches to short.
    pub opt_short_branch: bool,
    /// OW - optimise absolute long to absolute word.
    pub opt_abs_word: bool,
    /// OZ - optimise zero displacements away.
    pub opt_zero_disp: bool,
    /// OAQ - optimise ADD to ADDQ.
    pub opt_addq: bool,
    /// OSQ - optimise SUB to SUBQ.
    pub opt_subq: bool,
    /// OMQ - optimise MOVE to MOVEQ.
    pub opt_moveq: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            auto_even: true,
            alt_numeric: false,
            case_sensitive: false,
            descope_equ: false,
            print_error_lines: true,
            local_char: '@',
            equ_as_label: false,
            locals_in_symfile: false,
            warnings: true,
            white_space: false,
            xref_in_section: false,
            opt_pc_rel: false,
            opt_short_branch: false,
            opt_abs_word: false,
            opt_zero_disp: false,
            opt_addq: false,
            opt_subq: false,
            opt_moveq: false,
        }
    }
}

impl Options {
    /// Applies a comma separated option list such as `ae-,l.,ow+`.
    pub fn apply(&mut self, list: &str) -> Result<(), String> {
        let mut rest = list.trim();
        while !rest.is_empty() {
            rest = self.apply_one(rest)?;
            rest = rest.trim_start();
            if let Some(r) = rest.strip_prefix(',') {
                rest = r.trim_start();
            } else if !rest.is_empty() {
                return Err("Expected comma between options".into());
            }
        }
        Ok(())
    }

    /// Applies the option at the start of `s` and returns what follows it.
    fn apply_one<'a>(&mut self, s: &'a str) -> Result<&'a str, String> {
        let bytes = s.as_bytes();
        let mut n = 0;
        while n < bytes.len() && bytes[n].is_ascii_alphabetic() {
            n += 1;
        }
        let name = s[..n].to_ascii_lowercase();

        // The local label option takes any character as its argument.
        if name == "l" {
            let c = s[n..].chars().next().ok_or("Expecting '+' or '-' after option")?;
            self.local_char = match c {
                '+' => '.',
                '-' => '@',
                c => c,
            };
            return Ok(&s[n + c.len_utf8()..]);
        }
        // "l" directly followed by a letter, e.g. "lx".
        if name.len() == 2 && name.starts_with('l') && !matches!(&s[n..].chars().next(), Some('+' | '-')) {
            self.local_char = name.chars().nth(1).unwrap();
            return Ok(&s[n..]);
        }

        let on = match bytes.get(n) {
            Some(b'+') => true,
            Some(b'-') => false,
            _ => return Err("Expecting '+' or '-' after option".into()),
        };
        let flag = match name.as_str() {
            "ae" => &mut self.auto_even,
            "an" => &mut self.alt_numeric,
            "c" => &mut self.case_sensitive,
            "d" => &mut self.descope_equ,
            "e" => &mut self.print_error_lines,
            "s" => &mut self.equ_as_label,
            "v" => &mut self.locals_in_symfile,
            "w" => &mut self.warnings,
            "ws" => &mut self.white_space,
            "x" => &mut self.xref_in_section,
            "op" => &mut self.opt_pc_rel,
            "os" => &mut self.opt_short_branch,
            "ow" => &mut self.opt_abs_word,
            "oz" => &mut self.opt_zero_disp,
            "oaq" => &mut self.opt_addq,
            "osq" => &mut self.opt_subq,
            "omq" => &mut self.opt_moveq,
            _ => return Err("Unknown option".into()),
        };
        *flag = on;
        Ok(&s[n + 1..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_option_lists() {
        let mut o = Options::default();
        o.apply("ae-,ow+, oaq+").unwrap();
        assert!(!o.auto_even);
        assert!(o.opt_abs_word);
        assert!(o.opt_addq);
        assert!(!o.opt_subq);
    }

    #[test]
    fn local_label_character() {
        let mut o = Options::default();
        o.apply("l.").unwrap();
        assert_eq!(o.local_char, '.');
        o.apply("l-").unwrap();
        assert_eq!(o.local_char, '@');
        o.apply("l+,w-").unwrap();
        assert_eq!(o.local_char, '.');
        assert!(!o.warnings);
        o.apply("l:").unwrap();
        assert_eq!(o.local_char, ':');
    }

    #[test]
    fn rejects_unknown_options() {
        let mut o = Options::default();
        assert!(o.apply("zz+").is_err());
        assert!(o.apply("ae").is_err());
    }
}
