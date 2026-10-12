# Contracts

The interfaces that separate pieces of ogo rely on: the flash layout, what lives on the SD card and in
NVS, the rules for switching between launcher and games, and the file and network formats. Changing
any of these can break a launcher, recovery or game built earlier, so check this page first.

| Contract | Defined in |
|---|---|
| [Partition table](#partition-table) | [partitions.csv](../partitions.csv), [slots.rs](../ogo-format/src/slots.rs) (labels, checked against the CSV by a test) |
| [SD card layout](#sd-card-layout) | [library.rs](../ogo-launcher/src/library.rs), [saves.rs](../ogo-sdk/src/saves.rs) |
| [NVS keys](#nvs-keys) | [boot.rs](../ogo-board/src/boot.rs), [settings.rs](../ogo-board/src/settings.rs), [launcher settings.rs](../ogo-launcher/src/settings.rs) |
| [Boot contract](#boot-contract) | [boot.rs](../ogo-board/src/boot.rs) |
| [Image description](#image-description) | [ogo-format image.rs](../ogo-format/src/image.rs), [ogo-board image.rs](../ogo-board/src/image.rs) |
| [`game.toml` and `.ogo` packages](#gametoml-and-ogo-packages) | [game.rs](../ogo-format/src/game.rs), [package.rs](../ogo-format/src/package.rs) |
| [Web API](#web-api) | [web.rs](../ogo-launcher/src/web.rs) |
| [Compatibility](#compatibility) | [game.rs](../ogo-format/src/game.rs) (`SDK_VERSION`) |

## Partition table

16 MB flash. Changing it means flashing over USB and reinstalling every launcher slot. App slots
must be 64 KB aligned.

| Name | Type / subtype | Offset | Size | Content |
|---|---|---|---|---|
| nvs | data / nvs | 0x9000 | 16 KB | Settings, WiFi network, slot bookkeeping |
| otadata | data / ota | 0xD000 | 8 KB | Which app slot boots |
| phy_init | data / phy | 0xF000 | 4 KB | RF calibration |
| recovery | app / factory | 0x10000 | 2 MB | `ogo-recovery` |
| launcher_a | app / ota_0 | 0x210000 | 2.5 MB | Launcher, slot A |
| launcher_b | app / ota_1 | 0x490000 | 2.5 MB | Launcher, slot B |
| game_0 | app / ota_2 | 0x710000 | ~8.9 MB | The installed game |

Image sizes: launcher 1.6 MB (WiFi and the HTTP server make up most of it), recovery 620 KB, Snake
670 KB.

## SD card layout

```
/ogo/
  games/<game-id>/
    game.toml        # metadata, see below
    game.bin         # firmware image
    icon.rgb565      # optional, see below
  games/.incoming/   # an upload being checked; folders starting with "." are ignored
  saves/<game-id>/   # one file per save name, written by ogo-sdk
  updates/
    upload.ogo       # an upload from the web page, while it's received and installed
    launcher.bin     # a launcher image recovery can install into launcher_a
  settings.toml      # optional, for the first WiFi setup
```

Deleting a game on the web page removes its folder under `games/`; its saves stay.

**`settings.toml`** is only read when NVS has no WiFi network:

```toml
[wifi]
ssid = "MyNetwork"
password = "secret"
```

## NVS keys

All in the namespace `ogo` of the default NVS partition. Keys are limited to 15 characters.

| Key | Type | Written by | Purpose |
|---|---|---|---|
| `launcher` | str | launcher | Launcher slot that started the game |
| `game0_sha256` | str | launcher | SHA-256 of the image in `game_0`; removed while it's being replaced |
| `game0_addr` | u32 | launcher | Flash address of `game_0` when the hash was stored; a moved slot counts as empty |
| `update_target` | str | launcher | Slot a launcher update went to, until the next start has checked it |
| `wifi_ssid`, `wifi_pass` | str | launcher | The WiFi network set on the web page; removed by "Forget WiFi network" |
| `brightness` | u8 | launcher (settings screen) | Backlight in percent: 10, 20, … 100. Default 100 |
| `volume` | u8 | launcher (settings screen), games (VOL) | Volume in percent: 0, 25, 50, 75, 100. Default 50 |

Missing keys mean the defaults. Values between the steps are rounded to the nearest step.

ESP-IDF's WiFi driver keeps its own data in NVS too, under its own namespaces.

## Boot contract

The rules for switching between launcher and games. The module docs of
[boot.rs](../ogo-board/src/boot.rs) are the reference; in short:

1. **Launching.** The launcher writes the game into `game_0` (unless `game0_sha256` already matches),
   stores its own slot under `launcher`, makes `game_0` the boot slot and restarts.
2. **Boot-back.** The first thing a game does (`Boot::return_to_launcher_on_reset`, called by the
   SDK) is confirm its slot and make the launcher slot from `launcher` the boot slot again. From then
   on any reset, crash or power-off lands in the launcher. `exit_to_launcher()` only restarts.
3. **Rollback.** App rollback is enabled, so every newly selected slot starts "pending verify" and
   must confirm itself (`mark_running_valid`), or the next reset goes back to the previous slot. The
   launcher confirms itself on every start, once the display and SD card work; games confirm in step 2.
4. **Launcher updates** go into the launcher slot that isn't running. The launcher selects it, stores
   it under `update_target` and restarts. The next start compares: running from `update_target`
   means the update worked; otherwise it was rolled back.
5. **Recovery** lives in the factory slot. Holding MENU for 1 s at power-on makes the bootloader clear
   otadata and start it. Started without MENU (otadata was empty, e.g. after flashing), it starts the
   first slot holding an `ogo-launcher` image.

## Image description

Every ogo firmware image carries a 80-byte description right after ESP-IDF's `esp_app_desc_t`, at
offset 288 of the image (section `.rodata_custom_desc`). ESP-IDF's own description can't tell ogo
apps apart: with esp-idf-sys every project is called `libespidf`.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | Magic `OGO2` |
| 4 | 28 | Kind: `ogo-launcher`, `ogo-recovery` or `ogo-game` |
| 32 | 32 | Crate version, e.g. `0.1.0` |
| 64 | 16 | Short git commit, e.g. `0a1b710c`; empty unless built with `cargo xtask` |

Strings are zero-terminated (so at most size − 1 characters). Declared with
`ogo_board::image_desc!(Launcher | Recovery | Game)` (variants of `ImageKind`); the SDK's `main!`
does it for games. The commit comes from the `OGO_COMMIT` environment variable at compile time.
`cargo xtask` checks the kind and commit of every image it builds.

Who relies on it:
- Recovery only starts a slot whose kind is `ogo-launcher`.
- The web page only accepts a launcher update whose kind is `ogo-launcher`, since a game in a
  launcher slot would never return.
- The settings screen, the web page and recovery show version and commit.

**Build ID.** Not part of the image: the first 8 hex digits of the SHA-256 of the image in a slot,
computed on the device (~0.1 s for 1.6 MB). It tells apart builds of the same version and commit,
e.g. with uncommitted changes. (ESP-IDF's `app_elf_sha256` would be cheaper, but espflash leaves it
zero.)

## `game.toml` and `.ogo` packages

**`game.toml`**, written by `cargo xtask pack` from the game crate's Cargo.toml:

```toml
id = "snake"            # [package] name: a-z, 0-9 and -; also the folder name
name = "Snake"          # [package.metadata.ogo] name
version = "1.0.0"       # [package] version
author = "ogo examples" # first of [package] authors; may be empty
sdk_version = 1         # SDK_VERSION of the SDK the game was built with
sha256 = "693bbd…"      # of game.bin, lowercase hex
```

**`icon.rgb565`** (optional): 64×64 pixels, RGB565 little-endian, row by row, exactly 8192 bytes.
A file of another size is ignored.

**`.ogo` package**: an uncompressed ustar archive with `game.toml`, `game.bin` and optionally
`icon.rgb565`, with flat names. Other entries are skipped. `ogo-format` has its own small tar reader
and writer (the `tar` crate pulls in `filetime`, which needs Unix calls ESP-IDF lacks).

The launcher accepts a game folder or package when:
- `game.toml` parses, `id` is valid and matches the folder name
- `sdk_version` is the one the launcher supports
- `sha256` is a 64-digit hex string and `game.bin` exists
- for uploads also: `game.bin` matches `sha256` and fits into `game_0`

An upload (max 10 MB) is unpacked into `games/.incoming/`, checked, and only then moved to
`games/<id>/`, replacing an older version. A broken upload never replaces a working game. The image
itself is checked when it's written into `game_0`.

## Web API

Served only while the WiFi screen is open, on port 80. Everything under `/api/` except the login
needs the session cookie `ogo_session` (`HttpOnly`, `SameSite=Strict`), which `POST /api/login` sets
for the PIN shown on the device. After 5 wrong PINs the device picks a new one. Errors are JSON
`{"message": "…"}`.

| Endpoint | Method | Purpose |
|---|---|---|
| `/`, `/logo.png` | GET | The page and the logo (no session needed) |
| `/api/login` | POST | `{"pin": "1234"}` → session cookie |
| `/api/status` | GET | Version, battery, SD space, both launcher slots (kind, version, build ID, commit, build time), network |
| `/api/games` | GET | All library entries, including broken ones with the reason |
| `/api/games` | POST | Upload an `.ogo` package as the raw body |
| `/api/games/<id>/icon.bmp` | GET | The icon as a 16-bit BMP (RGB565 bitfields) |
| `/api/games/<id>` | DELETE | Delete a game folder; its saves stay |
| `/api/launcher` | POST | A `launcher.bin` as the raw body: written into the inactive launcher slot, then the device restarts into it |
| `/api/wifi/networks` | GET | Networks from the last scan |
| `/api/wifi/scan` | POST | Scan again |
| `/api/wifi` | POST | `{"ssid", "password"}`: save to NVS and join |
| `/api/restart` | POST | Restart the device |

## Compatibility

- **`sdk_version`** in `game.toml` must equal the launcher's `SDK_VERSION`. There is one
  definition, `ogo_format::game::SDK_VERSION` (currently 1): the launcher accepts it, and
  `cargo xtask pack` writes it, since games are built against the SDK in the same tree. Bump it
  when a change breaks games built earlier: the boot contract, the partition table, or what the
  SDK expects from NVS.
- New NVS keys with defaults don't need a bump: games that don't know a key ignore it, and games
  that read one fall back to its default when it's missing.
- Changing the image description's layout needs a new magic, and recovery and launcher must then be
  flashed together (`cargo xtask flash --all`): each only recognises images with its own magic.
