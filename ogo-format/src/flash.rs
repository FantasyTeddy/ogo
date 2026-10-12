//! The flash layout outside the app slots, and merging firmware parts into one flash image.
//!
//! A complete image starts at address 0 and holds the bootloader, the partition table and app images
//! at their addresses; gaps are 0xFF, the value of erased flash. Writing it with
//! `espflash write-bin 0x0 <file>` erases what it covers first.

/// Where the ESP32's ROM loads the second-stage bootloader from.
pub const BOOTLOADER_OFFSET: u32 = 0x1000;
/// Where the bootloader reads the partition table (ESP-IDF's default).
pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;

/// One row of `partitions.csv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub label: String,
    /// "app" or "data"
    pub kind: String,
    pub subtype: String,
    pub offset: u32,
    pub size: u32,
}

/// Reads an ESP-IDF partition table in CSV form. Every row needs an explicit offset and size.
pub fn parse_partitions(csv: &str) -> Result<Vec<Partition>, String> {
    csv.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            if fields.len() < 5 {
                return Err(format!("partition row with fewer than 5 fields: {line}"));
            }
            Ok(Partition {
                label: fields[0].to_owned(),
                kind: fields[1].to_owned(),
                subtype: fields[2].to_owned(),
                offset: number(fields[3]).ok_or_else(|| format!("bad offset in: {line}"))?,
                size: number(fields[4]).ok_or_else(|| format!("bad size in: {line}"))?,
            })
        })
        .collect()
}

/// Merges `parts` (flash address, contents) into one image from address 0, filling gaps with 0xFF.
/// Fails if two parts overlap.
pub fn merge(parts: &[(u32, &[u8])]) -> Result<Vec<u8>, String> {
    let mut sorted: Vec<&(u32, &[u8])> = parts.iter().collect();
    sorted.sort_by_key(|(offset, _)| *offset);
    let mut image = Vec::new();
    for (offset, data) in sorted {
        let offset = *offset as usize;
        if offset < image.len() {
            return Err(format!("the part at {offset:#x} overlaps the one before (which ends at {:#x})", image.len()));
        }
        image.resize(offset, 0xFF);
        image.extend_from_slice(data);
    }
    Ok(image)
}

/// "0x10000", "64K", "1M" or decimal, as ESP-IDF allows.
fn number(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    let (digits, factor) = match text.chars().last()? {
        'K' | 'k' => (&text[..text.len() - 1], 1024),
        'M' | 'm' => (&text[..text.len() - 1], 1024 * 1024),
        _ => (text, 1),
    };
    digits.parse::<u32>().ok()?.checked_mul(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_project_table() {
        let partitions = parse_partitions(include_str!("../../partitions.csv")).unwrap();
        let recovery = partitions.iter().find(|p| p.label == "recovery").unwrap();
        assert_eq!((recovery.kind.as_str(), recovery.offset, recovery.size), ("app", 0x10000, 0x200000));
        // Nothing may overlap the partition table, and the rows must not overlap each other.
        let mut end = PARTITION_TABLE_OFFSET + 0x1000;
        for partition in &partitions {
            assert!(partition.offset >= end, "{} overlaps the one before", partition.label);
            end = partition.offset + partition.size;
        }
        assert!(end <= 16 * 1024 * 1024, "the table doesn't fit into 16 MB");
    }

    #[test]
    fn numbers() {
        assert_eq!(number("0x9000"), Some(0x9000));
        assert_eq!(number("64K"), Some(65536));
        assert_eq!(number("2M"), Some(2 * 1024 * 1024));
        assert_eq!(number("4096"), Some(4096));
        assert_eq!(number("0xZZ"), None);
        assert_eq!(number(""), None);
    }

    #[test]
    fn bad_rows() {
        assert!(parse_partitions("nvs, data, nvs, 0x9000").is_err());
        assert!(parse_partitions("nvs, data, nvs, nine, 0x4000").is_err());
    }

    #[test]
    fn merge_fills_gaps_and_sorts() {
        let image = merge(&[(4, &[2, 2]), (1, &[1])]).unwrap();
        assert_eq!(image, [0xFF, 1, 0xFF, 0xFF, 2, 2]);
    }

    #[test]
    fn merge_rejects_overlaps() {
        assert!(merge(&[(0, &[1, 1, 1]), (2, &[2])]).is_err());
        assert!(merge(&[(0, &[1, 1]), (2, &[2])]).is_ok());
    }
}
