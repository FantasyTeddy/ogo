# ogo: Design & Roadmap

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

### Flash partition table (16 MB). Final since M5

| Name | Type / subtype | Offset | Size | Content |
|---|---|---|---|---|
| nvs | data / nvs | 0x9000 | 16 KB | Settings, WiFi credentials, slot bookkeeping |
| otadata | data / ota | 0xD000 | 8 KB | Which app slot boots |
| phy_init | data / phy | 0xF000 | 4 KB | RF calibration |
| recovery | app / factory | 0x10000 | 2 MB | `ogo-recovery` |
| launcher_a | app / ota_0 | 0x210000 | 2.5 MB | Launcher, slot A |
| launcher_b | app / ota_1 | 0x490000 | 2.5 MB | Launcher, slot B |
| game_0 | app / ota_2 | 0x710000 | ~8.9 MB | Installed game |

Recovery got 2 MB instead of the first guess of 1.5 MB, because it gets WiFi upload in M7 (WiFi alone
adds about 600 KB). Sizes at M5: recovery 620 KB, launcher 900 KB, Snake 650 KB. The NVS entry for
the installed game stores `game_0`'s address too, so a moved slot counts as empty.

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

- Bootloader option `CONFIG_BOOTLOADER_FACTORY_RESET` with the pin set to **MENU (GPIO13)**, active
  low, held for 1 s (`CONFIG_BOOTLOADER_HOLD_TIME_GPIO`). The bootloader only boots the factory slot
  if it also clears otadata (`CONFIG_BOOTLOADER_OTA_DATA_ERASE`); otherwise it boots the usual slot.
  The list of data partitions to erase is empty, so NVS survives.
- Because otadata is cleared, every later reset also lands in recovery until a launcher is chosen.
  Recovery tells a MENU boot from an empty otadata (e.g. after flashing) by the bootloader's
  factory-reset flag in RTC memory (`bootloader_common_get_rtc_retain_mem_factory_reset_state`).
  Without MENU it starts the first valid launcher slot right away.
- Recovery's menu (built in M5): start launcher A or B (with version and build time), install
  `/ogo/updates/launcher.bin` from the SD card into launcher A, restart. "WiFi upload mode" follows
  in M7.
- The bootloader is the one esp-idf-sys builds from `sdkconfig.defaults`; `tools/flash.ps1` passes
  it to espflash, whose default bootloader lacks the MENU option.
- The last resort is still USB serial flashing.

### Launcher self-update (A/B with rollback)

1. Upload `launcher.bin` through the web page. It is streamed into the *inactive* launcher slot.
2. Set it as the boot partition and restart.
3. With `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`, the new launcher must call
   `mark_running_slot_valid()` after a successful self-check (display up, SD card mounted). Otherwise
   the next reset rolls back to the old slot.

---

## 6. On-device web UI

As built in M6:

- **WiFi screen.** SELECT in the game list opens it; B closes it. WiFi and the web server only run
  while it's open: the radio costs battery, games get the whole device, and the device is only
  reachable while someone is looking at that screen. ("Always on" could become a setting in M8.)
  Leaving the screen rescans the library if the page changed it.
- **Joining.** When the screen opens, the device first scans for networks (~2 s, station mode only),
  then joins the network from NVS (set via the page) or, for a first setup, from
  `/ogo/settings.toml` on the SD card (`[wifi] ssid = "…" password = "…"`). If there is none or
  joining fails (~15 s timeout), it opens an access point `ogo-XXXX` without password, named after
  the last two bytes of the chip's MAC address, so it's stable and unique. The access point runs in
  mixed mode, so rescanning still works (the access point drops out for a moment).
- **Device screen.** Numbered steps in the order you need them: (join the WiFi `ogo-XXXX`,) open
  `http://<ip>/`, enter the PIN.
- **PIN and sessions.** A random 4-digit PIN is shown on the device each time the screen opens. The
  page shows only a PIN prompt until `POST /api/login` succeeds, which sets an `HttpOnly`,
  `SameSite=Strict` session cookie (also needed for `<img>` icon requests, which can't send headers).
  Every other `/api/` endpoint needs that session. After 5 wrong PINs the device picks a new one.
- **Page.** Plain HTML/JS (`ogo-launcher/src/web/index.html`, embedded with `include_str!`), two tabs
  with hash routing (`#games`, `#settings`). Battery (drawn like on the device) and free SD space sit
  in the header. Networks are a list with signal bars and a lock, each row opening its own password
  field; "Other network…" covers hidden ones.

| Endpoint | Method | Purpose |
|---|---|---|
| `/`, `/logo.png` | GET | The page and the logo (no session needed) |
| `/api/login` | POST | `{"pin": "1234"}` → session cookie |
| `/api/status` | GET | Version, battery, SD space, network |
| `/api/games` | GET | All library entries, including broken ones with the reason |
| `/api/games` | POST | Upload an `.ogo` package (raw body), see below |
| `/api/games/<id>/icon.bmp` | GET | The icon as a 16-bit BMP (RGB565 bitfields, no conversion needed) |
| `/api/games/<id>` | DELETE | Delete a game folder; its saves stay |
| `/api/wifi/networks` | GET | Networks from the last scan |
| `/api/wifi/scan` | POST | Scan again |
| `/api/wifi` | POST | `{"ssid", "password"}`: save to NVS and join |
| `/api/restart` | POST | Restart the device |

Still to come: `/api/launcher` (self-update, M7) and `/api/saves/<id>` (save backup, M8).

- **Uploads** are streamed in 4 KB chunks to `/ogo/updates/upload.ogo` (max 10 MB), then unpacked by
  a minimal tar reader into `/ogo/games/.incoming/` (folders starting with a dot are ignored by the
  library), checked like a library entry plus the `game.bin` sha256 and size, and only then moved to
  `/ogo/games/<id>/`. A broken upload never replaces a working game. (The `tar` crate pulls in
  `filetime`, which needs Unix calls ESP-IDF lacks, hence the own reader.)
- WiFi and lwIP buffers may go to PSRAM (`CONFIG_SPIRAM_TRY_ALLOCATE_WIFI_LWIP`). The launcher grew
  from 0.9 to 1.6 MB with WiFi and the HTTP server (62 % of its slot).
- Nice to have later: mDNS (`ogo.local`) via the ESP-IDF `mdns` component.

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

As built in M4 (`ogo-sdk`, example: `games/snake`):
- `update` runs at `Game::FPS` (default 30); if the loop falls far behind it skips ahead instead of
  catching up. FreeRTOS runs at 1 kHz ticks (`CONFIG_FREERTOS_HZ`) for steady frame timing.
- Two full-screen `Canvas` buffers (150 KB each, in PSRAM). A `display` thread sends one to the LCD
  while the game draws the other. A full frame takes about 31 ms on the 40 MHz bus, so ~30 fps is
  the ceiling for full redraws.
- Input is sampled once per frame, which also debounces. `Ctx::input()` has pressed/held/released.
- Sound: `Ctx::sound().play(&[Note])`, sines with fades (see the audio notes in §2). No PCM yet.
- Saves are text files `/ogo/saves/<id>/<name>`, written via a temp file. `read::<T: FromStr>` and
  `write::<T: Display>`, plus raw bytes.
- `Ctx::random()` uses the hardware RNG. VOL cycles the volume with an on-screen indicator (not yet
  persisted; settings come in M8).
- Game crates need a `build.rs` with `embuild::espidf::sysenv::output()` and a direct
  `esp-idf-svc` dependency: the build script only sees ESP-IDF's linker arguments from a direct
  dependency.

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
5. **Flash:** `tools/flash.ps1` (launcher only) or `tools/flash.ps1 -All` (also recovery, and clears
   otadata). It builds, finds the bootloader esp-idf-sys built, and runs `cargo espflash flash` with
   `--bootloader`, `--target-app-partition` (cargo-espflash 4.3 ignores that key in `espflash.toml`)
   and the partition table from `espflash.toml`. `CONFIG_SPIRAM_MEMTEST=n` saves ~0.9 s per boot.
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
| M4 | `ogo-sdk` + first game | `snake` built on the SDK. The boot-back trick and `exit_to_launcher()` work, and saves persist. **Done 2026-10-11** |
| M5 | Recovery | Partition table final, MENU-at-boot enters `ogo-recovery`, which can boot the launcher. **Done 2026-10-11** |
| M6 | WiFi + web UI | Upload / delete games and view status from the browser. PIN protection. **Done 2026-10-11** |
| M7 | Launcher self-OTA | Upload `launcher.bin` via the web page. A/B switch with rollback, tested with a deliberately broken build |
| M8 | Polish | Settings screen (WiFi, brightness, volume), save backup download, mDNS, `cargo xtask pack`, charging indicator |
| M9 | Emulators | First emulator packaged as a "game" that browses ROMs on the SD card |

M0 to M4 give you a usable device without any networking. M6 and M7 deliver the OTA goal.

---

## 10. Risks & open questions

- **Binary size.** Rust `std` + WiFi images can reach 1.5 MB or more. Measure this in M0/M6 and adjust
  the partition table *before* M5 freezes it. Changing the table later means a USB re-flash.
- **Shared SPI bus.** Resolved in M1: ESP-IDF's SPI master driver arbitrates between the LCD and
  SD devices. Recheck once the SDK pushes full frames by DMA while a game reads saves.
- **Speaker idle noise.** Resolved in M4: the audio thread switches the amplifier off after 3 s of
  silence, and a sound that finds it off starts after 150 ms of idle level while it wakes up.
- **Partition-write API.** Writing a *specific* OTA slot needs `esp-idf-sys` FFI (unsafe). Wrap it in
  a small safe module.
- **Emulators (M9).** Either port individual C emulators (e.g. gnuboy, nofrendo) behind the SDK, or
  find out whether a Retro-Go build can be packaged as a single image. Retro-Go normally uses its own
  multi-partition layout, so it probably won't drop in as-is.
- **Number of game slots.** One slot is simplest. Two slots save re-flashing when you alternate
  between games, but each would hold only ~4.7 MB. Decide this after measuring game sizes.
- **Charging detection (M8).** The ODROID-GO wires no charge-status or USB-power signal to the ESP32
  (Retro-Go's board config only has the battery ADC). Plugging in raises the measured voltage by
  ~0.1–0.2 V at once (3.72 → 3.87 V measured), so a heuristic can show a charging icon on such a jump
  or a steady rise, and hide it on a sudden drop. It can't tell at power-on while already plugged in
  until the voltage has risen for a while, and the percentage reads high while charging.
- **Version compatibility.** Games depend on the launcher's partition table and NVS keys. `sdk_version`
  in `game.toml` guards against mismatches. Bump it whenever the boot contract changes.
