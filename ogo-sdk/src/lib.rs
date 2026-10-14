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
//! once per update, and handles MENU (pause menu with volume, music on/off and "Exit to launcher")
//! and VOL (all sound, effects only, muted). It applies the brightness and sound settings from the
//! launcher's settings, and saves the changes made in the game.
//!
//! A game crate needs, besides `ogo-sdk`: a `build.rs` calling `embuild::espidf::sysenv::output()`,
//! and a direct dependency on `esp-idf-svc` even if unused, because the build script only gets
//! ESP-IDF's linker arguments from a direct dependency. See `games/snake` for a complete example.

mod ctx;
mod overlay;
mod presenter;
mod saves;

pub mod pitch;

use std::time::{Duration, Instant};

use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use esp_idf_svc::nvs::EspDefaultNvs;
use ogo_board::{
    boot::{self, Boot},
    settings, Board, Button, Settings,
};

pub use ctx::{Ctx, Input};
#[doc(hidden)]
pub use ogo_board as __board;
/// Samples per second of the speaker, for a [`Stream`].
pub use ogo_board::speaker::SAMPLE_RATE;
/// The launcher's colours and drawing helpers (`text`, `selection`, `panel`, `progress_bar`), for
/// games that want to match its look.
pub use ogo_board::ui;
pub use ogo_board::{in_internal_ram, BatteryLevel, Note, Part, Speaker, Stream, Track, Wave};
pub use saves::Saves;

use overlay::{PauseAction, PauseMenu, SoundIndicator, INDICATOR_AREA, MENU_SOUND};
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

    pub use crate::{Ctx, Framebuffer, Game, Note, Part, Track, Wave, SCREEN_HEIGHT, SCREEN_WIDTH};
}

pub trait Game: Sized {
    /// Updates (and frames) per second. The display manages about 30 full frames per second, or
    /// more with a smaller [`Game::area`].
    const FPS: u32 = 30;

    /// Whether the game has music of its own, apart from its sound effects. Without (e.g. an
    /// emulator, whose sound can't be split like that), VOL only switches between sound on and
    /// muted, and the pause menu has no music line.
    const MUSIC: bool = true;

    fn new(ctx: &mut Ctx) -> Self;

    /// Advances the game by one step. Not called while the pause menu is open.
    fn update(&mut self, ctx: &mut Ctx);

    /// Draws the whole frame, or all of [`Game::area`]; the buffer still holds an older frame, so
    /// clear or overwrite all of it.
    fn draw(&self, fb: &mut Framebuffer);

    /// The part of the screen the game draws, if not all of it; asked before each `draw`. `draw`
    /// then gets a framebuffer of just that size in internal RAM, with (0, 0) at the area's
    /// top-left corner. The SDK keeps the rest of the screen black and sends only this part to the
    /// display. Both are much faster: a quarter of the screen takes about a quarter of the time to
    /// send, and internal RAM is several times faster to draw into than the PSRAM of a full frame.
    fn area(&self) -> Option<Rectangle> {
        None
    }

    /// Called when the pause menu opens: a good moment to save, since "Exit to launcher" and
    /// switching off often follow.
    fn paused(&mut self, _ctx: &mut Ctx) {}
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
    let nvs =
        match Boot::take().and_then(|mut boot| boot.return_to_launcher_on_reset().map(|()| boot)) {
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
    let Board {
        display,
        backlight: _backlight,
        buttons,
        speaker,
        battery,
        led,
        sd,
        modem: _,
    } = Board::take_with(&settings, |_| Ok(()))?;
    let mut presenter = Presenter::new(display)?;
    let mut ctx = Ctx::new(game_id, buttons, speaker, battery, led, sd, nvs, settings);
    let mut game = G::new(&mut ctx);

    let frame_time = Duration::from_micros(1_000_000 / u64::from(G::FPS.max(1)));
    let mut next_frame = Instant::now();
    let mut pause: Option<PauseMenu> = None;
    let mut indicator = SoundIndicator::new(G::MUSIC);
    let screen = Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, SCREEN_HEIGHT));
    let mut area = None;
    // Where the screen still shows something outside the game's area (an overlay, an earlier
    // area): cleared with the next frame.
    let mut stale: Option<Rectangle> = None;

    loop {
        ctx.poll()?;

        if ctx.input().pressed(Button::Volume) {
            if G::MUSIC {
                ctx.change_settings(|s| s.set_sound_mode(s.sound_mode().next()));
            } else {
                // Only on and muted; the music setting stays as it is for other games
                ctx.change_settings(|s| s.muted = !s.muted);
            }
            ctx.sound().play(MENU_SOUND);
            indicator.show();
        }

        match &mut pause {
            Some(menu) => match menu.update(ctx.input()) {
                Some(PauseAction::Resume) => {
                    pause = None;
                    ctx.input_mut().ignore_held();
                    ctx.save_settings_now();
                    ctx.sound().set_music_paused(false);
                }
                Some(PauseAction::Volume(delta)) => {
                    ctx.change_settings(|s| {
                        s.volume = settings::step(&settings::VOLUME_STEPS, s.volume, delta);
                        s.muted = false;
                    });
                    ctx.sound().play(MENU_SOUND);
                }
                Some(PauseAction::ToggleMusic) => {
                    ctx.change_settings(|s| s.music = !s.music);
                    ctx.sound().play(MENU_SOUND);
                }
                Some(PauseAction::Exit) => ctx.exit_to_launcher(),
                None => {}
            },
            None if ctx.input().pressed(Button::Menu) => {
                ctx.sound().set_music_paused(true);
                ctx.sound().play(MENU_SOUND);
                pause = Some(PauseMenu::new(G::MUSIC));
                game.paused(&mut ctx);
            }
            None => {
                game.update(&mut ctx);
                ctx.advance_frame();
            }
        }

        let new_area = game.area().filter(|a| *a != screen);
        if new_area != area {
            area = new_area;
            stale = Some(screen);
        }
        let overlays = |fb: &mut Framebuffer, indicator: &mut SoundIndicator| {
            let mut drawn = pause.as_ref().map(|menu| {
                menu.draw(fb, ctx.settings());
                screen
            });
            if let Some(rect) = indicator.draw(fb, ctx.settings()) {
                drawn = Some(drawn.map_or(rect, |d| envelope(d, rect)));
            }
            drawn
        };
        match area {
            None => {
                let mut fb = presenter.take_full()?;
                game.draw(&mut fb);
                overlays(&mut fb, &mut indicator);
                stale = None;
                presenter.present(fb, vec![screen])?;
            }
            Some(area) => {
                let mut canvas = presenter.take_area(area)?;
                game.draw(&mut canvas);
                let indicator_shown = indicator.visible();
                let indicator_overlaps =
                    indicator_shown && !INDICATOR_AREA.intersection(&area).is_zero_sized();
                if pause.is_none() && stale != Some(screen) && !indicator_overlaps {
                    // The area from its own buffer; the volume box beside it, and the black where
                    // it was, in small buffers of their own
                    let whole = canvas.bounding_box();
                    presenter.present(canvas, vec![whole])?;
                    if indicator_shown {
                        let mut boxed = in_internal_ram(|| Framebuffer::new(INDICATOR_AREA));
                        indicator.draw(
                            &mut boxed.translated(-INDICATOR_AREA.top_left),
                            ctx.settings(),
                        );
                        let whole = boxed.bounding_box();
                        presenter.present(boxed, vec![whole])?;
                        stale = Some(INDICATOR_AREA);
                    } else if let Some(rect) = stale.take() {
                        let black = in_internal_ram(|| Framebuffer::new(rect));
                        let whole = black.bounding_box();
                        presenter.present(black, vec![whole])?;
                    }
                } else {
                    // Overlays need the whole screen: the area goes into a full-screen buffer.
                    // Only what's sent must be right there: the area, the overlays (drawn whole)
                    // and where an overlay was (black). Clearing all of PSRAM's 150 KB takes
                    // milliseconds, so that's only for the pause menu, which dims everything.
                    let mut fb = presenter.take_full()?;
                    if pause.is_some() || stale == Some(screen) {
                        fb.clear(Rgb565::BLACK).ok();
                    } else if let Some(rect) = stale {
                        fb.fill_solid(&rect, Rgb565::BLACK).ok();
                    }
                    fb.copy_from(&canvas);
                    presenter.recycle(canvas);
                    let mut send: Vec<Rectangle> =
                        [Some(area), stale.take()].into_iter().flatten().collect();
                    if let Some(rect) = overlays(&mut fb, &mut indicator) {
                        send.push(rect);
                        stale = Some(rect);
                    }
                    if send.contains(&screen) {
                        send = vec![screen];
                    }
                    presenter.present(fb, send)?;
                }
            }
        }

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

/// The smallest rectangle containing both.
fn envelope(a: Rectangle, b: Rectangle) -> Rectangle {
    let top_left = a.top_left.component_min(b.top_left);
    let bottom_right = (a.top_left + a.size).component_max(b.top_left + b.size);
    Rectangle::with_corners(top_left, bottom_right - Point::new(1, 1))
}
