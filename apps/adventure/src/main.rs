//! Adventure: walk around in tile worlds, made for children who can't read yet.
//!
//! The worlds are `.adv` files in `/adventure/` on the SD card, which the web page can fill, and
//! the sample village built into adventure-engine after them. The start screen shows each world's
//! picture with its name (Left/Right, A), starting at the one played last, then, if a world has
//! more than one hero, the heroes. Progress is saved per world whenever something changes and when
//! the pause menu opens; the next start goes on from there, as the same hero.

use std::{fs, io::Read, path::PathBuf};

use adventure_engine::{sample, Buttons, Event, Game, Recordings, Save, HEIGHT, WIDTH};
use adventure_format::{Dir, Head, Sound, World, TILE, TRANSPARENT};
use embedded_graphics::{
    mono_font::{iso_8859_1::FONT_6X10, MonoTextStyle},
    pixelcolor::raw::RawU16,
    primitives::Rectangle,
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use ogo_sdk::prelude::*;
use ogo_sdk::{in_psram, Stream, SAMPLE_RATE};

const WORLD_DIR: &str = "/sd/adventure";
/// The save that remembers the world played last
const LAST: &str = "last";

const PICTURE: Size = Size::new(WIDTH as u32, HEIGHT as u32);
/// The 160x120 picture, doubled to fill the screen
const AREA: Area = Area::scaled(
    PICTURE,
    Rectangle::new(Point::zero(), Size::new(SCREEN_WIDTH, SCREEN_HEIGHT)),
);

const BACKGROUND: u16 = rgb(0x3a6ea5);
const FRAME: u16 = 0xFFFF;
const ARROW: u16 = rgb(0xffe066);
const DOT: u16 = rgb(0x9cc8ff);
const BROKEN: u16 = rgb(0x8a8a8a);
const CROSS: u16 = rgb(0xe33b3b);

/// `0xRRGGBB` as RGB565.
const fn rgb(hex: u32) -> u16 {
    let (r, g, b) = ((hex >> 16) & 0xFF, (hex >> 8) & 0xFF, hex & 0xFF);
    ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) as u16
}

/// A world to choose from.
struct Entry {
    /// None: the built-in sample
    path: Option<PathBuf>,
    /// None: the file can't be read
    head: Option<Head>,
}

impl Entry {
    /// The world in PSRAM: see [`in_psram`].
    fn load(&self) -> anyhow::Result<World> {
        match &self.path {
            None => Ok(in_psram(sample::world)),
            Some(path) => {
                let bytes = fs::read(path)?;
                Ok(in_psram(|| World::from_bytes(&bytes))?)
            }
        }
    }
}

enum Screen {
    Worlds { selected: usize },
    Heroes { world: Box<World>, selected: usize },
    Playing(Box<Playing>),
}

struct Playing {
    game: Game,
    /// The world's music, made `'static` for the speaker: built once per world played, and kept
    /// until the app exits
    tracks: Vec<Track>,
    saved: u32,
    recordings: Recordings,
    /// Open while recorded sounds play: the speaker stays on while one is open
    stream: Option<Stream>,
    /// Room for the samples handed to the stream
    samples: Vec<i16>,
}

struct Adventure {
    worlds: Vec<Entry>,
    screen: Screen,
    picture: Vec<u16>,
    frame: u32,
}

impl App for Adventure {
    const MUSIC: bool = true;

    fn new(ctx: &mut Ctx) -> Self {
        let worlds = scan();
        let last: Option<String> = ctx.saves().read(LAST);
        let selected = worlds
            .iter()
            .position(|entry| entry.head.as_ref().map(|head| save_name(head.uid)) == last)
            .unwrap_or(0);
        Adventure {
            screen: Screen::Worlds { selected },
            worlds,
            picture: vec![0; WIDTH * HEIGHT],
            frame: 0,
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        self.frame = self.frame.wrapping_add(1);
        match &mut self.screen {
            Screen::Worlds { selected } => {
                let count = self.worlds.len();
                let selected = *selected;
                if let Some(next) = browse(ctx, selected, count) {
                    self.screen = Screen::Worlds { selected: next };
                } else if ctx.input().pressed(Button::A) {
                    self.open(ctx, selected);
                }
            }
            Screen::Heroes { world, selected } => {
                if let Some(next) = browse(ctx, *selected, world.heroes.len()) {
                    *selected = next;
                } else if ctx.input().pressed(Button::A) {
                    let hero = *selected;
                    let Screen::Heroes { world, .. } =
                        std::mem::replace(&mut self.screen, Screen::Worlds { selected: 0 })
                    else {
                        unreachable!()
                    };
                    ctx.sound().play(&CHOSEN);
                    match in_psram(|| Game::new(*world, hero)) {
                        Ok(game) => self.play(ctx, game),
                        Err(error) => log::error!("can't start: {error}"),
                    }
                }
            }
            Screen::Playing(playing) => playing.update(ctx),
        }
        self.draw_picture();
    }

    fn draw(&self, fb: &mut Framebuffer) {
        let pixels = self.picture.iter().map(|&p| Rgb565::from(RawU16::new(p)));
        fb.fill_contiguous(&fb.bounding_box(), pixels).ok();
        // The world's name, small under its picture: for the grown-ups, and for learning letters
        if let Screen::Worlds { selected } = &self.screen {
            if let Some(head) = &self.worlds[*selected].head {
                let style = MonoTextStyle::new(&FONT_6X10, Rgb565::from(RawU16::new(FRAME)));
                let layout = TextStyleBuilder::new()
                    .alignment(Alignment::Center)
                    .baseline(Baseline::Top)
                    .build();
                let at = Point::new(WIDTH as i32 / 2, NAME_TOP as i32);
                Text::with_text_style(&caption(&head.title), at, style, layout)
                    .draw(fb)
                    .ok();
            }
        }
    }

    fn area(&self) -> Option<Area> {
        Some(AREA)
    }

    fn paused(&mut self, ctx: &mut Ctx) {
        if let Screen::Playing(playing) = &mut self.screen {
            playing.save(ctx);
            // An open stream keeps the speaker on, hissing, while the menu is open
            playing.recordings.stop();
            playing.stream = None;
        }
    }
}

const MOVED: [Note; 1] = [Note::new(660, 40)];
const CHOSEN: [Note; 2] = [Note::new(660, 60), Note::new(990, 90)];
const REFUSED: [Note; 2] = [Note::new(220, 100), Note::new(165, 160)];
const BUMP: [Note; 1] = [Note::new(150, 40)];
/// Samples the stream is kept ahead by (100 ms): it starts playing at 75 ms and drops the oldest
/// beyond 125 ms.
const STREAM_AHEAD: usize = SAMPLE_RATE as usize / 10;
/// Fewer left than this (10 ms), the stream waits for more that won't come: done.
const STREAM_DONE: usize = SAMPLE_RATE as usize / 100;

/// Left and Right through `count` choices, round and round.
fn browse(ctx: &mut Ctx, selected: usize, count: usize) -> Option<usize> {
    let input = ctx.input();
    let next = if input.pressed(Button::Left) {
        (selected + count - 1) % count
    } else if input.pressed(Button::Right) {
        (selected + 1) % count
    } else {
        return None;
    };
    if count > 1 {
        ctx.sound().play(&MOVED);
    }
    Some(next)
}

/// The worlds on the SD card, by file name, then the sample.
fn scan() -> Vec<Entry> {
    let mut paths: Vec<PathBuf> = fs::read_dir(WORLD_DIR)
        .inspect_err(|error| log::info!("{WORLD_DIR}: {error}"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or(".");
                    !name.starts_with('.') && name.to_ascii_lowercase().ends_with(".adv")
                })
                .collect()
        })
        .unwrap_or_default();
    paths.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    log::info!("{} worlds in {WORLD_DIR}", paths.len());
    let mut worlds: Vec<Entry> = paths
        .into_iter()
        .map(|path| {
            let head = read_head(&path)
                .inspect_err(|error| log::warn!("{}: {error}", path.display()))
                .ok();
            Entry {
                path: Some(path),
                head,
            }
        })
        .collect();
    let sample = sample::world();
    worlds.push(Entry {
        path: None,
        head: Some(Head {
            uid: sample.uid,
            title: sample.title,
            thumbnail: sample.thumbnail,
        }),
    });
    worlds
}

fn read_head(path: &PathBuf) -> anyhow::Result<Head> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(Head::MAX_BYTES as u64)
        .read_to_end(&mut bytes)?;
    Ok(Head::from_bytes(&bytes)?)
}

fn save_name(uid: u64) -> String {
    format!("{uid:016x}")
}

impl Adventure {
    /// Opens world `index`: goes on with its save, or lets the player choose a hero.
    fn open(&mut self, ctx: &mut Ctx, index: usize) {
        let world = match self.worlds[index].load() {
            Ok(world) => world,
            Err(error) => {
                log::error!("can't open world {index}: {error}");
                self.worlds[index].head = None;
                ctx.sound().play(&REFUSED);
                return;
            }
        };
        ctx.sound().play(&CHOSEN);
        let name = save_name(world.uid);
        if let Err(error) = ctx.saves().write(LAST, &name) {
            log::warn!("can't remember the world: {error}");
        }
        let save = ctx
            .saves()
            .read_bytes(&name)
            .and_then(|bytes| Save::from_bytes(&bytes));
        let game = match save {
            Some(save) => in_psram(|| Game::resume(world, &save)),
            None if world.heroes.len() > 1 => {
                self.screen = Screen::Heroes {
                    world: Box::new(world),
                    selected: 0,
                };
                return;
            }
            None => in_psram(|| Game::new(world, 0)),
        };
        match game {
            Ok(game) => self.play(ctx, game),
            Err(error) => log::error!("can't start world {index}: {error}"),
        }
    }

    fn play(&mut self, ctx: &mut Ctx, game: Game) {
        let tracks = in_psram(|| game.world().music.iter().map(track).collect());
        let mut playing = Playing {
            game,
            tracks,
            saved: 0,
            recordings: Recordings::default(),
            stream: None,
            samples: Vec::new(),
        };
        playing.save(ctx);
        self.screen = Screen::Playing(Box::new(playing));
    }

    fn draw_picture(&mut self) {
        let out = &mut self.picture;
        match &self.screen {
            Screen::Playing(playing) => playing.game.draw(out),
            Screen::Worlds { selected } => {
                out.fill(BACKGROUND);
                let thumbnail = self.worlds[*selected]
                    .head
                    .as_ref()
                    .map(|head| &head.thumbnail);
                match thumbnail {
                    Some(picture) => framed(
                        out,
                        &picture.pixels.0,
                        picture.width as usize,
                        picture.height as usize,
                    ),
                    None => broken(out),
                }
                if self.worlds.len() > 1 {
                    arrows(out);
                }
                dots(out, *selected, self.worlds.len());
            }
            Screen::Heroes { world, selected } => {
                out.fill(BACKGROUND);
                let sprite = &world.sprites[world.heroes[*selected].0 as usize];
                // The chosen one walks on the spot: standing, a step, standing, the other step
                let beat = self.frame as usize / 8;
                let frame = match sprite.frames as usize {
                    frames if frames > 1 && beat % 2 == 1 => 1 + (beat / 2) % (frames - 1),
                    _ => 0,
                };
                let pixels = sprite.picture(Dir::Down, frame);
                framed(out, pixels, TILE, TILE);
                if world.heroes.len() > 1 {
                    arrows(out);
                }
                dots(out, *selected, world.heroes.len());
            }
        }
    }
}

impl Playing {
    fn update(&mut self, ctx: &mut Ctx) {
        let input = ctx.input();
        let buttons = Buttons {
            up: input.held(Button::Up),
            down: input.held(Button::Down),
            left: input.held(Button::Left),
            right: input.held(Button::Right),
            a: input.held(Button::A),
            b: input.held(Button::B),
        };
        for event in self.game.step(buttons) {
            match event {
                Event::Sound(sound) => match &self.game.world().sounds[sound.0 as usize] {
                    Sound::Notes(notes) => {
                        let notes: Vec<Note> = notes
                            .iter()
                            .map(|note| Note::new(note.hz as u32, note.ms as u32))
                            .collect();
                        ctx.sound().play(&notes);
                    }
                    Sound::Samples(_) => self.recordings.start(&self.game.world().sounds, sound),
                },
                Event::Music(Some(music)) => ctx.sound().play_music(self.tracks[music.0 as usize]),
                Event::Music(None) => ctx.sound().stop_music(),
                Event::Bump => ctx.sound().play(&BUMP),
            }
        }
        self.feed(ctx);
        if self.game.progress() != self.saved {
            self.save(ctx);
        }
    }

    /// Keeps the stream about [`STREAM_AHEAD`] ahead while recorded sounds play, and closes it once
    /// what's left has played.
    fn feed(&mut self, ctx: &mut Ctx) {
        if self.recordings.is_playing() {
            let stream = self.stream.get_or_insert_with(|| ctx.sound().open_stream());
            let missing = STREAM_AHEAD.saturating_sub(stream.buffered());
            if missing > 0 {
                self.samples.resize(missing, 0);
                let sounds = &self.game.world().sounds;
                self.recordings.fill(sounds, SAMPLE_RATE, &mut self.samples);
                stream.push(&self.samples);
            }
        } else if self
            .stream
            .as_ref()
            .is_some_and(|stream| stream.buffered() < STREAM_DONE)
        {
            self.stream = None;
        }
    }

    fn save(&mut self, ctx: &mut Ctx) {
        let save = self.game.save();
        match ctx
            .saves()
            .write_bytes(&save_name(save.uid), &save.to_bytes())
        {
            Ok(()) => self.saved = self.game.progress(),
            Err(error) => log::warn!("can't save: {error}"),
        }
    }
}

/// A world's music for the speaker, which takes `'static` notes.
fn track(music: &adventure_format::Music) -> Track {
    let parts: Vec<Part> = music
        .parts
        .iter()
        .map(|part| Part {
            wave: match part.wave {
                adventure_format::Wave::Sine => Wave::Sine,
                adventure_format::Wave::Square => Wave::Square,
                adventure_format::Wave::Triangle => Wave::Triangle,
            },
            notes: Vec::leak(
                part.notes
                    .iter()
                    .map(|note| Note::new(note.hz as u32, note.ms as u32))
                    .collect(),
            ),
        })
        .collect();
    // Validated by adventure-format: 1 or 2 parts of equal length
    Track::new(Vec::leak(parts))
}

// ---- Drawing the choices ---------------------------------------------------------------------

/// The space for the picture being chosen: centred, leaving room for the arrows, the name and the
/// dots
const BOX: (usize, usize) = (120, 84);
const BOX_TOP: usize = 5;
/// The top of the world's name, under its picture
const NAME_TOP: usize = 96;
/// The most characters of a name that fit across the screen, 6 pixels each
const NAME_CHARS: usize = 26;

/// `title` as it fits across the screen: cut short, with dots, if it's too long.
fn caption(title: &str) -> String {
    if title.chars().count() <= NAME_CHARS {
        title.to_string()
    } else {
        title
            .chars()
            .take(NAME_CHARS - 3)
            .collect::<String>()
            .trim_end()
            .to_string()
            + "..."
    }
}

/// A picture as large as fits in [`BOX`] (keeping its shape), with a white frame.
fn framed(out: &mut [u16], pixels: &[u16], width: usize, height: usize) {
    let scale = (BOX.0 * 256 / width).min(BOX.1 * 256 / height);
    let (w, h) = (width * scale / 256, height * scale / 256);
    let (x, y) = ((WIDTH - w) / 2, BOX_TOP + (BOX.1 - h) / 2);
    fill(out, x - 2, y - 2, w + 4, h + 4, FRAME);
    fill(out, x, y, w, h, BACKGROUND);
    for row in 0..h {
        for col in 0..w {
            let pixel = pixels[row * height / h * width + col * width / w];
            if pixel != TRANSPARENT {
                out[(y + row) * WIDTH + x + col] = pixel;
            }
        }
    }
}

/// In place of a world that can't be read: a grey box with a red cross.
fn broken(out: &mut [u16]) {
    let (w, h) = (80, 60);
    let (x, y) = ((WIDTH - w) / 2, BOX_TOP + (BOX.1 - h) / 2);
    fill(out, x - 2, y - 2, w + 4, h + 4, FRAME);
    fill(out, x, y, w, h, BROKEN);
    for i in 0..40 {
        for t in 0..3 {
            let (cx, cy) = (x + 20 + i, y + 10 + i);
            out[(cy) * WIDTH + cx + t] = CROSS;
            out[(cy) * WIDTH + x + 60 - i + t - 1] = CROSS;
        }
    }
}

/// Triangles left and right: there's more to choose from.
fn arrows(out: &mut [u16]) {
    let middle = BOX_TOP + BOX.1 / 2;
    for i in 0..8 {
        let height = 2 * i + 1;
        fill(out, 4 + i, middle - i, 1, height, ARROW);
        fill(out, WIDTH - 5 - i, middle - i, 1, height, ARROW);
    }
}

/// One dot per choice, the chosen one larger.
fn dots(out: &mut [u16], selected: usize, count: usize) {
    if count < 2 {
        return;
    }
    let spacing = 8;
    let shown = count.min(WIDTH / spacing - 2);
    let left = (WIDTH - (shown - 1) * spacing) / 2;
    for i in 0..shown {
        let x = left + i * spacing;
        if i == selected {
            fill(out, x - 2, 109, 5, 5, FRAME);
        } else {
            fill(out, x - 1, 110, 3, 3, DOT);
        }
    }
}

fn fill(out: &mut [u16], x: usize, y: usize, w: usize, h: usize, colour: u16) {
    for row in y..(y + h).min(HEIGHT) {
        out[row * WIDTH + x..row * WIDTH + (x + w).min(WIDTH)].fill(colour);
    }
}

ogo_sdk::main!(Adventure);
