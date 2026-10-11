//! ogo's file formats, contract values and pure helpers: everything that doesn't need the ESP32.
//!
//! The firmware crates and xtask share these, and the tests run on the PC: `cargo xtask test`.
//! The formats themselves are described in docs/CONTRACTS.md.

pub mod battery;
pub mod game;
pub mod image;
pub mod package;
pub mod settings;
pub mod text;
