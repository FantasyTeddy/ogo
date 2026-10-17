//! ogo's file formats, contract values and pure helpers: everything that doesn't need the ESP32.
//!
//! The firmware crates and xtask share these, and the tests run on the PC: `cargo xtask test`.
//! The formats themselves are described in docs/CONTRACTS.md.

pub mod battery;
pub mod flash;
pub mod game;
pub mod image;
pub mod package;
pub mod scale;
pub mod settings;
pub mod slots;
pub mod text;
pub mod url;

/// Lowercase hex, e.g. of a SHA-256.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex() {
        assert_eq!(super::hex(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
        assert_eq!(super::hex(&[]), "");
    }
}
