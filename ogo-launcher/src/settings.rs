//! WiFi credentials: stored in NVS by the web page, or read from `/ogo/settings.toml` on the SD
//! card for the first setup. NVS wins once it has been set. The settings screen can forget the
//! one in NVS.
//!
//! ```toml
//! # /ogo/settings.toml
//! [wifi]
//! ssid = "MyNetwork"
//! password = "secret"
//! ```

use std::fs;

use esp_idf_svc::nvs::EspDefaultNvs;
use ogo_board::SdCard;
use serde::Deserialize;

const KEY_SSID: &str = "wifi_ssid";
const KEY_PASSWORD: &str = "wifi_pass";
const SETTINGS_FILE: &str = "ogo/settings.toml";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WifiCredentials {
    pub ssid: String,
    #[serde(default)]
    pub password: String,
}

/// Where the saved network comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiSource {
    Nvs,
    /// `/ogo/settings.toml`
    SdCard,
}

#[derive(Deserialize)]
struct SettingsFile {
    wifi: Option<WifiCredentials>,
}

pub fn load_wifi(nvs: &EspDefaultNvs, sd: Option<&SdCard>) -> Option<WifiCredentials> {
    saved_wifi(nvs, sd).map(|(credentials, _)| credentials)
}

/// Like [`load_wifi`], and tells where the network comes from.
pub fn saved_wifi(
    nvs: &EspDefaultNvs,
    sd: Option<&SdCard>,
) -> Option<(WifiCredentials, WifiSource)> {
    let mut ssid = [0u8; 33];
    let mut password = [0u8; 65];
    if let Ok(Some(ssid)) = nvs.get_str(KEY_SSID, &mut ssid) {
        let password = nvs
            .get_str(KEY_PASSWORD, &mut password)
            .ok()
            .flatten()
            .unwrap_or_default();
        let credentials = WifiCredentials {
            ssid: ssid.to_owned(),
            password: password.to_owned(),
        };
        return Some((credentials, WifiSource::Nvs));
    }
    let text = fs::read_to_string(sd?.path(SETTINGS_FILE)).ok()?;
    match toml::from_str::<SettingsFile>(&text) {
        Ok(settings) => settings
            .wifi
            .map(|credentials| (credentials, WifiSource::SdCard)),
        Err(e) => {
            log::warn!("Ignoring /{SETTINGS_FILE}: {}", e.message());
            None
        }
    }
}

pub fn save_wifi(nvs: &EspDefaultNvs, credentials: &WifiCredentials) -> anyhow::Result<()> {
    nvs.set_str(KEY_SSID, &credentials.ssid)?;
    nvs.set_str(KEY_PASSWORD, &credentials.password)?;
    Ok(())
}

/// Removes the network saved in NVS. One in `/ogo/settings.toml` is used again afterwards.
pub fn forget_wifi(nvs: &EspDefaultNvs) -> anyhow::Result<()> {
    nvs.remove(KEY_SSID)?;
    nvs.remove(KEY_PASSWORD)?;
    Ok(())
}
