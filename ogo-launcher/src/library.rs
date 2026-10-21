//! The app library: one folder per app under `/ogo/apps/` on the SD card.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub use ogo_format::app::ICON_SIZE;
use ogo_format::app::{Folder, Kind, Manifest, ICON_BYTES, SDK_VERSION};

/// Folder with the installed apps, relative to the SD card root.
pub const APPS_DIR: &str = "ogo/apps";

#[derive(Debug, Clone)]
pub struct App {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub version: String,
    pub author: String,
    /// SHA-256 of `app.bin`, lowercase hex.
    pub sha256: String,
    pub dir: PathBuf,
    pub has_icon: bool,
    /// Folders on the SD card the app reads files from (`app.toml`).
    pub folders: Vec<Folder>,
}

impl App {
    /// Reads the icon, or `None` if the app has none.
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
    App(App),
    Broken(Broken),
}

impl Entry {
    pub fn title(&self) -> &str {
        match self {
            Entry::App(app) => &app.name,
            Entry::Broken(broken) => &broken.folder,
        }
    }

    /// The section of the list it's in: its kind, or `None` for one that can't be used.
    pub fn kind(&self) -> Option<Kind> {
        match self {
            Entry::App(app) => Some(app.kind),
            Entry::Broken(_) => None,
        }
    }
}

#[derive(Debug)]
pub enum Library {
    NoSdCard,
    /// The card has no `/ogo/apps` folder.
    NoAppsDir,
    /// Sorted by kind (those that can't be used last), then by title ignoring case. Can be empty.
    Entries(Vec<Entry>),
}

impl Library {
    /// The entries; none without a card or library folder.
    pub fn entries(&self) -> &[Entry] {
        match self {
            Library::Entries(entries) => entries,
            Library::NoSdCard | Library::NoAppsDir => &[],
        }
    }
}

pub fn scan(sd: Option<&ogo_board::SdCard>) -> Library {
    let Some(sd) = sd else {
        return Library::NoSdCard;
    };
    match scan_dir(&PathBuf::from(sd.path(APPS_DIR))) {
        Some(entries) => Library::Entries(entries),
        None => Library::NoAppsDir,
    }
}

/// The entries in `apps_dir`, sorted like [`Library::Entries`]; `None` if the folder can't be read.
pub fn scan_dir(apps_dir: &Path) -> Option<Vec<Entry>> {
    let folders = match fs::read_dir(apps_dir) {
        Ok(folders) => folders,
        Err(e) => {
            log::info!("No app library at {}: {e}", apps_dir.display());
            return None;
        }
    };

    let mut entries: Vec<Entry> = folders
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        // Folders starting with a dot are work in progress, e.g. an upload being unpacked.
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| {
            let folder = entry.file_name().to_string_lossy().into_owned();
            match load_app(&entry.path(), Some(&folder)) {
                Ok(app) => Entry::App(app),
                Err(reason) => {
                    log::warn!("Skipping app folder {folder}: {reason}");
                    Entry::Broken(Broken { folder, reason })
                }
            }
        })
        .collect();
    entries.sort_by_key(|entry| {
        let kind = entry.kind();
        (kind.is_none(), kind, entry.title().to_lowercase())
    });
    log::info!("Found {} entries in {}", entries.len(), apps_dir.display());
    Some(entries)
}

/// Stack for parsing `app.toml`, see [`parse_manifest`]: twice what GB's needed (4.8 KB).
const PARSE_STACK_BYTES: usize = 10 * 1024;

/// Parses `app.toml` on a thread of its own. The toml deserializer recurses for every level of
/// nesting (`[[folders]]` with its `extensions` adds several), which overflowed the main task's
/// 8 KB stack under the launcher's own frames; the HTTP server's task calls this too.
fn parse_manifest(text: &str) -> Result<Manifest, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(PARSE_STACK_BYTES)
            .spawn_scoped(scope, || Manifest::parse(text))
            .map_err(|e| format!("app.toml: can't start the parser: {e}"))?
            .join()
            .map_err(|_| "app.toml: the parser crashed".to_owned())?
    })
}

/// Reads and checks `dir/app.toml`. With `folder`, the id must match it.
pub fn load_app(dir: &Path, folder: Option<&str>) -> Result<App, String> {
    let text = fs::read_to_string(dir.join("app.toml")).map_err(|e| format!("app.toml: {e}"))?;
    let manifest = parse_manifest(&text)?;
    manifest.check(folder, SDK_VERSION)?;
    if !dir.join("app.bin").is_file() {
        return Err("app.bin is missing".into());
    }

    let has_icon = match fs::metadata(dir.join("icon.rgb565")) {
        Ok(meta) if meta.len() == ICON_BYTES as u64 => true,
        Ok(meta) => {
            log::warn!(
                "{}: icon.rgb565 has {} bytes, expected {ICON_BYTES}; ignoring it",
                manifest.id,
                meta.len()
            );
            false
        }
        Err(_) => false,
    };

    Ok(App {
        id: manifest.id,
        name: manifest.name,
        kind: manifest.kind,
        version: manifest.version,
        author: manifest.author,
        sha256: manifest.sha256.to_ascii_lowercase(),
        dir: dir.to_owned(),
        has_icon,
        folders: manifest.folders,
    })
}
