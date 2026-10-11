//! Battery percentage from the measured voltage.

/// Voltages treated as empty and full (Li-Po under light load), as in Retro-Go.
pub const EMPTY_MV: u32 = 3500;
pub const FULL_MV: u32 = 4200;

/// 0-100, linear between [`EMPTY_MV`] and [`FULL_MV`]. Rough, but good enough for an indicator.
pub fn percent(millivolts: u32) -> u8 {
    ((millivolts.clamp(EMPTY_MV, FULL_MV) - EMPTY_MV) * 100 / (FULL_MV - EMPTY_MV)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_linear_and_clamped() {
        assert_eq!(percent(0), 0);
        assert_eq!(percent(EMPTY_MV), 0);
        assert_eq!(percent(3850), 50);
        assert_eq!(percent(FULL_MV), 100);
        assert_eq!(percent(5000), 100);
    }
}
