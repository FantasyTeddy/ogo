//! ILI9341 LCD, 320x240 landscape, on the shared SPI bus.

use anyhow::anyhow;
use esp_idf_svc::hal::{
    delay::Ets,
    gpio::{Output, OutputPin, PinDriver},
    ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver, LowSpeed, Resolution, CHANNEL0, TIMER0},
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

/// LCD backlight, dimmed by PWM (LEDC).
pub struct Backlight {
    channel: LedcDriver<'static>,
    // Dropping the timer driver would stop the timer.
    _timer: LedcTimerDriver<'static, LowSpeed>,
}

/// High enough not to flicker, low enough for 13-bit steps from the 80 MHz clock.
const BACKLIGHT_PWM_HZ: u32 = 5_000;
/// Duty cycle at the lowest brightness: dim, but the screen stays readable.
const MIN_DUTY: f32 = 0.04;

impl Backlight {
    /// Starts dark; [`Backlight::set_brightness`] switches it on.
    pub(crate) fn new(
        timer: TIMER0<'static>,
        channel: CHANNEL0<'static>,
        pin: impl OutputPin + 'static,
    ) -> anyhow::Result<Self> {
        let config = TimerConfig::new().frequency(BACKLIGHT_PWM_HZ.Hz()).resolution(Resolution::Bits13);
        let timer = LedcTimerDriver::new(timer, &config)?;
        let mut channel = LedcDriver::new(channel, &timer, pin)?;
        channel.set_duty(0)?;
        Ok(Self { channel, _timer: timer })
    }

    /// Brightness in percent; 0 switches the backlight off. The eye sees brightness roughly as
    /// the square root of the duty cycle, so the duty grows quadratically, from [`MIN_DUTY`] at
    /// 1 % up to 100 %.
    pub fn set_brightness(&mut self, percent: u8) -> anyhow::Result<()> {
        let fraction = match percent.min(100) {
            0 => 0.0,
            p => MIN_DUTY + (1.0 - MIN_DUTY) * (f32::from(p) / 100.0).powi(2),
        };
        let max = self.channel.get_max_duty();
        self.channel.set_duty((max as f32 * fraction).round() as u32)?;
        Ok(())
    }
}
