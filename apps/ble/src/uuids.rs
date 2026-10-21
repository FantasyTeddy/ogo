//! Names for 16-bit service UUIDs: the Bluetooth SIG's standard services, published formats, and
//! the owners of member UUIDs seen in advertisements.

use esp32_nimble::utilities::BleUuid;

use crate::advert;

/// The service's name, if it's a known one.
pub fn service_name(uuid: &BleUuid) -> Option<&'static str> {
    let BleUuid::Uuid16(uuid) = uuid else {
        return None;
    };
    Some(match uuid {
        0x1800 => "Generic Access",
        0x1801 => "Generic Attribute",
        0x1802 => "Immediate Alert",
        0x1803 => "Link Loss",
        0x1804 => "Tx Power",
        0x1805 => "Current Time",
        0x1809 => "Health Thermometer",
        0x180A => "Device Information",
        0x180D => "Heart Rate",
        0x180F => "Battery",
        0x1810 => "Blood Pressure",
        0x1812 => "Human Interface Device",
        0x1814 => "Running Speed",
        0x1816 => "Cycling Speed",
        0x1818 => "Cycling Power",
        0x181A => "Environmental Sensing",
        0x181C => "User Data",
        0x181D => "Weight Scale",
        0x1826 => "Fitness Machine",
        0xFD6F => "Exposure Notification",
        0xFE2C => "Fast Pair",
        0xFEAA => "Eddystone",
        uuid => return advert::member_uuid_owner(*uuid),
    })
}
