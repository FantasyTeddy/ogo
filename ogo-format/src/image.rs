//! The description every ogo firmware image carries (docs/CONTRACTS.md, "Image description").
//!
//! It sits right after ESP-IDF's `esp_app_desc_t`, at [`OFFSET_IN_IMAGE`], and says what the image
//! is (`ogo-launcher`, `ogo-recovery`, `ogo-game`), its crate version and the git commit it was
//! built from. Apps declare it with `ogo_board::image_desc!`.

pub const MAGIC: [u8; 4] = *b"OGO2";
/// `esp_image_header_t` + `esp_image_segment_header_t` + `esp_app_desc_t`.
pub const OFFSET_IN_IMAGE: usize = 24 + 8 + 256;
/// Size of [`ImageDesc`] in the image.
pub const SIZE: usize = core::mem::size_of::<ImageDesc>();

/// What an image is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Launcher,
    Recovery,
    Game,
}

impl ImageKind {
    /// As stored in the image, e.g. "ogo-launcher".
    pub const fn name(self) -> &'static str {
        match self {
            ImageKind::Launcher => "ogo-launcher",
            ImageKind::Recovery => "ogo-recovery",
            ImageKind::Game => "ogo-game",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [ImageKind::Launcher, ImageKind::Recovery, ImageKind::Game]
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDesc {
    magic: [u8; 4],
    kind: [u8; 28],
    version: [u8; 32],
    commit: [u8; 16],
}

impl ImageDesc {
    /// Used by `image_desc!`. Longer strings are cut.
    pub const fn new(kind: ImageKind, version: &str, commit: &str) -> Self {
        Self {
            magic: MAGIC,
            kind: fixed(kind.name()),
            version: fixed(version),
            commit: fixed(commit),
        }
    }

    /// Reads a description from its bytes in an image; `None` if they don't hold one.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..SIZE)?;
        if bytes[..4] != MAGIC {
            return None;
        }
        Some(Self {
            magic: MAGIC,
            kind: array(&bytes[4..]),
            version: array(&bytes[32..]),
            commit: array(&bytes[64..]),
        })
    }

    /// Reads the description of a whole firmware image.
    pub fn in_image(image: &[u8]) -> Option<Self> {
        Self::from_bytes(image.get(OFFSET_IN_IMAGE..)?)
    }

    /// `None` for a kind this build doesn't know.
    pub fn kind(&self) -> Option<ImageKind> {
        ImageKind::from_name(self.kind_name())
    }

    /// The kind as stored, e.g. "ogo-launcher".
    pub fn kind_name(&self) -> &str {
        text(&self.kind)
    }

    /// The crate version, e.g. "0.1.0".
    pub fn version(&self) -> &str {
        text(&self.version)
    }

    /// The short git commit, e.g. "220357ce"; `None` for builds made without `cargo xtask`.
    pub fn commit(&self) -> Option<&str> {
        Some(text(&self.commit)).filter(|commit| !commit.is_empty())
    }
}

const fn fixed<const N: usize>(s: &str) -> [u8; N] {
    let bytes = s.as_bytes();
    let mut out = [0u8; N];
    let mut i = 0;
    // Leave room for a terminating zero.
    while i < bytes.len() && i < N - 1 {
        out[i] = bytes[i];
        i += 1;
    }
    out
}

/// The first `N` bytes of `bytes`.
fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes[..N].try_into().expect("slice has N bytes")
}

fn text(field: &[u8]) -> &str {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    core::str::from_utf8(&field[..end]).unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes of a description, as they appear in an image.
    fn bytes(desc: &ImageDesc) -> Vec<u8> {
        let mut bytes = Vec::from(desc.magic);
        bytes.extend_from_slice(&desc.kind);
        bytes.extend_from_slice(&desc.version);
        bytes.extend_from_slice(&desc.commit);
        bytes
    }

    #[test]
    fn layout_is_80_bytes() {
        assert_eq!(SIZE, 80);
    }

    #[test]
    fn round_trip() {
        let desc = ImageDesc::new(ImageKind::Launcher, "0.1.0", "0a1b710c");
        let read = ImageDesc::from_bytes(&bytes(&desc)).unwrap();
        assert_eq!(read.kind(), Some(ImageKind::Launcher));
        assert_eq!(read.kind_name(), "ogo-launcher");
        assert_eq!(read.version(), "0.1.0");
        assert_eq!(read.commit(), Some("0a1b710c"));
    }

    #[test]
    fn found_in_an_image() {
        let mut image = vec![0xE9; OFFSET_IN_IMAGE];
        image.extend(bytes(&ImageDesc::new(ImageKind::Game, "1.0.0", "")));
        image.extend([0; 100]);
        let desc = ImageDesc::in_image(&image).unwrap();
        assert_eq!(desc.kind(), Some(ImageKind::Game));
        assert_eq!(desc.commit(), None);
    }

    #[test]
    fn rejects_other_magic_and_short_input() {
        let mut old = bytes(&ImageDesc::new(ImageKind::Launcher, "0.1.0", ""));
        old[..4].copy_from_slice(b"OGO1");
        assert_eq!(ImageDesc::from_bytes(&old), None);
        assert_eq!(
            ImageDesc::from_bytes(&bytes(&ImageDesc::new(ImageKind::Game, "b", "c"))[..79]),
            None
        );
        assert_eq!(ImageDesc::in_image(&[0; 300]), None);
    }

    #[test]
    fn kind_names() {
        for kind in [ImageKind::Launcher, ImageKind::Recovery, ImageKind::Game] {
            assert_eq!(ImageKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ImageKind::from_name("ogo-emulator"), None);
    }

    #[test]
    fn long_strings_are_cut() {
        let desc = ImageDesc::new(ImageKind::Game, "1.0.0", "0123456789abcdef0123");
        assert_eq!(desc.commit(), Some("0123456789abcde"));
    }
}
