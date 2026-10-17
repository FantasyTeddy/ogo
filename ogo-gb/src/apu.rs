//! The sound: two square channels (the first with a frequency sweep), a wave channel and a noise
//! channel, with their length counters and volume envelopes, mixed to mono samples.
//!
//! The channels advance in steps of whole samples: each sample is the channels' output at that
//! moment, passed through a high-pass filter like the capacitor at the console's output.

use alloc::vec::Vec;

use crate::CLOCK_HZ;

/// The frame sequencer steps at 512 Hz: length counters on every other step (256 Hz), the sweep
/// on every fourth (128 Hz), envelopes on every eighth (64 Hz).
const FRAME_SEQUENCER_CYCLES: u32 = CLOCK_HZ / 512;
/// The square waves' duty cycles, as 8 steps from bit 7 down: 12.5%, 25%, 50%, 75%; a byte each,
/// packed into one constant so that the mixing doesn't read a table from flash on the ESP32.
const DUTY: u32 = u32::from_le_bytes([0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110]);
/// NR32's output levels (mute, 100%, 50%, 25%) as right shifts of the wave sample, 4 bits each.
const WAVE_SHIFTS: u32 = 0x2104;
const NOISE_DIVISORS: [u32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];
/// Bits of NR10-NR52 (0xFF10-0xFF26) that read as 1.
const READ_MASK: [u8; 0x17] = [
    0x80, 0x3f, 0x00, 0xff, 0xbf, // NR10-NR14
    0xff, 0x3f, 0x00, 0xff, 0xbf, // -, NR21-NR24
    0x7f, 0xff, 0x9f, 0xff, 0xbf, // NR30-NR34
    0xff, 0xff, 0x00, 0x00, 0xbf, // -, NR41-NR44
    0x00, 0x00, 0x70, // NR50-NR52
];
/// Samples kept when nobody takes them, at most.
const MAX_BUFFERED: usize = 1 << 16;

#[derive(Debug, Clone, Default)]
struct Length {
    counter: u16,
    enabled: bool,
}

impl Length {
    /// Counts down if enabled; returns whether it just reached 0 (the channel stops).
    fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            self.counter == 0
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Envelope {
    initial: u8,
    increase: bool,
    pace: u8,
    volume: u8,
    timer: u8,
}

impl Envelope {
    fn write(&mut self, value: u8) {
        self.initial = value >> 4;
        self.increase = value & 0x08 != 0;
        self.pace = value & 0x07;
    }

    /// The channel's DAC is on unless volume and direction are all 0.
    #[inline(always)]
    fn dac_on(&self) -> bool {
        self.initial != 0 || self.increase
    }

    fn trigger(&mut self) {
        self.volume = self.initial;
        self.timer = self.pace;
    }

    fn clock(&mut self) {
        if self.pace == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.pace;
            if self.increase && self.volume < 15 {
                self.volume += 1;
            } else if !self.increase && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

/// Counts down `timer` (1..=`period`) by `cycles`; returns how often it ran out, reloading it each
/// time.
#[inline(always)]
fn run_timer(timer: &mut u32, period: u32, cycles: u32) -> u32 {
    if cycles < *timer {
        *timer -= cycles;
        0
    } else {
        let over = cycles - *timer;
        if over < period {
            // Ran out once: the usual case, without dividing (slow on the ESP32)
            *timer = period - over;
            1
        } else {
            *timer = period - over % period;
            over / period + 1
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Square {
    on: bool,
    duty: u8,
    step: u8,
    period: u16,
    timer: u32,
    length: Length,
    envelope: Envelope,
}

impl Square {
    #[inline(always)]
    fn timer_period(&self) -> u32 {
        (2048 - self.period as u32) * 4
    }

    #[inline(always)]
    fn advance(&mut self, cycles: u32) {
        let period = self.timer_period();
        let steps = run_timer(&mut self.timer, period, cycles);
        self.step = (self.step + steps as u8) & 7;
    }

    #[inline(always)]
    fn output(&self) -> u8 {
        if self.on && DUTY >> (self.duty as u32 * 8 + 7 - self.step as u32) & 1 != 0 {
            self.envelope.volume
        } else {
            0
        }
    }
}

/// Channel 1's frequency sweep.
#[derive(Debug, Clone, Default)]
struct Sweep {
    pace: u8,
    down: bool,
    shift: u8,
    timer: u8,
    enabled: bool,
    shadow: u16,
    /// A downward calculation happened since the last trigger: switching to upward then stops the
    /// channel.
    went_down: bool,
}

impl Sweep {
    /// The next period; above 2047 the channel stops.
    fn next_period(&mut self) -> u16 {
        let delta = self.shadow >> self.shift;
        if self.down {
            self.went_down = true;
            self.shadow - delta
        } else {
            self.shadow + delta
        }
    }

    fn reload_timer(&mut self) {
        self.timer = if self.pace == 0 { 8 } else { self.pace };
    }
}

#[derive(Debug, Clone, Default)]
struct WaveChannel {
    on: bool,
    dac: bool,
    /// NR32's output level: mute, 100%, 50%, 25%.
    level: u8,
    period: u16,
    timer: u32,
    position: u8,
    sample: u8,
    length: Length,
}

impl WaveChannel {
    #[inline(always)]
    fn timer_period(&self) -> u32 {
        (2048 - self.period as u32) * 2
    }

    #[inline(always)]
    fn advance(&mut self, cycles: u32, ram: &[u8; 16]) {
        let period = self.timer_period();
        let steps = run_timer(&mut self.timer, period, cycles);
        if steps > 0 {
            self.position = ((self.position as u32 + steps) & 31) as u8;
            let byte = ram[self.position as usize / 2];
            self.sample = if self.position & 1 == 0 {
                byte >> 4
            } else {
                byte & 0x0f
            };
        }
    }

    #[inline(always)]
    fn output(&self) -> u8 {
        if self.on {
            self.sample >> (WAVE_SHIFTS >> (self.level * 4) & 0xf)
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Noise {
    on: bool,
    shift: u8,
    short: bool,
    divisor: u8,
    lfsr: u16,
    timer: u32,
    length: Length,
    envelope: Envelope,
}

impl Noise {
    #[inline(always)]
    fn timer_period(&self) -> u32 {
        NOISE_DIVISORS[self.divisor as usize] << self.shift
    }

    #[inline(always)]
    fn advance(&mut self, cycles: u32) {
        let period = self.timer_period();
        let steps = run_timer(&mut self.timer, period, cycles);
        for _ in 0..steps.min(64) {
            let bit = (self.lfsr ^ self.lfsr >> 1) & 1;
            self.lfsr = self.lfsr >> 1 | bit << 14;
            if self.short {
                self.lfsr = self.lfsr & !0x40 | bit << 6;
            }
        }
    }

    #[inline(always)]
    fn output(&self) -> u8 {
        if self.on && self.lfsr & 1 == 0 {
            self.envelope.volume
        } else {
            0
        }
    }
}

pub(crate) struct Apu {
    /// NR10-NR51 as last written, for reading back.
    regs: [u8; 0x17],
    power: bool,
    ch1: Square,
    sweep: Sweep,
    ch2: Square,
    ch3: WaveChannel,
    ch4: Noise,
    wave_ram: [u8; 16],
    /// The next frame sequencer step (0-7) and the cycles until it.
    sequencer_step: u8,
    sequencer_timer: u32,
    /// Samples per second; 0 makes none.
    sample_rate: u32,
    /// Counts `sample_rate` per cycle; a sample is due at each `CLOCK_HZ`.
    sample_phase: u32,
    /// Cycles until `sample_phase` reaches `CLOCK_HZ`: `CLOCK_HZ / sample_rate`, plus one while
    /// `sample_phase` is below `CLOCK_HZ % sample_rate` (computed once, as dividing is slow).
    to_sample: u32,
    cycles_per_sample: u32,
    sample_remainder: u32,
    /// Each channel's factor in the mix: its left and right volumes (NR50) where NR51 routes it,
    /// 0 while its DAC is off. Updated on every register write.
    weights: [i32; 4],
    /// The high-pass filter's state and how much of it remains after one sample.
    capacitor: f32,
    charge: f32,
    pub samples: Vec<i16>,
}

impl Apu {
    pub fn new() -> Self {
        let mut apu = Apu {
            regs: [0; 0x17],
            power: false,
            ch1: Square::default(),
            sweep: Sweep::default(),
            ch2: Square::default(),
            ch3: WaveChannel::default(),
            ch4: Noise::default(),
            wave_ram: [0; 16],
            sequencer_step: 0,
            sequencer_timer: FRAME_SEQUENCER_CYCLES,
            sample_rate: 0,
            sample_phase: 0,
            to_sample: u32::MAX,
            cycles_per_sample: 0,
            sample_remainder: 0,
            weights: [0; 4],
            capacitor: 0.0,
            charge: 0.0,
            samples: Vec::new(),
        };
        // The state the boot ROM leaves: sound on, channel 1 having played its chime
        apu.write(0xff26, 0x80);
        for (addr, value) in [
            (0xff10, 0x80),
            (0xff11, 0xbf),
            (0xff12, 0xf3),
            (0xff14, 0xbf),
            (0xff16, 0x3f),
            (0xff19, 0xbf),
            (0xff1a, 0x7f),
            (0xff1b, 0xff),
            (0xff1c, 0x9f),
            (0xff1e, 0xbf),
            (0xff20, 0xff),
            (0xff23, 0xbf),
            (0xff24, 0x77),
            (0xff25, 0xf3),
        ] {
            apu.write(addr, value);
        }
        apu.ch1.on = false;
        apu.ch3.on = false;
        apu.ch4.on = false;
        apu
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
        self.sample_phase = 0;
        if let (Some(cycles), Some(remainder)) =
            (CLOCK_HZ.checked_div(rate), CLOCK_HZ.checked_rem(rate))
        {
            self.cycles_per_sample = cycles;
            self.sample_remainder = remainder;
            self.to_sample = self.cycles_to_sample();
        } else {
            self.to_sample = u32::MAX;
        }
        // The capacitor keeps 0.999958 of its charge per cycle
        let cycles_per_sample = CLOCK_HZ as f32 / rate.max(1) as f32;
        self.charge = libm_powf(0.999_958, cycles_per_sample);
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.ogo_gb")]
    pub fn tick(&mut self, mut cycles: u32) {
        while cycles > 0 {
            let step = cycles.min(self.sequencer_timer).min(self.to_sample);
            self.advance_channels(step);
            cycles -= step;

            self.sequencer_timer -= step;
            if self.sequencer_timer == 0 {
                self.sequencer_timer = FRAME_SEQUENCER_CYCLES;
                if self.power {
                    self.step_sequencer();
                }
            }
            if self.sample_rate != 0 {
                self.sample_phase += step * self.sample_rate;
                self.to_sample -= step;
                if self.to_sample == 0 {
                    self.sample_phase -= CLOCK_HZ;
                    self.to_sample = self.cycles_to_sample();
                    self.push_sample();
                }
            }
        }
    }

    /// `(CLOCK_HZ - sample_phase).div_ceil(sample_rate)`, for `sample_phase < sample_rate`.
    #[inline(always)]
    fn cycles_to_sample(&self) -> u32 {
        self.cycles_per_sample + (self.sample_phase < self.sample_remainder) as u32
    }

    #[inline(always)]
    fn advance_channels(&mut self, cycles: u32) {
        // Only heard through the samples: without them, the waveforms needn't move
        if self.sample_rate == 0 || !self.power {
            return;
        }
        self.ch1.advance(cycles);
        self.ch2.advance(cycles);
        self.ch3.advance(cycles, &self.wave_ram);
        self.ch4.advance(cycles);
    }

    fn step_sequencer(&mut self) {
        let step = self.sequencer_step;
        if step.is_multiple_of(2) {
            self.clock_lengths();
        }
        if step == 2 || step == 6 {
            self.clock_sweep();
        }
        if step == 7 {
            self.ch1.envelope.clock();
            self.ch2.envelope.clock();
            self.ch4.envelope.clock();
        }
        self.sequencer_step = (step + 1) & 7;
    }

    fn clock_lengths(&mut self) {
        if self.ch1.length.clock() {
            self.ch1.on = false;
        }
        if self.ch2.length.clock() {
            self.ch2.on = false;
        }
        if self.ch3.length.clock() {
            self.ch3.on = false;
        }
        if self.ch4.length.clock() {
            self.ch4.on = false;
        }
    }

    fn clock_sweep(&mut self) {
        self.sweep.timer = self.sweep.timer.saturating_sub(1);
        if self.sweep.timer != 0 {
            return;
        }
        self.sweep.reload_timer();
        if !self.sweep.enabled || self.sweep.pace == 0 {
            return;
        }
        let period = self.sweep.next_period();
        if period > 2047 {
            self.ch1.on = false;
        } else if self.sweep.shift != 0 {
            self.sweep.shadow = period;
            self.ch1.period = period;
            // A second calculation, only to check for overflow
            if self.sweep.next_period() > 2047 {
                self.ch1.on = false;
            }
        }
    }

    #[inline(always)]
    fn push_sample(&mut self) {
        // The DACs map 0..15 to an analog level around 0
        let level = |output: u8| 2 * output as i32 - 15;
        let [w1, w2, w3, w4] = self.weights;
        let sum = w1 * level(self.ch1.output())
            + w2 * level(self.ch2.output())
            + w3 * level(self.ch3.output())
            + w4 * level(self.ch4.output());
        // At most 4 channels x 15 x 8 on each side
        let input = sum as f32 * (1.0 / 960.0);
        let output = input - self.capacitor;
        self.capacitor = input - output * self.charge;
        if self.samples.len() >= MAX_BUFFERED {
            self.samples.clear();
        }
        self.samples
            .push((output * 32767.0).clamp(-32768.0, 32767.0) as i16);
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xff26 => {
                let on = [self.ch1.on, self.ch2.on, self.ch3.on, self.ch4.on]
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (i, &on)| bits | (on as u8) << i);
                0x70 | (self.power as u8) << 7 | on
            }
            0xff10..=0xff25 => {
                let i = (addr - 0xff10) as usize;
                self.regs[i] | READ_MASK[i]
            }
            0xff30..=0xff3f => self.wave_ram[(addr - 0xff30) as usize],
            _ => 0xff,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        self.write_register(addr, value);
        self.update_weights();
    }

    /// The channels' factors in the mix, from NR50, NR51 and whether their DACs are on.
    fn update_weights(&mut self) {
        let nr50 = self.regs[0x14] as i32;
        let nr51 = self.regs[0x15];
        let (left, right) = ((nr50 >> 4 & 7) + 1, (nr50 & 7) + 1);
        let dacs = [
            self.ch1.envelope.dac_on(),
            self.ch2.envelope.dac_on(),
            self.ch3.dac,
            self.ch4.envelope.dac_on(),
        ];
        for (channel, dac) in dacs.into_iter().enumerate() {
            self.weights[channel] = if self.power && dac {
                (nr51 >> (4 + channel) & 1) as i32 * left + (nr51 >> channel & 1) as i32 * right
            } else {
                0
            };
        }
    }

    fn write_register(&mut self, addr: u16, value: u8) {
        if let 0xff30..=0xff3f = addr {
            self.wave_ram[(addr - 0xff30) as usize] = value;
            return;
        }
        if addr == 0xff26 {
            self.write_power(value & 0x80 != 0);
            return;
        }
        if !(0xff10..=0xff25).contains(&addr) {
            return;
        }
        if !self.power {
            // While off, only the length counters can be written
            match addr {
                0xff11 => self.ch1.length.counter = 64 - (value & 0x3f) as u16,
                0xff16 => self.ch2.length.counter = 64 - (value & 0x3f) as u16,
                0xff1b => self.ch3.length.counter = 256 - value as u16,
                0xff20 => self.ch4.length.counter = 64 - (value & 0x3f) as u16,
                _ => {}
            }
            return;
        }
        self.regs[(addr - 0xff10) as usize] = value;
        // Length enabled in the half of the frame sequencer period after a length clock gets an
        // extra clock
        let extra_length_clock = self.sequencer_step & 1 == 1;
        match addr {
            0xff10 => {
                self.sweep.pace = value >> 4 & 7;
                let down = value & 0x08 != 0;
                if self.sweep.down && !down && self.sweep.went_down {
                    self.ch1.on = false;
                }
                self.sweep.down = down;
                self.sweep.shift = value & 7;
            }
            0xff11 => {
                self.ch1.duty = value >> 6;
                self.ch1.length.counter = 64 - (value & 0x3f) as u16;
            }
            0xff12 => {
                self.ch1.envelope.write(value);
                if !self.ch1.envelope.dac_on() {
                    self.ch1.on = false;
                }
            }
            0xff13 => self.ch1.period = self.ch1.period & 0x700 | value as u16,
            0xff14 => {
                self.ch1.period = self.ch1.period & 0xff | ((value & 7) as u16) << 8;
                let trigger = value & 0x80 != 0;
                if write_length_enable(&mut self.ch1.length, value, extra_length_clock, trigger, 64)
                {
                    self.ch1.on = false;
                }
                if trigger {
                    self.trigger_ch1();
                }
            }
            0xff16 => {
                self.ch2.duty = value >> 6;
                self.ch2.length.counter = 64 - (value & 0x3f) as u16;
            }
            0xff17 => {
                self.ch2.envelope.write(value);
                if !self.ch2.envelope.dac_on() {
                    self.ch2.on = false;
                }
            }
            0xff18 => self.ch2.period = self.ch2.period & 0x700 | value as u16,
            0xff19 => {
                self.ch2.period = self.ch2.period & 0xff | ((value & 7) as u16) << 8;
                let trigger = value & 0x80 != 0;
                if write_length_enable(&mut self.ch2.length, value, extra_length_clock, trigger, 64)
                {
                    self.ch2.on = false;
                }
                if trigger {
                    trigger_square(&mut self.ch2);
                }
            }
            0xff1a => {
                self.ch3.dac = value & 0x80 != 0;
                if !self.ch3.dac {
                    self.ch3.on = false;
                }
            }
            0xff1b => self.ch3.length.counter = 256 - value as u16,
            0xff1c => self.ch3.level = value >> 5 & 3,
            0xff1d => self.ch3.period = self.ch3.period & 0x700 | value as u16,
            0xff1e => {
                self.ch3.period = self.ch3.period & 0xff | ((value & 7) as u16) << 8;
                let trigger = value & 0x80 != 0;
                if write_length_enable(
                    &mut self.ch3.length,
                    value,
                    extra_length_clock,
                    trigger,
                    256,
                ) {
                    self.ch3.on = false;
                }
                if trigger {
                    let ch = &mut self.ch3;
                    ch.on = ch.dac;
                    ch.timer = ch.timer_period();
                    ch.position = 0;
                }
            }
            0xff20 => self.ch4.length.counter = 64 - (value & 0x3f) as u16,
            0xff21 => {
                self.ch4.envelope.write(value);
                if !self.ch4.envelope.dac_on() {
                    self.ch4.on = false;
                }
            }
            0xff22 => {
                self.ch4.shift = value >> 4;
                self.ch4.short = value & 0x08 != 0;
                self.ch4.divisor = value & 7;
            }
            0xff23 => {
                let trigger = value & 0x80 != 0;
                if write_length_enable(&mut self.ch4.length, value, extra_length_clock, trigger, 64)
                {
                    self.ch4.on = false;
                }
                if trigger {
                    let ch = &mut self.ch4;
                    ch.on = ch.envelope.dac_on();
                    ch.envelope.trigger();
                    ch.timer = ch.timer_period();
                    ch.lfsr = 0x7fff;
                }
            }
            _ => {} // NR50, NR51: read when mixing
        }
    }

    fn trigger_ch1(&mut self) {
        trigger_square(&mut self.ch1);
        let sweep = &mut self.sweep;
        sweep.shadow = self.ch1.period;
        sweep.reload_timer();
        sweep.enabled = sweep.pace != 0 || sweep.shift != 0;
        sweep.went_down = false;
        if sweep.shift != 0 && sweep.next_period() > 2047 {
            self.ch1.on = false;
        }
    }

    fn write_power(&mut self, on: bool) {
        if on == self.power {
            return;
        }
        if on {
            self.power = true;
            self.sequencer_step = 0;
            self.sequencer_timer = FRAME_SEQUENCER_CYCLES;
            return;
        }
        // Off clears every register; the length counters stay
        for addr in 0xff10..=0xff25 {
            self.write(addr, 0);
        }
        let lengths = [
            self.ch1.length.counter,
            self.ch2.length.counter,
            self.ch3.length.counter,
            self.ch4.length.counter,
        ];
        self.ch1 = Square::default();
        self.ch2 = Square::default();
        self.ch3 = WaveChannel::default();
        self.ch4 = Noise::default();
        self.sweep = Sweep::default();
        self.ch1.length.counter = lengths[0];
        self.ch2.length.counter = lengths[1];
        self.ch3.length.counter = lengths[2];
        self.ch4.length.counter = lengths[3];
        self.power = false;
    }
}

fn trigger_square(ch: &mut Square) {
    ch.on = ch.envelope.dac_on();
    ch.envelope.trigger();
    ch.timer = ch.timer_period();
}

/// Applies NRx4's length enable bit, with the quirks around it; returns whether the channel stops.
fn write_length_enable(
    length: &mut Length,
    value: u8,
    extra_clock: bool,
    trigger: bool,
    max: u16,
) -> bool {
    let was_enabled = length.enabled;
    length.enabled = value & 0x40 != 0;
    let mut stops = false;
    if extra_clock && !was_enabled && length.enabled && length.counter > 0 {
        length.counter -= 1;
        stops = length.counter == 0 && !trigger;
    }
    if trigger && length.counter == 0 {
        length.counter = if length.enabled && extra_clock {
            max - 1
        } else {
            max
        };
    }
    stops
}

/// `base` to the power of `exponent`, for a positive base; `no_std` has no `powf`.
fn libm_powf(base: f32, exponent: f32) -> f32 {
    // exp(exponent * ln(base)), with ln and exp from their series
    let x = exponent * ln(base);
    let mut term = 1.0;
    let mut sum = 1.0;
    for n in 1..30 {
        term *= x / n as f32;
        sum += term;
    }
    sum
}

/// The natural logarithm of `x` (0 < x <= 1 is enough here), via atanh's series.
fn ln(x: f32) -> f32 {
    let y = (x - 1.0) / (x + 1.0);
    let y2 = y * y;
    let mut term = y;
    let mut sum = 0.0;
    for n in 0..30 {
        sum += term / (2 * n + 1) as f32;
        term *= y2;
    }
    2.0 * sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powf() {
        let value = libm_powf(0.999_958, 131.072);
        assert!((value - 0.994_51).abs() < 1e-4, "{value}");
    }

    #[test]
    fn square_wave_is_heard() {
        let mut apu = Apu::new();
        apu.set_sample_rate(32_000);
        apu.write(0xff12, 0xf0); // full volume
        apu.write(0xff13, 0x00);
        apu.write(0xff14, 0x87); // 131072 / (2048 - 0x700) = 256 Hz
        apu.tick(CLOCK_HZ / 8);
        let samples = core::mem::take(&mut apu.samples);
        assert_eq!(samples.len(), 4000);
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 5000, "peak {peak}");
    }
}
