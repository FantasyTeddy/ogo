//! MIDI files made into the device's music: at most two parts that play one note at a time. A
//! file's notes are grouped by track and channel (drums left out); a part is one group, made into
//! a single line by keeping the top note where several sound at once (a melody) or the bottom one
//! (a bass).

use adventure_format::{Music, Note, Wave};
use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

/// The channel General MIDI keeps for drums, counted from 0.
const DRUMS: u8 = 9;
/// The longest note or pause the format holds is 65535 ms; longer ones are split.
const MAX_MS: u32 = 60_000;
/// The longest song kept, in ms: anything after is cut off.
pub const MAX_SONG_MS: u32 = 10 * 60_000;

/// What a MIDI file holds, ready to choose parts from.
#[derive(Debug)]
pub struct Song {
    pub groups: Vec<Group>,
    /// Up to the end of the last note, rounded up to a whole beat, so that it loops in time.
    pub length_ms: u32,
}

/// The notes of one channel in one track.
#[derive(Debug)]
pub struct Group {
    /// For people: the track's name or the instrument, and how many notes in which range.
    pub label: String,
    pub notes: Vec<Timed>,
}

/// A note from the file, in ms from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timed {
    pub start: u32,
    pub end: u32,
    pub key: u8,
}

impl Group {
    fn average_key(&self) -> f32 {
        self.notes.iter().map(|n| f32::from(n.key)).sum::<f32>() / self.notes.len().max(1) as f32
    }
}

/// Reads a MIDI file.
pub fn read(bytes: &[u8]) -> Result<Song, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("not a MIDI file: {e}"))?;
    let clock = Clock::new(&smf);
    let mut groups = Vec::new();
    let mut last_tick = 0;
    let mut end_tick = 0;
    for (t, track) in smf.tracks.iter().enumerate() {
        let mut name = None;
        let mut programs = [None; 16];
        // Per channel: notes held (key, start tick), and the finished ones (start, end, key)
        let mut held: Vec<Vec<(u8, u64)>> = vec![Vec::new(); 16];
        let mut done: Vec<Vec<(u64, u64, u8)>> = vec![Vec::new(); 16];
        let mut tick = 0u64;
        for event in track {
            tick += u64::from(event.delta.as_int());
            match event.kind {
                TrackEventKind::Meta(MetaMessage::EndOfTrack) => end_tick = end_tick.max(tick),
                TrackEventKind::Meta(MetaMessage::TrackName(bytes)) => {
                    let text = String::from_utf8_lossy(bytes).trim().to_string();
                    if !text.is_empty() {
                        name = Some(text);
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let c = channel.as_int() as usize;
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            held[c].push((key.as_int(), tick));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(i) = held[c].iter().position(|&(k, _)| k == key.as_int()) {
                                let (key, start) = held[c].remove(i);
                                done[c].push((start, tick, key));
                                last_tick = last_tick.max(tick);
                            }
                        }
                        MidiMessage::ProgramChange { program } => {
                            programs[c].get_or_insert(program.as_int());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        for (channel, notes) in done.into_iter().enumerate() {
            if notes.is_empty() || channel == DRUMS as usize {
                continue;
            }
            let mut notes: Vec<Timed> = notes
                .into_iter()
                .map(|(start, end, key)| Timed {
                    start: clock.ms(start),
                    end: clock.ms(end),
                    key,
                })
                .filter(|n| n.start < MAX_SONG_MS && n.end > n.start)
                .collect();
            notes.sort_by_key(|n| (n.start, n.key));
            if notes.is_empty() {
                continue;
            }
            let (low, high) = notes
                .iter()
                .fold((u8::MAX, 0), |(lo, hi), n| (lo.min(n.key), hi.max(n.key)));
            let what = name.clone().unwrap_or_else(|| match programs[channel] {
                Some(program) => family(program).to_string(),
                None => format!("Track {}", t + 1),
            });
            groups.push(Group {
                label: format!(
                    "{what} · {} notes, {}–{}",
                    notes.len(),
                    key_name(low),
                    key_name(high)
                ),
                notes,
            });
        }
    }
    if groups.is_empty() {
        return Err("it has no notes (apart from drums)".into());
    }
    // A rest at the end, as far as the tracks' end says (but not much more than a bar of it)
    let end = end_tick.clamp(last_tick, last_tick + 4 * clock.per_beat);
    let length_ms = clock.ms(clock.whole_beats(end)).min(MAX_SONG_MS);
    Ok(Song { groups, length_ms })
}

/// A melody and, if there's another group, a bass: the highest and the lowest on average.
pub fn suggest(song: &Song) -> (usize, Option<usize>) {
    let by_pitch = |i: &usize| song.groups[*i].average_key();
    let all = 0..song.groups.len();
    let melody = all
        .clone()
        .max_by(|a, b| by_pitch(a).total_cmp(&by_pitch(b)))
        .unwrap_or(0);
    let bass = all
        .filter(|&i| i != melody)
        .min_by(|a, b| by_pitch(a).total_cmp(&by_pitch(b)));
    (melody, bass)
}

/// One group as notes of the format, exactly `length_ms` long: where several of its notes sound
/// at once, the top one (`top`) or the bottom one; pauses where none does.
pub fn line(group: &Group, top: bool, length_ms: u32) -> Vec<Note> {
    // Every moment a note starts or ends cuts the time into pieces; each piece plays one note
    let mut cuts: Vec<u32> = group
        .notes
        .iter()
        .flat_map(|n| [n.start, n.end])
        .chain([0, length_ms])
        .filter(|&t| t <= length_ms)
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    // (start, end, which note of the group or none)
    let mut pieces: Vec<(u32, u32, Option<usize>)> = Vec::new();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let sounding = group
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.start <= from && n.end >= to);
        let chosen = if top {
            sounding.max_by_key(|(i, n)| (n.key, usize::MAX - i))
        } else {
            sounding.min_by_key(|(i, n)| (n.key, *i))
        }
        .map(|(i, _)| i);
        match pieces.last_mut() {
            // The same note going on, or a pause going on: one longer piece
            Some(last) if last.2 == chosen => last.1 = to,
            _ => pieces.push((from, to, chosen)),
        }
    }
    let mut notes = Vec::new();
    for (from, to, chosen) in pieces {
        let hz = chosen.map_or(0, |i| frequency(group.notes[i].key));
        let mut ms = to - from;
        while ms > 0 {
            let part = ms.min(MAX_MS);
            notes.push(Note::new(hz, part as u16));
            ms -= part;
        }
    }
    notes
}

/// `music` as a MIDI file: a track per part, on a channel each, named "Melody" and "Bass", with
/// the beat chosen so that every note is a whole number of ticks; [`read`] and [`line`] give back
/// the same notes.
pub fn write(music: &Music) -> Vec<u8> {
    const PER_BEAT: u64 = 480;
    let gcd = |mut a: u64, mut b: u64| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let beat = music
        .parts
        .iter()
        .flat_map(|part| &part.notes)
        .map(|note| u64::from(note.ms))
        .fold(0, gcd)
        .clamp(1, 16_000);
    let ticks = |ms: u64| ms * PER_BEAT / beat;
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(PER_BEAT as u16)),
    ));
    let end = |delta: u64| TrackEvent {
        delta: u28::new(delta as u32),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    };
    smf.tracks.push(vec![
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(beat as u32 * 1000))),
        },
        end(0),
    ]);
    for (p, part) in music.parts.iter().enumerate() {
        let channel = u4::new(p as u8);
        let name: &'static [u8] = if p == 0 { b"Melody" } else { b"Bass" };
        // General MIDI instruments that sound like the waves: square lead, synth bass, flute
        let program = match part.wave {
            Wave::Square => 80,
            Wave::Triangle => 38,
            Wave::Sine => 73,
        };
        let mut track = vec![
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::TrackName(name)),
            },
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::ProgramChange {
                        program: u7::new(program),
                    },
                },
            },
        ];
        // Ticks since the last event written
        let mut waiting = 0;
        for note in &part.notes {
            let length = ticks(u64::from(note.ms));
            if note.hz == 0 {
                waiting += length;
                continue;
            }
            let key = u7::new(key(note.hz));
            track.push(TrackEvent {
                delta: u28::new(waiting as u32),
                kind: TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn {
                        key,
                        vel: u7::new(100),
                    },
                },
            });
            track.push(TrackEvent {
                delta: u28::new(length as u32),
                kind: TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff {
                        key,
                        vel: u7::new(0),
                    },
                },
            });
            waiting = 0;
        }
        track.push(end(waiting));
        smf.tracks.push(track);
    }
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)
        .expect("writing into memory doesn't fail");
    bytes
}

/// The MIDI key nearest to `hz`.
fn key(hz: u16) -> u8 {
    (69.0 + 12.0 * (f32::from(hz) / 440.0).log2())
        .round()
        .clamp(0.0, 127.0) as u8
}

/// The frequency of MIDI key `key`, where 69 is A at 440 Hz.
fn frequency(key: u8) -> u16 {
    (440.0 * 2f32.powf((f32::from(key) - 69.0) / 12.0)).round() as u16
}

fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    format!("{}{}", NAMES[key as usize % 12], key as i32 / 12 - 1)
}

/// The instrument family of a General MIDI program.
fn family(program: u8) -> &'static str {
    const FAMILIES: [&str; 16] = [
        "Piano",
        "Bells",
        "Organ",
        "Guitar",
        "Bass",
        "Strings",
        "Ensemble",
        "Brass",
        "Reed",
        "Pipe",
        "Synth lead",
        "Synth pad",
        "Synth effects",
        "Ethnic",
        "Percussive",
        "Sound effects",
    ];
    FAMILIES[(program as usize / 8).min(15)]
}

/// From ticks to ms, following the file's tempo changes.
struct Clock {
    /// Ticks per beat, for metrical timing
    per_beat: u64,
    /// For timecode timing: ticks per second
    per_second: Option<u64>,
    /// (tick, µs per beat from there), by tick
    tempos: Vec<(u64, u64)>,
}

impl Clock {
    fn new(smf: &Smf) -> Clock {
        let mut tempos = vec![(0, 500_000)];
        for track in &smf.tracks {
            let mut tick = 0u64;
            for event in track {
                tick += u64::from(event.delta.as_int());
                if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = event.kind {
                    tempos.push((tick, u64::from(tempo.as_int()).max(1)));
                }
            }
        }
        // A later change at the same tick wins: sort keeps their order
        tempos.sort_by_key(|&(tick, _)| tick);
        match smf.header.timing {
            Timing::Metrical(per_beat) => Clock {
                per_beat: u64::from(per_beat.as_int()).max(1),
                per_second: None,
                tempos,
            },
            Timing::Timecode(fps, sub) => Clock {
                per_beat: 1,
                per_second: Some((fps.as_f32() * f32::from(sub)).max(1.0) as u64),
                tempos,
            },
        }
    }

    fn ms(&self, tick: u64) -> u32 {
        if let Some(per_second) = self.per_second {
            return (tick * 1000 / per_second).min(u64::from(u32::MAX)) as u32;
        }
        let mut micros = 0u64;
        for (i, &(from, tempo)) in self.tempos.iter().enumerate() {
            if from >= tick {
                break;
            }
            let to = self
                .tempos
                .get(i + 1)
                .map_or(tick, |&(next, _)| next.min(tick));
            micros += (to - from) * tempo / self.per_beat;
        }
        (micros / 1000).min(u64::from(u32::MAX)) as u32
    }

    /// `tick` rounded up to a whole beat.
    fn whole_beats(&self, tick: u64) -> u64 {
        tick.div_ceil(self.per_beat) * self.per_beat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file at 480 ticks per beat and 120 beats per minute (a beat is 500 ms): each note is
    /// (channel, key, start tick, end tick).
    fn file(tracks: &[&[(u8, u8, u32, u32)]]) -> Vec<u8> {
        let mut smf = Smf::new(Header::new(
            Format::Parallel,
            Timing::Metrical(u15::new(480)),
        ));
        for notes in tracks {
            let mut events: Vec<(u32, TrackEventKind)> = Vec::new();
            for &(channel, key, start, end) in notes.iter() {
                let (channel, key) = (u4::new(channel), u7::new(key));
                events.push((
                    start,
                    TrackEventKind::Midi {
                        channel,
                        message: MidiMessage::NoteOn {
                            key,
                            vel: u7::new(100),
                        },
                    },
                ));
                events.push((
                    end,
                    TrackEventKind::Midi {
                        channel,
                        message: MidiMessage::NoteOff {
                            key,
                            vel: u7::new(0),
                        },
                    },
                ));
            }
            events.sort_by_key(|&(tick, kind)| {
                // Offs before ons at the same tick
                let on = matches!(
                    kind,
                    TrackEventKind::Midi {
                        message: MidiMessage::NoteOn { .. },
                        ..
                    }
                );
                (tick, on)
            });
            let mut last = 0;
            let mut track: Vec<TrackEvent> = events
                .into_iter()
                .map(|(tick, kind)| {
                    let delta = tick - last;
                    last = tick;
                    TrackEvent {
                        delta: u28::new(delta),
                        kind,
                    }
                })
                .collect();
            track.push(TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            });
            smf.tracks.push(track);
        }
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn groups_by_channel_without_drums() {
        let bytes = file(&[
            &[(0, 72, 0, 480), (0, 74, 480, 960)],
            &[(1, 48, 0, 960), (DRUMS, 36, 0, 240)],
        ]);
        let song = read(&bytes).unwrap();
        assert_eq!(song.groups.len(), 2, "the drums are left out");
        assert_eq!(
            song.groups[0].notes[1],
            Timed {
                start: 500,
                end: 1000,
                key: 74
            }
        );
        assert_eq!(song.length_ms, 1000);
        assert_eq!(suggest(&song), (0, Some(1)));
        assert!(song.groups[1].label.contains("C3–C3"));
    }

    #[test]
    fn a_line_keeps_the_top_or_bottom_note_and_the_length() {
        // A chord of C and E for a beat, then a pause, then G: in a song of 2 beats and a bit
        let bytes = file(&[&[(0, 60, 0, 480), (0, 64, 0, 480), (0, 67, 600, 900)]]);
        let song = read(&bytes).unwrap();
        assert_eq!(song.length_ms, 1000, "rounded up to whole beats");
        let top = line(&song.groups[0], true, song.length_ms);
        assert_eq!(
            top,
            [
                Note::new(330, 500),
                Note::new(0, 125),
                Note::new(392, 312),
                Note::new(0, 63)
            ]
        );
        let bottom = line(&song.groups[0], false, song.length_ms);
        assert_eq!(bottom[0], Note::new(262, 500));
        let total = |notes: &[Note]| notes.iter().map(|n| u32::from(n.ms)).sum::<u32>();
        assert_eq!(total(&top), 1000);
        assert_eq!(total(&bottom), 1000);
    }

    #[test]
    fn repeated_notes_stay_separate() {
        let bytes = file(&[&[(0, 69, 0, 480), (0, 69, 480, 960)]]);
        let song = read(&bytes).unwrap();
        let notes = line(&song.groups[0], true, song.length_ms);
        assert_eq!(notes, [Note::new(440, 500), Note::new(440, 500)]);
    }

    #[test]
    fn follows_tempo_changes() {
        let original = file(&[&[(0, 60, 0, 960)]]);
        let mut smf = Smf::parse(&original).unwrap();
        // Twice as fast from the second beat on: the first beat takes 500 ms, the second 250
        smf.tracks[0].insert(
            1,
            TrackEvent {
                delta: u28::new(480),
                kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(250_000))),
            },
        );
        // The note's end comes 480 ticks after the tempo change now, not 960 after the start
        smf.tracks[0][2].delta = u28::new(480);
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        assert_eq!(read(&bytes).unwrap().groups[0].notes[0].end, 750);
    }

    #[test]
    fn written_music_reads_back_the_same() {
        let music = Music {
            parts: vec![
                adventure_format::Part {
                    wave: Wave::Square,
                    notes: vec![
                        Note::new(523, 300),
                        Note::new(659, 300),
                        Note::new(523, 300),
                        Note::new(0, 300),
                    ],
                },
                adventure_format::Part {
                    wave: Wave::Triangle,
                    notes: vec![Note::new(131, 600), Note::new(196, 600)],
                },
            ],
        };
        let song = read(&write(&music)).unwrap();
        assert_eq!(song.length_ms, 1200, "the rest at the end too");
        assert_eq!(suggest(&song), (0, Some(1)));
        assert!(song.groups[0].label.starts_with("Melody"));
        assert_eq!(
            line(&song.groups[0], true, song.length_ms),
            music.parts[0].notes
        );
        assert_eq!(
            line(&song.groups[1], false, song.length_ms),
            music.parts[1].notes
        );
    }

    #[test]
    fn refuses_what_isnt_music() {
        assert!(read(b"RIFF....").unwrap_err().contains("not a MIDI"));
        let drums_only = file(&[&[(DRUMS, 36, 0, 240)]]);
        assert!(read(&drums_only).unwrap_err().contains("no notes"));
    }
}
