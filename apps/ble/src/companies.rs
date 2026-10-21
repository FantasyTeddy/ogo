//! Names for the Bluetooth SIG company identifiers (Assigned Numbers, company_identifiers.yaml)
//! seen most often in manufacturer data.

/// The company's short name, if it's one of the common ones.
pub fn name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0002 => "Intel",
        0x0006 => "Microsoft",
        0x000F => "Broadcom",
        0x004C => "Apple",
        0x0059 => "Nordic",
        0x0075 => "Samsung",
        0x0087 => "Garmin",
        0x009E => "Bose",
        0x00E0 => "Google",
        0x012D => "Sony",
        0x0171 => "Amazon",
        0x01DA => "Logitech",
        0x027D => "Huawei",
        0x02E5 => "Espressif",
        0x038F => "Xiaomi",
        0x0499 => "Ruuvi",
        _ => return None,
    })
}
