//! Battery voltage, measured through a 1:2 resistor divider.

use esp_idf_svc::hal::{
    adc::{oneshot::AdcChannelDriver, ADCCH0, ADCU1},
    gpio::ADCPin,
};

use crate::adc::{Adc1, Adc1Channel, CHANNEL_CONFIG};

/// Voltages treated as empty and full for the percentage (Li-Po under light load).
const EMPTY_MV: u32 = 3500;
const FULL_MV: u32 = 4200;
/// Samples averaged per reading, to smooth out ADC noise.
const SAMPLES: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryLevel {
    pub millivolts: u32,
    /// 0-100, linear between `EMPTY_MV` and `FULL_MV`. Rough, but good enough for an indicator.
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
        let percent = (millivolts.clamp(EMPTY_MV, FULL_MV) - EMPTY_MV) * 100 / (FULL_MV - EMPTY_MV);
        Ok(BatteryLevel { millivolts, percent: percent as u8 })
    }
}
