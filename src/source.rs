//! Source text handling and file lookup.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A line of source, along with where it came from.
#[derive(Clone, Debug)]
pub struct SrcLine {
    pub text: Rc<str>,
    pub file: u32,
    pub line: u32,
}

/// Decodes source bytes. Each byte becomes the character with the same
/// code, so that text in any 8-bit or multi-byte encoding passes through
/// the assembler unchanged.
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// The inverse of `decode`.
pub fn encode(text: &str) -> Vec<u8> {
    text.chars().map(|c| c as u32 as u8).collect()
}

/// Splits source text into lines. A DOS end-of-file marker ends the text.
pub fn split_lines(bytes: &[u8], file: u32) -> Vec<SrcLine> {
    let bytes = match bytes.iter().position(|&b| b == 0x1a) {
        Some(n) => &bytes[..n],
        None => bytes,
    };
    let mut lines = Vec::new();
    if bytes.is_empty() {
        return lines;
    }
    for (n, raw) in bytes.split(|&b| b == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        lines.push(SrcLine { text: decode(raw).into(), file, line: n as u32 + 1 });
    }
    // A final newline does not start another line.
    if bytes.ends_with(b"\n") {
        lines.pop();
    }
    lines
}

/// Converts a file name as written in the source to a native path.
pub fn native_path(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(name)
    } else {
        // Drop any drive letter and use forward slashes.
        let name = name.replace('\\', "/");
        let b = name.as_bytes();
        let name = if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            name[2..].to_string()
        } else {
            name
        };
        PathBuf::from(name)
    }
}

/// Finds a file whose name matches `path` without regard to case, for
/// sources written on file systems that do not distinguish case.
fn find_ignoring_case(path: &Path) -> Option<PathBuf> {
    let mut found = PathBuf::new();
    for comp in path.components() {
        use std::path::Component::*;
        match comp {
            Prefix(_) | RootDir | CurDir | ParentDir => found.push(comp.as_os_str()),
            Normal(name) => {
                let exact = found.join(name);
                if exact.exists() {
                    found = exact;
                    continue;
                }
                let dir = if found.as_os_str().is_empty() { Path::new(".") } else { found.as_path() };
                let wanted = name.to_string_lossy().to_lowercase();
                let mut matches: Vec<_> = fs::read_dir(dir)
                    .ok()?
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .filter(|n| n.to_string_lossy().to_lowercase() == wanted)
                    .collect();
                matches.sort();
                found.push(matches.into_iter().next()?);
            }
        }
    }
    Some(found)
}

/// Locates a file named in the source or on the command line.
pub fn locate(name: &str, include_dirs: &[PathBuf], relative_to: Option<&Path>) -> Option<PathBuf> {
    let path = native_path(name);
    let mut candidates = vec![path.clone()];
    if path.is_relative() {
        for dir in include_dirs {
            candidates.push(dir.join(&path));
        }
        if let Some(dir) = relative_to {
            candidates.push(dir.join(&path));
        }
    }
    for c in &candidates {
        if c.is_file() {
            return Some(c.clone());
        }
    }
    if cfg!(windows) {
        return None;
    }
    candidates
        .iter()
        .filter_map(|c| find_ignoring_case(c))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_with_any_ending() {
        let l = split_lines(b"a\r\nb\nc", 0);
        let t: Vec<&str> = l.iter().map(|l| &*l.text).collect();
        assert_eq!(t, ["a", "b", "c"]);
        assert_eq!(l[2].line, 3);
        assert_eq!(split_lines(b"a\n", 0).len(), 1);
        assert_eq!(split_lines(b"a\n\n", 0).len(), 2);
        assert_eq!(split_lines(b"", 0).len(), 0);
        assert_eq!(split_lines(b"a\nb\x1ajunk", 0).len(), 2);
    }

    #[test]
    fn bytes_round_trip() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(encode(&decode(&bytes)), bytes);
    }

    #[cfg(not(windows))]
    #[test]
    fn converts_dos_paths() {
        assert_eq!(native_path("src\\main.68k"), PathBuf::from("src/main.68k"));
        assert_eq!(native_path("d:\\src\\a.bin"), PathBuf::from("/src/a.bin"));
    }
}
