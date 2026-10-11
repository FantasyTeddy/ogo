//! GPIO assignments, from the official library's `src/utility/Config.h`.
//!
//! The drivers take the typed pins from `Peripherals`; these constants are for
//! documentation and logging.

pub const SPI_SCLK: u8 = 18;
pub const SPI_MOSI: u8 = 23;
pub const SPI_MISO: u8 = 19;

pub const LCD_CS: u8 = 5;
pub const LCD_DC: u8 = 21;
pub const LCD_BACKLIGHT: u8 = 14;

pub const SD_CS: u8 = 22;

pub const BUTTON_A: u8 = 32;
pub const BUTTON_B: u8 = 33;
pub const BUTTON_SELECT: u8 = 27;
pub const BUTTON_START: u8 = 39;
pub const BUTTON_MENU: u8 = 13;
/// Strapping pin: holding Volume at reset enters serial download mode.
pub const BUTTON_VOLUME: u8 = 0;
/// D-pad left/right, analog (ADC1 channel 6).
pub const DPAD_X: u8 = 34;
/// D-pad up/down, analog (ADC1 channel 7).
pub const DPAD_Y: u8 = 35;

/// Battery voltage through a 1:2 divider (ADC1 channel 0).
pub const BATTERY: u8 = 36;

pub const LED: u8 = 2;

/// Amplifier enable, high = on (confirmed by test; Go-Play drives it low to mute the amplifier).
pub const SPEAKER_ENABLE: u8 = 25;
/// Speaker signal, DAC channel 1.
pub const SPEAKER_OUT: u8 = 26;
