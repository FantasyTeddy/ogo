<p align="center"><img src="docs/logo.png" alt="ogo: a crab whose shell is a handheld console" width="300"></p>

# ogo

A game launcher for the [ODROID-GO](https://wiki.odroid.com/odroid_go/odroid_go) handheld, written in
Rust. It installs native Rust games from the SD card into a flash slot and boots them. Games and
launcher updates are managed from a web page served by the device itself.

## Features

- Game library on the SD card, with icons
- Installs a game into a dedicated flash slot and boots it; reinstalls are skipped when the game is
  unchanged
- Games built on a small SDK (`ogo-sdk`): frame loop, input, sound, saves, pause menu, exit back to
  the launcher
- Recovery app, entered by holding MENU at power-on: start either launcher slot, or reinstall the
  launcher from the SD card
- Web page over WiFi, protected by a PIN shown on the device: upload and delete games, pick the WiFi
  network, see battery and free space
- Launcher updates from the web page, into the other of two launcher slots; if the new launcher
  doesn't start properly, the device goes back to the previous one by itself
- Settings screen (MENU): device info, brightness and volume (both also used by games), forget the
  WiFi network

More documentation in [docs/](docs/):

- [ARCHITECTURE.md](docs/ARCHITECTURE.md): how it's built and why, including the build setup and
  testing
- [CONTRACTS.md](docs/CONTRACTS.md): partition table, SD card and NVS layout, boot contract, file
  formats, web API
- [GAMES.md](docs/GAMES.md): writing a game
- [HARDWARE.md](docs/HARDWARE.md): pins and measured hardware details

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
   The project is pinned to this toolchain in [rust-toolchain.toml](rust-toolchain.toml); it sits
   next to any other Xtensa toolchain you have. `espup` also sets `LIBCLANG_PATH` in your user
   environment.
3. **Python 3.14 or older**, because ESP-IDF's Python requirements don't install on 3.15 on Windows.
   With the [Python install manager](https://docs.python.org/3/using/windows.html) there's nothing to
   do: builds use 3.12, which the install manager installs on first use. Otherwise make sure the
   first `python` on PATH is 3.14 or older.
4. **On Windows, a short build folder**, see below.

ESP-IDF itself (v5.5.3) is downloaded automatically on the first build. That build takes several
minutes; later builds are fast. The reasons behind these requirements are in
[ARCHITECTURE.md](docs/ARCHITECTURE.md#build-and-toolchain).

### Build folder

On Windows, Cargo's build folder may have **at most 17 characters**, because esp-idf-sys refuses
longer paths (`Too long output directory: …`).

- **Cloned to a short path**, e.g. `C:\ogo`: the default build folder `C:\ogo\target` fits; nothing
  to do.
- **Cloned to a longer path**: put a short folder into a `.cargo\config.toml` in a **parent folder**
  of the clone, e.g. `C:\Users\<you>\Downloads\.cargo\config.toml` for a clone in `Downloads`. It
  applies only to projects below that folder:

  ```toml
  [build]
  target-dir = "D:/ogot"
  ```

## Build and flash

Use the build tasks in [xtask/](xtask/) (replace `COM7` with your port; without `--port`,
espflash picks or asks):

```powershell
cargo xtask flash --all --port COM7    # first time: bootloader, partition table, recovery and launcher
cargo xtask flash --port COM7          # afterwards: launcher only
cargo xtask flash --monitor            # launcher, then show the serial log
cargo xtask release                    # launcher.bin, recovery.bin and every game's .ogo in dist\
cargo xtask test                       # unit tests, on the PC
cargo xtask samples E:\                # fake games on an SD card, to test the game list
```

`flash` uses this project's own bootloader (it starts recovery when MENU is held) and the partition
table from [partitions.csv](partitions.csv). Don't use `cargo espflash flash` directly: it would
flash espflash's default bootloader, without the MENU option (for the same reason `cargo run`
doesn't flash anything). Every image built by `cargo xtask` carries the git commit, shown on the
settings screen, the web page and in recovery.

A plain `cargo build --release` also works; its firmware ends up in
`<build folder>\xtensa-esp32-espidf\release\`, without a commit.

### Versions

Versions live in `Cargo.toml`. Launcher, recovery and SDK share the workspace version in the root
[Cargo.toml](Cargo.toml): raise it when you release a launcher. Each game has its own version in its
crate's Cargo.toml (see [docs/GAMES.md](docs/GAMES.md)). Builds of the same version still differ by
their commit. Tags are optional, e.g. `git tag launcher-v1.0.0` or `git tag snake-v1.1.0` to mark a
release in the history; nothing reads them.

## Games

Games are separate firmware images that the launcher copies from the SD card into the `game_0` flash
slot and boots. To build a game and copy it to an SD card (here `E:\`):

```powershell
cargo xtask pack snake --sd E:\
```

On the device, select the game and press A. The first start (or after an update) installs it with a
progress bar. In a game, MENU opens the pause menu with "Exit to launcher", and VOL changes the
volume (kept for the launcher and other games too). Any reset while a game runs also lands back in
the launcher. MENU in the game list opens the settings.

Or over WiFi: pack the game with `cargo xtask pack snake` (writes `dist\snake.ogo`, or pass
`--ogo <file>`), then upload it on the device's web page (see below).

To write a game, see [docs/GAMES.md](docs/GAMES.md); [games/snake](games/snake/) is a complete
example.

## Web page

Press **SELECT** in the game list. The device shows what to do, step by step:

- The first time (no WiFi configured), join the device's own WiFi `ogo-XXXX` (no password), open
  `http://192.168.71.1/` and enter the PIN shown. On the **Settings** tab, pick your WiFi; the device
  then joins it and shows its new address.
- Afterwards it joins your WiFi directly; open the address it shows from any device on that network.

Instead of picking the WiFi on the page, you can put it on the SD card as `/ogo/settings.toml`:

```toml
[wifi]
ssid = "MyNetwork"
password = "secret"
```

WiFi is only on while this screen is open; **B** switches it off again.

### Updating the launcher

Build an image and upload `dist\launcher.bin` on the **Settings** tab under "Update launcher":

```powershell
cargo xtask release
```

The device writes it into the launcher slot that isn't running and restarts. The new launcher
confirms itself once it's up; if it crashes before that, the device returns to the previous one and
says so. Settings → Device shows both slots with version and commit.

## Recovery

Hold **MENU** while switching on, for about a second. The recovery app can start either launcher slot,
or install `/ogo/updates/launcher.bin` from the SD card into launcher slot A. To put a launcher image
there:

```powershell
cargo xtask release
Copy-Item dist\launcher.bin E:\ogo\updates\launcher.bin
```

If MENU wasn't held (for example right after flashing), recovery starts the launcher by itself.
Holding **SELECT** at power-on opens the launcher's hardware test screen.

## Project layout

| Path | Contents |
|---|---|
| [ogo-board/](ogo-board/) | Board support (display, buttons, speaker, battery, LED, SD card), flash writing, shared settings and look (`ui`), the image description and the launcher/game boot contract |
| [ogo-launcher/](ogo-launcher/) | Launcher firmware: game list, installer, WiFi and web page, settings screen, boot screen, hardware test screen |
| [ogo-recovery/](ogo-recovery/) | Recovery app in the factory slot |
| [ogo-sdk/](ogo-sdk/) | Game SDK: frame loop, input, sound, saves, pause menu, return to the launcher |
| [ogo-format/](ogo-format/) | File formats and pure logic shared by the firmware and xtask, with unit tests that run on the PC |
| [games/snake/](games/snake/) | Snake, built on `ogo-sdk` |
| [games/hello/](games/hello/) | Minimal test game using `ogo-board` directly |
| [xtask/](xtask/) | Build tasks (`cargo xtask pack / release / flash / test / samples / icon`), run on the PC |
| [docs/](docs/) | Architecture, contracts, game guide, hardware notes |
| [partitions.csv](partitions.csv) | Flash layout |
| [rust-toolchain.toml](rust-toolchain.toml) | The pinned Xtensa toolchain |
| [.cargo/config.toml](.cargo/config.toml) | Target, linker, ESP-IDF version, Python version for the build, the xtask alias |
| [sdkconfig.defaults](sdkconfig.defaults) | ESP-IDF settings: flash, PSRAM, bootloader (MENU for recovery, rollback), scheduler, FAT |
| [tools/make_logo.py](tools/make_logo.py) | Draws the boot logo (mascot and wordmark) |

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

## License

[MIT](LICENSE).

ODROID and ODROID-GO are trademarks of Hardkernel. ogo is an independent project, not affiliated with
Hardkernel.
