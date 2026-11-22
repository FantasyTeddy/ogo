//! Sound for the Play window, made like the device's speaker makes it: sound effects are sine
//! tones played one after another from a short queue, music is up to two looping parts, quieter,
//! under them. Every note fades in and out.

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};

use adventure_format::{Music, Note, Wave};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

/// Sound effects waiting to play; more are dropped.
const QUEUE_LEN: usize = 4;
/// Share of the full level for effects while music plays, and for all music parts together.
const EFFECT_GAIN_WITH_MUSIC: f32 = 0.75;
const MUSIC_GAIN: f32 = 0.25;
const FADE_MS: u32 = 5;
/// The PC's speakers are much louder than the handheld's.
const LEVEL: f32 = 0.3;

/// The sound output while something plays; dropping it stops the sound.
pub struct Speaker {
    mixer: Arc<Mutex<Mixer>>,
    _stream: cpal::Stream,
}

impl Speaker {
    /// The default output; Err when the PC has none (the game then plays silently).
    pub fn open() -> Result<Speaker, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no sound output")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let mixer = Arc::new(Mutex::new(Mixer::new(config.sample_rate)));
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream::<f32>(&device, config, &mixer),
            cpal::SampleFormat::I16 => stream::<i16>(&device, config, &mixer),
            cpal::SampleFormat::U16 => stream::<u16>(&device, config, &mixer),
            cpal::SampleFormat::I32 => stream::<i32>(&device, config, &mixer),
            other => return Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Speaker {
            mixer,
            _stream: stream,
        })
    }

    /// Queues the notes after any sounds already queued.
    pub fn play(&self, notes: &[Note]) {
        if !notes.is_empty() {
            self.mixer.lock().unwrap().effect(notes.to_vec());
        }
    }

    /// Plays `music` in a loop from its start, instead of any before; None stops it.
    pub fn music(&self, music: Option<&Music>) {
        self.mixer.lock().unwrap().music(music.cloned());
    }
}

fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mixer: &Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels.max(1) as usize;
    let mixer = mixer.clone();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut mixer = mixer.lock().unwrap();
                for frame in out.chunks_mut(channels) {
                    let value = T::from_sample(LEVEL * mixer.next());
                    frame.fill(value);
                }
            },
            |error| eprintln!("sound: {error}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Makes the samples: the sound effects and the music, mixed.
struct Mixer {
    rate: u32,
    effects: VecDeque<Vec<Note>>,
    /// Plays the first of `effects`.
    effect: Voice,
    music: Option<Music>,
    music_voices: [Voice; 2],
}

impl Mixer {
    fn new(rate: u32) -> Mixer {
        Mixer {
            rate,
            effects: VecDeque::with_capacity(QUEUE_LEN),
            effect: Voice::default(),
            music: None,
            music_voices: Default::default(),
        }
    }

    fn effect(&mut self, notes: Vec<Note>) {
        if self.effects.len() < QUEUE_LEN {
            self.effects.push_back(notes);
        }
    }

    fn music(&mut self, music: Option<Music>) {
        self.music = music;
        self.music_voices = Default::default();
    }

    /// The next sample, -1.0 to 1.0.
    fn next(&mut self) -> f32 {
        let effect_gain = if self.music.is_some() {
            EFFECT_GAIN_WITH_MUSIC
        } else {
            1.0
        };
        let mut value = effect_gain * self.next_effect();
        if let Some(music) = &self.music {
            let gain = MUSIC_GAIN / music.parts.len() as f32;
            for (voice, part) in self.music_voices.iter_mut().zip(&music.parts) {
                value += gain
                    * voice
                        .next(&part.notes, part.wave, true, self.rate)
                        .unwrap_or(0.0);
            }
        }
        value
    }

    fn next_effect(&mut self) -> f32 {
        while let Some(notes) = self.effects.front() {
            if let Some(value) = self.effect.next(notes, Wave::Sine, false, self.rate) {
                return value;
            }
            self.effects.pop_front();
            self.effect = Voice::default();
        }
        0.0
    }
}

/// Plays a list of notes, one sample at a time.
#[derive(Debug, Default, Clone, Copy)]
struct Voice {
    /// The note to start after the current one.
    next: usize,
    /// Position in the current note and its length, in samples.
    position: u32,
    length: u32,
    hz: u16,
    /// Where in the wave, 0.0 to 1.0.
    phase: f32,
}

impl Voice {
    /// The next sample of `notes`, -1.0 to 1.0, with a raised-cosine fade at both ends of each
    /// note. None after the last note, unless `looping`.
    fn next(&mut self, notes: &[Note], wave: Wave, looping: bool, rate: u32) -> Option<f32> {
        let mut started = 0;
        while self.position >= self.length {
            if self.next >= notes.len() {
                if !looping {
                    return None;
                }
                self.next = 0;
            }
            // Only notes of length 0, or none at all
            if started > notes.len() {
                return None;
            }
            started += 1;
            let note = notes[self.next];
            self.next += 1;
            self.position = 0;
            self.length = (u64::from(rate) * u64::from(note.ms) / 1000) as u32;
            self.hz = note.hz;
        }
        let position = self.position;
        self.position += 1;
        if self.hz == 0 {
            return Some(0.0);
        }
        let fade = (rate * FADE_MS / 1000).min(self.length / 2).max(1);
        let edge = position.min(self.length - 1 - position);
        let envelope = if edge < fade {
            0.5 - 0.5 * (std::f32::consts::PI * edge as f32 / fade as f32).cos()
        } else {
            1.0
        };
        let value = match wave {
            Wave::Sine => (TAU * self.phase).sin(),
            Wave::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Triangle => 1.0 - 4.0 * (self.phase - 0.5).abs(),
        };
        self.phase = (self.phase + f32::from(self.hz) / rate as f32).fract();
        Some(envelope * value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_play_in_turn_and_end() {
        let mut mixer = Mixer::new(1000);
        mixer.effect(vec![Note::new(100, 10)]);
        mixer.effect(vec![Note::new(0, 5), Note::new(200, 10)]);
        let samples: Vec<f32> = (0..40).map(|_| mixer.next()).collect();
        assert!(
            samples[..10].iter().any(|&v| v.abs() > 0.1),
            "the first tone"
        );
        assert!(samples[10..15].iter().all(|&v| v == 0.0), "the pause");
        assert!(
            samples[15..25].iter().any(|&v| v.abs() > 0.1),
            "the second tone"
        );
        assert!(samples[25..].iter().all(|&v| v == 0.0), "then silence");
    }

    #[test]
    fn music_loops_under_the_effects() {
        let mut mixer = Mixer::new(1000);
        mixer.music(Some(Music {
            parts: vec![adventure_format::Part {
                wave: Wave::Square,
                notes: vec![Note::new(250, 20)],
            }],
        }));
        let loud = (0..100).map(|_| mixer.next().abs()).fold(0.0, f32::max);
        assert!(
            loud > 0.0 && loud <= MUSIC_GAIN + 0.001,
            "quiet, and still playing: {loud}"
        );
    }
}
