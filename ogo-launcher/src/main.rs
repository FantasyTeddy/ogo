//! ogo-launcher: lists the apps on the SD card, installs the selected one into the app slot
//! and boots it.
//!
//! SELECT opens the WiFi screen with the web page (see `web`), MENU the settings screen. Hold
//! SELECT at power-on for the hardware test screen.

mod connect;
mod hwtest;
mod input;
mod install;
mod library;
mod network;
mod package;
mod settings;
mod settings_screen;
mod ui;
mod web;

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use esp_idf_svc::{hal::delay::FreeRtos, nvs::EspDefaultNvs};
use ogo_board::{
    boot::{Boot, UpdateResult},
    BatteryLevel, Board, Button, Note, Settings,
};

use input::{Nav, Navigator};
use library::{App, Entry, Library};
use ogo_board::flash::Progress;
use ui::{ListState, Ui, VISIBLE_ROWS};

ogo_board::image_desc!(Launcher);

const POLL_MS: u32 = 20;
/// The boot screen stays at least this long, unless a button is pressed.
const BOOT_SCREEN_MIN: Duration = Duration::from_millis(1500);
const BATTERY_INTERVAL: Duration = Duration::from_secs(5);
const MESSAGE_DURATION: Duration = Duration::from_secs(2);
const SOUND_ERROR: &[Note] = &[Note::new(440, 90), Note::new(330, 140)];
const SOUND_UPDATED: &[Note] = &[Note::new(523, 80), Note::new(784, 80), Note::new(1047, 140)];

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let boot = Boot::take()?;
    let nvs = Settings::open_nvs(boot.nvs_partition())?;
    let settings = Settings::load(&nvs);
    let mut boot_screen_shown = Instant::now();
    let mut board = Board::take_with(&settings, |display| {
        ui::draw_boot_screen(display)?;
        boot_screen_shown = Instant::now();
        Ok(())
    })?;
    if board.buttons.read()?.is_held(Button::Select) {
        ogo_board::boot::mark_running_valid();
        return hwtest::run(&mut board);
    }

    // Self-check passed (display and SD card set up): keep this slot. A launcher update that
    // crashes before this point is rolled back by the bootloader on the next reset.
    #[cfg(feature = "rollback-test")]
    panic!("rollback-test build: crashing on purpose before confirming the slot");
    ogo_board::boot::mark_running_valid();

    log::info!(
        "ogo-launcher, running from {}",
        ogo_board::boot::running_partition_label()
    );
    let mut launcher = Launcher::new(board, boot, nvs, settings);
    launcher.wait_for_boot_screen(boot_screen_shown)?;
    launcher.run()
}

/// What the launcher shows. Each screen handles its buttons in its own module.
enum Screen {
    /// The app list.
    Apps,
    /// A message over the app list, until the next button press.
    Notice,
    /// WiFi and the web page (`connect`).
    Wifi(connect::Session),
    /// The settings (`settings_screen`).
    Settings(settings_screen::State),
}

struct Launcher {
    screen: Screen,
    board: Board,
    boot: Boot,
    ui: Ui,
    library: Library,
    list: ListState,
    /// Icons by entry index; `None` if the entry has no icon.
    icons: HashMap<usize, Option<Vec<u8>>>,
    /// Footer message and when it disappears.
    message: Option<Instant>,
    battery: Option<BatteryLevel>,
    /// The `ogo` NVS namespace: settings and the WiFi network.
    nvs: EspDefaultNvs,
    /// As currently applied; saved when the settings screen closes.
    settings: Settings,
    /// Created the first time the WiFi screen opens.
    network: Option<network::Network>,
}

impl Launcher {
    /// Scans the SD card; the boot screen stays up meanwhile.
    fn new(mut board: Board, boot: Boot, nvs: EspDefaultNvs, settings: Settings) -> Self {
        let library = library::scan(board.sd.as_ref());
        let battery = board.battery.read().ok();
        Self {
            screen: Screen::Apps,
            board,
            boot,
            ui: Ui::new(),
            library,
            list: ListState::default(),
            icons: HashMap::new(),
            message: None,
            battery,
            nvs,
            settings,
            network: None,
        }
    }

    /// Tells whether a launcher update from the web page worked or was rolled back.
    fn report_update(&mut self) -> anyhow::Result<()> {
        match self.boot.take_update_result()? {
            None => Ok(()),
            Some(UpdateResult::Installed) => {
                self.board.speaker.play(SOUND_UPDATED);
                self.show_message(concat!("Launcher updated to v", env!("CARGO_PKG_VERSION")))
            }
            Some(UpdateResult::RolledBack { slot }) => {
                log::warn!("The update in {slot} was rolled back");
                self.board.speaker.play(SOUND_ERROR);
                self.screen = Screen::Notice;
                let detail = format!(
                    "The new launcher in {slot} stopped before it was ready, so the device went back to this one."
                );
                self.ui.draw_notice(
                    &mut self.board.display,
                    "Update didn't start",
                    &detail,
                    ui::WARNING,
                )?;
                self.show_message("Press any button")
            }
        }
    }

    /// Keeps the boot screen up until [`BOOT_SCREEN_MIN`] has passed or a button is pressed.
    fn wait_for_boot_screen(&mut self, shown: Instant) -> anyhow::Result<()> {
        while shown.elapsed() < BOOT_SCREEN_MIN {
            if self.board.buttons.read()?.any() {
                break;
            }
            FreeRtos::delay_ms(POLL_MS);
        }
        Ok(())
    }

    fn run(mut self) -> anyhow::Result<()> {
        self.draw_top_bar()?;
        self.draw_footer(None)?;
        self.draw_library()?;
        self.report_update()?;

        let mut navigator = Navigator::default();
        let mut next_battery_read = Instant::now() + BATTERY_INTERVAL;
        loop {
            let now = Instant::now();
            if let Some(nav) = navigator.update(self.board.buttons.read()?, now) {
                match self.screen {
                    _ if nav == Nav::Mute => self.toggle_mute()?,
                    Screen::Apps => self.handle_apps(nav)?,
                    Screen::Notice => self.dismiss_notice()?,
                    Screen::Wifi(_) => self.handle_connect(nav)?,
                    Screen::Settings(_) => self.handle_settings(nav)?,
                }
            }
            self.poll_connect()?;

            if self.message.is_some_and(|until| now >= until) {
                self.message = None;
                self.draw_footer(None)?;
            }
            if now >= next_battery_read {
                next_battery_read = now + BATTERY_INTERVAL;
                let level = self.board.battery.read().ok();
                if level != self.battery {
                    self.battery = level;
                    self.draw_top_bar()?;
                }
            }

            FreeRtos::delay_ms(POLL_MS);
        }
    }

    fn dismiss_notice(&mut self) -> anyhow::Result<()> {
        self.screen = Screen::Apps;
        self.draw_library()
    }

    fn handle_apps(&mut self, nav: Nav) -> anyhow::Result<()> {
        let entries = self.library.entries();
        let page = VISIBLE_ROWS as isize;
        let before = self.list;
        match nav {
            Nav::Up => self.list.move_by(-1, entries, true),
            Nav::Down => self.list.move_by(1, entries, true),
            Nav::Left => self.list.move_by(-page, entries, false),
            Nav::Right => self.list.move_by(page, entries, false),
            Nav::Select => return self.select(),
            Nav::Rescan => return self.rescan(),
            Nav::Connect => return self.open_connect(),
            Nav::Settings => return self.open_settings(),
            Nav::Back | Nav::Mute => {}
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
        let app = match entry {
            Entry::App(app) => app.clone(),
            Entry::Broken(_) => {
                self.board.speaker.play(SOUND_ERROR);
                return self.show_message("This app can't be used");
            }
        };
        // Writing flash stalls the audio DMA, which would click; launching restarts anyway.
        self.board.speaker.set_muted(true);
        let result = self.launch(&app);
        self.board.speaker.set_muted(false);
        if let Err(e) = result {
            log::error!("Launching {} failed: {e:?}", app.id);
            self.board.speaker.play(SOUND_ERROR);
            self.screen = Screen::Notice;
            let detail = format!("{e:#}");
            self.ui.draw_notice(
                &mut self.board.display,
                &format!("Can't start {}", app.name),
                &detail,
                ui::WARNING,
            )?;
            self.show_message("Press any button")?;
        }
        Ok(())
    }

    /// Installs the app if the slot holds something else, then boots it.
    /// Only returns if that fails.
    fn launch(&mut self, app: &App) -> anyhow::Result<()> {
        let title = format!("Installing {}", app.name);
        let (ui, display) = (&mut self.ui, &mut self.board.display);
        // Redraw only when the shown percentage changes.
        let mut shown = None;
        let installed = install::install(app, &mut self.boot, |progress| {
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
            self.ui.draw_progress(
                &mut self.board.display,
                &format!("Starting {}", app.name),
                "",
                None,
            )?;
        }
        self.boot.launch_app()
    }

    fn rescan(&mut self) -> anyhow::Result<()> {
        self.library = library::scan(self.board.sd.as_ref());
        self.icons.clear();
        // Keep the selection where it was, as far as the new list allows.
        self.list.move_by(0, self.library.entries(), false);
        self.draw_library()?;
        let apps = self
            .library
            .entries()
            .iter()
            .filter(|e| matches!(e, Entry::App(_)))
            .count();
        self.show_message(&format!("Found {apps} apps"))
    }

    fn show_message(&mut self, message: &str) -> anyhow::Result<()> {
        self.message = Some(Instant::now() + MESSAGE_DURATION);
        self.draw_footer(Some(message))
    }

    /// The current screen's controls, or `message` in their place.
    fn draw_footer(&mut self, message: Option<&str>) -> anyhow::Result<()> {
        let controls = match &self.screen {
            Screen::Apps | Screen::Notice => ui::APPS_CONTROLS,
            Screen::Wifi(_) => connect::CONTROLS,
            Screen::Settings(state) => state.controls(),
        };
        self.ui
            .draw_footer(&mut self.board.display, controls, message)
    }

    fn draw_top_bar(&mut self) -> anyhow::Result<()> {
        let title = self.title();
        self.ui
            .draw_top_bar(&mut self.board.display, title, self.battery)
    }

    fn title(&self) -> &'static str {
        match self.screen {
            Screen::Apps | Screen::Notice => "Apps",
            Screen::Wifi(_) => "WiFi",
            Screen::Settings(_) => "Settings",
        }
    }

    fn draw_library(&mut self) -> anyhow::Result<()> {
        let icon = match &self.library {
            Library::Entries(entries) => match entries.get(self.list.selected) {
                Some(Entry::App(app)) => self
                    .icons
                    .entry(self.list.selected)
                    .or_insert_with(|| app.load_icon())
                    .as_deref(),
                _ => None,
            },
            _ => None,
        };
        self.ui
            .draw_library(&mut self.board.display, &self.library, self.list, icon)
    }
}
