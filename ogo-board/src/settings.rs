//! Settings shared by the launcher and games, stored in NVS (namespace `ogo`).
//!
//! The launcher's settings screen changes them; games read them at start ([`crate::Board::take_with`]
//! applies them), and the SDK saves volume changes made with VOL. Missing or unreadable keys fall
//! back to the defaults, so an empty NVS is fine.

use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};

pub const NVS_NAMESPACE: &str = "ogo";
const KEY_BRIGHTNESS: &str = "brightness";
const KEY_VOLUME: &str = "volume";

/// Backlight levels in percent. The lowest still shows the screen; see [`crate::Backlight`].
pub const BRIGHTNESS_STEPS: [u8; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// Volume levels in percent, as cycled by VOL in games.
pub const VOLUME_STEPS: [u8; 5] = [0, 25, 50, 75, 100];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Percent, one of [`BRIGHTNESS_STEPS`].
    pub brightness: u8,
    /// Percent, one of [`VOLUME_STEPS`].
    pub volume: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self { brightness: 100, volume: 50 }
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
        }
    }

    pub fn save(&self, nvs: &EspDefaultNvs) -> anyhow::Result<()> {
        nvs.set_u8(KEY_BRIGHTNESS, self.brightness)?;
        Self::save_volume(nvs, self.volume)
    }

    /// Saves only the volume, for VOL in games.
    pub fn save_volume(nvs: &EspDefaultNvs, volume: u8) -> anyhow::Result<()> {
        nvs.set_u8(KEY_VOLUME, volume)?;
        Ok(())
    }
}

/// The step `delta` steps away from `current`, clamped to the ends.
pub fn step(steps: &[u8], current: u8, delta: isize) -> u8 {
    let index = steps.iter().position(|&s| s >= current).unwrap_or(steps.len() - 1);
    steps[(index as isize + delta).clamp(0, steps.len() as isize - 1) as usize]
}

/// The step after `current`, starting over after the last one.
pub fn cycle(steps: &[u8], current: u8) -> u8 {
    let index = steps.iter().position(|&s| s >= current).unwrap_or(0);
    steps[(index + 1) % steps.len()]
}

fn nearest_step(steps: &[u8], value: u8) -> u8 {
    *steps.iter().min_by_key(|&&s| s.abs_diff(value)).expect("steps aren't empty")
}
