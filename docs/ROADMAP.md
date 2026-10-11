# Roadmap

## Milestones

| # | Milestone | Done when |
|---|---|---|
| M0 | Toolchain & hello world | A Rust app built on Windows draws text on the LCD and logs over serial. **Done 2026-10-11** |
| M1 | `ogo-board` crate | Display, all buttons, speaker beep, SD card (shared bus with LCD), battery reading all work. **Done 2026-10-11** (LED not visually confirmed) |
| M2 | Launcher UI (offline) | Scrollable game list read from `/ogo/games/*/game.toml`, with icons and battery. **Done 2026-10-11** |
| M3 | Install & launch | Picking a game flashes `game_0` (with progress + hash skip) and boots it. **Done 2026-10-11** |
| M4 | `ogo-sdk` + first game | `snake` built on the SDK. The boot-back trick and `exit_to_launcher()` work, and saves persist. **Done 2026-10-11** |
| M5 | Recovery | Partition table final, MENU-at-boot enters `ogo-recovery`, which can boot the launcher. **Done 2026-10-11** |
| M6 | WiFi + web UI | Upload / delete games and view status from the browser. PIN protection. **Done 2026-10-11** |
| M7 | Launcher self-OTA | Upload `launcher.bin` via the web page. A/B switch with rollback, tested with a deliberately broken build. **Done 2026-10-11** |
| M8 | Settings screen | MENU opens settings with device info, brightness, volume and "forget WiFi network". Brightness and volume are stored in NVS and apply in games too, and VOL in games is saved. **Done 2026-10-11** |
| M9 | Build tooling | `cargo xtask pack / release / flash` replace the scripts, and every image carries the git commit. **Done 2026-10-11** |
| M10 | Polish | Save backup download, mDNS (`ogo.local`), charging indicator |
| M11 | Emulators | First emulator packaged as a "game" that browses ROMs on the SD card |

M0 to M4 made a usable device without networking; M6 and M7 delivered updates over WiFi.

## Open questions and risks

- **Emulators (M11).** Either port individual C emulators (e.g. gnuboy, nofrendo) behind the SDK, or
  find out whether a Retro-Go build can be packaged as a single image (Retro-Go normally uses its own
  multi-partition layout, so it probably won't drop in as-is). Either way the SDK needs more than it
  has now: PCM audio, scaled or partial screen updates, and file access for ROMs.
- **Number of game slots.** One slot (~8.9 MB) is simplest. Two slots would save re-flashing when
  alternating between games, but each would hold only ~4.5 MB, and changing the partition table
  means flashing over USB. Decide after measuring emulator image sizes.
- **Charging indicator (M10).** The board has no charge-status signal (see
  [HARDWARE.md](HARDWARE.md#findings)). A heuristic can show a charging icon when the voltage jumps
  by ~0.1–0.2 V or rises steadily, and hide it on a sudden drop. It can't tell at power-on while
  already plugged in until the voltage has risen for a while, and the percentage reads high while
  charging.
- **mDNS (M10).** The ESP-IDF `mdns` component; check how much it adds to the launcher (1.6 MB of a
  2.5 MB slot today).
- **Shared SPI bus.** Works for the launcher and Snake. Recheck when a game streams full frames by
  DMA while reading files (e.g. an emulator loading ROM data).
- **WiFi always on.** WiFi only runs while the WiFi screen is open. "Always on" could become a
  setting; joining currently blocks the UI for up to 15 s.
- **Compatibility.** Games depend on the partition table, the boot contract and NVS keys; see
  [CONTRACTS.md](CONTRACTS.md#compatibility) for when to bump `sdk_version`.
