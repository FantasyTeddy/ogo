//! Adventure: walk around in tile worlds, made for children who can't read yet.
//!
//! The worlds are `.adv` files in `/adventure/` on the SD card, which the web page can fill; without
//! any, the sample village built into adventure-engine. The start screen shows each world's picture
//! (Left/Right, A), then, if a world has more than one hero, the heroes. With one world it starts
//! right away. Progress is saved per world whenever something changes and when the pause menu
//! opens; the next start goes on from there, as the same hero.

use std::{fs, io::Read, path::PathBuf};

use adventure_engine::{sample, Buttons, Event, Game, Save, HEIGHT, WIDTH};
use adventure_format::{Dir, Head, World, TILE, TRANSPARENT};
use embedded_graphics::{pixelcolor::raw::RawU16, primitives::Rectangle};
use ogo_sdk::prelude::*;

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
    fn load(&self) -> anyhow::Result<World> {
        match &self.path {
            None => Ok(sample::world()),
            Some(path) => Ok(World::from_bytes(&fs::read(path)?)?),
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
        let mut app = Adventure {
            screen: Screen::Worlds { selected },
            worlds,
            picture: vec![0; WIDTH * HEIGHT],
            frame: 0,
        };
        if app.worlds.len() == 1 {
            app.open(ctx, 0);
        }
        app
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
                    match Game::new(*world, hero) {
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
    }

    fn area(&self) -> Option<Area> {
        Some(AREA)
    }

    fn paused(&mut self, ctx: &mut Ctx) {
        if let Screen::Playing(playing) = &mut self.screen {
            playing.save(ctx);
        }
    }
}

const MOVED: [Note; 1] = [Note::new(660, 40)];
const CHOSEN: [Note; 2] = [Note::new(660, 60), Note::new(990, 90)];
const REFUSED: [Note; 2] = [Note::new(220, 100), Note::new(165, 160)];
const BUMP: [Note; 1] = [Note::new(150, 40)];

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

/// The worlds on the SD card, by file name; the sample if there are none.
fn scan() -> Vec<Entry> {
    let mut paths: Vec<PathBuf> = fs::read_dir(WORLD_DIR)
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
    if paths.is_empty() {
        let world = sample::world();
        let head = Head {
            uid: world.uid,
            title: world.title,
            thumbnail: world.thumbnail,
        };
        return vec![Entry {
            path: None,
            head: Some(head),
        }];
    }
    paths
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
        .collect()
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
            Some(save) => Game::resume(world, &save),
            None if world.heroes.len() > 1 => {
                self.screen = Screen::Heroes {
                    world: Box::new(world),
                    selected: 0,
                };
                return;
            }
            None => Game::new(world, 0),
        };
        match game {
            Ok(game) => self.play(ctx, game),
            Err(error) => log::error!("can't start world {index}: {error}"),
        }
    }

    fn play(&mut self, ctx: &mut Ctx, game: Game) {
        let tracks = game.world().music.iter().map(track).collect();
        let mut playing = Playing {
            game,
            tracks,
            saved: 0,
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
                Event::Sound(sound) => {
                    let notes: Vec<Note> = self.game.world().sounds[sound.0 as usize]
                        .0
                        .iter()
                        .map(|note| Note::new(note.hz as u32, note.ms as u32))
                        .collect();
                    ctx.sound().play(&notes);
                }
                Event::Music(Some(music)) => ctx.sound().play_music(self.tracks[music.0 as usize]),
                Event::Music(None) => ctx.sound().stop_music(),
                Event::Bump => ctx.sound().play(&BUMP),
            }
        }
        if self.game.progress() != self.saved {
            self.save(ctx);
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

/// The space for the picture being chosen: centred, leaving room for the arrows and dots
const BOX: (usize, usize) = (120, 90);
const BOX_TOP: usize = 8;

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
            fill(out, x - 2, 106, 5, 5, FRAME);
        } else {
            fill(out, x - 1, 107, 3, 3, DOT);
        }
    }
}

fn fill(out: &mut [u16], x: usize, y: usize, w: usize, h: usize, colour: u16) {
    for row in y..(y + h).min(HEIGHT) {
        out[row * WIDTH + x..row * WIDTH + (x + w).min(WIDTH)].fill(colour);
    }
}

ogo_sdk::main!(Adventure);
