//! Off-screen pixel buffer. Screen areas are drawn here and then sent to the LCD in one transfer,
//! so a redraw never shows a half-cleared area.

use core::convert::Infallible;

use crate::{Display, DisplayResultExt};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

pub struct Canvas {
    /// Where the canvas goes on the screen.
    area: Rectangle,
    pixels: Vec<Rgb565>,
}

impl Canvas {
    /// Buffers larger than 16 KB end up in PSRAM.
    pub fn new(area: Rectangle) -> Self {
        let len = (area.size.width * area.size.height) as usize;
        Self {
            area,
            pixels: vec![Rgb565::BLACK; len],
        }
    }

    /// Where the canvas goes on the screen.
    pub fn area(&self) -> Rectangle {
        self.area
    }

    /// Copies another canvas into this one, at its place on the screen.
    pub fn copy_from(&mut self, other: &Canvas) {
        let target = Rectangle::new(other.area.top_left - self.area.top_left, other.area.size);
        self.fill_contiguous(&target, other.pixels.iter().copied())
            .ok();
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

    /// Sends part of the canvas (in canvas coordinates) to its place on the screen.
    pub fn present_area(&self, display: &mut Display, area: Rectangle) -> anyhow::Result<()> {
        let area = area.intersection(&self.bounding_box());
        let Some(bottom_right) = area.bottom_right() else {
            return Ok(());
        };
        let width = self.area.size.width as usize;
        let (x, row_len) = (area.top_left.x as usize, area.size.width as usize);
        let pixels = area.rows().flat_map(|y| {
            let start = y as usize * width + x;
            self.pixels[start..start + row_len].iter().copied()
        });
        let top_left = self.area.top_left + area.top_left;
        let bottom_right = self.area.top_left + bottom_right;
        display
            .set_pixels(
                top_left.x as u16,
                top_left.y as u16,
                bottom_right.x as u16,
                bottom_right.y as u16,
                pixels,
            )
            .display_err()
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
