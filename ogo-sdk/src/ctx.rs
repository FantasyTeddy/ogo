//! [`Ctx`]: what a game can use while it runs.

use std::{
    ops::Range,
    time::{Duration, Instant},
};

use esp_idf_svc::{nvs::EspDefaultNvs, sys};
use ogo_board::{
    boot, Battery, BatteryLevel, Button, ButtonState, Buttons, Led, SdCard, Settings, Speaker,
};

use crate::saves::Saves;

const BATTERY_INTERVAL: Duration = Duration::from_secs(5);
/// A changed setting (VOL, the pause menu) is saved this long after the last change: by then the
/// amplifier is usually off already (see `ogo_board::speaker`), so muting it for the flash write
/// doesn't cut a sound. While music plays, the save waits until it stops or pauses; closing
/// the pause menu saves at once, before the music goes on.
const SETTINGS_SAVE_DELAY: Duration = Duration::from_secs(4);

/// Button state for the current frame.
#[derive(Debug, Default, Clone, Copy)]
pub struct Input {
    held: ButtonState,
    previous: ButtonState,
}

impl Input {
    /// Went down since the previous frame.
    pub fn pressed(&self, button: Button) -> bool {
        self.held.just_pressed(self.previous, button)
    }

    /// Is down.
    pub fn held(&self, button: Button) -> bool {
        self.held.is_held(button)
    }

    /// Went up since the previous frame.
    pub fn released(&self, button: Button) -> bool {
        self.held.just_released(self.previous, button)
    }

    pub(crate) fn update(&mut self, state: ButtonState) {
        self.previous = self.held;
        self.held = state;
    }
}

pub struct Ctx {
    input: Input,
    buttons: Buttons,
    speaker: Speaker,
    battery_sensor: Battery,
    battery: Option<BatteryLevel>,
    next_battery_read: Instant,
    led: Led,
    saves: Saves,
    /// The settings in NVS; `None` if they couldn't be opened.
    nvs: Option<EspDefaultNvs>,
    /// As applied to the speaker; VOL and the pause menu change them.
    settings: Settings,
    /// When to save changed settings.
    settings_save_at: Option<Instant>,
    frame: u64,
    // Keeps the card mounted for the saves.
    _sd: Option<SdCard>,
}

impl Ctx {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        game_id: &str,
        buttons: Buttons,
        speaker: Speaker,
        mut battery_sensor: Battery,
        led: Led,
        sd: Option<SdCard>,
        nvs: Option<EspDefaultNvs>,
        settings: Settings,
    ) -> Self {
        let battery = battery_sensor.read().ok();
        Self {
            input: Input::default(),
            buttons,
            speaker,
            battery_sensor,
            battery,
            next_battery_read: Instant::now() + BATTERY_INTERVAL,
            led,
            saves: Saves::new(sd.as_ref(), game_id),
            nvs,
            settings,
            settings_save_at: None,
            frame: 0,
            _sd: sd,
        }
    }

    /// Reads the buttons for a new frame, and the battery now and then.
    pub(crate) fn poll(&mut self) -> anyhow::Result<()> {
        self.input.update(self.buttons.read()?);
        let now = Instant::now();
        if now >= self.next_battery_read {
            self.next_battery_read = now + BATTERY_INTERVAL;
            self.battery = self.battery_sensor.read().ok();
        }
        if self.settings_save_at.is_some_and(|at| now >= at) && !self.speaker.music_playing() {
            self.save_settings();
        }
        Ok(())
    }

    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Changes the settings (VOL, the pause menu), applies them to the speaker and saves them soon.
    pub(crate) fn change_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        self.speaker.set_volume(self.settings.speaker_volume());
        self.speaker.set_music_enabled(self.settings.music);
        self.settings_save_at = Some(Instant::now() + SETTINGS_SAVE_DELAY);
    }

    /// Saves a changed setting now, if there is one.
    pub(crate) fn save_settings_now(&mut self) {
        if self.settings_save_at.is_some() {
            self.save_settings();
        }
    }

    fn save_settings(&mut self) {
        self.settings_save_at = None;
        let Some(nvs) = &self.nvs else {
            return;
        };
        // Writing flash stalls the audio DMA, which would click.
        self.speaker.set_muted(true);
        if let Err(e) = self.settings.save(nvs) {
            log::warn!("Saving the settings failed: {e:?}");
        }
        self.speaker.set_muted(false);
    }

    pub(crate) fn advance_frame(&mut self) {
        self.frame += 1;
    }

    pub fn input(&self) -> &Input {
        &self.input
    }

    pub fn sound(&mut self) -> &mut Speaker {
        &mut self.speaker
    }

    pub fn saves(&self) -> &Saves {
        &self.saves
    }

    /// Updated every few seconds; `None` if it couldn't be read.
    pub fn battery(&self) -> Option<BatteryLevel> {
        self.battery
    }

    pub fn set_led(&mut self, on: bool) {
        if let Err(e) = self.led.set_on(on) {
            log::warn!("Setting the LED failed: {e:?}");
        }
    }

    /// Number of updates so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// A random number from the hardware random number generator.
    pub fn random(&self) -> u32 {
        unsafe { sys::esp_random() }
    }

    /// A random number in `range`. Panics if the range is empty.
    pub fn random_range(&self, range: Range<u32>) -> u32 {
        assert!(!range.is_empty(), "empty range");
        range.start + self.random() % (range.end - range.start)
    }

    /// Restarts into the launcher. Saves are written immediately, so nothing is lost.
    pub fn exit_to_launcher(&mut self) -> ! {
        log::info!("Exiting to the launcher");
        self.save_settings_now();
        boot::restart()
    }
}
