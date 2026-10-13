//! The cartridge: ROM, external RAM and the memory bank controller (MBC) that maps them.

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

const ROM_BANK: usize = 0x4000;
const RAM_BANK: usize = 0x2000;
/// The MBC3 clock counts emulated time: the Game Boy runs 4 194 304 cycles per second.
const CYCLES_PER_SECOND: u32 = 4_194_304;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartError {
    /// Shorter than the 32 KiB every cartridge has.
    TooSmall,
    /// A cartridge type (header byte 0x147) this core doesn't emulate.
    Unsupported(u8),
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CartError::TooSmall => write!(f, "ROM is smaller than 32 KiB"),
            CartError::Unsupported(kind) => write!(f, "unsupported cartridge type {kind:#04x}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mbc {
    None,
    Mbc1,
    Mbc3,
    Mbc5,
}

/// The MBC3 real-time clock: seconds, minutes, hours, day low, day high (bit 0: day bit 8,
/// bit 6: halt, bit 7: day carry).
#[derive(Debug, Clone, Default)]
struct Rtc {
    regs: [u8; 5],
    latched: [u8; 5],
    latch_armed: bool,
    cycles: u32,
}

impl Rtc {
    fn tick(&mut self, cycles: u32) {
        if self.regs[4] & 0x40 != 0 {
            return;
        }
        self.cycles += cycles;
        while self.cycles >= CYCLES_PER_SECOND {
            self.cycles -= CYCLES_PER_SECOND;
            self.advance_second();
        }
    }

    fn advance_second(&mut self) {
        let r = &mut self.regs;
        // The counters wrap at their register width, so values set out of range count on to 63/31
        // before wrapping, like the real clock.
        r[0] = (r[0] + 1) & 0x3f;
        if r[0] != 60 {
            return;
        }
        r[0] = 0;
        r[1] = (r[1] + 1) & 0x3f;
        if r[1] != 60 {
            return;
        }
        r[1] = 0;
        r[2] = (r[2] + 1) & 0x1f;
        if r[2] != 24 {
            return;
        }
        r[2] = 0;
        let day = ((r[4] as u16 & 1) << 8 | r[3] as u16) + 1;
        r[3] = day as u8;
        r[4] = (r[4] & 0xfe) | ((day >> 8) as u8 & 1);
        if day > 0x1ff {
            r[4] |= 0x80;
        }
    }
}

pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    mbc: Mbc,
    battery: bool,
    rtc: Option<Rtc>,
    ram_enabled: bool,
    /// MBC1: the 5-bit bank register; MBC3/MBC5: the whole ROM bank number.
    rom_bank: usize,
    /// MBC1: the 2-bit register used for the upper ROM bits or the RAM bank; MBC3: RAM bank or
    /// clock register (0x08-0x0C); MBC5: RAM bank.
    bank2: usize,
    /// MBC1 banking mode 1: `bank2` also switches the 0x0000 area and the RAM.
    mode1: bool,
    /// Byte offsets of the banks mapped at 0x0000, 0x4000 and 0xA000, kept up to date on writes.
    rom0_offset: usize,
    rom1_offset: usize,
    ram_offset: usize,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Result<Self, CartError> {
        if rom.len() < 2 * ROM_BANK {
            return Err(CartError::TooSmall);
        }
        let kind = rom[0x147];
        let (mbc, battery, rtc) = match kind {
            0x00 | 0x08 => (Mbc::None, false, false),
            0x09 => (Mbc::None, true, false),
            0x01 | 0x02 => (Mbc::Mbc1, false, false),
            0x03 => (Mbc::Mbc1, true, false),
            0x11 | 0x12 => (Mbc::Mbc3, false, false),
            0x13 => (Mbc::Mbc3, true, false),
            0x0f | 0x10 => (Mbc::Mbc3, true, true),
            0x19 | 0x1a | 0x1c | 0x1d => (Mbc::Mbc5, false, false),
            0x1b | 0x1e => (Mbc::Mbc5, true, false),
            _ => return Err(CartError::Unsupported(kind)),
        };
        let ram_size = match rom[0x149] {
            1 => 0x800,
            2 => RAM_BANK,
            3 => 4 * RAM_BANK,
            4 => 16 * RAM_BANK,
            5 => 8 * RAM_BANK,
            _ => 0,
        };
        let mut cart = Cartridge {
            rom,
            ram: vec![0; ram_size],
            mbc,
            battery,
            rtc: rtc.then(Rtc::default),
            ram_enabled: false,
            rom_bank: 1,
            bank2: 0,
            mode1: false,
            rom0_offset: 0,
            rom1_offset: ROM_BANK,
            ram_offset: 0,
        };
        cart.update_offsets();
        Ok(cart)
    }

    /// The game's title from the header, up to 16 ASCII characters.
    pub fn title(&self) -> &str {
        let bytes = &self.rom[0x134..0x144];
        let end = bytes
            .iter()
            .position(|&b| b == 0 || !b.is_ascii_graphic() && b != b' ')
            .unwrap_or(bytes.len());
        core::str::from_utf8(&bytes[..end]).unwrap_or("").trim_end()
    }

    /// Whether the cartridge keeps its RAM (and clock) with a battery, i.e. has a save.
    pub fn has_battery(&self) -> bool {
        self.battery
    }

    /// Whether the cartridge has a clock (MBC3 with timer).
    #[inline(always)]
    pub(crate) fn has_clock(&self) -> bool {
        self.rtc.is_some()
    }

    /// The external RAM, e.g. to write a save file.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// The external RAM, e.g. to load a save file into it.
    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }

    #[inline(always)]
    pub(crate) fn read_rom(&self, addr: u16) -> u8 {
        let addr = addr as usize;
        if addr < ROM_BANK {
            self.rom[self.rom0_offset + addr]
        } else {
            self.rom[self.rom1_offset + addr - ROM_BANK]
        }
    }

    pub(crate) fn write_rom(&mut self, addr: u16, value: u8) {
        match (self.mbc, addr >> 13) {
            (Mbc::None, _) => {}
            (_, 0) => self.ram_enabled = value & 0x0f == 0x0a,
            (Mbc::Mbc1, 1) => self.rom_bank = (value & 0x1f) as usize,
            (Mbc::Mbc1, 2) => self.bank2 = (value & 3) as usize,
            (Mbc::Mbc1, _) => self.mode1 = value & 1 != 0,
            (Mbc::Mbc3, 1) => self.rom_bank = (value & 0x7f) as usize,
            (Mbc::Mbc3, 2) => self.bank2 = value as usize,
            (Mbc::Mbc3, _) => {
                if let Some(rtc) = &mut self.rtc {
                    if value == 1 && rtc.latch_armed {
                        rtc.latched = rtc.regs;
                    }
                    rtc.latch_armed = value == 0;
                }
            }
            (Mbc::Mbc5, 1) if addr < 0x3000 => {
                self.rom_bank = (self.rom_bank & 0x100) | value as usize;
            }
            (Mbc::Mbc5, 1) => {
                self.rom_bank = (self.rom_bank & 0xff) | ((value as usize & 1) << 8);
            }
            (Mbc::Mbc5, 2) => self.bank2 = (value & 0x0f) as usize,
            (Mbc::Mbc5, _) => {}
        }
        self.update_offsets();
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    pub(crate) fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled {
            return 0xff;
        }
        if let Some(rtc) = &self.rtc {
            if (0x08..=0x0c).contains(&self.bank2) {
                return rtc.latched[self.bank2 - 0x08];
            }
        }
        if self.ram.is_empty() {
            return 0xff;
        }
        self.ram[(self.ram_offset + (addr as usize & 0x1fff)) % self.ram.len()]
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    pub(crate) fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled {
            return;
        }
        if let Some(rtc) = &mut self.rtc {
            if (0x08..=0x0c).contains(&self.bank2) {
                let i = self.bank2 - 0x08;
                const MASKS: [u8; 5] = [0x3f, 0x3f, 0x1f, 0xff, 0xc1];
                rtc.regs[i] = value & MASKS[i];
                rtc.latched[i] = rtc.regs[i];
                if i == 0 {
                    rtc.cycles = 0;
                }
                return;
            }
        }
        if self.ram.is_empty() {
            return;
        }
        let len = self.ram.len();
        self.ram[(self.ram_offset + (addr as usize & 0x1fff)) % len] = value;
    }

    /// Lets the clock of an MBC3 cartridge run for `cycles`.
    pub(crate) fn tick(&mut self, cycles: u32) {
        if let Some(rtc) = &mut self.rtc {
            rtc.tick(cycles);
        }
    }

    fn update_offsets(&mut self) {
        let banks = self.rom.len() / ROM_BANK;
        let (rom0, rom1, ram) = match self.mbc {
            Mbc::None => (0, 1, 0),
            Mbc::Mbc1 => {
                let low = if self.rom_bank == 0 { 1 } else { self.rom_bank };
                let high = self.bank2 << 5;
                if self.mode1 {
                    (high, high | low, self.bank2)
                } else {
                    (0, high | low, 0)
                }
            }
            Mbc::Mbc3 => (0, self.rom_bank.max(1), self.bank2 & 3),
            Mbc::Mbc5 => (0, self.rom_bank, self.bank2),
        };
        self.rom0_offset = (rom0 % banks) * ROM_BANK;
        self.rom1_offset = (rom1 % banks) * ROM_BANK;
        self.ram_offset = ram * RAM_BANK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom(kind: u8, banks: usize, ram: u8) -> Vec<u8> {
        let mut rom = vec![0; banks * ROM_BANK];
        for bank in 0..banks {
            rom[bank * ROM_BANK] = bank as u8;
            rom[bank * ROM_BANK + 1] = (bank >> 8) as u8;
        }
        rom[0x147] = kind;
        rom[0x149] = ram;
        rom[0x134..0x139].copy_from_slice(b"HELLO");
        rom
    }

    fn bank_at(cart: &Cartridge, addr: u16) -> usize {
        cart.read_rom(addr) as usize | (cart.read_rom(addr + 1) as usize) << 8
    }

    #[test]
    fn header() {
        let cart = Cartridge::new(rom(0x03, 4, 2)).unwrap();
        assert_eq!(cart.title(), "HELLO");
        assert!(cart.has_battery());
        assert_eq!(cart.ram().len(), 0x2000);
        assert_eq!(
            Cartridge::new(rom(0x20, 4, 0)).err(),
            Some(CartError::Unsupported(0x20))
        );
        assert_eq!(
            Cartridge::new(vec![0; 0x4000]).err(),
            Some(CartError::TooSmall)
        );
    }

    #[test]
    fn mbc1_banks() {
        let mut cart = Cartridge::new(rom(0x01, 128, 0)).unwrap();
        assert_eq!(bank_at(&cart, 0x4000), 1);
        cart.write_rom(0x2000, 0); // bank 0 selects 1
        assert_eq!(bank_at(&cart, 0x4000), 1);
        cart.write_rom(0x2000, 0x13);
        cart.write_rom(0x4000, 2);
        assert_eq!(bank_at(&cart, 0x4000), 0x53);
        assert_eq!(bank_at(&cart, 0x0000), 0);
        cart.write_rom(0x6000, 1);
        assert_eq!(bank_at(&cart, 0x0000), 0x40);
    }

    #[test]
    fn mbc5_banks_and_ram() {
        let mut cart = Cartridge::new(rom(0x1b, 512, 3)).unwrap();
        cart.write_rom(0x2000, 0x34);
        cart.write_rom(0x3000, 1);
        assert_eq!(bank_at(&cart, 0x4000), 0x134);
        cart.write_rom(0x2000, 0);
        cart.write_rom(0x3000, 0);
        assert_eq!(bank_at(&cart, 0x4000), 0);

        assert_eq!(cart.read_ram(0xa000), 0xff); // disabled
        cart.write_rom(0x0000, 0x0a);
        cart.write_rom(0x4000, 2);
        cart.write_ram(0xa001, 0x42);
        assert_eq!(cart.ram()[2 * RAM_BANK + 1], 0x42);
    }

    #[test]
    fn mbc3_clock() {
        let mut cart = Cartridge::new(rom(0x10, 4, 3)).unwrap();
        cart.write_rom(0x0000, 0x0a);
        cart.write_rom(0x4000, 0x08);
        cart.write_ram(0xa000, 59);
        cart.write_rom(0x4000, 0x09);
        cart.write_ram(0xa000, 59);
        cart.write_rom(0x4000, 0x0a);
        cart.write_ram(0xa000, 23);
        cart.tick(CYCLES_PER_SECOND);
        // Reads show the latched values until the next latch
        assert_eq!(cart.read_ram(0xa000), 23);
        cart.write_rom(0x6000, 0);
        cart.write_rom(0x6000, 1);
        let regs: Vec<u8> = (0x08..=0x0c)
            .map(|reg| {
                cart.write_rom(0x4000, reg);
                cart.read_ram(0xa000)
            })
            .collect();
        assert_eq!(regs, [0, 0, 0, 1, 0]);
    }
}
