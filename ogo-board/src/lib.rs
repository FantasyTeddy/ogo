//! Board support for the ODROID-GO.
//!
//! [`Board::take`] sets up all on-board hardware once. The LCD and SD card share one SPI bus
//! (SPI3 / VSPI) with separate chip selects; ESP-IDF's SPI master driver arbitrates between them.

mod adc;
pub mod battery;
pub mod boot;
pub mod buttons;
pub mod canvas;
pub mod display;
pub mod flash;
pub mod image;
pub mod pins;
pub mod sd;
pub mod settings;
pub mod speaker;
pub mod ui;

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use esp_idf_svc::{
    hal::{
        adc::oneshot::AdcDriver,
        gpio::{Output, OutputPin, PinDriver},
        modem::Modem,
        peripherals::Peripherals,
        spi::{Dma, SpiDriver, SpiDriverConfig},
    },
    sys,
};

pub use battery::{Battery, BatteryLevel};
pub use buttons::{Button, ButtonState, Buttons, DpadRaw};
pub use canvas::Canvas;
pub use display::{Backlight, Display, DisplayResultExt};
pub use sd::SdCard;
pub use settings::Settings;
pub use speaker::{Note, Speaker};

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
    /// The WiFi/Bluetooth radio; take it to start WiFi.
    pub modem: Option<Modem<'static>>,
}

impl Board {
    /// Takes the ESP32 peripherals and initializes all board hardware, with the default
    /// [`Settings`] (full brightness). Call once.
    pub fn take() -> anyhow::Result<Self> {
        Self::take_with(&Settings::default(), |_| Ok(()))
    }

    /// Like [`Board::take`], but applies `settings` (brightness, volume), and calls `first_frame`
    /// as soon as the display works, before the backlight turns on and before the slower SD card
    /// setup. Use it to show a boot screen.
    pub fn take_with(
        settings: &Settings,
        first_frame: impl FnOnce(&mut Display) -> anyhow::Result<()>,
    ) -> anyhow::Result<Self> {
        let own = image::running();
        log::info!("{} {} (commit {})", own.kind_name(), own.version(), own.commit().unwrap_or("unknown"));
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

        let mut display = display::init(spi, pins.gpio5, pins.gpio21)?;
        // The panel still shows whatever the previous app left; clear it before it becomes visible.
        display.clear(Rgb565::BLACK).display_err()?;
        first_frame(&mut display)?;
        let mut backlight = Backlight::new(p.ledc.timer0, p.ledc.channel0, pins.gpio14)?;
        backlight.set_brightness(settings.brightness)?;

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
        let mut speaker = Speaker::new(pins.gpio25, pins.gpio26)?;
        speaker.set_volume(settings.volume);
        let led = Led::new(pins.gpio2)?;

        Ok(Self { display, backlight, buttons, speaker, battery, led, sd, modem: Some(p.modem) })
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
