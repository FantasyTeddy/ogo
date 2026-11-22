//! The Adventure editor: makes worlds on the PC. A world is a project folder (see [`project`]);
//! the editor paints its maps with the tiles of a PNG tileset, plays it with the engine, and
//! exports it as an `.adv` file or sends it straight to the device.
//!
//! `cargo run --manifest-path adventure/Cargo.toml --target host-tuple -p adventure-editor
//! [-- <folder>]`: without a folder, the one open last time.

mod export;
mod images;
mod project;
mod send;
mod theme;

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use adventure_engine::{Buttons, Game, FPS, HEIGHT, WIDTH};
use adventure_format::{Dir, LayerKind, MusicRef, Place, Pos, SoundRef, TILE, TRANSPARENT};
use eframe::egui::{
    self, load::SizedTexture, pos2, vec2, Align, Align2, Color32, ColorImage, CornerRadius, Frame,
    Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Popup, PopupCloseBehavior, Rect,
    RectAlign, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui,
};

use export::Assets;
use images::Rgba;
use project::{Project, TileDef};
use theme::*;

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
    Pencil,
    Fill,
    Box,
    Eraser,
    Start,
}

const TOOLS: [(Tool, &str, &str, &str); 5] = [
    (
        Tool::Pencil,
        PENCIL,
        "Pencil",
        "Paint the selected tile. Right-click picks a tile up",
    ),
    (Tool::Fill, FILL, "Fill", "Fill the area of same tiles"),
    (Tool::Box, BOX, "Box", "Drag to fill a rectangle"),
    (
        Tool::Eraser,
        ERASER,
        "Eraser",
        "Empty cells of the layer you're painting on",
    ),
    (Tool::Start, FLAG, "Start", "Click where the player starts"),
];

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
        "Above the player",
        "Treetops, roofs: drawn over everyone",
    ),
];

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
}

/// What to do once the unsaved changes are saved or dropped.
#[derive(Debug, Clone, Copy)]
enum Pending {
    New,
    Open,
    Close,
}

/// How a map is drawn into a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// For painting: the layers above this one see-through
    Painting(LayerKind),
    /// Everything, for the map's tab
    Whole,
    /// One layer alone, for the layer bar
    Layer(LayerKind),
}

impl View {
    fn code(self) -> u8 {
        match self {
            View::Painting(layer) => layer as u8,
            View::Whole => 10,
            View::Layer(layer) => 20 + layer as u8,
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
}

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

struct Editor {
    open: Option<Open>,
    map: usize,
    layer: LayerKind,
    tool: Tool,
    tile: usize,
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
    device: String,
    sending: Option<mpsc::Receiver<Result<String, String>>>,
    /// A check of the device running: for which address
    checking: Option<(String, mpsc::Receiver<Result<String, String>>)>,
    /// The last check's result, and for which address
    checked: Option<(String, Result<String, String>)>,
    pending: Option<Pending>,
    closing: bool,
    /// Where the open map is on the screen, for the zoom and the messages
    sheet: Rect,
    /// Pick the largest zoom that shows the whole map, next time it's drawn
    fit: bool,
}

const UNDO_STEPS: usize = 200;
const CHECK_PICTURES: Duration = Duration::from_secs(1);

impl Editor {
    fn new(cc: &eframe::CreationContext) -> Self {
        let stored = |key: &str| cc.storage.and_then(|s| s.get_string(key));
        let mut editor = Editor {
            open: None,
            map: 0,
            layer: LayerKind::Ground,
            tool: Tool::Pencil,
            tile: 0,
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
            sending: None,
            checking: None,
            checked: None,
            pending: None,
            closing: false,
            sheet: Rect::NOTHING,
            fit: true,
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
            (self.map, self.tile, self.layer as u8, self.tool as u8),
            (self.dialog, self.zoom as u8, self.grid, self.play.is_some()),
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
            Pending::New => {
                // A save dialog, so a new name can be typed in: the world gets a folder of that name
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Name of the new world")
                    .set_file_name("My world");
                if let Some(parent) = self.open.as_ref().and_then(|open| open.dir.parent()) {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(dir) = dialog.save_file() {
                    match export::write_starter(&dir) {
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
            open.redo.push(std::mem::replace(&mut open.project, before));
            open.committed = open.project.clone();
        }
        self.keep_selection_valid();
    }

    fn redo(&mut self) {
        let Some(open) = &mut self.open else { return };
        if let Some(after) = open.redo.pop() {
            open.undo.push(std::mem::replace(&mut open.project, after));
            open.committed = open.project.clone();
        }
        self.keep_selection_valid();
    }

    fn keep_selection_valid(&mut self) {
        if let Some(open) = &self.open {
            self.map = self.map.min(open.project.maps.len().saturating_sub(1));
            self.tile = self.tile.min(open.project.tiles.len().saturating_sub(1));
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
            self.say("Pictures reloaded");
        }
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
        let Some(world) = self.world() else { return };
        let (address, name, bytes) = (
            self.device.clone(),
            send::file_name(&world.title),
            world.to_bytes(),
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(send::send(&address, &name, &bytes));
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
                    Err(error) => self.fail(format!("Not sent: {error}")),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => self.sending = None,
        }
    }

    /// Looks for the device at the address typed in, unless that was done already.
    fn check_device(&mut self) {
        let address = self.device.trim().to_string();
        let done = self.checked.as_ref().is_some_and(|(a, _)| *a == address);
        let running = self.checking.as_ref().is_some_and(|(a, _)| *a == address);
        if done || running || address.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let target = address.clone();
        std::thread::spawn(move || {
            let _ = tx.send(send::check(&target));
        });
        self.checking = Some((address, rx));
    }

    fn poll_check(&mut self, ctx: &egui::Context) {
        let Some((address, rx)) = &self.checking else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.checked = Some((address.clone(), result));
                self.checking = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => self.checking = None,
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
        match Game::new(world, 0) {
            Ok(game) => {
                self.play = Some(Play {
                    game,
                    title,
                    picture: vec![0; WIDTH * HEIGHT],
                    texture: None,
                    last: Instant::now(),
                    behind: 0.0,
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
                if ui.button(format!("{NEW}   New world…")).clicked() {
                    action = Some(Pending::New);
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
                let button = ui.add_enabled(
                    self.sending.is_none(),
                    egui::Button::new(format!("{WIFI}  Send…")),
                );
                Popup::from_toggle_button_response(&button)
                    .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                    .align(RectAlign::BOTTOM_END)
                    .width(320.0)
                    .show(|ui| {
                        ui.label(muted("The device's address · on its WiFi screen"));
                        text_input(ui, &mut self.device, "e.g. 192.168.1.23");
                        self.check_device();
                        let address = self.device.trim().to_string();
                        let result = self
                            .checked
                            .as_ref()
                            .filter(|(a, _)| *a == address)
                            .map(|(_, r)| r.clone());
                        let found = matches!(result, Some(Ok(_)));
                        match result {
                            _ if address.is_empty() => {}
                            None => {
                                ui.label(muted("Looking for it…"));
                            }
                            Some(Ok(message)) => {
                                ui.label(RichText::new(format!("●  {message}")).color(OK));
                            }
                            Some(Err(message)) => {
                                ui.label(
                                    RichText::new(format!("{WARNING}  {message}")).color(WARN),
                                );
                            }
                        }
                        let label = format!("{WIFI}  Send \"{title}\"");
                        let width = ui.available_width();
                        if accent(ui, found, primary(label).min_size(vec2(width, 32.0))).clicked() {
                            send = true;
                            ui.close();
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

    fn tool_strip(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (tool, icon, label, hint) in TOOLS {
                if theme::tool(ui, icon, label, self.tool == tool)
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.tool = tool;
                }
            }
        });
    }

    fn palette(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        ensure_tileset(open, &ctx);
        let mut new_tile = false;
        ui.horizontal(|ui| {
            heading(ui, "Tiles");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(quiet(RichText::new(PLUS).size(17.0)))
                    .on_hover_text("New tile")
                    .clicked()
                {
                    new_tile = true;
                }
            });
        });
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
            project.tiles.push(TileDef {
                name: format!("Tile {}", project.tiles.len() + 1),
                cells: vec![cell],
                frame_ms: 300,
                solid: false,
                step_sound: None,
            });
        };
        if new_tile {
            add(&mut open.project, unused.first().copied().unwrap_or(0));
            self.tile = open.project.tiles.len() - 1;
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let gap = 8.0;
                let side = ((ui.available_width() - 3.0 * gap) / 4.0).floor();
                ui.spacing_mut().item_spacing = vec2(gap, gap);
                ui.horizontal_wrapped(|ui| {
                    for (i, tile) in open.project.tiles.iter().enumerate() {
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(side, side), Sense::click());
                        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                        let cell = tile.cells.first().copied().unwrap_or(0);
                        ui.painter().rect_filled(rect, 10, PANEL2);
                        egui::Image::from_texture(SizedTexture::new(
                            tileset.id(),
                            vec2(side, side),
                        ))
                        .uv(cell_uv(size, cell))
                        .corner_radius(10)
                        .paint_at(ui, rect);
                        if i == self.tile {
                            ring(ui, rect, 12);
                        } else if response.hovered() {
                            ui.painter().rect_stroke(
                                rect,
                                10,
                                Stroke::new(1.5, MUTED),
                                StrokeKind::Outside,
                            );
                        }
                        if response.on_hover_text(&tile.name).clicked() {
                            self.tile = i;
                            if matches!(self.tool, Tool::Eraser | Tool::Start) {
                                self.tool = Tool::Pencil;
                            }
                        }
                    }
                });
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
    }

    fn inspector(&mut self, ui: &mut Ui) {
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
                header(ui, "Tile", "a kind of tile, wherever it's used");
                let Some(tile) = open.project.tiles.get_mut(self.tile) else {
                    ui.label(muted("No tile selected."));
                    return;
                };
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                    ui.painter().rect_filled(rect, 12, PANEL2);
                    egui::Image::from_texture(SizedTexture::new(tileset.id(), vec2(72.0, 72.0)))
                        .uv(cell_uv(
                            tileset_size,
                            tile.cells.first().copied().unwrap_or(0),
                        ))
                        .corner_radius(12)
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
                            ui.painter().rect_filled(rect, 8, PANEL2);
                            egui::Image::from_texture(SizedTexture::new(
                                tileset.id(),
                                vec2(36.0, 36.0),
                            ))
                            .uv(cell_uv(tileset_size, cell))
                            .corner_radius(8)
                            .paint_at(ui, rect);
                            if removable {
                                let response = response
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("Remove this picture");
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        8,
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
                }
            });
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
                        ui.set_width(150.0);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (s, icon, label) in [
                            (Section::General, INFO, "General"),
                            (Section::Pictures, IMAGE, "Pictures"),
                        ] {
                            let on = s == section;
                            let look = Frame::new()
                                .fill(if on { SELECT } else { Color32::TRANSPARENT })
                                .corner_radius(12)
                                .inner_margin(Margin::symmetric(10, 7));
                            let item = surface(ui, look, (!on).then_some(HOVER), |ui| {
                                ui.set_width(130.0);
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
                        // As tall as the tallest section, so switching doesn't jump
                        ui.set_min_height(430.0);
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
                            Section::Pictures => {
                                field(ui, "Tileset", |ui| {
                                    if let Some(tileset) = &open.tileset {
                                        let size = tileset.size_vec2() * 2.0;
                                        let scale = (440.0 / size.x).min(1.0);
                                        ui.add(
                                            egui::Image::from_texture(SizedTexture::new(
                                                tileset.id(),
                                                size * scale,
                                            ))
                                            .corner_radius(8),
                                        );
                                    }
                                    ui.label(
                                        muted(
                                            "tiles.png in the world's folder: 16×16 tiles side by \
                                             side. Paint it in any program; it reloads by itself.",
                                        )
                                        .small(),
                                    );
                                    if ui
                                        .button(format!("{IMAGE}  Import tileset…"))
                                        .on_hover_text(
                                            "Replaces tiles.png; the tiles keep their cells",
                                        )
                                        .clicked()
                                    {
                                        import = true;
                                    }
                                });
                                ui.add_space(6.0);
                                field(ui, "Characters", |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for (def, picture) in
                                            open.project.sprites.iter().zip(&open.sprite_pictures)
                                        {
                                            ui.vertical(|ui| {
                                                ui.set_width(64.0);
                                                let (rect, _) = ui.allocate_exact_size(
                                                    vec2(48.0, 48.0),
                                                    Sense::hover(),
                                                );
                                                ui.painter().rect_filled(rect, 10, PANEL);
                                                if let Some((texture, uv)) = picture {
                                                    egui::Image::from_texture(SizedTexture::new(
                                                        texture.id(),
                                                        rect.size(),
                                                    ))
                                                    .uv(*uv)
                                                    .paint_at(ui, rect);
                                                }
                                                ui.label(muted(&def.name).small());
                                            });
                                        }
                                    });
                                    ui.label(
                                        muted(
                                            "One PNG each in sprites/: a row per direction \
                                             (down, up, left, right), a column per picture.",
                                        )
                                        .small(),
                                    );
                                });
                            }
                        }
                    });
                });
            });
        if let Some(section) = go {
            self.dialog = Some(Dialog::World(section));
        }
        if import {
            self.import_tileset();
        }
        if close || modal.should_close() {
            self.dialog = None;
            self.thumbnail = None;
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
                self.layer_bar(ui);
                self.canvas(ui);
            });
        self.sheet = sheet.response.rect;
        self.zoom_control(&ctx);
    }

    fn layer_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label(muted("Painting on"));
            Frame::new()
                .fill(PANEL)
                .corner_radius(PILL)
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (n, (layer, label, hint)) in LAYERS.into_iter().enumerate() {
                            if n > 0 {
                                ui.label(RichText::new(CARET_RIGHT).color(MUTED).size(13.0));
                            }
                            let on = self.layer == layer;
                            let preview = map_picture(open, self.map, View::Layer(layer), &ctx);
                            let look = Frame::new()
                                .fill(if on { SELECT } else { Color32::TRANSPARENT })
                                .corner_radius(12)
                                .inner_margin(Margin {
                                    left: 4,
                                    right: 10,
                                    top: 3,
                                    bottom: 3,
                                });
                            let hover = (!on).then_some(HOVER);
                            let segment = surface(ui, look, hover, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let (rect, _) =
                                        ui.allocate_exact_size(vec2(30.0, 20.0), Sense::hover());
                                    egui::Image::from_texture(SizedTexture::new(
                                        preview.id(),
                                        rect.size(),
                                    ))
                                    .uv(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))
                                    .corner_radius(4)
                                    .paint_at(ui, rect);
                                    ui.label(RichText::new(label).color(if on {
                                        TEXT
                                    } else {
                                        MUTED
                                    }));
                                });
                            });
                            if segment.on_hover_text(hint).clicked() {
                                chosen = Some(layer);
                            }
                        }
                    });
                });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label("Grid");
                switch(ui, &mut self.grid);
            });
        });
        if let Some(layer) = chosen {
            self.layer = layer;
        }
    }

    fn canvas(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        let texture = map_picture(open, self.map, View::Painting(self.layer), &ctx);
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
            let response = ui
                .interact(
                    rect,
                    ui.scope_id().with("map canvas"),
                    Sense::click_and_drag(),
                )
                // A crosshair to paint with: a cell, not a button
                .on_hover_cursor(egui::CursorIcon::Crosshair);
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
            if let Some(at) = hover {
                painter.rect_stroke(cell_rect(at), 3, Stroke::new(1.5, TEXT), StrokeKind::Inside);
            }
            if let (Tool::Box, Some(from), Some(to)) = (tool, rect_from, hover) {
                painter.rect_stroke(
                    cell_rect(from).union(cell_rect(to)),
                    3,
                    Stroke::new(2.0, TEXT),
                    StrokeKind::Inside,
                );
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
        self.apply(input);
    }

    fn zoom_control(&mut self, ctx: &egui::Context) {
        if self.sheet == Rect::NOTHING {
            return;
        }
        egui::Area::new(Id::NULL.with("zoom"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(self.sheet.right_bottom() - vec2(14.0, 14.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(PANEL)
                    .corner_radius(PILL)
                    .inner_margin(Margin::same(3))
                    .stroke(Stroke::new(1.0, LINE))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if ui
                                .add_enabled(self.zoom > 1.0, quiet(MINUS))
                                .on_hover_text("Zoom out")
                                .clicked()
                            {
                                self.zoom -= 1.0;
                            }
                            ui.label(muted(format!("{}×", self.zoom as u32)));
                            if ui
                                .add_enabled(self.zoom < 6.0, quiet(PLUS))
                                .on_hover_text("Zoom in")
                                .clicked()
                            {
                                self.zoom += 1.0;
                            }
                        });
                    });
            });
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
        let Some(open) = &mut self.open else { return };
        let Some(at) = input.cell else {
            if input.drag_stopped {
                self.rect_from = None;
            }
            return;
        };
        let tile = self.tile as u16 + 1;
        let has_tiles = !open.project.tiles.is_empty();
        if input.clicked || input.drag_started {
            self.cursor = Some(at);
        }
        let map = &mut open.project.maps[self.map];
        if input.picked {
            if let Some(n) = map.layer(self.layer).get(at).filter(|&n| n > 0) {
                self.tile = n as usize - 1;
                if matches!(self.tool, Tool::Eraser | Tool::Start) {
                    self.tool = Tool::Pencil;
                }
            }
            return;
        }
        if self.tool == Tool::Start {
            if input.clicked {
                open.project.start = Place {
                    map: map.id,
                    at,
                    facing: Dir::Down,
                };
            }
            return;
        }
        let layer = map.layer_mut(self.layer);
        match self.tool {
            Tool::Pencil if input.painting && has_tiles => layer.set(at, tile),
            Tool::Eraser if input.painting => layer.set(at, 0),
            Tool::Fill if input.clicked && has_tiles => layer.flood(at, tile),
            Tool::Box => {
                if input.drag_started {
                    self.rect_from = Some(at);
                }
                if input.drag_stopped {
                    if let Some(from) = self.rect_from.take() {
                        layer.rect(from, at, tile);
                    }
                } else if input.clicked {
                    layer.set(at, tile);
                }
            }
            _ => {}
        }
    }

    /// The game in a window of its own, so the map stays in view.
    fn play_window(&mut self, ctx: &egui::Context) {
        let Some(play) = &mut self.play else { return };
        let mut stop = false;
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("Playing {}", play.title))
            .with_inner_size([WIDTH as f32 * 4.0 + 24.0, HEIGHT as f32 * 4.0 + 56.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("play"), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
                stop = true;
            }
            let now = Instant::now();
            play.behind += now.duration_since(play.last).as_secs_f64();
            play.last = now;
            let buttons = ctx.input(|i| Buttons {
                up: i.key_down(Key::ArrowUp),
                down: i.key_down(Key::ArrowDown),
                left: i.key_down(Key::ArrowLeft),
                right: i.key_down(Key::ArrowRight),
                a: i.key_down(Key::Space) || i.key_down(Key::Enter),
                b: i.modifiers.shift,
            });
            let step = 1.0 / FPS as f64;
            let mut steps = 0;
            while play.behind >= step && steps < 5 {
                play.game.step(buttons);
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
                    None => play.texture = Some(ctx.load_texture("play", image, TextureOptions::NEAREST)),
                }
            }
            egui::CentralPanel::default().frame(Frame::new().fill(BG).inner_margin(Margin::same(12))).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(texture) = &play.texture {
                        let available = ui.available_size() - vec2(0.0, 28.0);
                        let zoom = (available.x / WIDTH as f32).min(available.y / HEIGHT as f32).floor().max(1.0);
                        ui.add(egui::Image::from_texture(SizedTexture::new(texture.id(), vec2(WIDTH as f32 * zoom, HEIGHT as f32 * zoom))).corner_radius(8));
                    }
                    ui.add_space(6.0);
                    ui.label(muted("Arrows walk · Space is A · Shift runs · Esc closes · no sound in the editor").small());
                });
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
        self.poll_check(&ctx);
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
            egui::Panel::left("tools")
                .resizable(false)
                .exact_size(86.0)
                .frame(
                    panel_frame()
                        .inner_margin(Margin::symmetric(6, 12))
                        .outer_margin(gap(10, 0)),
                )
                .show(ui, |ui| self.tool_strip(ui));
            egui::Panel::left("palette")
                .resizable(false)
                .exact_size(222.0)
                .frame(panel_frame().outer_margin(gap(10, 0)))
                .show(ui, |ui| self.palette(ui));
            egui::Panel::right("inspector")
                .resizable(false)
                .exact_size(284.0)
                .frame(panel_frame().outer_margin(gap(0, 10)))
                .show(ui, |ui| self.inspector(ui));
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
    let mut out: Vec<[u8; 4]> = match view {
        View::Layer(_) => (0..w * h)
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
    let mut draw = |pixels: &[u16], x: usize, y: usize, alpha: f32| {
        for (i, &pixel) in pixels.iter().enumerate() {
            if pixel == TRANSPARENT {
                continue;
            }
            let [r, g, b, _] = images::to_rgba(pixel);
            let target = &mut out[(y + i / TILE) * w + x + i % TILE];
            for (c, new) in target.iter_mut().zip([r, g, b]) {
                *c = (*c as f32 * (1.0 - alpha) + new as f32 * alpha) as u8;
            }
        }
    };
    // While painting, the layers above see-through, so what's being painted shows
    let alpha = |kind: LayerKind| match view {
        View::Painting(active) if kind as u8 > active as u8 => Some(0.3),
        View::Layer(only) if kind != only => None,
        _ => Some(1.0),
    };
    for kind in LayerKind::ALL {
        let Some(a) = alpha(kind) else { continue };
        for (y, row) in map.layer(kind).0.iter().enumerate() {
            for (x, &n) in row.iter().enumerate() {
                if let Some(Some(pixels)) = n.checked_sub(1).and_then(|t| pictures.get(t as usize))
                {
                    draw(pixels, x * TILE, y * TILE, a);
                }
            }
        }
        if kind == LayerKind::Decoration && !matches!(view, View::Layer(_)) {
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
                    draw(
                        &pixels,
                        thing.thing.at.x as usize * TILE,
                        thing.thing.at.y as usize * TILE,
                        a,
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
