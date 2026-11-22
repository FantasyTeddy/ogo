//! Recorded sounds, mixed into samples for a speaker at its own rate. Several can play at once;
//! the device and the editor's Play window both play them through this.

use adventure_format::{Sound, SoundRef};

/// The most recorded sounds playing at once; a new one ends the oldest.
pub const MAX_PLAYING: usize = 4;
/// From signed 8-bit to signed 16-bit, a little below full scale, so that a recording and the
/// speaker's own tones together rarely clip.
const GAIN: i32 = 200;

/// The recorded sounds playing, and how far along each is.
#[derive(Debug, Default)]
pub struct Recordings {
    playing: Vec<Playing>,
}

#[derive(Debug)]
struct Playing {
    sound: usize,
    /// Where in the sound's samples, in 1/65536 of a sample.
    position: u64,
}

impl Recordings {
    /// Starts `sound` from its beginning. Notes aren't played here.
    pub fn start(&mut self, sounds: &[Sound], sound: SoundRef) {
        if let Some(Sound::Samples(_)) = sounds.get(sound.0 as usize) {
            if self.playing.len() == MAX_PLAYING {
                self.playing.remove(0);
            }
            self.playing.push(Playing {
                sound: sound.0 as usize,
                position: 0,
            });
        }
    }

    pub fn is_playing(&self) -> bool {
        !self.playing.is_empty()
    }

    pub fn stop(&mut self) {
        self.playing.clear();
    }

    /// The next `out.len()` samples at `rate` per second, mono, signed 16-bit: the sounds
    /// playing, mixed; silence where none is.
    pub fn fill(&mut self, sounds: &[Sound], rate: u32, out: &mut [i16]) {
        out.fill(0);
        self.playing.retain_mut(|playing| {
            let Some(Sound::Samples(sound)) = sounds.get(playing.sound) else {
                return false;
            };
            let samples = &sound.samples;
            let step = (u64::from(sound.rate) << 16) / u64::from(rate.max(1));
            for value in out.iter_mut() {
                let i = (playing.position >> 16) as usize;
                let Some(&a) = samples.get(i) else {
                    return false;
                };
                // Between two samples: in proportion
                let b = samples.get(i + 1).copied().unwrap_or(0);
                let part = (playing.position & 0xFFFF) as i32;
                let level = (i32::from(a) * (0x10000 - part) + i32::from(b) * part) >> 16;
                *value = (i32::from(*value) + level * GAIN).clamp(-32768, 32767) as i16;
                playing.position += step;
            }
            ((playing.position >> 16) as usize) < samples.len()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adventure_format::{Note, Samples};

    fn sounds() -> Vec<Sound> {
        vec![
            Sound::Notes(vec![Note::new(440, 100)]),
            Sound::Samples(Samples {
                rate: 8000,
                samples: vec![100, -100, 50, 0],
            }),
        ]
    }

    #[test]
    fn plays_at_the_speakers_rate_then_stops() {
        let sounds = sounds();
        let mut recordings = Recordings::default();
        recordings.start(&sounds, SoundRef(0));
        assert!(!recordings.is_playing(), "notes aren't recordings");
        recordings.start(&sounds, SoundRef(1));
        // 16 kHz: every sample twice, with the step between them halfway
        let mut out = [0; 10];
        recordings.fill(&sounds, 16000, &mut out);
        assert_eq!(out[0], 100 * GAIN as i16);
        assert_eq!(out[1], 0);
        assert_eq!(out[2], -100 * GAIN as i16);
        assert_eq!(&out[8..], [0, 0], "silence after the end");
        assert!(!recordings.is_playing());
    }

    #[test]
    fn several_mix_and_the_oldest_makes_room() {
        let sounds = sounds();
        let mut recordings = Recordings::default();
        recordings.start(&sounds, SoundRef(1));
        recordings.start(&sounds, SoundRef(1));
        let mut out = [0; 1];
        recordings.fill(&sounds, 8000, &mut out);
        assert_eq!(out[0], 32767, "two at once add up, to full scale at most");
        for _ in 0..MAX_PLAYING + 2 {
            recordings.start(&sounds, SoundRef(1));
        }
        assert_eq!(recordings.playing.len(), MAX_PLAYING);
    }
}
