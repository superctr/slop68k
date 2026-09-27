//! Messages produced during assembly.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Text printed by INFORM with severity 0.
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub severity: Severity,
    /// Source file and line the message refers to, if any.
    pub location: Option<(String, u32)>,
    pub text: String,
    /// The offending source line.
    pub source: Option<String>,
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let kind = match self.severity {
            Severity::Info => return write!(f, "{}", self.text),
            Severity::Warning => "Warning",
            Severity::Error => "Error",
            Severity::Fatal => "Fatal",
        };
        match &self.location {
            Some((file, line)) => write!(f, "{}({}) : {} : {}", file, line, kind, self.text)?,
            None => write!(f, "{} : {}", kind, self.text)?,
        }
        if let Some(src) = &self.source {
            write!(f, "\n{}", src)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_asm68k() {
        let m = Message {
            severity: Severity::Error,
            location: Some(("main.68k".into(), 12)),
            text: "Illegal addressing mode".into(),
            source: Some("\tmove.b\td0,a0".into()),
        };
        assert_eq!(m.to_string(), "main.68k(12) : Error : Illegal addressing mode\n\tmove.b\td0,a0");
        let m = Message { severity: Severity::Info, location: None, text: "hello".into(), source: None };
        assert_eq!(m.to_string(), "hello");
    }
}
