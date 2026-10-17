//! The memory map: cartridge, video and work RAM, OAM, I/O registers, high RAM.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::apu::Apu;
use crate::cart::Cartridge;
use crate::ppu::Ppu;
use crate::timer::Timer;
use crate::{Buttons, INT_JOYPAD, INT_SERIAL, INT_TIMER};

/// Bits of the I/O registers 0xFF00-0xFF7F that read as 1 on the DMG: unused bits, write-only
/// registers and addresses without a register. The registers with their own model (timer, IF,
/// LCD) apply theirs themselves.
const IO_UNUSED_BITS: [u8; 0x80] = [
    0xc0, 0x00, 0x7e, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xe0,
    0x80, 0x3f, 0x00, 0xff, 0xbf, 0xff, 0x3f, 0x00, 0xff, 0xbf, 0x7f, 0xff, 0x9f, 0xff, 0xbf, 0xff,
    0xff, 0x00, 0x00, 0xbf, 0x00, 0x00, 0x70, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
];

/// The fields used for every instruction first (see `GameBoy`).
#[repr(C)]
pub(crate) struct Bus {
    /// Cycles the timer, the LCD and the cartridge clock are behind the CPU. They catch up in
    /// `sync`: when `next_event` is reached, or before the CPU accesses their registers.
    pending: u32,
    /// Cycles from the last sync until the next interrupt or LCD mode change can happen.
    next_event: u32,
    pub ie: u8,
    pub iflag: u8,
    pub wram: Box<[u8; 0x2000]>,
    pub cart: Cartridge,
    pub hram: [u8; 0x7f],
    /// I/O registers without their own model (sound, serial data), as written.
    pub io: [u8; 0x80],
    pub timer: Timer,
    pub ppu: Ppu,
    pub apu: Apu,
    /// P1 bits 4-5: which button group is selected (0 = selected).
    joypad_select: u8,
    buttons: Buttons,
    /// Bytes sent over the serial port, collected when enabled (test ROMs report there).
    pub serial_log: Option<Vec<u8>>,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        Bus {
            cart,
            wram: crate::zeroed(),
            hram: [0; 0x7f],
            io: [0; 0x80],
            ie: 0,
            iflag: 0xe1,
            timer: Timer::new(),
            ppu: Ppu::new(),
            apu: Apu::new(),
            joypad_select: 0x30,
            buttons: Buttons::default(),
            serial_log: None,
            pending: 0,
            next_event: 0,
        }
    }

    /// A read by the CPU: brings the timer, LCD or clock up to date first if it reads them.
    #[inline(always)]
    pub fn read(&mut self, addr: u16) -> u8 {
        match addr >> 12 {
            0x0..=0x7 => self.cart.read_rom(addr),
            0x8 | 0x9 => self.ppu.vram[addr as usize & 0x1fff],
            0xc..=0xe => self.wram[addr as usize & 0x1fff],
            _ if (0xff80..0xffff).contains(&addr) => self.hram[addr as usize - 0xff80],
            _ => self.read_slow(addr),
        }
    }

    /// Cartridge RAM and 0xF000-0xFFFF without high RAM: the areas that may need a sync first.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn read_slow(&mut self, addr: u16) -> u8 {
        if self.needs_sync(addr) {
            self.sync();
        }
        self.peek(addr)
    }

    /// A read without side effects, of the state as of the last sync.
    #[inline(always)]
    pub fn peek(&self, addr: u16) -> u8 {
        match addr >> 12 {
            0x0..=0x7 => self.cart.read_rom(addr),
            0x8 | 0x9 => self.ppu.vram[addr as usize & 0x1fff],
            0xa | 0xb => self.cart.read_ram(addr),
            0xc..=0xe => self.wram[addr as usize & 0x1fff],
            _ => self.read_high(addr),
        }
    }

    #[inline(always)]
    pub fn write(&mut self, addr: u16, value: u8) {
        match addr >> 12 {
            0x8 | 0x9 => self.ppu.vram[addr as usize & 0x1fff] = value,
            0xc..=0xe => self.wram[addr as usize & 0x1fff] = value,
            _ if (0xff80..0xffff).contains(&addr) => self.hram[addr as usize - 0xff80] = value,
            _ => self.write_slow(addr, value),
        }
    }

    /// The cartridge (MBC registers and RAM) and 0xF000-0xFFFF without high RAM: the areas that
    /// may need a sync first.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn write_slow(&mut self, addr: u16, value: u8) {
        let synced = self.needs_sync(addr);
        if synced {
            self.sync();
        }
        match addr >> 12 {
            0x0..=0x7 => self.cart.write_rom(addr, value),
            0x8 | 0x9 => self.ppu.vram[addr as usize & 0x1fff] = value,
            0xa | 0xb => self.cart.write_ram(addr, value),
            0xc..=0xe => self.wram[addr as usize & 0x1fff] = value,
            _ => self.write_high(addr, value),
        }
        if synced {
            // The write may have moved the next event (TAC, TIMA, LCDC, ...)
            self.next_event = self.cycles_to_event();
        }
    }

    /// Whether accessing `addr` touches state that runs behind: timer and IF, sound, the LCD
    /// timing registers, or the cartridge clock (whose latch is written in the ROM area).
    #[inline(always)]
    fn needs_sync(&self, addr: u16) -> bool {
        match addr {
            0xff04..=0xff45 => true,
            0x0000..=0x7fff | 0xa000..=0xbfff => self.cart.has_clock(),
            _ => false,
        }
    }

    /// 0xF000-0xFFFF: echo RAM, OAM, I/O, high RAM and IE.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn read_high(&self, addr: u16) -> u8 {
        match addr {
            ..=0xfdff => self.wram[addr as usize & 0x1fff],
            0xfe00..=0xfe9f => self.ppu.oam[addr as usize - 0xfe00],
            0xfea0..=0xfeff => 0xff,
            0xff00 => self.read_joypad(),
            0xff04 => self.timer.div(),
            0xff05 => self.timer.tima,
            0xff06 => self.timer.tma,
            0xff07 => self.timer.tac,
            0xff0f => self.iflag | 0xe0,
            0xff10..=0xff3f => self.apu.read(addr),
            0xff40 => self.ppu.lcdc,
            0xff41 => self.ppu.read_stat(),
            0xff42 => self.ppu.scy,
            0xff43 => self.ppu.scx,
            0xff44 => self.ppu.ly,
            0xff45 => self.ppu.lyc,
            0xff47 => self.ppu.bgp,
            0xff48 => self.ppu.obp0,
            0xff49 => self.ppu.obp1,
            0xff4a => self.ppu.wy,
            0xff4b => self.ppu.wx,
            0xff01..=0xff7f => {
                self.io[addr as usize - 0xff00] | IO_UNUSED_BITS[addr as usize - 0xff00]
            }
            0xff80..=0xfffe => self.hram[addr as usize - 0xff80],
            0xffff => self.ie,
        }
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn write_high(&mut self, addr: u16, value: u8) {
        match addr {
            ..=0xfdff => self.wram[addr as usize & 0x1fff] = value,
            0xfe00..=0xfe9f => self.ppu.oam[addr as usize - 0xfe00] = value,
            0xfea0..=0xfeff => {}
            0xff00 => self.joypad_select = value & 0x30,
            0xff02 => self.write_serial_control(value),
            0xff04 => {
                if self.timer.write_div() {
                    self.iflag |= INT_TIMER;
                }
            }
            0xff05 => self.timer.tima = value,
            0xff06 => self.timer.tma = value,
            0xff07 => {
                if self.timer.write_tac(value) {
                    self.iflag |= INT_TIMER;
                }
            }
            0xff0f => self.iflag = value & 0x1f,
            0xff10..=0xff3f => self.apu.write(addr, value),
            0xff40 => self.iflag |= self.ppu.write_lcdc(value),
            0xff41 => self.iflag |= self.ppu.write_stat(value),
            0xff42 => self.ppu.scy = value,
            0xff43 => self.ppu.scx = value,
            0xff44 => {} // LY is read-only
            0xff45 => self.iflag |= self.ppu.write_lyc(value),
            0xff46 => self.oam_dma(value),
            0xff47 => self.ppu.bgp = value,
            0xff48 => self.ppu.obp0 = value,
            0xff49 => self.ppu.obp1 = value,
            0xff4a => self.ppu.wy = value,
            0xff4b => self.ppu.wx = value,
            0xff01..=0xff7f => self.io[addr as usize - 0xff00] = value,
            0xff80..=0xfffe => self.hram[addr as usize - 0xff80] = value,
            0xffff => self.ie = value,
        }
    }

    /// Lets `cycles` pass for the timer, the LCD and the cartridge clock. They only catch up
    /// once something can happen.
    #[inline(always)]
    pub fn advance(&mut self, cycles: u32) {
        self.pending += cycles;
        if self.pending >= self.next_event {
            self.sync();
        }
    }

    /// Cycles from now until the next event.
    pub fn cycles_until_event(&self) -> u32 {
        self.next_event.saturating_sub(self.pending)
    }

    /// Brings the timer, the LCD and the cartridge clock up to date.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn sync(&mut self) {
        let cycles = self.pending;
        self.pending = 0;
        if self.timer.tick(cycles) {
            self.iflag |= INT_TIMER;
        }
        self.iflag |= self.ppu.tick(cycles);
        self.cart.tick(cycles);
        self.apu.tick(cycles);
        self.next_event = self.cycles_to_event();
    }

    /// Cycles from the last sync until the next event.
    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    fn cycles_to_event(&self) -> u32 {
        self.timer
            .cycles_to_overflow()
            .min(self.ppu.cycles_to_mode_change())
            .min(self.apu.cycles_to_event())
    }

    fn read_joypad(&self) -> u8 {
        let mut low = 0x0f;
        if self.joypad_select & 0x10 == 0 {
            low &= !self.buttons.0 & 0x0f;
        }
        if self.joypad_select & 0x20 == 0 {
            low &= !(self.buttons.0 >> 4) & 0x0f;
        }
        0xc0 | self.joypad_select | low
    }

    pub fn set_buttons(&mut self, buttons: Buttons) {
        if buttons.0 & !self.buttons.0 != 0 {
            self.iflag |= INT_JOYPAD;
        }
        self.buttons = buttons;
    }

    /// A transfer with the internal clock and nothing connected: the byte goes out at once and
    /// 0xFF comes back.
    fn write_serial_control(&mut self, value: u8) {
        if value & 0x81 == 0x81 {
            if let Some(log) = &mut self.serial_log {
                log.push(self.io[0x01]);
            }
            self.io[0x01] = 0xff;
            self.io[0x02] = value & 0x7f;
            self.iflag |= INT_SERIAL;
        } else {
            self.io[0x02] = value;
        }
    }

    /// Copies 160 bytes from `page << 8` into OAM, at once.
    fn oam_dma(&mut self, page: u8) {
        self.io[0x46] = page;
        let source = (page as u16) << 8;
        for i in 0..0xa0 {
            self.ppu.oam[i as usize] = self.peek(source + i);
        }
    }
}
