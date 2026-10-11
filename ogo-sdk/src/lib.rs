//! ogo-sdk: everything a game needs to run under the ODROID-GO launcher.
//!
//! Implement [`Game`] and hand it to [`main!`]:
//!
//! ```ignore
//! use ogo_sdk::prelude::*;
//!
//! struct MyGame;
//!
//! impl Game for MyGame {
//!     fn new(ctx: &mut Ctx) -> Self { MyGame }
//!     fn update(&mut self, ctx: &mut Ctx) {}
//!     fn draw(&self, fb: &mut Framebuffer) { fb.clear(Rgb565::BLACK).ok(); }
//! }
//!
//! ogo_sdk::main!(MyGame);
//! ```
//!
//! The SDK makes sure any reset returns to the launcher, runs `update` at a fixed rate and `draw`
//! once per update, and handles MENU (pause menu with "Exit to launcher") and VOL (volume). It
//! applies the brightness and volume from the launcher's settings, and saves VOL changes.
//!
//! A game crate needs, besides `ogo-sdk`: a `build.rs` calling `embuild::espidf::sysenv::output()`,
//! and a direct dependency on `esp-idf-svc` even if unused, because the build script only gets
//! ESP-IDF's linker arguments from a direct dependency. See `games/snake` for a complete example.

mod ctx;
mod overlay;
mod presenter;
mod saves;

use std::time::{Duration, Instant};

use anyhow::Context as _;
use esp_idf_svc::nvs::EspDefaultNvs;
use ogo_board::{
    boot::{self, Boot},
    settings, Board, Button, Settings,
};

pub use ctx::{Ctx, Input};
#[doc(hidden)]
pub use ogo_board as __board;
pub use ogo_board::{BatteryLevel, Note, Speaker};
/// The launcher's colours and drawing helpers (`text`, `selection`, `panel`, `progress_bar`), for
/// games that want to match its look.
pub use ogo_board::ui;
pub use saves::Saves;

use overlay::{PauseAction, PauseMenu, VolumeIndicator, MENU_SOUND};
use presenter::Presenter;

/// The 320x240 frame a game draws into. Implements embedded-graphics' `DrawTarget`.
pub type Framebuffer = ogo_board::Canvas;

pub const SCREEN_WIDTH: u32 = ogo_board::display::WIDTH as u32;
pub const SCREEN_HEIGHT: u32 = ogo_board::display::HEIGHT as u32;

/// The version of the contract between launcher and games; `cargo xtask pack` writes it into
/// `game.toml` as `sdk_version`.
pub use ogo_format::game::SDK_VERSION;

pub mod prelude {
    pub use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
    pub use ogo_board::Button;

    pub use crate::{Ctx, Framebuffer, Game, Note, SCREEN_HEIGHT, SCREEN_WIDTH};
}

pub trait Game: Sized {
    /// Updates (and frames) per second. The display manages about 30 full frames per second.
    const FPS: u32 = 30;

    fn new(ctx: &mut Ctx) -> Self;

    /// Advances the game by one step. Not called while the pause menu is open.
    fn update(&mut self, ctx: &mut Ctx);

    /// Draws the whole frame; the buffer still holds an older frame, so clear or overwrite all of it.
    fn draw(&self, fb: &mut Framebuffer);
}

/// Defines `main` for a game type, and the image description that marks it as a game.
#[macro_export]
macro_rules! main {
    ($game:ty) => {
        $crate::__board::image_desc!(Game);

        fn main() {
            $crate::run::<$game>(env!("CARGO_PKG_NAME"))
        }
    };
}

/// Runs a game. Use [`main!`] instead of calling this directly.
pub fn run<G: Game>(game_id: &str) -> ! {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // Before anything else can fail: make sure the next reset starts the launcher.
    let nvs = match Boot::take().and_then(|mut boot| boot.return_to_launcher_on_reset().map(|()| boot)) {
        Ok(boot) => {
            log::info!("{game_id} running from {}", boot::running_partition_label());
            Settings::open_nvs(boot.nvs_partition())
                .inspect_err(|e| log::warn!("Can't open the settings: {e:?}"))
                .ok()
        }
        Err(e) => {
            log::error!("Can't make the launcher the boot slot again: {e:?}");
            None
        }
    };

    if let Err(e) = run_game::<G>(game_id, nvs) {
        log::error!("{game_id} stopped: {e:?}");
    }
    boot::restart()
}

fn run_game<G: Game>(game_id: &str, nvs: Option<EspDefaultNvs>) -> anyhow::Result<()> {
    let settings = nvs.as_ref().map(Settings::load).unwrap_or_default();
    let Board { display, backlight: _backlight, buttons, speaker, battery, led, sd, modem: _ } =
        Board::take_with(&settings, |_| Ok(()))?;
    let mut presenter = Presenter::new(display)?;
    let mut ctx = Ctx::new(game_id, buttons, speaker, battery, led, sd, nvs);
    let mut game = G::new(&mut ctx);

    let frame_time = Duration::from_micros(1_000_000 / u64::from(G::FPS.max(1)));
    let mut next_frame = Instant::now();
    let mut pause: Option<PauseMenu> = None;
    let mut volume = VolumeIndicator::default();

    loop {
        ctx.poll()?;

        if ctx.input().pressed(Button::Volume) {
            let next = settings::cycle(&settings::VOLUME_STEPS, ctx.sound().volume());
            ctx.sound().set_volume(next);
            ctx.sound().play(MENU_SOUND);
            ctx.volume_changed();
            volume.show();
        }

        match &mut pause {
            Some(menu) => match menu.update(ctx.input()) {
                Some(PauseAction::Resume) => pause = None,
                Some(PauseAction::Exit) => ctx.exit_to_launcher(),
                None => {}
            },
            None if ctx.input().pressed(Button::Menu) => {
                ctx.sound().play(MENU_SOUND);
                pause = Some(PauseMenu::default());
            }
            None => {
                game.update(&mut ctx);
                ctx.advance_frame();
            }
        }

        let fb = presenter.frame().context("no frame buffer")?;
        game.draw(fb);
        if let Some(menu) = &pause {
            menu.draw(fb);
        }
        volume.draw(fb, ctx.sound().volume());
        presenter.present()?;

        // Fixed rate; if we fall far behind, don't try to catch up.
        next_frame += frame_time;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else if now - next_frame > frame_time * 3 {
            next_frame = now;
        }
    }
}
