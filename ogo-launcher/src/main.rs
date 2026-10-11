//! M1: hardware test screen for `ogo-board`.
//!
//! Shows every button, the raw D-pad readings, the battery level and the SD card contents.
//! A and B beep, Volume cycles the speaker volume, Start re-reads the SD card.

use std::{fs, time::Instant};

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_6X10, FONT_8X13},
        MonoTextStyle, MonoTextStyleBuilder,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use esp_idf_svc::hal::delay::FreeRtos;
use ogo_board::{sd, Board, Button, ButtonState, DisplayResultExt, SdCard, Speaker};

const POLL_MS: u32 = 20;
const DPAD_REDRAW_POLLS: u32 = 5;
const BATTERY_REDRAW_POLLS: u32 = 50;
const VOLUME_STEPS: [u8; 5] = [0, 25, 50, 75, 100];

const BUTTON_LABELS: [(Button, &str); 10] = [
    (Button::Up, "UP"),
    (Button::Down, "DOWN"),
    (Button::Left, "LEFT"),
    (Button::Right, "RIGHT"),
    (Button::A, "A"),
    (Button::B, "B"),
    (Button::Select, "SELECT"),
    (Button::Start, "START"),
    (Button::Menu, "MENU"),
    (Button::Volume, "VOL"),
];

const BOX_SIZE: Size = Size::new(56, 28);
const INFO_X: i32 = 8;
const INFO_Y: i32 = 112;
const LINE_HEIGHT: i32 = 18;

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("ogo-launcher M1 hardware test");

    let mut board = Board::take()?;
    let display = &mut board.display;

    display.clear(Rgb565::BLACK).display_err()?;
    Text::new("M1 hardware test", Point::new(8, 20), MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE))
        .draw(display).display_err()?;
    Text::with_alignment(
        "A/B beep  VOL volume  START re-read SD",
        Point::new(160, 234),
        MonoTextStyle::new(&FONT_6X10, Rgb565::CSS_GRAY),
        Alignment::Center,
    )
    .draw(display).display_err()?;

    let mut buttons = ButtonState::default();
    draw_buttons(display, buttons)?;
    let mut led_on = false;
    let mut sd_reads = 1;
    draw_info(display, 2, &status_line(board.speaker.volume(), led_on))?;
    draw_sd(display, board.sd.as_ref(), sd_reads)?;

    for poll in 0u32.. {
        let previous = buttons;
        buttons = board.buttons.read()?;

        if buttons != previous {
            draw_buttons(&mut board.display, buttons)?;
            if buttons.any() != led_on {
                led_on = buttons.any();
                board.led.set_on(led_on)?;
                draw_info(&mut board.display, 2, &status_line(board.speaker.volume(), led_on))?;
            }
        }
        if poll % DPAD_REDRAW_POLLS == 0 {
            let raw = board.buttons.dpad_raw();
            draw_info(&mut board.display, 0, &format!("D-pad raw  X {:4}  Y {:4}", raw.x, raw.y))?;
        }
        if poll % BATTERY_REDRAW_POLLS == 0 {
            let level = board.battery.read()?;
            let text = format!(
                "Battery    {}.{:02} V  {:3} %",
                level.millivolts / 1000,
                level.millivolts % 1000 / 10,
                level.percent
            );
            draw_info(&mut board.display, 1, &text)?;
        }

        if buttons.just_pressed(previous, Button::A) {
            beep(&mut board.speaker, 1000, 80);
        }
        if buttons.just_pressed(previous, Button::B) {
            beep(&mut board.speaker, 440, 80);
        }
        if buttons.just_pressed(previous, Button::Volume) {
            let current = VOLUME_STEPS.iter().position(|&v| v == board.speaker.volume()).unwrap_or(0);
            board.speaker.set_volume(VOLUME_STEPS[(current + 1) % VOLUME_STEPS.len()])?;
            draw_info(&mut board.display, 2, &status_line(board.speaker.volume(), led_on))?;
            beep(&mut board.speaker, 880, 60);
        }
        if buttons.just_pressed(previous, Button::Start) {
            sd_reads += 1;
            draw_sd(&mut board.display, board.sd.as_ref(), sd_reads)?;
        }

        FreeRtos::delay_ms(POLL_MS);
    }
    Ok(())
}

/// A failed beep shouldn't stop the test screen, so it is only logged.
fn beep(speaker: &mut Speaker, frequency_hz: u32, duration_ms: u32) {
    if let Err(e) = speaker.beep(frequency_hz, duration_ms) {
        log::error!("Beep failed: {e:?}");
    }
}

fn status_line(volume: u8, led_on: bool) -> String {
    format!("Volume     {volume:3} %   LED {}", if led_on { "on" } else { "off" })
}

/// Two rows of five boxes; a box is filled while its button is held.
fn draw_buttons<D>(display: &mut D, state: ButtonState) -> anyhow::Result<()>
where
    D: DrawTarget<Color = Rgb565>,
    D::Error: core::fmt::Debug,
{
    for (i, (button, label)) in BUTTON_LABELS.iter().enumerate() {
        let held = state.is_held(*button);
        let top_left = Point::new(8 + (i % 5) as i32 * 62, 34 + (i / 5) as i32 * 36);
        let style = PrimitiveStyleBuilder::new()
            .stroke_color(Rgb565::CSS_GRAY)
            .stroke_width(1)
            .fill_color(if held { Rgb565::GREEN } else { Rgb565::BLACK })
            .build();
        Rectangle::new(top_left, BOX_SIZE).into_styled(style).draw(display).display_err()?;
        let text_color = if held { Rgb565::BLACK } else { Rgb565::WHITE };
        Text::with_text_style(
            label,
            top_left + Point::new(BOX_SIZE.width as i32 / 2, BOX_SIZE.height as i32 / 2),
            MonoTextStyle::new(&FONT_6X10, text_color),
            TextStyleBuilder::new()
                .alignment(Alignment::Center)
                .baseline(Baseline::Middle)
                .build(),
        )
        .draw(display).display_err()?;
    }
    Ok(())
}

/// One full-width line of the info area, overwriting what was there.
fn draw_info<D>(display: &mut D, line: i32, text: &str) -> anyhow::Result<()>
where
    D: DrawTarget<Color = Rgb565>,
    D::Error: core::fmt::Debug,
{
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_8X13)
        .text_color(Rgb565::WHITE)
        .background_color(Rgb565::BLACK)
        .build();
    // 38 columns of 8 px fill the line, clearing longer previous text.
    let text = format!("{text:<38.38}");
    Text::with_baseline(&text, Point::new(INFO_X, INFO_Y + line * LINE_HEIGHT), style, Baseline::Top)
        .draw(display).display_err()?;
    Ok(())
}

/// Lists the card's root folder. Reading the card between LCD updates exercises the shared bus.
fn draw_sd<D>(display: &mut D, card: Option<&SdCard>, read_count: u32) -> anyhow::Result<()>
where
    D: DrawTarget<Color = Rgb565>,
    D::Error: core::fmt::Debug,
{
    let Some(card) = card else {
        draw_info(display, 3, "SD card    not mounted")?;
        for line in 4..6 {
            draw_info(display, line, "")?;
        }
        return Ok(());
    };

    let started = Instant::now();
    let mut names: Vec<String> = match fs::read_dir(sd::MOUNT_POINT) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => {
            log::warn!("Reading {} failed: {e}", sd::MOUNT_POINT);
            draw_info(display, 3, &format!("SD card    read error: {e}"))?;
            return Ok(());
        }
    };
    names.sort();
    log::info!("SD root: {names:?}");

    let ogo = if fs::metadata(card.path("ogo")).is_ok() { "present" } else { "missing" };
    let elapsed_ms = started.elapsed().as_millis();
    draw_info(
        display,
        3,
        &format!("SD card    {} MB, {} entries", card.capacity_bytes() >> 20, names.len()),
    )?;
    draw_info(display, 4, &format!("  /ogo {ogo}   read #{read_count} in {elapsed_ms} ms"))?;
    draw_info(display, 5, &format!("  {}", names.join(" ")))?;
    Ok(())
}
