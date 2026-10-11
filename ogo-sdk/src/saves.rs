//! Save files under `/ogo/saves/<game-id>/` on the SD card, one file per name.
//!
//! Values are stored as text, so they're easy to inspect on a PC. For anything more complex than
//! a number or a string, serialize it yourself and use [`Saves::read_bytes`] / [`Saves::write_bytes`].

use std::{
    fmt::Display,
    fs,
    path::PathBuf,
    str::FromStr,
};

use anyhow::{bail, Context};
use ogo_board::SdCard;

pub struct Saves {
    /// `None` without an SD card; reads then find nothing and writes fail.
    dir: Option<PathBuf>,
}

impl Saves {
    pub(crate) fn new(sd: Option<&SdCard>, game_id: &str) -> Self {
        Self { dir: sd.map(|sd| PathBuf::from(sd.path(&format!("ogo/saves/{game_id}")))) }
    }

    /// Reads and parses a value, or `None` if it was never saved or doesn't parse.
    pub fn read<T: FromStr>(&self, name: &str) -> Option<T> {
        let bytes = self.read_bytes(name)?;
        String::from_utf8(bytes).ok()?.trim().parse().ok()
    }

    /// Saves a value as text, replacing the previous one.
    pub fn write<T: Display>(&self, name: &str, value: &T) -> anyhow::Result<()> {
        self.write_bytes(name, value.to_string().as_bytes())
    }

    pub fn read_bytes(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.path(name).ok()?;
        fs::read(path).ok()
    }

    /// Writes to a temporary file first, so a crash mid-write keeps the previous save.
    pub fn write_bytes(&self, name: &str, data: &[u8]) -> anyhow::Result<()> {
        let path = self.path(name)?;
        let dir = path.parent().expect("save paths have a folder");
        fs::create_dir_all(dir).with_context(|| format!("can't create {}", dir.display()))?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, data).with_context(|| format!("can't write {}", temp.display()))?;
        // FAT can't rename over an existing file.
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&temp, &path).with_context(|| format!("can't write {}", path.display()))?;
        Ok(())
    }

    fn path(&self, name: &str) -> anyhow::Result<PathBuf> {
        let Some(dir) = &self.dir else {
            bail!("no SD card");
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            bail!("invalid save name \"{name}\" (use a-z, 0-9, - and _)");
        }
        Ok(dir.join(name))
    }
}
