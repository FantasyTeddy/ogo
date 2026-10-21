//! Settings shared by the launcher and apps, stored in NVS (namespace `ogo`). The steps and
//! defaults are in `ogo_format::settings`.
//!
//! The launcher's settings screen changes them; apps read them at start ([`crate::Board::take_with`]
//! applies them), and the SDK saves what VOL and the pause menu change in an app. VOL in the
//! launcher switches muted on and off. Missing or unreadable keys fall back to the defaults, so an
//! empty NVS is fine.

use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};
pub use ogo_format::settings::{
    cycle, step, volume_output, SoundMode, BRIGHTNESS_STEPS, VOLUME_STEPS,
};
use ogo_format::settings::{nearest_step, DEFAULT_BRIGHTNESS, DEFAULT_MUSIC, DEFAULT_VOLUME};

pub const NVS_NAMESPACE: &str = "ogo";
const KEY_BRIGHTNESS: &str = "brightness";
const KEY_VOLUME: &str = "volume";
const KEY_MUSIC: &str = "music";
const KEY_MUTED: &str = "muted";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Percent, one of [`BRIGHTNESS_STEPS`].
    pub brightness: u8,
    /// The level shown, in percent, one of [`VOLUME_STEPS`]; the speaker gets [`Self::speaker_volume`].
    pub volume: u8,
    /// Apps play their music.
    pub music: bool,
    /// No sound at all; `volume` is kept for when it's switched off again.
    pub muted: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: DEFAULT_BRIGHTNESS,
            volume: DEFAULT_VOLUME,
            music: DEFAULT_MUSIC,
            muted: false,
        }
    }
}

impl Settings {
    /// Opens the NVS namespace the settings live in.
    pub fn open_nvs(partition: EspDefaultNvsPartition) -> anyhow::Result<EspDefaultNvs> {
        Ok(EspDefaultNvs::new(partition, NVS_NAMESPACE, true)?)
    }

    pub fn load(nvs: &EspDefaultNvs) -> Self {
        let default = Self::default();
        let read_flag = |key: &str, fallback: bool| match nvs.get_u8(key) {
            Ok(Some(value)) => value != 0,
            Ok(None) => fallback,
            Err(e) => {
                log::warn!("Can't read setting {key}: {e:?}");
                fallback
            }
        };
        let read = |key: &str, steps: &[u8], fallback: u8| match nvs.get_u8(key) {
            Ok(Some(value)) => nearest_step(steps, value),
            Ok(None) => fallback,
            Err(e) => {
                log::warn!("Can't read setting {key}: {e:?}");
                fallback
            }
        };
        Self {
            brightness: read(KEY_BRIGHTNESS, &BRIGHTNESS_STEPS, default.brightness),
            volume: read(KEY_VOLUME, &VOLUME_STEPS, default.volume),
            music: read_flag(KEY_MUSIC, default.music),
            muted: read_flag(KEY_MUTED, default.muted),
        }
    }

    pub fn save(&self, nvs: &EspDefaultNvs) -> anyhow::Result<()> {
        nvs.set_u8(KEY_BRIGHTNESS, self.brightness)?;
        nvs.set_u8(KEY_VOLUME, self.volume)?;
        nvs.set_u8(KEY_MUSIC, u8::from(self.music))?;
        nvs.set_u8(KEY_MUTED, u8::from(self.muted))?;
        Ok(())
    }

    /// The volume to give the speaker: the level's output (see `VOLUME_OUTPUT`), 0 while muted.
    pub fn speaker_volume(&self) -> u8 {
        if self.muted {
            0
        } else {
            volume_output(self.volume)
        }
    }

    pub fn sound_mode(&self) -> SoundMode {
        SoundMode::new(self.music, self.muted)
    }

    /// Muted leaves the music setting alone; the others set it.
    pub fn set_sound_mode(&mut self, mode: SoundMode) {
        match mode {
            SoundMode::All => (self.music, self.muted) = (true, false),
            SoundMode::EffectsOnly => (self.music, self.muted) = (false, false),
            SoundMode::Muted => self.muted = true,
        }
    }
}
