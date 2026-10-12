//! The boot contract between the launcher and games (docs/CONTRACTS.md).
//!
//! - The launcher installs a game into the `game_0` slot, records which launcher slot it runs
//!   from, makes `game_0` the boot slot and restarts.
//! - The first thing a game does is [`Boot::return_to_launcher_on_reset`]: it makes the launcher
//!   the boot slot again, so any reset, crash or power-off lands back in the launcher.
//! - With app rollback enabled (sdkconfig), every newly selected slot starts "pending verify". It
//!   must call [`mark_running_valid`] once it's sure it works, or the next reset rolls back to the
//!   slot selected before. The launcher does that after its self-check on every start (a game's
//!   return makes the launcher "new" again), games do it in `return_to_launcher_on_reset`.
//! - Launcher updates go into the launcher slot that isn't running ([`inactive_launcher_slot`]).
//!   [`Boot::set_pending_update`] remembers the target across the restart, so the next start can
//!   tell an update that worked from one that was rolled back ([`Boot::take_update_result`]).
//!
//! Games and launchers that disagree about this contract must not be mixed; `sdk_version` in
//! `game.toml` guards against that.

use std::ffi::{CStr, CString};

use anyhow::{anyhow, Context};
use esp_idf_svc::{
    nvs::{EspDefaultNvs, EspDefaultNvsPartition},
    sys::{self, esp},
};

use ogo_format::slots::{self, LauncherSlot};

use crate::settings::NVS_NAMESPACE;

/// The flash slot games are installed into.
pub const GAME_PARTITION: &str = slots::GAME;

/// Label of the launcher slot that started the game.
const KEY_LAUNCHER: &str = "launcher";
/// SHA-256 (hex) of the image in `game_0`. NVS keys are limited to 15 characters.
const KEY_GAME_HASH: &str = "game0_sha256";
/// Flash address of `game_0` when the hash was stored; a changed partition table invalidates it.
const KEY_GAME_ADDRESS: &str = "game0_addr";
/// Launcher slot an update was written to, until the next start has checked it.
const KEY_UPDATE_TARGET: &str = "update_target";

/// What became of a launcher update, seen from the first start afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateResult {
    /// Running from the updated slot.
    Installed,
    /// The updated slot didn't confirm itself; this is the previous launcher again.
    RolledBack { slot: String },
}

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
        let address = find_app_partition(GAME_PARTITION)?.address;
        if self.nvs.get_u32(KEY_GAME_ADDRESS)? != Some(address) {
            return Ok(None);
        }
        let mut buf = [0u8; 65];
        Ok(self
            .nvs
            .get_str(KEY_GAME_HASH, &mut buf)?
            .map(str::to_owned))
    }

    /// Records the hash of the image in `game_0`; `None` while it's being replaced.
    pub fn set_installed_game_hash(&mut self, hash: Option<&str>) -> anyhow::Result<()> {
        match hash {
            Some(hash) => {
                self.nvs.set_str(KEY_GAME_HASH, hash)?;
                self.nvs.set_u32(
                    KEY_GAME_ADDRESS,
                    find_app_partition(GAME_PARTITION)?.address,
                )?;
            }
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

    /// Game side, call first thing: confirms the game slot (see the module docs) and makes the
    /// launcher the boot slot again.
    pub fn return_to_launcher_on_reset(&mut self) -> anyhow::Result<()> {
        mark_running_valid();
        let mut buf = [0u8; 17];
        // Slot A if NVS doesn't say which launcher slot started the game.
        let launcher = self
            .nvs
            .get_str(KEY_LAUNCHER, &mut buf)?
            .unwrap_or(slots::LAUNCHER_A.label)
            .to_owned();
        set_boot_partition(&launcher)
    }

    /// Launcher side, after writing an update into `slot`: makes it the boot slot and remembers it,
    /// for [`Boot::take_update_result`] after the restart.
    pub fn set_pending_update(&mut self, slot: &str) -> anyhow::Result<()> {
        set_boot_partition(slot)?;
        self.nvs.set_str(KEY_UPDATE_TARGET, slot)?;
        Ok(())
    }

    /// Launcher side, once per start: whether a launcher update was pending, and how it went.
    pub fn take_update_result(&mut self) -> anyhow::Result<Option<UpdateResult>> {
        let mut buf = [0u8; 17];
        let Some(target) = self
            .nvs
            .get_str(KEY_UPDATE_TARGET, &mut buf)?
            .map(str::to_owned)
        else {
            return Ok(None);
        };
        self.nvs.remove(KEY_UPDATE_TARGET)?;
        Ok(Some(if running_partition_label() == target {
            UpdateResult::Installed
        } else {
            UpdateResult::RolledBack { slot: target }
        }))
    }
}

/// Confirms that the running slot works, so the bootloader keeps it (see the module docs).
/// Does nothing if it was already confirmed or isn't an OTA slot.
pub fn mark_running_valid() {
    let result = esp!(unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() });
    if let Err(e) = result {
        log::warn!("Can't confirm the running slot: {e:?}");
    }
}

/// The launcher slot that isn't running, where an update goes. From a non-launcher slot (e.g.
/// recovery), slot A.
pub fn inactive_launcher_slot() -> LauncherSlot {
    slots::other_launcher_slot(&running_partition_label())
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
    unsafe { CStr::from_ptr((*partition).label.as_ptr()) }
        .to_string_lossy()
        .into_owned()
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

/// Makes the app slot `label` boot on the next restart. Fails if it holds no valid image.
pub fn set_boot_partition(label: &str) -> anyhow::Result<()> {
    let partition = find_app_partition(label)?;
    esp!(unsafe { sys::esp_ota_set_boot_partition(partition) })
        .with_context(|| format!("can't make {label} the boot partition"))
}
