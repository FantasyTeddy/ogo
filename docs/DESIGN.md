# Odroid-Go Rust Launcher: Design & Roadmap

*As of 2026-10-11*

A Rust launcher for the Odroid-Go. It installs native Rust games from the SD card into a flash slot
and boots them. You manage games and update the launcher itself from a web page served by the device.

---

## 1. Decisions so far

| Topic | Decision |
|---|---|
| Language | Rust, both the launcher and the games |
| Framework | `esp-idf-svc` (Rust `std` on top of ESP-IDF) for WiFi, HTTP, OTA, partitions and the SD card |
| Game format | Native firmware images (`.bin`), stored on the SD card and copied into a flash slot to run |
| Game updates | Each game is installed and updated on its own; replacing a game on the SD card is enough |
| PC side | A web page served by the device (upload, delete, update). No PC tool needed for transfer |
| Launcher updates | OTA with A/B slots and rollback, uploaded through the same web page |
| Emulators | Later. They get treated as installable "games" that load ROMs from the SD card |
| Scope | A longer project built in milestones (see §9) |

**Not in scope for now:** games in languages other than Rust, a PC CLI, pulling updates from an internet
server, or interpreted/WASM games.

---

## 2. Hardware constraints that shape the design

| Item | Odroid-Go | Consequence |
|---|---|---|
| SoC | ESP32 (dual Xtensa LX6, 240 MHz) | Needs the Xtensa Rust toolchain from esp-rs (`espup`) |
| Flash | 16 MB | Room for a recovery app, 2 launcher slots, and 1 or 2 game slots |
| PSRAM | 4 MB | **Data only.** The original ESP32 cannot run code from PSRAM or the SD card, so games must run from flash |
| Display | 2.4" ILI9341, 320×240, SPI | `mipidsi` + `embedded-graphics` |
| SD card | microSD, **shares the SPI bus with the LCD** | Both need a shared-bus driver with separate CS lines |
| Input | D-pad (2 ADC axes), A, B, Select, Start, Menu, Volume | Menu is a good recovery button at boot. Volume is on GPIO0, a strapping pin: holding it at reset enters serial download mode |
| Audio | Speaker on DAC GPIO26 through an amplifier, enabled by GPIO25 | Simple PCM/tones via the DAC or I2S-DAC mode. The amplifier needs time to wake up (see below) |
| Battery | Li-Po, voltage sensed via ADC | Battery indicator in the launcher |

Pin numbers, checked against the official library's
[`src/utility/Config.h`](https://github.com/hardkernel/ODROID-GO/blob/master/src/utility/Config.h)
on 2026-10-11: LCD CS 5, DC 21, backlight 14, no reset pin; SD CS 22; SPI CLK 18 / MOSI 23 /
MISO 19; A 32, B 33, Select 27, Start 39, Menu 13, Volume 0; D-pad X 34, Y 35; LED 2; speaker
enable 25, speaker out 26; battery sense 36 (not in that file, but confirmed in M1). All are in
`ogo-board/src/pins.rs`.

Display details confirmed in M0:
- Use **SPI3 (VSPI)**. Pins 18/23/19/5 are its IOMUX pins, which allows 40 MHz. Through SPI2 the
  signals go via the GPIO matrix, and ESP-IDF rejects anything above ~26.7 MHz.
- Orientation for landscape 320×240: rotate 90° **and** flip horizontally (the panel is mounted
  mirrored). Color order is BGR.
- Keep SD CS (22) high while only the LCD is in use.

Other hardware details confirmed in M1:
- **Shared SPI bus.** One `SpiDriver` (with DMA) is shared by the LCD device and the SD host. ESP-IDF's
  SPI master driver serializes transactions per bus, so no extra locking is needed.
- **SD card.** FAT with long filenames needs `CONFIG_FATFS_LFN_HEAP`; the default is 8.3 names only.
- **D-pad.** Each axis reads ~0 at rest, 4095 for left/up and ~1880 for right/down (12-bit, 12 dB
  attenuation). Thresholds 1024 and 3072, the same as Retro-Go.
- **Battery.** 1:2 divider. Measured 3.87 V while charging and 3.72 V on battery. The percentage is
  linear between 3.5 and 4.2 V, as in Retro-Go, so it's only a rough guide.
- **Speaker.** GPIO25 enables the amplifier (high = on) and GPIO26 carries the signal on DAC channel 1.
  The official library's name `SPEAKER_ENABLE_PIN` is right; driving both pins differentially (as
  Retro-Go's config suggests) only works when GPIO25 stays above the amplifier's enable threshold.
  The amplifier needs tens of milliseconds to wake up, so enabling it per beep swallows short beeps.
  It therefore stays on unless the volume is 0.
- **Audio output** (reworked in M2). The DAC stays in DMA mode (32 kHz, 8-bit) from boot on, and an
  `audio` thread keeps feeding it: queued sounds, otherwise the mid level. Every switch between DAC
  modes, or leaving the input undriven, made the amplifier click or crackle. Notes are sines with
  5 ms fades. The DAC's hardware cosine generator was tried first and dropped: it always starts at a
  peak, so every tone clicked. This audio thread is the starting point for game audio in M4.
- **LED** (GPIO2). The software drives it, but no LED was visible on the test unit.

Each ESP32 app partition is mapped through the MMU to the same virtual addresses. **So one game image
runs from any app slot without being rebuilt.** That is what makes "copy from SD, then boot" work.

---

## 3. Architecture

```
 ┌───────────────────────────── PC (browser) ─────────────────────────────┐
 │  http://<device-ip>/   upload .ogo game · delete · upload launcher.bin │
 └──────────────────────────────────┬─────────────────────────────────────┘
                                    │ WiFi (HTTP)
 ┌──────────────────────────────────▼─────────────────────────────────────┐
 │ LAUNCHER (ota_0 / ota_1)                                               │
 │  menu UI · game library · installer · web server · self-OTA · settings │
 └───────┬──────────────────────────────┬───────────────────────────┬─────┘
         │ reads/writes                 │ writes image + sets boot  │ uses
 ┌───────▼────────┐            ┌────────▼────────┐          ┌───────▼───────┐
 │ SD card        │            │ GAME SLOT(s)    │          │ ogo-board     │
 │ /ogo/games/... │            │ ota_2 (, ota_3) │          │ (shared crate)│
 └────────────────┘            └────────┬────────┘          └───────▲───────┘
                                        │ boots into                │ uses
                               ┌────────▼────────┐                  │
                               │ GAME            ├──── ogo-sdk ─────┘
                               │ (Rust app)      │  exit_to_launcher()
                               └─────────────────┘
 RECOVERY (factory): tiny fallback, entered by holding MENU at power-on
```

### Crates (one Cargo workspace)

| Crate | Purpose |
|---|---|
| `ogo-board` | Board support: display, buttons, audio, SD card, battery, LED. Used by everything |
| `ogo-sdk` | Game-facing API: frame loop, input, sound, saves, `exit_to_launcher()`, metadata macro |
| `ogo-launcher` | The launcher firmware |
| `ogo-recovery` | Minimal factory app: boot launcher A/B, or accept a launcher upload over WiFi |
| `games/*` | Example games (e.g. `snake`, `breakout`) |
| `xtask` | PC-side build helper: `cargo xtask pack <game>` → builds and produces a `.ogo` file |

---

## 4. Storage layout

### Flash partition table (16 MB). Sizes are initial guesses; tune them once real binary sizes are known

| Name | Type / subtype | Offset | Size | Content |
|---|---|---|---|---|
| nvs | data / nvs | 0x9000 | 16 KB | Settings, WiFi credentials, slot bookkeeping |
| otadata | data / ota | 0xD000 | 8 KB | Which app slot boots |
| phy_init | data / phy | 0xF000 | 4 KB | RF calibration |
| recovery | app / factory | 0x10000 | 1.5 MB | `ogo-recovery` |
| launcher_a | app / ota_0 | 0x190000 | 2.5 MB | Launcher, slot A |
| launcher_b | app / ota_1 | 0x410000 | 2.5 MB | Launcher, slot B |
| game_0 | app / ota_2 | 0x690000 | ~9.4 MB | Installed game |

A second game slot (`ota_3`) is possible by splitting `game_0`. It would act as a cache, so switching
between two games skips the re-flash. Begin with one slot.

App partitions must be 64 KB aligned. Note that Rust `std` + WiFi + HTTP firmware tends to be over 1 MB.
For reference, the M0 hello world (`std`, logging, display) is about 450 KB.

### SD card

```
/ogo/
  games/<game-id>/
    game.toml        # metadata (see §7)
    game.bin         # firmware image
    icon.rgb565      # 64×64 raw RGB565, little-endian, row by row: 8192 bytes (optional)
  saves/<game-id>/   # game save files, owned by the game via ogo-sdk
  updates/           # temporary staging for uploads
  settings.toml      # optional human-editable settings (WiFi etc.)
```

---

## 5. Boot, launch and recovery flows

### Launching a game

1. The user picks a game in the launcher.
2. The launcher compares the game's `sha256` (from `game.toml`) with the hash stored in NVS for `game_0`.
   - **Same** → skip to step 4.
   - **Different** → step 3.
3. Erase `game_0`, stream `game.bin` from the SD card into it with a progress bar, and verify the image.
   Use raw `esp_ota_begin` / `esp_ota_write` / `esp_ota_end` on the `game_0` partition. The safe
   `EspOta` wrapper always picks the "next" slot, so it doesn't fit here. Then store the hash in NVS.
4. Write `launcher_slot = <current launcher slot>` to NVS, set the boot partition to `game_0`, and restart.

### Inside the game: "boot-back" trick

The first thing `ogo_sdk::init()` does is **set the boot partition back to the launcher slot** (read
from NVS). After that, any reset, crash, watchdog or power-cycle lands in the launcher. The game only
runs when the launcher deliberately started it.

`exit_to_launcher()` then only needs to restart.

### Recovery (fallback for when the launcher itself is broken)

- Bootloader option `CONFIG_BOOTLOADER_FACTORY_RESET` with the pin set to **MENU (GPIO13)**: holding
  MENU at power-on boots the `recovery` (factory) partition.
  - Configure the "data partitions to erase on factory reset" list so it does **not** wipe NVS. Check
    the exact option names in the ESP-IDF version you use.
- Recovery shows a small menu: boot launcher A, boot launcher B, or "WiFi upload mode" to receive a
  fresh `launcher.bin`.
- The last resort is still USB serial flashing with `espflash`.

### Launcher self-update (A/B with rollback)

1. Upload `launcher.bin` through the web page. It is streamed into the *inactive* launcher slot.
2. Set it as the boot partition and restart.
3. With `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`, the new launcher must call
   `mark_running_slot_valid()` after a successful self-check (display up, SD card mounted). Otherwise
   the next reset rolls back to the old slot.

---

## 6. On-device web UI

- Served by `EspHttpServer` when the launcher's "Connect" screen is open, or always when WiFi is on
  (this is a setting).
- WiFi: station mode with credentials from NVS / `settings.toml`. If none are stored, fall back to
  SoftAP mode (`ODROID-GO-xxxx`) so you can enter credentials in the browser.
- Plain HTML + a bit of JS, embedded in the firmware with `include_str!`. No frameworks.

| Endpoint | Method | Purpose |
|---|---|---|
| `/` | GET | Page: game list, upload form, launcher version, battery, free SD space |
| `/api/games` | GET | JSON list of installed games |
| `/api/games` | POST | Upload a `.ogo` package (streamed to `/ogo/updates/`, then unpacked and validated) |
| `/api/games/<id>` | DELETE | Remove a game (and optionally its saves) |
| `/api/saves/<id>` | GET | Download a game's saves as a backup |
| `/api/launcher` | POST | Upload `launcher.bin` → self-OTA (§5) |
| `/api/reboot` | POST | Restart |

- Security: the server is only reachable on your LAN. Add a simple PIN/token that is shown on the
  device screen and required for write endpoints.
- Uploads are streamed in chunks straight to the SD card or flash. Never hold a whole image in RAM.
- Nice to have later: mDNS (`odroid-go.local`) via the ESP-IDF `mdns` component.

---

## 7. Game package & SDK

### `.ogo` package

An uncompressed tar file (easy to stream and unpack with the `tar` crate) containing:

```toml
# game.toml
id          = "snake"            # [a-z0-9-], unique, folder name
name        = "Snake"
version     = "1.2.0"
author      = "Matthias"
sdk_version = 1                  # launcher refuses packages with an unsupported SDK version
sha256      = "…"                # of game.bin, filled in by `cargo xtask pack`
```

plus `game.bin` and optionally `icon.rgb565`.

The launcher validates the package on upload:
- the `id` is valid
- the SHA-256 matches
- the ESP app header is valid
- the `sdk_version` is supported
- the image fits into `game_0`

Then it replaces `/ogo/games/<id>/` atomically: unpack to a temp folder, then rename.

### `ogo-sdk` API sketch

```rust
use ogo_sdk::{prelude::*, Game, Ctx};

struct Snake { /* state */ }

impl Game for Snake {
    fn new(ctx: &mut Ctx) -> Self {
        let best = ctx.saves().read::<u32>("best").unwrap_or(0);
        Snake { /* … */ }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let input = ctx.input();               // pressed / held / released per button
        if input.pressed(Button::Menu) {
            ctx.exit_to_launcher();            // restart into the launcher
        }
        // game logic …
    }

    fn draw(&self, fb: &mut Framebuffer) {    // 320×240 RGB565 in PSRAM, embedded-graphics DrawTarget
        fb.clear(Rgb565::BLACK);
        // …
    }
}

ogo_sdk::main!(Snake);                         // sets up board, boot-back, frame loop (e.g. 30/60 fps)
```

SDK responsibilities:
- board init
- the boot-back trick (§5)
- a fixed-timestep frame loop
- a double-buffered framebuffer pushed to the LCD (DMA where possible)
- button debouncing
- simple audio (tones / PCM)
- saves under `/ogo/saves/<id>/`
- a battery level query
- an in-game pause overlay on MENU (resume / exit)

---

## 8. Toolchain & project setup

1. **Install the Rust Xtensa toolchain**
   ```
   cargo install espup espflash ldproxy cargo-generate
   espup install
   ```
   On Windows, `espup` writes an `export-esp.ps1`. Run it in each shell, or add it to your profile.
2. **Project setup** (done in M0). The workspace is based on the esp-rs `esp-idf-template` for
   `std` / ESP-IDF v5.5.3, target `esp32`. `sdkconfig.defaults` enables 16 MB flash and PSRAM,
   including `CONFIG_SPIRAM_CACHE_WORKAROUND`, which this board's ESP32 revision v1.0 needs.
3. **Windows path length.** `esp-idf-sys` refuses to build if its output directory is too long, and
   enabling Windows long paths does not help. The source stays where it is, but `.cargo/config.toml`
   sets `target-dir = "D:/ogot"` (plus `CARGO_WORKSPACE_DIR`, which embuild then needs).
4. **Python version.** ESP-IDF 5.5 can't install its Python requirements on Python 3.15, and embuild
   always uses the first `python` on PATH. `env.ps1` loads `export-esp.ps1` and puts Python 3.12
   first. Dot-source it in each new shell: `. .\env.ps1`.
5. **Flash + monitor:**
   ```
   cargo espflash flash --release --package ogo-launcher --target-app-partition launcher_a --port COM7 --monitor
   ```
   `espflash.toml` supplies `partitions.csv`. cargo-espflash 4.3 ignores `target_app_partition` in
   that file, hence the flag. Until the recovery app exists (M5) the factory slot is empty and the
   bootloader falls back to `launcher_a`; add `--erase-parts otadata,recovery` if it boots something else.
6. **Useful crates:**
   - `esp-idf-svc`, `esp-idf-hal`, `esp-idf-sys`
   - `mipidsi`, `embedded-graphics`, `embedded-hal-bus` (shared SPI)
   - `serde` + `toml`, `tar`, `sha2`

Check current crate versions and the template's ESP-IDF version when you start. The esp-rs ecosystem
moves quickly.

---

## 9. Roadmap

| # | Milestone | Done when |
|---|---|---|
| M0 | Toolchain & hello world | A Rust app built on Windows draws text on the LCD and logs over serial. **Done 2026-10-11** |
| M1 | `ogo-board` crate | Display, all buttons, speaker beep, SD card (shared bus with LCD), battery reading all work. **Done 2026-10-11** (LED not visually confirmed) |
| M2 | Launcher UI (offline) | Scrollable game list read from `/ogo/games/*/game.toml`, with icons and battery. **Done 2026-10-11** |
| M3 | Install & launch | Picking a game flashes `game_0` (with progress + hash skip) and boots it. **Done 2026-10-11** |
| M4 | `ogo-sdk` + first game | `snake` built on the SDK. The boot-back trick and `exit_to_launcher()` work, and saves persist |
| M5 | Recovery | Partition table final, MENU-at-boot enters `ogo-recovery`, which can boot the launcher |
| M6 | WiFi + web UI | Upload / delete games and view status from the browser. PIN protection |
| M7 | Launcher self-OTA | Upload `launcher.bin` via the web page. A/B switch with rollback, tested with a deliberately broken build |
| M8 | Polish | Settings screen (WiFi, brightness, volume), save backup download, mDNS, `cargo xtask pack` |
| M9 | Emulators | First emulator packaged as a "game" that browses ROMs on the SD card |

M0 to M4 give you a usable device without any networking. M6 and M7 deliver the OTA goal.

---

## 10. Risks & open questions

- **Binary size.** Rust `std` + WiFi images can reach 1.5 MB or more. Measure this in M0/M6 and adjust
  the partition table *before* M5 freezes it. Changing the table later means a USB re-flash.
- **Shared SPI bus.** Resolved in M1: ESP-IDF's SPI master driver arbitrates between the LCD and
  SD devices. Recheck once the SDK pushes full frames by DMA while a game reads saves.
- **Speaker idle noise (M4).** With the amplifier always on, faint static is audible close to the
  speaker. In M4, switch the amplifier off after ~1 s of silence and wait for it to wake up before
  the next sound. Also compare against Go-Play's idle noise.
- **Partition-write API.** Writing a *specific* OTA slot needs `esp-idf-sys` FFI (unsafe). Wrap it in
  a small safe module.
- **Emulators (M9).** Either port individual C emulators (e.g. gnuboy, nofrendo) behind the SDK, or
  find out whether a Retro-Go build can be packaged as a single image. Retro-Go normally uses its own
  multi-partition layout, so it probably won't drop in as-is.
- **Number of game slots.** One slot is simplest. Two slots save re-flashing when you alternate
  between games, but each would hold only ~4.7 MB. Decide this after measuring game sizes.
- **Version compatibility.** Games depend on the launcher's partition table and NVS keys. `sdk_version`
  in `game.toml` guards against mismatches. Bump it whenever the boot contract changes.
