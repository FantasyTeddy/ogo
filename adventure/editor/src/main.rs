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

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use adventure_engine::{Buttons, Game, FPS, HEIGHT, WIDTH};
use adventure_format::{Dir, LayerKind, MusicRef, Place, Pos, SoundRef, TILE, TRANSPARENT};
use eframe::egui::{
    self, load::SizedTexture, pos2, vec2, Color32, ColorImage, Key, KeyboardShortcut, Modifiers,
    Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions,
};

use export::Assets;
use images::Rgba;
use project::{Project, TileDef};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Adventure editor"),
        ..Default::default()
    };
    eframe::run_native(
        "Adventure editor",
        options,
        Box::new(|cc| Ok(Box::new(Editor::new(cc)))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pencil,
    Fill,
    Rect,
    Eraser,
    Start,
}

/// What to do once the unsaved changes are saved or dropped.
#[derive(Debug, Clone, Copy)]
enum Pending {
    New,
    Open,
    Close,
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
    sprites: Vec<Option<TextureHandle>>,
    /// The map as one picture, and what it was made from
    map: Option<(u64, TextureHandle)>,
    /// Counts reloads of the pictures
    generation: u64,
    /// When the picture files were last looked at, and their times then
    checked: Instant,
    times: Vec<Option<SystemTime>>,
}

struct Play {
    game: Game,
    picture: Vec<u16>,
    texture: Option<TextureHandle>,
    last: Instant,
    /// Time not stepped yet, in seconds
    behind: f64,
}

struct Editor {
    open: Option<Open>,
    map: usize,
    layer: LayerKind,
    tool: Tool,
    tile: usize,
    dim: bool,
    grid: bool,
    zoom: f32,
    /// Where a rectangle being drawn started
    rect_from: Option<Pos>,
    /// The cell clicked last, where "Play here" starts
    cursor: Option<Pos>,
    /// The size being typed in for the map: (map, width, height)
    size: Option<(usize, u16, u16)>,
    play: Option<Play>,
    status: String,
    error: bool,
    device: String,
    sending: Option<mpsc::Receiver<Result<String, String>>>,
    pending: Option<Pending>,
    closing: bool,
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
            dim: false,
            grid: true,
            zoom: 2.0,
            rect_from: None,
            cursor: None,
            size: None,
            play: None,
            status: "Make a new world, or open one.".into(),
            error: false,
            device: stored("device").unwrap_or_default(),
            sending: None,
            pending: None,
            closing: false,
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

    fn say(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.error = false;
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.error = true;
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
        self.cursor = None;
        self.size = None;
        self.play = None;
        self.say(format!("Opened {}", dir.display()));
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
            sprites: Vec::new(),
            map: None,
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
                let message = format!("Saved {}", open.dir.join(project::FILE).display());
                self.say(message);
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
            return self.fail(format!("can't copy to {}: {error}", target.display()));
        }
        reload(open);
        open.times = picture_times(&open.dir, &open.project);
        match &open.assets.tileset {
            Ok(_) => self.say(format!("Imported {}", file.display())),
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
            Err(error) => self.fail(format!("can't write {}: {error}", path.display())),
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
            Ok(Ok(message)) => {
                self.sending = None;
                self.say(format!("Sent: {message}"));
            }
            Ok(Err(error)) => {
                self.sending = None;
                self.fail(format!("Not sent: {error}"));
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => self.sending = None,
        }
    }

    fn start_play(&mut self, here: bool, ctx: &egui::Context) {
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
        match Game::new(world, 0) {
            Ok(game) => {
                self.play = Some(Play {
                    game,
                    picture: vec![0; WIDTH * HEIGHT],
                    texture: None,
                    last: Instant::now(),
                    behind: 0.0,
                });
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.say("Playing: the arrows walk, Space is A, Shift runs, Esc stops. No sound in the editor.");
            }
            Err(error) => self.fail(error.to_string()),
        }
    }

    // ---- The window -------------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.play.is_some() || ctx.memory(|m| m.focused().is_some()) {
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

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal_wrapped(|ui| {
            if ui.button("New…").clicked() {
                self.ask(Pending::New, &ctx);
            }
            if ui.button("Open…").clicked() {
                self.ask(Pending::Open, &ctx);
            }
            let dirty = self.dirty();
            if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                self.save();
            }
            let Some(open) = &self.open else { return };
            let (can_undo, can_redo) = (!open.undo.is_empty(), !open.redo.is_empty());
            ui.label(format!(
                "{}{}",
                open.project.title,
                if dirty { " •" } else { "" }
            ));
            ui.separator();

            if self.play.is_some() {
                if ui.button("■ Stop").clicked() {
                    self.play = None;
                }
                return;
            }
            if ui
                .add_enabled(can_undo, egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(can_redo, egui::Button::new("Redo"))
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            for (tool, label, hint) in [
                (
                    Tool::Pencil,
                    "✏ Pencil",
                    "Paint the selected tile; right-click picks a tile up",
                ),
                (Tool::Fill, "🪣 Fill", "Fill the area of same tiles"),
                (Tool::Rect, "▭ Rectangle", "Drag a rectangle"),
                (Tool::Eraser, "Eraser", "Empty cells of this layer"),
                (
                    Tool::Start,
                    "Start",
                    "Where the player starts: click a cell",
                ),
            ] {
                ui.selectable_value(&mut self.tool, tool, label)
                    .on_hover_text(hint);
            }
            ui.separator();
            ui.label("Layer:");
            for (layer, label) in [
                (LayerKind::Ground, "Ground"),
                (LayerKind::Decoration, "Decoration"),
                (LayerKind::Above, "Above"),
            ] {
                ui.selectable_value(&mut self.layer, layer, label);
            }
            ui.checkbox(&mut self.dim, "Dim others");
            ui.checkbox(&mut self.grid, "Grid");
            ui.label(format!("Zoom {}×", self.zoom as u32));
            ui.spacing_mut().slider_width = 70.0;
            ui.add(
                egui::Slider::new(&mut self.zoom, 1.0..=6.0)
                    .step_by(1.0)
                    .show_value(false),
            );
            ui.separator();
            if ui
                .button("▶ Play")
                .on_hover_text("From the world's start")
                .clicked()
            {
                self.start_play(false, &ctx);
            }
            let here = self.cursor.is_some();
            if ui
                .add_enabled(here, egui::Button::new("▶ Here"))
                .on_hover_text("From the cell clicked last")
                .clicked()
            {
                self.start_play(true, &ctx);
            }
            ui.separator();
            if ui.button("Export…").clicked() {
                self.export();
            }
            let sending = self.sending.is_some();
            if ui
                .add_enabled(!sending, egui::Button::new("📶 Send to device"))
                .on_hover_text("Uploads the world to the Adventure app over WiFi")
                .clicked()
            {
                self.send();
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.device)
                    .hint_text("device address")
                    .desired_width(120.0),
            );
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let colour = if self.error {
            Color32::from_rgb(220, 70, 60)
        } else {
            ui.visuals().text_color()
        };
        ui.colored_label(colour, &self.status);
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            ui.heading("Adventure editor");
            ui.label("A world is a folder: world.json, the tileset tiles.png and the characters in sprites/.");
            ui.add_space(20.0);
            if ui.button("New world…").on_hover_text("Creates a folder of the name you type, starting from the sample village").clicked() {
                self.ask(Pending::New, &ctx);
            }
            if ui.button("Open world…").clicked() {
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
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Save the changes to this world first?");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        answer = Some(1);
                    }
                    if ui.button("Don't save").clicked() {
                        answer = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
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

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        ensure_textures(open, &ctx);

        ui.heading("Maps");
        for (i, map) in open.project.maps.iter().enumerate() {
            let label = if open.project.start.map == map.id {
                format!("{} (start)", map.name)
            } else {
                map.name.clone()
            };
            if ui.selectable_label(i == self.map, label).clicked() {
                self.map = i;
                self.cursor = None;
            }
        }
        if ui.button("+ New map").clicked() {
            let fill = if open.project.tiles.is_empty() {
                0
            } else {
                self.tile as u16 + 1
            };
            self.map = open.project.add_map(20, 15, fill);
            self.cursor = None;
        }

        ui.separator();
        ui.heading("Tiles");
        let Some(tileset) = open.tileset.clone() else {
            ui.label("The tileset can't be read.");
            return;
        };
        let size = tileset.size_vec2();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, tile) in open.project.tiles.iter().enumerate() {
                    let cell = tile.cells.first().copied().unwrap_or(0);
                    let image = egui::Image::from_texture(SizedTexture::new(
                        tileset.id(),
                        vec2(32.0, 32.0),
                    ))
                    .uv(cell_uv(size, cell));
                    let button = ui
                        .add(egui::Button::image(image).selected(i == self.tile))
                        .on_hover_text(&tile.name);
                    if button.clicked() {
                        self.tile = i;
                        if matches!(self.tool, Tool::Eraser | Tool::Start) {
                            self.tool = Tool::Pencil;
                        }
                    }
                }
            });
            // Cells of the tileset that have a picture but no tile yet
            let cells = (size.x as usize / TILE) * (size.y as usize / TILE);
            let tileset = open.assets.tileset.as_ref().ok();
            let unused: Vec<u16> = open
                .project
                .unused_cells(cells)
                .into_iter()
                .filter(|&cell| {
                    tileset
                        .and_then(|t| t.cell(cell as usize))
                        .is_some_and(|pixels| pixels.iter().any(|&p| p != TRANSPARENT))
                })
                .collect();
            if ui.button("+ New tile").clicked() {
                let cell = unused.first().copied().unwrap_or(0);
                open.project.tiles.push(TileDef {
                    name: format!("Tile {}", open.project.tiles.len() + 1),
                    cells: vec![cell],
                    frame_ms: 300,
                    solid: false,
                    step_sound: None,
                });
                self.tile = open.project.tiles.len() - 1;
            }
            if !unused.is_empty()
                && ui
                    .button(format!("+ A tile for each unused cell ({})", unused.len()))
                    .clicked()
            {
                for cell in unused {
                    open.project.tiles.push(TileDef {
                        name: format!("Tile {}", open.project.tiles.len() + 1),
                        cells: vec![cell],
                        frame_ms: 300,
                        solid: false,
                        step_sound: None,
                    });
                }
            }
        });
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        let mut import = false;
        let mut remove_map = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            let Some(open) = &mut self.open else { return };
            let project = &mut open.project;

            ui.heading("World");
            ui.horizontal(|ui| {
                ui.label("Title");
                ui.text_edit_singleline(&mut project.title);
            });
            if ui
                .button("Import tileset…")
                .on_hover_text("Replaces tiles.png; the tiles keep their cells")
                .clicked()
            {
                import = true;
            }
            ui.label(format!("Folder: {}", open.dir.display()));

            ui.separator();
            ui.heading("Map");
            let map_index = self.map;
            let map = &mut project.maps[map_index];
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut map.name);
            });
            let (width, height) = (map.width(), map.height());
            let size = match self.size {
                Some(size) if size.0 == map_index => size,
                _ => (map_index, width, height),
            };
            let (mut w, mut h) = (size.1, size.2);
            ui.horizontal(|ui| {
                ui.label("Size");
                ui.add(egui::DragValue::new(&mut w).range(1..=adventure_format::MAX_MAP_SIDE));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut h).range(1..=adventure_format::MAX_MAP_SIDE));
                let changed = (w, h) != (width, height);
                if ui
                    .add_enabled(changed, egui::Button::new("Resize"))
                    .on_hover_text("Keeps the top left; new ground gets the selected tile")
                    .clicked()
                {
                    let removed = map.resize(w, h, self.tile as u16 + 1);
                    if removed > 0 {
                        self.status = format!("Removed {removed} things that were off the map");
                        self.error = false;
                    }
                }
            });
            self.size = Some((map_index, w, h));
            let names: Vec<String> = project.music.iter().map(|m| m.name.clone()).collect();
            let map = &mut project.maps[map_index];
            let music = map
                .music
                .and_then(|m| names.get(m.0 as usize).cloned())
                .unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt("music")
                .selected_text(format!("Music: {music}"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut map.music, None, "None");
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut map.music, Some(MusicRef(i as u16)), name);
                    }
                });
            if ui.button("Delete map").clicked() {
                remove_map = Some(map_index);
            }

            ui.separator();
            ui.heading("Tile");
            let Some(tileset) = open.tileset.clone() else {
                return;
            };
            let tileset_size = tileset.size_vec2();
            let sounds: Vec<String> = project.sounds.iter().map(|s| s.name.clone()).collect();
            let tile_count = project.tiles.len();
            let Some(tile) = project.tiles.get_mut(self.tile) else {
                ui.label("No tile selected.");
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut tile.name);
            });
            ui.checkbox(&mut tile.solid, "Solid: can't be walked onto");
            let sound_name = tile
                .step_sound
                .and_then(|s| sounds.get(s.0 as usize).cloned())
                .unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt("step sound")
                .selected_text(format!("Step sound: {sound_name}"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut tile.step_sound, None, "None");
                    for (i, name) in sounds.iter().enumerate() {
                        ui.selectable_value(&mut tile.step_sound, Some(SoundRef(i as u16)), name);
                    }
                });
            ui.label("Pictures (more than one: animated)");
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (n, &cell) in tile.cells.iter().enumerate() {
                    let image = egui::Image::from_texture(SizedTexture::new(
                        tileset.id(),
                        vec2(32.0, 32.0),
                    ))
                    .uv(cell_uv(tileset_size, cell));
                    ui.add(image);
                    if tile.cells.len() > 1 && ui.small_button("✕").clicked() {
                        remove = Some(n);
                    }
                }
                ui.menu_button("+", |ui| {
                    ui.label("A cell of the tileset:");
                    let columns = tileset_size.x as usize / TILE;
                    let rows = tileset_size.y as usize / TILE;
                    egui::Grid::new("cells")
                        .spacing(vec2(2.0, 2.0))
                        .show(ui, |ui| {
                            for row in 0..rows {
                                for col in 0..columns {
                                    let cell = (row * columns + col) as u16;
                                    let image = egui::Image::from_texture(SizedTexture::new(
                                        tileset.id(),
                                        vec2(32.0, 32.0),
                                    ))
                                    .uv(cell_uv(tileset_size, cell));
                                    if ui.add(egui::Button::image(image)).clicked() {
                                        tile.cells.push(cell);
                                        ui.close();
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
            });
            if let Some(n) = remove {
                tile.cells.remove(n);
            }
            if tile.cells.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label("Each picture for");
                    ui.add(
                        egui::DragValue::new(&mut tile.frame_ms)
                            .range(50..=5000)
                            .suffix(" ms"),
                    );
                });
            }
            if tile_count > 1
                && ui
                    .button("Delete tile")
                    .on_hover_text("Where maps use it, they become empty")
                    .clicked()
            {
                project.remove_tile(self.tile);
                self.tile = self.tile.min(project.tiles.len() - 1);
            }
        });
        if import {
            self.import_tileset();
        }
        if let Some(index) = remove_map {
            let result = self
                .open
                .as_mut()
                .map(|open| open.project.remove_map(index));
            match result {
                Some(Ok(())) => {
                    self.map = self.map.saturating_sub(1).min(index);
                    self.keep_selection_valid();
                    self.say("Map deleted");
                }
                Some(Err(reason)) => self.fail(format!("Can't delete the map: {reason}")),
                None => {}
            }
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        ensure_textures(open, &ctx);
        self.map = self.map.min(open.project.maps.len() - 1);
        let texture = map_texture(open, self.map, self.layer, self.dim, &ctx);
        let map = &open.project.maps[self.map];
        let (width, height) = (map.width(), map.height());
        let scale = TILE as f32 * self.zoom;
        let start = (open.project.start.map == map.id).then_some(open.project.start.at);
        let (tool, rect_from, grid) = (self.tool, self.rect_from, self.grid);

        let mut input = CanvasInput::default();
        egui::ScrollArea::both().show(ui, |ui| {
            let size = vec2(width as f32 * scale, height as f32 * scale);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
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
                let line = Stroke::new(1.0, Color32::from_black_alpha(50));
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
                painter.rect_stroke(
                    r,
                    2.0,
                    Stroke::new(2.0, Color32::YELLOW),
                    StrokeKind::Inside,
                );
                painter.text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    "S",
                    egui::FontId::proportional(scale * 0.5),
                    Color32::YELLOW,
                );
            }
            let to_cell = |pos: egui::Pos2| {
                let p = (pos - rect.min) / scale;
                (p.x >= 0.0 && p.y >= 0.0 && p.x < width as f32 && p.y < height as f32)
                    .then(|| Pos::new(p.x as u16, p.y as u16))
            };
            let hover = response.hover_pos().and_then(to_cell);
            if let Some(at) = hover {
                painter.rect_stroke(
                    cell_rect(at),
                    0.0,
                    Stroke::new(1.0, Color32::WHITE),
                    StrokeKind::Inside,
                );
            }
            if let (Tool::Rect, Some(from), Some(to)) = (tool, rect_from, hover) {
                let r = cell_rect(from).union(cell_rect(to));
                painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
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
            Tool::Rect => {
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

    fn play_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(play) = &mut self.play else { return };
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.play = None;
            self.say("Stopped playing");
            return;
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
                None => {
                    play.texture = Some(ctx.load_texture("play", image, TextureOptions::NEAREST))
                }
            }
        }
        if let Some(texture) = &play.texture {
            let available = ui.available_size();
            let zoom = (available.x / WIDTH as f32)
                .min(available.y / HEIGHT as f32)
                .floor()
                .max(1.0);
            let size = vec2(WIDTH as f32 * zoom, HEIGHT as f32 * zoom);
            ui.centered_and_justified(|ui| {
                ui.add(egui::Image::from_texture(SizedTexture::new(
                    texture.id(),
                    size,
                )));
            });
        }
        ctx.request_repaint();
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }
        self.poll_send(&ctx);
        self.reload_changed_pictures();
        self.shortcuts(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.play.is_some() {
            egui::CentralPanel::default().show(ui, |ui| self.play_view(ui));
        } else if self.open.is_some() {
            egui::Panel::left("tiles")
                .resizable(true)
                .default_size(230.0)
                .show(ui, |ui| self.left_panel(ui));
            egui::Panel::right("properties")
                .resizable(true)
                .default_size(280.0)
                .show(ui, |ui| self.right_panel(ui));
            egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.welcome(ui));
        }
        self.commit(&ctx);
        self.confirm(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("device", self.device.clone());
        if let Some(open) = &self.open {
            storage.set_string("project", open.dir.display().to_string());
        }
    }
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
    open.sprites.clear();
    open.map = None;
    open.generation += 1;
}

fn ensure_textures(open: &mut Open, ctx: &egui::Context) {
    if open.tileset.is_none() {
        if let Ok(tileset) = &open.assets.tileset {
            open.tileset =
                Some(ctx.load_texture("tileset", rgba_image(tileset), TextureOptions::NEAREST));
        }
    }
    if open.sprites.len() != open.assets.sprites.len() {
        open.sprites = open
            .assets
            .sprites
            .iter()
            .enumerate()
            .map(|(i, sprite)| {
                sprite.as_ref().ok().map(|image| {
                    ctx.load_texture(
                        format!("sprite {i}"),
                        rgba_image(image),
                        TextureOptions::NEAREST,
                    )
                })
            })
            .collect();
    }
}

/// The map as one picture: the layers (the others dimmed if asked) and the things on it.
fn map_texture(
    open: &mut Open,
    index: usize,
    layer: LayerKind,
    dim: bool,
    ctx: &egui::Context,
) -> TextureHandle {
    let project = &open.project;
    let map = &project.maps[index];
    let mut hasher = DefaultHasher::new();
    (index, layer as u8, dim, open.generation).hash(&mut hasher);
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
    if let Some((made, texture)) = &open.map {
        if *made == key {
            return texture.clone();
        }
    }

    let (w, h) = (map.width() as usize * TILE, map.height() as usize * TILE);
    let mut out = vec![[24u8, 24, 28, 255]; w * h];
    let pictures: Vec<Option<Vec<u16>>> = project
        .tiles
        .iter()
        .map(|tile| {
            let tileset = open.assets.tileset.as_ref().ok()?;
            tileset.cell(*tile.cells.first()? as usize)
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
    for kind in LayerKind::ALL {
        let alpha = if dim && kind != layer { 0.3 } else { 1.0 };
        for (y, row) in map.layer(kind).0.iter().enumerate() {
            for (x, &n) in row.iter().enumerate() {
                if let Some(Some(pixels)) = n.checked_sub(1).and_then(|t| pictures.get(t as usize))
                {
                    draw(pixels, x * TILE, y * TILE, alpha);
                }
            }
        }
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
                    let alpha = if dim && layer != LayerKind::Decoration {
                        0.3
                    } else {
                        1.0
                    };
                    draw(
                        &pixels,
                        thing.thing.at.x as usize * TILE,
                        thing.thing.at.y as usize * TILE,
                        alpha,
                    );
                }
            }
        }
    }
    let image = ColorImage::from_rgba_unmultiplied([w, h], out.as_flattened());
    let texture = match open.map.take() {
        Some((_, mut texture)) => {
            texture.set(image, TextureOptions::NEAREST);
            texture
        }
        None => ctx.load_texture("map", image, TextureOptions::NEAREST),
    };
    open.map = Some((key, texture.clone()));
    texture
}
