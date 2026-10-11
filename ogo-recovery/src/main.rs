//! ogo-recovery: the fallback in the factory slot, for when the launcher itself is broken.
//!
//! Holding MENU for a second at power-on makes the bootloader start it (see sdkconfig.defaults).
//! It can start either launcher slot or write `/ogo/updates/launcher.bin` from the SD card into
//! `launcher_a`. If it was started without MENU (the boot selection was empty, e.g. after flashing
//! with `--erase-parts otadata`), it starts the first valid launcher right away.

use std::path::PathBuf;

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_6X10, FONT_8X13},
        MonoFont, MonoTextStyle,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use esp_idf_svc::hal::delay::FreeRtos;
use ogo_board::{
    boot,
    display::{HEIGHT, WIDTH},
    flash::{self, AppInfo, Progress},
    Board, Button, ButtonState, Canvas,
};

const LAUNCHERS: [(&str, &str); 2] = [("launcher_a", "A"), ("launcher_b", "B")];
/// Where "Install launcher from SD card" looks, relative to the card root.
const UPDATE_PATH: &str = "ogo/updates/launcher.bin";
/// The slot "Install launcher from SD card" writes to.
const INSTALL_SLOT: &str = "launcher_a";

const BACKGROUND: Rgb565 = Rgb565::new(6, 2, 2);
const BAR: Rgb565 = Rgb565::new(12, 4, 4);
const SELECTION: Rgb565 = Rgb565::CSS_STEEL_BLUE;
const TEXT: Rgb565 = Rgb565::WHITE;
const DIM: Rgb565 = Rgb565::CSS_SILVER;
const WARNING: Rgb565 = Rgb565::CSS_ORANGE;

const TOP_BAR_HEIGHT: i32 = 26;
const ROW_HEIGHT: i32 = 40;
const ROWS_TOP: i32 = 36;

extern "C" {
    /// True once after the bootloader started this app because MENU was held (clears the flag).
    fn bootloader_common_get_rtc_retain_mem_factory_reset_state() -> bool;
}

#[derive(Debug, Clone)]
enum Item {
    Start { label: &'static str, name: &'static str, info: Option<AppInfo> },
    Install { available: bool },
    Restart,
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let menu_boot = unsafe { bootloader_common_get_rtc_retain_mem_factory_reset_state() };
    log::info!("ogo-recovery, started {}", if menu_boot { "with MENU" } else { "without MENU" });
    if !menu_boot {
        // Nothing asked for recovery: carry on with a launcher if there is one.
        for (label, _) in LAUNCHERS {
            if flash::app_info(label).is_some() && boot::set_boot_partition(label).is_ok() {
                log::info!("Starting {label}");
                boot::restart();
            }
        }
        log::warn!("No valid launcher, staying in recovery");
    }

    Recovery::new(Board::take()?).run()
}

struct Recovery {
    board: Board,
    canvas: Canvas,
    items: Vec<Item>,
    selected: usize,
    status: Option<(String, Rgb565)>,
}

impl Recovery {
    fn new(board: Board) -> Self {
        let screen = Rectangle::new(Point::zero(), Size::new(WIDTH.into(), HEIGHT.into()));
        let mut recovery = Self { board, canvas: Canvas::new(screen), items: Vec::new(), selected: 0, status: None };
        recovery.refresh_items();
        recovery
    }

    fn update_path(&self) -> Option<PathBuf> {
        self.board.sd.as_ref().map(|sd| PathBuf::from(sd.path(UPDATE_PATH)))
    }

    fn refresh_items(&mut self) {
        let mut items: Vec<Item> = LAUNCHERS
            .iter()
            .map(|&(label, name)| Item::Start { label, name, info: flash::app_info(label) })
            .collect();
        items.push(Item::Install { available: self.update_path().is_some_and(|path| path.is_file()) });
        items.push(Item::Restart);
        self.items = items;
    }

    fn run(mut self) -> anyhow::Result<()> {
        self.draw()?;
        let mut previous = ButtonState::default();
        loop {
            let buttons = self.board.buttons.read()?;
            let pressed = |button| buttons.just_pressed(previous, button);
            if pressed(Button::Up) {
                self.selected = (self.selected + self.items.len() - 1) % self.items.len();
                self.draw()?;
            } else if pressed(Button::Down) {
                self.selected = (self.selected + 1) % self.items.len();
                self.draw()?;
            } else if pressed(Button::A) {
                self.activate()?;
            }
            previous = buttons;
            FreeRtos::delay_ms(20);
        }
    }

    fn activate(&mut self) -> anyhow::Result<()> {
        match self.items[self.selected].clone() {
            Item::Start { label, name, info } => {
                if info.is_none() {
                    return self.set_status(&format!("Launcher {name} is empty"), WARNING);
                }
                match boot::set_boot_partition(label) {
                    Ok(()) => boot::restart(),
                    Err(e) => self.set_status(&format!("{e:#}"), WARNING),
                }
            }
            Item::Install { available } => {
                if !available {
                    return self.set_status(&format!("Copy launcher.bin to /{UPDATE_PATH}"), WARNING);
                }
                self.install()
            }
            Item::Restart => boot::restart(),
        }
    }

    fn install(&mut self) -> anyhow::Result<()> {
        let path = self.update_path().expect("checked when listing the item");
        // Writing flash stalls the audio DMA, which would click.
        self.board.speaker.set_muted(true);
        let (canvas, display) = (&mut self.canvas, &mut self.board.display);
        let mut shown = None;
        let result = flash::write_image(&path, INSTALL_SLOT, None, |progress| {
            let percent = match progress {
                Progress::Erasing => None,
                Progress::Writing { written, total } => Some((written * 100 / total.max(1)) as u32),
            };
            if shown == Some(percent) {
                return;
            }
            shown = Some(percent);
            draw_progress(canvas, percent);
            if let Err(e) = canvas.present(display) {
                log::warn!("Drawing the progress failed: {e:?}");
            }
        });
        self.board.speaker.set_muted(false);

        match result.and_then(|()| boot::set_boot_partition(INSTALL_SLOT)) {
            Ok(()) => boot::restart(),
            Err(e) => {
                log::error!("Installing the launcher failed: {e:?}");
                self.refresh_items();
                self.set_status(&format!("{e:#}"), WARNING)
            }
        }
    }

    fn set_status(&mut self, status: &str, color: Rgb565) -> anyhow::Result<()> {
        self.status = Some((status.to_owned(), color));
        self.draw()
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        let canvas = &mut self.canvas;
        canvas.clear(BACKGROUND).ok();
        let width = WIDTH as i32;

        Rectangle::new(Point::zero(), Size::new(WIDTH.into(), TOP_BAR_HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(BAR))
            .draw(canvas)
            .ok();
        text(canvas, "Recovery", Point::new(8, TOP_BAR_HEIGHT / 2), &FONT_10X20, TEXT, Alignment::Left);

        for (i, item) in self.items.iter().enumerate() {
            let top = ROWS_TOP + i as i32 * ROW_HEIGHT;
            if i == self.selected {
                Rectangle::new(Point::new(4, top), Size::new(WIDTH as u32 - 8, ROW_HEIGHT as u32 - 4))
                    .into_styled(PrimitiveStyle::with_fill(SELECTION))
                    .draw(canvas)
                    .ok();
            }
            let (title, detail) = describe(item);
            text(canvas, &title, Point::new(12, top + 11), &FONT_8X13, TEXT, Alignment::Left);
            text(canvas, &detail, Point::new(12, top + 26), &FONT_6X10, DIM, Alignment::Left);
        }

        if let Some((status, color)) = &self.status {
            text(canvas, status, Point::new(width / 2, HEIGHT as i32 - 34), &FONT_6X10, *color, Alignment::Center);
        }
        text(canvas, "Up/Down select   A confirm", Point::new(width / 2, HEIGHT as i32 - 12), &FONT_6X10, DIM, Alignment::Center);
        canvas.present(&mut self.board.display)
    }
}

fn describe(item: &Item) -> (String, String) {
    match item {
        Item::Start { name, info: Some(info), .. } => {
            (format!("Start launcher {name}"), format!("{} {}, built {}", info.project, info.version, info.built))
        }
        Item::Start { name, info: None, .. } => (format!("Start launcher {name}"), "empty".to_owned()),
        Item::Install { available: true } => {
            ("Install launcher from SD card".to_owned(), format!("/{UPDATE_PATH} into slot A"))
        }
        Item::Install { available: false } => {
            ("Install launcher from SD card".to_owned(), format!("no /{UPDATE_PATH} found"))
        }
        Item::Restart => ("Restart".to_owned(), "starts the first working launcher".to_owned()),
    }
}

fn draw_progress(canvas: &mut Canvas, percent: Option<u32>) {
    canvas.clear(BACKGROUND).ok();
    let center_x = WIDTH as i32 / 2;
    text(canvas, "Installing launcher", Point::new(center_x, 80), &FONT_10X20, TEXT, Alignment::Center);
    let status = percent.map_or("Preparing...".to_owned(), |p| format!("{p} %"));
    text(canvas, &status, Point::new(center_x, 112), &FONT_8X13, DIM, Alignment::Center);
    let bar = Rectangle::new(Point::new(center_x - 120, 132), Size::new(240, 14));
    bar.into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(canvas).ok();
    let filled = 236 * percent.unwrap_or(0) / 100;
    Rectangle::new(bar.top_left + Point::new(2, 2), Size::new(filled, 10))
        .into_styled(PrimitiveStyle::with_fill(SELECTION))
        .draw(canvas)
        .ok();
}

fn text(canvas: &mut Canvas, text: &str, position: Point, font: &MonoFont<'_>, color: Rgb565, alignment: Alignment) {
    let style = TextStyleBuilder::new().alignment(alignment).baseline(Baseline::Middle).build();
    Text::with_text_style(text, position, MonoTextStyle::new(font, color), style).draw(canvas).ok();
}
