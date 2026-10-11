//! M0: hello world. Logs over serial and draws a test screen on the LCD.

use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use esp_idf_svc::hal::{
    delay::{Ets, FreeRtos},
    gpio::PinDriver,
    peripherals::Peripherals,
    spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriver, SpiDriverConfig},
    units::FromValueType,
};
use mipidsi::{
    interface::SpiInterface,
    models::ILI9341Rgb565,
    options::{ColorOrder, Orientation, Rotation},
    Builder,
};

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Hello from ogo-launcher (M0)");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // The SD card shares the SPI bus with the LCD. Keep its CS high so it stays off the bus.
    let mut sd_cs = PinDriver::output(pins.gpio22).unwrap();
    sd_cs.set_high().unwrap();

    // SPI3 (VSPI): these are its IOMUX pins, which allows 40 MHz. Routing them through
    // SPI2 would go via the GPIO matrix, which caps the clock at ~26 MHz.
    let spi = SpiDriver::new(
        peripherals.spi3,
        pins.gpio18,       // SCLK
        pins.gpio23,       // MOSI
        Some(pins.gpio19), // MISO
        &SpiDriverConfig::new(),
    )
    .unwrap();
    let lcd_spi = SpiDeviceDriver::new(
        spi,
        Some(pins.gpio5), // LCD CS
        &SpiConfig::new().baudrate(40.MHz().into()),
    )
    .unwrap();
    let dc = PinDriver::output(pins.gpio21).unwrap();

    let mut buffer = [0u8; 512];
    let di = SpiInterface::new(lcd_spi, dc, &mut buffer);
    // The ILI9341 is portrait natively; the Odroid-Go holds it landscape (320x240).
    // The panel is mounted mirrored, so rotating alone shows the image flipped left to right.
    // No reset pin is wired, so mipidsi falls back to a software reset.
    let mut display = Builder::new(ILI9341Rgb565, di)
        .orientation(Orientation::new().rotate(Rotation::Deg90).flip_horizontal())
        .color_order(ColorOrder::Bgr)
        .init(&mut Ets)
        .unwrap();

    let mut backlight = PinDriver::output(pins.gpio14).unwrap();
    backlight.set_high().unwrap();

    draw_test_screen(&mut display).unwrap();
    log::info!("Test screen drawn");

    loop {
        FreeRtos::delay_ms(1000);
    }
}

/// Corner labels show the orientation, the bars show the color order.
fn draw_test_screen<D>(display: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    display.clear(Rgb565::BLACK)?;

    let white = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    Text::with_alignment("Hello, Odroid-Go!", Point::new(160, 80), white, Alignment::Center)
        .draw(display)?;

    for (i, (color, label)) in [
        (Rgb565::RED, "R"),
        (Rgb565::GREEN, "G"),
        (Rgb565::BLUE, "B"),
    ]
    .into_iter()
    .enumerate()
    {
        let x = 70 + i as i32 * 64;
        Rectangle::new(Point::new(x, 110), Size::new(52, 52))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)?;
        Text::with_alignment(label, Point::new(x + 26, 184), white, Alignment::Center)
            .draw(display)?;
    }

    Text::new("TL", Point::new(4, 18), white).draw(display)?;
    Text::with_alignment("TR", Point::new(316, 18), white, Alignment::Right).draw(display)?;
    Text::new("BL", Point::new(4, 234), white).draw(display)?;
    Text::with_alignment("BR", Point::new(316, 234), white, Alignment::Right).draw(display)?;

    Ok(())
}
