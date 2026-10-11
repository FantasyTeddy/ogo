//! What the SDK draws over the game: the pause menu (MENU) and the volume indicator (VOL).

use std::time::{Duration, Instant};

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_8X13},
        MonoTextStyle,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use ogo_board::{Button, Canvas, Note};

use crate::ctx::Input;

const PANEL: Rgb565 = Rgb565::new(4, 8, 4);
const SELECTION: Rgb565 = Rgb565::CSS_STEEL_BLUE;
const TEXT: Rgb565 = Rgb565::WHITE;
const VOLUME_SHOWN_FOR: Duration = Duration::from_millis(1200);
pub const MENU_SOUND: &[Note] = &[Note::new(660, 30)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    Resume,
    Exit,
}

const ITEMS: [(&str, PauseAction); 2] = [("Resume", PauseAction::Resume), ("Exit to launcher", PauseAction::Exit)];

#[derive(Debug, Default)]
pub struct PauseMenu {
    selected: usize,
}

impl PauseMenu {
    /// Up/Down choose, A confirms, B or MENU resume.
    pub fn update(&mut self, input: &Input) -> Option<PauseAction> {
        if input.pressed(Button::Up) || input.pressed(Button::Down) {
            self.selected = (self.selected + 1) % ITEMS.len();
        }
        if input.pressed(Button::A) {
            return Some(ITEMS[self.selected].1);
        }
        if input.pressed(Button::B) || input.pressed(Button::Menu) {
            return Some(PauseAction::Resume);
        }
        None
    }

    /// Dims the game's frame and draws the menu on top.
    pub fn draw(&self, fb: &mut Canvas) {
        fb.dim();
        let panel = Rectangle::with_center(fb.bounding_box().center(), Size::new(200, 110));
        let style = PrimitiveStyleBuilder::new().fill_color(PANEL).stroke_color(TEXT).stroke_width(1).build();
        panel.into_styled(style).draw(fb).ok();

        let center_x = panel.center().x;
        text(fb, "Paused", Point::new(center_x, panel.top_left.y + 20), &FONT_10X20);
        for (i, (label, _)) in ITEMS.iter().enumerate() {
            let row = Rectangle::new(Point::new(panel.top_left.x + 10, panel.top_left.y + 44 + i as i32 * 28), Size::new(180, 24));
            if i == self.selected {
                row.into_styled(PrimitiveStyle::with_fill(SELECTION)).draw(fb).ok();
            }
            text(fb, label, row.center(), &FONT_8X13);
        }
    }
}

/// Shows the volume briefly after VOL is pressed.
#[derive(Debug, Default)]
pub struct VolumeIndicator {
    until: Option<Instant>,
}

impl VolumeIndicator {
    pub fn show(&mut self) {
        self.until = Some(Instant::now() + VOLUME_SHOWN_FOR);
    }

    pub fn draw(&mut self, fb: &mut Canvas, volume: u8) {
        if self.until.is_some_and(|until| Instant::now() >= until) {
            self.until = None;
        }
        if self.until.is_none() {
            return;
        }
        let panel = Rectangle::new(Point::new(fb.size().width as i32 - 130, 8), Size::new(122, 34));
        let style = PrimitiveStyleBuilder::new().fill_color(PANEL).stroke_color(TEXT).stroke_width(1).build();
        panel.into_styled(style).draw(fb).ok();
        let label = if volume == 0 { "Muted".to_owned() } else { format!("Volume {volume}%") };
        text(fb, &label, Point::new(panel.center().x, panel.top_left.y + 11), &FONT_8X13);
        // Bar showing the level.
        let bar = Rectangle::new(panel.top_left + Point::new(8, 21), Size::new(106, 6));
        bar.into_styled(PrimitiveStyle::with_stroke(TEXT, 1)).draw(fb).ok();
        Rectangle::new(bar.top_left, Size::new(106 * u32::from(volume) / 100, 6))
            .into_styled(PrimitiveStyle::with_fill(SELECTION))
            .draw(fb)
            .ok();
    }
}

fn text(fb: &mut Canvas, text: &str, center: Point, font: &embedded_graphics::mono_font::MonoFont<'_>) {
    let style = TextStyleBuilder::new().alignment(Alignment::Center).baseline(Baseline::Middle).build();
    Text::with_text_style(text, center, MonoTextStyle::new(font, TEXT), style).draw(fb).ok();
}
