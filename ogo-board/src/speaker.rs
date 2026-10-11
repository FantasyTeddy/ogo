//! Speaker: short sounds played as samples through the DAC's DMA mode.
//!
//! GPIO25 enables the amplifier (high = on) and GPIO26 (DAC channel 1) carries the signal.
//!
//! The DAC never leaves DMA mode: an audio thread keeps feeding it, with the idle level when
//! nothing plays. Switching DAC modes per sound, or leaving the input undriven, makes the
//! amplifier click or crackle. Every note also fades in and out.
//!
//! The amplifier hisses faintly while on, so the audio thread switches it off after
//! [`AMP_IDLE_OFF_MS`] of silence. It needs time to wake up (short beeps right after switching
//! it on are lost), so a sound that finds it off is delayed by [`AMP_WAKE_MS`].
//! It is also switched off around anything that disturbs the DAC: before a restart
//! ([`crate::boot::restart`]) and while writing flash ([`Speaker::set_muted`]).

use core::f32::consts::{PI, TAU};
use std::{
    sync::{
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex,
    },
    thread,
};

use esp_idf_svc::{
    hal::gpio::{Output, OutputPin, PinDriver},
    sys::{self, esp, EspError},
};

const DAC_CHANNEL_MASK: sys::dac_channel_mask_t = sys::dac_channel_mask_t_DAC_CHANNEL_MASK_CH1; // GPIO26
/// DAC level between sounds, and the center of every wave.
const IDLE_LEVEL: u8 = 128;
/// The ESP32's default DAC clock can't go below 19.6 kHz.
const SAMPLE_RATE: u32 = 32_000;
const DMA_DESCRIPTORS: u32 = 6;
const DMA_BUFFER_BYTES: usize = 128;
/// Samples the audio thread hands to the DMA per write (8 ms).
const CHUNK_SAMPLES: usize = 256;
/// Sounds waiting to play; more are dropped.
const QUEUE_LEN: usize = 4;
/// Fade-in and fade-out of every note.
const FADE_MS: u32 = 5;
/// Silence after which the amplifier is switched off.
pub const AMP_IDLE_OFF_MS: u32 = 3000;
/// Idle level played after switching the amplifier on, before the sound starts.
pub const AMP_WAKE_MS: u32 = 150;

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

/// The amplifier enable pin and what may switch it on. Shared by [`Speaker`] and the audio thread.
struct Amp {
    pin: PinDriver<'static, Output>,
    on: bool,
    /// Volume 0 or muted: must stay off.
    blocked: bool,
}

impl Amp {
    fn set(&mut self, on: bool) {
        let on = on && !self.blocked;
        if on != self.on {
            let result = if on { self.pin.set_high() } else { self.pin.set_low() };
            if let Err(e) = result {
                log::error!("Switching the amplifier failed: {e:?}");
                return;
            }
            self.on = on;
        }
    }
}

pub struct Speaker {
    amp: Arc<Mutex<Amp>>,
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
        let mut pin = PinDriver::output(enable)?;
        pin.set_low()?;
        let amp = Arc::new(Mutex::new(Amp { pin, on: false, blocked: false }));

        let dac = Dac::open()?;
        let (sounds, queue) = mpsc::sync_channel(QUEUE_LEN);
        let thread_amp = amp.clone();
        thread::Builder::new()
            .name("audio".into())
            .stack_size(4096)
            .spawn(move || feed(dac, queue, thread_amp))?;

        Ok(Self { amp, sounds, volume: 50, muted: false })
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// Volume 0-100. 0 is silent and keeps the amplifier off.
    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
        self.update_amp();
    }

    /// Keeps the amplifier off without changing the volume, e.g. while writing flash: that stalls
    /// the audio DMA, which would click. Sounds played while muted are lost.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.update_amp();
    }

    fn update_amp(&mut self) {
        let mut amp = self.amp.lock().unwrap();
        amp.blocked = self.volume == 0 || self.muted;
        if amp.blocked {
            amp.set(false);
        }
    }

    /// Queues one tone. Returns immediately.
    pub fn beep(&mut self, frequency_hz: u32, duration_ms: u32) {
        self.play(&[Note::new(frequency_hz, duration_ms)]);
    }

    /// Queues the notes to play back to back, after any sounds already queued. Returns immediately.
    /// If too many sounds are waiting, this one is dropped.
    pub fn play(&mut self, notes: &[Note]) {
        if self.volume == 0 || self.muted || notes.is_empty() {
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
        self.set_muted(true);
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

    /// Plays the idle level for about `ms`.
    fn write_idle(&mut self, ms: u32) -> Result<(), EspError> {
        let mut chunk = [IDLE_LEVEL; CHUNK_SAMPLES];
        let chunks = (SAMPLE_RATE * ms / 1000).div_ceil(CHUNK_SAMPLES as u32);
        (0..chunks).try_for_each(|_| self.write(&mut chunk))
    }
}

/// The audio thread: plays queued sounds one after another and the idle level in between, and
/// switches the amplifier on for sounds and off after a while of silence.
fn feed(mut dac: Dac, queue: Receiver<Vec<u8>>, amp: Arc<Mutex<Amp>>) {
    let chunk_ms = CHUNK_SAMPLES as u32 * 1000 / SAMPLE_RATE;
    let mut idle_ms = 0;
    let mut chunk = [IDLE_LEVEL; CHUNK_SAMPLES];
    loop {
        // Keep the DMA fed with the idle level while waiting for a sound.
        let sound = match queue.try_recv() {
            Ok(sound) => sound,
            Err(TryRecvError::Empty) => {
                if idle_ms < AMP_IDLE_OFF_MS {
                    idle_ms += chunk_ms;
                    if idle_ms >= AMP_IDLE_OFF_MS {
                        amp.lock().unwrap().set(false);
                    }
                }
                chunk.fill(IDLE_LEVEL);
                if let Err(e) = dac.write(&mut chunk) {
                    log::error!("Writing to the DAC failed, audio stops: {e:?}");
                    return;
                }
                continue;
            }
            Err(TryRecvError::Disconnected) => return,
        };

        let woke = {
            let mut amp = amp.lock().unwrap();
            let was_on = amp.on;
            amp.set(true);
            amp.on && !was_on
        };
        let result = (|| {
            if woke {
                dac.write_idle(AMP_WAKE_MS)?;
            }
            for samples in sound.chunks(CHUNK_SAMPLES) {
                chunk[..samples.len()].copy_from_slice(samples);
                chunk[samples.len()..].fill(IDLE_LEVEL);
                dac.write(&mut chunk)?;
            }
            Ok::<_, EspError>(())
        })();
        if let Err(e) = result {
            log::error!("Writing to the DAC failed, audio stops: {e:?}");
            return;
        }
        idle_ms = 0;
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
