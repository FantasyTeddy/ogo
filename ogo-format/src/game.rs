//! A game's `game.toml` and icon.

use serde::{Deserialize, Serialize};

/// The version of the contract between launcher and games (boot contract, partition table, NVS
/// keys games rely on). The launcher only starts games with this `sdk_version`; packages get the
/// version of the SDK they were built with. Bump it when a change breaks games built earlier.
pub const SDK_VERSION: u32 = 1;

/// Width and height of `icon.rgb565`.
pub const ICON_SIZE: u32 = 64;
/// `icon.rgb565`: 64x64 pixels, RGB565 little-endian, row by row.
pub const ICON_BYTES: usize = (ICON_SIZE * ICON_SIZE * 2) as usize;

/// Contents of `game.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    pub sdk_version: u32,
    /// SHA-256 of `game.bin`, hex.
    pub sha256: String,
}

impl Manifest {
    /// Parses `game.toml`; the error is a message for the user.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("game.toml: {}", e.message()))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a manifest always serializes")
    }

    /// Checks the fields: a valid id (matching `folder`, if given), the supported SDK version, and
    /// a SHA-256 hex string. The error is a message for the user.
    pub fn check(&self, folder: Option<&str>, supported_sdk_version: u32) -> Result<(), String> {
        if !is_valid_id(&self.id) {
            return Err(format!("invalid id \"{}\" (use a-z, 0-9 and -)", self.id));
        }
        if folder.is_some_and(|folder| self.id != folder) {
            return Err(format!("id \"{}\" doesn't match the folder name", self.id));
        }
        if self.sdk_version != supported_sdk_version {
            return Err(format!(
                "needs SDK version {}, this launcher supports {supported_sdk_version}",
                self.sdk_version
            ));
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("sha256 is not a SHA-256 hex string".into());
        }
        Ok(())
    }
}

/// Game ids: lowercase letters, digits and `-`. They are also folder names.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One pixel as stored in `icon.rgb565`: RGB565, little-endian.
pub fn rgb565_le([r, g, b]: [u8; 3]) -> [u8; 2] {
    let value = (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
    value.to_le_bytes()
}

/// A top-down BMP with RGB565 bitfields around raw little-endian RGB565 pixels, so browsers can
/// show an icon without converting it.
pub fn icon_bmp(pixels: &[u8], size: u32) -> Vec<u8> {
    const HEADERS: u32 = 14 + 40 + 12;
    let mut bmp = Vec::with_capacity(HEADERS as usize + pixels.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(HEADERS + pixels.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&HEADERS.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(size as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(size as i32)).to_le_bytes()); // negative height: rows top to bottom
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&16u16.to_le_bytes());
    bmp.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    bmp.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 16]);
    for mask in [0xF800u32, 0x07E0, 0x001F] {
        bmp.extend_from_slice(&mask.to_le_bytes());
    }
    bmp.extend_from_slice(pixels);
    bmp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            id: "snake".into(),
            name: "Snake \"classic\"".into(),
            version: "1.0.0".into(),
            author: String::new(),
            sdk_version: 1,
            sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let manifest = manifest();
        assert_eq!(Manifest::parse(&manifest.to_toml()), Ok(manifest));
    }

    #[test]
    fn author_is_optional() {
        let text = "id = \"a\"\nname = \"A\"\nversion = \"1\"\nsdk_version = 1\nsha256 = \"x\"\n";
        assert_eq!(Manifest::parse(text).unwrap().author, "");
    }

    #[test]
    fn parse_errors_name_the_file() {
        let error = Manifest::parse("id = \"a\"\nname = \"Missing quote\n").unwrap_err();
        assert!(error.starts_with("game.toml: "), "{error}");
    }

    #[test]
    fn check_accepts_a_valid_manifest() {
        assert_eq!(manifest().check(Some("snake"), 1), Ok(()));
        assert_eq!(manifest().check(None, 1), Ok(()));
    }

    #[test]
    fn check_rejects_bad_fields() {
        let check = |change: fn(&mut Manifest)| {
            let mut manifest = manifest();
            change(&mut manifest);
            manifest.check(None, 1)
        };
        assert!(check(|m| m.id = "Snake".into()).unwrap_err().contains("invalid id"));
        assert!(check(|m| m.sdk_version = 2).unwrap_err().contains("needs SDK version 2"));
        assert!(check(|m| m.sha256 = "abc".into()).unwrap_err().contains("sha256"));
        assert!(check(|m| m.sha256 = "g".repeat(64)).unwrap_err().contains("sha256"));
        assert!(manifest().check(Some("other"), 1).unwrap_err().contains("folder name"));
    }

    #[test]
    fn ids() {
        assert!(is_valid_id("snake"));
        assert!(is_valid_id("2048"));
        assert!(is_valid_id("long-name"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("Snake"));
        assert!(!is_valid_id("snake_2"));
        assert!(!is_valid_id("../x"));
    }

    #[test]
    fn rgb565_pixels() {
        assert_eq!(rgb565_le([255, 255, 255]), [0xFF, 0xFF]);
        assert_eq!(rgb565_le([255, 0, 0]), [0x00, 0xF8]);
        assert_eq!(rgb565_le([0, 255, 0]), [0xE0, 0x07]);
        assert_eq!(rgb565_le([0, 0, 255]), [0x1F, 0x00]);
        assert_eq!(rgb565_le([7, 3, 7]), [0x00, 0x00]);
    }

    #[test]
    fn bmp_wraps_the_pixels() {
        let pixels = vec![0x12; ICON_BYTES];
        let bmp = icon_bmp(&pixels, ICON_SIZE);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize, bmp.len());
        let offset = u32::from_le_bytes(bmp[10..14].try_into().unwrap()) as usize;
        assert_eq!(&bmp[offset..], &pixels[..]);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -(ICON_SIZE as i32));
    }
}
