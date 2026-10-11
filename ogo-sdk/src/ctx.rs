//! [`Ctx`]: what a game can use while it runs.

use std::{
    ops::Range,
    time::{Duration, Instant},
};

use esp_idf_svc::sys;
use ogo_board::{boot, Battery, BatteryLevel, Button, ButtonState, Buttons, Led, SdCard, Speaker};

use crate::saves::Saves;

const BATTERY_INTERVAL: Duration = Duration::from_secs(5);

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
    frame: u64,
    // Keeps the card mounted for the saves.
    _sd: Option<SdCard>,
}

impl Ctx {
    pub(crate) fn new(
        game_id: &str,
        buttons: Buttons,
        speaker: Speaker,
        mut battery_sensor: Battery,
        led: Led,
        sd: Option<SdCard>,
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
        Ok(())
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
        boot::restart()
    }
}
