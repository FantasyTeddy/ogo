//! The boot contract between the launcher and games (docs/DESIGN.md §5).
//!
//! - The launcher installs a game into the `game_0` slot, records which launcher slot it runs
//!   from, makes `game_0` the boot slot and restarts.
//! - The first thing a game does is [`Boot::return_to_launcher_on_reset`]: it makes the launcher
//!   the boot slot again, so any reset, crash or power-off lands back in the launcher.
//!
//! Games and launchers that disagree about this contract must not be mixed; `sdk_version` in
//! `game.toml` guards against that.

use std::ffi::{CStr, CString};

use anyhow::{anyhow, Context};
use esp_idf_svc::{
    nvs::{EspDefaultNvs, EspDefaultNvsPartition},
    sys::{self, esp},
};

/// The flash slot games are installed into.
pub const GAME_PARTITION: &str = "game_0";
/// Used if NVS doesn't say which launcher slot started the game.
const DEFAULT_LAUNCHER_PARTITION: &str = "launcher_a";

const NVS_NAMESPACE: &str = "ogo";
/// Label of the launcher slot that started the game.
const KEY_LAUNCHER: &str = "launcher";
/// SHA-256 (hex) of the image in `game_0`. NVS keys are limited to 15 characters.
const KEY_GAME_HASH: &str = "game0_sha256";

pub struct Boot {
    nvs: EspDefaultNvs,
    partition: EspDefaultNvsPartition,
}

impl Boot {
    /// Opens the boot settings in NVS. Takes the default NVS partition, so call it once;
    /// [`Boot::nvs_partition`] hands it on to other users such as WiFi.
    pub fn take() -> anyhow::Result<Self> {
        let partition = EspDefaultNvsPartition::take()?;
        let nvs = EspDefaultNvs::new(partition.clone(), NVS_NAMESPACE, true)?;
        Ok(Self { nvs, partition })
    }

    pub fn nvs_partition(&self) -> EspDefaultNvsPartition {
        self.partition.clone()
    }

    /// Hash of the game image in `game_0`, if one was installed completely.
    pub fn installed_game_hash(&self) -> anyhow::Result<Option<String>> {
        let mut buf = [0u8; 65];
        Ok(self.nvs.get_str(KEY_GAME_HASH, &mut buf)?.map(str::to_owned))
    }

    /// Records the hash of the image in `game_0`; `None` while it's being replaced.
    pub fn set_installed_game_hash(&mut self, hash: Option<&str>) -> anyhow::Result<()> {
        match hash {
            Some(hash) => self.nvs.set_str(KEY_GAME_HASH, hash)?,
            None => {
                self.nvs.remove(KEY_GAME_HASH)?;
            }
        }
        Ok(())
    }

    /// Launcher side: boots the game in `game_0`. Doesn't return unless switching fails.
    pub fn launch_game(&mut self) -> anyhow::Result<()> {
        let launcher = running_partition_label();
        self.nvs.set_str(KEY_LAUNCHER, &launcher)?;
        set_boot_partition(GAME_PARTITION)?;
        log::info!("Booting {GAME_PARTITION}; it returns to {launcher}");
        restart()
    }

    /// Game side, call first thing: makes the launcher the boot slot again.
    pub fn return_to_launcher_on_reset(&mut self) -> anyhow::Result<()> {
        let mut buf = [0u8; 17];
        let launcher = self.nvs.get_str(KEY_LAUNCHER, &mut buf)?.unwrap_or(DEFAULT_LAUNCHER_PARTITION).to_owned();
        set_boot_partition(&launcher)
    }
}

/// Time for the amplifier to go quiet before the restart.
const AMPLIFIER_OFF_MS: u32 = 20;

/// Restarts the chip. Switches the speaker amplifier off first, so the reset doesn't pop.
pub fn restart() -> ! {
    // Raw GPIO access: the Speaker that owns the pin may be anywhere. If the pin isn't configured
    // as an output yet, this has no effect, which is fine.
    unsafe { sys::gpio_set_level(crate::pins::SPEAKER_ENABLE.into(), 0) };
    esp_idf_svc::hal::delay::FreeRtos::delay_ms(AMPLIFIER_OFF_MS);
    unsafe { sys::esp_restart() }
}

/// Label of the app slot this firmware runs from, e.g. `launcher_a`.
pub fn running_partition_label() -> String {
    let partition = unsafe { sys::esp_ota_get_running_partition() };
    if partition.is_null() {
        return "?".into();
    }
    unsafe { CStr::from_ptr((*partition).label.as_ptr()) }.to_string_lossy().into_owned()
}

/// Finds an app slot by label.
pub fn find_app_partition(label: &str) -> anyhow::Result<&'static sys::esp_partition_t> {
    let c_label = CString::new(label)?;
    let partition = unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_APP,
            sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_ANY,
            c_label.as_ptr(),
        )
    };
    // Partition table entries live as long as the program.
    unsafe { partition.as_ref() }.ok_or_else(|| anyhow!("no app partition named {label}"))
}

fn set_boot_partition(label: &str) -> anyhow::Result<()> {
    let partition = find_app_partition(label)?;
    esp!(unsafe { sys::esp_ota_set_boot_partition(partition) })
        .with_context(|| format!("can't make {label} the boot partition"))
}
