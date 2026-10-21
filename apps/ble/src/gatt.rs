//! Connecting to a device and reading what the Bluetooth SIG's standard services say about it:
//! Generic Access (name, appearance), Device Information (maker, model, versions) and Battery.
//! Read only, without pairing: values that need it are reported as such.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use esp32_nimble::{utilities::BleUuid, BLEAddress, BLEClient, BLEDevice, BLEError};
use esp_idf_svc::sys;

use crate::appearance;

/// A device that hasn't answered by then won't: NimBLE would wait 30 s.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Characteristics to read: service, characteristic, label. The serial number is left out on
/// purpose: it identifies the one device.
const VALUES: [(u16, u16, &str); 8] = [
    (0x1800, 0x2A00, "Name"),
    (0x1800, 0x2A01, "Kind"),
    (0x180A, 0x2A29, "Maker"),
    (0x180A, 0x2A24, "Model"),
    (0x180A, 0x2A27, "Hardware"),
    (0x180A, 0x2A26, "Firmware"),
    (0x180A, 0x2A28, "Software"),
    (0x180F, 0x2A19, "Battery"),
];

/// What a device told about itself.
#[derive(Debug, Clone)]
pub struct Info {
    /// Label and value, e.g. ("Model", "MX Master 3"), in the order of [`VALUES`]; those the
    /// device doesn't have are left out.
    pub values: Vec<(&'static str, String)>,
    /// All the services it offers.
    pub services: Vec<BleUuid>,
}

/// What it is doing; shown to the user step by step.
pub type Step = &'static str;

/// Connects to `address`, reads, and disconnects. `step` is told each step as it starts.
pub async fn read(
    ble: &BLEDevice,
    address: BLEAddress,
    mut step: impl FnMut(Step),
) -> Result<Info, String> {
    step("Connecting...");
    let mut client = ble.new_client();
    connect(&mut client, address).await?;

    step("Reading...");
    let result = read_connected(&mut client).await;
    if let Err(e) = client.disconnect() {
        log::warn!("Disconnecting failed: {e:?}");
    }
    result
}

/// `client.connect`, cancelled after [`CONNECT_TIMEOUT`].
async fn connect(client: &mut BLEClient, address: BLEAddress) -> Result<(), String> {
    let connecting = Arc::new(AtomicBool::new(true));
    let watch = Arc::clone(&connecting);
    thread::Builder::new()
        .name("ble-timeout".into())
        .stack_size(3 * 1024)
        .spawn(move || {
            thread::sleep(CONNECT_TIMEOUT);
            if watch.load(Ordering::SeqCst) {
                // Ends the connect with an error; harmless if it has just finished
                unsafe { sys::ble_gap_conn_cancel() };
            }
        })
        .map_err(|e| format!("can't start the timeout: {e}"))?;
    let result = client.connect(&address).await;
    connecting.store(false, Ordering::SeqCst);
    result.map_err(|e| {
        log::info!("Connecting to {address} failed: {e:?}");
        "It didn't answer.".to_owned()
    })
}

async fn read_connected(client: &mut BLEClient) -> Result<Info, String> {
    let services: Vec<BleUuid> = client
        .get_services()
        .await
        .map_err(|e| format!("Listing its services failed: {e}"))?
        .map(|service| service.uuid())
        .collect();
    log::info!("Services: {services:?}");

    let mut values = Vec::new();
    for (service, characteristic, label) in VALUES {
        let service_uuid = BleUuid::from_uuid16(service);
        if !services.contains(&service_uuid) {
            continue;
        }
        let service = match client.get_service(service_uuid).await {
            Ok(service) => service,
            Err(e) => {
                log::info!("{label}: service {service_uuid} not found ({e:?})");
                continue;
            }
        };
        let characteristic_uuid = BleUuid::from_uuid16(characteristic);
        let characteristic = match service.get_characteristic(characteristic_uuid).await {
            Ok(characteristic) => characteristic,
            Err(e) => {
                log::info!("{label}: characteristic {characteristic_uuid} not found ({e:?})");
                continue;
            }
        };
        let value = match characteristic.read_value().await {
            Ok(bytes) => format_value(label, &bytes),
            Err(e) if needs_pairing(e) => Some("needs pairing".to_owned()),
            Err(e) => {
                log::info!("{label}: reading failed ({e:?})");
                Some(format!("can't be read ({e})"))
            }
        };
        if let Some(value) = value {
            values.push((label, value));
        }
    }
    Ok(Info { values, services })
}

/// Battery level in percent, appearance as its category, the rest as text.
fn format_value(label: &str, bytes: &[u8]) -> Option<String> {
    match label {
        "Battery" => bytes.first().map(|percent| format!("{percent} %")),
        "Kind" => {
            let value = u16::from_le_bytes(bytes.get(..2)?.try_into().ok()?);
            Some(appearance::describe(value).unwrap_or_else(|| format!("{value:04X}")))
        }
        _ => {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim_matches(|c: char| c.is_control() || c.is_whitespace());
            (!text.is_empty()).then(|| text.to_owned())
        }
    }
}

/// Whether the device refused because the connection isn't paired (authenticated or encrypted).
fn needs_pairing(error: BLEError) -> bool {
    [
        sys::BLE_ATT_ERR_INSUFFICIENT_AUTHEN,
        sys::BLE_ATT_ERR_INSUFFICIENT_AUTHOR,
        sys::BLE_ATT_ERR_INSUFFICIENT_ENC,
    ]
    .iter()
    .any(|&att| error.code() == sys::BLE_HS_ERR_ATT_BASE + att)
}
