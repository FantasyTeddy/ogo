//! ILI9341 LCD, 320x240 landscape, on the shared SPI bus.

use anyhow::anyhow;
use esp_idf_svc::hal::{
    delay::Ets,
    gpio::{Output, OutputPin, PinDriver},
    spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriver},
    units::FromValueType,
};
use mipidsi::{
    interface::SpiInterface,
    models::ILI9341Rgb565,
    options::{ColorOrder, Orientation, Rotation},
    Builder, NoResetPin,
};

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

/// Size of the buffer mipidsi batches pixels in before an SPI transfer.
const BUFFER_SIZE: usize = 4096;

pub type Display = mipidsi::Display<
    SpiInterface<'static, SpiDeviceDriver<'static, &'static SpiDriver<'static>>, PinDriver<'static, Output>>,
    ILI9341Rgb565,
    NoResetPin,
>;

pub(crate) fn init(
    spi: &'static SpiDriver<'static>,
    cs: impl OutputPin + 'static,
    dc: impl OutputPin + 'static,
) -> anyhow::Result<Display> {
    let device = SpiDeviceDriver::new(spi, Some(cs), &SpiConfig::new().baudrate(40.MHz().into()))?;
    let dc = PinDriver::output(dc)?;
    let buffer = Box::leak(Box::new([0u8; BUFFER_SIZE]));

    // The ILI9341 is portrait natively; the Odroid-Go holds it landscape (320x240).
    // The panel is mounted mirrored, so rotating alone shows the image flipped left to right.
    // No reset pin is wired, so mipidsi falls back to a software reset.
    Builder::new(ILI9341Rgb565, SpiInterface::new(device, dc, buffer))
        .orientation(Orientation::new().rotate(Rotation::Deg90).flip_horizontal())
        .color_order(ColorOrder::Bgr)
        .init(&mut Ets)
        .map_err(|e| anyhow!("display init failed: {e:?}"))
}

/// mipidsi's errors don't implement `std::error::Error`, so `?` can't turn them into
/// `anyhow::Error`. Call `.display_err()?` on drawing results instead.
pub trait DisplayResultExt<T> {
    fn display_err(self) -> anyhow::Result<T>;
}

impl<T, E: core::fmt::Debug> DisplayResultExt<T> for Result<T, E> {
    fn display_err(self) -> anyhow::Result<T> {
        self.map_err(|e| anyhow!("display error: {e:?}"))
    }
}

/// LCD backlight. On/off only for now; brightness via PWM comes with the settings screen (M8).
pub struct Backlight {
    pin: PinDriver<'static, Output>,
}

impl Backlight {
    pub(crate) fn new(pin: impl OutputPin + 'static) -> anyhow::Result<Self> {
        Ok(Self { pin: PinDriver::output(pin)? })
    }

    pub fn set_on(&mut self, on: bool) -> anyhow::Result<()> {
        if on {
            self.pin.set_high()?;
        } else {
            self.pin.set_low()?;
        }
        Ok(())
    }
}
