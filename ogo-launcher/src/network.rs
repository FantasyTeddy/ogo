//! WiFi: scans for networks, then joins the configured one, or opens an access point `ogo-XXXX`
//! (no password) if there is none or joining fails, so the web page can still be reached to set
//! one. The access point runs together with the station interface, so scanning keeps working.

use std::net::Ipv4Addr;

use anyhow::Context;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    nvs::EspDefaultNvsPartition,
    sys,
    wifi::{AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use serde::Serialize;

use crate::settings::WifiCredentials;

const AP_CHANNEL: u8 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Joined `ssid`.
    Station { ssid: String, ip: Ipv4Addr },
    /// Our own access point; `failed` says why the configured network wasn't joined, if there was one.
    AccessPoint { ssid: String, ip: Ipv4Addr, failed: Option<String> },
}

impl Mode {
    pub fn ip(&self) -> Ipv4Addr {
        match self {
            Mode::Station { ip, .. } | Mode::AccessPoint { ip, .. } => *ip,
        }
    }
}

/// A network found by a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScannedNetwork {
    pub ssid: String,
    /// Signal strength in dBm, e.g. -55 (good) to -90 (weak).
    pub rssi: i8,
    pub secured: bool,
}

pub struct Network {
    wifi: BlockingWifi<EspWifi<'static>>,
}

impl Network {
    pub fn new(modem: Modem<'static>, nvs: EspDefaultNvsPartition) -> anyhow::Result<Self> {
        let event_loop = EspSystemEventLoop::take()?;
        let wifi = EspWifi::new(modem, event_loop.clone(), Some(nvs))?;
        Ok(Self { wifi: BlockingWifi::wrap(wifi, event_loop)? })
    }

    /// Scans, then starts WiFi. Blocks while joining (up to ~15 s if the network doesn't answer).
    pub fn start(&mut self, credentials: Option<&WifiCredentials>) -> anyhow::Result<(Mode, Vec<ScannedNetwork>)> {
        self.wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
        self.wifi.start()?;
        let networks = self.scan().unwrap_or_else(|e| {
            log::warn!("Scanning failed: {e:?}");
            Vec::new()
        });

        let failed = match credentials {
            Some(credentials) => match self.join(credentials) {
                Ok(ip) => return Ok((Mode::Station { ssid: credentials.ssid.clone(), ip }, networks)),
                Err(e) => {
                    log::warn!("Joining {} failed: {e:?}", credentials.ssid);
                    Some(format!("Couldn't join {}", credentials.ssid))
                }
            },
            None => None,
        };
        let (ssid, ip) = self.open_access_point()?;
        Ok((Mode::AccessPoint { ssid, ip, failed }, networks))
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        if self.wifi.is_started()? {
            self.wifi.stop()?;
        }
        Ok(())
    }

    /// Networks in range, strongest first, one entry per name. Takes about 2 s; while scanning in
    /// access point mode, the access point drops out briefly.
    pub fn scan(&mut self) -> anyhow::Result<Vec<ScannedNetwork>> {
        let mut networks: Vec<ScannedNetwork> = Vec::new();
        for found in self.wifi.scan().context("scan failed")? {
            let ssid = found.ssid.as_str();
            if ssid.is_empty() || networks.iter().any(|n| n.ssid == ssid) {
                continue;
            }
            networks.push(ScannedNetwork {
                ssid: ssid.to_owned(),
                rssi: found.signal_strength,
                secured: found.auth_method.is_some_and(|auth| auth != AuthMethod::None),
            });
        }
        networks.sort_by_key(|n| std::cmp::Reverse(n.rssi));
        log::info!("Found {} networks", networks.len());
        Ok(networks)
    }

    fn join(&mut self, credentials: &WifiCredentials) -> anyhow::Result<Ipv4Addr> {
        let auth_method = if credentials.password.is_empty() { AuthMethod::None } else { AuthMethod::WPA2Personal };
        self.wifi.set_configuration(&Configuration::Client(client_config(credentials, auth_method)?))?;
        log::info!("Joining {}", credentials.ssid);
        self.wifi.connect().context("can't connect")?;
        self.wifi.wait_netif_up().context("no IP address")?;
        let ip = self.wifi.wifi().sta_netif().get_ip_info()?.ip;
        log::info!("Joined {}, IP {ip}", credentials.ssid);
        Ok(ip)
    }

    fn open_access_point(&mut self) -> anyhow::Result<(String, Ipv4Addr)> {
        let ssid = access_point_name();
        self.wifi.stop()?;
        self.wifi.set_configuration(&Configuration::Mixed(
            ClientConfiguration::default(),
            AccessPointConfiguration {
                ssid: ssid.as_str().try_into().map_err(|_| anyhow::anyhow!("SSID too long"))?,
                auth_method: AuthMethod::None,
                channel: AP_CHANNEL,
                ..Default::default()
            },
        ))?;
        self.wifi.start()?;
        // Not wait_netif_up(): in mixed mode it would also wait for the unconnected station side.
        // The access point's interface is up as soon as WiFi has started.
        let ip = self.wifi.wifi().ap_netif().get_ip_info()?.ip;
        log::info!("Access point {ssid} open, IP {ip}");
        Ok((ssid, ip))
    }
}

fn client_config(credentials: &WifiCredentials, auth_method: AuthMethod) -> anyhow::Result<ClientConfiguration> {
    Ok(ClientConfiguration {
        ssid: credentials.ssid.as_str().try_into().map_err(|_| anyhow::anyhow!("SSID too long"))?,
        password: credentials.password.as_str().try_into().map_err(|_| anyhow::anyhow!("password too long"))?,
        auth_method,
        ..Default::default()
    })
}

/// "ogo-" and the last two bytes of the access point's MAC address, e.g. "ogo-50A1".
fn access_point_name() -> String {
    let mut mac = [0u8; 6];
    unsafe { sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_SOFTAP) };
    format!("ogo-{:02X}{:02X}", mac[4], mac[5])
}
