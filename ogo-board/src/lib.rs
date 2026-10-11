//! Board support for the ODROID-GO.
//!
//! [`Board::take`] sets up all on-board hardware once. The LCD and SD card share one SPI bus
//! (SPI3 / VSPI) with separate chip selects; ESP-IDF's SPI master driver arbitrates between them.

mod adc;
pub mod battery;
pub mod buttons;
pub mod display;
pub mod pins;
pub mod sd;
pub mod speaker;

use esp_idf_svc::{
    hal::{
        adc::oneshot::AdcDriver,
        gpio::{Output, OutputPin, PinDriver},
        peripherals::Peripherals,
        spi::{Dma, SpiDriver, SpiDriverConfig},
    },
    sys,
};

pub use battery::{Battery, BatteryLevel};
pub use buttons::{Button, ButtonState, Buttons, DpadRaw};
pub use display::{Backlight, Display, DisplayResultExt};
pub use sd::SdCard;
pub use speaker::Speaker;

/// Largest single SPI transfer, in bytes. Used for LCD pixel batches and SD sectors.
const SPI_DMA_BUFFER: usize = 4096;

pub struct Board {
    pub display: Display,
    pub backlight: Backlight,
    pub buttons: Buttons,
    pub speaker: Speaker,
    pub battery: Battery,
    pub led: Led,
    /// `None` if no card is inserted or it could not be mounted.
    pub sd: Option<SdCard>,
}

impl Board {
    /// Takes the ESP32 peripherals and initializes all board hardware. Call once.
    pub fn take() -> anyhow::Result<Self> {
        let p = Peripherals::take()?;
        let pins = p.pins;

        let spi: &'static SpiDriver<'static> = Box::leak(Box::new(SpiDriver::new(
            p.spi3, // VSPI: pins 18/23/19 are its IOMUX pins, which allows 40 MHz
            pins.gpio18,
            pins.gpio23,
            Some(pins.gpio19),
            &SpiDriverConfig::new().dma(Dma::Auto(SPI_DMA_BUFFER)),
        )?));

        // Keep the SD card deselected while the LCD initializes; the SD driver takes over the pin later.
        deselect_sd_card();

        let display = display::init(spi, pins.gpio5, pins.gpio21)?;
        let mut backlight = Backlight::new(pins.gpio14)?;
        backlight.set_on(true)?;

        let sd = match SdCard::mount(spi, pins.gpio22) {
            Ok(card) => {
                log::info!("SD card mounted at {}, {} MB", sd::MOUNT_POINT, card.capacity_bytes() >> 20);
                Some(card)
            }
            Err(e) => {
                log::warn!("SD card not mounted: {e}");
                deselect_sd_card();
                None
            }
        };

        let adc: &'static AdcDriver<'static, _> = Box::leak(Box::new(AdcDriver::new(p.adc1)?));
        let buttons = Buttons::new(
            adc,
            pins.gpio32,
            pins.gpio33,
            pins.gpio27,
            pins.gpio39,
            pins.gpio13,
            pins.gpio0,
            pins.gpio34,
            pins.gpio35,
        )?;
        let battery = Battery::new(adc, pins.gpio36)?;
        let speaker = Speaker::new(pins.gpio25, pins.gpio26)?;
        let led = Led::new(pins.gpio2)?;

        Ok(Self { display, backlight, buttons, speaker, battery, led, sd })
    }
}

/// Drives SD CS high directly, so the card ignores LCD traffic when no SD driver owns the pin.
fn deselect_sd_card() {
    let pin = pins::SD_CS as i32;
    unsafe {
        sys::gpio_reset_pin(pin);
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
        sys::gpio_set_level(pin, 1);
    }
}

/// The blue status LED.
pub struct Led {
    pin: PinDriver<'static, Output>,
}

impl Led {
    fn new(pin: impl OutputPin + 'static) -> anyhow::Result<Self> {
        let mut pin = PinDriver::output(pin)?;
        pin.set_low()?;
        Ok(Self { pin })
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
