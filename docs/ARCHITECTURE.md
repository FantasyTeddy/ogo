# Architecture

How ogo is built and why. For the exact formats and rules between the parts see
[CONTRACTS.md](CONTRACTS.md); for the board see [HARDWARE.md](HARDWARE.md); for writing games see
[GAMES.md](GAMES.md).

## Goals and decisions

ogo is a launcher for the ODROID-GO. It installs native Rust games from the SD card into a flash
slot and boots them. Games and the launcher itself are managed from a web page served by the device.

| Topic | Decision |
|---|---|
| Language | Rust, for the launcher and the games |
| Framework | `esp-idf-svc` (Rust `std` on ESP-IDF v5.5) for WiFi, HTTP, OTA, partitions and the SD card |
| Game format | Native firmware images, stored on the SD card and copied into a flash slot to run (the ESP32 can't run code from PSRAM or the SD card) |
| Game updates | Each game is installed and updated on its own; replacing it on the SD card is enough |
| Transfer | A web page on the device (upload, delete, launcher update); no PC tool needed |
| Launcher updates | A/B slots with rollback, uploaded through the same page |
| Safety | A recovery app in the factory slot, entered with MENU at power-on |

**Out of scope:** games in other languages, interpreted or WASM games, pulling updates from the
internet.

## Overview

```
 ┌───────────────────────────── PC (browser) ─────────────────────────────┐
 │  http://<device-ip>/   upload .ogo game · delete · upload launcher.bin │
 └──────────────────────────────────┬─────────────────────────────────────┘
                                    │ WiFi (HTTP), only while the WiFi screen is open
 ┌──────────────────────────────────▼─────────────────────────────────────┐
 │ LAUNCHER (launcher_a / launcher_b)                                     │
 │  game list · installer · WiFi + web server · settings · self-update    │
 └───────┬──────────────────────────────┬───────────────────────────┬─────┘
         │ reads/writes                 │ writes image, boots it    │ uses
 ┌───────▼────────┐            ┌────────▼────────┐          ┌───────▼───────┐
 │ SD card        │            │ game_0          │          │ ogo-board     │
 │ /ogo/games/... │            │                 │          │ (shared crate)│
 └────────────────┘            └────────┬────────┘          └───────▲───────┘
                                        │ boots into                │ uses
                               ┌────────▼────────┐                  │
                               │ GAME            ├──── ogo-sdk ─────┘
                               │ (Rust app)      │  restart → back to the launcher
                               └─────────────────┘
 RECOVERY (factory slot): start a launcher slot or install one from the SD card. MENU at power-on
```

## Crates

| Crate | Purpose |
|---|---|
| [ogo-format](../ogo-format/src/lib.rs) | Everything that doesn't need the ESP32: `game.toml`, the `.ogo` reader and writer, the image description layout, settings steps, text fitting, battery percentage. Own workspace, tested on the PC |
| [ogo-board](../ogo-board/src/lib.rs) | Board support (display, backlight, buttons, speaker, SD card, battery, LED) and the contracts that touch the hardware: boot contract (`boot`), image description in flash (`image`), flash writing (`flash`), settings in NVS (`settings`). Also the shared look (`ui`: colours and drawing helpers for launcher, recovery, the SDK's overlays and games) |
| [ogo-sdk](../ogo-sdk/src/lib.rs) | What games build on: the `Game` trait, frame loop, input, sound, saves, pause menu, VOL |
| [ogo-launcher](../ogo-launcher/src/main.rs) | The launcher firmware |
| [ogo-recovery](../ogo-recovery/src/main.rs) | The fallback in the factory slot |
| [games/snake](../games/snake/src/main.rs) | Example game on the SDK |
| [xtask](../xtask/src/main.rs) | Build tasks on the PC (own workspace): `pack`, `release`, `flash`, `test`, `samples`, `icon` |

Launcher and recovery use `ogo-board` directly; games use `ogo-sdk`, which wraps it. Everything
about switching between slots goes through `ogo_board::boot`, so the rules are in one place. Formats
that the PC side writes and the device reads (`game.toml`, `.ogo`, the image description) are
defined once in `ogo-format`, which xtask uses too.

`ogo-format` and `xtask` have their own workspaces, excluded from the main one: the main workspace
builds everything for the ESP32 (`build.target` in `.cargo/config.toml`), while these two also build
for the PC. The firmware crates still depend on `ogo-format` by path.

## Launcher

[main.rs](../ogo-launcher/src/main.rs) owns the board and runs one loop: poll the buttons (turned into
navigation events with key repeat by [input.rs](../ogo-launcher/src/input.rs)), handle them for the
open screen, read the battery every 5 s, poll the web server's events. The open screen is one value
of `enum Screen` (game list, notice, WiFi, settings); the WiFi and settings screens keep their state
in their variant and live in their own modules, so a new screen is a new variant and module.
Drawing goes through [ui/mod.rs](../ogo-launcher/src/ui/mod.rs): each screen area is an off-screen
`Canvas` sent to the LCD in one transfer, so redraws don't flicker.

**Start.** The boot screen (logo, version) appears as soon as the display works and stays at least
1.5 s. Meanwhile the launcher scans the SD card and confirms its slot (rollback, see below). Holding
SELECT at power-on opens a hardware test screen instead.

**Game list.** [library.rs](../ogo-launcher/src/library.rs) reads `/ogo/games/*/game.toml`. Folders
that can't be used stay in the list, dimmed, with the reason. A starts the selected game, START
rescans, SELECT opens the WiFi screen, MENU the settings.

**Starting a game** ([install.rs](../ogo-launcher/src/install.rs)):
1. If the hash stored for `game_0` equals the game's `sha256`, skip to step 3.
2. Otherwise stream `game.bin` into `game_0` with a progress bar (raw `esp_ota_begin` / `write` /
   `end` on that slot, since the `EspOta` wrapper always picks the "next" slot), checking the hash
   while writing, then store it. The speaker is muted meanwhile, since flash writes stall its DMA.
3. Boot the game (see the boot contract in [CONTRACTS.md](CONTRACTS.md#boot-contract)).

**WiFi screen** ([connect.rs](../ogo-launcher/src/connect.rs),
[network.rs](../ogo-launcher/src/network.rs)). WiFi and the web server only run while it's open: the
radio costs battery, games get the whole device, and the device is only reachable while someone is
looking at it. Opening it scans for networks (~2 s), then joins the saved network (NVS, or
`/ogo/settings.toml` for a first setup). If there is none or joining fails (~15 s timeout), it opens
an access point `ogo-XXXX` without password, named after the last two bytes of the MAC address. The
access point runs in mixed mode, so rescans still work. The screen shows numbered steps: join the
WiFi (access point only), open the address, enter the PIN. Leaving it switches WiFi off and rescans
the library if the page changed it.

**Web page** ([web.rs](../ogo-launcher/src/web.rs), [index.html](../ogo-launcher/src/web/index.html)).
Plain HTML/JS embedded in the firmware, with two tabs (`#games`, `#settings`). Handlers run in the
HTTP server's task and never touch the display: they report to the main loop through `WebEvent`s over
a channel, and the loop updates the screen, plays sounds, saves the WiFi network or restarts. A
random 4-digit PIN shown on the device unlocks a session cookie (details in
[CONTRACTS.md](CONTRACTS.md#web-api)). WiFi and lwIP buffers may go to PSRAM.

**Settings screen** ([settings_screen.rs](../ogo-launcher/src/settings_screen.rs)). Info at the top
(launcher version and slot, build ID and commit, SD card space, saved WiFi network), then
brightness, volume and "Forget WiFi network" (A twice). Brightness and volume apply right away and
are saved to NVS when the screen closes. A network from `/ogo/settings.toml` can't be forgotten here.

**Self-update.** "Update launcher" on the web page streams `launcher.bin` straight into the launcher
slot that isn't running, checks it's an `ogo-launcher` image, selects it and restarts. The new
launcher confirms itself once the display and SD card work; if it crashes before that, the
bootloader goes back to the previous slot, and that launcher says "Update didn't start". Tested
with `--features rollback-test`, a launcher that panics before confirming. Consequences of rollback
being enabled:
- Every switch of the boot slot marks the new slot "pending verify", including a game returning to
  the launcher. So the launcher confirms itself on every start, and games confirm their slot first
  thing.
- Switching off in the ~1 s between the launcher starting and confirming makes the next start boot
  the previously selected slot, usually the last game, which returns to the launcher right away.
- A launcher that crashes on every start would ping-pong with the last game. Recovery gets out of
  that.

## Recovery

[ogo-recovery](../ogo-recovery/src/main.rs) is a small app in the factory slot, so it survives
anything the launcher or a web update does.
- **Entering.** The bootloader option `CONFIG_BOOTLOADER_FACTORY_RESET` on MENU (GPIO13, active low,
  held 1 s) starts the factory slot and clears otadata (`CONFIG_BOOTLOADER_OTA_DATA_ERASE`). NVS is
  kept. Since otadata is then empty, recovery tells a MENU start from an empty otadata (e.g. after
  flashing) by the bootloader's factory-reset flag in RTC memory. Without MENU it starts the first
  valid launcher slot right away.
- **Menu.** Start launcher A or B (each with version, commit and build date), install
  `/ogo/updates/launcher.bin` from the SD card into launcher A, restart.
- It uses the default settings (full brightness), so a bad setting can't lock you out.
- The bootloader is the one esp-idf-sys builds from `sdkconfig.defaults`; espflash's default
  bootloader lacks the MENU option. `cargo xtask flash` passes the right one.
- The last resort is flashing over USB.

## Game runtime

[ogo-sdk](../ogo-sdk/src/lib.rs) runs a game's `Game` implementation:
- **Start.** First the boot-back step of the boot contract, so any reset returns to the launcher.
  Then it loads brightness and volume from NVS and sets up the board with them, before the backlight
  turns on.
- **Frame loop.** `update` runs at `Game::FPS` (default 30), then `draw`. If the loop falls far
  behind, it skips ahead instead of catching up. FreeRTOS runs at 1 kHz ticks for steady timing.
- **Display.** Two full-screen buffers (150 KB each, in PSRAM). A display thread sends one to the
  LCD while the game draws the other ([presenter.rs](../ogo-sdk/src/presenter.rs)).
- **Input** is sampled once per frame, which also debounces.
- **MENU** opens the pause menu (resume, exit to the launcher); `update` isn't called meanwhile.
  **VOL** cycles the volume with an on-screen indicator and saves it 4 s after the last press (by
  then the amplifier is usually off, so muting it for the flash write cuts no sound), or at once
  when exiting.
- **Sound.** Notes (sine tones with fades) queued to the audio thread in `ogo-board`.
- **Saves.** One text or binary file per name under `/ogo/saves/<id>/`, written via a temp file.

## Settings

Brightness and volume are shared by the launcher and games through NVS
([ogo_board::settings](../ogo-board/src/settings.rs)). `Board::take_with` applies them before the
backlight turns on, so nothing flashes at full brightness first. The WiFi network is the launcher's
alone ([settings.rs](../ogo-launcher/src/settings.rs)).

## Build and toolchain

The steps are in [CONTRIBUTING.md](../CONTRIBUTING.md#prerequisites); this is why they are as they are.

1. **Toolchain.** The Rust Xtensa toolchain from esp-rs, a nightly-based fork that only `espup`
   installs. [rust-toolchain.toml](../rust-toolchain.toml) pins it by name (`esp-1.97`), so every
   machine builds with the same compiler: `espup`'s default name `esp` means whatever was installed
   last. rustup can't install the pinned one by itself; a missing one shows up as
   `toolchain 'esp-1.97' is not installed`. **Upgrading:** install the new version with `espup`
   (new `--name`, `--toolchain-version` and export file), change `channel` and the command in the
   comment in `rust-toolchain.toml` and in CONTRIBUTING.md, then rebuild, run the tests and test on
   the device (see [CONTRIBUTING.md](../CONTRIBUTING.md#testing)). CI reads the toolchain from
   `rust-toolchain.toml`, so it needs no change. The workspace started from the esp-rs
   `esp-idf-template` for `std` / ESP-IDF v5.5.3, target `esp32`.
2. **[sdkconfig.defaults](../sdkconfig.defaults)** enables 16 MB flash and PSRAM (with
   `CONFIG_SPIRAM_CACHE_WORKAROUND` for this ESP32 revision), the MENU recovery option, app rollback,
   1 kHz ticks and long FAT names. `CONFIG_SPIRAM_MEMTEST=n` saves ~0.9 s per boot.
3. **Build folder (Windows).** esp-idf-sys refuses to build if its output folder is longer than 88
   characters, and enabling long paths doesn't help. That folder lies 67 characters below Cargo's
   build folder, and Windows adds a 4-character `\\?\` prefix, so the build folder may have at most
   17 characters. A good one depends on the machine, so the project's
   [.cargo/config.toml](../.cargo/config.toml) doesn't set `target-dir`: a clone at a short path uses
   its own `target`, otherwise a `.cargo/config.toml` in a parent folder sets one. Cargo reads config
   files from parent folders too, so that only affects projects below it; a user-wide
   `CARGO_TARGET_DIR` would work as well, but moves the output of every Rust project. `cargo
   metadata`, and with it xtask, finds the folder either way. `CARGO_WORKSPACE_DIR` in the project
   config is what embuild needs when the build folder is outside the project. On Linux and macOS
   the length doesn't matter, but the path must not contain spaces.
4. **Python.** ESP-IDF's Python requirements don't install on Python 3.15 on Windows:
   `esp-idf-kconfig` needs `windows-curses`, which has no 3.15 build (2.4.2 goes up to 3.14) and no
   source package. embuild always runs the first `python` on PATH (there's no setting for another
   one), and the install manager's version setting takes no ranges, so `.cargo/config.toml` sets
   `PYTHON_MANAGER_DEFAULT = "3.12"` for build scripts: the install manager's `python` then starts
   3.12, installing it on first use if missing. Other setups ignore the variable. The line can go
   once `windows-curses` supports 3.15. The toolchain's `LIBCLANG_PATH` and clang folder are in the
   user environment (`espup` puts them there), so builds work in any shell, and in the editor's
   rust-analyzer too.
5. **`cargo xtask`** ([xtask/](../xtask/src/main.rs)). A host crate with its own workspace, since the
   main workspace builds everything for the ESP32. The alias in `.cargo/config.toml` passes
   `--target host-tuple` (the PC it runs on). Every image it builds carries the git commit, and it
   checks the kind and commit of each one.
   - `flash [--all] [--port COM7] [--monitor] [--features …]` builds, finds the bootloader
     esp-idf-sys built, and runs `cargo espflash flash` with it, `--target-app-partition launcher_a`
     (cargo-espflash 4.3 ignores that key in `espflash.toml`) and `--erase-parts otadata`: a USB
     flash only writes `launcher_a`, and otherwise a web update that selected `launcher_b` would
     keep running. `--all` also flashes recovery. The project config has no `runner`, because
     `cargo run` with espflash would flash its default bootloader, without the MENU option.
   - `pack <game> [--ogo <file> | --sd <root>]` builds a game into `dist/<id>.ogo`, another file,
     or an SD card's `/ogo/games/<id>/`.
   - `release [--features …]` builds `launcher.bin`, `recovery.bin` and every game's `.ogo` into
     `dist/`, plus `ogo-<version>-full.bin`: everything `flash --all` writes, as one image from
     address 0 (bootloader at `0x1000`, partition table at `0x8000`, recovery and launcher in their
     slots, `0xFF` in between, so NVS and otadata end up erased). `espflash write-bin 0x0` installs it
     without the toolchain. `--features` applies to the launcher.
   - `test` runs the unit tests of `ogo-format` on the PC.
   - `samples <sd-root> [--force]` writes fake games (placeholder images, two broken on purpose) to
     an SD card, to test the game list.
   - `icon <game> <image.png>` converts a PNG into the game's `icon.rgb565`.

   Versions come from `Cargo.toml`: the workspace version for launcher, recovery and SDK, and each
   game's own (written into its `game.toml`). Releases are tagged with the workspace version and
   bundle every game as it is at that commit; games have no tags of their own.

The esp-rs crates (`esp-idf-svc`, `esp-idf-hal`, `esp-idf-sys`) come from git, as in the template
(see `[patch.crates-io]` in [Cargo.toml](../Cargo.toml)). The ecosystem moves quickly: check versions
before upgrading.

## Testing

CI ([ci.yml](../.github/workflows/ci.yml)) runs the `ogo-format` tests on the PC and Clippy on
everything, and builds a complete release on Linux, so the build doesn't depend on Windows. What
needs the ESP32 is tested on the device; the checklist is in
[CONTRIBUTING.md](../CONTRIBUTING.md#testing).

## Limitations

- Joining a WiFi network blocks the launcher's screen for up to ~15 s while it waits.
- The web server keeps every login session until the WiFi screen closes.
- Games get notes (sine tones) only; there is no sample (PCM) playback.
- One game slot: switching to another game reinstalls it.
- The battery percentage is a rough linear estimate, and there's no charging indicator (the board has
  no charge-status signal, see [HARDWARE.md](HARDWARE.md#findings)).
