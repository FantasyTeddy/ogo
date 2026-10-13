# Writing a game

An ogo game is its own firmware image. The launcher copies it into the game slot and restarts into
it; the game then has the whole device until it restarts back into the launcher. Games are built on
[ogo-sdk](../ogo-sdk/src/lib.rs), which takes care of everything that isn't the game itself: the
switch back to the launcher, the frame loop, the screen, the pause menu and the sound settings.

[games/snake](../games/snake/src/main.rs) is a complete example. This guide assumes the toolchain
from [CONTRIBUTING.md](../CONTRIBUTING.md#prerequisites) is installed.

## 1. Create the crate

Games live in the workspace under `games/<id>/`. The package name is the game's id: lowercase
letters, digits and `-`.

`games/pong/Cargo.toml`:

```toml
[package]
name = "pong"
version = "0.1.0"
authors = ["Your Name"]           # the first author is shown in the launcher
edition.workspace = true

# Read by `cargo xtask pack` for game.toml
[package.metadata.ogo]
name = "Pong"                     # shown in the launcher's list

[[bin]]
name = "pong"
harness = false

[dependencies]
ogo-sdk.workspace = true
embedded-graphics.workspace = true
log.workspace = true
# Not used directly, but build.rs only gets ESP-IDF's linker arguments from a direct dependency.
esp-idf-svc.workspace = true

[build-dependencies]
embuild.workspace = true
```

`games/pong/build.rs`:

```rust
fn main() {
    embuild::espidf::sysenv::output();
}
```

Then add `"games/pong"` to `members` in the root [Cargo.toml](../Cargo.toml).

## 2. Implement `Game`

`games/pong/src/main.rs`:

```rust
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use ogo_sdk::prelude::*;

struct Pong {
    x: i32,
}

impl Game for Pong {
    fn new(_ctx: &mut Ctx) -> Self {
        Pong { x: 160 }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.input().held(Button::Left) {
            self.x -= 4;
        }
        if ctx.input().held(Button::Right) {
            self.x += 4;
        }
        if ctx.input().pressed(Button::A) {
            ctx.sound().play(&[Note::new(880, 40)]);
        }
    }

    fn draw(&self, fb: &mut Framebuffer) {
        fb.clear(Rgb565::BLACK).ok();
        Rectangle::new(Point::new(self.x - 20, 220), Size::new(40, 6))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::WHITE))
            .draw(fb)
            .ok();
    }
}

ogo_sdk::main!(Pong);
```

The SDK calls `new` once, then `update` and `draw` once per frame at `Game::FPS` (default 30;
override it with `const FPS: u32 = 60;` in the `impl`). Keep game logic in `update` and only drawing
in `draw`.

## 3. The SDK

**Screen.** `draw` gets a 320×240 `Framebuffer` (`SCREEN_WIDTH` × `SCREEN_HEIGHT`), an
[embedded-graphics](https://docs.rs/embedded-graphics/0.8) `DrawTarget` in RGB565. It still holds an
older frame, so clear or overwrite all of it every time. The SDK sends finished frames to the LCD
from another thread while the game draws the next one. A full frame takes about 31 ms to send, so
30 fps is the realistic rate for full redraws.

A game that only uses part of the screen can say so in `area()`, asked before each `draw`, e.g.
`Some(Rectangle::new(Point::new(80, 48), Size::new(160, 144)))`. The SDK then keeps the rest of the
screen black (also after the pause menu or the volume box) and sends only that part, which is
correspondingly faster; `draw` only has to cover the area. Returning `None` again goes back to the
whole screen. `Framebuffer::fill_contiguous` copies a block of pixels row by row, e.g. a picture
drawn every frame.

`ogo_sdk::ui` has the launcher's colours (`TEXT`, `DIM`, `BAR`, `SELECTION`, …) and a few helpers:
`text` (one line, vertically centred on a point), `selection` (highlight a row), `panel` (a box with
a border) and `progress_bar`. They are optional, for games that want to match the launcher's look:

```rust
use embedded_graphics::{mono_font::ascii::FONT_10X20, text::Alignment};
use ogo_sdk::ui;

ui::text(fb, "Game over", Point::new(160, 120), &FONT_10X20, ui::TEXT, Alignment::Center);
```

**Input.** `ctx.input()` is sampled once per frame:
- `pressed(button)`: went down since the last frame
- `held(button)`: is down
- `released(button)`: went up since the last frame

Buttons: `Up`, `Down`, `Left`, `Right`, `A`, `B`, `Select`, `Start`. **MENU and VOL belong to the
SDK:** MENU opens the pause menu (resume, volume, music on/off, exit to the launcher; `update` isn't
called while it's open, `draw` is, and the music pauses), VOL switches between all sound, effects
only (music off) and muted.

**Sound.** `ctx.sound().play(&[Note::new(frequency_hz, duration_ms), …])` queues notes (sine tones;
a frequency of 0 is a pause) and returns at once. Up to 4 sounds can wait; more are dropped. The
volume is the user's setting, so leave it alone. There is no sample (PCM) playback.

**Music.** A `Track` loops under the sound effects, a little quieter. It has one or two `Part`s
played together, e.g. a melody and a bass line, each with its own `Wave` (`Square`, `Triangle` or
`Sine`). `ogo_sdk::pitch` names the frequencies (`A4` is 440 Hz, `CS4` is C sharp, `REST` a pause).
The parts must be equally long; `Track::new` in a `const` checks that when compiling.

```rust
use ogo_sdk::pitch::*;

const MELODY: &[Note] = &[Note::new(C5, 200), Note::new(E5, 200), Note::new(G5, 400)];
const BASS: &[Note] = &[Note::new(C3, 400), Note::new(G3, 400)];
const MUSIC: Track = Track::new(&[
    Part { wave: Wave::Square, notes: MELODY },
    Part { wave: Wave::Triangle, notes: BASS },
]);

ctx.sound().play_music(MUSIC);        // from the start, replacing any track before
ctx.sound().set_music_speed(120);     // tempo in percent, e.g. faster on later levels
ctx.sound().stop_music();
```

The player can switch music off (VOL, pause menu, launcher settings); the game still calls
`play_music` as usual and just isn't heard.

**Saves.** `ctx.saves()` stores one file per name in `/ogo/saves/<id>/` on the SD card (names: a-z,
A-Z, 0-9, `-` and `_`). They survive reinstalling and deleting the game.

```rust
let best: u32 = ctx.saves().read("best").unwrap_or(0);   // None if missing or unparsable
ctx.saves().write("best", &best)?;                       // as text, via a temp file
ctx.saves().write_bytes("level", &bytes)?;               // anything else: serialize it yourself
```

Writes go to the SD card immediately, so there's nothing to flush before exiting. For data that
changes all the time, `fn paused(&mut self, ctx: &mut Ctx)` in the `impl Game` is called when the
pause menu opens: a good moment to save, since exiting or switching off often follows.

**Other things on `Ctx`:**
- `random()`, `random_range(a..b)`: from the hardware random number generator
- `frame()`: number of updates so far, e.g. for animations
- `battery()`: `Option<BatteryLevel>` with `percent` and `millivolts`, updated every 5 s
- `set_led(on)`: the blue LED
- `set_cpu_mhz(mhz)`: 80, 160 (the default) or 240 MHz, for games that need more speed; faster uses
  more battery
- `exit_to_launcher()`: restarts into the launcher (the pause menu does this too)

**Logging.** `log::info!` and friends go to the serial port.

## 4. Icon (optional)

`games/<id>/icon.rgb565` is shown next to the list: 64×64 pixels, RGB565 little-endian, row by row,
exactly 8192 bytes. Without one, the launcher draws the first letter of the name. To make one from a
PNG (ideally square; transparent parts become black, like the launcher's background):

```powershell
cargo xtask icon pong pong.png    # writes games/pong/icon.rgb565
```

## 5. Build and run

```powershell
cargo xtask pack pong --sd E:\    # build and copy to the SD card as /ogo/games/pong/
cargo xtask pack pong             # or build dist\pong.ogo and upload it on the web page
```

Then pick the game in the launcher. The first start installs it (a few seconds); later starts skip
that unless the image changed. To see the game's log, keep a serial monitor open while you start it
from the launcher, e.g. with `cargo xtask flash --monitor` or `espflash monitor`; restarts between
launcher and game don't close it.

To release a new version, raise `version` in Cargo.toml and pack again. Uploading a package with the
same id replaces the old one; the saves stay. Games in this repository are also part of every ogo
release, as `.ogo` packages.

## Limits

- **Image size:** the game slot holds ~8.9 MB. Snake is about 670 KB, most of it Rust `std` and
  ESP-IDF.
- **Memory:** 4 MB PSRAM for data, of which the SDK's two frame buffers use 300 KB. Allocations of
  16 KB or more go to PSRAM automatically; smaller ones use the ESP32's internal RAM (a few hundred
  KB, shared with ESP-IDF).
- **Stack:** the game runs on the main task with an 8 KB stack (`CONFIG_ESP_MAIN_TASK_STACK_SIZE`);
  put large arrays on the heap (`Vec`, `Box`). Threads you spawn get 4 KB unless you set
  `stack_size`.
- **No code from the SD card or PSRAM:** everything the game runs must be in its image. Data (levels,
  graphics) can be loaded from the SD card with `std::fs` under `/sd/`.
- **WiFi** isn't set up for games.

## Without the SDK

A game can also use `ogo-board` directly instead of the SDK. It must still follow the boot contract
([CONTRACTS.md](CONTRACTS.md#boot-contract)): declare `ogo_board::image_desc!(Game)`, and call
`Boot::take()?.return_to_launcher_on_reset()` before anything else, or the device stays in the game
after a reset.
