//! Scans for BLE advertisements on a thread of its own and keeps what each device said last.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use esp32_nimble::{
    enums::AdvType, utilities::BleUuid, BLEAddressType, BLEAdvertisedData, BLEAdvertisedDevice,
    BLEDevice, BLEScan,
};
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
/// Seconds of signal history kept per device, for the graph.
pub const HISTORY_SECONDS: usize = 60;

/// What a device advertised. Name, maker and description often come in different packets
/// (advertisement and scan response), so each keeps its last known value.
#[derive(Debug, Clone)]
pub struct Device {
    /// E.g. "c4:7c:8d:6a:12:3b".
    pub address: String,
    /// E.g. "public" or "random, changes".
    pub address_kind: &'static str,
    pub name: Option<String>,
    /// What it is, from a published kind of advertisement, e.g. "iBeacon".
    pub description: Option<&'static str>,
    /// Whether it accepts connections, from its last advertisement (scan responses don't say).
    pub connectable: Option<bool>,
    /// The transmit power it says it uses, in dBm.
    pub tx_power: Option<i8>,
    /// The services it advertises, from service UUIDs and service data.
    pub services: Vec<BleUuid>,
    /// Company identifier and the rest of the manufacturer data.
    pub manufacturer: Option<(u16, Vec<u8>)>,
    pub service_data: Option<(BleUuid, Vec<u8>)>,
    /// Signal strength in dBm, averaged over the last packets.
    pub signal: f32,
    /// The last whole seconds, oldest first, up to [`HISTORY_SECONDS`].
    pub history: VecDeque<Second>,
    pub first_seen: Instant,
    pub last_seen: Instant,
    /// Packets in the second that's still running.
    packets: u16,
    second_started: Instant,
}

/// One second of a device's history.
#[derive(Debug, Clone, Copy)]
pub struct Second {
    /// The averaged signal at its end; `None` without a packet in it.
    pub signal: Option<i8>,
    pub packets: u16,
}

/// Seconds the packet rate is averaged over: many devices send only every second or two.
const RATE_SECONDS: usize = 10;

impl Device {
    /// The signal strength as shown, in whole dBm.
    pub fn rssi(&self) -> i32 {
        self.signal.round() as i32
    }

    /// Packets per second, over the last [`RATE_SECONDS`] (fewer for a device heard only since).
    pub fn packets_per_second(&self) -> f32 {
        let seconds = self.history.len().min(RATE_SECONDS);
        if seconds == 0 {
            return f32::from(self.packets);
        }
        let packets: u32 = self
            .history
            .iter()
            .rev()
            .take(seconds)
            .map(|second| u32::from(second.packets))
            .sum();
        packets as f32 / seconds as f32
    }

    /// The Bluetooth SIG company identifier from the manufacturer data.
    pub fn company(&self) -> Option<u16> {
        self.manufacturer.as_ref().map(|(id, _)| *id)
    }

    /// What to call it: its name, else what it is; `None` if neither is known (the maker is
    /// shown next to it anyway).
    pub fn label(&self) -> Option<String> {
        self.name
            .clone()
            .or_else(|| self.description.map(str::to_owned))
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
    let connectable = match device.adv_type() {
        AdvType::Ind | AdvType::DirectInd => Some(true),
        AdvType::ScanInd | AdvType::NonconnInd => Some(false),
        _ => None,
    };
    let services: Vec<BleUuid> = data
        .service_uuids()
        .chain(data.service_data().map(|service| service.uuid))
        .collect();
    let manufacturer = data.manufacture_data();
    let service_data = data.service_data();

    let now = Instant::now();
    let mut devices = devices.lock().unwrap();
    let entry = devices
        .entry(address.as_le_bytes())
        .or_insert_with(|| Device {
            address: address.to_string(),
            address_kind: address_kind(address.addr_type(), address.as_be_bytes()[0]),
            name: None,
            description: None,
            connectable: None,
            tx_power: None,
            services: Vec::new(),
            manufacturer: None,
            service_data: None,
            signal: rssi,
            history: VecDeque::with_capacity(HISTORY_SECONDS),
            first_seen: now,
            last_seen: now,
            packets: 0,
            second_started: now,
        });

    // Seconds that ended since the last packet: the one it was in, then empty ones
    let seconds = now.duration_since(entry.second_started).as_secs() as u32;
    if seconds >= 1 {
        let finished = Second {
            signal: Some(entry.signal.round() as i8),
            packets: entry.packets,
        };
        let empty = Second {
            signal: None,
            packets: 0,
        };
        let count = (seconds as usize).min(HISTORY_SECONDS);
        for second in std::iter::once(finished).chain(std::iter::repeat_n(empty, count - 1)) {
            if entry.history.len() == HISTORY_SECONDS {
                entry.history.pop_front();
            }
            entry.history.push_back(second);
        }
        entry.packets = 0;
        entry.second_started += Duration::from_secs(u64::from(seconds));
    }
    entry.packets = entry.packets.saturating_add(1);
    entry.signal += (rssi - entry.signal) * SIGNAL_WEIGHT;
    entry.last_seen = now;

    if name.is_some() {
        entry.name = name;
    }
    if description.is_some() {
        entry.description = description;
    }
    if connectable.is_some() {
        entry.connectable = connectable;
    }
    if let Some(power) = data.tx_power() {
        entry.tx_power = Some(power as i8);
    }
    for uuid in services {
        if !entry.services.contains(&uuid) {
            entry.services.push(uuid);
        }
    }
    // Copied only when changed: most packets repeat the last one
    if let Some(m) = manufacturer {
        if entry.manufacturer.as_ref().is_none_or(|(id, payload)| {
            *id != m.company_identifier || payload.as_slice() != m.payload
        }) {
            entry.manufacturer = Some((m.company_identifier, m.payload.to_vec()));
        }
    }
    if let Some(s) = service_data {
        if entry
            .service_data
            .as_ref()
            .is_none_or(|(uuid, bytes)| *uuid != s.uuid || bytes.as_slice() != s.service_data)
        {
            entry.service_data = Some((s.uuid, s.service_data.to_vec()));
        }
    }
}

/// What kind of address it is. Random ones say in their top two bits whether they stay
/// (static) or change every few minutes, so the device can't be followed (private).
fn address_kind(kind: BLEAddressType, top_byte: u8) -> &'static str {
    match kind {
        BLEAddressType::Public | BLEAddressType::PublicID => "public",
        BLEAddressType::Random | BLEAddressType::RandomID => match top_byte >> 6 {
            0b11 => "random, static",
            0b01 => "random, changes",
            _ => "random, one-off",
        },
    }
}
