//! Minimal test game for M3: proves that the launcher can install and boot a game, and that the
//! game gets back to the launcher. MENU returns; A plays a sound.

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_8X13},
        MonoTextStyle,
    },
    pixelcolor::Rgb565,
    prelude::*,
    text::{Alignment, Text},
};
use esp_idf_svc::hal::delay::FreeRtos;
use ogo_board::{
    boot::{self, Boot},
    Board, Button, ButtonState, DisplayResultExt, Note,
};

ogo_board::image_desc!("ogo-game");

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // Before anything else can fail: make sure the next reset starts the launcher.
    let mut boot = Boot::take()?;
    boot.return_to_launcher_on_reset()?;
    log::info!("hello running from {}", boot::running_partition_label());

    let mut board = Board::take()?;
    let display = &mut board.display;
    display.clear(Rgb565::CSS_DARK_GREEN).display_err()?;
    let big = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let small = MonoTextStyle::new(&FONT_8X13, Rgb565::WHITE);
    Text::with_alignment("Hello from a game!", Point::new(160, 80), big, Alignment::Center)
        .draw(display)
        .display_err()?;
    let slot = format!("Running from {}", boot::running_partition_label());
    Text::with_alignment(&slot, Point::new(160, 120), small, Alignment::Center).draw(display).display_err()?;
    Text::with_alignment("A: sound   MENU: back to launcher", Point::new(160, 200), small, Alignment::Center)
        .draw(display)
        .display_err()?;

    let mut previous = ButtonState::default();
    loop {
        let buttons = board.buttons.read()?;
        if buttons.just_pressed(previous, Button::A) {
            board.speaker.play(&[Note::new(523, 80), Note::new(659, 80), Note::new(784, 120)]);
        }
        if buttons.just_pressed(previous, Button::Menu) {
            log::info!("Back to the launcher");
            boot::restart();
        }
        previous = buttons;
        FreeRtos::delay_ms(20);
    }
}
