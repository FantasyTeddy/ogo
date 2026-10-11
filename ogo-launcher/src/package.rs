//! `.ogo` game packages: an uncompressed tar with `game.toml`, `game.bin` and optionally
//! `icon.rgb565` (docs/CONTRACTS.md).
//!
//! A package is unpacked into a hidden folder next to the games, checked like a library entry
//! (plus the sha256 of `game.bin` and its size), and only then moved into place, so a broken
//! upload never replaces a working game.

use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context};
use ogo_board::{
    boot::{find_app_partition, GAME_PARTITION},
    flash::file_sha256,
};

use crate::library::{self, Game};

/// Unpacks and checks `package`, then installs it as `<games_dir>/<id>/`, replacing an older
/// version. Returns the game.
pub fn install(package: &Path, games_dir: &Path) -> anyhow::Result<Game> {
    fs::create_dir_all(games_dir)?;
    let staging = games_dir.join(".incoming");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let result = unpack(package, &staging).and_then(|()| check(&staging));
    let game = match result {
        Ok(game) => game,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    let target = games_dir.join(&game.id);
    if target.exists() {
        fs::remove_dir_all(&target).with_context(|| format!("can't remove the old {}", game.id))?;
    }
    fs::rename(&staging, &target).with_context(|| format!("can't move {} into place", game.id))?;
    log::info!("Installed package {} {}", game.id, game.version);
    library::load_game(&target, Some(&game.id)).map_err(|e| anyhow!(e))
}

fn check(dir: &Path) -> anyhow::Result<Game> {
    let game = library::load_game(dir, None).map_err(|e| anyhow!(e))?;
    let bin = dir.join("game.bin");
    let size = fs::metadata(&bin)?.len();
    let slot = find_app_partition(GAME_PARTITION)?.size;
    if size > u64::from(slot) {
        bail!("game.bin is {} KB, the game slot holds {} KB", size / 1024, slot / 1024);
    }
    if file_sha256(&bin)? != game.sha256 {
        bail!("game.bin doesn't match the sha256 in game.toml");
    }
    Ok(game)
}

/// Unpacks the package files into `into`.
fn unpack(package: &Path, into: &Path) -> anyhow::Result<()> {
    let file = File::open(package).with_context(|| format!("can't open {}", package.display()))?;
    // 4 KB, like the upload chunks: this runs while WiFi needs internal RAM too.
    ogo_format::package::read(io::BufReader::with_capacity(4096, file), |name, data| {
        io::copy(data, &mut File::create(into.join(name))?).map(|_| ())
    })?;
    Ok(())
}

/// Where uploads are stored while they arrive.
pub fn upload_path(sd_root: &Path) -> PathBuf {
    sd_root.join("ogo").join("updates").join("upload.ogo")
}
