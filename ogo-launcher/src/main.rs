//! ogo-launcher: lists the games on the SD card. Launching them comes in M3.
//!
//! Hold SELECT at power-on for the hardware test screen.

mod hwtest;
mod input;
mod library;
mod ui;

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use esp_idf_svc::hal::delay::FreeRtos;
use ogo_board::{Board, Button, Note};

use input::{Nav, Navigator};
use library::{Entry, Library};
use ui::{ListState, Ui, VISIBLE_ROWS};

const POLL_MS: u32 = 20;
const BATTERY_INTERVAL: Duration = Duration::from_secs(5);
const MESSAGE_DURATION: Duration = Duration::from_secs(2);
const SOUND_OK: &[Note] = &[Note::new(880, 40), Note::new(1320, 60)];
const SOUND_ERROR: &[Note] = &[Note::new(440, 90), Note::new(330, 140)];

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut board = Board::take()?;
    if board.buttons.read()?.is_held(Button::Select) {
        return hwtest::run(&mut board);
    }

    log::info!("ogo-launcher");
    Launcher::new(board)?.run()
}

struct Launcher {
    board: Board,
    ui: Ui,
    library: Library,
    list: ListState,
    /// Icons by entry index; `None` if the entry has no icon.
    icons: HashMap<usize, Option<Vec<u8>>>,
    /// Footer message and when it disappears.
    message: Option<Instant>,
}

impl Launcher {
    fn new(board: Board) -> anyhow::Result<Self> {
        let library = library::scan(board.sd.as_ref());
        Ok(Self { board, ui: Ui::new(), library, list: ListState::default(), icons: HashMap::new(), message: None })
    }

    fn run(mut self) -> anyhow::Result<()> {
        let mut battery = self.board.battery.read().ok();
        self.ui.draw_top_bar(&mut self.board.display, battery)?;
        self.ui.draw_footer(&mut self.board.display, None)?;
        self.draw_library()?;

        let mut navigator = Navigator::default();
        let mut next_battery_read = Instant::now() + BATTERY_INTERVAL;
        loop {
            let now = Instant::now();
            if let Some(nav) = navigator.update(self.board.buttons.read()?, now) {
                self.handle(nav)?;
            }

            if self.message.is_some_and(|until| now >= until) {
                self.message = None;
                self.ui.draw_footer(&mut self.board.display, None)?;
            }
            if now >= next_battery_read {
                next_battery_read = now + BATTERY_INTERVAL;
                let level = self.board.battery.read().ok();
                if level != battery {
                    battery = level;
                    self.ui.draw_top_bar(&mut self.board.display, battery)?;
                }
            }

            FreeRtos::delay_ms(POLL_MS);
        }
    }

    fn handle(&mut self, nav: Nav) -> anyhow::Result<()> {
        let len = match &self.library {
            Library::Entries(entries) => entries.len(),
            _ => 0,
        };
        let page = VISIBLE_ROWS as isize;
        let before = self.list;
        match nav {
            Nav::Up => self.list.move_by(-1, len, true),
            Nav::Down => self.list.move_by(1, len, true),
            Nav::PageUp => self.list.move_by(-page, len, false),
            Nav::PageDown => self.list.move_by(page, len, false),
            Nav::Select => return self.select(),
            Nav::Rescan => return self.rescan(),
        }
        if self.list != before {
            self.draw_library()?;
        }
        Ok(())
    }

    fn select(&mut self) -> anyhow::Result<()> {
        let Library::Entries(entries) = &self.library else {
            return Ok(());
        };
        let Some(entry) = entries.get(self.list.selected) else {
            return Ok(());
        };
        let message = match entry {
            Entry::Game(game) => format!("Launching {} comes in M3", game.name),
            Entry::Broken(_) => "This game can't be used".to_owned(),
        };
        let sound = if matches!(entry, Entry::Game(_)) { SOUND_OK } else { SOUND_ERROR };
        self.show_message(&message)?;
        self.board.speaker.play(sound);
        Ok(())
    }

    fn rescan(&mut self) -> anyhow::Result<()> {
        self.library = library::scan(self.board.sd.as_ref());
        self.icons.clear();
        let len = match &self.library {
            Library::Entries(entries) => entries.len(),
            _ => 0,
        };
        // Keep the selection where it was, as far as the new list allows.
        self.list.move_by(0, len, false);
        self.draw_library()?;
        let games = match &self.library {
            Library::Entries(entries) => entries.iter().filter(|e| matches!(e, Entry::Game(_))).count(),
            _ => 0,
        };
        self.show_message(&format!("Found {games} games"))
    }

    fn show_message(&mut self, message: &str) -> anyhow::Result<()> {
        self.message = Some(Instant::now() + MESSAGE_DURATION);
        self.ui.draw_footer(&mut self.board.display, Some(message))
    }

    fn draw_library(&mut self) -> anyhow::Result<()> {
        let icon = match &self.library {
            Library::Entries(entries) => match entries.get(self.list.selected) {
                Some(Entry::Game(game)) => {
                    self.icons.entry(self.list.selected).or_insert_with(|| game.load_icon()).as_deref()
                }
                _ => None,
            },
            _ => None,
        };
        self.ui.draw_library(&mut self.board.display, &self.library, self.list, icon)
    }
}
