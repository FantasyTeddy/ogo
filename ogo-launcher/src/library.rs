//! The game library: one folder per game under `/ogo/games/` on the SD card.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// Folder with the installed games, relative to the SD card root.
pub const GAMES_DIR: &str = "ogo/games";
/// The `sdk_version` this launcher can boot.
pub const SUPPORTED_SDK_VERSION: u32 = 1;

pub const ICON_SIZE: u32 = 64;
/// `icon.rgb565`: 64x64 pixels, RGB565 little-endian, row by row.
const ICON_BYTES: u64 = (ICON_SIZE * ICON_SIZE * 2) as u64;

/// Contents of `game.toml`.
#[derive(Debug, Deserialize)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    author: String,
    sdk_version: u32,
    sha256: String,
}

#[derive(Debug)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub dir: PathBuf,
    pub has_icon: bool,
}

impl Game {
    /// Reads the icon, or `None` if the game has none.
    pub fn load_icon(&self) -> Option<Vec<u8>> {
        if !self.has_icon {
            return None;
        }
        fs::read(self.dir.join("icon.rgb565"))
            .inspect_err(|e| log::warn!("Reading the icon of {} failed: {e}", self.id))
            .ok()
    }
}

/// A folder in the library that can't be used, with the reason.
#[derive(Debug)]
pub struct Broken {
    pub folder: String,
    pub reason: String,
}

#[derive(Debug)]
pub enum Entry {
    Game(Game),
    Broken(Broken),
}

impl Entry {
    pub fn title(&self) -> &str {
        match self {
            Entry::Game(game) => &game.name,
            Entry::Broken(broken) => &broken.folder,
        }
    }
}

#[derive(Debug)]
pub enum Library {
    NoSdCard,
    /// The card has no `/ogo/games` folder.
    NoGamesDir,
    /// Sorted by title, ignoring case. Can be empty.
    Entries(Vec<Entry>),
}

pub fn scan(sd: Option<&ogo_board::SdCard>) -> Library {
    let Some(sd) = sd else {
        return Library::NoSdCard;
    };
    let games_dir = PathBuf::from(sd.path(GAMES_DIR));
    let folders = match fs::read_dir(&games_dir) {
        Ok(folders) => folders,
        Err(e) => {
            log::info!("No game library at {}: {e}", games_dir.display());
            return Library::NoGamesDir;
        }
    };

    let mut entries: Vec<Entry> = folders
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| {
            let folder = entry.file_name().to_string_lossy().into_owned();
            match load_game(&entry.path(), &folder) {
                Ok(game) => Entry::Game(game),
                Err(reason) => {
                    log::warn!("Skipping game folder {folder}: {reason}");
                    Entry::Broken(Broken { folder, reason })
                }
            }
        })
        .collect();
    entries.sort_by_key(|entry| entry.title().to_lowercase());
    log::info!("Found {} entries in {}", entries.len(), games_dir.display());
    Library::Entries(entries)
}

fn load_game(dir: &Path, folder: &str) -> Result<Game, String> {
    let text = fs::read_to_string(dir.join("game.toml")).map_err(|e| format!("game.toml: {e}"))?;
    let manifest: Manifest = toml::from_str(&text).map_err(|e| format!("game.toml: {}", e.message()))?;

    if !is_valid_id(&manifest.id) {
        return Err(format!("invalid id \"{}\" (use a-z, 0-9 and -)", manifest.id));
    }
    if manifest.id != folder {
        return Err(format!("id \"{}\" doesn't match the folder name", manifest.id));
    }
    if manifest.sdk_version != SUPPORTED_SDK_VERSION {
        return Err(format!(
            "needs SDK version {}, this launcher supports {SUPPORTED_SDK_VERSION}",
            manifest.sdk_version
        ));
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("sha256 is not a SHA-256 hex string".into());
    }
    if !dir.join("game.bin").is_file() {
        return Err("game.bin is missing".into());
    }

    let has_icon = match fs::metadata(dir.join("icon.rgb565")) {
        Ok(meta) if meta.len() == ICON_BYTES => true,
        Ok(meta) => {
            log::warn!("{folder}: icon.rgb565 has {} bytes, expected {ICON_BYTES}; ignoring it", meta.len());
            false
        }
        Err(_) => false,
    };

    Ok(Game {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        author: manifest.author,
        dir: dir.to_owned(),
        has_icon,
    })
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
