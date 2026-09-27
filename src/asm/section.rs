//! Sections and the code assembled into them.

/// A run of bytes placed at a known position.
#[derive(Clone, Debug, Default)]
pub struct Chunk {
    /// Address of the first byte. For relocatable sections this is the
    /// offset from the start of the section.
    pub addr: u32,
    pub data: Vec<u8>,
}

impl Chunk {
    pub fn end(&self) -> u32 {
        self.addr.wrapping_add(self.data.len() as u32)
    }
}

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    /// Code is placed at fixed addresses with ORG.
    pub absolute: bool,
    pub chunks: Vec<Chunk>,
    /// Group the section belongs to.
    pub group: Option<usize>,
    /// Holds no initialised data.
    pub bss: bool,
    /// Size of the section, which for BSS sections has no data behind it.
    pub size: u32,
    /// Required alignment in bytes.
    pub align: u32,
    /// Location counter to return to when the section is resumed.
    pub pc: u32,
    /// Address of the section once the layout has been decided.
    pub base: u32,
}

impl Section {
    pub fn new(name: &str, absolute: bool) -> Section {
        Section {
            name: name.to_string(),
            absolute,
            chunks: vec![Chunk::default()],
            group: None,
            bss: false,
            size: 0,
            align: if absolute { 1 } else { 2 },
            pc: 0,
            base: 0,
        }
    }

    pub fn has_data(&self) -> bool {
        self.chunks.iter().any(|c| !c.data.is_empty())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub name: String,
    pub org: Option<u32>,
    pub obj: Option<u32>,
    pub bss: bool,
    pub word: bool,
    pub size: Option<u32>,
    pub file: Option<String>,
    pub over: Option<usize>,
}

/// The lowest and highest addresses covered by a set of chunks.
pub fn extent<'a>(chunks: impl Iterator<Item = &'a Chunk>) -> Option<(u32, u32)> {
    let mut range: Option<(u32, u32)> = None;
    for c in chunks.filter(|c| !c.data.is_empty()) {
        range = Some(match range {
            None => (c.addr, c.end()),
            Some((lo, hi)) => (lo.min(c.addr), hi.max(c.end())),
        });
    }
    range
}

/// Builds a memory image from chunks. Gaps are filled with zeros and later
/// chunks overwrite earlier ones.
pub fn flatten<'a>(chunks: impl Iterator<Item = &'a Chunk> + Clone) -> (u32, Vec<u8>) {
    let Some((lo, hi)) = extent(chunks.clone()) else {
        return (0, Vec::new());
    };
    let mut image = vec![0u8; (hi - lo) as usize];
    for c in chunks.filter(|c| !c.data.is_empty()) {
        let at = (c.addr - lo) as usize;
        image[at..at + c.data.len()].copy_from_slice(&c.data);
    }
    (lo, image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_spans_all_chunks() {
        let chunks = [
            Chunk { addr: 0x100, data: vec![1] },
            Chunk { addr: 0x110, data: vec![2] },
            Chunk { addr: 0x108, data: vec![3] },
            Chunk { addr: 0x200, data: vec![] },
        ];
        let (base, image) = flatten(chunks.iter());
        assert_eq!(base, 0x100);
        assert_eq!(image.len(), 0x11);
        assert_eq!(image[0], 1);
        assert_eq!(image[8], 3);
        assert_eq!(image[0x10], 2);
    }

    #[test]
    fn later_chunks_overwrite() {
        let chunks = [Chunk { addr: 0, data: vec![1, 1, 1] }, Chunk { addr: 1, data: vec![2] }];
        assert_eq!(flatten(chunks.iter()).1, [1, 2, 1]);
    }

    #[test]
    fn empty_image() {
        let chunks = [Chunk { addr: 0x100, data: vec![] }];
        assert_eq!(flatten(chunks.iter()), (0, vec![]));
    }
}
