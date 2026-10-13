//! The SM83 CPU: registers, the instruction set and interrupt dispatch.
//!
//! Timing is counted per instruction: every memory access adds one machine cycle (4 clock
//! cycles), internal delays add theirs explicitly, and the instruction's cycles are passed on to
//! the bus afterwards (`Bus::advance`).

use crate::ppu::LINE_CYCLES;
use crate::GameBoy;

const Z: u8 = 0x80;
const N: u8 = 0x40;
const H: u8 = 0x20;
const C: u8 = 0x10;

/// The CPU registers, in their state right after the boot ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl Default for Registers {
    fn default() -> Self {
        Registers {
            a: 0x01,
            f: 0xb0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,
            sp: 0xfffe,
            pc: 0x0100,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Cpu {
    pub r: Registers,
    /// Interrupt master enable.
    ime: bool,
    /// EI enables interrupts only after the following instruction.
    ime_pending: bool,
    halted: bool,
    /// HALT with interrupts disabled but one already pending: the next opcode is read twice.
    halt_bug: bool,
    /// An illegal opcode hangs the CPU until reset.
    locked: bool,
    /// Clock cycles of the instruction being executed.
    cycles: u32,
}

impl GameBoy {
    /// Runs one instruction (or interrupt dispatch, or one machine cycle of HALT) and lets the
    /// rest of the machine catch up. Returns the clock cycles it took.
    pub fn step(&mut self) -> u32 {
        self.cpu.cycles = 0;
        let pending = self.bus.ie & self.bus.iflag & 0x1f;
        if self.cpu.halted || self.cpu.locked {
            if pending == 0 || self.cpu.locked {
                // Nothing changes before the next event: skip there in whole machine cycles, at
                // most a line at a time
                let cycles = (self.bus.cycles_until_event().clamp(4, LINE_CYCLES) + 3) & !3;
                self.bus.advance(cycles);
                return cycles;
            }
            self.cpu.halted = false;
        }
        if self.cpu.ime && pending != 0 {
            self.dispatch_interrupt(pending);
        } else {
            let enable_ime = self.cpu.ime_pending;
            let opcode = self.fetch_opcode();
            self.execute(opcode);
            // DI right after EI clears `ime_pending` again
            if enable_ime && self.cpu.ime_pending {
                self.cpu.ime = true;
                self.cpu.ime_pending = false;
            }
        }
        let cycles = self.cpu.cycles;
        self.bus.advance(cycles);
        cycles
    }

    /// The CPU registers, e.g. for tests and debugging.
    pub fn registers(&self) -> &Registers {
        &self.cpu.r
    }

    fn dispatch_interrupt(&mut self, pending: u8) {
        self.cpu.ime = false;
        let bit = pending.trailing_zeros();
        self.bus.iflag &= !(1 << bit);
        self.cpu.cycles += 8;
        let pc = self.cpu.r.pc;
        self.push(pc);
        self.cpu.r.pc = 0x40 + 8 * bit as u16;
        self.cpu.cycles += 4;
    }

    // Memory access, one machine cycle each

    #[inline(always)]
    fn read(&mut self, addr: u16) -> u8 {
        self.cpu.cycles += 4;
        self.bus.read(addr)
    }

    #[inline(always)]
    fn write(&mut self, addr: u16, value: u8) {
        self.cpu.cycles += 4;
        self.bus.write(addr, value);
    }

    #[inline(always)]
    fn fetch_opcode(&mut self) -> u8 {
        let opcode = self.read(self.cpu.r.pc);
        if self.cpu.halt_bug {
            self.cpu.halt_bug = false;
        } else {
            self.cpu.r.pc = self.cpu.r.pc.wrapping_add(1);
        }
        opcode
    }

    #[inline(always)]
    fn fetch8(&mut self) -> u8 {
        let value = self.read(self.cpu.r.pc);
        self.cpu.r.pc = self.cpu.r.pc.wrapping_add(1);
        value
    }

    #[inline(always)]
    fn fetch16(&mut self) -> u16 {
        let low = self.fetch8() as u16;
        let high = self.fetch8() as u16;
        high << 8 | low
    }

    fn push(&mut self, value: u16) {
        self.cpu.r.sp = self.cpu.r.sp.wrapping_sub(1);
        self.write(self.cpu.r.sp, (value >> 8) as u8);
        self.cpu.r.sp = self.cpu.r.sp.wrapping_sub(1);
        self.write(self.cpu.r.sp, value as u8);
    }

    fn pop(&mut self) -> u16 {
        let low = self.read(self.cpu.r.sp) as u16;
        self.cpu.r.sp = self.cpu.r.sp.wrapping_add(1);
        let high = self.read(self.cpu.r.sp) as u16;
        self.cpu.r.sp = self.cpu.r.sp.wrapping_add(1);
        high << 8 | low
    }

    // Registers

    #[inline(always)]
    fn hl(&self) -> u16 {
        (self.cpu.r.h as u16) << 8 | self.cpu.r.l as u16
    }

    #[inline(always)]
    fn set_hl(&mut self, value: u16) {
        self.cpu.r.h = (value >> 8) as u8;
        self.cpu.r.l = value as u8;
    }

    /// BC, DE, HL, SP by their 2-bit index in the opcode.
    #[inline(always)]
    fn rr(&self, index: u8) -> u16 {
        let r = &self.cpu.r;
        match index {
            0 => (r.b as u16) << 8 | r.c as u16,
            1 => (r.d as u16) << 8 | r.e as u16,
            2 => self.hl(),
            _ => r.sp,
        }
    }

    #[inline(always)]
    fn set_rr(&mut self, index: u8, value: u16) {
        let (high, low) = ((value >> 8) as u8, value as u8);
        let r = &mut self.cpu.r;
        match index {
            0 => (r.b, r.c) = (high, low),
            1 => (r.d, r.e) = (high, low),
            2 => (r.h, r.l) = (high, low),
            _ => r.sp = value,
        }
    }

    /// B, C, D, E, H, L, (HL), A by their 3-bit index in the opcode.
    #[inline(always)]
    fn r8(&mut self, index: u8) -> u8 {
        let r = &self.cpu.r;
        match index {
            0 => r.b,
            1 => r.c,
            2 => r.d,
            3 => r.e,
            4 => r.h,
            5 => r.l,
            6 => self.read(self.hl()),
            _ => r.a,
        }
    }

    #[inline(always)]
    fn set_r8(&mut self, index: u8, value: u8) {
        let r = &mut self.cpu.r;
        match index {
            0 => r.b = value,
            1 => r.c = value,
            2 => r.d = value,
            3 => r.e = value,
            4 => r.h = value,
            5 => r.l = value,
            6 => self.write(self.hl(), value),
            _ => r.a = value,
        }
    }

    /// NZ, Z, NC, C by their 2-bit index in the opcode.
    #[inline(always)]
    fn condition(&self, index: u8) -> bool {
        let f = self.cpu.r.f;
        match index {
            0 => f & Z == 0,
            1 => f & Z != 0,
            2 => f & C == 0,
            _ => f & C != 0,
        }
    }

    #[inline(always)]
    fn carry(&self) -> u8 {
        (self.cpu.r.f & C != 0) as u8
    }

    // Arithmetic

    /// ADD, ADC, SUB, SBC, AND, XOR, OR, CP by their 3-bit index in the opcode.
    fn alu(&mut self, op: u8, value: u8) {
        let a = self.cpu.r.a;
        let (result, flags) = match op {
            0 | 1 => {
                let carry = if op == 1 { self.carry() } else { 0 };
                let sum = a as u16 + value as u16 + carry as u16;
                let half = (a & 0x0f) + (value & 0x0f) + carry > 0x0f;
                (sum as u8, flag(H, half) | flag(C, sum > 0xff))
            }
            2 | 3 | 7 => {
                let carry = if op == 3 { self.carry() } else { 0 };
                let diff = (a as u16)
                    .wrapping_sub(value as u16)
                    .wrapping_sub(carry as u16);
                let half = (a & 0x0f) < (value & 0x0f) + carry;
                (diff as u8, N | flag(H, half) | flag(C, diff > 0xff))
            }
            4 => (a & value, H),
            5 => (a ^ value, 0),
            _ => (a | value, 0),
        };
        self.cpu.r.f = flag(Z, result == 0) | flags;
        if op != 7 {
            self.cpu.r.a = result;
        }
    }

    fn inc8(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.cpu.r.f = (self.cpu.r.f & C) | flag(Z, result == 0) | flag(H, value & 0x0f == 0x0f);
        result
    }

    fn dec8(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.cpu.r.f = (self.cpu.r.f & C) | N | flag(Z, result == 0) | flag(H, value & 0x0f == 0);
        result
    }

    fn add_hl(&mut self, value: u16) {
        let hl = self.hl();
        let (result, carry) = hl.overflowing_add(value);
        let half = (hl & 0x0fff) + (value & 0x0fff) > 0x0fff;
        self.cpu.r.f = (self.cpu.r.f & Z) | flag(H, half) | flag(C, carry);
        self.set_hl(result);
        self.cpu.cycles += 4;
    }

    /// SP plus a signed byte, for ADD SP,e and LD HL,SP+e. The flags come from the low byte.
    fn sp_plus_offset(&mut self) -> u16 {
        let offset = self.fetch8();
        let sp = self.cpu.r.sp;
        let half = (sp & 0x0f) + (offset as u16 & 0x0f) > 0x0f;
        let carry = (sp & 0xff) + offset as u16 > 0xff;
        self.cpu.r.f = flag(H, half) | flag(C, carry);
        sp.wrapping_add(offset as i8 as u16)
    }

    fn daa(&mut self) {
        let f = self.cpu.r.f;
        let mut a = self.cpu.r.a;
        let mut carry = f & C != 0;
        if f & N == 0 {
            if carry || a > 0x99 {
                a = a.wrapping_add(0x60);
                carry = true;
            }
            if f & H != 0 || a & 0x0f > 0x09 {
                a = a.wrapping_add(0x06);
            }
        } else {
            if carry {
                a = a.wrapping_sub(0x60);
            }
            if f & H != 0 {
                a = a.wrapping_sub(0x06);
            }
        }
        self.cpu.r.a = a;
        self.cpu.r.f = flag(Z, a == 0) | (f & N) | flag(C, carry);
    }

    /// RLC, RRC, RL, RR, SLA, SRA, SWAP, SRL by their 3-bit index in a CB opcode. Sets Z and C.
    fn shift(&mut self, op: u8, value: u8) -> u8 {
        let carry_in = self.carry();
        let (result, carry) = match op {
            0 => (value.rotate_left(1), value & 0x80 != 0),
            1 => (value.rotate_right(1), value & 1 != 0),
            2 => (value << 1 | carry_in, value & 0x80 != 0),
            3 => (value >> 1 | carry_in << 7, value & 1 != 0),
            4 => (value << 1, value & 0x80 != 0),
            5 => (value >> 1 | (value & 0x80), value & 1 != 0),
            6 => (value.rotate_left(4), false),
            _ => (value >> 1, value & 1 != 0),
        };
        self.cpu.r.f = flag(Z, result == 0) | flag(C, carry);
        result
    }

    // Control flow

    fn jump(&mut self, target: u16) {
        self.cpu.r.pc = target;
        self.cpu.cycles += 4;
    }

    fn call(&mut self, target: u16) {
        self.cpu.cycles += 4;
        let pc = self.cpu.r.pc;
        self.push(pc);
        self.cpu.r.pc = target;
    }

    fn ret(&mut self) {
        let target = self.pop();
        self.jump(target);
    }

    fn halt(&mut self) {
        if !self.cpu.ime && self.bus.ie & self.bus.iflag & 0x1f != 0 {
            self.cpu.halt_bug = true;
        } else {
            self.cpu.halted = true;
        }
    }

    // Instructions

    fn execute(&mut self, opcode: u8) {
        match opcode {
            0x00 => {}
            // LD rr,d16
            0x01 | 0x11 | 0x21 | 0x31 => {
                let value = self.fetch16();
                self.set_rr(opcode >> 4, value);
            }
            // LD (BC),A / LD (DE),A
            0x02 | 0x12 => self.write(self.rr(opcode >> 4), self.cpu.r.a),
            // LD (HL+),A / LD (HL-),A
            0x22 | 0x32 => {
                let hl = self.hl();
                self.write(hl, self.cpu.r.a);
                self.set_hl(if opcode == 0x22 {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                });
            }
            // LD A,(BC) / LD A,(DE)
            0x0a | 0x1a => self.cpu.r.a = self.read(self.rr(opcode >> 4)),
            // LD A,(HL+) / LD A,(HL-)
            0x2a | 0x3a => {
                let hl = self.hl();
                self.cpu.r.a = self.read(hl);
                self.set_hl(if opcode == 0x2a {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                });
            }
            // INC rr / DEC rr
            0x03 | 0x13 | 0x23 | 0x33 => {
                let index = opcode >> 4;
                self.set_rr(index, self.rr(index).wrapping_add(1));
                self.cpu.cycles += 4;
            }
            0x0b | 0x1b | 0x2b | 0x3b => {
                let index = opcode >> 4;
                self.set_rr(index, self.rr(index).wrapping_sub(1));
                self.cpu.cycles += 4;
            }
            // INC r / DEC r / LD r,d8
            0x04 | 0x0c | 0x14 | 0x1c | 0x24 | 0x2c | 0x34 | 0x3c => {
                let index = opcode >> 3 & 7;
                let value = self.r8(index);
                let result = self.inc8(value);
                self.set_r8(index, result);
            }
            0x05 | 0x0d | 0x15 | 0x1d | 0x25 | 0x2d | 0x35 | 0x3d => {
                let index = opcode >> 3 & 7;
                let value = self.r8(index);
                let result = self.dec8(value);
                self.set_r8(index, result);
            }
            0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x36 | 0x3e => {
                let value = self.fetch8();
                self.set_r8(opcode >> 3 & 7, value);
            }
            // RLCA, RRCA, RLA, RRA: like the CB shifts on A, but Z is always cleared
            0x07 | 0x0f | 0x17 | 0x1f => {
                self.cpu.r.a = self.shift(opcode >> 3, self.cpu.r.a);
                self.cpu.r.f &= C;
            }
            0x27 => self.daa(),
            // CPL, SCF, CCF
            0x2f => {
                self.cpu.r.a = !self.cpu.r.a;
                self.cpu.r.f |= N | H;
            }
            0x37 => self.cpu.r.f = (self.cpu.r.f & Z) | C,
            0x3f => self.cpu.r.f = (self.cpu.r.f & (Z | C)) ^ C,
            // LD (a16),SP
            0x08 => {
                let addr = self.fetch16();
                let sp = self.cpu.r.sp;
                self.write(addr, sp as u8);
                self.write(addr.wrapping_add(1), (sp >> 8) as u8);
            }
            // ADD HL,rr
            0x09 | 0x19 | 0x29 | 0x39 => self.add_hl(self.rr(opcode >> 4)),
            // STOP: no speed switch or low-power mode here, it skips its operand
            0x10 => {
                self.fetch8();
            }
            // JR e / JR cc,e
            0x18 => {
                let offset = self.fetch8() as i8;
                self.jump(self.cpu.r.pc.wrapping_add(offset as u16));
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let offset = self.fetch8() as i8;
                if self.condition(opcode >> 3 & 3) {
                    self.jump(self.cpu.r.pc.wrapping_add(offset as u16));
                }
            }
            0x76 => self.halt(),
            // LD r,r
            0x40..=0x7f => {
                let value = self.r8(opcode & 7);
                self.set_r8(opcode >> 3 & 7, value);
            }
            // ALU A,r
            0x80..=0xbf => {
                let value = self.r8(opcode & 7);
                self.alu(opcode >> 3 & 7, value);
            }
            // RET cc / RET / RETI
            0xc0 | 0xc8 | 0xd0 | 0xd8 => {
                self.cpu.cycles += 4;
                if self.condition(opcode >> 3 & 3) {
                    self.ret();
                }
            }
            0xc9 => self.ret(),
            0xd9 => {
                self.ret();
                self.cpu.ime = true;
            }
            // POP / PUSH: BC, DE, HL, AF
            0xc1 | 0xd1 | 0xe1 | 0xf1 => {
                let value = self.pop();
                match opcode {
                    0xf1 => {
                        self.cpu.r.a = (value >> 8) as u8;
                        self.cpu.r.f = value as u8 & 0xf0;
                    }
                    _ => self.set_rr(opcode >> 4 & 3, value),
                }
            }
            0xc5 | 0xd5 | 0xe5 | 0xf5 => {
                let value = match opcode {
                    0xf5 => (self.cpu.r.a as u16) << 8 | self.cpu.r.f as u16,
                    _ => self.rr(opcode >> 4 & 3),
                };
                self.cpu.cycles += 4;
                self.push(value);
            }
            // JP a16 / JP cc,a16 / JP HL
            0xc3 => {
                let target = self.fetch16();
                self.jump(target);
            }
            0xc2 | 0xca | 0xd2 | 0xda => {
                let target = self.fetch16();
                if self.condition(opcode >> 3 & 3) {
                    self.jump(target);
                }
            }
            0xe9 => self.cpu.r.pc = self.hl(),
            // CALL a16 / CALL cc,a16
            0xcd => {
                let target = self.fetch16();
                self.call(target);
            }
            0xc4 | 0xcc | 0xd4 | 0xdc => {
                let target = self.fetch16();
                if self.condition(opcode >> 3 & 3) {
                    self.call(target);
                }
            }
            // ALU A,d8
            0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => {
                let value = self.fetch8();
                self.alu(opcode >> 3 & 7, value);
            }
            // RST
            0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => {
                self.call((opcode & 0x38) as u16);
            }
            0xcb => self.execute_cb(),
            // LDH (a8),A / LDH A,(a8) / LD (C),A / LD A,(C)
            0xe0 => {
                let addr = 0xff00 | self.fetch8() as u16;
                self.write(addr, self.cpu.r.a);
            }
            0xf0 => {
                let addr = 0xff00 | self.fetch8() as u16;
                self.cpu.r.a = self.read(addr);
            }
            0xe2 => self.write(0xff00 | self.cpu.r.c as u16, self.cpu.r.a),
            0xf2 => self.cpu.r.a = self.read(0xff00 | self.cpu.r.c as u16),
            // LD (a16),A / LD A,(a16)
            0xea => {
                let addr = self.fetch16();
                self.write(addr, self.cpu.r.a);
            }
            0xfa => {
                let addr = self.fetch16();
                self.cpu.r.a = self.read(addr);
            }
            // ADD SP,e / LD HL,SP+e / LD SP,HL
            0xe8 => {
                self.cpu.r.sp = self.sp_plus_offset();
                self.cpu.cycles += 8;
            }
            0xf8 => {
                let value = self.sp_plus_offset();
                self.set_hl(value);
                self.cpu.cycles += 4;
            }
            0xf9 => {
                self.cpu.r.sp = self.hl();
                self.cpu.cycles += 4;
            }
            // DI / EI
            0xf3 => {
                self.cpu.ime = false;
                self.cpu.ime_pending = false;
            }
            0xfb => self.cpu.ime_pending = true,
            // 0xD3, 0xDB, 0xDD, 0xE3, 0xE4, 0xEB, 0xEC, 0xED, 0xF4, 0xFC, 0xFD
            _ => self.cpu.locked = true,
        }
    }

    fn execute_cb(&mut self) {
        let opcode = self.fetch8();
        let index = opcode & 7;
        let bit = opcode >> 3 & 7;
        let value = self.r8(index);
        match opcode >> 6 {
            0 => {
                let result = self.shift(bit, value);
                self.set_r8(index, result);
            }
            // BIT: Z from the bit, H set, C kept
            1 => self.cpu.r.f = (self.cpu.r.f & C) | H | flag(Z, value & (1 << bit) == 0),
            2 => self.set_r8(index, value & !(1 << bit)),
            _ => self.set_r8(index, value | 1 << bit),
        }
    }
}

#[inline(always)]
fn flag(mask: u8, set: bool) -> u8 {
    if set {
        mask
    } else {
        0
    }
}
