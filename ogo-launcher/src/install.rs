//! Installs a game's `game.bin` into the `game_0` flash slot.
//!
//! The image is checked against `sha256` from `game.toml` while it's written. If the slot already
//! holds that image, nothing is written.

use ogo_board::{
    boot::{Boot, GAME_PARTITION},
    flash::{self, Progress},
};

use crate::library::Game;

/// Makes sure `game_0` holds `game`'s image. Returns `false` if it already did.
pub fn install(game: &Game, boot: &mut Boot, progress: impl FnMut(Progress)) -> anyhow::Result<bool> {
    if boot.installed_game_hash()?.as_deref() == Some(game.sha256.as_str()) {
        log::info!("{} is already installed", game.id);
        return Ok(false);
    }
    // From here on the slot no longer holds the old image.
    boot.set_installed_game_hash(None)?;
    flash::write_image(&game.dir.join("game.bin"), GAME_PARTITION, Some(&game.sha256), progress)?;
    boot.set_installed_game_hash(Some(&game.sha256))?;
    log::info!("Installed {}", game.id);
    Ok(true)
}
