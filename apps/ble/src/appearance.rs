//! The GAP Appearance value (characteristic 0x2A01): what kind of device it is, from the Bluetooth
//! SIG's Assigned Numbers (appearance_values.yaml). Bits 15 to 6 are the category, bits 5 to 0 the
//! subcategory.

/// E.g. "Watch" or "Human Interface Device: Mouse"; `None` for unknown values.
pub fn describe(value: u16) -> Option<String> {
    let category = value >> 6;
    let subcategory = value & 0x3F;
    let name = category_name(category)?;
    Some(match subcategory_name(category, subcategory) {
        Some(sub) => format!("{name}: {sub}"),
        None => name.to_owned(),
    })
}

fn category_name(category: u16) -> Option<&'static str> {
    Some(match category {
        0x000 => "Unknown",
        0x001 => "Phone",
        0x002 => "Computer",
        0x003 => "Watch",
        0x004 => "Clock",
        0x005 => "Display",
        0x006 => "Remote Control",
        0x007 => "Eye-glasses",
        0x008 => "Tag",
        0x009 => "Keyring",
        0x00A => "Media Player",
        0x00B => "Barcode Scanner",
        0x00C => "Thermometer",
        0x00D => "Heart Rate Sensor",
        0x00E => "Blood Pressure",
        0x00F => "Human Interface Device",
        0x010 => "Glucose Meter",
        0x011 => "Running Walking Sensor",
        0x012 => "Cycling",
        0x013 => "Control Device",
        0x014 => "Network Device",
        0x015 => "Sensor",
        0x016 => "Light Fixtures",
        0x017 => "Fan",
        0x018 => "HVAC",
        0x019 => "Air Conditioning",
        0x01A => "Humidifier",
        0x01B => "Heating",
        0x01C => "Access Control",
        0x01D => "Motorized Device",
        0x01E => "Power Device",
        0x01F => "Light Source",
        0x020 => "Window Covering",
        0x021 => "Audio Sink",
        0x022 => "Audio Source",
        0x023 => "Motorized Vehicle",
        0x024 => "Domestic Appliance",
        0x025 => "Wearable Audio Device",
        0x026 => "Aircraft",
        0x027 => "AV Equipment",
        0x028 => "Display Equipment",
        0x029 => "Hearing aid",
        0x02A => "Gaming",
        0x02B => "Signage",
        0x031 => "Pulse Oximeter",
        0x032 => "Weight Scale",
        0x033 => "Personal Mobility Device",
        0x034 => "Continuous Glucose Monitor",
        0x035 => "Insulin Pump",
        0x036 => "Medication Delivery",
        0x037 => "Spirometer",
        0x051 => "Outdoor Sports Activity",
        0x052 => "Industrial Measurement Device",
        0x053 => "Industrial Tools",
        0x054 => "Cookware Device",
        _ => return None,
    })
}

/// Subcategories for the categories most often met (computers, watches, input devices).
fn subcategory_name(category: u16, subcategory: u16) -> Option<&'static str> {
    Some(match (category, subcategory) {
        (0x002, 0x01) => "Desktop",
        (0x002, 0x02) => "Server",
        (0x002, 0x03) => "Laptop",
        (0x002, 0x04) => "Handheld PC/PDA",
        (0x002, 0x05) => "Palm-size PC/PDA",
        (0x002, 0x06) => "Wearable computer",
        (0x002, 0x07) => "Tablet",
        (0x002, 0x08) => "Docking Station",
        (0x002, 0x09) => "All in One",
        (0x002, 0x0A) => "Blade Server",
        (0x002, 0x0B) => "Convertible",
        (0x002, 0x0C) => "Detachable",
        (0x002, 0x0D) => "IoT Gateway",
        (0x002, 0x0E) => "Mini PC",
        (0x002, 0x0F) => "Stick PC",
        (0x003, 0x01) => "Sports Watch",
        (0x003, 0x02) => "Smartwatch",
        (0x00F, 0x01) => "Keyboard",
        (0x00F, 0x02) => "Mouse",
        (0x00F, 0x03) => "Joystick",
        (0x00F, 0x04) => "Gamepad",
        (0x00F, 0x05) => "Digitizer Tablet",
        (0x00F, 0x06) => "Card Reader",
        (0x00F, 0x07) => "Digital Pen",
        (0x00F, 0x08) => "Barcode Scanner",
        (0x00F, 0x09) => "Touchpad",
        (0x00F, 0x0A) => "Presentation Remote",
        _ => return None,
    })
}
