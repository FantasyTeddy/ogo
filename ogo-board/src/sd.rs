//! microSD card on the shared SPI bus, mounted as FAT at [`MOUNT_POINT`].
//!
//! Use `std::fs` with paths under the mount point. The ESP-IDF SPI master driver serializes
//! transactions per bus, so SD and LCD access from the same or different tasks don't collide.

use esp_idf_svc::{
    fs::fatfs::Fatfs,
    hal::{
        gpio::{AnyIOPin, OutputPin},
        sd::{spi::SdSpiHostDriver, SdCardConfiguration, SdCardDriver},
        spi::SpiDriver,
    },
    io::vfs::MountedFatfs,
};

pub const MOUNT_POINT: &str = "/sd";

/// Total and free bytes on the mounted card, or `None` if no card is mounted.
pub fn space() -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(MOUNT_POINT).ok()?;
    let (mut total, mut free) = (0u64, 0u64);
    esp_idf_svc::sys::esp!(unsafe { esp_idf_svc::sys::esp_vfs_fat_info(path.as_ptr(), &mut total, &mut free) }).ok()?;
    Some((total, free))
}

/// Files that can be open at the same time.
const MAX_OPEN_FILES: usize = 8;

type CardDriver = SdCardDriver<SdSpiHostDriver<'static, &'static SpiDriver<'static>>>;

pub struct SdCard {
    _mounted: MountedFatfs<Fatfs<CardDriver>>,
    capacity_bytes: u64,
}

impl SdCard {
    pub(crate) fn mount(
        spi: &'static SpiDriver<'static>,
        cs: impl OutputPin + 'static,
    ) -> anyhow::Result<Self> {
        let host = SdSpiHostDriver::new(
            spi,
            Some(cs),
            AnyIOPin::none(),
            AnyIOPin::none(),
            AnyIOPin::none(),
            None,
        )?;
        let card = SdCardDriver::new_spi(host, &SdCardConfiguration::new())?;
        let csd = &card.card().csd;
        let capacity_bytes = csd.capacity as u64 * csd.sector_size as u64;
        let mounted = MountedFatfs::mount(Fatfs::new_sdcard(0, card)?, MOUNT_POINT, MAX_OPEN_FILES)?;
        Ok(Self { _mounted: mounted, capacity_bytes })
    }

    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    /// Absolute path for a path relative to the card root.
    pub fn path(&self, relative: &str) -> String {
        format!("{MOUNT_POINT}/{}", relative.trim_start_matches('/'))
    }
}
