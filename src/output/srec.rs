//! Motorola S-records.

/// Writes a memory image as S-records.
pub fn write(base: u32, image: &[u8], entry: u32) -> Vec<u8> {
    let mut out = String::new();
    record(&mut out, 0, 0, 2, b"slop68k");
    for (n, data) in image.chunks(32).enumerate() {
        record(&mut out, 3, base.wrapping_add(n as u32 * 32), 4, data);
    }
    record(&mut out, 7, entry, 4, &[]);
    out.into_bytes()
}

fn record(out: &mut String, kind: u8, addr: u32, addr_len: usize, data: &[u8]) {
    let count = (addr_len + data.len() + 1) as u8;
    let mut sum = count as u32;
    out.push_str(&format!("S{}{:02X}", kind, count));
    for b in &addr.to_be_bytes()[4 - addr_len..] {
        out.push_str(&format!("{:02X}", b));
        sum += *b as u32;
    }
    for b in data {
        out.push_str(&format!("{:02X}", b));
        sum += *b as u32;
    }
    out.push_str(&format!("{:02X}\r\n", !sum as u8));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_records() {
        let s = String::from_utf8(write(0x100, &[0x4e, 0x75], 0x100)).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("S0"));
        assert_eq!(lines[1], "S307000001004E7534");
        assert_eq!(lines[2], "S70500000100F9");
    }
}
