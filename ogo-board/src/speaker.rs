//! Speaker: tones from the DAC's hardware cosine generator.
//!
//! GPIO25 enables the amplifier (high = on) and GPIO26 (DAC channel 1) carries the signal.
//! The amplifier stays on unless the volume is 0: it needs tens of milliseconds to wake up,
//! so switching it on per tone swallows short beeps. Between tones the DAC holds a steady
//! mid level; an undriven input makes the amplifier crackle.
//! PCM playback through the DAC comes later with the SDK's audio (M4).

use esp_idf_svc::{
    hal::{
        delay::FreeRtos,
        gpio::{Output, OutputPin, PinDriver},
    },
    sys::{self, esp, EspError},
};

/// The cosine generator can't go lower with the default RTC clock.
const MIN_FREQUENCY_HZ: u32 = 130;
/// DAC level between tones; the cosine wave is centered on it too.
const IDLE_LEVEL: u8 = 128;
const DAC_CHANNEL: sys::dac_channel_t = sys::dac_channel_t_DAC_CHAN_1; // GPIO26

/// What drives the speaker pin. The ESP-IDF DAC drivers own the channel exclusively,
/// so switching between a steady level and a tone means deleting one and creating the other.
enum Dac {
    /// Released; only while the volume is 0 and the amplifier is off.
    Off,
    Idle(sys::dac_oneshot_handle_t),
    Tone(sys::dac_cosine_handle_t),
}

pub struct Speaker {
    enable: PinDriver<'static, Output>,
    dac: Dac,
    /// 0-100
    volume: u8,
}

impl Speaker {
    /// `_signal` is taken so nothing else can claim the DAC pin; the DAC driver configures it.
    pub(crate) fn new(
        enable: impl OutputPin + 'static,
        _signal: impl OutputPin + 'static,
    ) -> anyhow::Result<Self> {
        let mut speaker = Self { enable: PinDriver::output(enable)?, dac: Dac::Off, volume: 0 };
        speaker.set_volume(50)?;
        Ok(speaker)
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// Volume 0-100. The hardware only has four amplitude steps (0, -6, -12, -18 dB), so the
    /// value is rounded to the nearest of 25/50/75/100; 0 is silent and turns the amplifier off.
    pub fn set_volume(&mut self, volume: u8) -> anyhow::Result<()> {
        self.volume = volume.min(100);
        if self.volume == 0 {
            self.enable.set_low()?;
            self.release_dac()?;
        } else {
            if matches!(self.dac, Dac::Off) {
                self.dac = Dac::Idle(new_idle()?);
            }
            self.enable.set_high()?;
        }
        Ok(())
    }

    /// Plays a tone and blocks until it ends.
    pub fn beep(&mut self, frequency_hz: u32, duration_ms: u32) -> anyhow::Result<()> {
        self.start_tone(frequency_hz)?;
        FreeRtos::delay_ms(duration_ms);
        self.stop()
    }

    /// Starts a tone that plays until [`Speaker::stop`]. Frequencies below 130 Hz are raised to 130 Hz.
    pub fn start_tone(&mut self, frequency_hz: u32) -> anyhow::Result<()> {
        let Some(atten) = attenuation(self.volume) else {
            return Ok(());
        };
        self.release_dac()?;
        self.dac = Dac::Tone(new_tone(frequency_hz.max(MIN_FREQUENCY_HZ), atten)?);
        Ok(())
    }

    /// Stops the current tone, if any.
    pub fn stop(&mut self) -> anyhow::Result<()> {
        if matches!(self.dac, Dac::Tone(_)) {
            self.release_dac()?;
            self.dac = Dac::Idle(new_idle()?);
        }
        Ok(())
    }

    fn release_dac(&mut self) -> Result<(), EspError> {
        match std::mem::replace(&mut self.dac, Dac::Off) {
            Dac::Off => Ok(()),
            Dac::Idle(handle) => esp!(unsafe { sys::dac_oneshot_del_channel(handle) }),
            Dac::Tone(handle) => {
                esp!(unsafe { sys::dac_cosine_stop(handle) })?;
                esp!(unsafe { sys::dac_cosine_del_channel(handle) })
            }
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        let _ = self.set_volume(0);
    }
}

fn attenuation(volume: u8) -> Option<sys::dac_cosine_atten_t> {
    match volume {
        0 => None,
        1..=37 => Some(sys::dac_cosine_atten_t_DAC_COSINE_ATTEN_DB_18),
        38..=62 => Some(sys::dac_cosine_atten_t_DAC_COSINE_ATTEN_DB_12),
        63..=87 => Some(sys::dac_cosine_atten_t_DAC_COSINE_ATTEN_DB_6),
        _ => Some(sys::dac_cosine_atten_t_DAC_COSINE_ATTEN_DB_0),
    }
}

fn new_idle() -> Result<sys::dac_oneshot_handle_t, EspError> {
    let config = sys::dac_oneshot_config_t { chan_id: DAC_CHANNEL };
    let mut handle = core::ptr::null_mut();
    esp!(unsafe { sys::dac_oneshot_new_channel(&config, &mut handle) })?;
    if let Err(e) = esp!(unsafe { sys::dac_oneshot_output_voltage(handle, IDLE_LEVEL) }) {
        unsafe { sys::dac_oneshot_del_channel(handle) };
        return Err(e);
    }
    Ok(handle)
}

fn new_tone(freq_hz: u32, atten: sys::dac_cosine_atten_t) -> Result<sys::dac_cosine_handle_t, EspError> {
    let config = sys::dac_cosine_config_t {
        chan_id: DAC_CHANNEL,
        freq_hz,
        clk_src: sys::soc_periph_dac_cosine_clk_src_t_DAC_COSINE_CLK_SRC_DEFAULT,
        atten,
        phase: sys::dac_cosine_phase_t_DAC_COSINE_PHASE_0,
        offset: 0,
        flags: Default::default(),
    };
    let mut handle = core::ptr::null_mut();
    esp!(unsafe { sys::dac_cosine_new_channel(&config, &mut handle) })?;
    if let Err(e) = esp!(unsafe { sys::dac_cosine_start(handle) }) {
        unsafe { sys::dac_cosine_del_channel(handle) };
        return Err(e);
    }
    Ok(handle)
}
