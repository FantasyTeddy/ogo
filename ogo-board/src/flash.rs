//! Writing firmware images from files into app slots, and reading what a slot holds.

use std::{ffi::CStr, fs::File, io::Read, path::Path};

use anyhow::{bail, Context};
use esp_idf_svc::sys::{self, esp};
use sha2::{Digest, Sha256};

use crate::boot::find_app_partition;

const CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Making room in the slot; can take a few seconds for large images.
    Erasing,
    Writing { written: u64, total: u64 },
}

/// Writes the firmware image at `path` into the app slot `label`, streaming it in chunks.
/// With `expected_sha256` (lowercase hex of the whole file), a mismatch fails the write.
/// The slot is only usable if this returns `Ok`.
pub fn write_image(
    path: &Path,
    label: &str,
    expected_sha256: Option<&str>,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<()> {
    let mut file = File::open(path).with_context(|| format!("can't open {}", path.display()))?;
    let total = file.metadata()?.len();
    let partition = find_app_partition(label)?;
    if total > u64::from(partition.size) {
        bail!("the image is {} KB, the {label} slot holds {} KB", total / 1024, partition.size / 1024);
    }
    log::info!("Writing {} ({total} bytes) into {label}", path.display());

    progress(Progress::Erasing);
    let mut handle: sys::esp_ota_handle_t = 0;
    // Passing the size erases only as much of the slot as the image needs.
    esp!(unsafe { sys::esp_ota_begin(partition, total as usize, &mut handle) })
        .with_context(|| format!("can't prepare the {label} slot"))?;

    if let Err(e) = write_chunks(&mut file, handle, total, expected_sha256, &mut progress) {
        unsafe { sys::esp_ota_abort(handle) };
        return Err(e);
    }
    // Checks the image format and its checksums.
    esp!(unsafe { sys::esp_ota_end(handle) }).context("not a valid firmware image")?;
    log::info!("Wrote {label}");
    Ok(())
}

fn write_chunks(
    file: &mut File,
    handle: sys::esp_ota_handle_t,
    total: u64,
    expected_sha256: Option<&str>,
    progress: &mut impl FnMut(Progress),
) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut written = 0u64;
    loop {
        let count = file.read(&mut buffer).context("reading the image failed")?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        esp!(unsafe { sys::esp_ota_write(handle, buffer.as_ptr().cast(), count) }).context("writing flash failed")?;
        written += count as u64;
        progress(Progress::Writing { written, total });
    }

    if let Some(expected) = expected_sha256 {
        if format!("{:x}", hasher.finalize()) != expected {
            bail!("the image doesn't match its sha256");
        }
    }
    Ok(())
}

/// Version and build time of the firmware in an app slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub project: String,
    pub version: String,
    /// Build date and time, e.g. "Oct 11 2026 17:22:05".
    pub built: String,
}

/// What the app slot `label` holds, or `None` if it holds no valid image.
pub fn app_info(label: &str) -> Option<AppInfo> {
    let partition = find_app_partition(label).ok()?;
    let mut desc = sys::esp_app_desc_t::default();
    esp!(unsafe { sys::esp_ota_get_partition_description(partition, &mut desc) }).ok()?;
    let text = |chars: &[core::ffi::c_char]| {
        unsafe { CStr::from_ptr(chars.as_ptr()) }.to_string_lossy().trim().to_owned()
    };
    Some(AppInfo {
        project: text(&desc.project_name),
        version: text(&desc.version),
        built: format!("{} {}", text(&desc.date), text(&desc.time)),
    })
}
