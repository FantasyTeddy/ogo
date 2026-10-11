# Odroid-Go Rust Launcher

A launcher for the [ODROID-GO](https://wiki.odroid.com/odroid_go/odroid_go) handheld, written in Rust.
It installs native Rust games from the SD card into a flash slot and boots them. Games and launcher
updates are managed from a web page served by the device itself.

> **Status:** early development. Milestones M0–M2 are done: the launcher lists the games on the SD
> card with icons and a battery indicator, but can't start them yet (M3). Hold SELECT at power-on
> for a hardware test screen. See the [roadmap](docs/DESIGN.md#9-roadmap).

## Planned features

- Game library on the SD card, with icons
- Installs a game into a dedicated flash slot and boots it; reinstalls are skipped when the game is unchanged
- Games built on a small SDK (`ogo-sdk`): frame loop, input, sound, saves, exit back to the launcher
- Web UI over WiFi: upload and delete games, download saves
- Launcher self-update over the air, with A/B slots and rollback
- Recovery app, entered by holding MENU at power-on
- Later: emulators packaged as games

The full design is in [docs/DESIGN.md](docs/DESIGN.md).

## Hardware

ODROID-GO with an ESP32 (dual-core 240 MHz), 16 MB flash, 4 MB PSRAM, a 320×240 ILI9341 display and a
microSD slot. It connects over USB through a Silicon Labs CP210x USB-to-UART bridge
([driver](https://www.silabs.com/developer-tools/usb-to-uart-bridge-vcp-drivers)).

## Prerequisites

Developed on Windows with PowerShell.

1. **Rust** via [rustup](https://rustup.rs/).
2. **Xtensa toolchain and flashing tools:**
   ```powershell
   cargo install espup espflash cargo-espflash ldproxy
   espup install --name esp-1.97 --toolchain-version 1.97.0.0 --targets esp32 --std --export-file "$HOME\export-esp-1.97.ps1"
   ```
   The project is pinned to this toolchain in [rust-toolchain.toml](rust-toolchain.toml). The named
   install sits next to any other Xtensa toolchain you have. Note that `espup` also sets
   `LIBCLANG_PATH` in your user environment.
3. **Python 3.12.** ESP-IDF 5.5 doesn't install on newer versions such as 3.15. `env.ps1` expects it at
   `%LOCALAPPDATA%\Python\pythoncore-3.12-64`; edit the path there if yours is elsewhere.
4. **A drive `D:`** for build output. Build output goes to `D:\ogot` because ESP-IDF refuses to build
   under long paths. To use a different short folder, change `target-dir` in
   [.cargo/config.toml](.cargo/config.toml).

ESP-IDF itself (v5.5.3) is downloaded automatically on the first build. That build takes several
minutes; later builds are fast.

## Build and flash

In every new PowerShell window, load the environment first:

```powershell
. .\env.ps1
```

Then build, flash and open the serial monitor (replace `COM7` with your port):

```powershell
cargo espflash flash --release --package ogo-launcher --port COM7 --monitor
```

To build only:

```powershell
cargo build --release
```

The firmware ends up in `D:\ogot\xtensa-esp32-espidf\release\ogo-launcher`.

## Project layout

| Path | Contents |
|---|---|
| [ogo-board/](ogo-board/) | Board support: display, buttons, speaker, battery, LED, SD card |
| [ogo-launcher/](ogo-launcher/) | Launcher firmware: game list, hardware test screen |
| [docs/DESIGN.md](docs/DESIGN.md) | Design, flash layout, boot flows and roadmap |
| [.cargo/config.toml](.cargo/config.toml) | Target, linker, ESP-IDF version and build directory |
| [sdkconfig.defaults](sdkconfig.defaults) | ESP-IDF settings (flash size, PSRAM, stack sizes) |
| [env.ps1](env.ps1) | Shell setup: Xtensa toolchain and Python 3.12 |
| [tools/make_sample_games.py](tools/make_sample_games.py) | Writes sample (not runnable) games to an SD card, to test the game list |

More crates (`ogo-sdk`, `ogo-recovery`, example games, `xtask`) are added as the milestones
progress.

## Restoring the original firmware

Flashing this project overwrites the stock firmware. If you made a full backup first, for example:

```powershell
cargo espflash read-flash --port COM7 0 0x1000000 backup\original-flash-16MB.bin
```

you can restore it with:

```powershell
cargo espflash write-bin --port COM7 0 backup\original-flash-16MB.bin
```

The `backup/` folder is excluded from git.
