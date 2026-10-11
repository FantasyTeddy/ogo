# Writing a game

An ogo game is its own firmware image. The launcher copies it into the game slot and restarts into
it; the game then has the whole device until it restarts back into the launcher. Games are built on
[ogo-sdk](../ogo-sdk/src/lib.rs), which takes care of everything that isn't the game itself: the
switch back to the launcher, the frame loop, the screen, the pause menu and the volume.

[games/snake](../games/snake/src/main.rs) is a complete example. This guide assumes the toolchain
from the [README](../README.md#prerequisites) is installed.

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
rust-version.workspace = true

# Read by `cargo xtask pack` for game.toml
[package.metadata.ogo]
name = "Pong"                     # shown in the launcher's list
sdk_version = 1                   # must match the launcher, see docs/CONTRACTS.md

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

**Input.** `ctx.input()` is sampled once per frame:
- `pressed(button)`: went down since the last frame
- `held(button)`: is down
- `released(button)`: went up since the last frame

Buttons: `Up`, `Down`, `Left`, `Right`, `A`, `B`, `Select`, `Start`. **MENU and VOL belong to the
SDK:** MENU opens the pause menu (resume, exit to the launcher; `update` isn't called while it's
open, `draw` is), VOL cycles the volume.

**Sound.** `ctx.sound().play(&[Note::new(frequency_hz, duration_ms), …])` queues notes (sine tones;
a frequency of 0 is a pause) and returns at once. Up to 4 sounds can wait; more are dropped. The
volume is the user's setting, so leave it alone. There is no sample (PCM) playback yet.

**Saves.** `ctx.saves()` stores one file per name in `/ogo/saves/<id>/` on the SD card (names: a-z,
A-Z, 0-9, `-` and `_`). They survive reinstalling and deleting the game.

```rust
let best: u32 = ctx.saves().read("best").unwrap_or(0);   // None if missing or unparsable
ctx.saves().write("best", &best)?;                       // as text, via a temp file
ctx.saves().write_bytes("level", &bytes)?;               // anything else: serialize it yourself
```

Writes go to the SD card immediately, so there's nothing to flush before exiting.

**Other things on `Ctx`:**
- `random()`, `random_range(a..b)`: from the hardware random number generator
- `frame()`: number of updates so far, e.g. for animations
- `battery()`: `Option<BatteryLevel>` with `percent` and `millivolts`, updated every 5 s
- `set_led(on)`: the blue LED
- `exit_to_launcher()`: restarts into the launcher (the pause menu does this too)

**Logging.** `log::info!` and friends go to the serial port.

## 4. Icon (optional)

`games/<id>/icon.rgb565` is shown next to the list: 64×64 pixels, RGB565 little-endian, row by row,
exactly 8192 bytes. Without one, the launcher draws the first letter of the name. To convert a PNG
(needs [Pillow](https://pypi.org/project/pillow/)):

```python
from PIL import Image

image = Image.open("icon.png").convert("RGB").resize((64, 64))
with open("icon.rgb565", "wb") as out:
    for r, g, b in image.getdata():
        out.write((((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)).to_bytes(2, "little"))
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
same id replaces the old one; the saves stay.

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

[games/hello](../games/hello/src/main.rs) uses `ogo-board` directly. A game that does this must
still follow the boot contract ([CONTRACTS.md](CONTRACTS.md#boot-contract)): declare
`ogo_board::image_desc!("ogo-game")`, and call `Boot::take()?.return_to_launcher_on_reset()` before
anything else, or the device stays in the game after a reset.
