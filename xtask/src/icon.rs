//! `cargo xtask icon <game> <image.png>`: converts a PNG into the game's `icon.rgb565`.

use std::{fs, path::Path};

use anyhow::Context;
use image::imageops::{self, FilterType};
use ogo_format::game::{rgb565_le, ICON_BYTES, ICON_SIZE};

/// Scales `input` to 64x64 and writes it to `output` as RGB565. Transparent parts become black,
/// like the launcher's background.
pub fn convert(input: &Path, output: &Path) -> anyhow::Result<()> {
    let image = image::open(input)
        .with_context(|| format!("can't read {}", input.display()))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    if width != height {
        println!(
            "{} is {width}x{height}, not square: it gets stretched",
            input.display()
        );
    }
    let icon = imageops::resize(&image, ICON_SIZE, ICON_SIZE, FilterType::Lanczos3);
    let mut pixels = Vec::with_capacity(ICON_BYTES);
    for pixel in icon.pixels() {
        let [r, g, b, a] = pixel.0;
        let over_black = |c: u8| (u16::from(c) * u16::from(a) / 255) as u8;
        pixels.extend_from_slice(&rgb565_le([over_black(r), over_black(g), over_black(b)]));
    }
    fs::write(output, pixels).with_context(|| format!("can't write {}", output.display()))?;
    println!("Wrote {}", output.display());
    Ok(())
}
