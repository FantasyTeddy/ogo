//! The settings screen (MENU): device info at the top, then brightness, volume and forgetting the
//! WiFi network. Brightness and volume apply right away and are saved to NVS when leaving.

use ogo_board::{boot, flash, image, sd, settings, Note, Settings};

use crate::{
    input::Nav,
    settings::{self as wifi, WifiSource},
    ui::{SettingsItem, SettingsView, TEXT, WARNING},
    Launcher, SOUND_ERROR,
};

/// Played when the volume changes, at the new volume.
const SOUND_VOLUME: &[Note] = &[Note::new(660, 40)];
const SOUND_FORGOTTEN: &[Note] = &[Note::new(660, 60), Note::new(440, 100)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Brightness,
    Volume,
    ForgetWifi,
}

const ITEMS: [Item; 3] = [Item::Brightness, Item::Volume, Item::ForgetWifi];

pub struct Screen {
    selected: usize,
    /// The settings as in NVS, to tell whether leaving needs to save.
    saved: Settings,
    /// Launcher and build lines of the info section; computing the build ID takes a moment, so
    /// only once.
    launcher_info: (String, String),
    /// A was pressed once on "Forget WiFi network"; the next A forgets.
    confirm_forget: bool,
}

impl Launcher {
    pub(crate) fn open_settings(&mut self) -> anyhow::Result<()> {
        self.settings_screen = Some(Screen {
            selected: 0,
            saved: self.settings,
            launcher_info: launcher_info(),
            confirm_forget: false,
        });
        self.draw_top_bar()?;
        self.draw_footer(None)?;
        self.draw_settings()
    }

    /// Saves changed settings and goes back to the game list.
    fn close_settings(&mut self) -> anyhow::Result<()> {
        if let Some(screen) = self.settings_screen.take() {
            if self.settings != screen.saved {
                // Writing flash stalls the audio DMA, which would click.
                self.board.speaker.set_muted(true);
                let result = self.settings.save(&self.nvs);
                self.board.speaker.set_muted(false);
                if let Err(e) = result {
                    log::error!("Saving the settings failed: {e:?}");
                    self.board.speaker.play(SOUND_ERROR);
                }
            }
        }
        self.draw_top_bar()?;
        self.draw_footer(None)?;
        self.draw_library()
    }

    pub(crate) fn handle_settings(&mut self, nav: Nav) -> anyhow::Result<()> {
        let Some(screen) = &mut self.settings_screen else {
            return Ok(());
        };
        let item = ITEMS[screen.selected];
        match nav {
            Nav::Back | Nav::Settings => return self.close_settings(),
            Nav::Up | Nav::Down => {
                let delta = if nav == Nav::Up { ITEMS.len() - 1 } else { 1 };
                screen.selected = (screen.selected + delta) % ITEMS.len();
                screen.confirm_forget = false;
                self.draw_footer(None)?;
            }
            Nav::Left | Nav::Right => {
                let delta = if nav == Nav::Left { -1 } else { 1 };
                match item {
                    Item::Brightness => {
                        let brightness = settings::step(&settings::BRIGHTNESS_STEPS, self.settings.brightness, delta);
                        if brightness == self.settings.brightness {
                            return Ok(());
                        }
                        self.settings.brightness = brightness;
                        self.board.backlight.set_brightness(brightness)?;
                    }
                    Item::Volume => {
                        let volume = settings::step(&settings::VOLUME_STEPS, self.settings.volume, delta);
                        if volume == self.settings.volume {
                            return Ok(());
                        }
                        self.settings.volume = volume;
                        self.board.speaker.set_volume(volume);
                        self.board.speaker.play(SOUND_VOLUME);
                    }
                    Item::ForgetWifi => return Ok(()),
                }
            }
            Nav::Select if item == Item::ForgetWifi => return self.forget_wifi(),
            Nav::Select | Nav::Rescan | Nav::Connect => return Ok(()),
        }
        self.draw_settings()
    }

    /// The first A asks for confirmation, the second forgets the network in NVS.
    fn forget_wifi(&mut self) -> anyhow::Result<()> {
        let Some(screen) = &mut self.settings_screen else {
            return Ok(());
        };
        match wifi::saved_wifi(&self.nvs, self.board.sd.as_ref()) {
            None => return self.show_message("No WiFi network saved"),
            Some((_, WifiSource::SdCard)) => {
                self.board.speaker.play(SOUND_ERROR);
                return self.show_message("It's in /ogo/settings.toml on the SD card");
            }
            Some((credentials, WifiSource::Nvs)) if screen.confirm_forget => {
                screen.confirm_forget = false;
                self.board.speaker.set_muted(true);
                let result = wifi::forget_wifi(&self.nvs);
                self.board.speaker.set_muted(false);
                match result {
                    Ok(()) => {
                        self.board.speaker.play(SOUND_FORGOTTEN);
                        self.show_message(&format!("Forgot {}", credentials.ssid))?;
                    }
                    Err(e) => {
                        log::error!("Forgetting the WiFi network failed: {e:?}");
                        self.board.speaker.play(SOUND_ERROR);
                        self.show_message("Forgetting the network failed")?;
                    }
                }
            }
            Some(_) => screen.confirm_forget = true,
        }
        self.draw_settings()
    }

    fn draw_settings(&mut self) -> anyhow::Result<()> {
        let Some(screen) = &self.settings_screen else {
            return Ok(());
        };
        let saved = wifi::saved_wifi(&self.nvs, self.board.sd.as_ref());
        let wifi_info = match &saved {
            None => "None saved".to_owned(),
            Some((credentials, WifiSource::Nvs)) => credentials.ssid.clone(),
            Some((credentials, WifiSource::SdCard)) => format!("{} (SD card)", credentials.ssid),
        };
        let sd_info = match self.board.sd.as_ref().and_then(|_| sd::space()) {
            Some((total, free)) => format!("{} free of {}", format_bytes(free), format_bytes(total)),
            None => "No card".to_owned(),
        };
        let forget = match &saved {
            Some((credentials, WifiSource::Nvs)) if screen.confirm_forget => SettingsItem::Action {
                label: format!("Press A again to forget {}", credentials.ssid),
                color: WARNING,
                enabled: true,
            },
            saved => SettingsItem::Action {
                label: "Forget WiFi network".to_owned(),
                color: TEXT,
                enabled: matches!(saved, Some((_, WifiSource::Nvs))),
            },
        };
        let volume_value = if self.settings.volume == 0 { "Off".to_owned() } else { format!("{}%", self.settings.volume) };
        let view = SettingsView {
            info: vec![
                ("Launcher", screen.launcher_info.0.clone()),
                ("Build", screen.launcher_info.1.clone()),
                ("SD card", sd_info),
                ("WiFi", wifi_info),
            ],
            items: vec![
                SettingsItem::Level {
                    label: "Brightness",
                    percent: self.settings.brightness,
                    value: format!("{}%", self.settings.brightness),
                },
                SettingsItem::Level { label: "Volume", percent: self.settings.volume, value: volume_value },
                forget,
            ],
            selected: screen.selected,
        };
        self.ui.draw_settings(&mut self.board.display, &view)
    }

    /// Footer controls for the settings screen.
    pub(crate) fn settings_controls(&self) -> &'static str {
        match self.settings_screen.as_ref().map(|screen| ITEMS[screen.selected]) {
            Some(Item::ForgetWifi) => "U/D choose  A forget  B back",
            _ => "U/D choose  L/R change  B back",
        }
    }
}

/// e.g. "v0.1.0, slot A" and "3fa9c2e1, commit 220357ce" (build ID and commit).
fn launcher_info() -> (String, String) {
    let running = boot::running_partition_label();
    let slot = match running.as_str() {
        "launcher_a" => "A",
        "launcher_b" => "B",
        other => other,
    };
    let build_id = flash::app_info(&running).map_or_else(|| "?".to_owned(), |info| info.build_id);
    let build = match image::running().commit() {
        Some(commit) => format!("{build_id}, commit {commit}"),
        None => build_id,
    };
    (format!("v{}, slot {slot}", env!("CARGO_PKG_VERSION")), build)
}

/// e.g. "512 MB" or "14.8 GB"
fn format_bytes(bytes: u64) -> String {
    const MB: u64 = 1 << 20;
    const GB: u64 = 1 << 30;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes / MB)
    }
}
