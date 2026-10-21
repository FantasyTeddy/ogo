//! The app slots of the partition table (partitions.csv) by label, and the launcher slots' names.

/// The factory slot with `ogo-recovery`.
pub const RECOVERY: &str = "recovery";
/// The slot apps are installed into.
pub const APP: &str = "app_0";

/// One of the two launcher slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherSlot {
    /// Partition label, e.g. "launcher_a".
    pub label: &'static str,
    /// What the UI calls it, e.g. "A".
    pub name: &'static str,
}

pub const LAUNCHER_A: LauncherSlot = LauncherSlot {
    label: "launcher_a",
    name: "A",
};
pub const LAUNCHER_B: LauncherSlot = LauncherSlot {
    label: "launcher_b",
    name: "B",
};
pub const LAUNCHER_SLOTS: [LauncherSlot; 2] = [LAUNCHER_A, LAUNCHER_B];

/// The launcher slot with this partition label, if it is one.
pub fn launcher_slot(label: &str) -> Option<LauncherSlot> {
    LAUNCHER_SLOTS.into_iter().find(|slot| slot.label == label)
}

/// Where an update goes when running from `label`: the other launcher slot, or slot A when not
/// running from a launcher slot (e.g. from recovery).
pub fn other_launcher_slot(label: &str) -> LauncherSlot {
    if label == LAUNCHER_A.label {
        LAUNCHER_B
    } else {
        LAUNCHER_A
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_other_slot() {
        assert_eq!(launcher_slot("launcher_b").map(|slot| slot.name), Some("B"));
        assert_eq!(launcher_slot(APP), None);
        assert_eq!(other_launcher_slot("launcher_a"), LAUNCHER_B);
        assert_eq!(other_launcher_slot("launcher_b"), LAUNCHER_A);
        assert_eq!(other_launcher_slot(RECOVERY), LAUNCHER_A);
    }

    /// The labels here must be the app slots of the real partition table.
    #[test]
    fn match_partitions_csv() {
        let partitions =
            crate::flash::parse_partitions(include_str!("../../partitions.csv")).unwrap();
        let apps: Vec<(&str, &str)> = partitions
            .iter()
            .filter(|p| p.kind == "app")
            .map(|p| (p.label.as_str(), p.subtype.as_str()))
            .collect();
        let expected = [
            (RECOVERY, "factory"),
            (LAUNCHER_A.label, "ota_0"),
            (LAUNCHER_B.label, "ota_1"),
            (APP, "ota_2"),
        ];
        assert_eq!(apps, expected);
    }
}
