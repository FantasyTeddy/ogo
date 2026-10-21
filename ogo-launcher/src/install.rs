//! Installs an app's `app.bin` into the `app_0` flash slot.
//!
//! The image is checked against `sha256` from `app.toml` while it's written. If the slot already
//! holds that image, nothing is written.

use ogo_board::{
    boot::{Boot, APP_PARTITION},
    flash::{self, Progress},
};

use crate::library::App;

/// Makes sure `app_0` holds `app`'s image. Returns `false` if it already did.
pub fn install(app: &App, boot: &mut Boot, progress: impl FnMut(Progress)) -> anyhow::Result<bool> {
    if boot.installed_app_hash()?.as_deref() == Some(app.sha256.as_str()) {
        log::info!("{} is already installed", app.id);
        return Ok(false);
    }
    // From here on the slot no longer holds the old image.
    boot.set_installed_app_hash(None)?;
    flash::write_image(
        &app.dir.join("app.bin"),
        APP_PARTITION,
        Some(&app.sha256),
        progress,
    )?;
    boot.set_installed_app_hash(Some(&app.sha256))?;
    log::info!("Installed {}", app.id);
    Ok(true)
}
