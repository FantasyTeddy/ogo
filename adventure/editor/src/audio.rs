//! WAV files made into the recorded sounds the device plays: mono, [`RATE`], signed 8-bit, with the
//! quiet ends cut off and the loudest moment at full level.

use std::path::Path;

use adventure_format::{Samples, MAX_SAMPLE_SECONDS};

/// Samples per second of a converted sound: plenty for a voice on the device's small speaker.
pub const RATE: u16 = 16000;
/// Quieter than this share of the loudest moment counts as silence at the ends.
const QUIET: f32 = 0.03;

/// The WAV at `path`, converted.
pub fn load(path: &Path) -> Result<Samples, String> {
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut reader = hound::WavReader::open(path).map_err(|e| error(&e))?;
    let spec = reader.spec();
    let full = match spec.sample_format {
        hound::SampleFormat::Int => (1u64 << (spec.bits_per_sample.max(1) - 1)) as f32,
        hound::SampleFormat::Float => 1.0,
    };
    let values: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / full))
            .collect::<Result<_, _>>(),
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>(),
    }
    .map_err(|e| error(&e))?;
    convert(&values, spec.channels.max(1) as usize, spec.sample_rate).map_err(|e| error(&e))
}

/// Interleaved samples of `channels` at `rate`, -1.0 to 1.0, converted.
pub fn convert(values: &[f32], channels: usize, rate: u32) -> Result<Samples, String> {
    let mono: Vec<f32> = values
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    let peak = mono.iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
    if peak < 1e-4 {
        return Err("it's silent".into());
    }
    let loud = |v: &&f32| v.abs() >= peak * QUIET;
    let first = mono.iter().position(|v| loud(&v)).unwrap_or(0);
    let last = mono.iter().rposition(|v| loud(&v)).unwrap_or(0);
    let sound = &mono[first..=last];
    let seconds = sound.len() as f32 / rate.max(1) as f32;
    if seconds > MAX_SAMPLE_SECONDS as f32 {
        return Err(format!(
            "it's {seconds:.1} seconds long; at most {MAX_SAMPLE_SECONDS} fit"
        ));
    }
    // To RATE, in proportion between the two nearest samples
    let length = ((sound.len() as u64 * u64::from(RATE)) / u64::from(rate.max(1))).max(1) as usize;
    let step = rate as f64 / f64::from(RATE);
    let samples = (0..length)
        .map(|i| {
            let at = i as f64 * step;
            let (j, part) = (at as usize, at.fract() as f32);
            let a = sound[j.min(sound.len() - 1)];
            let b = sound[(j + 1).min(sound.len() - 1)];
            let value = (a + (b - a) * part) / peak;
            (value * 127.0).round().clamp(-128.0, 127.0) as i8
        })
        .collect();
    Ok(Samples {
        rate: RATE,
        samples,
    })
}

/// How long a converted sound plays, in seconds.
pub fn seconds(samples: &Samples) -> f32 {
    samples.samples.len() as f32 / f32::from(samples.rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_trimmed_loud_and_resampled() {
        // Stereo at 32 kHz: silence, a quiet tone, silence
        let mut values = vec![0.0; 200];
        for i in 0..400 {
            let v = if i % 2 == 0 { 0.25 } else { -0.25 };
            values.extend([v, v]);
        }
        values.extend(vec![0.0; 200]);
        let sound = convert(&values, 2, 32000).unwrap();
        assert_eq!(sound.rate, RATE);
        assert_eq!(sound.samples.len(), 200, "half as many, and no silence");
        assert_eq!(sound.samples.iter().map(|s| s.abs()).max(), Some(127));
    }

    #[test]
    fn refuses_silence_and_long_sounds() {
        assert!(convert(&[0.0; 100], 1, 8000)
            .unwrap_err()
            .contains("silent"));
        let long = vec![0.5; 8000 * 5];
        assert!(convert(&long, 1, 8000).unwrap_err().contains("5.0 seconds"));
    }

    #[test]
    fn reads_a_wav() {
        let dir = std::env::temp_dir().join(format!("adventure-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("beep.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..800 {
            writer
                .write_sample(if i % 4 < 2 { 8000i16 } else { -8000 })
                .unwrap();
        }
        writer.finalize().unwrap();
        let sound = load(&path).unwrap();
        assert_eq!(sound.samples.len(), 1600);
        assert!((seconds(&sound) - 0.1).abs() < 0.01);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
