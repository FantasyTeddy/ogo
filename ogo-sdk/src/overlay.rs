//! What the SDK draws over the game: the pause menu (MENU) and the sound indicator (VOL).

use std::time::{Duration, Instant};

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_8X13},
        MonoFont,
    },
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Alignment,
};
use ogo_board::{
    settings::SoundMode,
    ui::{self, panel, selection, DIM, SELECTION, TEXT},
    Button, Canvas, Note, Settings,
};

use crate::ctx::Input;

const INDICATOR_SHOWN_FOR: Duration = Duration::from_millis(1200);
pub const MENU_SOUND: &[Note] = &[Note::new(660, 30)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    Resume,
    /// Steps down (-1) or up (1); unmutes.
    Volume(isize),
    ToggleMusic,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Resume,
    Volume,
    Music,
    Exit,
}

const ITEMS: [Item; 4] = [Item::Resume, Item::Volume, Item::Music, Item::Exit];
const ROW_HEIGHT: i32 = 28;

#[derive(Debug, Default)]
pub struct PauseMenu {
    selected: usize,
}

impl PauseMenu {
    /// Up/Down choose, Left/Right change the volume or switch the music, A confirms, B or MENU
    /// resume.
    pub fn update(&mut self, input: &Input) -> Option<PauseAction> {
        if input.pressed(Button::Up) {
            self.selected = (self.selected + ITEMS.len() - 1) % ITEMS.len();
        }
        if input.pressed(Button::Down) {
            self.selected = (self.selected + 1) % ITEMS.len();
        }
        let left = input.pressed(Button::Left);
        let right = input.pressed(Button::Right);
        let a = input.pressed(Button::A);
        match ITEMS[self.selected] {
            Item::Resume if a => return Some(PauseAction::Resume),
            Item::Volume if left => return Some(PauseAction::Volume(-1)),
            Item::Volume if right => return Some(PauseAction::Volume(1)),
            Item::Music if a || left || right => return Some(PauseAction::ToggleMusic),
            Item::Exit if a => return Some(PauseAction::Exit),
            _ => {}
        }
        if input.pressed(Button::B) || input.pressed(Button::Menu) {
            return Some(PauseAction::Resume);
        }
        None
    }

    /// Dims the game's frame and draws the menu on top.
    pub fn draw(&self, fb: &mut Canvas, settings: &Settings) {
        fb.dim();
        let height = 54 + ITEMS.len() as i32 * ROW_HEIGHT;
        let area =
            Rectangle::with_center(fb.bounding_box().center(), Size::new(200, height as u32));
        panel(fb, area);

        let center_x = area.center().x;
        text(
            fb,
            "Paused",
            Point::new(center_x, area.top_left.y + 20),
            &FONT_10X20,
        );
        for (i, item) in ITEMS.into_iter().enumerate() {
            let label = match item {
                Item::Resume => "Resume".to_owned(),
                Item::Volume => format!("Volume: {}", volume_text(settings)),
                Item::Music => format!("Music: {}", if settings.music { "On" } else { "Off" }),
                Item::Exit => "Exit to launcher".to_owned(),
            };
            let row = Rectangle::new(
                Point::new(
                    area.top_left.x + 10,
                    area.top_left.y + 44 + i as i32 * ROW_HEIGHT,
                ),
                Size::new(180, 24),
            );
            let selected = i == self.selected;
            if selected {
                selection(fb, row);
            }
            text(fb, &label, row.center(), &FONT_8X13);
            // Left/Right change these.
            if selected && matches!(item, Item::Volume | Item::Music) {
                let middle = row.center().y;
                text(fb, "<", Point::new(row.top_left.x + 10, middle), &FONT_8X13);
                let right = row.top_left.x + row.size.width as i32 - 10;
                text(fb, ">", Point::new(right, middle), &FONT_8X13);
            }
        }
    }
}

/// Shows the sound mode and volume briefly after VOL is pressed.
#[derive(Debug, Default)]
pub struct SoundIndicator {
    until: Option<Instant>,
}

impl SoundIndicator {
    pub fn show(&mut self) {
        self.until = Some(Instant::now() + INDICATOR_SHOWN_FOR);
    }

    pub fn draw(&mut self, fb: &mut Canvas, settings: &Settings) {
        if self.until.is_some_and(|until| Instant::now() >= until) {
            self.until = None;
        }
        if self.until.is_none() {
            return;
        }
        let area = Rectangle::new(
            Point::new(fb.size().width as i32 - 130, 8),
            Size::new(122, 34),
        );
        panel(fb, area);
        let label = match settings.sound_mode() {
            SoundMode::Muted => "Muted",
            _ if settings.volume == 0 => "Volume off",
            SoundMode::All => "Sound on",
            SoundMode::EffectsOnly => "Music off",
        };
        text(
            fb,
            label,
            Point::new(area.center().x, area.top_left.y + 11),
            &FONT_8X13,
        );
        // Bar showing the volume, empty while muted.
        let bar = Rectangle::new(area.top_left + Point::new(8, 21), Size::new(106, 6));
        let muted = settings.muted;
        bar.into_styled(PrimitiveStyle::with_stroke(
            if muted { DIM } else { TEXT },
            1,
        ))
        .draw(fb)
        .ok();
        if !muted {
            let filled = 106 * u32::from(settings.volume) / 100;
            Rectangle::new(bar.top_left, Size::new(filled, 6))
                .into_styled(PrimitiveStyle::with_fill(SELECTION))
                .draw(fb)
                .ok();
        }
    }
}

/// e.g. "50%", "Off" (0) or "Muted".
fn volume_text(settings: &Settings) -> String {
    if settings.muted {
        "Muted".to_owned()
    } else if settings.volume == 0 {
        "Off".to_owned()
    } else {
        format!("{}%", settings.volume)
    }
}

/// White text centred on `center`.
fn text(fb: &mut Canvas, text: &str, center: Point, font: &MonoFont<'_>) {
    ui::text(fb, text, center, font, TEXT, Alignment::Center);
}
