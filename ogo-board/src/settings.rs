//! Settings shared by the launcher and games, stored in NVS (namespace `ogo`). The steps and
//! defaults are in `ogo_format::settings`.
//!
//! The launcher's settings screen changes them; games read them at start ([`crate::Board::take_with`]
//! applies them), and the SDK saves the volume (VOL) and music (pause menu) changed in a game.
//! Missing or unreadable keys fall back to the defaults, so an empty NVS is fine.

use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};
pub use ogo_format::settings::{cycle, step, BRIGHTNESS_STEPS, VOLUME_STEPS};
use ogo_format::settings::{nearest_step, DEFAULT_BRIGHTNESS, DEFAULT_MUSIC, DEFAULT_VOLUME};

pub const NVS_NAMESPACE: &str = "ogo";
const KEY_BRIGHTNESS: &str = "brightness";
const KEY_VOLUME: &str = "volume";
const KEY_MUSIC: &str = "music";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Percent, one of [`BRIGHTNESS_STEPS`].
    pub brightness: u8,
    /// Percent, one of [`VOLUME_STEPS`].
    pub volume: u8,
    /// Games play their music.
    pub music: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: DEFAULT_BRIGHTNESS,
            volume: DEFAULT_VOLUME,
            music: DEFAULT_MUSIC,
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
            music: match nvs.get_u8(KEY_MUSIC) {
                Ok(Some(value)) => value != 0,
                Ok(None) => default.music,
                Err(e) => {
                    log::warn!("Can't read setting {KEY_MUSIC}: {e:?}");
                    default.music
                }
            },
        }
    }

    pub fn save(&self, nvs: &EspDefaultNvs) -> anyhow::Result<()> {
        nvs.set_u8(KEY_BRIGHTNESS, self.brightness)?;
        nvs.set_u8(KEY_VOLUME, self.volume)?;
        nvs.set_u8(KEY_MUSIC, u8::from(self.music))?;
        Ok(())
    }
}
