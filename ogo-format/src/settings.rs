//! The steps of the shared settings (brightness, volume). Storing them in NVS is
//! `ogo_board::settings`.

/// Backlight levels in percent. The lowest still shows the screen.
pub const BRIGHTNESS_STEPS: [u8; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// Volume levels in percent, as cycled by VOL in games.
pub const VOLUME_STEPS: [u8; 5] = [0, 25, 50, 75, 100];
pub const DEFAULT_BRIGHTNESS: u8 = 100;
pub const DEFAULT_VOLUME: u8 = 50;

/// The step `delta` steps away from `current`, clamped to the ends.
pub fn step(steps: &[u8], current: u8, delta: isize) -> u8 {
    let index = steps
        .iter()
        .position(|&s| s >= current)
        .unwrap_or(steps.len() - 1);
    steps[(index as isize + delta).clamp(0, steps.len() as isize - 1) as usize]
}

/// The step after `current`, starting over after the last one.
pub fn cycle(steps: &[u8], current: u8) -> u8 {
    let index = steps.iter().position(|&s| s >= current).unwrap_or(0);
    steps[(index + 1) % steps.len()]
}

/// The step closest to `value`, e.g. for a stored value that isn't a step.
pub fn nearest_step(steps: &[u8], value: u8) -> u8 {
    *steps
        .iter()
        .min_by_key(|&&s| s.abs_diff(value))
        .expect("steps aren't empty")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_moves_and_clamps() {
        assert_eq!(step(&BRIGHTNESS_STEPS, 50, 1), 60);
        assert_eq!(step(&BRIGHTNESS_STEPS, 50, -1), 40);
        assert_eq!(step(&BRIGHTNESS_STEPS, 100, 1), 100);
        assert_eq!(step(&BRIGHTNESS_STEPS, 10, -1), 10);
        assert_eq!(step(&VOLUME_STEPS, 0, -1), 0);
    }

    #[test]
    fn cycle_wraps_around() {
        let mut volume = DEFAULT_VOLUME;
        let seen: Vec<u8> = (0..5)
            .map(|_| {
                volume = cycle(&VOLUME_STEPS, volume);
                volume
            })
            .collect();
        assert_eq!(seen, [75, 100, 0, 25, 50]);
    }

    #[test]
    fn nearest() {
        assert_eq!(nearest_step(&VOLUME_STEPS, 30), 25);
        assert_eq!(nearest_step(&VOLUME_STEPS, 255), 100);
        assert_eq!(nearest_step(&BRIGHTNESS_STEPS, 0), 10);
    }

    #[test]
    fn defaults_are_steps() {
        assert!(BRIGHTNESS_STEPS.contains(&DEFAULT_BRIGHTNESS));
        assert!(VOLUME_STEPS.contains(&DEFAULT_VOLUME));
    }
}
