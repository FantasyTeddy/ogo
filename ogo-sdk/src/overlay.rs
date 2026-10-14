//! What the SDK draws over the game: the pause menu (MENU) and the sound indicator (VOL).

use std::time::{Duration, Instant};

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_8X13},
        MonoFont,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Alignment,
};
use ogo_board::{
    settings::SoundMode,
    ui::{self, panel, selection, DIM, SELECTION, TEXT},
    Button, Canvas, Note, Settings,
};

use crate::{ctx::Input, MenuSetting};

const INDICATOR_SHOWN_FOR: Duration = Duration::from_millis(1200);
/// Where the sound indicator shows, top right.
pub const INDICATOR_AREA: Rectangle = Rectangle::new(
    Point::new(ogo_board::display::WIDTH as i32 - 130, 8),
    Size::new(122, 34),
);
pub const MENU_SOUND: &[Note] = &[Note::new(660, 30)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    Resume,
    /// Steps down (-1) or up (1); unmutes.
    Volume(isize),
    ToggleMusic,
    /// One of the game's own settings, by index, changed by -1 or 1.
    GameSetting(usize, isize),
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Resume,
    Volume,
    Music,
    /// One of the game's own settings, by index.
    Game(usize),
    Exit,
}
const ROW_HEIGHT: i32 = 28;

#[derive(Debug)]
pub struct PauseMenu {
    items: Vec<Item>,
    selected: usize,
}

impl PauseMenu {
    /// `music`: the game has music of its own, so the menu has a line to switch it.
    /// `game_settings`: how many settings of its own the game has.
    pub fn new(music: bool, game_settings: usize) -> Self {
        let mut items = vec![Item::Resume, Item::Volume];
        if music {
            items.push(Item::Music);
        }
        items.extend((0..game_settings).map(Item::Game));
        items.push(Item::Exit);
        Self { items, selected: 0 }
    }

    /// Up/Down choose, Left/Right change the volume or switch the music, A confirms, B or MENU
    /// resume.
    pub fn update(&mut self, input: &Input) -> Option<PauseAction> {
        let count = self.items.len();
        if input.pressed(Button::Up) {
            self.selected = (self.selected + count - 1) % count;
        }
        if input.pressed(Button::Down) {
            self.selected = (self.selected + 1) % count;
        }
        let left = input.pressed(Button::Left);
        let right = input.pressed(Button::Right);
        let a = input.pressed(Button::A);
        match self.items[self.selected] {
            Item::Resume if a => return Some(PauseAction::Resume),
            Item::Volume if left => return Some(PauseAction::Volume(-1)),
            Item::Volume if right => return Some(PauseAction::Volume(1)),
            Item::Music if a || left || right => return Some(PauseAction::ToggleMusic),
            Item::Game(index) if left => return Some(PauseAction::GameSetting(index, -1)),
            Item::Game(index) if a || right => return Some(PauseAction::GameSetting(index, 1)),
            Item::Exit if a => return Some(PauseAction::Exit),
            _ => {}
        }
        if input.pressed(Button::B) || input.pressed(Button::Menu) {
            return Some(PauseAction::Resume);
        }
        None
    }

    /// Dims the game's frame and draws the menu on top, with the game's own settings.
    pub fn draw(&self, fb: &mut Canvas, settings: &Settings, game_settings: &[MenuSetting]) {
        fb.dim();
        let height = 54 + self.items.len() as i32 * ROW_HEIGHT;
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
        for (i, &item) in self.items.iter().enumerate() {
            let label = match item {
                Item::Resume => "Resume".to_owned(),
                Item::Volume => format!("Volume: {}", volume_text(settings)),
                Item::Music => format!("Music: {}", if settings.music { "On" } else { "Off" }),
                Item::Game(index) => game_settings
                    .get(index)
                    .map_or_else(String::new, |s| format!("{}: {}", s.label, s.value)),
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
            if selected && matches!(item, Item::Volume | Item::Music | Item::Game(_)) {
                let middle = row.center().y;
                text(fb, "<", Point::new(row.top_left.x + 10, middle), &FONT_8X13);
                let right = row.top_left.x + row.size.width as i32 - 10;
                text(fb, ">", Point::new(right, middle), &FONT_8X13);
            }
        }
    }
}

/// Shows the sound mode and volume briefly after VOL is pressed.
#[derive(Debug)]
pub struct SoundIndicator {
    until: Option<Instant>,
    /// The game has music of its own; without, the music setting doesn't show.
    music: bool,
}

impl SoundIndicator {
    pub fn new(music: bool) -> Self {
        Self { until: None, music }
    }

    pub fn show(&mut self) {
        self.until = Some(Instant::now() + INDICATOR_SHOWN_FOR);
    }

    /// Whether the indicator is shown now.
    pub fn visible(&mut self) -> bool {
        if self.until.is_some_and(|until| Instant::now() >= until) {
            self.until = None;
        }
        self.until.is_some()
    }

    /// Draws the indicator while it's shown, at [`INDICATOR_AREA`] in screen coordinates; returns
    /// where.
    pub fn draw<D: DrawTarget<Color = Rgb565>>(
        &mut self,
        fb: &mut D,
        settings: &Settings,
    ) -> Option<Rectangle> {
        if !self.visible() {
            return None;
        }
        let area = INDICATOR_AREA;
        panel(fb, area);
        let label = match settings.sound_mode() {
            SoundMode::Muted => "Muted",
            _ if settings.volume == 0 => "Volume off",
            SoundMode::EffectsOnly if self.music => "Music off",
            SoundMode::All | SoundMode::EffectsOnly => "Sound on",
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
        Some(area)
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
fn text<D: DrawTarget<Color = Rgb565>>(fb: &mut D, text: &str, center: Point, font: &MonoFont<'_>) {
    ui::text(fb, text, center, font, TEXT, Alignment::Center);
}
