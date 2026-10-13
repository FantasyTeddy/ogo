# Contributing

How to build, test and release ogo. For how it works inside, see
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); for writing games, [docs/GAMES.md](docs/GAMES.md).
Bugs and ideas go into [GitHub issues](https://github.com/FantasyTeddy/ogo/issues).

## Prerequisites

Developed on Windows with PowerShell; the build also runs on Linux (that's what CI uses).

1. **Rust** via [rustup](https://rustup.rs/).
2. **Xtensa toolchain and flashing tools:**
   ```powershell
   cargo install espup espflash cargo-espflash ldproxy
   espup install --name esp-1.97 --toolchain-version 1.97.0.0 --targets esp32 --std --export-file "$HOME\export-esp-1.97.ps1"
   ```
   The project is pinned to this toolchain in [rust-toolchain.toml](rust-toolchain.toml); it sits
   next to any other Xtensa toolchain you have. On Windows, `espup` also sets `LIBCLANG_PATH` in
   your user environment; on Linux and macOS, source the export script it wrote first.
3. **Python 3.14 or older**, because ESP-IDF's Python requirements don't install on 3.15 on Windows.
   With the [Python install manager](https://docs.python.org/3/using/windows.html) there's nothing to
   do: builds use 3.12, which the install manager installs on first use. Otherwise make sure the
   first `python` on PATH is 3.14 or older.
4. **On Windows, a short build folder**, see below.

ESP-IDF itself (v5.5.5) is downloaded automatically on the first build. That build takes several
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
cargo xtask release                    # everything a release contains, in dist\
cargo xtask test                       # unit tests, on the PC
```

`flash` uses this project's own bootloader (it starts recovery when MENU is held) and the partition
table from [partitions.csv](partitions.csv). Don't use `cargo espflash flash` directly: it would
flash espflash's default bootloader, without the MENU option (for the same reason `cargo run`
doesn't flash anything). Every image built by `cargo xtask` carries the git commit, shown on the
settings screen, the web page and in recovery.

`release` writes `ogo-<version>-full.bin` (the complete flash image users install), `launcher.bin`,
`recovery.bin` and every game's `.ogo` package. A plain `cargo build --release` also works; its
firmware ends up in `<build folder>\xtensa-esp32-espidf\release\`, without a commit.

## Games

```powershell
cargo xtask pack snake --sd E:\        # build a game and copy it to an SD card
cargo xtask pack snake                 # or write dist\snake.ogo, for the web page
cargo xtask icon snake snake.png       # make games/snake/icon.rgb565 from a PNG
cargo xtask samples E:\                # fake games on an SD card, to test the game list
```

Writing a game is described in [docs/GAMES.md](docs/GAMES.md).

## Testing

**CI** ([ci.yml](.github/workflows/ci.yml)) runs on every push to `main` and every pull request:
the `ogo-format` and `ogo-gb` tests and Clippy on them and xtask with stable Rust, then, on Linux with the
pinned Xtensa toolchain, the format check, Clippy on the firmware and `cargo xtask release`, whose
files are kept as an artifact of the run. Formatting differences and Clippy warnings fail the build.

**Formatting:** the code uses rustfmt's default style. Before committing, run `cargo fmt` in the
project folder, and with `--manifest-path` `ogo-format/Cargo.toml`, `ogo-gb/Cargo.toml` and
`xtask/Cargo.toml` for the three crates with their own workspace. Run
`git config blame.ignoreRevsFile .git-blame-ignore-revs` once, so `git blame` skips the commit that
introduced the formatting.

**On the PC:** `cargo xtask test` runs the unit tests of `ogo-format`: `game.toml`, the `.ogo` reader
and writer, the image description, the partition table and slot labels (checked against
`partitions.csv`), merging the flash image, settings steps, text fitting. Then those of `ogo-gb`, the
Game Boy core, which also run public test ROMs (Blargg's CPU and timing tests, Mooneye's CPU, timer
and MBC tests, and screenshot comparisons such as dmg-acid2; a differing picture is written to the
build folder's `tmp`). Unpack the
[test ROM collection v7.0](https://github.com/c-sp/game-boy-test-roms/releases/tag/v7.0) into
`ogo-gb\test-roms\` (ignored by git), or point `OGO_GB_TEST_ROMS` at it; without them those tests are
skipped. `cargo run --release --manifest-path ogo-gb/Cargo.toml --target host-tuple --example bench
-- <rom>` measures the core's speed on the PC. Everything that needs the ESP32 is tested on the device.

**On the device**, after `cargo xtask flash --all` (recovery too, when the shared crates changed):
1. The launcher starts; the settings screen shows the expected version, commit and slot; VOL mutes
   and unmutes.
2. A game starts (installing it first if it changed), its music plays under the sound effects, its
   pause menu (volume and music too) and VOL's three sound modes work, and "Exit to launcher" returns.
3. The WiFi screen: the web page logs in with the PIN, uploads and deletes a game, and the game list
   is rescanned when the screen closes.
4. A launcher update from the web page restarts into the other slot and says "Launcher updated".
5. Recovery (MENU at power-on) lists both launcher slots and can install `/ogo/updates/launcher.bin`.

A rollback can be tested with `cargo xtask release --features rollback-test`: that launcher panics
before confirming itself, so uploading it must bring back the previous one with "Update didn't
start". Before a release, also install the full image once with `espflash write-bin 0x0`.

## Versions and releases

Versions live in `Cargo.toml`. Launcher, recovery and SDK share the workspace version in the root
[Cargo.toml](Cargo.toml); each game has its own version in its crate's Cargo.toml. Builds of the
same version still differ by their commit.

To publish a release, raise the workspace version, commit, and push a tag with that version:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

CI then builds and checks everything and publishes a GitHub release "ogo 0.2.0" with the full flash
image, `launcher.bin`, `recovery.bin` and the `.ogo` package of every game. A tag that doesn't match
the workspace version fails before anything is published.

## Project layout

| Path | Contents |
|---|---|
| [ogo-board/](ogo-board/) | Board support (display, buttons, speaker, battery, LED, SD card), flash writing, shared settings and look (`ui`), the image description and the launcher/game boot contract |
| [ogo-launcher/](ogo-launcher/) | Launcher firmware: game list, installer, WiFi and web page, settings screen, boot screen, hardware test screen |
| [ogo-recovery/](ogo-recovery/) | Recovery app in the factory slot |
| [ogo-sdk/](ogo-sdk/) | Game SDK: frame loop, input, sound, saves, pause menu, return to the launcher |
| [ogo-format/](ogo-format/) | File formats and pure logic shared by the firmware and xtask, with unit tests that run on the PC |
| [ogo-gb/](ogo-gb/) | Game Boy emulator core (`no_std`): CPU, memory map, cartridges, timer, LCD timing and drawing; tested on the PC |
| [games/snake/](games/snake/) | Snake, built on `ogo-sdk` |
| [games/brickfall/](games/brickfall/) | Brickfall, a falling-bricks puzzle built on `ogo-sdk` |
| [games/smash/](games/smash/) | Smash, a brick-breaker built on `ogo-sdk` |
| [xtask/](xtask/) | Build tasks (`cargo xtask pack / release / flash / test / samples / icon`), run on the PC |
| [docs/](docs/) | Architecture, contracts, game guide, hardware notes |
| [.github/workflows/ci.yml](.github/workflows/ci.yml) | CI and releases |
| [partitions.csv](partitions.csv) | Flash layout |
| [rust-toolchain.toml](rust-toolchain.toml) | The pinned Xtensa toolchain |
| [.cargo/config.toml](.cargo/config.toml) | Target, linker, ESP-IDF version, Python version for the build, the xtask alias |
| [sdkconfig.defaults](sdkconfig.defaults) | ESP-IDF settings: flash, PSRAM, bootloader (MENU for recovery, rollback), scheduler, FAT |
| [tools/make_logo.py](tools/make_logo.py) | Draws the boot logo (mascot and wordmark) |
