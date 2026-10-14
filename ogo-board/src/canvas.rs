//! Off-screen pixel buffer. Screen areas are drawn here and then sent to the LCD in one transfer,
//! so a redraw never shows a half-cleared area.
//!
//! A canvas can be shown larger than it is ([`Canvas::scaled`]): it's scaled up while it's sent.
//! Where a screen pixel falls between two of the canvas's, it gets their average, so a 1.5x picture
//! looks even instead of having every other pixel doubled.

use core::convert::Infallible;

use crate::{Display, DisplayResultExt};
use embedded_graphics::{
    pixelcolor::{raw::RawU16, IntoStorage, Rgb565},
    prelude::*,
    primitives::Rectangle,
};
use ogo_format::scale;

pub struct Canvas {
    /// Where the canvas goes on the screen, and its size.
    area: Rectangle,
    /// Its size on the screen; larger than `area.size` when it's scaled up.
    shown: Size,
    pixels: Vec<Rgb565>,
}

impl Canvas {
    /// Buffers larger than 16 KB end up in PSRAM.
    pub fn new(area: Rectangle) -> Self {
        Self::scaled(area.size, area)
    }

    /// A canvas of `size`, scaled up to fill `screen` when it's sent.
    pub fn scaled(size: Size, screen: Rectangle) -> Self {
        let len = (size.width * size.height) as usize;
        Self {
            area: Rectangle::new(screen.top_left, size),
            shown: screen.size,
            pixels: vec![Rgb565::BLACK; len],
        }
    }

    /// Where the canvas goes on the screen, at its own size.
    pub fn area(&self) -> Rectangle {
        self.area
    }

    /// Where the canvas shows on the screen, scaled if it is.
    pub fn screen_area(&self) -> Rectangle {
        Rectangle::new(self.area.top_left, self.shown)
    }

    fn is_scaled(&self) -> bool {
        self.shown != self.area.size
    }

    /// Copies another canvas into this one, at its place on the screen (scaled if it is).
    pub fn copy_from(&mut self, other: &Canvas) {
        let target = Rectangle::new(other.area.top_left - self.area.top_left, other.shown);
        if other.is_scaled() {
            self.fill_contiguous(&target, other.scaled_pixels()).ok();
        } else {
            self.fill_contiguous(&target, other.pixels.iter().copied())
                .ok();
        }
    }

    /// The canvas's pixels as shown on the screen, row by row.
    fn scaled_pixels(&self) -> ScaledPixels<'_> {
        let columns = scale::steps(self.area.size.width, self.shown.width);
        ScaledPixels {
            pixels: &self.pixels,
            width: self.area.size.width as usize,
            rows: scale::steps(self.area.size.height, self.shown.height),
            row: 0,
            buffer: vec![Rgb565::BLACK; columns.len()],
            next: columns.len(),
            columns,
        }
    }

    /// Darkens every pixel to half brightness, e.g. behind a menu.
    pub fn dim(&mut self) {
        for pixel in &mut self.pixels {
            *pixel = Rgb565::new(pixel.r() / 2, pixel.g() / 2, pixel.b() / 2);
        }
    }

    /// Sends the whole canvas to its place on the screen.
    pub fn present(&self, display: &mut Display) -> anyhow::Result<()> {
        self.present_area(display, self.bounding_box())
    }

    /// Sends part of the canvas (in canvas coordinates) to its place on the screen. A scaled
    /// canvas is always sent whole.
    pub fn present_area(&self, display: &mut Display, area: Rectangle) -> anyhow::Result<()> {
        let width = self.area.size.width as usize;
        if self.is_scaled() {
            let columns = scale::steps(self.area.size.width, self.shown.width);
            let rows = scale::steps(self.area.size.height, self.shown.height);
            let source = Rows::Scaled {
                pixels: &self.pixels,
                width,
                rows: &rows,
                columns: &columns,
            };
            return send(display, self.screen_area(), &source);
        }
        let area = area.intersection(&self.bounding_box());
        let source = Rows::Plain {
            pixels: &self.pixels,
            width,
            left: area.top_left.x as usize,
            top: area.top_left.y as usize,
        };
        let on_screen = Rectangle::new(self.area.top_left + area.top_left, area.size);
        send(display, on_screen, &source)
    }
}

/// Where the rows of pixels to send come from.
enum Rows<'a> {
    /// Part of a canvas, from (`left`, `top`) on.
    Plain {
        pixels: &'a [Rgb565],
        width: usize,
        left: usize,
        top: usize,
    },
    /// A scaled canvas, with the canvas rows and columns each screen row and column shows (see
    /// `ogo_format::scale`).
    Scaled {
        pixels: &'a [Rgb565],
        width: usize,
        rows: &'a [(usize, usize)],
        columns: &'a [(usize, usize)],
    },
}

/// Sends `area` of the screen, row by row from `source`. Whole rows go into the display's
/// transfer buffer, as many as fit.
fn send(display: &mut Display, area: Rectangle, source: &Rows) -> anyhow::Result<()> {
    let Some(bottom_right) = area.bottom_right() else {
        return Ok(());
    };
    // Sets the window and starts writing to the panel's memory, with no pixels yet
    display
        .set_pixels(
            area.top_left.x as u16,
            area.top_left.y as u16,
            bottom_right.x as u16,
            bottom_right.y as u16,
            core::iter::empty(),
        )
        .display_err()?;
    let row_bytes = 2 * area.size.width as usize;
    let rows = area.size.height as usize;
    let mut next = 0;
    // SAFETY: only pixel data follows the memory write that `set_pixels` started; nothing about
    // the panel's state changes.
    let interface = unsafe { display.dcs() };
    interface
        .write_with(|buffer| {
            let count = (buffer.len() / row_bytes).min(rows - next);
            write_rows(source, next, &mut buffer[..count * row_bytes], row_bytes);
            next += count;
            count * row_bytes
        })
        .display_err()
}

/// Writes rows from `first` on into `bytes`, `row_bytes` each, as the panel's big-endian RGB565.
/// In IRAM: run from flash, this loop waited on the cache more than it worked, as the other core
/// keeps the external memory busy (e.g. an emulator reading its ROM from PSRAM).
#[inline(never)]
#[link_section = ".iram1.ogo_board"]
fn write_rows(source: &Rows, first: usize, bytes: &mut [u8], row_bytes: usize) {
    let mut lines = ScaledLines::new();
    for (i, out) in bytes.chunks_exact_mut(row_bytes).enumerate() {
        let y = first + i;
        match *source {
            Rows::Plain {
                pixels,
                width,
                left,
                top,
            } => {
                let start = (top + y) * width + left;
                let row = &pixels[start..start + out.len() / 2];
                put_pixels(out, row.len(), |x| row[x].into_storage());
            }
            Rows::Scaled { rows, .. } => {
                let (y0, y1) = rows[y];
                let a = lines.get(source, y0, None);
                let count = out.len() / 2;
                if y0 == y1 {
                    let line = &lines.rows[a][..count];
                    put_pixels(out, count, |x| line[x]);
                } else {
                    let b = lines.get(source, y1, Some(a));
                    let (first, second) = (&lines.rows[a][..count], &lines.rows[b][..count]);
                    put_pixels(out, count, |x| average(first[x], second[x]));
                }
            }
        }
    }
}

/// Canvas rows scaled to the screen's width, the last two: each screen row shows one of them or
/// the average of both, so each canvas row is scaled once for the rows that show it.
struct ScaledLines {
    rows: [[u16; crate::display::WIDTH as usize]; 2],
    /// The canvas row each holds.
    canvas_rows: [Option<usize>; 2],
}

impl ScaledLines {
    fn new() -> Self {
        Self {
            rows: [[0; crate::display::WIDTH as usize]; 2],
            canvas_rows: [None; 2],
        }
    }

    /// Which of the two holds canvas row `y` of a scaled `source`, scaling it first if neither
    /// does, into the one that isn't `keep`.
    #[inline(always)]
    fn get(&mut self, source: &Rows, y: usize, keep: Option<usize>) -> usize {
        if let Some(i) = self.canvas_rows.iter().position(|&row| row == Some(y)) {
            return i;
        }
        let i = match keep {
            Some(keep) => 1 - keep,
            // The older row, as rows only go down
            None => match self.canvas_rows {
                [Some(a), Some(b)] if a > b => 1,
                [Some(_), None] => 1,
                _ => 0,
            },
        };
        if let Rows::Scaled {
            pixels,
            width,
            columns,
            ..
        } = *source
        {
            let row = &pixels[y * width..][..width];
            debug_assert!(columns.iter().all(|&(a, b)| a < width && b < width));
            for (out, &(x0, x1)) in self.rows[i].iter_mut().zip(columns) {
                // SAFETY: `scale::steps` only gives columns below the canvas width
                let left = unsafe { row.get_unchecked(x0) }.into_storage();
                *out = if x0 == x1 {
                    left
                } else {
                    average(left, unsafe { row.get_unchecked(x1) }.into_storage())
                };
            }
        }
        self.canvas_rows[i] = Some(y);
        i
    }
}

/// Writes `count` pixels, `pixel(x)` as raw RGB565, into `out` as the panel's big-endian bytes:
/// two pixels per 32-bit store where `out` allows (the display's buffers are word-aligned, and
/// rows of an even width keep that), else byte by byte.
#[inline(always)]
fn put_pixels(out: &mut [u8], count: usize, pixel: impl Fn(usize) -> u16) {
    // SAFETY: any bit pattern is a valid u32; what isn't aligned ends up in `prefix` or `suffix`
    let (prefix, words, suffix) = unsafe { out.align_to_mut::<u32>() };
    if prefix.is_empty() && suffix.is_empty() && words.len() * 2 == count {
        for (i, word) in words.iter_mut().enumerate() {
            let (a, b) = (pixel(2 * i), pixel(2 * i + 1));
            // In memory, the low half's bytes come first: a's high byte, a's low byte, then b's
            *word = a.swap_bytes() as u32 | (b.swap_bytes() as u32) << 16;
        }
    } else {
        for (x, pair) in out.chunks_exact_mut(2).enumerate().take(count) {
            let value = pixel(x);
            pair[0] = (value >> 8) as u8;
            pair[1] = value as u8;
        }
    }
}

impl OriginDimensions for Canvas {
    fn size(&self) -> Size {
        self.area.size
    }
}

impl DrawTarget for Canvas {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let Size { width, height } = self.area.size;
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.y >= 0 && (point.x as u32) < width && (point.y as u32) < height
            {
                self.pixels[point.y as usize * width as usize + point.x as usize] = color;
            }
        }
        Ok(())
    }

    /// Row by row instead of pixel by pixel when `area` lies within the canvas, e.g. for a picture
    /// copied in every frame.
    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        if area.intersection(&self.bounding_box()) != *area {
            return self.draw_iter(
                area.points()
                    .zip(colors)
                    .map(|(point, color)| Pixel(point, color)),
            );
        }
        let width = self.area.size.width as usize;
        let row_len = area.size.width as usize;
        let mut colors = colors.into_iter();
        for y in area.rows() {
            let start = y as usize * width + area.top_left.x as usize;
            for (pixel, color) in self.pixels[start..start + row_len]
                .iter_mut()
                .zip(&mut colors)
            {
                *pixel = color;
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let area = area.intersection(&self.bounding_box());
        let width = self.area.size.width as usize;
        for y in area.rows() {
            let start = y as usize * width + area.top_left.x as usize;
            self.pixels[start..start + area.size.width as usize].fill(color);
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.pixels.fill(color);
        Ok(())
    }
}

/// A scaled canvas's pixels as shown on the screen, row by row: each row is computed into a buffer
/// when it's reached. A concrete type rather than a chain of closures, so the display driver's
/// loop over the pixels can inline it.
struct ScaledPixels<'a> {
    pixels: &'a [Rgb565],
    width: usize,
    /// For each screen column and row, the canvas pixels it shows (see `ogo_format::scale`).
    columns: Vec<(usize, usize)>,
    rows: Vec<(usize, usize)>,
    /// The next screen row to compute, and the current one with the position in it.
    row: usize,
    buffer: Vec<Rgb565>,
    next: usize,
}

impl ScaledPixels<'_> {
    fn fill_row(&mut self) {
        let rows = self.rows[self.row];
        self.row += 1;
        self.next = 0;
        let buffer = &mut self.buffer;
        scaled_row(self.pixels, self.width, rows, &self.columns, |x, pixel| {
            buffer[x] = Rgb565::from(RawU16::new(pixel));
        });
    }
}

impl Iterator for ScaledPixels<'_> {
    type Item = Rgb565;

    #[inline]
    fn next(&mut self) -> Option<Rgb565> {
        if self.next == self.buffer.len() {
            if self.row == self.rows.len() {
                return None;
            }
            self.fill_row();
        }
        let pixel = self.buffer[self.next];
        self.next += 1;
        Some(pixel)
    }
}

/// Computes one screen row of a scaled picture: `rows` are the picture rows it shows (see
/// `ogo_format::scale`), `columns` the same for each screen column; `put(x, pixel)` gets the
/// screen row's pixels as raw RGB565.
#[inline(always)]
fn scaled_row(
    pixels: &[Rgb565],
    width: usize,
    (y0, y1): (usize, usize),
    columns: &[(usize, usize)],
    mut put: impl FnMut(usize, u16),
) {
    let row0 = &pixels[y0 * width..][..width];
    let row1 = &pixels[y1 * width..][..width];
    for (x, &(x0, x1)) in columns.iter().enumerate() {
        let top = average(row0[x0].into_storage(), row0[x1].into_storage());
        let pixel = if y0 == y1 {
            top
        } else {
            average(
                top,
                average(row1[x0].into_storage(), row1[x1].into_storage()),
            )
        };
        put(x, pixel);
    }
}

/// The average of two RGB565 colours, all three channels at once: each channel's lowest bit is
/// masked before the halving, so it can't spill into the next channel.
#[inline(always)]
fn average(a: u16, b: u16) -> u16 {
    (a & b) + (((a ^ b) & 0xf7de) >> 1)
}
