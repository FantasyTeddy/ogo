//! The LCD controller: video RAM, OAM, registers, timing (lines, modes, LY, the VBlank and STAT
//! interrupts) and drawing.
//!
//! Each visible line is drawn in one go when it enters HBlank, with the registers as they are
//! then: background, window and up to 10 sprites, through their palettes into `frame`.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::{INT_STAT, INT_VBLANK, SCREEN_HEIGHT, SCREEN_WIDTH};

pub const LINE_CYCLES: u32 = 456;
pub const LINES: u8 = 154;
pub const VISIBLE_LINES: u8 = 144;
const MODE2_END: u32 = 80;
const MODE3_END: u32 = 252;
const SPRITES_PER_LINE: usize = 10;
/// Tiles decoded per line: one more than the 20 on screen, for fine scrolling.
const LINE_TILES: usize = SCREEN_WIDTH / 8 + 1;

pub(crate) struct Ppu {
    pub vram: Box<[u8; 0x2000]>,
    pub oam: [u8; 0xa0],
    pub lcdc: u8,
    /// Only the interrupt-select bits 3-6; mode and LYC=LY are computed when read.
    pub stat: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    /// Cycle within the current line.
    dot: u32,
    /// The STAT interrupt line: the interrupt fires when it goes up.
    stat_line: bool,
    /// LY has matched WY in this frame, so the window may show from here on.
    window_triggered: bool,
    /// The window's own line counter: it only counts lines on which the window was drawn.
    window_line: u8,
    /// The picture, one shade (0 = white to 3 = black) per pixel, row by row.
    pub frame: Vec<u8>,
    /// Set when a frame is complete (VBlank begins).
    pub frame_done: bool,
    /// Lines are drawn into `frame`; off for frames that are skipped.
    pub drawing: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
            vram: crate::zeroed(),
            oam: [0; 0xa0],
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xfc,
            obp0: 0xff,
            obp1: 0xff,
            wy: 0,
            wx: 0,
            dot: 0,
            stat_line: false,
            window_triggered: false,
            window_line: 0,
            frame: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT],
            frame_done: false,
            drawing: true,
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn enabled(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    pub fn mode(&self) -> u8 {
        if !self.enabled() {
            0
        } else if self.ly >= VISIBLE_LINES {
            1
        } else if self.dot < MODE2_END {
            2
        } else if self.dot < MODE3_END {
            3
        } else {
            0
        }
    }

    pub fn read_stat(&self) -> u8 {
        let coincidence = if self.enabled() && self.ly == self.lyc {
            4
        } else {
            0
        };
        0x80 | self.stat | coincidence | self.mode()
    }

    pub fn write_stat(&mut self, value: u8) -> u8 {
        self.stat = value & 0x78;
        self.update_stat_line()
    }

    pub fn write_lyc(&mut self, value: u8) -> u8 {
        self.lyc = value;
        self.update_stat_line()
    }

    pub fn write_lcdc(&mut self, value: u8) -> u8 {
        let was_enabled = self.enabled();
        self.lcdc = value;
        if was_enabled && !self.enabled() {
            // An LCD that is off shows white
            self.ly = 0;
            self.dot = 0;
            self.stat_line = false;
            self.window_triggered = false;
            self.window_line = 0;
            self.frame.fill(0);
        }
        self.update_stat_line()
    }

    /// Cycles until the next mode change (a new line counts as one); `u32::MAX` while the LCD is
    /// off.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    pub fn cycles_to_mode_change(&self) -> u32 {
        if !self.enabled() {
            return u32::MAX;
        }
        let next = if self.ly >= VISIBLE_LINES {
            LINE_CYCLES
        } else if self.dot < MODE2_END {
            MODE2_END
        } else if self.dot < MODE3_END {
            MODE3_END
        } else {
            LINE_CYCLES
        };
        next - self.dot
    }

    /// Advances by `cycles`; returns the interrupts requested on the way (IF bits).
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    pub fn tick(&mut self, mut cycles: u32) -> u8 {
        if !self.enabled() {
            return 0;
        }
        let mut irq = 0;
        while cycles > 0 {
            // Step to the next mode change at most, so no STAT edge or line is skipped
            let step = self.cycles_to_mode_change().min(cycles);
            self.dot += step;
            cycles -= step;
            if self.dot == MODE3_END && self.ly < VISIBLE_LINES {
                if self.drawing {
                    self.draw_line();
                }
            } else if self.dot == LINE_CYCLES {
                self.dot = 0;
                self.ly = (self.ly + 1) % LINES;
                if self.ly == VISIBLE_LINES {
                    irq |= INT_VBLANK;
                    self.frame_done = true;
                } else if self.ly == 0 {
                    self.window_triggered = false;
                    self.window_line = 0;
                }
            }
            irq |= self.update_stat_line();
        }
        irq
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn update_stat_line(&mut self) -> u8 {
        if !self.enabled() {
            return 0;
        }
        let mode = self.mode();
        let line = (self.stat & 0x40 != 0 && self.ly == self.lyc)
            || (self.stat & 0x08 != 0 && mode == 0)
            || (self.stat & 0x10 != 0 && mode == 1)
            || (self.stat & 0x20 != 0 && mode == 2);
        let rising = line && !self.stat_line;
        self.stat_line = line;
        if rising {
            INT_STAT
        } else {
            0
        }
    }

    // Drawing

    /// The two bit planes of row `row` (0-7) of a background or window tile.
    #[inline(always)]
    fn tile_row(&self, tile: u8, row: u8) -> (u8, u8) {
        let addr = if self.lcdc & 0x10 != 0 {
            tile as usize * 16
        } else {
            (0x1000 + tile as i8 as isize * 16) as usize
        } + row as usize * 2;
        (self.vram[addr], self.vram[addr + 1])
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn draw_line(&mut self) {
        if self.ly == self.wy {
            self.window_triggered = true;
        }
        // Colour numbers (0-3) before the palette; sprites need them for their priority
        let mut colors = [0u8; SCREEN_WIDTH];
        if self.lcdc & 0x01 != 0 {
            self.draw_background(&mut colors);
            self.draw_window(&mut colors);
        }
        let row = self.ly as usize * SCREEN_WIDTH;
        let line = &mut self.frame[row..row + SCREEN_WIDTH];
        if self.lcdc & 0x01 != 0 {
            let palette = shades(self.bgp);
            for (shade, &color) in line.iter_mut().zip(&colors) {
                *shade = palette[color as usize & 3];
            }
        } else {
            // Background and window off: white
            line.fill(0);
        }
        if self.lcdc & 0x02 != 0 {
            self.draw_sprites(&colors);
        }
    }

    /// The colour numbers of `LINE_TILES` tiles of a tile map on map line `y`, from tile column
    /// `first_column` on (wrapping around after 32).
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn decode_tiles(&self, map: usize, first_column: usize, y: u8) -> [u8; LINE_TILES * 8] {
        let mut pixels = [0u8; LINE_TILES * 8];
        let map_row = map + (y as usize / 8) * 32;
        for (i, tile_pixels) in pixels.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let tile = self.vram[map_row + (first_column + i) % 32];
            let (low, high) = self.tile_row(tile, y & 7);
            *tile_pixels = decode_row(low, high);
        }
        pixels
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn draw_background(&self, colors: &mut [u8; SCREEN_WIDTH]) {
        let map = if self.lcdc & 0x08 != 0 {
            0x1c00
        } else {
            0x1800
        };
        let y = self.scy.wrapping_add(self.ly);
        let pixels = self.decode_tiles(map, self.scx as usize / 8, y);
        let fine = self.scx as usize & 7;
        colors.copy_from_slice(&pixels[fine..fine + SCREEN_WIDTH]);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn draw_window(&mut self, colors: &mut [u8; SCREEN_WIDTH]) {
        if self.lcdc & 0x20 == 0 || !self.window_triggered || self.wx > 166 {
            return;
        }
        let map = if self.lcdc & 0x40 != 0 {
            0x1c00
        } else {
            0x1800
        };
        let pixels = self.decode_tiles(map, 0, self.window_line);
        // The window starts at WX - 7, which can be left of the screen
        let (screen_x, window_x) = match self.wx.checked_sub(7) {
            Some(x) => (x as usize, 0),
            None => (0, 7 - self.wx as usize),
        };
        let width = SCREEN_WIDTH - screen_x;
        colors[screen_x..].copy_from_slice(&pixels[window_x..window_x + width]);
        self.window_line += 1;
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn draw_sprites(&mut self, colors: &[u8; SCREEN_WIDTH]) {
        let height = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let ly = self.ly as i16;
        // The first 10 sprites in OAM on this line, then sorted by priority: the smaller X wins,
        // on a tie the one earlier in OAM
        let mut sprites = [0usize; SPRITES_PER_LINE];
        let mut count = 0;
        for index in 0..40 {
            let top = self.oam[index * 4] as i16 - 16;
            if (top..top + height).contains(&ly) {
                sprites[count] = index;
                count += 1;
                if count == SPRITES_PER_LINE {
                    break;
                }
            }
        }
        let sprites = &mut sprites[..count];
        // Insertion sort: stable, and without the allocation of `sort_by_key`
        for i in 1..count {
            let mut j = i;
            while j > 0 && self.oam[sprites[j - 1] * 4 + 1] > self.oam[sprites[j] * 4 + 1] {
                sprites.swap(j - 1, j);
                j -= 1;
            }
        }

        let row = self.ly as usize * SCREEN_WIDTH;
        let mut taken = [false; SCREEN_WIDTH];
        for &index in sprites.iter() {
            let [y, x, tile, attributes] = self.oam[index * 4..index * 4 + 4] else {
                unreachable!()
            };
            let mut sprite_row = (ly - (y as i16 - 16)) as usize;
            if attributes & 0x40 != 0 {
                sprite_row = height as usize - 1 - sprite_row;
            }
            let tile = if height == 16 { tile & 0xfe } else { tile };
            let addr = tile as usize * 16 + sprite_row * 2;
            let (low, high) = (self.vram[addr], self.vram[addr + 1]);
            let palette = if attributes & 0x10 != 0 {
                self.obp1
            } else {
                self.obp0
            };
            for i in 0..8 {
                let screen_x = x as isize - 8 + i as isize;
                if !(0..SCREEN_WIDTH as isize).contains(&screen_x) {
                    continue;
                }
                let screen_x = screen_x as usize;
                let bit = if attributes & 0x20 != 0 { i } else { 7 - i };
                let color = (high >> bit & 1) << 1 | (low >> bit & 1);
                // Colour 0 is transparent; any other pixel hides the sprites behind it, even
                // where the background covers it
                if color == 0 || taken[screen_x] {
                    continue;
                }
                taken[screen_x] = true;
                if attributes & 0x80 != 0 && colors[screen_x] != 0 {
                    continue;
                }
                self.frame[row + screen_x] = palette >> (color * 2) & 3;
            }
        }
    }
}

/// The shades of colour numbers 0-3 in a palette register.
#[inline(always)]
fn shades(palette: u8) -> [u8; 4] {
    [
        palette & 3,
        palette >> 2 & 3,
        palette >> 4 & 3,
        palette >> 6,
    ]
}

/// The colour numbers of the 8 pixels in a tile row, from its two bit planes.
#[inline(always)]
fn decode_row(low: u8, high: u8) -> [u8; 8] {
    (SPREAD_BITS[low as usize] | SPREAD_BITS[high as usize] << 1).to_le_bytes()
}

/// Each byte's bits spread out to one byte each, bit 7 (the leftmost pixel) into the lowest.
#[cfg_attr(target_os = "espidf", link_section = ".dram1.ogo_gb")]
static SPREAD_BITS: [u64; 256] = spread_bits();

const fn spread_bits() -> [u64; 256] {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut pixel = 0;
        while pixel < 8 {
            if byte & (0x80 >> pixel) != 0 {
                table[byte] |= 1 << (pixel * 8);
            }
            pixel += 1;
        }
        byte += 1;
    }
    table
}
