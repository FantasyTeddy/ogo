//! Turns raw button states into menu navigation, with key repeat for the D-pad.

use std::time::{Duration, Instant};

use ogo_board::{Button, ButtonState};

const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    PageUp,
    PageDown,
    Select,
    Rescan,
    /// SELECT: open the WiFi / web page screen.
    Connect,
    /// B: leave the current screen.
    Back,
}

/// Press events; Up/Down/Left/Right repeat while held.
#[derive(Default)]
pub struct Navigator {
    previous: ButtonState,
    /// The repeating direction and when it fires next.
    repeat: Option<(Button, Instant)>,
}

impl Navigator {
    pub fn update(&mut self, state: ButtonState, now: Instant) -> Option<Nav> {
        let previous = std::mem::replace(&mut self.previous, state);

        if state.just_pressed(previous, Button::A) {
            return Some(Nav::Select);
        }
        if state.just_pressed(previous, Button::Start) {
            return Some(Nav::Rescan);
        }
        if state.just_pressed(previous, Button::Select) {
            return Some(Nav::Connect);
        }
        if state.just_pressed(previous, Button::B) {
            return Some(Nav::Back);
        }

        for button in [Button::Up, Button::Down, Button::Left, Button::Right] {
            if state.just_pressed(previous, button) {
                self.repeat = Some((button, now + REPEAT_DELAY));
                return Some(direction(button));
            }
        }
        match self.repeat {
            Some((button, _)) if !state.is_held(button) => self.repeat = None,
            Some((button, next)) if now >= next => {
                self.repeat = Some((button, now + REPEAT_INTERVAL));
                return Some(direction(button));
            }
            _ => {}
        }
        None
    }
}

fn direction(button: Button) -> Nav {
    match button {
        Button::Up => Nav::Up,
        Button::Down => Nav::Down,
        Button::Left => Nav::PageUp,
        _ => Nav::PageDown,
    }
}
