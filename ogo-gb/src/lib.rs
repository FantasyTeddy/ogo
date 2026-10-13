//! A Game Boy (DMG) emulator core.
//!
//! `no_std` with `alloc`, so it builds for the ESP32 and for the PC alike; its tests run on the PC
//! (`cargo xtask test`), including the public test ROMs when they are present (see
//! tests/test_roms.rs). It starts in the state the boot ROM leaves behind, without running one.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod bus;
mod cart;
mod cpu;
mod ppu;
mod timer;

use alloc::vec::Vec;

use bus::Bus;
pub use cart::{CartError, Cartridge};
use cpu::Cpu;
pub use cpu::Registers;

/// Clock cycles per second.
pub const CLOCK_HZ: u32 = 4_194_304;
/// Clock cycles per frame: 154 lines of 456 cycles, about 59.7 frames per second.
pub const CYCLES_PER_FRAME: u32 = ppu::LINE_CYCLES * ppu::LINES as u32;

// Interrupt bits in IE and IF
const INT_VBLANK: u8 = 0x01;
const INT_STAT: u8 = 0x02;
const INT_TIMER: u8 = 0x04;
const INT_SERIAL: u8 = 0x08;
const INT_JOYPAD: u8 = 0x10;

/// The pressed buttons, one bit each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buttons(pub u8);

impl Buttons {
    pub const RIGHT: u8 = 0x01;
    pub const LEFT: u8 = 0x02;
    pub const UP: u8 = 0x04;
    pub const DOWN: u8 = 0x08;
    pub const A: u8 = 0x10;
    pub const B: u8 = 0x20;
    pub const SELECT: u8 = 0x40;
    pub const START: u8 = 0x80;
}

pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
    /// Cycles run past the end of the last frame.
    frame_cycles: u32,
}

impl GameBoy {
    pub fn new(cart: Cartridge) -> Self {
        GameBoy {
            cpu: Cpu::default(),
            bus: Bus::new(cart),
            frame_cycles: 0,
        }
    }

    /// Runs for one frame's worth of cycles.
    pub fn run_frame(&mut self) {
        while self.frame_cycles < CYCLES_PER_FRAME {
            self.frame_cycles += self.step();
        }
        self.frame_cycles -= CYCLES_PER_FRAME;
    }

    pub fn set_buttons(&mut self, buttons: Buttons) {
        self.bus.set_buttons(buttons);
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.bus.cart
    }

    pub fn cartridge_mut(&mut self) -> &mut Cartridge {
        &mut self.bus.cart
    }

    /// Starts collecting the bytes sent over the serial port.
    pub fn capture_serial(&mut self) {
        self.bus.serial_log.get_or_insert_with(Vec::new);
    }

    /// The bytes sent over the serial port since `capture_serial`.
    pub fn serial_output(&self) -> &[u8] {
        self.bus.serial_log.as_deref().unwrap_or(&[])
    }

    /// Reads memory as the CPU would, without side effects. Timer and LCD registers may be a few
    /// cycles behind.
    pub fn peek(&self, addr: u16) -> u8 {
        self.bus.peek(addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// A cartridge without MBC whose code at 0x100 is `program`, followed by `JR -2`.
    fn run(program: &[u8], steps: usize) -> GameBoy {
        let mut rom = vec![0; 0x8000];
        rom[0x100..0x100 + program.len()].copy_from_slice(program);
        rom[0x100 + program.len()..][..2].copy_from_slice(&[0x18, 0xfe]);
        let mut gb = GameBoy::new(Cartridge::new(rom).unwrap());
        for _ in 0..steps {
            gb.step();
        }
        gb
    }

    #[test]
    fn daa_after_add_and_sub() {
        // LD A,0x45; ADD A,0x38; DAA
        let gb = run(&[0x3e, 0x45, 0xc6, 0x38, 0x27], 3);
        assert_eq!(gb.registers().a, 0x83);
        // LD A,0x10; SUB 0x01; DAA
        let gb = run(&[0x3e, 0x10, 0xd6, 0x01, 0x27], 3);
        assert_eq!(gb.registers().a, 0x09);
        assert_eq!(gb.registers().f, 0x40);
    }

    #[test]
    fn cycles_per_instruction() {
        // NOP; LD BC,d16; CALL a16 (to 0x0108, a RET); RET; PUSH BC
        let mut rom = vec![0; 0x8000];
        rom[0x100..0x109].copy_from_slice(&[0x00, 0x01, 0x34, 0x12, 0xcd, 0x08, 0x01, 0xc5, 0xc9]);
        let mut gb = GameBoy::new(Cartridge::new(rom).unwrap());
        let cycles: Vec<u32> = (0..5).map(|_| gb.step()).collect();
        assert_eq!(cycles, [4, 12, 24, 16, 16]);
    }

    #[test]
    fn timer_interrupt_wakes_halt() {
        // LD A,0x05; LDH (TAC),A (enabled, every 16 cycles); LD A,0x04; LDH (IE),A; EI; HALT
        let mut gb = run(
            &[0x3e, 0x05, 0xe0, 0x07, 0x3e, 0x04, 0xe0, 0xff, 0xfb, 0x76],
            6,
        );
        for _ in 0..2000 {
            gb.step();
            if gb.registers().pc == 0x51 {
                return;
            }
        }
        panic!("no timer interrupt, PC {:#06x}", gb.registers().pc);
    }

    #[test]
    fn joypad() {
        let mut gb = run(&[], 0);
        gb.set_buttons(Buttons(Buttons::START | Buttons::LEFT));
        gb.bus.write(0xff00, 0x20); // directions
        assert_eq!(gb.peek(0xff00), 0xed);
        gb.bus.write(0xff00, 0x10); // buttons
        assert_eq!(gb.peek(0xff00), 0xd7);
    }
}
