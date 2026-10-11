//! Installs a game's `game.bin` into the `game_0` flash slot.
//!
//! The image is streamed from the SD card, hashed on the way and checked against `sha256` from
//! `game.toml` before the slot is marked as holding it. If the slot already holds that image,
//! nothing is written.

use std::{fs::File, io::Read};

use anyhow::{bail, Context};
use esp_idf_svc::sys::{self, esp};
use ogo_board::boot::{self, Boot, GAME_PARTITION};
use sha2::{Digest, Sha256};

use crate::library::Game;

const CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Making room in the flash slot; can take a few seconds for large games.
    Erasing,
    Writing { written: u64, total: u64 },
}

/// Makes sure `game_0` holds `game`'s image. Returns `false` if it already did.
pub fn install(game: &Game, boot: &mut Boot, mut progress: impl FnMut(Progress)) -> anyhow::Result<bool> {
    if boot.installed_game_hash()?.as_deref() == Some(game.sha256.as_str()) {
        log::info!("{} is already installed", game.id);
        return Ok(false);
    }

    let path = game.dir.join("game.bin");
    let mut file = File::open(&path).with_context(|| format!("can't open {}", path.display()))?;
    let total = file.metadata()?.len();
    let partition = boot::find_app_partition(GAME_PARTITION)?;
    if total > u64::from(partition.size) {
        bail!("game.bin is {} KB, the game slot holds {} KB", total / 1024, partition.size / 1024);
    }

    // From here on the slot no longer holds the old image.
    boot.set_installed_game_hash(None)?;
    log::info!("Installing {} ({total} bytes) into {GAME_PARTITION}", game.id);

    progress(Progress::Erasing);
    let mut handle: sys::esp_ota_handle_t = 0;
    // Passing the size erases only as much of the slot as the image needs.
    esp!(unsafe { sys::esp_ota_begin(partition, total as usize, &mut handle) }).context("can't prepare the game slot")?;

    let result = write_image(&mut file, handle, total, &game.sha256, &mut progress);
    if let Err(e) = result {
        unsafe { sys::esp_ota_abort(handle) };
        return Err(e);
    }
    // Checks the image format and its checksums.
    esp!(unsafe { sys::esp_ota_end(handle) }).context("game.bin is not a valid firmware image")?;

    boot.set_installed_game_hash(Some(&game.sha256))?;
    log::info!("Installed {}", game.id);
    Ok(true)
}

fn write_image(
    file: &mut File,
    handle: sys::esp_ota_handle_t,
    total: u64,
    expected_sha256: &str,
    progress: &mut impl FnMut(Progress),
) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut written = 0u64;
    loop {
        let count = file.read(&mut buffer).context("reading game.bin failed")?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        esp!(unsafe { sys::esp_ota_write(handle, buffer.as_ptr().cast(), count) }).context("writing the game slot failed")?;
        written += count as u64;
        progress(Progress::Writing { written, total });
    }

    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        bail!("game.bin doesn't match the sha256 in game.toml");
    }
    Ok(())
}
