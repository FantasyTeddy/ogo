//! ILI9341 LCD, 320x240 landscape, on the shared SPI bus.

use anyhow::anyhow;
use esp_idf_svc::hal::{
    delay::Ets,
    delay::BLOCK,
    gpio::{Output, OutputPin, PinDriver},
    ledc::{
        config::TimerConfig, LedcDriver, LedcTimerDriver, LowSpeed, Resolution, CHANNEL0, TIMER0,
    },
    spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriver},
    sys::{
        esp, spi_device_get_trans_result, spi_device_handle_t, spi_device_queue_trans,
        spi_transaction_t, EspError,
    },
    units::FromValueType,
};
use mipidsi::{
    interface::{Interface, InterfaceKind},
    models::ILI9341Rgb565,
    options::{ColorOrder, Orientation, Rotation},
    Builder, NoResetPin,
};

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

/// Size of the buffer pixels are batched in before an SPI transfer.
const BUFFER_SIZE: usize = 4096;

pub type Display = mipidsi::Display<DisplaySpi, ILI9341Rgb565, NoResetPin>;

/// The panel's SPI connection, for mipidsi: what its `SpiInterface` does, plus
/// [`DisplaySpi::write_with`], which lets the caller fill the transfer buffer with pixel bytes
/// directly. mipidsi's own way takes the pixels one by one through an iterator, which costs as
/// much time as the transfer itself.
pub struct DisplaySpi {
    spi: SpiDeviceDriver<'static, &'static SpiDriver<'static>>,
    /// Data/command select: low for a command byte, high for its parameters and pixel data.
    dc: PinDriver<'static, Output>,
    /// Two, so one can be filled while the other is sent.
    buffers: [&'static mut [u8; BUFFER_SIZE]; 2],
}

impl DisplaySpi {
    /// Sends pixel data after a memory write was started (`set_pixels` with no pixels): `fill`
    /// writes the next bytes (RGB565, big-endian) into the buffer it gets and returns how many;
    /// 0 ends it. While the SPI hardware sends one buffer, `fill` already fills the other, so a
    /// picture takes about as long as the slower of the two rather than both together.
    pub fn write_with(&mut self, mut fill: impl FnMut(&mut [u8]) -> usize) -> Result<(), EspError> {
        let device = self.spi.device();
        // Must stay where they are until their transfer is done: the driver keeps pointers
        let mut transfers = [spi_transaction_t::default(); 2];
        let mut queued = [false; 2];
        let mut result = Ok(());
        let mut i = 0;
        loop {
            // The older of the two transfers is this buffer's
            if queued[i] {
                queued[i] = false;
                if let Err(e) = wait_for_transfer(device) {
                    result = Err(e);
                    break;
                }
            }
            let len = fill(&mut self.buffers[i][..]);
            if len == 0 {
                break;
            }
            transfers[i] = spi_transaction_t {
                length: len * 8,
                ..Default::default()
            };
            transfers[i].__bindgen_anon_1.tx_buffer = self.buffers[i].as_ptr().cast();
            if let Err(e) =
                esp!(unsafe { spi_device_queue_trans(device, &mut transfers[i], BLOCK) })
            {
                result = Err(e);
                break;
            }
            queued[i] = true;
            i = 1 - i;
        }
        // Whatever is still on its way, also after an error: the buffers and transfers must not
        // be touched before
        for _ in queued.iter().filter(|&&q| q) {
            let done = wait_for_transfer(device);
            result = result.and(done);
        }
        result
    }
}

fn wait_for_transfer(device: spi_device_handle_t) -> Result<(), EspError> {
    let mut done = core::ptr::null_mut();
    esp!(unsafe { spi_device_get_trans_result(device, &mut done, BLOCK) })
}

impl Interface for DisplaySpi {
    type Word = u8;
    type Error = EspError;

    const KIND: InterfaceKind = InterfaceKind::Serial4Line;

    fn send_command(&mut self, command: u8, args: &[u8]) -> Result<(), EspError> {
        self.dc.set_low()?;
        self.spi.write(&[command])?;
        self.dc.set_high()?;
        if !args.is_empty() {
            self.spi.write(args)?;
        }
        Ok(())
    }

    fn send_pixels<const N: usize>(
        &mut self,
        pixels: impl IntoIterator<Item = [u8; N]>,
    ) -> Result<(), EspError> {
        let mut pixels = pixels.into_iter();
        self.write_with(|buffer| {
            let mut len = 0;
            for chunk in buffer.chunks_exact_mut(N) {
                let Some(pixel) = pixels.next() else {
                    break;
                };
                chunk.copy_from_slice(&pixel);
                len += N;
            }
            len
        })
    }

    fn send_repeated_pixel<const N: usize>(
        &mut self,
        pixel: [u8; N],
        count: u32,
    ) -> Result<(), EspError> {
        // The buffer is filled once and sent as often as needed
        let per_buffer = (BUFFER_SIZE / N).min(count as usize);
        for chunk in self.buffers[0][..per_buffer * N].chunks_exact_mut(N) {
            chunk.copy_from_slice(&pixel);
        }
        let mut left = count as usize;
        while left > 0 {
            let now = left.min(per_buffer);
            self.spi.write(&self.buffers[0][..now * N])?;
            left -= now;
        }
        Ok(())
    }
}

pub(crate) fn init(
    spi: &'static SpiDriver<'static>,
    cs: impl OutputPin + 'static,
    dc: impl OutputPin + 'static,
) -> anyhow::Result<Display> {
    // Queued (interrupt-driven) transfers, two at a time: see `DisplaySpi::write_with`
    let config = SpiConfig::new()
        .baudrate(40.MHz().into())
        .polling(false)
        .queue_size(2);
    let device = SpiDeviceDriver::new(spi, Some(cs), &config)?;
    let dc = PinDriver::output(dc)?;
    let buffers = [word_aligned_buffer(), word_aligned_buffer()];

    // The ILI9341 is portrait natively; the Odroid-Go holds it landscape (320x240).
    // The panel is mounted mirrored, so rotating alone shows the image flipped left to right.
    // No reset pin is wired, so mipidsi falls back to a software reset.
    let interface = DisplaySpi {
        spi: device,
        dc,
        buffers,
    };
    Builder::new(ILI9341Rgb565, interface)
        .orientation(Orientation::new().rotate(Rotation::Deg90).flip_horizontal())
        .color_order(ColorOrder::Bgr)
        .init(&mut Ets)
        .map_err(|e| anyhow!("display init failed: {e:?}"))
}

/// A transfer buffer aligned to 32 bits, so pixels can be written into it two at a time.
fn word_aligned_buffer() -> &'static mut [u8; BUFFER_SIZE] {
    let words: &'static mut [u32; BUFFER_SIZE / 4] = Box::leak(Box::new([0; BUFFER_SIZE / 4]));
    // SAFETY: the same memory as bytes: same size, and bytes need no alignment
    unsafe { &mut *(words as *mut [u32; BUFFER_SIZE / 4]).cast::<[u8; BUFFER_SIZE]>() }
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
        let config = TimerConfig::new()
            .frequency(BACKLIGHT_PWM_HZ.Hz())
            .resolution(Resolution::Bits13);
        let timer = LedcTimerDriver::new(timer, &config)?;
        let mut channel = LedcDriver::new(channel, &timer, pin)?;
        channel.set_duty(0)?;
        Ok(Self {
            channel,
            _timer: timer,
        })
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
        self.channel
            .set_duty((max as f32 * fraction).round() as u32)?;
        Ok(())
    }
}
