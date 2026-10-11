//! Launcher screen: top bar with battery, game list, detail panel and footer. The WiFi and settings
//! screens replace the list and detail panel.
//!
//! Each area is drawn into its own [`Canvas`] and presented in one transfer.

use embedded_graphics::{
    image::{Image, ImageRawLE},
    mono_font::{
        ascii::{FONT_10X20, FONT_6X10, FONT_8X13, FONT_9X15_BOLD},
        MonoTextStyle,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
pub use ogo_board::ui::{DIM, SUCCESS, TEXT, WARNING};
use ogo_board::{
    display,
    ui::{progress_bar, selection, text, ACCENT, BACKGROUND, BAR, SELECTION},
    BatteryLevel, Canvas, Display, DisplayResultExt,
};
use ogo_format::text::{truncate, wrap};

use crate::library::{Entry, Library, GAMES_DIR, ICON_SIZE};

/// Center of the step numbers on the WiFi screen.
const STEP_X: i32 = 34;

/// Footer controls of the game list.
pub const GAMES_CONTROLS: &str = "A start  START rescan  SELECT WiFi  MENU settings";

/// What the settings screen shows.
#[derive(Debug, Clone)]
pub struct SettingsView {
    /// Read-only lines at the top: label and value.
    pub info: Vec<(&'static str, String)>,
    pub items: Vec<SettingsItem>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum SettingsItem {
    /// A level shown as a bar, changed with Left/Right. `value` is the text next to the bar.
    Level { label: &'static str, percent: u8, value: String },
    /// Done with A. A disabled action is drawn dimmed.
    Action { label: String, color: Rgb565, enabled: bool },
}

/// What the WiFi screen shows.
#[derive(Debug, Clone, Default)]
pub struct ConnectView {
    pub heading: String,
    /// Shown as numbered steps, in this order: join this WiFi, open the URL, enter the PIN.
    pub join_wifi: Option<String>,
    pub url: Option<String>,
    pub pin: Option<String>,
    /// The latest thing that happened, e.g. an upload.
    pub activity: Option<(String, Rgb565)>,
}

const WIDTH: u32 = display::WIDTH as u32;
const HEIGHT: u32 = display::HEIGHT as u32;
const TOP_BAR_HEIGHT: u32 = 24;
const FOOTER_HEIGHT: u32 = 20;
const BODY_HEIGHT: u32 = HEIGHT - TOP_BAR_HEIGHT - FOOTER_HEIGHT;
const LIST_WIDTH: u32 = 192;
const DETAIL_WIDTH: u32 = WIDTH - LIST_WIDTH;

const ROW_HEIGHT: u32 = 28;
/// Rows that fit into the list.
pub const VISIBLE_ROWS: usize = (BODY_HEIGHT / ROW_HEIGHT) as usize;
/// Settings screen: where values start, and the level bars.
const SETTINGS_VALUE_X: i32 = 100;
const SETTINGS_BAR_WIDTH: u32 = 140;
const SCROLLBAR_WIDTH: u32 = 3;
const LIST_TEXT_X: i32 = 8;

/// Which part of the library is on screen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ListState {
    pub selected: usize,
    /// Index of the topmost visible row.
    pub first: usize,
}

impl ListState {
    /// Moves the selection by `delta`, clamped to the list, and scrolls it into view.
    /// With `wrap`, moving past either end continues at the other end.
    pub fn move_by(&mut self, delta: isize, len: usize, wrap: bool) {
        if len == 0 {
            *self = Self::default();
            return;
        }
        let target = self.selected as isize + delta;
        self.selected = if wrap && target < 0 && self.selected == 0 {
            len - 1
        } else if wrap && target >= len as isize && self.selected == len - 1 {
            0
        } else {
            target.clamp(0, len as isize - 1) as usize
        };
        if self.selected < self.first {
            self.first = self.selected;
        } else if self.selected >= self.first + VISIBLE_ROWS {
            self.first = self.selected + 1 - VISIBLE_ROWS;
        }
    }
}

pub struct Ui {
    top_bar: Canvas,
    list: Canvas,
    detail: Canvas,
    /// List and detail area together, for messages when there is nothing to list.
    body: Canvas,
    footer: Canvas,
}

impl Ui {
    pub fn new() -> Self {
        let body_top = TOP_BAR_HEIGHT as i32;
        Self {
            top_bar: Canvas::new(Rectangle::new(Point::zero(), Size::new(WIDTH, TOP_BAR_HEIGHT))),
            list: Canvas::new(Rectangle::new(Point::new(0, body_top), Size::new(LIST_WIDTH, BODY_HEIGHT))),
            detail: Canvas::new(Rectangle::new(
                Point::new(LIST_WIDTH as i32, body_top),
                Size::new(DETAIL_WIDTH, BODY_HEIGHT),
            )),
            body: Canvas::new(Rectangle::new(Point::new(0, body_top), Size::new(WIDTH, BODY_HEIGHT))),
            footer: Canvas::new(Rectangle::new(
                Point::new(0, (HEIGHT - FOOTER_HEIGHT) as i32),
                Size::new(WIDTH, FOOTER_HEIGHT),
            )),
        }
    }

    pub fn draw_top_bar(&mut self, display: &mut Display, title: &str, battery: Option<BatteryLevel>) -> anyhow::Result<()> {
        let canvas = &mut self.top_bar;
        canvas.clear(BAR).ok();
        let middle = TOP_BAR_HEIGHT as i32 / 2;
        text(canvas, title, Point::new(8, middle), &FONT_9X15_BOLD, TEXT, Alignment::Left);

        // Battery: outline with a fill level, and the percentage to its left.
        let icon = Rectangle::new(Point::new(WIDTH as i32 - 32, middle - 5), Size::new(22, 11));
        icon.into_styled(PrimitiveStyle::with_stroke(TEXT, 1)).draw(canvas).ok();
        Rectangle::new(icon.top_left + Point::new(22, 3), Size::new(2, 5))
            .into_styled(PrimitiveStyle::with_fill(TEXT))
            .draw(canvas)
            .ok();
        let label = match battery {
            Some(level) => {
                let color = match level.percent {
                    0..=15 => Rgb565::RED,
                    16..=30 => Rgb565::YELLOW,
                    _ => Rgb565::GREEN,
                };
                let fill_width = (18 * u32::from(level.percent)).div_ceil(100);
                Rectangle::new(icon.top_left + Point::new(2, 2), Size::new(fill_width, 7))
                    .into_styled(PrimitiveStyle::with_fill(color))
                    .draw(canvas)
                    .ok();
                format!("{}%", level.percent)
            }
            None => "--".into(),
        };
        text(canvas, &label, Point::new(WIDTH as i32 - 38, middle), &FONT_8X13, TEXT, Alignment::Right);

        canvas.present(display)
    }

    /// The footer shows the screen's `controls`, or `message` in their place.
    pub fn draw_footer(&mut self, display: &mut Display, controls: &str, message: Option<&str>) -> anyhow::Result<()> {
        let canvas = &mut self.footer;
        canvas.clear(BAR).ok();
        let center = Point::new(WIDTH as i32 / 2, FOOTER_HEIGHT as i32 / 2);
        match message {
            Some(message) => text(canvas, message, center, &FONT_6X10, TEXT, Alignment::Center),
            None => text(canvas, controls, center, &FONT_6X10, DIM, Alignment::Center),
        }
        canvas.present(display)
    }

    /// Draws the list and the detail panel, or a message if there is nothing to list.
    /// `icon` is the selected game's icon, if it has one.
    pub fn draw_library(
        &mut self,
        display: &mut Display,
        library: &Library,
        state: ListState,
        icon: Option<&[u8]>,
    ) -> anyhow::Result<()> {
        let entries = match library {
            Library::NoSdCard => return self.draw_message(display, "No SD card", &["Insert a card and restart."]),
            Library::NoGamesDir => return self.draw_no_games(display),
            Library::Entries(entries) if entries.is_empty() => return self.draw_no_games(display),
            Library::Entries(entries) => entries,
        };
        self.draw_list(entries, state);
        self.draw_detail(&entries[state.selected], state.selected, entries.len(), icon);
        self.list.present(display)?;
        self.detail.present(display)
    }

    /// Full-body progress screen. Without `fraction`, the bar is left out.
    pub fn draw_progress(
        &mut self,
        display: &mut Display,
        title: &str,
        status: &str,
        fraction: Option<f32>,
    ) -> anyhow::Result<()> {
        let canvas = &mut self.body;
        canvas.clear(BACKGROUND).ok();
        let center_x = WIDTH as i32 / 2;
        text(canvas, &truncate(title, 30), Point::new(center_x, 70), &FONT_10X20, TEXT, Alignment::Center);
        text(canvas, status, Point::new(center_x, 104), &FONT_8X13, DIM, Alignment::Center);
        if let Some(fraction) = fraction {
            progress_bar(canvas, Rectangle::new(Point::new(center_x - 120, 126), Size::new(240, 14)), fraction, DIM);
        }
        canvas.present(display)
    }

    /// The WiFi screen: where to point the browser, the PIN, and what's going on.
    pub fn draw_connect(&mut self, display: &mut Display, view: &ConnectView) -> anyhow::Result<()> {
        let canvas = &mut self.body;
        canvas.clear(BACKGROUND).ok();
        let center_x = WIDTH as i32 / 2;
        text(canvas, &truncate(&view.heading, 30), Point::new(center_x, 16), &FONT_10X20, TEXT, Alignment::Center);

        // Numbered steps: a small label, and the thing to type or pick in large letters.
        let mut steps: Vec<(String, String)> = Vec::new();
        if let Some(ssid) = &view.join_wifi {
            steps.push(("Join the WiFi (no password)".into(), ssid.clone()));
        }
        if let Some(url) = &view.url {
            let label = if view.join_wifi.is_some() { "Open in a browser" } else { "In the same network, open" };
            steps.push((label.into(), url.clone()));
        }
        if let Some(pin) = &view.pin {
            let spaced: String = pin.chars().flat_map(|c| [c, ' ']).collect();
            steps.push(("Enter the PIN".into(), spaced.trim_end().to_owned()));
        }
        for (i, (label, value)) in steps.iter().enumerate() {
            let top = 36 + i as i32 * 42;
            let number = Point::new(STEP_X, top + 14);
            Circle::with_center(number, 22).into_styled(PrimitiveStyle::with_fill(SELECTION)).draw(canvas).ok();
            text(canvas, &(i + 1).to_string(), number, &FONT_8X13, TEXT, Alignment::Center);
            text(canvas, label, Point::new(STEP_X + 22, top + 4), &FONT_6X10, DIM, Alignment::Left);
            text(canvas, &truncate(value, 23), Point::new(STEP_X + 22, top + 21), &FONT_10X20, ACCENT, Alignment::Left);
        }

        if let Some((activity, color)) = &view.activity {
            for (i, line) in wrap(activity, 38, 2).iter().enumerate() {
                text(canvas, line, Point::new(center_x, 168 + i as i32 * 14), &FONT_8X13, *color, Alignment::Center);
            }
        }
        canvas.present(display)
    }

    /// The settings screen: info lines at the top, then the items to change.
    pub fn draw_settings(&mut self, display: &mut Display, view: &SettingsView) -> anyhow::Result<()> {
        let canvas = &mut self.body;
        canvas.clear(BACKGROUND).ok();
        let max_chars = (WIDTH as i32 - SETTINGS_VALUE_X - LIST_TEXT_X) as usize / FONT_8X13.character_size.width as usize;
        for (i, (label, value)) in view.info.iter().enumerate() {
            let y = 14 + i as i32 * 18;
            text(canvas, label, Point::new(LIST_TEXT_X + 4, y), &FONT_8X13, DIM, Alignment::Left);
            text(canvas, &truncate(value, max_chars), Point::new(SETTINGS_VALUE_X, y), &FONT_8X13, TEXT, Alignment::Left);
        }
        let items_top = 14 + view.info.len() as i32 * 18;
        Rectangle::new(Point::new(LIST_TEXT_X, items_top), Size::new(WIDTH - 2 * LIST_TEXT_X as u32, 1))
            .into_styled(PrimitiveStyle::with_fill(BAR))
            .draw(canvas)
            .ok();

        for (i, item) in view.items.iter().enumerate() {
            let top = items_top + 8 + i as i32 * ROW_HEIGHT as i32;
            let middle = top + ROW_HEIGHT as i32 / 2;
            if i == view.selected {
                selection(canvas, Rectangle::new(Point::new(0, top), Size::new(WIDTH, ROW_HEIGHT)));
            }
            match item {
                SettingsItem::Level { label, percent, value } => {
                    text(canvas, label, Point::new(LIST_TEXT_X + 4, middle), &FONT_8X13, TEXT, Alignment::Left);
                    let bar = Rectangle::new(Point::new(SETTINGS_VALUE_X, middle - 4), Size::new(SETTINGS_BAR_WIDTH, 9));
                    bar.into_styled(PrimitiveStyle::with_stroke(TEXT, 1)).draw(canvas).ok();
                    let filled = (SETTINGS_BAR_WIDTH - 4) * u32::from(*percent) / 100;
                    Rectangle::new(bar.top_left + Point::new(2, 2), Size::new(filled, 5))
                        .into_styled(PrimitiveStyle::with_fill(ACCENT))
                        .draw(canvas)
                        .ok();
                    let value_position = Point::new(WIDTH as i32 - LIST_TEXT_X - 4, middle);
                    text(canvas, value, value_position, &FONT_8X13, TEXT, Alignment::Right);
                }
                SettingsItem::Action { label, color, enabled } => {
                    let color = if *enabled { *color } else { DIM };
                    let max_chars = (WIDTH as i32 - 2 * (LIST_TEXT_X + 4)) as usize / FONT_8X13.character_size.width as usize;
                    let position = Point::new(LIST_TEXT_X + 4, middle);
                    text(canvas, &truncate(label, max_chars), position, &FONT_8X13, color, Alignment::Left);
                }
            }
        }
        canvas.present(display)
    }

    /// Full-body message, e.g. an error. `detail` is word-wrapped.
    pub fn draw_notice(&mut self, display: &mut Display, title: &str, detail: &str, color: Rgb565) -> anyhow::Result<()> {
        let canvas = &mut self.body;
        canvas.clear(BACKGROUND).ok();
        let center_x = WIDTH as i32 / 2;
        text(canvas, &truncate(title, 30), Point::new(center_x, 60), &FONT_10X20, color, Alignment::Center);
        for (i, line) in wrap(detail, 36, 6).iter().enumerate() {
            text(canvas, line, Point::new(center_x, 94 + i as i32 * 16), &FONT_8X13, TEXT, Alignment::Center);
        }
        canvas.present(display)
    }

    fn draw_no_games(&mut self, display: &mut Display) -> anyhow::Result<()> {
        let folder = format!("/{GAMES_DIR}/<game-id>/");
        self.draw_message(display, "No games yet", &["Copy games to", &folder, "on the SD card, then press START."])
    }

    fn draw_message(&mut self, display: &mut Display, title: &str, lines: &[&str]) -> anyhow::Result<()> {
        let canvas = &mut self.body;
        canvas.clear(BACKGROUND).ok();
        let center_x = WIDTH as i32 / 2;
        text(canvas, title, Point::new(center_x, 70), &FONT_10X20, TEXT, Alignment::Center);
        for (i, line) in lines.iter().enumerate() {
            text(canvas, line, Point::new(center_x, 104 + i as i32 * 16), &FONT_8X13, DIM, Alignment::Center);
        }
        canvas.present(display)
    }

    fn draw_list(&mut self, entries: &[Entry], state: ListState) {
        let canvas = &mut self.list;
        canvas.clear(BACKGROUND).ok();
        let max_chars = ((LIST_WIDTH - SCROLLBAR_WIDTH) as i32 - 2 * LIST_TEXT_X) as usize / FONT_8X13.character_size.width as usize;

        for (row, (index, entry)) in entries.iter().enumerate().skip(state.first).take(VISIBLE_ROWS).enumerate() {
            let top = row as i32 * ROW_HEIGHT as i32;
            if index == state.selected {
                selection(canvas, Rectangle::new(Point::new(0, top), Size::new(LIST_WIDTH - SCROLLBAR_WIDTH, ROW_HEIGHT)));
            }
            let (label, color) = match entry {
                Entry::Game(game) => (truncate(&game.name, max_chars), TEXT),
                Entry::Broken(broken) => (truncate(&format!("! {}", broken.folder), max_chars), DIM),
            };
            let position = Point::new(LIST_TEXT_X, top + ROW_HEIGHT as i32 / 2);
            text(canvas, &label, position, &FONT_8X13, color, Alignment::Left);
        }

        // Scrollbar, only when the list doesn't fit.
        if entries.len() > VISIBLE_ROWS {
            let track = BODY_HEIGHT;
            let thumb = (track * VISIBLE_ROWS as u32 / entries.len() as u32).max(8);
            let max_first = entries.len() - VISIBLE_ROWS;
            let offset = (track - thumb) * state.first as u32 / max_first as u32;
            Rectangle::new(Point::new((LIST_WIDTH - SCROLLBAR_WIDTH) as i32, offset as i32), Size::new(SCROLLBAR_WIDTH, thumb))
                .into_styled(PrimitiveStyle::with_fill(DIM))
                .draw(canvas)
                .ok();
        }
    }

    fn draw_detail(&mut self, entry: &Entry, index: usize, count: usize, icon: Option<&[u8]>) {
        let canvas = &mut self.detail;
        canvas.clear(BACKGROUND).ok();
        Rectangle::new(Point::zero(), Size::new(1, BODY_HEIGHT))
            .into_styled(PrimitiveStyle::with_fill(BAR))
            .draw(canvas)
            .ok();

        let center_x = DETAIL_WIDTH as i32 / 2;
        let icon_top_left = Point::new(center_x - ICON_SIZE as i32 / 2, 10);
        match (entry, icon) {
            (Entry::Game(_), Some(icon)) => {
                let raw = ImageRawLE::<Rgb565>::new(icon, ICON_SIZE);
                Image::new(&raw, icon_top_left).draw(canvas).ok();
            }
            (Entry::Game(game), None) => {
                let initial = game.name.chars().next().unwrap_or('?').to_ascii_uppercase().to_string();
                placeholder_icon(canvas, icon_top_left, &initial, DIM);
            }
            (Entry::Broken(_), _) => placeholder_icon(canvas, icon_top_left, "!", WARNING),
        }

        let mut y = 10 + ICON_SIZE as i32 + 14;
        for line in wrap(entry.title(), 15, 3) {
            text(canvas, &line, Point::new(center_x, y), &FONT_8X13, TEXT, Alignment::Center);
            y += 15;
        }
        y += 4;
        match entry {
            Entry::Game(game) => {
                text(canvas, &format!("v{}", game.version), Point::new(center_x, y), &FONT_6X10, DIM, Alignment::Center);
                if !game.author.is_empty() {
                    for line in wrap(&format!("by {}", game.author), 20, 2) {
                        y += 12;
                        text(canvas, &line, Point::new(center_x, y), &FONT_6X10, DIM, Alignment::Center);
                    }
                }
            }
            Entry::Broken(broken) => {
                text(canvas, "Can't be used:", Point::new(center_x, y), &FONT_6X10, WARNING, Alignment::Center);
                for line in wrap(&broken.reason, 20, 5) {
                    y += 12;
                    text(canvas, &line, Point::new(center_x, y), &FONT_6X10, WARNING, Alignment::Center);
                }
            }
        }

        let position = format!("{} / {count}", index + 1);
        text(canvas, &position, Point::new(center_x, BODY_HEIGHT as i32 - 10), &FONT_6X10, DIM, Alignment::Center);
    }
}

/// The ogo mascot and wordmark (tools/make_logo.py): RGB565 little-endian.
const LOGO: &[u8] = include_bytes!("../../assets/logo.rgb565");
const LOGO_WIDTH: u32 = 200;
const LOGO_HEIGHT: u32 = 184;

/// Logo and version on black, shown from power-on until the game list is ready.
pub fn draw_boot_screen(display: &mut Display) -> anyhow::Result<()> {
    let raw = ImageRawLE::<Rgb565>::new(LOGO, LOGO_WIDTH);
    let top_left = Point::new((WIDTH - LOGO_WIDTH) as i32 / 2, (HEIGHT - LOGO_HEIGHT) as i32 / 2 - 8);
    Image::new(&raw, top_left).draw(display).display_err()?;
    let version = concat!("launcher v", env!("CARGO_PKG_VERSION"));
    let style = TextStyleBuilder::new().alignment(Alignment::Center).baseline(Baseline::Middle).build();
    Text::with_text_style(version, Point::new(WIDTH as i32 / 2, HEIGHT as i32 - 14), MonoTextStyle::new(&FONT_6X10, DIM), style)
        .draw(display)
        .display_err()?;
    Ok(())
}

/// A frame with a big character, for games without an icon and for broken entries.
fn placeholder_icon(canvas: &mut Canvas, top_left: Point, label: &str, color: Rgb565) {
    let style = PrimitiveStyleBuilder::new().stroke_color(color).stroke_width(1).fill_color(BAR).build();
    Rectangle::new(top_left, Size::new(ICON_SIZE, ICON_SIZE)).into_styled(style).draw(canvas).ok();
    let center = top_left + Point::new(ICON_SIZE as i32 / 2, ICON_SIZE as i32 / 2);
    text(canvas, label, center, &FONT_10X20, color, Alignment::Center);
}
