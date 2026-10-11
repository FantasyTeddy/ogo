# Architecture

How ogo is built and why. For the exact formats and rules between the parts see
[CONTRACTS.md](CONTRACTS.md); for the board see [HARDWARE.md](HARDWARE.md); for writing games see
[GAMES.md](GAMES.md); for what's next see [ROADMAP.md](ROADMAP.md).

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
| Emulators | Later, packaged as games that load ROMs from the SD card |

**Not in scope for now:** games in other languages, interpreted or WASM games, pulling updates from
the internet.

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
| [ogo-board](../ogo-board/src/lib.rs) | Board support (display, backlight, buttons, speaker, SD card, battery, LED) and the shared contracts: boot contract (`boot`), image description (`image`), flash writing (`flash`), settings (`settings`) |
| [ogo-sdk](../ogo-sdk/src/lib.rs) | What games build on: the `Game` trait, frame loop, input, sound, saves, pause menu, VOL |
| [ogo-launcher](../ogo-launcher/src/main.rs) | The launcher firmware |
| [ogo-recovery](../ogo-recovery/src/main.rs) | The fallback in the factory slot |
| [games/snake](../games/snake/src/main.rs) | Example game on the SDK |
| [games/hello](../games/hello/src/main.rs) | Minimal test game using `ogo-board` directly |
| [xtask](../xtask/src/main.rs) | Build tasks on the PC (own workspace): `pack`, `release`, `flash` |

Launcher and recovery use `ogo-board` directly; games use `ogo-sdk`, which wraps it. Everything
about switching between slots goes through `ogo_board::boot`, so the rules are in one place.

## Launcher

[main.rs](../ogo-launcher/src/main.rs) owns the board and runs one loop: poll the buttons (turned into
navigation events with key repeat by [input.rs](../ogo-launcher/src/input.rs)), handle them for the
open screen, read the battery every 5 s, poll the web server's events. Drawing goes through
[ui/mod.rs](../ogo-launcher/src/ui/mod.rs): each screen area is an off-screen `Canvas` sent to the
LCD in one transfer, so redraws don't flicker.

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

**WiFi screen** ([connect.rs](../ogo-launcher/src/connect.rs), [network.rs](../ogo-launcher/src/network.rs)).
WiFi and the web server only run while it's open: the radio costs battery, games get the whole
device, and the device is only reachable while someone is looking at it. Opening it scans for
networks (~2 s), then joins the saved network (NVS, or `/ogo/settings.toml` for a first setup). If
there is none or joining fails (~15 s timeout), it opens an access point `ogo-XXXX` without password,
named after the last two bytes of the MAC address. The access point runs in mixed mode, so rescans
still work. The screen shows numbered steps: join the WiFi (access point only), open the address,
enter the PIN. Leaving it switches WiFi off and rescans the library if the page changed it.

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

1. **Toolchain.** The Rust Xtensa toolchain from esp-rs, pinned in
   [rust-toolchain.toml](../rust-toolchain.toml); installation steps are in the
   [README](../README.md#prerequisites). The workspace started from the esp-rs `esp-idf-template`
   for `std` / ESP-IDF v5.5.3, target `esp32`.
2. **[sdkconfig.defaults](../sdkconfig.defaults)** enables 16 MB flash and PSRAM (with
   `CONFIG_SPIRAM_CACHE_WORKAROUND` for this ESP32 revision), the MENU recovery option, app rollback,
   1 kHz ticks and long FAT names. `CONFIG_SPIRAM_MEMTEST=n` saves ~0.9 s per boot.
3. **Windows path length.** esp-idf-sys refuses to build if its output path is too long, and
   enabling long paths doesn't help. So [.cargo/config.toml](../.cargo/config.toml) puts the build
   output into `D:/ogot` (plus `CARGO_WORKSPACE_DIR`, which embuild then needs).
4. **Python.** ESP-IDF 5.5 can't install its Python requirements on Python 3.15, and embuild always
   runs the first `python` on PATH (there's no setting for another one). `.cargo/config.toml` sets
   `PYTHON_MANAGER_DEFAULT = "3.12"` for build scripts, which makes the Python install manager's
   `python` start 3.12. The toolchain's `LIBCLANG_PATH` and clang folder are in the user environment
   (`espup` puts them there), so builds work in any shell, and in the editor's rust-analyzer too.
5. **`cargo xtask`** ([xtask/](../xtask/src/main.rs)). A host crate with its own workspace, since the
   main workspace builds everything for the ESP32. The alias in `.cargo/config.toml` passes
   `--target x86_64-pc-windows-msvc`. Every image it builds carries the git commit.
   - `flash [--all] [--port COM7] [--monitor] [--features …]` builds, finds the bootloader
     esp-idf-sys built, and runs `cargo espflash flash` with it, `--target-app-partition launcher_a`
     (cargo-espflash 4.3 ignores that key in `espflash.toml`) and `--erase-parts otadata`: a USB
     flash only writes `launcher_a`, and otherwise a web update that selected `launcher_b` would
     keep running. `--all` also flashes recovery.
   - `pack <game> [--ogo <file> | --sd <root>]` builds a game into `dist/<id>.ogo`, another file,
     or an SD card's `/ogo/games/<id>/`.
   - `release [--features …]` builds `launcher.bin`, `recovery.bin` and every game's `.ogo` into
     `dist/`. `--features` applies to the launcher.

The esp-rs crates (`esp-idf-svc`, `esp-idf-hal`, `esp-idf-sys`) come from git, as in the template
(see `[patch.crates-io]` in [Cargo.toml](../Cargo.toml)). The ecosystem moves quickly: check versions
before upgrading.
