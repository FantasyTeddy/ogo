//! The steps and defaults of the shared settings (brightness, volume, music, muted), and the sound
//! modes VOL switches between. Storing them in NVS is `ogo_board::settings`.

/// Backlight levels in percent. The lowest still shows the screen.
pub const BRIGHTNESS_STEPS: [u8; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// Volume levels in percent, as shown and stored. The speaker gets [`VOLUME_OUTPUT`] of them.
pub const VOLUME_STEPS: [u8; 11] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// What the speaker plays for each of [`VOLUME_STEPS`], in percent of full scale. Loudness is heard
/// on a log scale, so these grow by the same factor (about 2.2 dB) per step from 10 to 100: each step
/// sounds about equally louder. Below 10% the 8-bit DAC has too few levels left for the music, which
/// then sounds distorted.
pub const VOLUME_OUTPUT: [u8; 11] = [0, 10, 13, 17, 22, 28, 36, 46, 60, 77, 100];
pub const DEFAULT_BRIGHTNESS: u8 = 100;
pub const DEFAULT_VOLUME: u8 = 50;
pub const DEFAULT_MUSIC: bool = true;

/// The step `delta` steps away from `current`, clamped to the ends.
pub fn step(steps: &[u8], current: u8, delta: isize) -> u8 {
    let index = steps
        .iter()
        .position(|&s| s >= current)
        .unwrap_or(steps.len() - 1);
    steps[(index as isize + delta).clamp(0, steps.len() as isize - 1) as usize]
}

/// What VOL switches between in apps; in the launcher it only switches muted on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundMode {
    All,
    /// Music off.
    EffectsOnly,
    Muted,
}

impl SoundMode {
    pub fn new(music: bool, muted: bool) -> Self {
        match (music, muted) {
            (_, true) => SoundMode::Muted,
            (false, false) => SoundMode::EffectsOnly,
            (true, false) => SoundMode::All,
        }
    }

    /// The next one for VOL: all sound, effects only, muted, all again.
    pub fn next(self) -> Self {
        match self {
            SoundMode::All => SoundMode::EffectsOnly,
            SoundMode::EffectsOnly => SoundMode::Muted,
            SoundMode::Muted => SoundMode::All,
        }
    }
}

/// The speaker volume for a volume level (see [`VOLUME_OUTPUT`]).
pub fn volume_output(volume: u8) -> u8 {
    let index = VOLUME_STEPS
        .iter()
        .position(|&s| s == nearest_step(&VOLUME_STEPS, volume))
        .expect("the nearest step is a step");
    VOLUME_OUTPUT[index]
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
        let mut volume = 80;
        let seen: Vec<u8> = (0..4)
            .map(|_| {
                volume = cycle(&VOLUME_STEPS, volume);
                volume
            })
            .collect();
        assert_eq!(seen, [90, 100, 0, 10]);
    }

    #[test]
    fn sound_modes_cycle() {
        assert_eq!(SoundMode::new(true, false), SoundMode::All);
        assert_eq!(SoundMode::new(false, false), SoundMode::EffectsOnly);
        assert_eq!(SoundMode::new(true, true), SoundMode::Muted);
        assert_eq!(SoundMode::new(false, true), SoundMode::Muted);
        assert_eq!(SoundMode::All.next(), SoundMode::EffectsOnly);
        assert_eq!(SoundMode::EffectsOnly.next(), SoundMode::Muted);
        assert_eq!(SoundMode::Muted.next(), SoundMode::All);
    }

    #[test]
    fn volume_output_grows_evenly() {
        assert_eq!(volume_output(0), 0);
        assert_eq!(volume_output(10), 10);
        assert_eq!(volume_output(50), 28);
        assert_eq!(volume_output(100), 100);
        assert_eq!(volume_output(48), 28);
        // Each step about 1.29 times the one before.
        for pair in VOLUME_OUTPUT[1..].windows(2) {
            let ratio = f32::from(pair[1]) / f32::from(pair[0]);
            assert!((1.2..1.4).contains(&ratio), "{pair:?}");
        }
    }

    #[test]
    fn nearest() {
        assert_eq!(nearest_step(&VOLUME_STEPS, 74), 70);
        assert_eq!(nearest_step(&VOLUME_STEPS, 5), 0);
        assert_eq!(nearest_step(&VOLUME_STEPS, 255), 100);
        assert_eq!(nearest_step(&BRIGHTNESS_STEPS, 0), 10);
    }

    #[test]
    fn defaults_are_steps() {
        assert!(BRIGHTNESS_STEPS.contains(&DEFAULT_BRIGHTNESS));
        assert!(VOLUME_STEPS.contains(&DEFAULT_VOLUME));
    }
}
