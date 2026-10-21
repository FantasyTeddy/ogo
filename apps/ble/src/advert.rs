//! What a device is, from what it advertises, for devices that send no name: most phones, watches
//! and trackers don't, for privacy, but their manufacturer data or service UUIDs say what kind of
//! message it is.

use esp32_nimble::{utilities::BleUuid, BLEAdvertisedData};

const APPLE: u16 = 0x004C;
const MICROSOFT: u16 = 0x0006;

/// A short description, e.g. "Apple: AirPlay", if the advertisement is a known kind.
pub fn describe(data: &BLEAdvertisedData<&[u8]>) -> Option<&'static str> {
    if let Some(mfg) = data.manufacture_data() {
        // Apple's "Continuity" messages and Microsoft's beacons start with a type byte
        let kind = mfg.payload.first().copied();
        let described = match (mfg.company_identifier, kind) {
            (APPLE, Some(0x02)) => Some("iBeacon"),
            (APPLE, Some(0x05)) => Some("Apple: AirDrop"),
            (APPLE, Some(0x07)) => Some("Apple: AirPods"),
            (APPLE, Some(0x09)) => Some("Apple: AirPlay"),
            (APPLE, Some(0x0C)) => Some("Apple: Handoff"),
            (APPLE, Some(0x0F)) => Some("Apple: Nearby"),
            (APPLE, Some(0x10)) => Some("Apple device"),
            (APPLE, Some(0x12)) => Some("Apple: Find My"),
            (MICROSOFT, Some(0x01)) => Some("Windows device"),
            (MICROSOFT, Some(0x03)) => Some("Swift Pair"),
            _ => None,
        };
        if described.is_some() {
            return described;
        }
    }
    data.service_uuids()
        .chain(data.service_data().map(|service| service.uuid))
        .find_map(|uuid| match uuid {
            BleUuid::Uuid16(0xFE2C) => Some("Google: Fast Pair"),
            BleUuid::Uuid16(0xFCF1 | 0xFEF3 | 0xFE9F) => Some("Google"),
            BleUuid::Uuid16(0xFEAA) => Some("Eddystone"),
            BleUuid::Uuid16(0xFD6F) => Some("Exposure notif."),
            BleUuid::Uuid16(0xFEED) => Some("Tile"),
            BleUuid::Uuid16(0xFD5A) => Some("Samsung: SmartTag"),
            _ => None,
        })
}
