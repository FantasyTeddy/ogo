//! ogo-launcher: lists the games on the SD card, installs the selected one into the game slot
//! and boots it.
//!
//! Hold SELECT at power-on for the hardware test screen.

mod hwtest;
mod input;
mod install;
mod library;
mod ui;

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use esp_idf_svc::hal::delay::FreeRtos;
use ogo_board::{boot::Boot, Board, Button, Note};

use input::{Nav, Navigator};
use install::Progress;
use library::{Entry, Game, Library};
use ui::{ListState, Ui, VISIBLE_ROWS};

const POLL_MS: u32 = 20;
const BATTERY_INTERVAL: Duration = Duration::from_secs(5);
const MESSAGE_DURATION: Duration = Duration::from_secs(2);
const SOUND_ERROR: &[Note] = &[Note::new(440, 90), Note::new(330, 140)];

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let boot = Boot::take()?;
    let mut board = Board::take()?;
    if board.buttons.read()?.is_held(Button::Select) {
        return hwtest::run(&mut board);
    }

    log::info!("ogo-launcher, running from {}", ogo_board::boot::running_partition_label());
    Launcher::new(board, boot)?.run()
}

struct Launcher {
    board: Board,
    boot: Boot,
    ui: Ui,
    library: Library,
    list: ListState,
    /// Icons by entry index; `None` if the entry has no icon.
    icons: HashMap<usize, Option<Vec<u8>>>,
    /// Footer message and when it disappears.
    message: Option<Instant>,
    /// A notice covers the list until the next button press.
    notice: bool,
}

impl Launcher {
    fn new(board: Board, boot: Boot) -> anyhow::Result<Self> {
        let library = library::scan(board.sd.as_ref());
        Ok(Self {
            board,
            boot,
            ui: Ui::new(),
            library,
            list: ListState::default(),
            icons: HashMap::new(),
            message: None,
            notice: false,
        })
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
        if self.notice {
            self.notice = false;
            return self.draw_library();
        }
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
        let game = match entry {
            Entry::Game(game) => game.clone(),
            Entry::Broken(_) => {
                self.board.speaker.play(SOUND_ERROR);
                return self.show_message("This game can't be used");
            }
        };
        // Writing flash stalls the audio DMA, which would click; launching restarts anyway.
        self.board.speaker.set_muted(true)?;
        let result = self.launch(&game);
        self.board.speaker.set_muted(false)?;
        if let Err(e) = result {
            log::error!("Launching {} failed: {e:?}", game.id);
            self.board.speaker.play(SOUND_ERROR);
            self.notice = true;
            let detail = format!("{e:#}");
            self.ui.draw_notice(&mut self.board.display, &format!("Can't start {}", game.name), &detail, ui::WARNING)?;
            self.show_message("Press any button")?;
        }
        Ok(())
    }

    /// Installs the game if the slot holds something else, then boots it.
    /// Only returns if that fails.
    fn launch(&mut self, game: &Game) -> anyhow::Result<()> {
        let title = format!("Installing {}", game.name);
        let (ui, display) = (&mut self.ui, &mut self.board.display);
        // Redraw only when the shown percentage changes.
        let mut shown = None;
        let installed = install::install(game, &mut self.boot, |progress| {
            let (status, fraction) = match progress {
                Progress::Erasing => ("Preparing...".to_owned(), None),
                Progress::Writing { written, total } => {
                    let percent = (written * 100 / total.max(1)) as u32;
                    (format!("{percent} %"), Some(percent))
                }
            };
            if shown == Some(fraction) {
                return;
            }
            shown = Some(fraction);
            let fraction = fraction.map(|percent| percent as f32 / 100.0);
            if let Err(e) = ui.draw_progress(display, &title, &status, fraction) {
                log::warn!("Drawing the progress failed: {e:?}");
            }
        })?;
        if !installed {
            self.ui.draw_progress(&mut self.board.display, &format!("Starting {}", game.name), "", None)?;
        }
        self.boot.launch_game()
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
