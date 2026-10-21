//! `.ogo` app packages: an uncompressed tar with `app.toml`, `app.bin` and optionally
//! `icon.rgb565` (docs/CONTRACTS.md).
//!
//! A package is unpacked into a hidden folder next to the apps, checked like a library entry
//! (plus the sha256 of `app.bin` and its size), and only then moved into place, so a broken
//! upload never replaces a working app.

use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context};
use ogo_board::{
    boot::{find_app_partition, APP_PARTITION},
    flash::file_sha256,
};

use crate::library::{self, App};

/// Unpacks and checks `package`, then installs it as `<apps_dir>/<id>/`, replacing an older
/// version. Returns the app.
pub fn install(package: &Path, apps_dir: &Path) -> anyhow::Result<App> {
    fs::create_dir_all(apps_dir)?;
    let staging = apps_dir.join(".incoming");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let result = unpack(package, &staging).and_then(|()| check(&staging));
    let app = match result {
        Ok(app) => app,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    let target = apps_dir.join(&app.id);
    if target.exists() {
        fs::remove_dir_all(&target).with_context(|| format!("can't remove the old {}", app.id))?;
    }
    fs::rename(&staging, &target).with_context(|| format!("can't move {} into place", app.id))?;
    log::info!("Installed package {} {}", app.id, app.version);
    library::load_app(&target, Some(&app.id)).map_err(|e| anyhow!(e))
}

fn check(dir: &Path) -> anyhow::Result<App> {
    let app = library::load_app(dir, None).map_err(|e| anyhow!(e))?;
    let bin = dir.join("app.bin");
    let size = fs::metadata(&bin)?.len();
    let slot = find_app_partition(APP_PARTITION)?.size;
    if size > u64::from(slot) {
        bail!(
            "app.bin is {} KB, the app slot holds {} KB",
            size / 1024,
            slot / 1024
        );
    }
    if file_sha256(&bin)? != app.sha256 {
        bail!("app.bin doesn't match the sha256 in app.toml");
    }
    Ok(app)
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
