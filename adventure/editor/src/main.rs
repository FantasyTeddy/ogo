//! The Adventure editor: makes worlds on the PC. A world is a project folder (see [`project`]);
//! the editor paints its maps with the tiles of a PNG tileset, plays it with the engine, and
//! exports it as an `.adv` file or sends it straight to the device.
//!
//! `cargo run --manifest-path adventure/Cargo.toml --target host-tuple -p adventure-editor
//! [-- <folder>]`: without a folder, the one open last time.

mod audio;
mod export;
mod handheld;
mod images;
mod midi;
mod project;
mod send;
mod sound;
mod theme;
mod things;

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use adventure_engine::{Buttons, Event, Game, FPS, HEIGHT, WIDTH};
use adventure_format::{Dir, LayerKind, MusicRef, Note, Place, Pos, SoundRef, TILE, TRANSPARENT};
use eframe::egui::{
    self, load::SizedTexture, pos2, vec2, Align, Align2, Color32, ColorImage, CornerRadius, Frame,
    Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Popup, PopupCloseBehavior, Rect,
    RectAlign, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, UiBuilder,
};

use export::Assets;
use handheld::{Handheld, HandheldButton};
use images::Rgba;
use project::{Brush, Project, SoundSource, TileDef};
use theme::*;
use things::{Stamp, Trigger};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 840.0])
            .with_min_inner_size([980.0, 620.0])
            .with_title("Adventure editor"),
        ..Default::default()
    };
    eframe::run_native(
        "Adventure editor",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(Editor::new(cc)))
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    // For tiles
    Pencil,
    Fill,
    Box,
    Eraser,
    // For objects
    Select,
    Place,
    Remove,
    Start,
}

const OBJECT_TOOLS: [(Tool, &str, &str, &str); 4] = [
    (
        Tool::Select,
        CURSOR,
        "Select",
        "Click an object to select it; drag to move it",
    ),
    (
        Tool::Place,
        STAMP,
        "Place",
        "Click an empty cell to place what's chosen on the left",
    ),
    (
        Tool::Remove,
        TRASH,
        "Remove",
        "Click an object to remove it",
    ),
    (Tool::Start, FLAG, "Start", "Click where the player starts"),
];

const TILE_TOOLS: [(Tool, &str, &str, &str); 4] = [
    (
        Tool::Pencil,
        PENCIL,
        "Pencil",
        "Paint the selected tile on its layer. Right-click picks a tile up",
    ),
    (
        Tool::Fill,
        FILL,
        "Fill",
        "Fill the area of same tiles on the tile's layer",
    ),
    (Tool::Box, BOX, "Box", "Drag to fill a rectangle"),
    (
        Tool::Eraser,
        ERASER,
        "Eraser",
        "Remove the top tile, or the one on the layer clicked below; a drag keeps to one layer",
    ),
];

/// The layers, as the layer bar and a tile's settings name them.
const LAYERS: [(LayerKind, &str, &str); 3] = [
    (
        LayerKind::Ground,
        "Ground",
        "Grass, paths, water: drawn first",
    ),
    (
        LayerKind::Decoration,
        "Decoration",
        "Trunks, fences, flowers: on the ground, with the characters",
    ),
    (
        LayerKind::Above,
        "Above",
        "Treetops, roofs: drawn over everyone, the player too; never in the way",
    ),
];

fn layer_name(kind: LayerKind) -> &'static str {
    LAYERS[kind as usize].1
}

/// What the left panel offers and clicking the map does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Tiles,
    Things,
}

/// A door's target being picked on the map: for which action of which thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pick {
    /// The map of the thing
    map: usize,
    thing: usize,
    trigger: Trigger,
    rule: usize,
    action: usize,
}

/// A dialog window over the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialog {
    World(Section),
    Map(usize),
}

/// A section of the world's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Pictures,
    Sounds,
    Items,
}

/// What a click in the world's settings wants to hear.
enum Preview {
    Sound(usize),
    Music(usize),
    Stop,
}

/// What to do once the unsaved changes are saved or dropped.
#[derive(Debug, Clone, Copy)]
enum Pending {
    New,
    NewEmpty,
    Open,
    Close,
}

/// How a map is drawn into a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// For painting: how each layer shows
    Painting([Show; 3]),
    /// Everything, for the map's tab
    Whole,
    /// For placing objects: the tiles darker, the objects bright
    Objects,
}

/// How a layer shows while painting: the eye in the layer bar goes through these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Show {
    Shown,
    /// See-through, so the others stand out
    Faint,
    /// Not drawn, not erased, not picked up from
    Off,
}

impl Show {
    fn next(self) -> Show {
        match self {
            Show::Shown => Show::Faint,
            Show::Faint => Show::Off,
            Show::Off => Show::Shown,
        }
    }
}

impl View {
    fn code(self) -> u8 {
        match self {
            // Two bits a layer: 0 to 42
            View::Painting(shows) => (0..3).map(|i| (shows[i] as u8) << (2 * i)).sum(),
            View::Whole => 100,
            View::Objects => 101,
        }
    }

    fn show(self, kind: LayerKind) -> Show {
        match self {
            View::Painting(shows) => shows[kind as usize],
            _ => Show::Shown,
        }
    }
}

/// An open project.
struct Open {
    dir: PathBuf,
    project: Project,
    assets: Assets,
    /// As last saved
    saved: Project,
    /// What undo goes back to: the project after the last finished change
    committed: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
    tileset: Option<TextureHandle>,
    /// Each sprite's first picture: its texture and where in it
    sprite_pictures: Vec<Option<(TextureHandle, Rect)>>,
    /// Pictures of maps, by (map, view), with what they were made from
    pictures: HashMap<(usize, u8), (u64, TextureHandle)>,
    /// Counts reloads of the pictures
    generation: u64,
    /// When the picture files were last looked at, and their times then
    checked: Instant,
    times: Vec<Option<SystemTime>>,
}

struct Play {
    game: Game,
    title: String,
    picture: Vec<u16>,
    texture: Option<TextureHandle>,
    last: Instant,
    /// Time not stepped yet, in seconds
    behind: f64,
    /// None when the PC has no sound output: the game plays silently
    speaker: Option<sound::Speaker>,
}

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

/// How tall the segments in the bars around the map are.
const BAR_HEIGHT: f32 = 30.0;

/// A rounded strip that holds segments, like the tools or the zoom: as round as the segments, 3
/// further out, so the corners run alongside each other.
fn pill() -> Frame {
    const MARGIN: u8 = 3;
    Frame::new()
        .fill(PANEL2)
        .corner_radius(PILL + MARGIN)
        .inner_margin(Margin::same(MARGIN as i8))
}

/// A block picked from the tileset: its corners there (column, row), and what it paints.
struct Block {
    from: (usize, usize),
    to: (usize, usize),
    brush: Brush,
}

struct Editor {
    open: Option<Open>,
    map: usize,
    /// The layers turned off, by layer: not drawn, not erased, not picked up from
    shows: [Show; 3],
    /// The layer clicked in the layer bar, which painting and erasing keep to until another tile
    /// is chosen; None: each tile goes on its own
    layer: Option<LayerKind>,
    /// The layer the stroke being erased keeps to
    erasing: Option<LayerKind>,
    tool: Tool,
    tile: usize,
    /// A block of tiles picked from the tileset, painted instead of the one tile
    block: Option<Block>,
    /// Where the stroke being painted began, which blocks line up with
    stroke_from: Option<Pos>,
    /// The cell of the tileset where a drag across the palette began
    sheet_drag: Option<(usize, usize)>,
    dialog: Option<Dialog>,
    /// The map tab under the mouse last frame, which shows its "⋯"
    tab_hover: Option<usize>,
    /// The world's picture for its settings, made when they open
    thumbnail: Option<TextureHandle>,
    grid: bool,
    zoom: f32,
    /// Where a rectangle being drawn started
    rect_from: Option<Pos>,
    /// The cell clicked last, where "Play from the cell clicked last" starts
    cursor: Option<Pos>,
    /// The size being typed in for the map: (map, width, height)
    size: Option<(usize, u16, u16)>,
    play: Option<Play>,
    toasts: Vec<Toast>,
    /// The device's address, as typed in the Send popup
    device: String,
    /// The PIN its WiFi screen shows, as typed
    pin: String,
    /// Logged in to the device, after Connect
    session: Option<send::Session>,
    connecting: Option<mpsc::Receiver<Result<send::Session, String>>>,
    /// Why the last Connect failed
    connect_error: Option<String>,
    sending: Option<mpsc::Receiver<Result<String, send::Failure>>>,
    pending: Option<Pending>,
    closing: bool,
    /// Where the open map is on the screen, for the zoom and the messages
    sheet: Rect,
    /// Pick the largest zoom that shows the whole map, next time it's drawn
    fit: bool,
    mode: Mode,
    /// The selected thing on the current map
    thing: Option<usize>,
    stamp: Stamp,
    /// A thing being dragged to another cell
    dragging: Option<usize>,
    picking: Option<Pick>,
    /// Plays the sounds and music listened to in the world's settings; opened on the first
    preview: Option<sound::Speaker>,
    /// The music being listened to there
    previewing: Option<usize>,
    /// The song whose parts show there
    song_open: Option<usize>,
}

const UNDO_STEPS: usize = 200;
/// Walking into something that doesn't react, as on the device.
const BUMP: [Note; 1] = [Note::new(150, 40)];
const CHECK_PICTURES: Duration = Duration::from_secs(1);
/// Where the device's address and PIN are typed, the first time Send is clicked.
const CONNECT_POPUP: &str = "connect";

impl Editor {
    fn new(cc: &eframe::CreationContext) -> Self {
        let stored = |key: &str| cc.storage.and_then(|s| s.get_string(key));
        let mut editor = Editor {
            open: None,
            map: 0,
            shows: [Show::Shown; 3],
            layer: None,
            erasing: None,
            tool: Tool::Pencil,
            tile: 0,
            block: None,
            stroke_from: None,
            sheet_drag: None,
            dialog: None,
            tab_hover: None,
            thumbnail: None,
            grid: true,
            zoom: 2.0,
            rect_from: None,
            cursor: None,
            size: None,
            play: None,
            toasts: Vec::new(),
            device: stored("device").unwrap_or_default(),
            pin: String::new(),
            session: None,
            connecting: None,
            connect_error: None,
            sending: None,
            pending: None,
            closing: false,
            sheet: Rect::NOTHING,
            fit: true,
            mode: Mode::Tiles,
            thing: None,
            stamp: Stamp::Sprite(adventure_format::SpriteRef(0)),
            dragging: None,
            picking: None,
            preview: None,
            previewing: None,
            song_open: None,
        };
        // The folder given on the command line, or the one open last time
        let folder = std::env::args().nth(1).or_else(|| stored("project"));
        if let Some(dir) = folder.map(PathBuf::from) {
            if dir.join(project::FILE).exists() {
                editor.open_dir(&dir);
            }
        }
        editor
    }

    /// What decides how the window looks, to notice when a frame changed it.
    fn view(&self) -> impl PartialEq {
        (
            (self.map, self.tile, self.shows, self.layer, self.tool as u8),
            (self.dialog, self.zoom as u8, self.grid, self.play.is_some()),
            (self.mode, self.thing, self.stamp, self.picking),
            (
                self.open.is_some(),
                self.pending.is_some(),
                self.dirty(),
                self.toasts.len(),
            ),
        )
    }

    fn say(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast {
            text: message.into(),
            error: false,
            at: Instant::now(),
        });
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast {
            text: message.into(),
            error: true,
            at: Instant::now(),
        });
    }

    fn dirty(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.project != open.saved)
    }

    // ---- Projects ---------------------------------------------------------------------------

    fn open_dir(&mut self, dir: &Path) {
        let project = match Project::load(dir) {
            Ok(project) => project,
            Err(error) => return self.fail(error),
        };
        let assets = Assets::load(dir, &project);
        let times = picture_times(dir, &project);
        self.map = project.map_index(project.start.map).unwrap_or(0);
        self.tile = 0;
        self.block = None;
        self.layer = None;
        self.dialog = None;
        self.cursor = None;
        self.size = None;
        self.play = None;
        self.fit = true;
        let name = dir
            .file_name()
            .map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into());
        self.say(format!("Opened {name}"));
        if let Err(error) = &assets.tileset {
            self.fail(error.clone());
        }
        self.open = Some(Open {
            dir: dir.to_path_buf(),
            saved: project.clone(),
            committed: project.clone(),
            project,
            assets,
            undo: Vec::new(),
            redo: Vec::new(),
            tileset: None,
            sprite_pictures: Vec::new(),
            pictures: HashMap::new(),
            generation: 0,
            checked: Instant::now(),
            times,
        });
    }

    /// Asks about unsaved changes first, if there are any.
    fn ask(&mut self, then: Pending, ctx: &egui::Context) {
        if self.dirty() {
            self.pending = Some(then);
        } else {
            self.proceed(then, ctx);
        }
    }

    fn proceed(&mut self, then: Pending, ctx: &egui::Context) {
        match then {
            Pending::New | Pending::NewEmpty => {
                // A save dialog, so a new name can be typed in: the world gets a folder of that name
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Name of the new world")
                    .set_file_name("My world");
                if let Some(parent) = self.open.as_ref().and_then(|open| open.dir.parent()) {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(dir) = dialog.save_file() {
                    let written = match then {
                        Pending::New => export::write_starter(&dir),
                        _ => export::write_empty(&dir),
                    };
                    match written {
                        Ok(_) => self.open_dir(&dir),
                        Err(error) => self.fail(error),
                    }
                }
            }
            Pending::Open => {
                let folder = rfd::FileDialog::new()
                    .set_title("The folder of a world (with world.json)")
                    .pick_folder();
                if let Some(dir) = folder {
                    self.open_dir(&dir);
                }
            }
            Pending::Close => {
                self.closing = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn save(&mut self) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        match open.project.save(&open.dir) {
            Ok(()) => {
                open.saved = open.project.clone();
                self.say("Saved");
                true
            }
            Err(error) => {
                self.fail(error);
                false
            }
        }
    }

    /// Takes a finished change into the undo history: once no mouse button is down, so that a
    /// whole stroke of painting is one step.
    fn commit(&mut self, ctx: &egui::Context) {
        let Some(open) = &mut self.open else { return };
        if ctx.input(|i| i.pointer.any_down()) || open.project == open.committed {
            return;
        }
        let before = std::mem::replace(&mut open.committed, open.project.clone());
        open.undo.push(before);
        if open.undo.len() > UNDO_STEPS {
            open.undo.remove(0);
        }
        open.redo.clear();
    }

    fn undo(&mut self) {
        let Some(open) = &mut self.open else { return };
        if let Some(before) = open.undo.pop() {
            let files = files(&open.project);
            open.redo.push(std::mem::replace(&mut open.project, before));
            open.committed = open.project.clone();
            if files != self::files(&open.project) {
                reload(open);
            }
        }
        self.keep_selection_valid();
    }

    fn redo(&mut self) {
        let Some(open) = &mut self.open else { return };
        if let Some(after) = open.redo.pop() {
            let files = files(&open.project);
            open.undo.push(std::mem::replace(&mut open.project, after));
            open.committed = open.project.clone();
            if files != self::files(&open.project) {
                reload(open);
            }
        }
        self.keep_selection_valid();
    }

    fn keep_selection_valid(&mut self) {
        if let Some(open) = &self.open {
            self.map = self.map.min(open.project.maps.len().saturating_sub(1));
            self.tile = self.tile.min(open.project.tiles.len().saturating_sub(1));
            let tiles = open.project.tiles.len() as u16;
            if self
                .block
                .as_ref()
                .is_some_and(|b| b.brush.tiles.iter().any(|&t| t > tiles))
            {
                self.block = None;
            }
            let things = open
                .project
                .maps
                .get(self.map)
                .map_or(0, |m| m.things.len());
            self.thing = self.thing.filter(|&i| i < things);
            self.picking = self.picking.filter(|p| p.map < open.project.maps.len());
        }
    }

    /// Reloads the pictures when their files changed, e.g. after painting in another program.
    fn reload_changed_pictures(&mut self) {
        let Some(open) = &mut self.open else { return };
        if open.checked.elapsed() < CHECK_PICTURES {
            return;
        }
        open.checked = Instant::now();
        let times = picture_times(&open.dir, &open.project);
        if times != open.times {
            open.times = times;
            reload(open);
            self.say("Changed files reloaded");
        }
    }

    /// Adds a PNG's 16×16 tiles to the tileset, and a tile for each one that isn't empty.
    fn add_tiles(&mut self) {
        let Some(open) = &mut self.open else { return };
        let Some(file) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_title("Tiles to add: 16x16 side by side")
            .pick_file()
        else {
            return;
        };
        let extra = match Rgba::load(&file) {
            Ok(image) if image.width % TILE == 0 && image.height % TILE == 0 && image.width > 0 => {
                image
            }
            Ok(image) => {
                return self.fail(format!(
                    "{} is {}x{}: tiles need a picture of whole 16x16 cells",
                    file.display(),
                    image.width,
                    image.height
                ))
            }
            Err(error) => return self.fail(error),
        };
        // An empty tileset (a new, empty world's) is replaced rather than added to
        let base = open
            .assets
            .tileset
            .as_ref()
            .ok()
            .filter(|t| t.pixels.iter().any(|p| p[3] >= 128))
            .cloned()
            .unwrap_or_else(|| Rgba::new(0, 0));
        let (both, cells) = images::append_cells(&base, &extra);
        if let Err(error) = both.save(&open.dir.join(project::TILESET)) {
            return self.fail(error);
        }
        // "tilemap_packed" → "Tilemap packed"
        let stem = file.file_stem().map_or("Tile".into(), |s| {
            s.to_string_lossy().replace(['_', '-'], " ")
        });
        let stem = stem
            .get(0..1)
            .map_or(stem.clone(), |c| c.to_uppercase() + &stem[1..]);
        let mut added = 0;
        for cell in cells {
            let pixels = both.cell(cell as usize);
            let empty = pixels
                .as_ref()
                .is_none_or(|pixels| pixels.iter().all(|&p| p == TRANSPARENT));
            if !empty {
                added += 1;
                open.project.tiles.push(TileDef {
                    name: format!("{stem} {added}"),
                    layer: project::layer_for(pixels.as_deref()),
                    cells: vec![cell],
                    frame_ms: 300,
                    solid: false,
                    step_sound: None,
                });
            }
        }
        reload(open);
        open.times = picture_times(&open.dir, &open.project);
        self.say(format!("Added {added} tiles"));
    }

    /// Copies PNGs into sprites/ and adds them as pictures; the first becomes the player if there
    /// isn't one yet.
    fn add_pictures(&mut self) {
        let Some(open) = &mut self.open else { return };
        let Some(files) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_title("Pictures: 16x16, or rows of them for directions and steps")
            .pick_files()
        else {
            return;
        };
        let folder = open.dir.join(project::SPRITES);
        if let Err(error) = std::fs::create_dir_all(&folder) {
            return self.fail(format!("Can't create {}: {error}", folder.display()));
        }
        let first = open.project.sprites.len();
        for file in &files {
            let (name, pretty) = match copy_in(file, &folder, "png") {
                Ok(names) => names,
                Err(error) => return self.fail(error),
            };
            open.project.sprites.push(project::SpriteDef {
                name: pretty,
                file: name,
            });
        }
        reload(open);
        open.times = picture_times(&open.dir, &open.project);
        let problems: Vec<String> = (first..open.project.sprites.len())
            .filter_map(|i| export::sprite(&open.project.sprites[i], &open.assets.sprites[i]).err())
            .collect();
        let mut message = format!("Added {} pictures", files.len());
        if open.project.heroes.is_empty() && first < open.project.sprites.len() {
            open.project.heroes = vec![adventure_format::SpriteRef(first as u16)];
            message += &format!("; {} is the player", open.project.sprites[first].name);
        }
        self.say(message);
        for problem in problems {
            self.fail(problem);
        }
    }

    /// Copies WAVs into sounds/ and adds them as sounds.
    fn add_sounds(&mut self) {
        let Some(open) = &mut self.open else { return };
        let Some(files) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_title("Sounds: WAV files, at most 4 seconds")
            .pick_files()
        else {
            return;
        };
        let folder = open.dir.join(project::SOUNDS);
        if let Err(error) = std::fs::create_dir_all(&folder) {
            return self.fail(format!("Can't create {}: {error}", folder.display()));
        }
        let first = open.project.sounds.len();
        for file in &files {
            let (name, pretty) = match copy_in(file, &folder, "wav") {
                Ok(names) => names,
                Err(error) => return self.fail(error),
            };
            open.project.sounds.push(project::SoundDef {
                name: pretty,
                source: SoundSource::File { file: name },
            });
        }
        reload(open);
        open.times = picture_times(&open.dir, &open.project);
        let problems: Vec<String> = (first..open.project.sounds.len())
            .filter_map(|i| export::sound(&open.project.sounds[i], &open.assets.sounds[i]).err())
            .collect();
        self.say(format!("Added {} sounds", files.len()));
        for problem in problems {
            self.fail(problem);
        }
    }

    /// Plays what was clicked in the world's settings.
    fn listen(&mut self, preview: Preview) {
        let Some(open) = &self.open else { return };
        if self.preview.is_none() {
            match sound::Speaker::open() {
                Ok(speaker) => self.preview = Some(speaker),
                Err(error) => return self.fail(format!("Can't play sound: {error}")),
            }
        }
        let Some(speaker) = &self.preview else { return };
        match preview {
            Preview::Sound(i) => {
                let sound = open
                    .assets
                    .sounds
                    .get(i)
                    .map(|s| export::sound(&open.project.sounds[i], s));
                if let Some(Ok(sound)) = sound {
                    speaker.preview(sound);
                }
            }
            Preview::Music(i) => {
                let music = open
                    .project
                    .music
                    .get(i)
                    .zip(open.assets.songs.get(i))
                    .map(|(def, song)| export::music(def, song));
                match music {
                    Some(Ok(music)) => {
                        speaker.music(Some(&music));
                        self.previewing = Some(i);
                    }
                    Some(Err(error)) => self.fail(error),
                    None => {}
                }
            }
            Preview::Stop => {
                speaker.music(None);
                self.previewing = None;
            }
        }
    }

    /// Copies a MIDI file into music/ and adds it as a song, with a melody and a bass suggested;
    /// its parts show, to change them.
    fn import_midi(&mut self) {
        let Some(open) = &mut self.open else { return };
        let Some(file) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .set_title("A song: a MIDI file")
            .pick_file()
        else {
            return;
        };
        let song = match export::SongFile::load(&file) {
            Ok(song) => song.song,
            Err(error) => return self.fail(format!("Can't import: {error}")),
        };
        let folder = open.dir.join(project::MUSIC);
        if let Err(error) = std::fs::create_dir_all(&folder) {
            return self.fail(format!("Can't create {}: {error}", folder.display()));
        }
        let (name, pretty) = match copy_in(&file, &folder, "mid") {
            Ok(names) => names,
            Err(error) => return self.fail(error),
        };
        let (melody, bass) = midi::suggest(&song);
        open.project.music.push(project::MusicDef {
            name: pretty.clone(),
            file: name,
            melody,
            bass,
            melody_wave: adventure_format::Wave::Square,
            bass_wave: adventure_format::Wave::Triangle,
        });
        reload(open);
        open.times = picture_times(&open.dir, &open.project);
        self.song_open = Some(open.project.music.len() - 1);
        self.say(format!("Added {pretty}: choose it in a map's settings"));
    }

    fn import_tileset(&mut self) {
        let Some(open) = &mut self.open else { return };
        let Some(file) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_title("A tileset: 16x16 tiles side by side")
            .pick_file()
        else {
            return;
        };
        let target = open.dir.join(project::TILESET);
        if let Err(error) = std::fs::copy(&file, &target) {
            return self.fail(format!("Can't copy to {}: {error}", target.display()));
        }
        reload(open);
        open.times = picture_times(&open.dir, &open.project);
        match &open.assets.tileset {
            Ok(_) => self.say("Tileset imported"),
            Err(error) => {
                let error = error.clone();
                self.fail(error);
            }
        }
    }

    // ---- Export, send, play -----------------------------------------------------------------

    fn world(&mut self) -> Option<adventure_format::World> {
        let open = self.open.as_ref()?;
        match export::world(&open.project, &open.assets) {
            Ok(world) => Some(world),
            Err(error) => {
                self.fail(format!("The world isn't ready: {error}"));
                None
            }
        }
    }

    fn export(&mut self) {
        let Some(world) = self.world() else { return };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Adventure world", &["adv"])
            .set_file_name(send::file_name(&world.title))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, world.to_bytes()) {
            Ok(()) => self.say(format!("Exported {}", path.display())),
            Err(error) => self.fail(format!("Can't write {}: {error}", path.display())),
        }
    }

    fn send(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(world) = self.world() else { return };
        let (name, bytes) = (send::file_name(&world.title), world.to_bytes());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(send::send(&session, &name, &bytes));
        });
        self.sending = Some(rx);
        self.say("Sending…");
    }

    fn poll_send(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.sending else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.sending = None;
                match result {
                    Ok(_) => self.say("Sent to the device"),
                    Err(send::Failure::LoggedOut) => {
                        // The device shows a new PIN when its WiFi screen opens again
                        self.session = None;
                        self.pin.clear();
                        self.fail("Not sent: connect again with the PIN the device shows now");
                    }
                    Err(send::Failure::Other(error)) => self.fail(format!("Not sent: {error}")),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => self.sending = None,
        }
    }

    /// Logs in to the device with the address and PIN typed in.
    fn connect(&mut self) {
        if self.connecting.is_some() {
            return;
        }
        let (address, pin) = (self.device.clone(), self.pin.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(send::connect(&address, &pin));
        });
        self.session = None;
        self.connect_error = None;
        self.connecting = Some(rx);
    }

    fn poll_connect(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.connecting else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.connecting = None;
                match result {
                    Ok(session) => {
                        self.session = Some(session);
                        // Connect and send, unless the popup was closed meanwhile
                        if Popup::is_id_open(ctx, Id::unique(CONNECT_POPUP)) {
                            self.send();
                        }
                    }
                    Err(error) => self.connect_error = Some(error),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => self.connecting = None,
        }
    }

    fn start_play(&mut self, here: bool) {
        let Some(mut world) = self.world() else {
            return;
        };
        if let (true, Some(at), Some(open)) = (here, self.cursor, &self.open) {
            world.start = Place {
                map: open.project.maps[self.map].id,
                at,
                facing: Dir::Down,
            };
        }
        let title = world.title.clone();
        let world_sounds = world.sounds.clone();
        match Game::new(world, 0) {
            Ok(game) => {
                self.play = Some(Play {
                    game,
                    title,
                    picture: vec![0; WIDTH * HEIGHT],
                    texture: None,
                    last: Instant::now(),
                    behind: 0.0,
                    speaker: sound::Speaker::open()
                        .inspect_err(|e| eprintln!("no sound: {e}"))
                        .ok()
                        .inspect(|speaker| speaker.set_sounds(world_sounds)),
                });
            }
            Err(error) => self.fail(error.to_string()),
        }
    }

    // ---- The window -------------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        if self.picking.is_some() && ctx.input(|i| i.key_pressed(Key::Escape)) {
            if let Some(pick) = self.picking.take() {
                self.map = pick.map;
                self.thing = Some(pick.thing);
            }
        }
        if self.mode == Mode::Things && ctx.input(|i| i.key_pressed(Key::Delete)) {
            if let Some(i) = self.thing {
                self.delete_thing(self.map, i);
            }
        }
        let pressed = |key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key)))
        };
        let redo = ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Z,
            ))
        });
        if redo || pressed(Key::Y) {
            self.redo();
        } else if pressed(Key::Z) {
            self.undo();
        }
        if pressed(Key::S) {
            self.save();
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut action = None;
        let mut play: Option<bool> = None;
        let mut export = false;
        let mut send = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (logo, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            ui.painter().rect_filled(logo, 6, ACCENT);
            ui.painter().rect_filled(logo.shrink(6.0), 2, BAR);
            let app = ui.add(
                egui::Button::new(brand(&format!("Adventure {CARET_DOWN}")))
                    .frame_when_inactive(false),
            );
            Popup::menu(&app).show(|ui| {
                ui.set_min_width(220.0);
                if ui
                    .button(format!("{NEW}   New world from the sample village…"))
                    .clicked()
                {
                    action = Some(Pending::New);
                }
                if ui.button(format!("{NEW}   New empty world…")).clicked() {
                    action = Some(Pending::NewEmpty);
                }
                if ui.button(format!("{OPEN}   Open world…")).clicked() {
                    action = Some(Pending::Open);
                }
                ui.separator();
                let dirty = self.dirty();
                if ui
                    .add_enabled(
                        dirty,
                        egui::Button::new(format!("{SAVE}   Save")).shortcut_text("Ctrl+S"),
                    )
                    .clicked()
                {
                    self.save();
                }
            });
            let Some(open) = &self.open else { return };
            ui.separator();
            let title = open.project.title.clone();
            let (can_undo, can_redo) = (!open.undo.is_empty(), !open.redo.is_empty());
            let world = ui
                .add(quiet(strong(&title)))
                .on_hover_text("The world's settings: title, tileset, characters");
            if world.clicked() {
                self.dialog = Some(Dialog::World(Section::General));
            }
            if self.dirty() {
                ui.label(RichText::new("●").color(ACCENT).size(10.0))
                    .on_hover_text("Unsaved changes");
            }
            ui.add_space(6.0);
            if ui
                .add_enabled(can_undo, quiet(RichText::new(UNDO).size(18.0)))
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(can_redo, quiet(RichText::new(REDO).size(18.0)))
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                self.redo();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Connected: the device the worlds go to, and a way to leave it
                if let Some(session) = &self.session {
                    let chip = ui
                        .add(quiet(
                            RichText::new(format!("●  {}  {CARET_DOWN}", session.address))
                                .color(OK),
                        ))
                        .on_hover_text("The device Send sends to");
                    let found = session.found.clone();
                    Popup::from_toggle_button_response(&chip)
                        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                        .align(RectAlign::BOTTOM_END)
                        .width(280.0)
                        .show(|ui| {
                            ui.label(RichText::new(format!("●  Connected: {found}")).color(OK));
                            ui.label(muted("Send puts the world in its Adventure app"));
                            ui.add_space(4.0);
                            let width = ui.available_width();
                            let button =
                                egui::Button::new("Disconnect").min_size(vec2(width, 28.0));
                            if ui.add(button).clicked() {
                                self.session = None;
                                self.pin.clear();
                                ui.close();
                            }
                        });
                }

                let connected = self.session.is_some();
                let (label, hint) = if connected {
                    (
                        format!("{WIFI}  Send"),
                        format!("Send \"{title}\" to the device"),
                    )
                } else {
                    (
                        format!("{WIFI}  Send…"),
                        "Connect to the device and send the world".to_string(),
                    )
                };
                let button = ui
                    .add_enabled(self.sending.is_none(), egui::Button::new(label))
                    .on_hover_text(hint);
                if button.clicked() && connected {
                    send = true;
                }
                // Not connected: the address and PIN, then the world is sent
                Popup::from_response(&button)
                    .id(Id::unique(CONNECT_POPUP))
                    .open_memory((button.clicked() && !connected).then_some(true.into()))
                    .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                    .align(RectAlign::BOTTOM_END)
                    .width(320.0)
                    .show(|ui| {
                        if self.session.is_some() {
                            ui.close();
                        }
                        ui.label(muted(
                            "Open the WiFi screen on the device: it shows the address and the PIN",
                        ));
                        ui.add_space(4.0);
                        let enter = |ui: &Ui, field: &egui::Response| {
                            field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
                        };
                        ui.label("Address");
                        let field = text_input(ui, &mut self.device, "e.g. 192.168.1.23");
                        let mut connect = enter(ui, &field);
                        let address = send::host(&self.device);
                        // Not while it's being typed: "192.168." is on its way to an address
                        if let (Err(error), false) = (&address, field.has_focus()) {
                            if !self.device.trim().is_empty() {
                                let error = format!("{WARNING}  {error}");
                                ui.label(RichText::new(error).color(WARN));
                            }
                        }
                        ui.label("PIN");
                        let field = text_input(ui, &mut self.pin, "e.g. 1234");
                        // Only its digits: the device shows them with spaces between
                        self.pin.retain(|c| c.is_ascii_digit());
                        self.pin.truncate(send::PIN_DIGITS);
                        connect |= enter(ui, &field);
                        if self.connecting.is_some() {
                            ui.label(muted("Connecting…"));
                        } else if let Some(error) = &self.connect_error {
                            let error = format!("{WARNING}  {error}");
                            ui.label(RichText::new(error).color(WARN));
                        }
                        ui.add_space(4.0);
                        let ready = self.connecting.is_none()
                            && address.is_ok()
                            && send::pin(&self.pin).is_ok();
                        let width = ui.available_width();
                        let button = primary(format!("{WIFI}  Connect and send"))
                            .min_size(vec2(width, 32.0));
                        connect &= ready;
                        connect |= accent(ui, ready, button).clicked();
                        if connect {
                            self.connect();
                        }
                    });
                if ui
                    .button(format!("{EXPORT}  Export…"))
                    .on_hover_text("Write the world as an .adv file")
                    .clicked()
                {
                    export = true;
                }
                ui.separator();
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    let caret = accent(
                        ui,
                        true,
                        primary(CARET_DOWN).corner_radius(CornerRadius {
                            nw: 0,
                            sw: 0,
                            ne: PILL,
                            se: PILL,
                        }),
                    );
                    let start = accent(
                        ui,
                        true,
                        primary(format!("{PLAY}  Play")).corner_radius(CornerRadius {
                            nw: PILL,
                            sw: PILL,
                            ne: 0,
                            se: 0,
                        }),
                    );
                    if start.on_hover_text("From where the world begins").clicked() {
                        play = Some(false);
                    }
                    Popup::menu(&caret).align(RectAlign::BOTTOM_END).show(|ui| {
                        ui.set_min_width(280.0);
                        if ui.button(format!("{FLAG}   From the start")).clicked() {
                            play = Some(false);
                        }
                        let here = self.cursor.is_some();
                        if ui
                            .add_enabled(
                                here,
                                egui::Button::new(format!(
                                    "{CROSSHAIR}   From the cell clicked last"
                                )),
                            )
                            .on_disabled_hover_text("Click a cell on the map first")
                            .clicked()
                        {
                            play = Some(true);
                        }
                    });
                });
            });
        });
        if let Some(then) = action {
            self.ask(then, &ctx);
        }
        if let Some(here) = play {
            self.start_play(here);
        }
        if export {
            self.export();
        }
        if send {
            self.send();
        }
    }

    /// Above the map: the tools of the tab in the middle (or what a door's target asks), the grid
    /// on the right.
    fn tool_bar(&mut self, ui: &mut Ui) {
        let (row, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), BAR_HEIGHT + 8.0), Sense::hover());
        let side = |layout| UiBuilder::new().max_rect(row).layout(layout);
        ui.scope_builder(side(Layout::right_to_left(Align::Center)), |ui| {
            ui.label("Grid");
            switch(ui, &mut self.grid);
        });
        if self.picking.is_some() {
            ui.scope_builder(side(Layout::left_to_right(Align::Center)), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{MAP_PIN}  Click the cell where the player arrives"
                    ))
                    .color(WARN),
                );
                ui.label(muted("· Esc cancels"));
            });
            return;
        }
        let tools = match self.mode {
            Mode::Tiles => &TILE_TOOLS[..],
            Mode::Things => &OBJECT_TOOLS[..],
        };
        // Centred by last frame's width; the first frame draws again once it's known
        let width_id = Id::NULL.with(("tools width", self.mode == Mode::Tiles));
        let width: Option<f32> = ui.data(|d| d.get_temp(width_id));
        let middle = Rect::from_center_size(
            row.center(),
            vec2(width.unwrap_or(row.width()), row.height()),
        );
        let builder = UiBuilder::new()
            .max_rect(middle)
            .layout(Layout::left_to_right(Align::Center));
        let drawn = ui
            .scope_builder(builder, |ui| {
                pill()
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for &(tool, icon, label, hint) in tools {
                            if theme::tool(ui, icon, label, self.tool == tool)
                                .on_hover_text(hint)
                                .clicked()
                            {
                                self.tool = tool;
                            }
                        }
                    })
                    .response
                    .rect
                    .width()
            })
            .inner;
        if width != Some(drawn) {
            ui.data_mut(|d| d.insert_temp(width_id, drawn));
            ui.ctx().request_repaint();
        }
    }

    /// Under the map: the layer being painted on (or what a click does to objects), and the zoom.
    fn view_bar(&mut self, ui: &mut Ui) {
        // A row of fixed height, so both sides are centred on the same line
        let row = vec2(ui.available_width(), BAR_HEIGHT + 8.0);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_size(row);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                pill().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_height(BAR_HEIGHT);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui
                            .add_enabled(self.zoom < 6.0, quiet(PLUS))
                            .on_hover_text("Zoom in")
                            .clicked()
                        {
                            self.zoom += 1.0;
                        }
                        ui.label(muted(format!("{}×", self.zoom as u32)));
                        if ui
                            .add_enabled(self.zoom > 1.0, quiet(MINUS))
                            .on_hover_text("Zoom out")
                            .clicked()
                        {
                            self.zoom -= 1.0;
                        }
                    });
                });
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    match (self.mode, self.picking) {
                        (_, Some(_)) => {}
                        (Mode::Tiles, None) => self.layer_bar(ui),
                        (Mode::Things, None) => {
                            let hint = match self.tool {
                                Tool::Place => {
                                    "Click an empty cell to place what's chosen on the left"
                                }
                                Tool::Remove => "Click an object to remove it",
                                Tool::Start => "Click where the player starts",
                                _ => "Click an object to select it · drag to move it",
                            };
                            ui.add(egui::Label::new(muted(hint)).truncate());
                        }
                    }
                });
            });
        });
    }

    fn palette(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut new_tile = false;
        let mut mode = self.mode;
        ui.horizontal(|ui| {
            pill().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (m, label) in [(Mode::Tiles, "Tiles"), (Mode::Things, "Objects")] {
                        let on = mode == m;
                        let look = Frame::new()
                            .fill(if on { SELECT } else { Color32::TRANSPARENT })
                            .corner_radius(12)
                            .inner_margin(Margin::symmetric(12, 3));
                        let tab = surface(ui, look, (!on).then_some(HOVER), |ui| {
                            ui.label(strong(label).color(if on { TEXT } else { MUTED }));
                        });
                        if tab.clicked() {
                            mode = m;
                        }
                    }
                });
            });
            if mode == Mode::Tiles {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(quiet(RichText::new(PLUS).size(17.0)))
                        .on_hover_text("New tile")
                        .clicked()
                    {
                        new_tile = true;
                    }
                });
            }
        });
        ui.add_space(4.0);
        if mode != self.mode {
            self.mode = mode;
            self.picking = None;
            self.dragging = None;
            // The right panel changes width, so the map's room does too
            self.fit = true;
            self.tool = match mode {
                Mode::Tiles => Tool::Pencil,
                Mode::Things => Tool::Select,
            };
        }
        if self.mode == Mode::Things {
            self.things_palette(ui);
            return;
        }
        let Some(open) = &mut self.open else { return };
        ensure_tileset(open, &ctx);
        let Some(tileset) = open.tileset.clone() else {
            ui.label(muted("The tileset can't be read."));
            return;
        };
        let size = tileset.size_vec2();
        // Cells of the tileset that have a picture but no tile yet
        let cells = (size.x as usize / TILE) * (size.y as usize / TILE);
        let picture = open.assets.tileset.as_ref().ok();
        let unused: Vec<u16> = open
            .project
            .unused_cells(cells)
            .into_iter()
            .filter(|&cell| {
                picture
                    .and_then(|t| t.cell(cell as usize))
                    .is_some_and(|pixels| pixels.iter().any(|&p| p != TRANSPARENT))
            })
            .collect();
        let add = |project: &mut Project, cell: u16| {
            let pixels = picture.and_then(|t| t.cell(cell as usize));
            project.tiles.push(TileDef {
                name: format!("Tile {}", project.tiles.len() + 1),
                layer: project::layer_for(pixels.as_deref()),
                cells: vec![cell],
                frame_ms: 300,
                solid: false,
                step_sound: None,
            });
        };
        if new_tile {
            add(&mut open.project, unused.first().copied().unwrap_or(0));
            self.tile = open.project.tiles.len() - 1;
            self.block = None;
            self.layer = None;
        }
        // Each tile where its picture is in the tileset, so shapes drawn across several cells
        // stay together; tiles sharing a picture with another come after it
        let columns = (size.x as usize / TILE).max(1);
        let rows = size.y as usize / TILE;
        let mut at_cell: Vec<Option<usize>> = vec![None; columns * rows];
        let mut others = Vec::new();
        for (i, tile) in open.project.tiles.iter().enumerate() {
            match at_cell.get_mut(tile.cells.first().copied().unwrap_or(0) as usize) {
                Some(slot @ None) => *slot = Some(i),
                _ => others.push(i),
            }
        }
        // Cells picked this frame: a click picks one, a drag a block
        let mut chosen = None;
        ui.add_space(4.0);
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            // As big as the panel allows, but big enough to click: then it scrolls sideways
            let side = ((ui.available_width() - 6.0) / columns as f32)
                .floor()
                .clamp(24.0, 48.0);
            let (outer, response) = ui.allocate_exact_size(
                vec2(columns as f32, rows as f32) * side + vec2(6.0, 6.0),
                Sense::click_and_drag(),
            );
            let rect = outer.shrink(3.0);
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            let cell_at = |pos: egui::Pos2| {
                let p = (pos - rect.min) / side;
                (
                    (p.x.max(0.0) as usize).min(columns - 1),
                    (p.y.max(0.0) as usize).min(rows.saturating_sub(1)),
                )
            };
            let cell_rect = |(x, y): (usize, usize)| {
                Rect::from_min_size(rect.min + vec2(x as f32, y as f32) * side, vec2(side, side))
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 0, PANEL2);
            // Pictures that aren't a tile yet faintly, the tiles over them
            let whole = Rect::from_min_max(
                pos2(0.0, 0.0),
                pos2(
                    (columns * TILE) as f32 / size.x,
                    (rows * TILE) as f32 / size.y,
                ),
            );
            painter.image(tileset.id(), rect, whole, Color32::from_white_alpha(70));
            // The tiles of faint layers darker, of those turned off darker still
            let shows = self.shows;
            let tint = |kind: LayerKind| match shows[kind as usize] {
                Show::Shown => Color32::WHITE,
                Show::Faint => Color32::from_gray(130),
                Show::Off => Color32::from_gray(60),
            };
            for (cell, tile) in at_cell.iter().enumerate() {
                if let Some(i) = *tile {
                    let r = cell_rect((cell % columns, cell / columns));
                    painter.rect_filled(r, 0, PANEL2);
                    let colour = tint(open.project.tiles[i].layer);
                    painter.image(tileset.id(), r, cell_uv(size, cell as u16), colour);
                }
            }
            let hover = response.hover_pos().map(cell_at);
            let latest = ui.input(|i| i.pointer.latest_pos()).map(cell_at);
            if response.drag_started() {
                self.sheet_drag = ui.input(|i| i.pointer.press_origin()).map(cell_at);
            }
            let dragged = self.sheet_drag.zip(latest).filter(|_| response.dragged());
            if response.drag_stopped() {
                chosen = self.sheet_drag.take().zip(latest);
            } else if response.clicked() {
                chosen = hover.map(|c| (c, c));
            }
            let picked = match (dragged, &self.block) {
                (Some((a, b)), _) => Some(cell_rect(a).union(cell_rect(b))),
                (None, Some(block)) => Some(cell_rect(block.from).union(cell_rect(block.to))),
                (None, None) => at_cell
                    .iter()
                    .position(|&t| t == Some(self.tile))
                    .map(|cell| cell_rect((cell % columns, cell / columns))),
            };
            if let Some(at) = hover.filter(|_| dragged.is_none()) {
                painter.rect_stroke(
                    cell_rect(at),
                    2,
                    Stroke::new(1.5, MUTED),
                    StrokeKind::Outside,
                );
            }
            if let Some(r) = picked {
                ring(ui, r, 4);
            }
            let hint = hover.and_then(|(x, y)| {
                let cell = y * columns + x;
                match at_cell[cell] {
                    Some(i) => {
                        let tile = &open.project.tiles[i];
                        Some(format!("{} · {}", tile.name, layer_name(tile.layer)))
                    }
                    None if unused.contains(&(cell as u16)) => {
                        Some("Not a tile yet: click to use it".to_string())
                    }
                    None => None,
                }
            });
            if let Some(hint) = hint {
                response.on_hover_text(hint);
            }
            if !others.is_empty() {
                ui.add_space(8.0);
                ui.label(muted("More tiles on the same pictures").small());
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                ui.horizontal_wrapped(|ui| {
                    for &i in &others {
                        let tile = &open.project.tiles[i];
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(side, side), Sense::click());
                        egui::Image::from_texture(SizedTexture::new(tileset.id(), rect.size()))
                            .uv(cell_uv(size, tile.cells.first().copied().unwrap_or(0)))
                            .tint(tint(tile.layer))
                            .paint_at(ui, rect);
                        if i == self.tile && self.block.is_none() {
                            ring(ui, rect, 4);
                        }
                        if response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(format!("{} · {}", tile.name, layer_name(tile.layer)))
                            .clicked()
                        {
                            self.tile = i;
                            self.block = None;
                            self.layer = None;
                        }
                    }
                });
            }
            if !unused.is_empty() {
                ui.add_space(6.0);
                let label = format!("{PLUS}  A tile for each unused cell ({})", unused.len());
                if ui.add(quiet(muted(label))).clicked() {
                    for &cell in &unused {
                        add(&mut open.project, cell);
                    }
                }
            }
        });
        let Some((a, b)) = chosen else { return };
        // The picked cells as tiles: pictures that aren't one yet become one
        let ((x0, x1), (y0, y1)) = ((a.0.min(b.0), a.0.max(b.0)), (a.1.min(b.1), a.1.max(b.1)));
        let mut tiles = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let cell = y * columns + x;
                tiles.push(match at_cell[cell] {
                    Some(i) => i as u16 + 1,
                    None if unused.contains(&(cell as u16)) => {
                        add(&mut open.project, cell as u16);
                        open.project.tiles.len() as u16
                    }
                    None => 0,
                });
            }
        }
        let Some(&first) = tiles.iter().find(|&&t| t > 0) else {
            return;
        };
        self.tile = first as usize - 1;
        self.layer = None;
        self.block = (tiles.len() > 1).then(|| Block {
            from: (x0, y0),
            to: (x1, y1),
            brush: Brush {
                width: (x1 - x0 + 1) as u16,
                height: (y1 - y0 + 1) as u16,
                tiles,
            },
        });
        if self.tool == Tool::Eraser {
            self.tool = Tool::Pencil;
        }
    }

    fn inspector(&mut self, ui: &mut Ui) {
        let mut move_them = false;
        // After moving: how many uses stayed on other layers
        let mut moved = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let Some(open) = &mut self.open else { return };
                ensure_tileset(open, ui.ctx());
                let Some(tileset) = open.tileset.clone() else {
                    return;
                };
                let tileset_size = tileset.size_vec2();
                let sounds: Vec<String> =
                    open.project.sounds.iter().map(|s| s.name.clone()).collect();
                let tile_count = open.project.tiles.len();
                let (uses, on_maps) = usage(&open.project, self.tile as u16 + 1);
                let by_layer = open.project.layer_uses(self.tile);
                let opaque = open.project.tiles.get(self.tile).is_some_and(|tile| {
                    let cell = tile.cells.first().copied().unwrap_or(0) as usize;
                    let pixels = open.assets.tileset.as_ref().ok().and_then(|t| t.cell(cell));
                    pixels.is_some_and(|p| !p.contains(&TRANSPARENT))
                });
                header(ui, "Tile", "a kind of tile, wherever it's used");
                let Some(tile) = open.project.tiles.get_mut(self.tile) else {
                    ui.label(muted("No tile selected."));
                    return;
                };
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0, PANEL2);
                    egui::Image::from_texture(SizedTexture::new(tileset.id(), vec2(72.0, 72.0)))
                        .uv(cell_uv(
                            tileset_size,
                            tile.cells.first().copied().unwrap_or(0),
                        ))
                        .paint_at(ui, rect);
                    ui.vertical(|ui| {
                        ui.label(muted("Name").small());
                        text_input(ui, &mut tile.name, "the tile's name");
                    });
                });
                let used = match (uses, on_maps) {
                    (0, _) => "Not used on any map yet".to_string(),
                    (1, _) => "Used once · changes apply there".to_string(),
                    (n, 1) => format!("Used {n} times on 1 map · changes apply everywhere"),
                    (n, m) => format!("Used {n} times on {m} maps · changes apply everywhere"),
                };
                ui.add_space(2.0);
                ui.label(muted(used).small());
                ui.add_space(8.0);
                field(ui, "Layer · where painting puts it", |ui| {
                    layer_choice(ui, &mut tile.layer);
                });
                let elsewhere: usize = LayerKind::ALL
                    .iter()
                    .filter(|&&k| k != tile.layer)
                    .map(|&k| by_layer[k as usize])
                    .sum();
                if elsewhere > 0 {
                    ui.horizontal(|ui| {
                        let text = match elsewhere {
                            1 => "1 use on another layer".to_string(),
                            n => format!("{n} uses on other layers"),
                        };
                        ui.label(
                            RichText::new(format!("{WARNING} {text}"))
                                .color(WARN)
                                .small(),
                        );
                        if ui
                            .add(quiet(RichText::new("Move them").small()))
                            .on_hover_text(format!(
                                "Onto {}, where it's empty or the tile covered it anyway",
                                layer_name(tile.layer)
                            ))
                            .clicked()
                        {
                            move_them = true;
                        }
                    });
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(strong("Solid"));
                        ui.label(muted("can't be walked onto").small());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        switch(ui, &mut tile.solid);
                    });
                });
                field(ui, "Step sound", |ui| {
                    let current = tile
                        .step_sound
                        .and_then(|s| sounds.get(s.0 as usize).cloned())
                        .unwrap_or_else(|| "None".into());
                    egui::ComboBox::from_id_salt("step sound")
                        .selected_text(current)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut tile.step_sound, None, "None");
                            for (i, name) in sounds.iter().enumerate() {
                                ui.selectable_value(
                                    &mut tile.step_sound,
                                    Some(SoundRef(i as u16)),
                                    name,
                                );
                            }
                        });
                });
                let mut remove = None;
                field(ui, "Pictures · more than one makes it move", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (n, &cell) in tile.cells.iter().enumerate() {
                            let (rect, response) =
                                ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                            let removable = tile.cells.len() > 1;
                            ui.painter().rect_filled(rect, 0, PANEL2);
                            egui::Image::from_texture(SizedTexture::new(
                                tileset.id(),
                                vec2(36.0, 36.0),
                            ))
                            .uv(cell_uv(tileset_size, cell))
                            .paint_at(ui, rect);
                            if removable {
                                let response = response
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("Remove this picture");
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        0,
                                        Color32::from_black_alpha(140),
                                    );
                                    ui.painter().text(
                                        rect.center(),
                                        Align2::CENTER_CENTER,
                                        CROSS,
                                        egui::FontId::proportional(18.0),
                                        TEXT,
                                    );
                                }
                                if response.clicked() {
                                    remove = Some(n);
                                }
                            }
                        }
                        ui.menu_button(RichText::new(PLUS).size(16.0), |ui| {
                            ui.label(muted("A cell of the tileset:"));
                            let columns = tileset_size.x as usize / TILE;
                            let rows = tileset_size.y as usize / TILE;
                            egui::Grid::new("cells")
                                .spacing(vec2(3.0, 3.0))
                                .show(ui, |ui| {
                                    for row in 0..rows {
                                        for col in 0..columns {
                                            let cell = (row * columns + col) as u16;
                                            let image = egui::Image::from_texture(
                                                SizedTexture::new(tileset.id(), vec2(32.0, 32.0)),
                                            )
                                            .uv(cell_uv(tileset_size, cell));
                                            if ui
                                                .add(egui::Button::image(image).frame(false))
                                                .clicked()
                                            {
                                                tile.cells.push(cell);
                                                ui.close();
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        })
                        .response
                        .on_hover_text("Add a picture");
                    });
                });
                if let Some(n) = remove {
                    tile.cells.remove(n);
                }
                if tile.cells.len() > 1 {
                    field(ui, "Each picture shows for", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut tile.frame_ms)
                                .range(50..=5000)
                                .suffix(" ms"),
                        );
                    });
                }
                ui.add_space(4.0);
                if tile_count > 1
                    && ui
                        .add(danger(format!("{TRASH}  Delete tile")))
                        .on_hover_text("Where maps use it, they become empty")
                        .clicked()
                {
                    open.project.remove_tile(self.tile);
                    self.tile = self.tile.min(open.project.tiles.len() - 1);
                    self.block = None;
                    self.layer = None;
                } else if move_them {
                    moved = Some(open.project.move_to_own_layer(self.tile, opaque));
                }
            });
        match moved {
            Some(0) => self.say("Moved onto the tile's layer"),
            Some(n) => self.say(format!(
                "Moved; {n} stay where the tile's layer has something else"
            )),
            None => {}
        }
    }

    fn delete_map(&mut self, index: usize) {
        let result = self
            .open
            .as_mut()
            .map(|open| open.project.remove_map(index));
        match result {
            Some(Ok(())) => {
                self.map = self.map.saturating_sub(1).min(index);
                self.keep_selection_valid();
                self.dialog = None;
                self.fit = true;
                self.say("Map deleted");
            }
            Some(Err(reason)) => self.fail(format!("Can't delete the map: {reason}")),
            None => {}
        }
    }

    /// The world's settings, in a window with a section per topic.
    fn world_dialog(&mut self, ctx: &egui::Context, section: Section) {
        // The picture, made from the world as it is when the settings open
        if self.thumbnail.is_none() {
            let world = self
                .open
                .as_ref()
                .and_then(|open| export::world(&open.project, &open.assets).ok());
            if let Some(world) = world {
                let picture = &world.thumbnail;
                let pixels = picture.pixels.0.iter().map(|&p| colour(p)).collect();
                let size = [picture.width as usize, picture.height as usize];
                let image = ColorImage::new(size, pixels);
                self.thumbnail =
                    Some(ctx.load_texture("thumbnail", image, TextureOptions::NEAREST));
            }
        }
        let thumbnail = self.thumbnail.clone();
        let mut close = false;
        let mut import = false;
        let mut add_tiles = false;
        let mut add_pictures = false;
        let mut removed_picture = false;
        let mut add_sounds = false;
        let mut removed_sound = false;
        let mut preview = None;
        let previewing = self.previewing;
        let mut import_midi = false;
        let mut removed_music = false;
        let song_open = self.song_open;
        let mut toggle_song = None;
        let mut changed_song = None;
        let mut go = None;
        let modal = egui::Modal::new(Id::NULL.with("world settings"))
            .frame(dialog_frame())
            .show(ctx, |ui| {
                let Some(open) = &mut self.open else { return };
                ensure_tileset(open, ui.ctx());
                ensure_sprites(open, ui.ctx());
                ui.set_width(680.0);
                dialog_title(ui, "World settings", &mut close);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (s, icon, label) in [
                            (Section::General, INFO, "General"),
                            (Section::Pictures, IMAGE, "Pictures"),
                            (Section::Sounds, SPEAKER, "Sounds & music"),
                            (Section::Items, FLAG, "Items & flags"),
                        ] {
                            let on = s == section;
                            let look = Frame::new()
                                .fill(if on { SELECT } else { Color32::TRANSPARENT })
                                .corner_radius(12)
                                .inner_margin(Margin::symmetric(10, 7));
                            let item = surface(ui, look, (!on).then_some(HOVER), |ui| {
                                ui.set_width(150.0);
                                let colour = if on { TEXT } else { MUTED };
                                ui.label(RichText::new(format!("{icon}   {label}")).color(colour));
                            });
                            if item.clicked() {
                                go = Some(s);
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        // As tall as the tallest section, so switching doesn't jump; scrolling when
                        // the window is too small for it
                        ui.set_min_height(430.0);
                        let room = (ui.ctx().content_rect().height() - 220.0).max(300.0);
                        egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
                        match section {
                            Section::General => {
                                field(ui, "Title", |ui| {
                                    text_input(ui, &mut open.project.title, "the world's name");
                                });
                                ui.label(
                                    muted("Shown on the web page and used as the file's name.")
                                        .small(),
                                );
                                ui.add_space(8.0);
                                field(ui, "Picture on the device", |ui| {
                                    if let Some(texture) = &thumbnail {
                                        ui.add(
                                            egui::Image::from_texture(SizedTexture::new(
                                                texture.id(),
                                                vec2(WIDTH as f32, HEIGHT as f32) * 1.5,
                                            ))
                                            .corner_radius(8),
                                        );
                                    }
                                    ui.label(
                                        muted(
                                            "What the player sees at the start: the start screen \
                                             shows it to pick the world by.",
                                        )
                                        .small(),
                                    );
                                });
                                field(ui, "Folder", |ui| {
                                    ui.label(muted(open.dir.display().to_string()).small());
                                });
                            }
                            Section::Sounds => {
                                field(ui, "Sounds · for objects' rules and tiles' steps", |ui| {
                                    let width = ui.available_width();
                                    let uses: Vec<usize> = (0..open.project.sounds.len())
                                        .map(|i| open.project.sound_uses(i))
                                        .collect();
                                    let mut remove = None;
                                    for (i, def) in open.project.sounds.iter_mut().enumerate() {
                                        let loaded = open.assets.sounds.get(i).and_then(Option::as_ref);
                                        let problem = loaded.and_then(|l| l.as_ref().err());
                                        let what = match (&def.source, loaded) {
                                            (SoundSource::Notes { notes }, _) => {
                                                format!("{} notes", notes.len())
                                            }
                                            (SoundSource::File { file }, Some(Ok(samples))) => {
                                                format!("{file} · {:.1} s", audio::seconds(samples))
                                            }
                                            (SoundSource::File { file }, _) => file.clone(),
                                        };
                                        let used = match uses[i] {
                                            0 => format!("{what} · not used yet"),
                                            1 => format!("{what} · used once"),
                                            n => format!("{what} · used {n} times"),
                                        };
                                        things::card(ui, width, |ui| {
                                            let icon = if problem.is_some() { WARNING } else { PLAY };
                                            let hint = problem.map_or("Listen".to_string(), Clone::clone);
                                            if listen_button(ui, icon, problem.is_some())
                                                .on_hover_text(hint)
                                                .clicked()
                                                && problem.is_none()
                                            {
                                                preview = Some(Preview::Sound(i));
                                            }
                                            things::name_and_use(ui, &mut def.name, "the sound's name", &used, uses[i], &mut remove, i);
                                        });
                                    }
                                    if let Some(i) = remove {
                                        open.project.remove_sound(i);
                                        removed_sound = true;
                                    }
                                    if ui.add(quiet(muted(format!("{PLUS}  Add sounds…")))).clicked() {
                                        add_sounds = true;
                                    }
                                    ui.label(
                                        muted(
                                            "WAVs go into sounds/: made mono, 16 kHz and 8-bit, \
                                             quiet ends cut off, at most 4 seconds.",
                                        )
                                        .small(),
                                    );
                                });
                                ui.add_space(6.0);
                                field(ui, "Music · for maps", |ui| {
                                    let width = ui.available_width();
                                    let uses: Vec<usize> = (0..open.project.music.len())
                                        .map(|i| open.project.music_uses(i))
                                        .collect();
                                    let mut remove = None;
                                    for (i, def) in open.project.music.iter_mut().enumerate() {
                                        let loaded = open.assets.songs.get(i);
                                        let problem = match loaded {
                                            Some(Ok(_)) => None,
                                            Some(Err(error)) => Some(error.clone()),
                                            None => Some(format!("{} isn't loaded", def.file)),
                                        };
                                        let song = loaded.and_then(|l| l.as_ref().ok()).map(|f| &f.song);
                                        let length = song.map_or(String::new(), |s| {
                                            seconds(s.length_ms)
                                        });
                                        let used = match uses[i] {
                                            0 => format!("{}{length} · no map plays it yet", def.file),
                                            1 => format!("{}{length} · played on 1 map", def.file),
                                            n => format!("{}{length} · played on {n} maps", def.file),
                                        };
                                        let playing = previewing == Some(i);
                                        let before = (def.melody, def.bass, def.melody_wave, def.bass_wave);
                                        things::card_vertical(ui, width, |ui| {
                                            ui.horizontal(|ui| {
                                                let (icon, hint) = match (&problem, playing) {
                                                    (Some(problem), _) => (WARNING, problem.clone()),
                                                    (None, true) => (STOP, "Stop".to_string()),
                                                    (None, false) => (PLAY, "Listen".to_string()),
                                                };
                                                if listen_button(ui, icon, problem.is_some())
                                                    .on_hover_text(hint)
                                                    .clicked()
                                                    && problem.is_none()
                                                {
                                                    preview = Some(if playing { Preview::Stop } else { Preview::Music(i) });
                                                }
                                                things::name_and_use(ui, &mut def.name, "the music's name", &used, uses[i], &mut remove, i);
                                            });
                                                let Some(song) = song else { return };
                                                let labels: Vec<&str> = song.groups.iter().map(|g| g.label.as_str()).collect();
                                                let short = |i: usize| {
                                                    labels.get(i).map_or("missing", |l| l.split(" · ").next().unwrap_or(l))
                                                };
                                                let summary = match def.bass {
                                                    Some(bass) => format!("melody: {}, bass: {}", short(def.melody), short(bass)),
                                                    None => format!("melody: {}, no bass", short(def.melody)),
                                                };
                                                let expanded = song_open == Some(i);
                                                let caret = if expanded { CARET_DOWN } else { CARET_RIGHT };
                                                if ui
                                                    .add(quiet(muted(format!("{caret}  Parts · {summary}")).small()))
                                                    .on_hover_text("Which of the file's parts play, and how they sound")
                                                    .clicked()
                                                {
                                                    toggle_song = Some(if expanded { None } else { Some(i) });
                                                }
                                                if !expanded {
                                                    return;
                                                }
                                                let missing = "missing: choose another";
                                                egui::Grid::new(("music parts", i)).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
                                                    ui.label("Melody");
                                                    egui::ComboBox::from_id_salt(("melody", i))
                                                        .width(260.0)
                                                        .selected_text(labels.get(def.melody).copied().unwrap_or(missing))
                                                        .show_ui(ui, |ui| {
                                                            for (g, label) in labels.iter().enumerate() {
                                                                ui.selectable_value(&mut def.melody, g, *label);
                                                            }
                                                        });
                                                    wave_choice(ui, &format!("melody wave {i}"), &mut def.melody_wave);
                                                    ui.end_row();
                                                    ui.label("Bass");
                                                    let current = match def.bass {
                                                        None => "None",
                                                        Some(b) => labels.get(b).copied().unwrap_or(missing),
                                                    };
                                                    egui::ComboBox::from_id_salt(("bass", i))
                                                        .width(260.0)
                                                        .selected_text(current)
                                                        .show_ui(ui, |ui| {
                                                            ui.selectable_value(&mut def.bass, None, "None");
                                                            for (g, label) in labels.iter().enumerate() {
                                                                ui.selectable_value(&mut def.bass, Some(g), *label);
                                                            }
                                                        });
                                                    if def.bass.is_some() {
                                                        wave_choice(ui, &format!("bass wave {i}"), &mut def.bass_wave);
                                                    }
                                                    ui.end_row();
                                                });
                                                ui.label(
                                                    muted(
                                                        "Where notes sound together, the melody keeps the top one \
                                                         and the bass the bottom one. Change the MIDI file in any \
                                                         music program: it reloads by itself.",
                                                    )
                                                    .small(),
                                                );
                                        });
                                        // A change while listening: listen to the new choice
                                        let after = (def.melody, def.bass, def.melody_wave, def.bass_wave);
                                        if after != before && playing {
                                            changed_song = Some(i);
                                        }
                                    }
                                    if let Some(i) = remove {
                                        open.project.remove_music(i);
                                        removed_music = true;
                                    }
                                    if open.project.music.is_empty() {
                                        ui.label(muted("No music yet.").small());
                                    }
                                    if ui.add(quiet(muted(format!("{PLUS}  Import MIDI…")))).clicked() {
                                        import_midi = true;
                                    }
                                    ui.label(
                                        muted(
                                            "MIDI files go into music/: a song plays its melody, and a bass \
                                             if you like, one note at a time each.",
                                        )
                                        .small(),
                                    );
                                });
                            }
                            Section::Items => {
                                things::items_and_flags(
                                    ui,
                                    &mut open.project,
                                    &open.sprite_pictures,
                                );
                            }
                            Section::Pictures => {
                                field(ui, "Tileset", |ui| {
                                    if let Some(tileset) = &open.tileset {
                                        // At most 440 wide and 220 high: a big tileset stays a preview
                                        let size = tileset.size_vec2() * 2.0;
                                        let scale = (440.0 / size.x).min(220.0 / size.y).min(1.0);
                                        ui.add(
                                            egui::Image::from_texture(SizedTexture::new(
                                                tileset.id(),
                                                size * scale,
                                            )),
                                        );
                                    }
                                    ui.label(
                                        muted(
                                            "tiles.png in the world's folder: 16×16 tiles side by \
                                             side. Paint it in any program; it reloads by itself.",
                                        )
                                        .small(),
                                    );
                                    ui.horizontal(|ui| {
                                        if ui
                                            .button(format!("{PLUS}  Add tiles from a PNG…"))
                                            .on_hover_text("Adds its 16×16 tiles to the tileset and the palette; the ones there stay")
                                            .clicked()
                                        {
                                            add_tiles = true;
                                        }
                                        if ui
                                            .button(format!("{IMAGE}  Replace tileset…"))
                                            .on_hover_text("Replaces tiles.png; the tiles keep their cells")
                                            .clicked()
                                        {
                                            import = true;
                                        }
                                    });
                                });
                                ui.add_space(6.0);
                                field(ui, "The player walks as", |ui| {
                                    if open.project.sprites.is_empty() {
                                        ui.label(muted("Add a picture below first.").small());
                                    } else {
                                        let mut hero = open
                                            .project
                                            .heroes
                                            .first()
                                            .copied()
                                            .unwrap_or(adventure_format::SpriteRef(u16::MAX));
                                        let before = hero;
                                        things::sprite_grid(ui, &open.sprite_pictures, &mut hero, 440.0, false);
                                        if hero != before {
                                            open.project.heroes = vec![hero];
                                        }
                                        ui.label(
                                            muted(
                                                "A picture facing the viewer turns by mirroring; one with \
                                                 4 rows (down, up, left, right) turns properly.",
                                            )
                                            .small(),
                                        );
                                    }
                                });
                                ui.add_space(6.0);
                                field(ui, "Pictures · characters, items and bubbles", |ui| {
                                    let width = ui.available_width();
                                    let uses: Vec<usize> = (0..open.project.sprites.len())
                                        .map(|i| open.project.sprite_uses(i))
                                        .collect();
                                    let mut remove = None;
                                    for (i, def) in open.project.sprites.iter_mut().enumerate() {
                                        let problem = open
                                            .assets
                                            .sprites
                                            .get(i)
                                            .and_then(|image| export::sprite(def, image).err());
                                        things::card(ui, width, |ui| {
                                            let (rect, response) =
                                                ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                                            ui.painter().rect_filled(rect, things::IN_CARD, PANEL);
                                            match (&problem, open.sprite_pictures.get(i)) {
                                                (None, Some(Some((texture, uv)))) => {
                                                    things::picture(ui, texture, *uv, rect.shrink(4.0));
                                                }
                                                _ => {
                                                    ui.painter().text(
                                                        rect.center(),
                                                        Align2::CENTER_CENTER,
                                                        WARNING,
                                                        egui::FontId::proportional(22.0),
                                                        WARN,
                                                    );
                                                }
                                            }
                                            if let Some(problem) = &problem {
                                                response.on_hover_text(problem);
                                            }
                                            let used = match uses[i] {
                                                0 => "not used yet".to_string(),
                                                1 => "used once".to_string(),
                                                n => format!("used {n} times"),
                                            };
                                            things::name_and_use(ui, &mut def.name, "the picture's name", &used, uses[i], &mut remove, i);
                                        });
                                    }
                                    if let Some(i) = remove {
                                        open.project.remove_sprite(i);
                                        removed_picture = true;
                                    }
                                    if ui.add(quiet(muted(format!("{PLUS}  Add pictures…")))).clicked() {
                                        add_pictures = true;
                                    }
                                    ui.label(
                                        muted(
                                            "PNGs go into sprites/: one 16×16 picture, or a row per \
                                             direction (down, up, left, right) and a column per step.",
                                        )
                                        .small(),
                                    );
                                });
                            }
                        }
                        });
                    });
                });
            });
        if let Some(section) = go {
            self.dialog = Some(Dialog::World(section));
        }
        if import {
            self.import_tileset();
        }
        if add_tiles {
            self.add_tiles();
        }
        if add_pictures {
            self.add_pictures();
        }
        if removed_picture {
            if let Some(open) = &mut self.open {
                reload(open);
            }
            self.say("Picture removed from the world; its file stays in sprites/");
        }
        if add_sounds {
            self.add_sounds();
        }
        if removed_sound {
            if let Some(open) = &mut self.open {
                reload(open);
            }
            self.say("Sound removed from the world; its file stays in sounds/");
        }
        if import_midi {
            self.import_midi();
        }
        if removed_music {
            if let Some(open) = &mut self.open {
                reload(open);
            }
            self.song_open = None;
            preview = Some(Preview::Stop);
            self.say("Music removed from the world; its file stays in music/");
        }
        if let Some(song) = toggle_song {
            self.song_open = song;
        }
        if let Some(i) = changed_song {
            preview = Some(Preview::Music(i));
        }
        if let Some(preview) = preview {
            self.listen(preview);
        }
        if close || modal.should_close() {
            self.dialog = None;
            self.thumbnail = None;
            self.preview = None;
            self.previewing = None;
        }
    }

    /// A map's settings: name, size, music.
    fn map_dialog(&mut self, ctx: &egui::Context, index: usize) {
        let mut close = false;
        let mut delete = false;
        let mut removed = 0;
        let modal = egui::Modal::new(Id::NULL.with("map settings"))
            .frame(dialog_frame())
            .show(ctx, |ui| {
                let Some(open) = &mut self.open else { return };
                let Some(map) = open.project.maps.get_mut(index) else {
                    close = true;
                    return;
                };
                let names: Vec<String> =
                    open.project.music.iter().map(|m| m.name.clone()).collect();
                ui.set_width(360.0);
                dialog_title(ui, "Map settings", &mut close);
                field(ui, "Name", |ui| {
                    text_input(ui, &mut map.name, "the map's name");
                });
                let (width, height) = (map.width(), map.height());
                let size = match self.size {
                    Some(size) if size.0 == index => size,
                    _ => (index, width, height),
                };
                let (mut w, mut h) = (size.1, size.2);
                field(ui, "Size in tiles", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut w).range(1..=adventure_format::MAX_MAP_SIDE),
                        );
                        ui.label(muted("×"));
                        ui.add(
                            egui::DragValue::new(&mut h).range(1..=adventure_format::MAX_MAP_SIDE),
                        );
                        let changed = (w, h) != (width, height);
                        if ui
                            .add_enabled(changed, egui::Button::new("Resize"))
                            .on_hover_text("Keeps the top left; new ground gets the selected tile")
                            .clicked()
                        {
                            removed = map.resize(w, h, self.tile as u16 + 1);
                        }
                    });
                });
                self.size = Some((index, w, h));
                field(ui, "Music", |ui| {
                    let current = map
                        .music
                        .and_then(|m| names.get(m.0 as usize).cloned())
                        .unwrap_or_else(|| "None".into());
                    egui::ComboBox::from_id_salt("music")
                        .selected_text(current)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut map.music, None, "None");
                            for (i, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut map.music, Some(MusicRef(i as u16)), name);
                            }
                        });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(danger(format!("{TRASH}  Delete map"))).clicked() {
                        delete = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if accent(ui, true, primary("Done")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if removed > 0 {
            self.say(format!("Removed {removed} things that were off the map"));
            self.fit = true;
        }
        if delete {
            self.delete_map(index);
        } else if close || modal.should_close() {
            self.dialog = None;
        }
    }

    fn center(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        self.map = self.map.min(open.project.maps.len() - 1);
        ui.spacing_mut().item_spacing.y = 0.0;

        // The maps as tabs, the open one merging into the sheet below
        let mut select = None;
        let mut settings = None;
        let mut delete = None;
        let mut add_map = false;
        let mut hovered_tab = None;
        let row = vec2(ui.available_width(), 40.0);
        ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for i in 0..open.project.maps.len() {
                let active = i == self.map;
                let thumb = map_picture(open, i, View::Whole, &ctx);
                let map = &open.project.maps[i];
                let start = open.project.start.map == map.id;
                let tab = Frame::new()
                    .fill(if active { SHEET } else { TAB_IDLE })
                    .corner_radius(CornerRadius {
                        nw: 12,
                        ne: 12,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(Margin {
                        left: 8,
                        right: if active { 6 } else { 12 },
                        top: 7,
                        bottom: if active { 9 } else { 7 },
                    });
                let hover = (!active).then_some(HOVER);
                let mut dots = Rect::NOTHING;
                let response = surface(ui, tab, hover, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(30.0, 21.0), Sense::hover());
                        let w = map.width() as f32;
                        let h = map.height() as f32;
                        let uv = Rect::from_min_max(
                            pos2(0.0, 0.0),
                            pos2(
                                (w.min(h * 30.0 / 21.0) / w).min(1.0),
                                (h.min(w * 21.0 / 30.0) / h).min(1.0),
                            ),
                        );
                        egui::Image::from_texture(SizedTexture::new(thumb.id(), rect.size()))
                            .uv(uv)
                            .corner_radius(4)
                            .paint_at(ui, rect);
                        let colour = if active { TEXT } else { MUTED };
                        ui.label(
                            RichText::new(&map.name)
                                .color(colour)
                                .family(egui::FontFamily::Name("bold".into())),
                        );
                        if start {
                            ui.label(RichText::new(FLAG).color(MUTED).size(14.0))
                                .on_hover_text("The world starts on this map");
                        }
                        if active {
                            // "⋯" for the map's settings, shown while the mouse is on the tab
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            dots = rect;
                            if self.tab_hover == Some(i) {
                                let over = ui.rect_contains_pointer(rect);
                                if over {
                                    ui.painter().rect_filled(rect, 6, HOVER);
                                }
                                ui.painter().text(
                                    rect.center(),
                                    Align2::CENTER_CENTER,
                                    DOTS,
                                    egui::FontId::proportional(16.0),
                                    if over { TEXT } else { MUTED },
                                );
                            }
                        }
                    });
                });
                if response.hovered() {
                    hovered_tab = Some(i);
                }
                let on_dots = response.hover_pos().is_some_and(|pos| dots.contains(pos));
                let hint = match (active, on_dots) {
                    (true, true) => "Map settings",
                    (true, false) => "Double-click or right-click for the map's settings",
                    (false, _) => "Open this map",
                };
                let response = response.on_hover_text(hint);
                if response.double_clicked() || (response.clicked() && on_dots) {
                    settings = Some(i);
                } else if response.clicked() && !active {
                    select = Some(i);
                }
                response.context_menu(|ui| {
                    if ui.button(format!("{GEAR}   Map settings…")).clicked() {
                        settings = Some(i);
                    }
                    if ui.add(danger(format!("{TRASH}   Delete map"))).clicked() {
                        delete = Some(i);
                    }
                });
            }
            ui.add_space(4.0);
            if ui
                .add(quiet(RichText::new(PLUS).size(17.0)))
                .on_hover_text("New map")
                .clicked()
            {
                add_map = true;
            }
        });
        self.tab_hover = hovered_tab;
        if let Some(i) = select {
            self.map = i;
            self.fit = true;
            self.cursor = None;
            self.thing = None;
        }
        if let Some(i) = settings {
            self.map = i;
            self.size = None;
            self.dialog = Some(Dialog::Map(i));
        }
        if add_map {
            let fill = if open.project.tiles.is_empty() {
                0
            } else {
                self.tile as u16 + 1
            };
            self.map = open.project.add_map(20, 15, fill);
            self.fit = true;
            self.cursor = None;
            self.size = None;
            self.dialog = Some(Dialog::Map(self.map));
        }
        if let Some(i) = delete {
            self.delete_map(i);
            return;
        }

        let first = self.map == 0;
        let sheet = Frame::new()
            .fill(SHEET)
            .corner_radius(CornerRadius {
                nw: if first { 0 } else { PANEL_RADIUS },
                ne: PANEL_RADIUS,
                sw: PANEL_RADIUS,
                se: PANEL_RADIUS,
            })
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.spacing_mut().item_spacing.y = 10.0;
                self.tool_bar(ui);
                // The map, and the bar under it
                let room = ui.available_size() - vec2(0.0, 46.0);
                ui.allocate_ui(room.max(vec2(0.0, 0.0)), |ui| self.canvas(ui));
                self.view_bar(ui);
            });
        self.sheet = sheet.response.rect;
    }

    /// The three layers: clicking one paints on it (until another tile is chosen), the lit ones are
    /// those painted on; the eye before each makes it faint, turns it off and on again.
    fn layer_bar(&mut self, ui: &mut Ui) {
        let Some(open) = &self.open else { return };
        let tiles = &open.project.tiles;
        // The selected tile's own layers: one, or a block's
        let own: Vec<LayerKind> = match &self.block {
            Some(block) => block
                .brush
                .tiles
                .iter()
                .filter_map(|&n| Some(tiles.get(n.checked_sub(1)? as usize)?.layer))
                .collect(),
            None => tiles.get(self.tile).map(|t| t.layer).into_iter().collect(),
        };
        let painted = self.layer.map_or_else(|| own.clone(), |kind| vec![kind]);
        let (mut eye_clicked, mut chosen) = (None, None);
        pill().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(BAR_HEIGHT);
                ui.spacing_mut().item_spacing.x = 2.0;
                for (n, (kind, label, hint)) in LAYERS.into_iter().enumerate() {
                    if n > 0 {
                        ui.add_space(2.0);
                        ui.label(RichText::new(CARET_RIGHT).color(MUTED).size(13.0));
                        ui.add_space(2.0);
                    }
                    let show = self.shows[kind as usize];
                    let target = painted.contains(&kind);
                    let (eye, next) = match show {
                        Show::Shown => (EYE, "Make it faint"),
                        Show::Faint => (EYE_CLOSED, "Turn it off"),
                        Show::Off => (EYE_SLASH, "Show it again"),
                    };
                    // Faint or off, greyed out; on the lit chip a see-through white, which grey
                    // would vanish into
                    let colour = match (show, target) {
                        (Show::Shown, _) => TEXT,
                        (_, true) => Color32::from_white_alpha(140),
                        (_, false) => MUTED,
                    };
                    // A chip with the eye in it: a click on the eye changes how the layer
                    // shows, one on the rest of the chip chooses the layer
                    let look = Frame::new()
                        .fill(if target { SELECT } else { Color32::TRANSPARENT })
                        .corner_radius(15)
                        .inner_margin(Margin {
                            left: 4,
                            right: 12,
                            top: 0,
                            bottom: 0,
                        });
                    let mut eye_rect = Rect::NOTHING;
                    let segment = surface(ui, look, (!target).then_some(HOVER), |ui| {
                        ui.horizontal(|ui| {
                            ui.set_height(BAR_HEIGHT);
                            ui.spacing_mut().item_spacing.x = 4.0;
                            // A round spot behind the eye under the mouse
                            let (spot, _) =
                                ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                            eye_rect = spot;
                            if ui.rect_contains_pointer(spot) {
                                let shade = Color32::from_white_alpha(if target { 50 } else { 25 });
                                ui.painter().circle_filled(spot.center(), 12.0, shade);
                            }
                            ui.painter().text(
                                spot.center(),
                                Align2::CENTER_CENTER,
                                eye,
                                egui::FontId::proportional(16.0),
                                colour,
                            );
                            let name = RichText::new(label).color(colour);
                            ui.label(if show == Show::Off {
                                name.strikethrough()
                            } else {
                                name
                            });
                        });
                    });
                    let on_eye =
                        |pos: Option<egui::Pos2>| pos.is_some_and(|p| eye_rect.contains(p));
                    if on_eye(segment.hover_pos()) {
                        if segment.on_hover_text(next).clicked() {
                            eye_clicked = Some(kind);
                        }
                        continue;
                    }
                    let tip = match (target, self.layer, own.as_slice()) {
                        (true, None, _) => format!("{hint}\nThe selected tile goes here"),
                        (true, Some(_), &[mine]) if mine != kind => format!(
                            "{hint}\nPainting here instead of on {}, until you choose a tile",
                            layer_name(mine)
                        ),
                        (true, Some(_), _) => format!("{hint}\nPainting and erasing here"),
                        (false, _, _) => format!("{hint}\nClick to paint and erase here"),
                    };
                    if segment.on_hover_text(tip).clicked() {
                        chosen = Some(kind);
                    }
                }
            });
        });
        if let Some(kind) = eye_clicked {
            let show = &mut self.shows[kind as usize];
            *show = show.next();
        }
        // The tile's own layer again is the same as following the tile
        if let Some(kind) = chosen {
            self.layer = (own != [kind]).then_some(kind);
            let show = &mut self.shows[kind as usize];
            if *show == Show::Off {
                *show = Show::Shown;
            }
        }
    }

    fn canvas(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        let view = match self.mode {
            Mode::Tiles => View::Painting(self.shows),
            Mode::Things => View::Objects,
        };
        let texture = map_picture(open, self.map, view, &ctx);
        let things_mode = self.mode == Mode::Things || self.picking.is_some();
        let marks: Vec<(Pos, bool)> = open.project.maps[self.map]
            .things
            .iter()
            .map(|t| (t.thing.at, t.thing.look.is_none()))
            .collect();
        let (selected, dragging) = (self.thing, self.dragging);
        let start_tool = self.tool == Tool::Start;
        // What a click on an empty cell would place, shown faintly under the mouse
        ensure_sprites(open, &ctx);
        let ghost = match (self.tool, self.stamp) {
            (Tool::Place, Stamp::Sprite(sprite)) if self.picking.is_none() => open
                .sprite_pictures
                .get(sprite.0 as usize)
                .cloned()
                .flatten(),
            _ => None,
        };
        let place_spot = self.tool == Tool::Place && self.stamp == Stamp::Spot;
        // What the pencil would paint, shown faintly under the mouse: (column, row, picture)
        let tileset = open.tileset.clone();
        let mut brush_ghost = Vec::new();
        if let (false, Tool::Pencil | Tool::Box, Some(tileset)) = (things_mode, self.tool, &tileset)
        {
            let brush = self
                .block
                .as_ref()
                .map_or_else(|| Brush::one(self.tile as u16 + 1), |b| b.brush.clone());
            for (i, &tile) in brush.tiles.iter().enumerate() {
                let Some(def) = tile
                    .checked_sub(1)
                    .and_then(|t| open.project.tiles.get(t as usize))
                else {
                    continue;
                };
                let cell = def.cells.first().copied().unwrap_or(0);
                let (x, y) = (i as u16 % brush.width, i as u16 / brush.width);
                brush_ghost.push((x, y, cell_uv(tileset.size_vec2(), cell)));
            }
        }
        let mut delete = None;
        let map = &open.project.maps[self.map];
        let (width, height) = (map.width(), map.height());
        if self.fit {
            let room = ui.available_size() - vec2(8.0, 8.0);
            let fits =
                (room.x / (width as f32 * TILE as f32)).min(room.y / (height as f32 * TILE as f32));
            // Rounded: a little scrolling beats a map shown much too small
            self.zoom = fits.round().clamp(1.0, 6.0);
            self.fit = false;
        }
        let scale = TILE as f32 * self.zoom;
        let start = (open.project.start.map == map.id).then_some(open.project.start.at);
        let (tool, rect_from, grid) = (self.tool, self.rect_from, self.grid);

        let mut input = CanvasInput::default();
        // Scroll bars over the map, shown while the mouse is on it: a map a little too big for
        // its room shouldn't get a line drawn under it
        let scroll = &mut ui.spacing_mut().scroll;
        *scroll = egui::style::ScrollStyle::floating();
        scroll.dormant_background_opacity = 0.0;
        scroll.dormant_handle_opacity = 0.0;
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let size = vec2(width as f32 * scale, height as f32 * scale);
            // Centred while it fits, scrolled when it doesn't
            let (outer, _) = ui.allocate_exact_size(
                size.max(ui.available_size() - vec2(0.0, 4.0)),
                Sense::hover(),
            );
            let rect = Rect::from_center_size(
                pos2(
                    if size.x < outer.width() {
                        outer.center().x
                    } else {
                        outer.left() + size.x / 2.0
                    },
                    if size.y < outer.height() {
                        outer.center().y
                    } else {
                        outer.top() + size.y / 2.0
                    },
                ),
                size,
            );
            let response = ui.interact(
                rect,
                ui.scope_id().with("map canvas"),
                Sense::click_and_drag(),
            );
            // A crosshair to paint with, a hand over an object to pick
            let over_object = things_mode
                && !start_tool
                && response.hover_pos().is_some_and(|pos| {
                    let p = (pos - rect.min) / scale;
                    marks
                        .iter()
                        .any(|&(at, _)| at.x as f32 == p.x.floor() && at.y as f32 == p.y.floor())
                });
            let response = response.on_hover_cursor(if over_object {
                egui::CursorIcon::PointingHand
            } else {
                egui::CursorIcon::Crosshair
            });
            let painter = ui.painter_at(outer);
            painter.rect_filled(rect.expand(1.0), 4, Color32::from_black_alpha(160));
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let cell_rect = |at: Pos| {
                Rect::from_min_size(
                    rect.min + vec2(at.x as f32 * scale, at.y as f32 * scale),
                    vec2(scale, scale),
                )
            };
            if grid && scale >= 16.0 {
                let line = Stroke::new(1.0, Color32::from_black_alpha(45));
                for x in 0..=width {
                    let px = rect.min.x + x as f32 * scale;
                    painter.line_segment([pos2(px, rect.min.y), pos2(px, rect.max.y)], line);
                }
                for y in 0..=height {
                    let py = rect.min.y + y as f32 * scale;
                    painter.line_segment([pos2(rect.min.x, py), pos2(rect.max.x, py)], line);
                }
            }
            if let Some(at) = start {
                let r = cell_rect(at);
                painter.rect_filled(r, 4, Color32::from_black_alpha(60));
                painter.rect_stroke(r, 4, Stroke::new(2.0, WARN), StrokeKind::Inside);
                painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    FLAG,
                    egui::FontId::proportional(scale * 0.55),
                    WARN,
                );
            }
            let to_cell = |pos: egui::Pos2| {
                let p = (pos - rect.min) / scale;
                (p.x >= 0.0 && p.y >= 0.0 && p.x < width as f32 && p.y < height as f32)
                    .then(|| Pos::new(p.x as u16, p.y as u16))
            };
            let hover = response.hover_pos().and_then(to_cell);
            if let Some(at) = hover.filter(|_| !over_object) {
                let mut outline = cell_rect(at);
                if let (Some(tileset), None) = (&tileset, rect_from) {
                    for &(x, y, uv) in &brush_ghost {
                        let (px, py) = (at.x + x, at.y + y);
                        if px < width && py < height {
                            let r = cell_rect(Pos::new(px, py));
                            painter.image(tileset.id(), r, uv, Color32::from_white_alpha(150));
                            outline = outline.union(r);
                        }
                    }
                }
                painter.rect_stroke(outline, 3, Stroke::new(1.5, TEXT), StrokeKind::Inside);
            }
            if let (Tool::Box, Some(from), Some(to), false) = (tool, rect_from, hover, things_mode)
            {
                painter.rect_stroke(
                    cell_rect(from).union(cell_rect(to)),
                    3,
                    Stroke::new(2.0, TEXT),
                    StrokeKind::Inside,
                );
            }
            if things_mode {
                // Every object framed (invisible ones dashed), the one under the mouse lit,
                // the selected one ringed
                for (i, &(at, invisible)) in marks.iter().enumerate() {
                    let r = cell_rect(at);
                    if invisible {
                        things::spot(ui, r.shrink(scale * 0.12));
                    } else {
                        painter.rect_stroke(
                            r.shrink(1.0),
                            5,
                            Stroke::new(1.0, Color32::from_white_alpha(70)),
                            StrokeKind::Inside,
                        );
                    }
                    if hover == Some(at) && !start_tool {
                        painter.rect_filled(r, 5, Color32::from_white_alpha(28));
                        painter.rect_stroke(r, 5, Stroke::new(2.0, TEXT), StrokeKind::Inside);
                    }
                    if selected == Some(i) {
                        painter.rect_stroke(
                            r.expand(1.0),
                            5,
                            Stroke::new(2.5, SELECT),
                            StrokeKind::Outside,
                        );
                    }
                }
                if let Some(at) = hover.filter(|at| marks.iter().all(|(m, _)| m != at)) {
                    let r = cell_rect(at);
                    if let Some((texture, uv)) = &ghost {
                        painter.image(texture.id(), r, *uv, Color32::from_white_alpha(140));
                    } else if place_spot {
                        things::spot(ui, r.shrink(scale * 0.12));
                    }
                }
                if let (Some(i), Some(to)) = (dragging, hover) {
                    if let Some(&(from, _)) = marks.get(i) {
                        painter.line_segment(
                            [cell_rect(from).center(), cell_rect(to).center()],
                            Stroke::new(1.5, SELECT),
                        );
                        painter.rect_stroke(
                            cell_rect(to),
                            5,
                            Stroke::new(2.0, SELECT),
                            StrokeKind::Inside,
                        );
                    }
                }
                // Right-click a thing: remove it
                let target_id = ui.scope_id().with("right-clicked thing");
                if response.secondary_clicked() {
                    let hit = response
                        .interact_pointer_pos()
                        .and_then(to_cell)
                        .and_then(|at| marks.iter().position(|&(m, _)| m == at));
                    ui.data_mut(|d| d.insert_temp(target_id, hit));
                }
                let target: Option<usize> = ui.data(|d| d.get_temp(target_id)).flatten();
                response.context_menu(|ui| match target {
                    Some(i) => {
                        if ui.add(danger(format!("{TRASH}   Delete object"))).clicked() {
                            delete = Some(i);
                            ui.close();
                        }
                    }
                    None => {
                        ui.label(muted("No object here"));
                    }
                });
            }
            let primary_down = ctx.input(|i| i.pointer.primary_down());
            input = CanvasInput {
                cell: response.interact_pointer_pos().and_then(to_cell).or(hover),
                painting: response.is_pointer_button_down_on() && primary_down,
                clicked: response.clicked(),
                picked: response.secondary_clicked(),
                drag_started: response.drag_started_by(egui::PointerButton::Primary),
                drag_stopped: response.drag_stopped_by(egui::PointerButton::Primary),
            };
        });
        if let Some(i) = delete {
            self.delete_thing(self.map, i);
        }
        self.apply(input);
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let life = |toast: &Toast| if toast.error { 6.0 } else { 2.5 };
        self.toasts
            .retain(|toast| toast.at.elapsed().as_secs_f32() < life(toast));
        if self.toasts.is_empty() {
            return;
        }
        let area = if self.sheet == Rect::NOTHING {
            ctx.content_rect()
        } else {
            self.sheet
        };
        egui::Area::new(Id::NULL.with("toasts"))
            .order(egui::Order::Tooltip)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(area.center().x, area.bottom() - 16.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    for toast in self.toasts.iter().rev().take(3).rev() {
                        let left = life(toast) - toast.at.elapsed().as_secs_f32();
                        ui.scope(|ui| {
                            ui.multiply_opacity(left.clamp(0.0, 0.5) * 2.0);
                            let (icon, colour) = if toast.error {
                                (WARNING, WARN)
                            } else {
                                (CHECK, OK)
                            };
                            Frame::new()
                                .fill(PANEL2)
                                .corner_radius(PILL)
                                .stroke(Stroke::new(1.0, LINE))
                                .inner_margin(Margin::symmetric(14, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(icon).color(colour));
                                        ui.label(&toast.text);
                                    });
                                });
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn apply(&mut self, input: CanvasInput) {
        if self.mode == Mode::Things || self.picking.is_some() {
            if let Some(at) = input.cell {
                self.things_click(at, &input);
            } else if input.drag_stopped {
                self.dragging = None;
            }
            return;
        }
        let Some(open) = &mut self.open else { return };
        if !input.painting {
            self.stroke_from = None;
            self.erasing = None;
        }
        let Some(at) = input.cell else {
            if input.drag_stopped {
                self.rect_from = None;
            }
            return;
        };
        let brush = self
            .block
            .as_ref()
            .map_or_else(|| Brush::one(self.tile as u16 + 1), |b| b.brush.clone());
        let has_tiles = !open.project.tiles.is_empty();
        if input.clicked || input.drag_started {
            self.cursor = Some(at);
        }
        // Each tile on its own layer, or all on the one clicked in the layer bar
        let layers = match self.layer {
            Some(kind) => vec![kind; open.project.tiles.len()],
            None => open.project.tile_layers(),
        };
        let map = &mut open.project.maps[self.map];
        // Picking up takes the top tile that's shown, faint or not; erasing too, unless a layer
        // was clicked
        let off = self.shows.map(|show| show == Show::Off);
        let top = map.top_layer(at, off);
        // Painting or erasing on a layer that's turned off turns it on, so what changes shows
        let changes: Vec<LayerKind> = match self.tool {
            Tool::Pencil | Tool::Fill | Tool::Box if input.painting && has_tiles => brush
                .tiles
                .iter()
                .filter_map(|&n| layers.get(n.checked_sub(1)? as usize).copied())
                .collect(),
            Tool::Eraser if input.painting => self.layer.into_iter().collect(),
            _ => Vec::new(),
        };
        for kind in changes {
            let show = &mut self.shows[kind as usize];
            if *show == Show::Off {
                *show = Show::Shown;
            }
        }
        if input.picked {
            if let Some(n) = top
                .and_then(|kind| map.layer(kind).get(at))
                .filter(|&n| n > 0)
            {
                self.tile = n as usize - 1;
                self.block = None;
                self.layer = None;
                if self.tool == Tool::Eraser {
                    self.tool = Tool::Pencil;
                }
            }
            return;
        }
        match self.tool {
            Tool::Pencil if input.painting && has_tiles => {
                // A block goes down whole, side by side with the others of the stroke
                let from = *self.stroke_from.get_or_insert(at);
                map.stamp(&layers, brush.snap(from, at), &brush);
            }
            Tool::Eraser if input.painting => {
                // The whole stroke on the layer it began on
                if self.erasing.is_none() {
                    self.erasing = self.layer.or(top);
                }
                if let Some(kind) = self.erasing {
                    map.layer_mut(kind).set(at, 0);
                }
            }
            Tool::Fill if input.clicked && has_tiles => map.flood(&layers, at, &brush),
            Tool::Box => {
                if input.drag_started {
                    self.rect_from = Some(at);
                }
                if input.drag_stopped {
                    if let Some(from) = self.rect_from.take() {
                        map.rect(&layers, from, at, &brush);
                    }
                } else if input.clicked {
                    map.stamp(&layers, at, &brush);
                }
            }
            _ => {}
        }
    }

    /// The game in a window of its own, so the map stays in view: on a handheld like the device,
    /// whose buttons light up when pressed and can be clicked too.
    fn play_window(&mut self, ctx: &egui::Context) {
        let Some(play) = &mut self.play else { return };
        let mut stop = false;
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("Playing {}", play.title))
            .with_inner_size([460.0, 800.0])
            .with_min_inner_size([300.0, 520.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("play"), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
                stop = true;
            }
            egui::CentralPanel::default()
                .frame(Frame::new().fill(BG))
                .show(ui, |ui| {
                    let area = ui.available_rect_before_wrap();
                    let handheld = Handheld::fit(area);

                    // The buttons: keys, or the mouse held on the handheld's buttons
                    let mouse = ctx.input(|i| i.pointer.primary_down().then(|| i.pointer.latest_pos()).flatten());
                    let clicked = |button: HandheldButton| mouse.is_some_and(|pos| handheld.hit(button, pos));
                    let buttons = ctx.input(|i| Buttons {
                        up: i.key_down(Key::ArrowUp) || clicked(HandheldButton::Up),
                        down: i.key_down(Key::ArrowDown) || clicked(HandheldButton::Down),
                        left: i.key_down(Key::ArrowLeft) || clicked(HandheldButton::Left),
                        right: i.key_down(Key::ArrowRight) || clicked(HandheldButton::Right),
                        a: i.key_down(Key::Space) || i.key_down(Key::Enter) || clicked(HandheldButton::A),
                        b: i.modifiers.shift || clicked(HandheldButton::B),
                    });
                    // Clicks on the handheld press its buttons, they don't drag the window
                    ui.allocate_rect(area, Sense::click_and_drag());

                    let now = Instant::now();
                    play.behind += now.duration_since(play.last).as_secs_f64();
                    play.last = now;
                    let step = 1.0 / FPS as f64;
                    let mut steps = 0;
                    while play.behind >= step && steps < 5 {
                        let events = play.game.step(buttons);
                        if let Some(speaker) = &play.speaker {
                            let world = play.game.world();
                            for event in events {
                                match event {
                                    Event::Sound(sound) => speaker.sound(sound),
                                    Event::Music(music) => speaker
                                        .music(music.and_then(|m| world.music.get(m.0 as usize))),
                                    Event::Bump => speaker.play(&BUMP),
                                }
                            }
                        }
                        play.behind -= step;
                        steps += 1;
                    }
                    if steps == 5 {
                        play.behind = 0.0;
                    }
                    if steps > 0 || play.texture.is_none() {
                        play.game.draw(&mut play.picture);
                        let pixels = play.picture.iter().map(|&p| colour(p)).collect();
                        let image = ColorImage::new([WIDTH, HEIGHT], pixels);
                        match &mut play.texture {
                            Some(texture) => texture.set(image, TextureOptions::NEAREST),
                            None => {
                                play.texture = Some(ctx.load_texture("play", image, TextureOptions::NEAREST))
                            }
                        }
                    }
                    if let Some(texture) = &play.texture {
                        handheld.paint(ui, texture, buttons);
                    }
                    let hint = "Arrows walk · Space is A · Shift is B (run) · Esc closes · no sound in the editor";
                    ui.painter().text(
                        pos2(area.center().x, area.bottom() - 14.0),
                        Align2::CENTER_CENTER,
                        hint,
                        egui::FontId::proportional(11.5),
                        MUTED,
                    );
                });
            ctx.request_repaint();
        });
        if stop {
            self.play = None;
        }
    }

    fn welcome(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.label(brand("Adventure").size(28.0));
            ui.add_space(8.0);
            ui.label(muted(
                "Make little worlds to walk around in, for the Adventure app.",
            ));
            ui.add_space(20.0);
            let new = primary(format!("{NEW}  New world…")).min_size(vec2(200.0, 36.0));
            if accent(ui, true, new)
                .on_hover_text("Asks for a name; starts from the sample village")
                .clicked()
            {
                self.ask(Pending::New, &ctx);
            }
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(format!("{NEW}  New empty world…"))
                        .min_size(vec2(200.0, 36.0)),
                )
                .on_hover_text(
                    "Asks for a name; starts with one empty map, for your own tiles and pictures",
                )
                .clicked()
            {
                self.ask(Pending::NewEmpty, &ctx);
            }
            ui.add_space(4.0);
            if ui
                .add(egui::Button::new(format!("{OPEN}  Open world…")).min_size(vec2(200.0, 36.0)))
                .clicked()
            {
                self.ask(Pending::Open, &ctx);
            }
        });
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        let Some(then) = self.pending else { return };
        let mut answer = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                heading(ui, "Unsaved changes");
                ui.label("Save the changes to this world first?");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if accent(ui, true, primary("Save")).clicked() {
                        answer = Some(1);
                    }
                    if ui.button("Don't save").clicked() {
                        answer = Some(2);
                    }
                    if ui.add(quiet("Cancel")).clicked() {
                        answer = Some(0);
                    }
                });
            });
        match answer {
            Some(1) => {
                self.pending = None;
                if self.save() {
                    self.proceed(then, ctx);
                }
            }
            Some(2) => {
                self.pending = None;
                if let Some(open) = &mut self.open {
                    open.saved = open.project.clone();
                }
                self.proceed(then, ctx);
            }
            Some(_) => self.pending = None,
            None => {}
        }
    }
}

#[derive(Default)]
struct CanvasInput {
    cell: Option<Pos>,
    painting: bool,
    clicked: bool,
    picked: bool,
    drag_started: bool,
    drag_stopped: bool,
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let before = self.view();
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }
        self.poll_send(&ctx);
        self.poll_connect(&ctx);
        self.reload_changed_pictures();
        self.shortcuts(&ctx);

        egui::Panel::top("bar")
            .frame(
                Frame::new()
                    .fill(BAR)
                    .inner_margin(Margin::symmetric(14, 9)),
            )
            .show(ui, |ui| self.top_bar(ui));
        let gap = |left: i8, right: i8| Margin {
            left,
            right,
            top: 10,
            bottom: 10,
        };
        if self.open.is_some() {
            // Wider for a tileset of many columns: drag its edge
            egui::Panel::left("palette sheet")
                .resizable(true)
                .default_size(318.0)
                .size_range(222.0..=640.0)
                .frame(panel_frame().outer_margin(gap(10, 0)))
                .show(ui, |ui| self.palette(ui));
            // One panel per mode: egui keeps a panel's size, so a changing width would be laid out stale
            let inspector = match self.mode {
                Mode::Tiles => "inspector",
                Mode::Things => "thing inspector",
            };
            egui::Panel::right(inspector)
                .resizable(false)
                .exact_size(if self.mode == Mode::Things {
                    380.0
                } else {
                    284.0
                })
                .frame(panel_frame().outer_margin(gap(0, 10)))
                .show(ui, |ui| match self.mode {
                    Mode::Tiles => self.inspector(ui),
                    Mode::Things => self.thing_inspector(ui),
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(BG).inner_margin(Margin {
                    left: 10,
                    right: 10,
                    top: 10,
                    bottom: 10,
                }))
                .show(ui, |ui| self.center(ui));
        } else {
            self.sheet = Rect::NOTHING;
            egui::CentralPanel::default()
                .frame(Frame::new().fill(BG))
                .show(ui, |ui| self.welcome(ui));
        }
        self.play_window(&ctx);
        match self.dialog {
            Some(Dialog::World(section)) => self.world_dialog(&ctx, section),
            Some(Dialog::Map(index)) => self.map_dialog(&ctx, index),
            None => {}
        }
        self.toasts(&ctx);
        self.commit(&ctx);
        self.confirm(&ctx);
        // A click often changes what's shown only after it was drawn (another map, a dialog's
        // section, the tool): draw once more, or it would show only once the mouse moves
        if self.view() != before {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("device", self.device.clone());
        if let Some(open) = &self.open {
            storage.set_string("project", open.dir.display().to_string());
        }
    }
}

/// A panel's heading, with a hint what it shows.
fn header(ui: &mut Ui, title: &str, what: &str) {
    ui.horizontal(|ui| {
        heading(ui, title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(muted(what).small());
        });
    });
    ui.add_space(6.0);
}

/// A small label with the control below it.
fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(2.0);
    ui.label(muted(label).small());
    add(ui);
    ui.add_space(4.0);
}

// ---- Pictures for the window -----------------------------------------------------------------

fn colour(pixel: u16) -> Color32 {
    let [r, g, b, a] = images::to_rgba(pixel);
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn rgba_image(image: &Rgba) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([image.width, image.height], image.pixels.as_flattened())
}

/// Where cell `n` of a tileset of `size` pixels is, as texture coordinates.
fn cell_uv(size: egui::Vec2, cell: u16) -> Rect {
    let columns = (size.x as usize / TILE).max(1);
    let (x, y) = (
        (cell as usize % columns * TILE) as f32,
        (cell as usize / columns * TILE) as f32,
    );
    let tile = TILE as f32;
    Rect::from_min_size(
        pos2(x / size.x, y / size.y),
        vec2(tile / size.x, tile / size.y),
    )
}

fn picture_times(dir: &Path, project: &Project) -> Vec<Option<SystemTime>> {
    let time = |path: PathBuf| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    std::iter::once(time(dir.join(project::TILESET)))
        .chain(
            project
                .sprites
                .iter()
                .map(|s| time(dir.join(project::SPRITES).join(&s.file))),
        )
        .chain(project.sounds.iter().map(|s| match &s.source {
            SoundSource::Notes { .. } => None,
            SoundSource::File { file } => time(dir.join(project::SOUNDS).join(file)),
        }))
        .chain(
            project
                .music
                .iter()
                .map(|m| time(dir.join(project::MUSIC).join(&m.file))),
        )
        .collect()
}

/// The files a project's pictures, sounds and music come from, in order: when they change, what
/// was loaded from them no longer lines up.
fn files(project: &Project) -> Vec<String> {
    let sounds = project.sounds.iter().map(|s| match &s.source {
        SoundSource::Notes { .. } => String::new(),
        SoundSource::File { file } => file.clone(),
    });
    project
        .sprites
        .iter()
        .map(|s| s.file.clone())
        .chain(std::iter::once("|".into()))
        .chain(sounds)
        .chain(std::iter::once("|".into()))
        .chain(project.music.iter().map(|m| m.file.clone()))
        .collect()
}

fn reload(open: &mut Open) {
    open.assets = Assets::load(&open.dir, &open.project);
    open.tileset = None;
    open.sprite_pictures.clear();
    open.pictures.clear();
    open.generation += 1;
}

fn ensure_tileset(open: &mut Open, ctx: &egui::Context) {
    if open.tileset.is_none() {
        if let Ok(tileset) = &open.assets.tileset {
            open.tileset =
                Some(ctx.load_texture("tileset", rgba_image(tileset), TextureOptions::NEAREST));
        }
    }
}

fn ensure_sprites(open: &mut Open, ctx: &egui::Context) {
    if open.sprite_pictures.len() == open.assets.sprites.len() {
        return;
    }
    open.sprite_pictures = open
        .assets
        .sprites
        .iter()
        .enumerate()
        .map(|(i, sprite)| {
            let image = sprite.as_ref().ok()?;
            let texture = ctx.load_texture(
                format!("sprite {i}"),
                rgba_image(image),
                TextureOptions::NEAREST,
            );
            let tile = TILE as f32;
            let uv = Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(tile / image.width as f32, tile / image.height as f32),
            );
            Some((texture, uv))
        })
        .collect();
}

/// The three layers side by side in a pill, the chosen one lit.
fn layer_choice(ui: &mut Ui, layer: &mut LayerKind) {
    pill().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (kind, label, hint) in LAYERS {
                let on = *layer == kind;
                let look = Frame::new()
                    .fill(if on { SELECT } else { Color32::TRANSPARENT })
                    .corner_radius(12)
                    .inner_margin(Margin::symmetric(10, 3));
                let segment = surface(ui, look, (!on).then_some(HOVER), |ui| {
                    ui.label(RichText::new(label).color(if on { TEXT } else { MUTED }));
                });
                if segment.on_hover_text(hint).clicked() {
                    *layer = kind;
                }
            }
        });
    });
}

/// How often tile `number` is used on all maps, and on how many maps.
fn usage(project: &Project, number: u16) -> (usize, usize) {
    let counts = project.maps.iter().map(|map| {
        LayerKind::ALL
            .iter()
            .flat_map(|&kind| map.layer(kind).0.iter().flatten())
            .filter(|&&n| n == number)
            .count()
    });
    counts.fold((0, 0), |(uses, maps), n| {
        (uses + n, maps + (n > 0) as usize)
    })
}

/// A dialog window: like the panels, a step lighter.
fn dialog_frame() -> Frame {
    panel_frame()
        .fill(PANEL2)
        .inner_margin(Margin::same(18))
        .stroke(Stroke::new(1.0, LINE))
}

/// A dialog's heading with a button to close it.
fn dialog_title(ui: &mut Ui, title: &str, close: &mut bool) {
    ui.horizontal(|ui| {
        heading(ui, title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(quiet(RichText::new(CROSS).size(16.0)))
                .on_hover_text("Close (Esc)")
                .clicked()
            {
                *close = true;
            }
        });
    });
    ui.add_space(10.0);
}

/// A map drawn into a picture, made again only when something it shows changed.
fn map_picture(open: &mut Open, index: usize, view: View, ctx: &egui::Context) -> TextureHandle {
    let project = &open.project;
    let map = &project.maps[index];
    let mut hasher = DefaultHasher::new();
    (index, view.code(), open.generation).hash(&mut hasher);
    for kind in LayerKind::ALL {
        map.layer(kind).hash(&mut hasher);
    }
    for tile in &project.tiles {
        tile.cells.hash(&mut hasher);
    }
    for thing in &map.things {
        (thing.thing.at, thing.thing.look.map(|l| l.0)).hash(&mut hasher);
    }
    let key = hasher.finish();
    let slot = (index, view.code());
    if let Some((made, texture)) = open.pictures.get(&slot) {
        if *made == key {
            return texture.clone();
        }
    }

    let (w, h) = (map.width() as usize * TILE, map.height() as usize * TILE);
    // Without the ground, a checkerboard shows what's empty
    let mut out: Vec<[u8; 4]> = match view {
        _ if view.show(LayerKind::Ground) == Show::Off => (0..w * h)
            .map(|i| {
                if (i % w / 4 + i / w / 4) % 2 == 0 {
                    [52, 52, 56, 255]
                } else {
                    [70, 70, 74, 255]
                }
            })
            .collect(),
        _ => vec![[24, 24, 28, 255]; w * h],
    };
    let pictures: Vec<Option<Vec<u16>>> = project
        .tiles
        .iter()
        .map(|tile| {
            open.assets
                .tileset
                .as_ref()
                .ok()?
                .cell(*tile.cells.first()? as usize)
        })
        .collect();
    // With objects, the tiles step back
    let dim = if view == View::Objects { 0.5 } else { 1.0 };
    let mut draw = |pixels: &[u16], x: usize, y: usize, alpha: f32, bright: f32| {
        for (i, &pixel) in pixels.iter().enumerate() {
            if pixel == TRANSPARENT {
                continue;
            }
            let [r, g, b, _] = images::to_rgba(pixel);
            let [r, g, b] = [r, g, b].map(|c| (c as f32 * bright) as u8);
            let target = &mut out[(y + i / TILE) * w + x + i % TILE];
            for (c, new) in target.iter_mut().zip([r, g, b]) {
                *c = (*c as f32 * (1.0 - alpha) + new as f32 * alpha) as u8;
            }
        }
    };
    let alpha = |kind: LayerKind| match view {
        View::Objects if kind == LayerKind::Above => 0.35,
        _ if view.show(kind) == Show::Faint => 0.25,
        _ => 1.0,
    };
    for kind in LayerKind::ALL {
        let a = alpha(kind);
        if view.show(kind) != Show::Off {
            for (y, row) in map.layer(kind).0.iter().enumerate() {
                for (x, &n) in row.iter().enumerate() {
                    if let Some(Some(pixels)) =
                        n.checked_sub(1).and_then(|t| pictures.get(t as usize))
                    {
                        draw(pixels, x * TILE, y * TILE, a, dim);
                    }
                }
            }
        }
        // The objects stand with the decoration, also while it's turned off
        if kind == LayerKind::Decoration {
            for thing in &map.things {
                let picture = thing.thing.look.and_then(|look| {
                    open.assets
                        .sprites
                        .get(look.0 as usize)?
                        .as_ref()
                        .ok()?
                        .cell(0)
                });
                if let Some(pixels) = picture {
                    let a = if view == View::Objects { 1.0 } else { a };
                    draw(
                        &pixels,
                        thing.thing.at.x as usize * TILE,
                        thing.thing.at.y as usize * TILE,
                        a,
                        1.0,
                    );
                }
            }
        }
    }
    let image = ColorImage::from_rgba_unmultiplied([w, h], out.as_flattened());
    let options = if view == View::Whole {
        TextureOptions::LINEAR
    } else {
        TextureOptions::NEAREST
    };
    let texture = match open.pictures.remove(&slot) {
        Some((_, mut texture)) => {
            texture.set(image, options);
            texture
        }
        None => ctx.load_texture(format!("map {index} {}", view.code()), image, options),
    };
    open.pictures.insert(slot, (key, texture.clone()));
    texture
}

/// Copies `file` into `folder` under a plain name of its own ending in `.extension`; that name,
/// and a name for people made from the file's.
fn copy_in(file: &Path, folder: &Path, extension: &str) -> Result<(String, String), String> {
    let stem = file
        .file_stem()
        .map_or("file".into(), |s| s.to_string_lossy().to_string());
    let name = project::file_name(folder, &stem, extension);
    std::fs::copy(file, folder.join(&name))
        .map_err(|error| format!("Can't copy {}: {error}", file.display()))?;
    let mut pretty: String = stem.replace(['_', '-'], " ");
    if let Some(c) = pretty.get(0..1) {
        pretty = c.to_uppercase() + &pretty[1..];
    }
    Ok((name, pretty))
}

/// A square button with an icon, for listening to a sound.
fn listen_button(ui: &mut Ui, icon: &str, broken: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
    let response = if broken {
        response
    } else {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    };
    let fill = if response.hovered() && !broken {
        HOVER
    } else {
        PANEL
    };
    ui.painter().rect_filled(rect, things::IN_CARD, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(20.0),
        if broken { WARN } else { TEXT },
    );
    response
}

/// The shape of a music part's tone.
fn wave_choice(ui: &mut Ui, salt: &str, wave: &mut adventure_format::Wave) {
    use adventure_format::Wave;
    let name = |wave: Wave| match wave {
        Wave::Square => "Square: bright",
        Wave::Triangle => "Triangle: soft",
        Wave::Sine => "Sine: softest",
    };
    egui::ComboBox::from_id_salt(salt)
        .width(120.0)
        .selected_text(name(*wave))
        .show_ui(ui, |ui| {
            for choice in [Wave::Square, Wave::Triangle, Wave::Sine] {
                ui.selectable_value(wave, choice, name(choice));
            }
        });
}

/// How long `ms` is, for people: tenths of a second for short things.
fn seconds(ms: u32) -> String {
    let seconds = ms as f32 / 1000.0;
    if seconds < 10.0 {
        format!(" · {seconds:.1} s")
    } else {
        format!(" · {seconds:.0} s")
    }
}
