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
        // LE Audio and newer services
        0x1843 => "Audio Input Control",
        0x1844 => "Volume Control",
        0x1845 => "Volume Offset Control",
        0x1846 => "Coordinated Set Identification",
        0x1847 => "Device Time",
        0x1848 => "Media Control",
        0x1849 => "Generic Media Control",
        0x184A => "Constant Tone Extension",
        0x184B => "Telephone Bearer",
        0x184C => "Generic Telephone Bearer",
        0x184D => "Microphone Control",
        0x184E => "Audio Stream Control",
        0x184F => "Broadcast Audio Scan",
        0x1850 => "Published Audio Capabilities",
        0x1851 => "Basic Audio Announcement",
        0x1852 => "Broadcast Audio Announcement",
        0x1853 => "Common Audio",
        0x1854 => "Hearing Access",
        0x1855 => "Telephony and Media Audio",
        0x1856 => "Public Broadcast Announcement",
        0x1857 => "Electronic Shelf Label",
        0x1858 => "Gaming Audio",
        0x1859 => "Mesh Proxy Solicitation",
        0x185A => "Industrial Measurement Device",
        0x185B => "Ranging",
        0x185C => "HID ISO",
        0x185D => "Cookware",
        0x185E => "Voice Assistant",
        0x185F => "Generic Voice Assistant",
        0x1860 => "Tire Pressure Monitoring System",
        0xFD6F => "Exposure Notification",
        0xFE2C => "Fast Pair",
        0xFEAA => "Eddystone",
        uuid => return advert::member_uuid_owner(*uuid),
    })
}
