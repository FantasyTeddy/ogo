//! Buttons. A, B, Select, Start, Menu and Volume are digital and active low.
//! The D-pad is two analog axes, each reading one of three voltage levels.

use esp_idf_svc::hal::{
    adc::{oneshot::AdcChannelDriver, ADCCH6, ADCCH7, ADCU1},
    gpio::{ADCPin, Input, InputPin, PinDriver, Pull},
};

use crate::adc::{Adc1, Adc1Channel, CHANNEL_CONFIG};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Button {
    Up = 1 << 0,
    Down = 1 << 1,
    Left = 1 << 2,
    Right = 1 << 3,
    A = 1 << 4,
    B = 1 << 5,
    Select = 1 << 6,
    Start = 1 << 7,
    Menu = 1 << 8,
    Volume = 1 << 9,
}

impl Button {
    pub const ALL: [Button; 10] = [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::A,
        Button::B,
        Button::Select,
        Button::Start,
        Button::Menu,
        Button::Volume,
    ];
}

/// The set of buttons held at one point in time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ButtonState(u16);

impl ButtonState {
    pub fn is_held(self, button: Button) -> bool {
        self.0 & button as u16 != 0
    }

    /// Held now but not in `previous`.
    pub fn just_pressed(self, previous: ButtonState, button: Button) -> bool {
        self.is_held(button) && !previous.is_held(button)
    }

    /// Held in `previous` but not now.
    pub fn just_released(self, previous: ButtonState, button: Button) -> bool {
        !self.is_held(button) && previous.is_held(button)
    }

    pub fn any(self) -> bool {
        self.0 != 0
    }

    /// The buttons held here but not in `other`.
    pub fn without(self, other: ButtonState) -> ButtonState {
        ButtonState(self.0 & !other.0)
    }

    /// The buttons held both here and in `other`.
    pub fn and(self, other: ButtonState) -> ButtonState {
        ButtonState(self.0 & other.0)
    }

    fn set(&mut self, button: Button, held: bool) {
        if held {
            self.0 |= button as u16;
        }
    }
}

/// Raw 12-bit ADC readings of the D-pad axes, for diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DpadRaw {
    pub x: u16,
    pub y: u16,
}

/// Axis thresholds (raw 12-bit): above `HIGH` is left/up, between `LOW` and `HIGH` is right/down.
const AXIS_HIGH: u16 = 2048 + 1024;
const AXIS_LOW: u16 = 1024;

pub struct Buttons {
    a: PinDriver<'static, Input>,
    b: PinDriver<'static, Input>,
    select: PinDriver<'static, Input>,
    start: PinDriver<'static, Input>,
    menu: PinDriver<'static, Input>,
    volume: PinDriver<'static, Input>,
    x: Adc1Channel<ADCCH6<ADCU1>>,
    y: Adc1Channel<ADCCH7<ADCU1>>,
    adc: Adc1,
    last_raw: DpadRaw,
}

impl Buttons {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        adc: Adc1,
        a: impl InputPin + 'static,
        b: impl InputPin + 'static,
        select: impl InputPin + 'static,
        start: impl InputPin + 'static,
        menu: impl InputPin + 'static,
        volume: impl InputPin + 'static,
        x: impl ADCPin<AdcChannel = ADCCH6<ADCU1>> + 'static,
        y: impl ADCPin<AdcChannel = ADCCH7<ADCU1>> + 'static,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            a: PinDriver::input(a, Pull::Up)?,
            b: PinDriver::input(b, Pull::Up)?,
            select: PinDriver::input(select, Pull::Up)?,
            // GPIO39 is input-only without internal pulls; the board has an external pull-up.
            start: PinDriver::input(start, Pull::Floating)?,
            menu: PinDriver::input(menu, Pull::Up)?,
            volume: PinDriver::input(volume, Pull::Up)?,
            x: AdcChannelDriver::new(adc, x, &CHANNEL_CONFIG)?,
            y: AdcChannelDriver::new(adc, y, &CHANNEL_CONFIG)?,
            adc,
            last_raw: DpadRaw::default(),
        })
    }

    /// Reads all buttons. Poll every 10-20 ms; that interval also debounces the contacts.
    pub fn read(&mut self) -> anyhow::Result<ButtonState> {
        let x = self.adc.read_raw(&mut self.x)?;
        let y = self.adc.read_raw(&mut self.y)?;
        self.last_raw = DpadRaw { x, y };

        let mut state = ButtonState::default();
        state.set(Button::Left, x > AXIS_HIGH);
        state.set(Button::Right, x > AXIS_LOW && x <= AXIS_HIGH);
        state.set(Button::Up, y > AXIS_HIGH);
        state.set(Button::Down, y > AXIS_LOW && y <= AXIS_HIGH);
        state.set(Button::A, self.a.is_low());
        state.set(Button::B, self.b.is_low());
        state.set(Button::Select, self.select.is_low());
        state.set(Button::Start, self.start.is_low());
        state.set(Button::Menu, self.menu.is_low());
        state.set(Button::Volume, self.volume.is_low());
        Ok(state)
    }

    /// D-pad readings from the last [`Buttons::read`].
    pub fn dpad_raw(&self) -> DpadRaw {
        self.last_raw
    }
}
