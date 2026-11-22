//! Pictures: PNG files and the conversion to and from the device's RGB565.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use adventure_format::{TILE, TRANSPARENT};

/// An RGBA picture, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
}

impl Rgba {
    pub fn new(width: usize, height: usize) -> Rgba {
        Rgba {
            width,
            height,
            pixels: vec![[0; 4]; width * height],
        }
    }

    pub fn load(path: &Path) -> Result<Rgba, String> {
        let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let file = File::open(path).map_err(|e| error(&e))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|e| error(&e))?;
        let mut buffer = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or_else(|| error(&"too big"))?
        ];
        let info = reader.next_frame(&mut buffer).map_err(|e| error(&e))?;
        let bytes = &buffer[..info.buffer_size()];
        let pixels = match info.color_type {
            png::ColorType::Rgba => bytes.as_chunks::<4>().0.to_vec(),
            png::ColorType::Rgb => bytes
                .as_chunks::<3>()
                .0
                .iter()
                .map(|&[r, g, b]| [r, g, b, 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[g, a]| [g, g, g, a])
                .collect(),
            png::ColorType::Grayscale => bytes.iter().map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return Err(error(&"unexpected palette after expanding")),
        };
        Ok(Rgba {
            width: info.width as usize,
            height: info.height as usize,
            pixels,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let file = File::create(path).map_err(|e| error(&e))?;
        let mut encoder =
            png::Encoder::new(BufWriter::new(file), self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| error(&e))?;
        writer
            .write_image_data(self.pixels.as_flattened())
            .map_err(|e| error(&e))
    }

    /// Cells of 16 × 16 per row and column.
    pub fn cells(&self) -> (usize, usize) {
        (self.width / TILE, self.height / TILE)
    }

    /// Cell `n`, counted row by row, as RGB565; None if there isn't one.
    pub fn cell(&self, n: usize) -> Option<Vec<u16>> {
        let (columns, rows) = self.cells();
        if n >= columns * rows {
            return None;
        }
        let (cx, cy) = (n % columns * TILE, n / columns * TILE);
        let mut out = Vec::with_capacity(TILE * TILE);
        for y in 0..TILE {
            for x in 0..TILE {
                out.push(to_rgb565(self.pixels[(cy + y) * self.width + cx + x]));
            }
        }
        Some(out)
    }

    /// Copies RGB565 pixels `width` wide into (x, y).
    pub fn paste(&mut self, pixels: &[u16], width: usize, x: usize, y: usize) {
        for (row, line) in pixels.chunks_exact(width).enumerate() {
            for (col, &pixel) in line.iter().enumerate() {
                self.pixels[(y + row) * self.width + x + col] = to_rgba(pixel);
            }
        }
    }
}

/// Mostly transparent pixels become [`TRANSPARENT`]; an opaque pixel that happens to have its
/// colour is nudged by one step of green, which nobody sees.
pub fn to_rgb565([r, g, b, a]: [u8; 4]) -> u16 {
    if a < 128 {
        return TRANSPARENT;
    }
    let pixel = (r as u16 >> 3) << 11 | (g as u16 >> 2) << 5 | b as u16 >> 3;
    if pixel == TRANSPARENT {
        pixel ^ 0x0020
    } else {
        pixel
    }
}

pub fn to_rgba(pixel: u16) -> [u8; 4] {
    if pixel == TRANSPARENT {
        return [0; 4];
    }
    let (r, g, b) = (
        (pixel >> 11) as u8,
        ((pixel >> 5) & 0x3F) as u8,
        (pixel & 0x1F) as u8,
    );
    [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2, 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip() {
        for pixel in [0u16, 0xFFFF, 0x07E0, 0x1234, 0xF800, TRANSPARENT] {
            assert_eq!(to_rgb565(to_rgba(pixel)), pixel);
        }
        assert_eq!(
            to_rgb565([255, 0, 255, 255]),
            0xF83F,
            "real magenta isn't transparent"
        );
        assert_eq!(to_rgb565([10, 20, 30, 0]), TRANSPARENT);
    }

    #[test]
    fn png_round_trip_and_cells() {
        let dir = std::env::temp_dir().join(format!("adventure-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut picture = Rgba::new(32, 16);
        picture.paste(&[0x07E0; TILE * TILE], TILE, 16, 0);
        let path = dir.join("cells.png");
        picture.save(&path).unwrap();
        let loaded = Rgba::load(&path).unwrap();
        assert_eq!(loaded, picture);
        assert_eq!(loaded.cells(), (2, 1));
        assert_eq!(loaded.cell(0).unwrap()[0], TRANSPARENT);
        assert_eq!(loaded.cell(1).unwrap()[255], 0x07E0);
        assert_eq!(loaded.cell(2), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
