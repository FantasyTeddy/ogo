//! Speaker: sound effects and music, generated as samples and played through the DAC's DMA mode.
//!
//! GPIO25 enables the amplifier (high = on) and GPIO26 (DAC channel 1) carries the signal.
//!
//! The DAC never leaves DMA mode: an audio thread keeps feeding it, with the idle level when
//! nothing plays. Switching DAC modes per sound, or leaving the input undriven, makes the
//! amplifier click or crackle. Every note also fades in and out.
//!
//! The audio thread generates the samples as it goes and mixes two things: sound effects (sine
//! tones, played one after another from a short queue) and music (a looping [`Track`] of up to
//! [`MAX_PARTS`] parts), which plays quieter, under the effects. Volume and tempo changes apply at
//! once.
//!
//! The amplifier hisses faintly while on, so the audio thread switches it off after
//! [`AMP_IDLE_OFF_MS`] of silence. It needs time to wake up (short beeps right after switching
//! it on are lost), so a sound that finds it off is delayed by [`AMP_WAKE_MS`].
//! It is also switched off around anything that disturbs the DAC: before a restart
//! ([`crate::boot::restart`]) and while writing flash ([`Speaker::set_muted`]).

use core::f32::consts::TAU;
use std::{
    collections::VecDeque,
    sync::{
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex, OnceLock,
    },
};

use esp_idf_svc::{
    hal::gpio::{Output, OutputPin, PinDriver},
    sys::{self, esp, EspError},
};

const DAC_CHANNEL_MASK: sys::dac_channel_mask_t = sys::dac_channel_mask_t_DAC_CHANNEL_MASK_CH1; // GPIO26
/// DAC level between sounds, and the center of every wave.
const IDLE_LEVEL: u8 = 128;
/// The ESP32's default DAC clock can't go below 19.6 kHz.
const SAMPLE_RATE: u32 = 32_000;
const DMA_DESCRIPTORS: u32 = 6;
const DMA_BUFFER_BYTES: usize = 128;
/// Samples the audio thread hands to the DMA per write (8 ms).
const CHUNK_SAMPLES: usize = 256;
/// Sound effects waiting to play; more are dropped.
const QUEUE_LEN: usize = 4;
/// Messages to the audio thread that can wait; it takes them every [`CHUNK_SAMPLES`].
const COMMAND_QUEUE_LEN: usize = 16;
/// The audio thread generates the samples itself (floats, envelopes, logging): 4 KB isn't enough.
const AUDIO_STACK_SIZE: usize = 8192;
/// Fade-in and fade-out of every note.
const FADE_SAMPLES: u32 = SAMPLE_RATE * 5 / 1000;
/// Share of the full level for effects while music plays, and for all music parts together. They
/// add up to 1, so the mix never clips.
const EFFECT_GAIN_WITH_MUSIC: f32 = 0.75;
const MUSIC_GAIN: f32 = 0.25;
/// Parts a [`Track`] can have.
pub const MAX_PARTS: usize = 2;
/// Silence after which the amplifier is switched off.
pub const AMP_IDLE_OFF_MS: u32 = 3000;
/// Idle level played after switching the amplifier on, before the sound starts.
pub const AMP_WAKE_MS: u32 = 150;

/// One tone in a sound. A frequency of 0 is a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub frequency_hz: u32,
    pub duration_ms: u32,
}

impl Note {
    pub const fn new(frequency_hz: u32, duration_ms: u32) -> Self {
        Self {
            frequency_hz,
            duration_ms,
        }
    }
}

/// The total length of `notes`.
pub const fn duration_ms(notes: &[Note]) -> u32 {
    let mut total = 0;
    let mut i = 0;
    while i < notes.len() {
        total += notes[i].duration_ms;
        i += 1;
    }
    total
}

/// The shape of a music part's tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    /// Soft and round, like the sound effects.
    Sine,
    /// Bright and buzzy; good for a melody.
    Square,
    /// Between the two; good for a bass line.
    Triangle,
}

impl Wave {
    /// The wave at `phase` (a full turn is 2^32), -1.0 to 1.0.
    fn at(self, phase: u32) -> f32 {
        match self {
            Wave::Sine => sine_table()[(phase >> 24) as usize],
            Wave::Square => {
                if phase < 1 << 31 {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Triangle => 1.0 - 4.0 * (phase as f32 / 4_294_967_296.0 - 0.5).abs(),
        }
    }
}

/// One voice of a [`Track`]: its notes, played in a loop.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    pub wave: Wave,
    pub notes: &'static [Note],
}

/// Music: up to [`MAX_PARTS`] parts played together, in a loop.
#[derive(Debug, Clone, Copy)]
pub struct Track {
    parts: &'static [Part],
}

impl Track {
    /// Panics if there are no parts or more than [`MAX_PARTS`], or if the parts differ in length,
    /// as they would drift apart. In a `const`, that's a compile error.
    pub const fn new(parts: &'static [Part]) -> Self {
        assert!(
            !parts.is_empty() && parts.len() <= MAX_PARTS,
            "a track has 1 or 2 parts"
        );
        let length = duration_ms(parts[0].notes);
        assert!(length > 0, "a track can't be empty");
        let mut i = 1;
        while i < parts.len() {
            assert!(
                duration_ms(parts[i].notes) == length,
                "the parts of a track differ in length"
            );
            i += 1;
        }
        Self { parts }
    }
}

/// The amplifier enable pin and what may switch it on. Shared by [`Speaker`] and the audio thread.
struct Amp {
    pin: PinDriver<'static, Output>,
    on: bool,
    /// Volume 0 or muted: must stay off.
    blocked: bool,
}

impl Amp {
    fn set(&mut self, on: bool) {
        let on = on && !self.blocked;
        if on != self.on {
            let result = if on {
                self.pin.set_high()
            } else {
                self.pin.set_low()
            };
            if let Err(e) = result {
                log::error!("Switching the amplifier failed: {e:?}");
                return;
            }
            self.on = on;
        }
    }
}

/// What the [`Speaker`] tells the audio thread.
enum Command {
    Effect(Vec<Note>),
    Volume(u8),
    Music(Option<Track>),
    MusicSpeed(u32),
    MusicEnabled(bool),
    MusicPaused(bool),
}

pub struct Speaker {
    amp: Arc<Mutex<Amp>>,
    commands: SyncSender<Command>,
    /// 0-100
    volume: u8,
    muted: bool,
    /// A track is set (it may be paused or switched off).
    music: bool,
    music_enabled: bool,
    music_paused: bool,
}

impl Speaker {
    /// `_signal` is taken so nothing else can claim the DAC pin; the DAC driver configures it.
    pub(crate) fn new(
        enable: impl OutputPin + 'static,
        _signal: impl OutputPin + 'static,
    ) -> anyhow::Result<Self> {
        let mut pin = PinDriver::output(enable)?;
        pin.set_low()?;
        let amp = Arc::new(Mutex::new(Amp {
            pin,
            on: false,
            blocked: false,
        }));

        let volume = 50;
        // Built here rather than on the audio thread's smaller stack.
        sine_table();
        let dac = Dac::open()?;
        let (commands, receiver) = mpsc::sync_channel(COMMAND_QUEUE_LEN);
        let thread_amp = amp.clone();
        crate::thread::spawn_on_second_core("audio", AUDIO_STACK_SIZE, move || {
            feed(dac, receiver, thread_amp, Mixer::new(volume))
        })?;

        Ok(Self {
            amp,
            commands,
            volume,
            muted: false,
            music: false,
            music_enabled: true,
            music_paused: false,
        })
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// Volume 0-100. 0 is silent and keeps the amplifier off.
    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
        self.update_amp();
        self.send(Command::Volume(self.volume));
    }

    /// Keeps the amplifier off without changing the volume, e.g. while writing flash: that stalls
    /// the audio DMA, which would click. Sounds played while muted are lost.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.update_amp();
    }

    fn update_amp(&mut self) {
        let mut amp = self.amp.lock().unwrap();
        amp.blocked = self.volume == 0 || self.muted;
        if amp.blocked {
            amp.set(false);
        }
    }

    /// Queues one tone. Returns immediately.
    pub fn beep(&mut self, frequency_hz: u32, duration_ms: u32) {
        self.play(&[Note::new(frequency_hz, duration_ms)]);
    }

    /// Queues the notes to play back to back, after any sounds already queued. Returns immediately.
    /// If too many sounds are waiting, this one is dropped.
    pub fn play(&mut self, notes: &[Note]) {
        if self.volume == 0 || self.muted || notes.is_empty() {
            return;
        }
        self.send(Command::Effect(notes.to_vec()));
    }

    /// Plays `track` in a loop from its start, instead of any track before. Sound effects play
    /// over it.
    pub fn play_music(&mut self, track: Track) {
        self.music = true;
        self.send(Command::Music(Some(track)));
    }

    pub fn stop_music(&mut self) {
        self.music = false;
        self.send(Command::Music(None));
    }

    /// Tempo of the music in percent of the written one, 25-400; applies from the next note.
    pub fn set_music_speed(&mut self, percent: u32) {
        self.send(Command::MusicSpeed(percent.clamp(25, 400)));
    }

    pub fn music_enabled(&self) -> bool {
        self.music_enabled
    }

    /// The user's music setting. While off, the track is kept, and it goes on from where it was
    /// when switched on again.
    pub fn set_music_enabled(&mut self, enabled: bool) {
        self.music_enabled = enabled;
        self.send(Command::MusicEnabled(enabled));
    }

    /// Holds the music where it is, e.g. while a pause menu is open.
    pub fn set_music_paused(&mut self, paused: bool) {
        self.music_paused = paused;
        self.send(Command::MusicPaused(paused));
    }

    /// Music can be heard: it's set, switched on and not paused, and the volume isn't 0.
    pub fn music_playing(&self) -> bool {
        self.music && self.music_enabled && !self.music_paused && self.volume > 0
    }

    fn send(&self, command: Command) {
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(Command::Effect(_))) => {
                log::debug!("Audio queue full, dropping a sound")
            }
            Err(TrySendError::Full(_)) => log::warn!("Audio queue full, dropping a change"),
            Err(TrySendError::Disconnected(_)) => log::error!("The audio thread has stopped"),
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.set_muted(true);
    }
}

/// The DAC channel in DMA mode. Owned by the audio thread.
struct Dac(sys::dac_continuous_handle_t);

// The handle is only used from the audio thread after it's been handed over.
unsafe impl Send for Dac {}

impl Dac {
    fn open() -> Result<Self, EspError> {
        let config = sys::dac_continuous_config_t {
            chan_mask: DAC_CHANNEL_MASK,
            desc_num: DMA_DESCRIPTORS,
            buf_size: DMA_BUFFER_BYTES,
            freq_hz: SAMPLE_RATE,
            offset: 0,
            clk_src: sys::soc_periph_dac_digi_clk_src_t_DAC_DIGI_CLK_SRC_DEFAULT,
            chan_mode: sys::dac_continuous_channel_mode_t_DAC_CHANNEL_MODE_SIMUL,
        };
        let mut handle = core::ptr::null_mut();
        esp!(unsafe { sys::dac_continuous_new_channels(&config, &mut handle) })?;
        if let Err(e) = esp!(unsafe { sys::dac_continuous_enable(handle) }) {
            unsafe { sys::dac_continuous_del_channels(handle) };
            return Err(e);
        }
        Ok(Self(handle))
    }

    /// Blocks until the samples are in the DMA buffers.
    fn write(&mut self, samples: &mut [u8]) -> Result<(), EspError> {
        let mut loaded = 0;
        esp!(unsafe {
            sys::dac_continuous_write(self.0, samples.as_mut_ptr(), samples.len(), &mut loaded, -1)
        })
    }

    /// Plays the idle level for about `ms`.
    fn write_idle(&mut self, ms: u32) -> Result<(), EspError> {
        let mut chunk = [IDLE_LEVEL; CHUNK_SAMPLES];
        let chunks = (SAMPLE_RATE * ms / 1000).div_ceil(CHUNK_SAMPLES as u32);
        (0..chunks).try_for_each(|_| self.write(&mut chunk))
    }
}

/// The audio thread: plays what the mixer makes, and the idle level when it's silent; switches the
/// amplifier on for sounds and off after a while of silence.
fn feed(mut dac: Dac, commands: Receiver<Command>, amp: Arc<Mutex<Amp>>, mut mixer: Mixer) {
    let chunk_ms = CHUNK_SAMPLES as u32 * 1000 / SAMPLE_RATE;
    let mut idle_ms = 0;
    let mut chunk = [IDLE_LEVEL; CHUNK_SAMPLES];
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => mixer.apply(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let result = if mixer.is_playing() {
            idle_ms = 0;
            let woke = {
                let mut amp = amp.lock().unwrap();
                let was_on = amp.on;
                amp.set(true);
                amp.on && !was_on
            };
            (|| {
                if woke {
                    dac.write_idle(AMP_WAKE_MS)?;
                }
                mixer.fill(&mut chunk);
                dac.write(&mut chunk)
            })()
        } else {
            // Keep the DMA fed with the idle level while waiting for a sound.
            if idle_ms < AMP_IDLE_OFF_MS {
                idle_ms += chunk_ms;
                if idle_ms >= AMP_IDLE_OFF_MS {
                    amp.lock().unwrap().set(false);
                }
            }
            chunk.fill(IDLE_LEVEL);
            dac.write(&mut chunk)
        };
        if let Err(e) = result {
            log::error!("Writing to the DAC failed, audio stops: {e:?}");
            return;
        }
    }
}

/// Makes the samples: the sound effects and the music, mixed. Owned by the audio thread.
struct Mixer {
    volume: u8,
    effects: VecDeque<Vec<Note>>,
    /// Plays the first of `effects`.
    effect: Voice,
    music: Option<Track>,
    music_voices: [Voice; MAX_PARTS],
    music_speed: u32,
    music_enabled: bool,
    music_paused: bool,
}

impl Mixer {
    fn new(volume: u8) -> Self {
        Self {
            volume,
            effects: VecDeque::with_capacity(QUEUE_LEN),
            effect: Voice::default(),
            music: None,
            music_voices: Default::default(),
            music_speed: 100,
            music_enabled: true,
            music_paused: false,
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Effect(notes) => {
                if self.effects.len() < QUEUE_LEN {
                    self.effects.push_back(notes);
                } else {
                    log::debug!("Sound queue full, dropping a sound");
                }
            }
            Command::Volume(volume) => self.volume = volume,
            Command::Music(track) => {
                self.music = track;
                self.music_voices = Default::default();
            }
            Command::MusicSpeed(percent) => self.music_speed = percent,
            Command::MusicEnabled(enabled) => self.music_enabled = enabled,
            Command::MusicPaused(paused) => self.music_paused = paused,
        }
    }

    /// The track, if it should be heard.
    fn music(&self) -> Option<Track> {
        self.music
            .filter(|_| self.music_enabled && !self.music_paused)
    }

    fn is_playing(&self) -> bool {
        !self.effects.is_empty() || self.music().is_some()
    }

    /// 8-bit unsigned samples around `IDLE_LEVEL`.
    fn fill(&mut self, chunk: &mut [u8]) {
        let amplitude = 127.0 * f32::from(self.volume) / 100.0;
        let music = self.music();
        let effect_gain = if music.is_some() {
            EFFECT_GAIN_WITH_MUSIC
        } else {
            1.0
        };
        for sample in chunk {
            let mut value = effect_gain * self.next_effect();
            if let Some(track) = music {
                let gain = MUSIC_GAIN / track.parts.len() as f32;
                for (voice, part) in self.music_voices.iter_mut().zip(track.parts) {
                    let next = voice.next(part.notes, part.wave, true, self.music_speed);
                    value += gain * next.unwrap_or(0.0);
                }
            }
            let level = f32::from(IDLE_LEVEL) + amplitude * value;
            *sample = level.round().clamp(0.0, 255.0) as u8;
        }
    }

    fn next_effect(&mut self) -> f32 {
        while let Some(notes) = self.effects.front() {
            if let Some(value) = self.effect.next(notes, Wave::Sine, false, 100) {
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
    frequency_hz: u32,
    /// A full turn is 2^32; `step` is added per sample.
    phase: u32,
    step: u32,
}

impl Voice {
    /// The next sample of `notes`, -1.0 to 1.0, with a raised-cosine fade at both ends of each
    /// note. `None` after the last note, unless `looping`. `speed` is the tempo in percent.
    fn next(&mut self, notes: &[Note], wave: Wave, looping: bool, speed: u32) -> Option<f32> {
        let mut started = 0;
        while self.position >= self.length {
            if self.next >= notes.len() {
                if !looping {
                    return None;
                }
                self.next = 0;
            }
            // Only notes of length 0, or none at all.
            if started > notes.len() {
                return None;
            }
            started += 1;
            let note = notes[self.next];
            self.next += 1;
            self.position = 0;
            self.length = (u64::from(SAMPLE_RATE) * u64::from(note.duration_ms) * 100
                / (1000 * u64::from(speed.max(1)))) as u32;
            self.frequency_hz = note.frequency_hz;
            self.step = ((u64::from(note.frequency_hz) << 32) / u64::from(SAMPLE_RATE)) as u32;
        }

        let position = self.position;
        self.position += 1;
        if self.frequency_hz == 0 {
            return Some(0.0);
        }
        let fade = FADE_SAMPLES.min(self.length / 2).max(1);
        let edge = position.min(self.length - 1 - position);
        let envelope = if edge < fade {
            // 0.5 - 0.5 * cos(pi * edge / fade), with cos(x) = sin(x + pi / 2)
            0.5 - 0.5 * sine_table()[((64 + edge * 128 / fade) & 255) as usize]
        } else {
            1.0
        };
        let value = wave.at(self.phase);
        self.phase = self.phase.wrapping_add(self.step);
        Some(envelope * value)
    }
}

/// One full turn of a sine in 256 steps, on the heap.
fn sine_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| (0..256).map(|i| (TAU * i as f32 / 256.0).sin()).collect())
}
