//! Writing firmware images into app slots (from files or streams), and reading what a slot holds.

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
    Writing {
        written: u64,
        total: u64,
    },
}

/// Writes the firmware image at `path` into the app slot `label`, streaming it in chunks.
/// With `expected_sha256` (lowercase hex of the whole file), a mismatch fails the write.
/// The slot is only usable if this returns `Ok`.
pub fn write_image(
    path: &Path,
    label: &str,
    expected_sha256: Option<&str>,
    progress: impl FnMut(Progress),
) -> anyhow::Result<()> {
    let mut file = File::open(path).with_context(|| format!("can't open {}", path.display()))?;
    let total = file.metadata()?.len();
    log::info!("Writing {} into {label}", path.display());
    write_stream(
        |buf| Ok(file.read(buf)?),
        total,
        label,
        expected_sha256,
        progress,
    )
}

/// Like [`write_image`], but takes the image from `read` (which returns 0 at the end), e.g. an
/// HTTP upload. `total` is the image size.
pub fn write_stream(
    mut read: impl FnMut(&mut [u8]) -> anyhow::Result<usize>,
    total: u64,
    label: &str,
    expected_sha256: Option<&str>,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<()> {
    let partition = find_app_partition(label)?;
    if total > u64::from(partition.size) {
        bail!(
            "the image is {} KB, the {label} slot holds {} KB",
            total / 1024,
            partition.size / 1024
        );
    }
    log::info!("Writing {total} bytes into {label}");

    progress(Progress::Erasing);
    let mut handle: sys::esp_ota_handle_t = 0;
    // Passing the size erases only as much of the slot as the image needs.
    esp!(unsafe { sys::esp_ota_begin(partition, total as usize, &mut handle) })
        .with_context(|| format!("can't prepare the {label} slot"))?;

    if let Err(e) = write_chunks(&mut read, handle, total, expected_sha256, &mut progress) {
        unsafe { sys::esp_ota_abort(handle) };
        return Err(e);
    }
    // Checks the image format and its checksums.
    esp!(unsafe { sys::esp_ota_end(handle) }).context("not a valid firmware image")?;
    log::info!("Wrote {label}");
    Ok(())
}

fn write_chunks(
    read: &mut impl FnMut(&mut [u8]) -> anyhow::Result<usize>,
    handle: sys::esp_ota_handle_t,
    total: u64,
    expected_sha256: Option<&str>,
    progress: &mut impl FnMut(Progress),
) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut written = 0u64;
    while written < total {
        let wanted = buffer.len().min((total - written) as usize);
        let count = read(&mut buffer[..wanted]).context("reading the image failed")?;
        if count == 0 {
            bail!("the image ended after {written} of {total} bytes");
        }
        hasher.update(&buffer[..count]);
        esp!(unsafe { sys::esp_ota_write(handle, buffer.as_ptr().cast(), count) })
            .context("writing flash failed")?;
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

/// SHA-256 of the image in an app slot (reads the whole image; ~0.1 s for 1.6 MB).
fn image_sha256(partition: &sys::esp_partition_t) -> Option<[u8; 32]> {
    let mut sha = [0u8; 32];
    esp!(unsafe { sys::esp_partition_get_sha256(partition, sha.as_mut_ptr()) }).ok()?;
    Some(sha)
}

/// SHA-256 of a file as lowercase hex, read in chunks.
pub fn file_sha256(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path).with_context(|| format!("can't open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

/// Version and build time of the firmware in an app slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub project: String,
    pub version: String,
    /// Build date and time, e.g. "Oct 11 2026 17:22:05".
    pub built: String,
    /// The first 8 hex digits of the image's SHA-256, computed from the slot: differs whenever
    /// the code differs, even between uncommitted builds of the same version. (ESP-IDF's own
    /// `app_elf_sha256` field stays empty in images built with espflash.)
    pub build_id: String,
}

/// What the app slot `label` holds, or `None` if it holds no valid image.
pub fn app_info(label: &str) -> Option<AppInfo> {
    let partition = find_app_partition(label).ok()?;
    let mut desc = sys::esp_app_desc_t::default();
    esp!(unsafe { sys::esp_ota_get_partition_description(partition, &mut desc) }).ok()?;
    let text = |chars: &[core::ffi::c_char]| {
        unsafe { CStr::from_ptr(chars.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_owned()
    };
    Some(AppInfo {
        project: text(&desc.project_name),
        version: text(&desc.version),
        built: format!("{} {}", text(&desc.date), text(&desc.time)),
        build_id: image_sha256(partition).map_or_else(
            || "?".into(),
            |sha| sha[..4].iter().map(|b| format!("{b:02x}")).collect(),
        ),
    })
}
