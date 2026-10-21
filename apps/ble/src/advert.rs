//! What a device's advertisements say, from published formats only: iBeacon (Apple's Proximity
//! Beacon Specification), Microsoft's Connected Devices Platform beacon ([MS-CDP] 2.2.2.2.3),
//! Google Fast Pair, Eddystone and Exposure Notification, and the owners of the service UUIDs the
//! Bluetooth SIG assigned to its members. Formats known only from reverse engineering, such as
//! Apple's other messages, are left out on purpose.

use esp32_nimble::{utilities::BleUuid, BLEAdvertisedData};

const APPLE: u16 = 0x004C;
const MICROSOFT: u16 = 0x0006;
const FAST_PAIR: u16 = 0xFE2C;
const EDDYSTONE: u16 = 0xFEAA;
const EXPOSURE_NOTIFICATION: u16 = 0xFD6F;

/// A short description for the list, e.g. "iBeacon" or "Windows laptop", if the advertisement is
/// of a published kind or names a service UUID's owner.
pub fn describe(data: &BLEAdvertisedData<&[u8]>) -> Option<&'static str> {
    if let Some(mfg) = data.manufacture_data() {
        match (mfg.company_identifier, mfg.payload) {
            (APPLE, [0x02, 0x15, ..]) => return Some("iBeacon"),
            (MICROSOFT, [0x01, version_and_type, ..]) => {
                return Some(cdp_device_type(*version_and_type).unwrap_or("Microsoft beacon"))
            }
            _ => {}
        }
    }
    data.service_uuids()
        .chain(data.service_data().map(|service| service.uuid))
        .find_map(|uuid| match uuid {
            BleUuid::Uuid16(FAST_PAIR) => Some("Fast Pair"),
            BleUuid::Uuid16(EDDYSTONE) => Some("Eddystone"),
            BleUuid::Uuid16(EXPOSURE_NOTIFICATION) => Some("Exposure Notification"),
            BleUuid::Uuid16(uuid) => member_uuid_owner(uuid),
            _ => None,
        })
}

/// The company the Bluetooth SIG assigned a member service UUID to, for those seen often.
pub fn member_uuid_owner(uuid: u16) -> Option<&'static str> {
    Some(match uuid {
        0xFCF1 | 0xFE9F | 0xFEF3 => "Google",
        0xFD5A => "Samsung",
        0xFEED => "Tile",
        _ => return None,
    })
}

/// What the manufacturer data and service data say, one entry per message, for the published
/// formats.
pub fn messages(
    manufacturer: Option<&(u16, Vec<u8>)>,
    service_data: Option<&(BleUuid, Vec<u8>)>,
) -> Vec<String> {
    let mut messages = Vec::new();
    match manufacturer.map(|(id, payload)| (*id, payload.as_slice())) {
        // iBeacon: type 0x02, length 0x15, UUID (16), major (2), minor (2), measured power (1)
        Some((APPLE, [0x02, 0x15, _uuid @ .., major_hi, major_lo, minor_hi, minor_lo, _power])) => {
            let major = u16::from_be_bytes([*major_hi, *major_lo]);
            let minor = u16::from_be_bytes([*minor_hi, *minor_lo]);
            messages.push(format!("iBeacon, major {major}, minor {minor}"));
        }
        Some((MICROSOFT, [0x01, version_and_type, ..])) => {
            messages.push(match cdp_device_type(*version_and_type) {
                Some(kind) => format!("Microsoft beacon: {kind}"),
                None => "Microsoft beacon".to_owned(),
            });
        }
        _ => {}
    }
    match service_data {
        Some((BleUuid::Uuid16(FAST_PAIR), data)) => messages.push(fast_pair(data)),
        Some((BleUuid::Uuid16(EDDYSTONE), data)) => messages.extend(eddystone(data)),
        Some((BleUuid::Uuid16(EXPOSURE_NOTIFICATION), _)) => {
            messages.push("Exposure Notification".to_owned())
        }
        _ => {}
    }
    messages
}

/// [MS-CDP]'s device types, from the low 5 bits of Version_and_Device_Type.
fn cdp_device_type(version_and_type: u8) -> Option<&'static str> {
    Some(match version_and_type & 0x1F {
        1 => "Xbox One",
        6 => "iPhone",
        7 => "iPad",
        8 => "Android device",
        9 => "Windows desktop",
        11 => "Windows phone",
        12 => "Linux device",
        13 => "Windows IoT",
        14 => "Surface Hub",
        15 => "Windows laptop",
        16 => "Windows tablet",
        _ => return None,
    })
}

/// Google Fast Pair: the 3-byte model ID while it's ready to pair, other data once it isn't.
fn fast_pair(data: &[u8]) -> String {
    match data {
        [a, b, c] => format!("Fast Pair, ready to pair (model {a:02X}{b:02X}{c:02X})"),
        _ => "Fast Pair, not pairing".to_owned(),
    }
}

/// Eddystone beacons: the first byte is the frame type; URLs are compressed with prefix and suffix
/// codes.
fn eddystone(data: &[u8]) -> Option<String> {
    match data {
        [0x00, ..] => Some("Eddystone UID".to_owned()),
        [0x10, _power, scheme, encoded @ ..] => {
            let mut url = match scheme {
                0 => "http://www.",
                1 => "https://www.",
                2 => "http://",
                3 => "https://",
                _ => return Some("Eddystone URL".to_owned()),
            }
            .to_owned();
            for &byte in encoded {
                match URL_CODES.get(usize::from(byte)) {
                    Some(code) => url.push_str(code),
                    None if byte.is_ascii_graphic() => url.push(char::from(byte)),
                    None => url.push('?'),
                }
            }
            Some(url)
        }
        [0x20, ..] => Some("Eddystone telemetry".to_owned()),
        [0x30, ..] => Some("Eddystone EID".to_owned()),
        _ => None,
    }
}

/// Eddystone-URL's codes 0x00 to 0x0D.
const URL_CODES: [&str; 14] = [
    ".com/", ".org/", ".edu/", ".net/", ".info/", ".biz/", ".gov/", ".com", ".org", ".edu", ".net",
    ".info", ".biz", ".gov",
];
