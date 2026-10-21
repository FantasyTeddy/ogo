//! The WiFi screen: while it's open, WiFi is on and the web page is served. Leaving it switches
//! WiFi off again, and rescans the library if the web page changed it.

use std::sync::{
    mpsc::{self, Receiver},
    Arc,
};

use anyhow::Context;
use esp_idf_svc::hal::delay::FreeRtos;
use ogo_board::{boot, sd, Note};

use crate::{
    input::Nav,
    network::{Mode, Network},
    settings::{self, WifiCredentials},
    ui::{ConnectView, DIM, SUCCESS, WARNING},
    web::{random_pin, NetworkInfo, WebEvent, WebServer, WebState},
    Launcher, Screen, SOUND_ERROR,
};

/// Footer controls of the WiFi screen.
pub const CONTROLS: &str = "B back";
const SOUND_INSTALLED: &[Note] = &[Note::new(660, 60), Note::new(880, 60), Note::new(1320, 100)];
/// Time to show "restarting" and let the HTTP response reach the browser.
const RESTART_DELAY_MS: u32 = 1500;

pub struct Session {
    _server: WebServer,
    state: Arc<WebState>,
    events: Receiver<WebEvent>,
    view: ConnectView,
    /// The web page added or deleted apps; rescan when leaving.
    library_changed: bool,
}

impl Launcher {
    pub(crate) fn open_connect(&mut self) -> anyhow::Result<()> {
        let pin = random_pin();
        self.ui
            .draw_top_bar(&mut self.board.display, "WiFi", self.battery)?;
        self.ui
            .draw_footer(&mut self.board.display, CONTROLS, None)?;
        match self.start_session(pin, None) {
            Ok(()) => Ok(()),
            Err(e) => {
                log::error!("Starting WiFi failed: {e:?}");
                self.board.speaker.play(SOUND_ERROR);
                self.close_connect()?;
                self.screen = Screen::Notice;
                self.ui.draw_notice(
                    &mut self.board.display,
                    "WiFi didn't start",
                    &format!("{e:#}"),
                    WARNING,
                )?;
                self.show_message("Press any button")
            }
        }
    }

    /// Starts WiFi and the web server and shows where to connect. `credentials` overrides the saved ones.
    fn start_session(
        &mut self,
        pin: String,
        credentials: Option<WifiCredentials>,
    ) -> anyhow::Result<()> {
        let credentials =
            credentials.or_else(|| settings::load_wifi(&self.nvs, self.board.sd.as_ref()));
        let heading = match &credentials {
            Some(credentials) => format!("Joining {}...", credentials.ssid),
            None => "Starting WiFi...".to_owned(),
        };
        self.ui.draw_connect(
            &mut self.board.display,
            &ConnectView {
                heading,
                ..Default::default()
            },
        )?;

        if self.network.is_none() {
            let modem = self
                .board
                .modem
                .take()
                .context("the radio is already in use")?;
            self.network = Some(Network::new(modem, self.boot.nvs_partition())?);
        }
        let (mode, networks) = self
            .network
            .as_mut()
            .expect("just created")
            .start(credentials.as_ref())?;

        let (sender, events) = mpsc::channel();
        let state = Arc::new(WebState::new(
            pin.clone(),
            self.board.sd.as_ref().map(|_| sd::MOUNT_POINT.into()),
            match &mode {
                Mode::Station { ssid, ip } => NetworkInfo {
                    mode: "station",
                    name: ssid.clone(),
                    ip: ip.to_string(),
                },
                Mode::AccessPoint { ssid, ip, .. } => NetworkInfo {
                    mode: "access_point",
                    name: ssid.clone(),
                    ip: ip.to_string(),
                },
            },
            networks,
            self.battery,
            sender,
        ));
        let server = WebServer::start(state.clone())?;

        let mut view = ConnectView {
            url: Some(format!("http://{}/", mode.ip())),
            pin: Some(pin),
            ..Default::default()
        };
        match &mode {
            Mode::Station { ssid, .. } => {
                view.heading = format!("Connected to {ssid}");
            }
            Mode::AccessPoint { ssid, failed, .. } => {
                view.heading = "WiFi setup".into();
                view.join_wifi = Some(ssid.clone());
                if let Some(failed) = failed {
                    view.activity = Some((failed.clone(), WARNING));
                }
            }
        }
        self.ui.draw_connect(&mut self.board.display, &view)?;
        self.screen = Screen::Wifi(Session {
            _server: server,
            state,
            events,
            view,
            library_changed: false,
        });
        Ok(())
    }

    pub(crate) fn handle_connect(&mut self, nav: Nav) -> anyhow::Result<()> {
        if nav == Nav::Back {
            self.close_connect()?;
        }
        Ok(())
    }

    /// Stops the web server and WiFi, and goes back to the app list.
    pub(crate) fn close_connect(&mut self) -> anyhow::Result<()> {
        let changed = self.end_session();
        self.draw_top_bar()?;
        self.draw_footer(None)?;
        if changed {
            self.rescan()
        } else {
            self.draw_library()
        }
    }

    /// Returns whether the library changed during the session.
    fn end_session(&mut self) -> bool {
        let changed = match std::mem::replace(&mut self.screen, Screen::Apps) {
            Screen::Wifi(session) => session.library_changed,
            other => {
                self.screen = other;
                false
            }
        };
        if let Some(network) = &mut self.network {
            if let Err(e) = network.stop() {
                log::warn!("Stopping WiFi failed: {e:?}");
            }
        }
        changed
    }

    /// Handles what happened on the web page since the last call.
    pub(crate) fn poll_connect(&mut self) -> anyhow::Result<()> {
        let Screen::Wifi(session) = &mut self.screen else {
            return Ok(());
        };
        *session.state.battery.lock().unwrap() = self.battery;

        let mut redraw = false;
        let mut reconnect = None;
        let mut scan = false;
        let mut restart_into = None;
        while let Ok(event) = session.events.try_recv() {
            redraw = true;
            session.view.activity = Some(match event {
                WebEvent::Uploading { received, total } => (
                    format!("Receiving upload... {} %", received * 100 / total.max(1)),
                    DIM,
                ),
                WebEvent::Installed { name } => {
                    session.library_changed = true;
                    self.board.speaker.play(SOUND_INSTALLED);
                    (format!("Installed {name}"), SUCCESS)
                }
                WebEvent::Deleted { id } => {
                    session.library_changed = true;
                    (format!("Deleted {id}"), SUCCESS)
                }
                WebEvent::FileSaved { name } => {
                    self.board.speaker.play(SOUND_INSTALLED);
                    (format!("Saved {name}"), SUCCESS)
                }
                WebEvent::FileDeleted { name } => (format!("Deleted {name}"), SUCCESS),
                WebEvent::Failed(reason) => {
                    self.board.speaker.set_muted(false);
                    self.board.speaker.play(SOUND_ERROR);
                    (reason, WARNING)
                }
                WebEvent::UpdateStarted => {
                    // Writing flash stalls the audio DMA, which would click.
                    self.board.speaker.set_muted(true);
                    ("Receiving a launcher update...".to_owned(), DIM)
                }
                WebEvent::Updating { received, total } => (
                    format!(
                        "Writing the launcher update... {} %",
                        received * 100 / total.max(1)
                    ),
                    DIM,
                ),
                WebEvent::LauncherWritten { slot } => {
                    restart_into = Some(slot);
                    ("Update installed, restarting...".to_owned(), SUCCESS)
                }
                WebEvent::WifiSaved(credentials) => {
                    settings::save_wifi(&self.nvs, &credentials)?;
                    let text = format!("Joining {}...", credentials.ssid);
                    reconnect = Some(credentials);
                    (text, DIM)
                }
                WebEvent::PinChanged(pin) => {
                    session.view.pin = Some(pin);
                    self.board.speaker.play(SOUND_ERROR);
                    ("Too many wrong PINs: new PIN".to_owned(), WARNING)
                }
                WebEvent::ScanRequested => {
                    scan = true;
                    ("Scanning for networks...".to_owned(), DIM)
                }
                WebEvent::Restart => boot::restart(),
            });
        }
        if redraw {
            self.ui
                .draw_connect(&mut self.board.display, &session.view)?;
        }
        if let Some(slot) = restart_into {
            self.boot.set_pending_update(slot)?;
            FreeRtos::delay_ms(RESTART_DELAY_MS);
            boot::restart();
        }
        if scan {
            if let Some(network) = &mut self.network {
                match network.scan() {
                    Ok(networks) => {
                        let count = networks.len();
                        *session.state.networks.lock().unwrap() = networks;
                        session.view.activity = Some((format!("Found {count} networks"), DIM));
                    }
                    Err(e) => {
                        session.view.activity = Some((format!("Scan failed: {e:#}"), WARNING))
                    }
                }
                self.ui
                    .draw_connect(&mut self.board.display, &session.view)?;
            }
        }

        if let Some(credentials) = reconnect {
            let pin = session.pin();
            let changed = self.end_session();
            if let Err(e) = self.start_session(pin, Some(credentials)) {
                log::error!("Rejoining WiFi failed: {e:?}");
                self.close_connect()?;
                return Ok(());
            }
            if let Screen::Wifi(session) = &mut self.screen {
                session.library_changed |= changed;
            }
        }
        Ok(())
    }
}

impl Session {
    fn pin(&self) -> String {
        self.state.pin.lock().unwrap().clone()
    }
}
