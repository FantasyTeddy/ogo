//! Speaker: short sounds played as samples through the DAC's DMA mode.
//!
//! GPIO25 enables the amplifier (high = on) and GPIO26 (DAC channel 1) carries the signal.
//! The amplifier stays on unless the volume is 0: it needs tens of milliseconds to wake up,
//! so switching it on per sound swallows short beeps.
//!
//! The DAC never leaves DMA mode: an audio thread keeps feeding it, with the idle level when
//! nothing plays. Switching DAC modes per sound, or leaving the input undriven, makes the
//! amplifier click or crackle. Every note also fades in and out.
//!
//! The amplifier is switched off around anything that disturbs the DAC: before a restart
//! ([`crate::boot::restart`]) and while writing flash ([`Speaker::set_muted`]).
//! Continuous game audio comes with the SDK (M4).

use core::f32::consts::{PI, TAU};
use std::{
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
};

use esp_idf_svc::{
    hal::{
        delay::FreeRtos,
        gpio::{Output, OutputPin, PinDriver},
    },
    sys::{self, esp, EspError},
};

const DAC_CHANNEL_MASK: sys::dac_channel_mask_t = sys::dac_channel_mask_t_DAC_CHANNEL_MASK_CH1; // GPIO26
/// DAC level between sounds, and the center of every wave.
const IDLE_LEVEL: u8 = 128;
/// The ESP32's default DAC clock can't go below 19.6 kHz.
const SAMPLE_RATE: u32 = 32_000;
const DMA_DESCRIPTORS: u32 = 6;
const DMA_BUFFER_BYTES: usize = 128;
/// Samples the audio thread hands to the DMA per write.
const CHUNK_SAMPLES: usize = 256;
/// Sounds waiting to play; more are dropped.
const QUEUE_LEN: usize = 4;
/// Fade-in and fade-out of every note.
const FADE_MS: u32 = 5;
/// Time for the DAC to settle on the idle level before the amplifier is switched on.
const SETTLE_MS: u32 = 20;

/// One tone in a sound. A frequency of 0 is a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub frequency_hz: u32,
    pub duration_ms: u32,
}

impl Note {
    pub const fn new(frequency_hz: u32, duration_ms: u32) -> Self {
        Self { frequency_hz, duration_ms }
    }
}

pub struct Speaker {
    enable: PinDriver<'static, Output>,
    sounds: SyncSender<Vec<u8>>,
    /// 0-100
    volume: u8,
    muted: bool,
}

impl Speaker {
    /// `_signal` is taken so nothing else can claim the DAC pin; the DAC driver configures it.
    pub(crate) fn new(
        enable: impl OutputPin + 'static,
        _signal: impl OutputPin + 'static,
    ) -> anyhow::Result<Self> {
        let dac = Dac::open()?;
        let (sounds, queue) = mpsc::sync_channel(QUEUE_LEN);
        thread::Builder::new()
            .name("audio".into())
            .stack_size(4096)
            .spawn(move || feed(dac, queue))?;

        let mut enable = PinDriver::output(enable)?;
        enable.set_low()?;
        FreeRtos::delay_ms(SETTLE_MS);
        let mut speaker = Self { enable, sounds, volume: 0, muted: false };
        speaker.set_volume(50)?;
        Ok(speaker)
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// Volume 0-100. 0 is silent and turns the amplifier off.
    pub fn set_volume(&mut self, volume: u8) -> anyhow::Result<()> {
        self.volume = volume.min(100);
        self.update_amplifier()
    }

    /// Switches the amplifier off without changing the volume, e.g. while writing flash: that
    /// stalls the audio DMA, which would click. Sounds played while muted are lost.
    pub fn set_muted(&mut self, muted: bool) -> anyhow::Result<()> {
        self.muted = muted;
        self.update_amplifier()
    }

    fn update_amplifier(&mut self) -> anyhow::Result<()> {
        if self.volume == 0 || self.muted {
            self.enable.set_low()?;
        } else {
            self.enable.set_high()?;
        }
        Ok(())
    }

    /// Queues one tone. Returns immediately.
    pub fn beep(&mut self, frequency_hz: u32, duration_ms: u32) {
        self.play(&[Note::new(frequency_hz, duration_ms)]);
    }

    /// Queues the notes to play back to back, after any sounds already queued. Returns immediately.
    /// If too many sounds are waiting, this one is dropped.
    pub fn play(&mut self, notes: &[Note]) {
        if self.volume == 0 || notes.is_empty() {
            return;
        }
        match self.sounds.try_send(render(notes, self.volume)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => log::debug!("Sound queue full, dropping a sound"),
            Err(TrySendError::Disconnected(_)) => log::error!("The audio thread has stopped"),
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        let _ = self.enable.set_low();
    }
}

/// The DAC channel in DMA mode. Owned by the audio thread.
struct Dac(sys::dac_continuous_handle_t);

// The handle is only used from the audio thread after it's been handed over.
unsafe impl Send for Dac {}

impl Dac {
    fn open() -> Result<Self, EspError> {
        let config = sys::dac_continuous_config_t {
            chan_mask: DAC_CHANNEL_MASK,
            desc_num: DMA_DESCRIPTORS,
            buf_size: DMA_BUFFER_BYTES,
            freq_hz: SAMPLE_RATE,
            offset: 0,
            clk_src: sys::soc_periph_dac_digi_clk_src_t_DAC_DIGI_CLK_SRC_DEFAULT,
            chan_mode: sys::dac_continuous_channel_mode_t_DAC_CHANNEL_MODE_SIMUL,
        };
        let mut handle = core::ptr::null_mut();
        esp!(unsafe { sys::dac_continuous_new_channels(&config, &mut handle) })?;
        if let Err(e) = esp!(unsafe { sys::dac_continuous_enable(handle) }) {
            unsafe { sys::dac_continuous_del_channels(handle) };
            return Err(e);
        }
        Ok(Self(handle))
    }

    /// Blocks until the samples are in the DMA buffers.
    fn write(&mut self, samples: &mut [u8]) -> Result<(), EspError> {
        let mut loaded = 0;
        esp!(unsafe { sys::dac_continuous_write(self.0, samples.as_mut_ptr(), samples.len(), &mut loaded, -1) })
    }
}

/// The audio thread: plays queued sounds one after another and the idle level in between.
fn feed(mut dac: Dac, queue: Receiver<Vec<u8>>) {
    let mut chunk = [IDLE_LEVEL; CHUNK_SAMPLES];
    let mut sound: Vec<u8> = Vec::new();
    let mut position = 0;
    loop {
        if position >= sound.len() {
            match queue.try_recv() {
                Ok(next) => (sound, position) = (next, 0),
                Err(TryRecvError::Empty) => (sound, position) = (Vec::new(), 0),
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let count = (sound.len() - position).min(CHUNK_SAMPLES);
        chunk[..count].copy_from_slice(&sound[position..position + count]);
        chunk[count..].fill(IDLE_LEVEL);
        position += count;

        if let Err(e) = dac.write(&mut chunk) {
            log::error!("Writing to the DAC failed, audio stops: {e:?}");
            return;
        }
    }
}

/// 8-bit unsigned samples around `IDLE_LEVEL`. Each note is a sine with a raised-cosine fade.
fn render(notes: &[Note], volume: u8) -> Vec<u8> {
    let amplitude = 127.0 * f32::from(volume) / 100.0;
    let fade_samples = (SAMPLE_RATE * FADE_MS / 1000) as usize;
    let mut samples = Vec::new();
    for note in notes {
        let count = (SAMPLE_RATE * note.duration_ms / 1000) as usize;
        if note.frequency_hz == 0 {
            samples.resize(samples.len() + count, IDLE_LEVEL);
            continue;
        }
        let fade = fade_samples.min(count / 2).max(1);
        let step = TAU * note.frequency_hz as f32 / SAMPLE_RATE as f32;
        samples.extend((0..count).map(|i| {
            let edge = i.min(count - 1 - i);
            let envelope = if edge < fade { 0.5 - 0.5 * (PI * edge as f32 / fade as f32).cos() } else { 1.0 };
            let value = f32::from(IDLE_LEVEL) + amplitude * envelope * (step * i as f32).sin();
            value.round().clamp(0.0, 255.0) as u8
        }));
    }
    samples
}
