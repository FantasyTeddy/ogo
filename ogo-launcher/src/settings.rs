//! WiFi credentials: stored in NVS by the web page, or read from `/ogo/settings.toml` on the SD
//! card for the first setup. NVS wins once it has been set.
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

#[derive(Deserialize)]
struct SettingsFile {
    wifi: Option<WifiCredentials>,
}

pub fn load_wifi(nvs: &EspDefaultNvs, sd: Option<&SdCard>) -> Option<WifiCredentials> {
    let mut ssid = [0u8; 33];
    let mut password = [0u8; 65];
    if let Ok(Some(ssid)) = nvs.get_str(KEY_SSID, &mut ssid) {
        let password = nvs.get_str(KEY_PASSWORD, &mut password).ok().flatten().unwrap_or_default();
        return Some(WifiCredentials { ssid: ssid.to_owned(), password: password.to_owned() });
    }
    let text = fs::read_to_string(sd?.path(SETTINGS_FILE)).ok()?;
    match toml::from_str::<SettingsFile>(&text) {
        Ok(settings) => settings.wifi,
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
