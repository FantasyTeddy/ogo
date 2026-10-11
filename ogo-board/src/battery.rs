//! Battery voltage, measured through a 1:2 resistor divider.

use esp_idf_svc::hal::{
    adc::{oneshot::AdcChannelDriver, ADCCH0, ADCU1},
    gpio::ADCPin,
};

use crate::adc::{Adc1, Adc1Channel, CHANNEL_CONFIG};

/// Samples averaged per reading, to smooth out ADC noise.
const SAMPLES: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryLevel {
    pub millivolts: u32,
    /// 0-100, see `ogo_format::battery::percent`. Rough, but good enough for an indicator.
    pub percent: u8,
}

pub struct Battery {
    channel: Adc1Channel<ADCCH0<ADCU1>>,
    adc: Adc1,
}

impl Battery {
    pub(crate) fn new(
        adc: Adc1,
        pin: impl ADCPin<AdcChannel = ADCCH0<ADCU1>> + 'static,
    ) -> anyhow::Result<Self> {
        Ok(Self { channel: AdcChannelDriver::new(adc, pin, &CHANNEL_CONFIG)?, adc })
    }

    pub fn read(&mut self) -> anyhow::Result<BatteryLevel> {
        let mut sum = 0;
        for _ in 0..SAMPLES {
            sum += u32::from(self.adc.read(&mut self.channel)?);
        }
        let millivolts = sum / SAMPLES * 2;
        Ok(BatteryLevel { millivolts, percent: ogo_format::battery::percent(millivolts) })
    }
}
