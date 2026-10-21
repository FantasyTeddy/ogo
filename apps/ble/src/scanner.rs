//! Scans for BLE advertisements on a thread of its own and keeps what each device said last.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use esp32_nimble::{BLEAdvertisedData, BLEAdvertisedDevice, BLEDevice, BLEScan};
use esp_idf_svc::hal::task::block_on;

use crate::advert;

/// How long one scan runs before the next starts; NimBLE scans for a limited time.
const SCAN_MS: i32 = 10_000;
/// The scan thread's stack: it only waits for scans, the callbacks run on NimBLE's host task.
const STACK_BYTES: usize = 8 * 1024;
/// Devices not heard for this long are dropped.
pub const FORGET_AFTER: Duration = Duration::from_secs(60);
/// How much a new packet's signal strength counts in the shown one: single packets vary by
/// several dB, so it's averaged over the last few.
const SIGNAL_WEIGHT: f32 = 0.25;

/// What a device advertised. Name, maker and description often come in different packets
/// (advertisement and scan response), so each keeps its last known value.
#[derive(Debug, Clone)]
pub struct Device {
    /// E.g. "c4:7c:8d:6a:12:3b".
    pub address: String,
    pub name: Option<String>,
    /// What it is, from the kind of advertisement, e.g. "Apple: AirPlay".
    pub description: Option<&'static str>,
    /// Bluetooth SIG company identifier from the manufacturer data.
    pub company: Option<u16>,
    /// Signal strength in dBm, averaged over the last packets.
    pub signal: f32,
    pub first_seen: Instant,
    pub last_seen: Instant,
}

impl Device {
    /// The signal strength as shown, in whole dBm.
    pub fn rssi(&self) -> i32 {
        self.signal.round() as i32
    }
}

/// The devices heard so far, filled by the scan thread.
pub struct Scanner {
    devices: Arc<Mutex<HashMap<[u8; 6], Device>>>,
}

impl Scanner {
    /// Starts Bluetooth and scans until the app ends.
    pub fn start() -> anyhow::Result<Self> {
        let devices = Arc::new(Mutex::new(HashMap::new()));
        let found = Arc::clone(&devices);
        thread::Builder::new()
            .name("ble-scan".into())
            .stack_size(STACK_BYTES)
            .spawn(move || scan_forever(&found))?;
        Ok(Self { devices })
    }

    /// The devices heard in the last [`FORGET_AFTER`], in no particular order.
    pub fn devices(&self) -> Vec<Device> {
        let mut devices = self.devices.lock().unwrap();
        devices.retain(|_, device| device.last_seen.elapsed() < FORGET_AFTER);
        devices.values().cloned().collect()
    }
}

fn scan_forever(devices: &Mutex<HashMap<[u8; 6], Device>>) {
    let ble = BLEDevice::take();
    let mut scan = BLEScan::new();
    // Active: ask for scan responses, which often carry the name. Every packet, not only the
    // first per device, so the signal strength stays current.
    scan.active_scan(true)
        .filter_duplicates(false)
        .interval(100)
        .window(90);
    loop {
        let result = block_on(scan.start(ble, SCAN_MS, |device, data| {
            heard(devices, device, data);
            None::<()>
        }));
        if let Err(e) = result {
            log::warn!("Scan failed: {e:?}");
            thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Runs on NimBLE's host task for every packet, so it does little: no logging.
fn heard(
    devices: &Mutex<HashMap<[u8; 6], Device>>,
    device: &BLEAdvertisedDevice,
    data: BLEAdvertisedData<&[u8]>,
) {
    let address = device.addr();
    let rssi = f32::from(device.rssi());
    // Some devices send a name of only zero bytes or spaces
    let name = data
        .name()
        .map(|name| name.to_string())
        .map(|name| {
            name.trim_matches(|c: char| c.is_control() || c.is_whitespace())
                .to_owned()
        })
        .filter(|name| !name.is_empty());
    let description = advert::describe(&data);
    let company = data.manufacture_data().map(|m| m.company_identifier);

    let now = Instant::now();
    let mut devices = devices.lock().unwrap();
    let entry = devices
        .entry(address.as_le_bytes())
        .or_insert_with(|| Device {
            address: address.to_string(),
            name: None,
            description: None,
            company: None,
            signal: rssi,
            first_seen: now,
            last_seen: now,
        });
    entry.signal += (rssi - entry.signal) * SIGNAL_WEIGHT;
    entry.last_seen = now;
    if name.is_some() {
        entry.name = name;
    }
    if description.is_some() {
        entry.description = description;
    }
    if company.is_some() {
        entry.company = company;
    }
}
