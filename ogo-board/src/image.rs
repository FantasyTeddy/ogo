//! What an ogo firmware image is: a small description every ogo app carries at a fixed offset in
//! its image, right after ESP-IDF's own `esp_app_desc_t` (section `.rodata_custom_desc`, see
//! ESP-IDF's "app image format" docs).
//!
//! ESP-IDF's description can't tell our apps apart: with esp-idf-sys, every app's project name is
//! `libespidf` and all share one version setting. This one says `ogo-launcher`, `ogo-recovery` or
//! `ogo-game`, the crate version, and the git commit it was built from (only for builds made with
//! `cargo xtask`, which sets `OGO_COMMIT`).
//!
//! Every app declares it with [`image_desc!`]. [`crate::Board::take`] reads it, which keeps the
//! linker from dropping it, and makes an app without one fail to link.

use esp_idf_svc::sys::{self, esp};

use crate::boot::find_app_partition;

const MAGIC: [u8; 4] = *b"OGO2";
/// `esp_image_header_t` + `esp_image_segment_header_t` + `esp_app_desc_t`.
const OFFSET_IN_IMAGE: usize = 24 + 8 + 256;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ImageDesc {
    magic: [u8; 4],
    kind: [u8; 28],
    version: [u8; 32],
    commit: [u8; 16],
}

impl ImageDesc {
    /// Used by [`image_desc!`]. Longer strings are cut.
    pub const fn new(kind: &str, version: &str, commit: &str) -> Self {
        Self { magic: MAGIC, kind: fixed(kind), version: fixed(version), commit: fixed(commit) }
    }

    /// e.g. "ogo-launcher"
    pub fn kind(&self) -> &str {
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

/// Declares this app's [`ImageDesc`]. Use once, at the crate root of every ogo app:
/// `ogo_board::image_desc!("ogo-launcher");`
///
/// The commit comes from `OGO_COMMIT` at compile time; cargo rebuilds the app when it changes.
#[macro_export]
macro_rules! image_desc {
    ($kind:expr) => {
        #[no_mangle]
        #[used]
        #[link_section = ".rodata_custom_desc"]
        pub static OGO_IMAGE_DESC: $crate::image::ImageDesc =
            $crate::image::ImageDesc::new($kind, env!("CARGO_PKG_VERSION"), match option_env!("OGO_COMMIT") {
                Some(commit) => commit,
                None => "",
            });
    };
}

extern "Rust" {
    /// Defined by [`image_desc!`] in the app.
    static OGO_IMAGE_DESC: ImageDesc;
}

/// The running app's description.
pub fn running() -> ImageDesc {
    // A volatile read: the reference must survive optimization, so the linker keeps the symbol.
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(OGO_IMAGE_DESC)) }
}

/// The description of the image in app slot `label`, or `None` if it holds no ogo image.
pub fn in_slot(label: &str) -> Option<ImageDesc> {
    let partition = find_app_partition(label).ok()?;
    let mut bytes = [0u8; core::mem::size_of::<ImageDesc>()];
    esp!(unsafe { sys::esp_partition_read(partition, OFFSET_IN_IMAGE, bytes.as_mut_ptr().cast(), bytes.len()) }).ok()?;
    if bytes[..4] != MAGIC {
        return None;
    }
    // All fields are byte arrays, so any bytes form a valid ImageDesc.
    Some(unsafe { core::mem::transmute::<[u8; core::mem::size_of::<ImageDesc>()], ImageDesc>(bytes) })
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

fn text(field: &[u8]) -> &str {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    core::str::from_utf8(&field[..end]).unwrap_or("?")
}
