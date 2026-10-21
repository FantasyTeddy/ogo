//! What an ogo firmware image is: a small description every ogo app carries at a fixed offset in
//! its image, right after ESP-IDF's own `esp_app_desc_t` (section `.rodata_custom_desc`, see
//! ESP-IDF's "app image format" docs).
//!
//! ESP-IDF's description can't tell our apps apart: with esp-idf-sys, every app's project name is
//! `libespidf` and all share one version setting. This one says `ogo-launcher`, `ogo-recovery` or
//! `ogo-app`, the crate version, and the git commit it was built from (only for builds made with
//! `cargo xtask`, which sets `OGO_COMMIT`).
//!
//! Every app declares it with [`image_desc!`]. [`crate::Board::take`] reads it, which keeps the
//! linker from dropping it, and makes an app without one fail to link.

use esp_idf_svc::sys::{self, esp};
pub use ogo_format::image::{ImageDesc, ImageKind};
use ogo_format::image::{OFFSET_IN_IMAGE, SIZE};

use crate::boot::find_app_partition;

/// Declares this app's [`ImageDesc`]. Use once, at the crate root of every ogo app, with an
/// [`ImageKind`] variant: `ogo_board::image_desc!(Launcher);`
///
/// The commit comes from `OGO_COMMIT` at compile time; cargo rebuilds the app when it changes.
#[macro_export]
macro_rules! image_desc {
    ($kind:ident) => {
        #[no_mangle]
        #[used]
        #[link_section = ".rodata_custom_desc"]
        pub static OGO_IMAGE_DESC: $crate::image::ImageDesc = $crate::image::ImageDesc::new(
            $crate::image::ImageKind::$kind,
            env!("CARGO_PKG_VERSION"),
            match option_env!("OGO_COMMIT") {
                Some(commit) => commit,
                None => "",
            },
        );
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
    let mut bytes = [0u8; SIZE];
    esp!(unsafe {
        sys::esp_partition_read(
            partition,
            OFFSET_IN_IMAGE,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    })
    .ok()?;
    ImageDesc::from_bytes(&bytes)
}
