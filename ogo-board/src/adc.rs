//! ADC1, shared by the D-pad axes and the battery sense pin.

use esp_idf_svc::hal::adc::{
    attenuation::DB_12,
    oneshot::{
        config::{AdcChannelConfig, Calibration},
        AdcChannelDriver, AdcDriver,
    },
    ADCU1,
};

pub(crate) type Adc1 = &'static AdcDriver<'static, ADCU1>;
pub(crate) type Adc1Channel<C> = AdcChannelDriver<'static, C, Adc1>;

/// Full range (about 0-3.1 V), with the factory calibration from eFuse for millivolt readings.
pub(crate) const CHANNEL_CONFIG: AdcChannelConfig = AdcChannelConfig {
    attenuation: DB_12,
    calibration: Calibration::Line,
    ..AdcChannelConfig::new()
};
