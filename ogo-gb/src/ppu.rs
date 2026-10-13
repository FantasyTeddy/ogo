//! The LCD controller's registers and timing: lines, modes, LY and the VBlank and STAT interrupts.

use crate::{INT_STAT, INT_VBLANK};

pub const LINE_CYCLES: u32 = 456;
pub const LINES: u8 = 154;
pub const VISIBLE_LINES: u8 = 144;
const MODE2_END: u32 = 80;
const MODE3_END: u32 = 252;

#[derive(Debug, Clone)]
pub(crate) struct Ppu {
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
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
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
        }
    }

    fn enabled(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

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
            self.ly = 0;
            self.dot = 0;
            self.stat_line = false;
        }
        self.update_stat_line()
    }

    /// Cycles until the next mode change (a new line counts as one); `u32::MAX` while the LCD is
    /// off.
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
    pub fn tick(&mut self, mut cycles: u32) -> u8 {
        if !self.enabled() {
            return 0;
        }
        let mut irq = 0;
        while cycles > 0 {
            // Step to the next mode change at most, so no STAT edge is skipped
            let step = self.cycles_to_mode_change().min(cycles);
            self.dot += step;
            cycles -= step;
            if self.dot == LINE_CYCLES {
                self.dot = 0;
                self.ly = (self.ly + 1) % LINES;
                if self.ly == VISIBLE_LINES {
                    irq |= INT_VBLANK;
                }
            }
            irq |= self.update_stat_line();
        }
        irq
    }

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
}
