//! DIV, TIMA, TMA and TAC.

/// The timer counts on a 16-bit counter whose upper byte is DIV. TIMA steps whenever the counter
/// bit selected by TAC falls from 1 to 0.
#[derive(Debug, Clone)]
pub(crate) struct Timer {
    pub counter: u16,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
}

impl Timer {
    pub fn new() -> Self {
        // The counter's value right after the boot ROM
        Timer {
            counter: 0xabcc,
            tima: 0,
            tma: 0,
            tac: 0xf8,
        }
    }

    /// The bit position above the selected counter bit: TIMA steps every `1 << shift` cycles.
    fn shift(tac: u8) -> u32 {
        match tac & 3 {
            0 => 10,
            1 => 4,
            2 => 6,
            _ => 8,
        }
    }

    fn selected_bit_set(&self, tac: u8) -> bool {
        tac & 4 != 0 && self.counter & (1 << (Self::shift(tac) - 1)) != 0
    }

    /// Advances by `cycles`; returns whether TIMA overflowed (the timer interrupt).
    pub fn tick(&mut self, cycles: u32) -> bool {
        let old = self.counter as u32;
        let new = old + cycles;
        self.counter = new as u16;
        if self.tac & 4 == 0 {
            return false;
        }
        let shift = Self::shift(self.tac);
        let steps = (new >> shift) - (old >> shift);
        let mut overflow = false;
        for _ in 0..steps {
            overflow |= self.step_tima();
        }
        overflow
    }

    fn step_tima(&mut self) -> bool {
        let (tima, overflow) = self.tima.overflowing_add(1);
        self.tima = if overflow { self.tma } else { tima };
        overflow
    }

    /// Cycles until TIMA overflows; `u32::MAX` while the timer is stopped.
    pub fn cycles_to_overflow(&self) -> u32 {
        if self.tac & 4 == 0 {
            return u32::MAX;
        }
        let period = 1u32 << Self::shift(self.tac);
        let to_next_step = period - (self.counter as u32 & (period - 1));
        to_next_step + (0xff - self.tima as u32) * period
    }

    pub fn div(&self) -> u8 {
        (self.counter >> 8) as u8
    }

    /// Writing DIV clears the counter, which is a falling edge if the selected bit was set.
    pub fn write_div(&mut self) -> bool {
        let edge = self.selected_bit_set(self.tac);
        self.counter = 0;
        edge && self.step_tima()
    }

    /// Changing TAC can also make the selected bit fall.
    pub fn write_tac(&mut self, value: u8) -> bool {
        let value = value | 0xf8;
        let edge = self.selected_bit_set(self.tac) && !self.selected_bit_set(value);
        self.tac = value;
        edge && self.step_tima()
    }
}
