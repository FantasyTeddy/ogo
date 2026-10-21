# Contracts

The interfaces that separate pieces of ogo rely on: the flash layout, what lives on the SD card and in
NVS, the rules for switching between launcher and apps, and the file and network formats. Changing
any of these can break a launcher, recovery or app built earlier, so check this page first.

| Contract | Defined in |
|---|---|
| [Partition table](#partition-table) | [partitions.csv](../partitions.csv), [slots.rs](../ogo-format/src/slots.rs) (labels, checked against the CSV by a test) |
| [SD card layout](#sd-card-layout) | [library.rs](../ogo-launcher/src/library.rs), [saves.rs](../ogo-sdk/src/saves.rs) |
| [NVS keys](#nvs-keys) | [boot.rs](../ogo-board/src/boot.rs), [settings.rs](../ogo-board/src/settings.rs), [launcher settings.rs](../ogo-launcher/src/settings.rs) |
| [Boot contract](#boot-contract) | [boot.rs](../ogo-board/src/boot.rs) |
| [Image description](#image-description) | [ogo-format image.rs](../ogo-format/src/image.rs), [ogo-board image.rs](../ogo-board/src/image.rs) |
| [`app.toml` and `.ogo` packages](#apptoml-and-ogo-packages) | [app.rs](../ogo-format/src/app.rs), [package.rs](../ogo-format/src/package.rs) |
| [Web API](#web-api) | [web.rs](../ogo-launcher/src/web.rs) |
| [Compatibility](#compatibility) | [app.rs](../ogo-format/src/app.rs) (`SDK_VERSION`) |

## Partition table

16 MB flash. Changing it means flashing over USB and reinstalling every launcher slot. App slots
must be 64 KB aligned. Below the table: the bootloader at `0x1000` and the partition table itself at
`0x8000` (see [flash.rs](../ogo-format/src/flash.rs)).

| Name | Type / subtype | Offset | Size | Content |
|---|---|---|---|---|
| nvs | data / nvs | 0x9000 | 16 KB | Settings, WiFi network, slot bookkeeping |
| otadata | data / ota | 0xD000 | 8 KB | Which app slot boots |
| phy_init | data / phy | 0xF000 | 4 KB | RF calibration |
| recovery | app / factory | 0x10000 | 2 MB | `ogo-recovery` |
| launcher_a | app / ota_0 | 0x210000 | 2.5 MB | Launcher, slot A |
| launcher_b | app / ota_1 | 0x490000 | 2.5 MB | Launcher, slot B |
| app_0 | app / ota_2 | 0x710000 | ~8.9 MB | The installed app |

Image sizes: launcher 1.6 MB (WiFi and the HTTP server make up most of it), recovery 620 KB, Snake
670 KB.

## SD card layout

```
/ogo/
  apps/<app-id>/
    app.toml        # metadata, see below
    app.bin         # firmware image
    icon.rgb565      # optional, see below
  apps/.incoming/   # an upload being checked; folders starting with "." are ignored
  saves/<app-id>/   # one file per save name, written by ogo-sdk
  updates/
    upload.ogo       # an upload from the web page, while it's received and installed
    launcher.bin     # a launcher image recovery can install into launcher_a
  settings.toml      # optional, for the first WiFi setup
```

Apps can also read files from folders outside `/ogo/` they declare in `app.toml` (see below),
which the web page lets the user fill. The GB app declares `/roms/gb/`, the folder other emulators
for the device use too. An upload into such a folder is received as `.upload.part` in it and renamed
once complete; files starting with "." aren't listed.

Deleting an app on the web page removes its folder under `apps/`; its saves and the files in its
declared folders stay.

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
| `launcher` | str | launcher | Launcher slot that started the app |
| `app0_sha256` | str | launcher | SHA-256 of the image in `app_0`; removed while it's being replaced |
| `app0_addr` | u32 | launcher | Flash address of `app_0` when the hash was stored; a moved slot counts as empty |
| `update_target` | str | launcher | Slot a launcher update went to, until the next start has checked it |
| `wifi_ssid`, `wifi_pass` | str | launcher | The WiFi network set on the web page; removed by "Forget WiFi network" |
| `brightness` | u8 | launcher (settings screen) | Backlight in percent: 10, 20, … 100. Default 100 |
| `volume` | u8 | launcher (settings screen), apps (pause menu) | Volume level in percent as shown: 0, 10, 20, … 100. The speaker plays a curve of it, so each step sounds equally louder (10 → 10%, 50 → 28%, 100 → 100%; `VOLUME_OUTPUT`). Default 50 |
| `music` | u8 | launcher (settings screen), apps (pause menu, VOL) | Apps play music: 1 on, 0 off. Default on |
| `muted` | u8 | launcher (VOL), apps (VOL; changing the volume unmutes) | No sound, `volume` kept: 1 muted, 0 not. Default not muted |

Missing keys mean the defaults. Values between the steps are rounded to the nearest step.

ESP-IDF's WiFi driver keeps its own data in NVS too, under its own namespaces.

## Boot contract

The rules for switching between launcher and apps. The module docs of
[boot.rs](../ogo-board/src/boot.rs) are the reference; in short:

1. **Launching.** The launcher writes the app into `app_0` (unless `app0_sha256` already matches),
   stores its own slot under `launcher`, makes `app_0` the boot slot and restarts.
2. **Boot-back.** The first thing an app does (`Boot::return_to_launcher_on_reset`, called by the
   SDK) is confirm its slot and make the launcher slot from `launcher` the boot slot again. From then
   on any reset, crash or power-off lands in the launcher. `exit_to_launcher()` only restarts.
3. **Rollback.** App rollback is enabled, so every newly selected slot starts "pending verify" and
   must confirm itself (`mark_running_valid`), or the next reset goes back to the previous slot. The
   launcher confirms itself on every start, once the display and SD card work; apps confirm in step 2.
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
| 4 | 28 | Kind: `ogo-launcher`, `ogo-recovery` or `ogo-app` |
| 32 | 32 | Crate version, e.g. `0.1.0` |
| 64 | 16 | Short git commit, e.g. `0a1b710c`; empty unless built with `cargo xtask` |

Strings are zero-terminated (so at most size − 1 characters). Declared with
`ogo_board::image_desc!(Launcher | Recovery | App)` (variants of `ImageKind`); the SDK's `main!`
does it for apps. The commit comes from the `OGO_COMMIT` environment variable at compile time.
`cargo xtask` checks the kind and commit of every image it builds.

Who relies on it:
- Recovery only starts a slot whose kind is `ogo-launcher`.
- The web page only accepts a launcher update whose kind is `ogo-launcher`, since an app in a
  launcher slot would never return.
- The settings screen, the web page and recovery show version and commit.

**Build ID.** Not part of the image: the first 8 hex digits of the SHA-256 of the image in a slot,
computed on the device (~0.1 s for 1.6 MB). It tells apart builds of the same version and commit,
e.g. with uncommitted changes. (ESP-IDF's `app_elf_sha256` would be cheaper, but espflash leaves it
zero.)

## `app.toml` and `.ogo` packages

**`app.toml`**, written by `cargo xtask pack` from the app crate's Cargo.toml:

```toml
id = "snake"            # [package] name: a-z, 0-9 and -; also the folder name
name = "Snake"          # [package.metadata.ogo] name
kind = "game"           # [package.metadata.ogo] kind: "game" or "tool"
version = "1.0.0"       # [package] version
author = "ogo examples" # first of [package] authors; may be empty
sdk_version = 2         # SDK_VERSION of the SDK the app was built with
sha256 = "693bbd…"      # of app.bin, lowercase hex

[[folders]]             # optional, any number: [package.metadata.ogo] folders
label = "ROMs"          # what the files are, shown on the web page
path = "roms/gb"        # relative to the SD card root
extensions = ["gb"]     # without the dot; matched ignoring case
```

A folder's `path` has parts of `a-z`, `0-9`, `-` and `_` separated by `/`, and isn't under `ogo/`.
Uploaded file names have at most 255 bytes, one of the extensions, don't start with "." or end in "."
or a space, and contain none of `/ \ : * ? " < > |` or control characters. File names on the SD
card are UTF-8.

**`icon.rgb565`** (optional): 64×64 pixels, RGB565 little-endian, row by row, exactly 8192 bytes.
A file of another size is ignored.

**`.ogo` package**: an uncompressed ustar archive with `app.toml`, `app.bin` and optionally
`icon.rgb565`, with flat names. Other entries are skipped. `ogo-format` has its own small tar reader
and writer (the `tar` crate pulls in `filetime`, which needs Unix calls ESP-IDF lacks).

The launcher accepts an app folder or package when:
- `app.toml` parses, `id` is valid and matches the folder name
- `sdk_version` is the one the launcher supports
- the `folders`, if any, follow the rules above
- `sha256` is a 64-digit hex string and `app.bin` exists
- for uploads also: `app.bin` matches `sha256` and fits into `app_0`

An upload (max 10 MB) is unpacked into `apps/.incoming/`, checked, and only then moved to
`apps/<id>/`, replacing an older version. A broken upload never replaces a working app. The image
itself is checked when it's written into `app_0`.

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
| `/api/apps` | GET | All library entries with their `kind` and `folders`, sorted like the launcher's list, including broken ones (`kind` null) with the reason |
| `/api/apps` | POST | Upload an `.ogo` package as the raw body |
| `/api/apps/<id>/icon.bmp` | GET | The icon as a 16-bit BMP (RGB565 bitfields) |
| `/api/apps/<id>` | DELETE | Delete an app folder; its saves and the files in its folders stay |
| `/api/apps/<id>/folders/<n>` | GET | The files in the app's `n`th folder (from 0): `[{"name", "size"}]` |
| `/api/apps/<id>/folders/<n>/<name>` | POST | Upload a file (max 10 MB) as the raw body, `<name>` percent-encoded; replaces one with the same name |
| `/api/apps/<id>/folders/<n>/<name>` | DELETE | Delete a file |
| `/api/launcher` | POST | A `launcher.bin` as the raw body: written into the inactive launcher slot, then the device restarts into it |
| `/api/wifi/networks` | GET | Networks from the last scan |
| `/api/wifi/scan` | POST | Scan again |
| `/api/wifi` | POST | `{"ssid", "password"}`: save to NVS and join |
| `/api/restart` | POST | Restart the device |

## Compatibility

- **`sdk_version`** in `app.toml` must equal the launcher's `SDK_VERSION`. There is one
  definition, `ogo_format::app::SDK_VERSION` (currently 2): the launcher accepts it, and
  `cargo xtask pack` writes it, since apps are built against the SDK in the same tree. Bump it
  when a change breaks apps built earlier: the boot contract, the partition table, or what the
  SDK expects from NVS.
- New NVS keys with defaults don't need a bump: apps that don't know a key ignore it, and apps
  that read one fall back to its default when it's missing.
- New optional fields in `app.toml` don't need one either: launchers ignore fields they don't know
  (an older launcher shows no folders), and missing ones get their defaults.
- Changing the image description's layout needs a new magic, and recovery and launcher must then be
  flashed together (`cargo xtask flash --all`): each only recognises images with its own magic.
