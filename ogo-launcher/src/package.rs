//! `.ogo` game packages: an uncompressed tar with `game.toml`, `game.bin` and optionally
//! `icon.rgb565` (docs/CONTRACTS.md).
//!
//! A package is unpacked into a hidden folder next to the games, checked like a library entry
//! (plus the sha256 of `game.bin` and its size), and only then moved into place, so a broken
//! upload never replaces a working game.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context};
use ogo_board::{
    boot::{find_app_partition, GAME_PARTITION},
    flash::file_sha256,
};

use crate::library::{self, Game};

/// Files a package may contain; anything else is skipped.
const PACKAGE_FILES: [&str; 3] = ["game.toml", "game.bin", "icon.rgb565"];
const BLOCK: usize = 512;

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

/// Minimal tar reader: regular files only, names from the 100-byte name field (enough for the
/// flat packages `cargo xtask pack` writes).
fn unpack(package: &Path, into: &Path) -> anyhow::Result<()> {
    let mut file = File::open(package).with_context(|| format!("can't open {}", package.display()))?;
    let mut header = [0u8; BLOCK];
    let mut found = Vec::new();
    loop {
        if read_block(&mut file, &mut header)?.is_none() || header.iter().all(|&b| b == 0) {
            break;
        }
        let name = field_str(&header[0..100]);
        let size = octal(&header[124..136]).context("not a valid .ogo package (bad tar header)")?;
        let kind = header[156];
        // Name without leading "./" or folders.
        let name = name.rsplit('/').next().unwrap_or_default().to_owned();
        let regular = kind == b'0' || kind == 0;

        if regular && PACKAGE_FILES.contains(&name.as_str()) {
            let mut out = File::create(into.join(&name))?;
            io::copy(&mut (&mut file).take(size), &mut out).context("not a valid .ogo package (truncated)")?;
            found.push(name);
        } else {
            io::copy(&mut (&mut file).take(size), &mut io::sink())?;
        }
        // Data is padded to whole blocks.
        let padding = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
        io::copy(&mut (&mut file).take(padding), &mut io::sink())?;
    }
    for required in ["game.toml", "game.bin"] {
        if !found.iter().any(|name| name == required) {
            bail!("the package has no {required}");
        }
    }
    Ok(())
}

/// Reads one block; `None` at the end of the file.
fn read_block(file: &mut File, block: &mut [u8; BLOCK]) -> anyhow::Result<Option<()>> {
    let mut filled = 0;
    while filled < BLOCK {
        let count = file.read(&mut block[filled..])?;
        if count == 0 {
            if filled == 0 {
                return Ok(None);
            }
            bail!("not a valid .ogo package (truncated)");
        }
        filled += count;
    }
    Ok(Some(()))
}

fn field_str(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn octal(field: &[u8]) -> Option<u64> {
    let text = field_str(field);
    u64::from_str_radix(text.trim(), 8).ok()
}

/// Where uploads are stored while they arrive.
pub fn upload_path(sd_root: &Path) -> PathBuf {
    sd_root.join("ogo").join("updates").join("upload.ogo")
}
