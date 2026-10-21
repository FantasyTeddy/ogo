# Hardware

The ODROID-GO as ogo uses it: pins, constraints that shape the design, and what was measured on the
device. The drivers are in [ogo-board](../ogo-board/src/).

## Overview

| Item | ODROID-GO | Consequence |
|---|---|---|
| SoC | ESP32 (dual Xtensa LX6, 240 MHz), revision v1.0 | Needs the Xtensa Rust toolchain from esp-rs (`espup`), and `CONFIG_SPIRAM_CACHE_WORKAROUND` |
| Flash | 16 MB | Room for recovery, two launcher slots and one app slot (see [CONTRACTS.md](CONTRACTS.md#partition-table)) |
| PSRAM | 4 MB | **Data only.** The ESP32 can't run code from PSRAM or the SD card, so apps run from flash |
| Display | 2.4" ILI9341, 320×240, SPI | `mipidsi` + `embedded-graphics` |
| SD card | microSD, **on the same SPI bus as the LCD** | One SPI driver with two devices (separate chip selects) |
| Input | D-pad (two analog axes), A, B, SELECT, START, MENU, VOL | MENU is the recovery button at power-on. VOL is on GPIO0, a strapping pin: holding it at reset enters serial download mode |
| Audio | Speaker on DAC GPIO26 through an amplifier, enabled by GPIO25 | 8-bit samples through the DAC; the amplifier needs time to wake up |
| Battery | Li-Po, voltage through a 1:2 divider on an ADC pin | Rough percentage only; no charge-status signal |

Every ESP32 app slot is mapped through the MMU to the same virtual addresses. **So one app image
runs from any app slot without being rebuilt.** That is what makes "copy from the SD card, then
boot" work.

## Pins

Checked against the official library's
[`src/utility/Config.h`](https://github.com/hardkernel/ODROID-GO/blob/master/src/utility/Config.h);
all are in [pins.rs](../ogo-board/src/pins.rs).

| Function | GPIO |
|---|---|
| SPI CLK / MOSI / MISO | 18 / 23 / 19 |
| LCD CS / DC / backlight | 5 / 21 / 14 (no reset pin) |
| SD CS | 22 |
| A / B / SELECT / START / MENU / VOL | 32 / 33 / 27 / 39 / 13 / 0 |
| D-pad X / Y (analog) | 34 / 35 |
| Battery sense (analog) | 36 (not in `Config.h`, confirmed on the device) |
| LED | 2 |
| Speaker enable / signal | 25 / 26 |

## Findings

**Display**
- Use **SPI3 (VSPI)**. Pins 18/23/19/5 are its IOMUX pins, which allows 40 MHz. Through SPI2 the
  signals go via the GPIO matrix, and ESP-IDF rejects anything above ~26.7 MHz.
- Landscape 320×240: rotate 90° **and** flip horizontally (the panel is mounted mirrored). Color
  order is BGR.
- A full frame takes about 45 ms at 40 MHz (about 4 MB/s, measured), so ~22 fps is the ceiling for
  full redraws.
- Pixel data is sent as queued transfers from two buffers (`display::DisplaySpi`), so filling one
  overlaps with sending the other; filling them pixel by pixel through mipidsi's iterator took as
  long as the transfer itself.
- The panel keeps whatever the previous app showed, so it's cleared before the backlight turns on.
- **Backlight:** LEDC PWM at 5 kHz with 13 bits on GPIO14 (high = on). The duty cycle grows
  quadratically with the brightness setting, since perceived brightness is roughly the square root of
  the duty cycle. The lowest step is a 4 % duty cycle, which is dim but readable.

**Shared SPI bus**
- One `SpiDriver` (with DMA) is shared by the LCD device and the SD host. ESP-IDF's SPI master driver
  serializes transactions per bus, so no extra locking is needed.
- Keep SD CS (22) high while only the LCD is in use, e.g. during display setup.

**SD card**
- FAT with long filenames needs `CONFIG_FATFS_LFN_HEAP`; the default is 8.3 names only.
- FAT can't rename over an existing file: delete first, then rename.

**Buttons**
- The D-pad axes read ~0 at rest, 4095 for left/up and ~1880 for right/down (12-bit, 12 dB
  attenuation). Thresholds 1024 and 3072, the same as Retro-Go.

**Battery**
- 1:2 divider. Measured 3.72 V on battery and 3.87 V right after plugging in. The percentage is
  linear between 3.5 and 4.2 V, as in Retro-Go, so it's only a rough guide.
- There is no charge-status or USB-power signal (Retro-Go's board config only has the battery ADC
  too). Plugging in raises the voltage by ~0.1–0.2 V at once, so charging can only be guessed from
  such jumps or a steady rise.

**Speaker**
- GPIO25 enables the amplifier (high = on), GPIO26 carries the signal on DAC channel 1. The official
  library's name `SPEAKER_ENABLE_PIN` is right; driving both pins differentially (as Retro-Go's
  config suggests) only works while GPIO25 stays above the amplifier's enable threshold.
- The DAC stays in DMA mode (32 kHz, 8-bit) from boot on, and an `audio` thread keeps feeding it:
  queued sounds, otherwise the mid level. Every switch between DAC modes, or leaving the input
  undriven, made the amplifier click or crackle. The DAC's hardware cosine generator was tried and
  dropped: it always starts at a peak, so every tone clicked. Notes are sines with 5 ms fades.
- The amplifier hisses faintly while on, so the audio thread switches it off after 3 s of silence.
  It needs tens of milliseconds to wake up, so a sound that finds it off starts after 150 ms of idle
  level.
- Writing flash stalls the audio DMA, which clicks. Code that writes flash (installing an app,
  saving settings) mutes the amplifier around the write.

**LED**
- GPIO2 is driven by software, but no LED was visible on the test unit.
