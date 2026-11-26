//! An editor project: a folder with `world.json`, the tileset `tiles.png` and one PNG per sprite in
//! `sprites/`. The JSON names everything for the editor, refers to pictures by file and to tiles by
//! their cell in the tileset, and uses adventure-format's own types for the rest.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;

use adventure_format::{
    Id, LayerKind, Music, MusicRef, Note, Part, Place, Pos, SoundRef, SpriteRef, Swap, Thing, Wave,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const FILE: &str = "world.json";
pub const TILESET: &str = "tiles.png";
pub const SPRITES: &str = "sprites";
pub const SOUNDS: &str = "sounds";
pub const MUSIC: &str = "music";
/// The version of `world.json`
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    /// The world's uid, made once when the project is created
    pub uid: u64,
    pub title: String,
    pub tiles: Vec<TileDef>,
    pub sprites: Vec<SpriteDef>,
    pub heroes: Vec<SpriteRef>,
    pub start: Place,
    pub items: Vec<ItemDef>,
    pub flags: Vec<FlagDef>,
    pub sounds: Vec<SoundDef>,
    pub music: Vec<MusicDef>,
    pub maps: Vec<MapDef>,
    /// The next stable ID to hand out
    pub next_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileDef {
    pub name: String,
    /// Cells of the tileset, numbered row by row: one, or several for an animation
    pub cells: Vec<u16>,
    pub frame_ms: u16,
    pub solid: bool,
    pub step_sound: Option<SoundRef>,
}

/// A sprite's pictures come from `sprites/<file>`: one row per direction (1, or 4 in the order
/// down, up, left, right), one column per picture (1 to 4), each 16 × 16.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteDef {
    pub name: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDef {
    pub id: Id,
    pub name: String,
    pub icon: SpriteRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlagDef {
    pub id: Id,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundDef {
    pub name: String,
    #[serde(flatten)]
    pub source: SoundSource,
}

/// Where a sound comes from: notes, or a WAV file in `sounds/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SoundSource {
    Notes { notes: Vec<Note> },
    File { file: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicDef {
    pub name: String,
    /// A MIDI file in `music/`
    pub file: String,
    /// The file's part played as the melody, numbered as [`crate::midi::read`] finds them
    pub melody: usize,
    /// And the one played as the bass, if any
    pub bass: Option<usize>,
    pub melody_wave: Wave,
    pub bass_wave: Wave,
}

/// Music as projects stored it before it came from MIDI files: notes for each part.
#[derive(Deserialize)]
struct OldMusic {
    name: String,
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapDef {
    pub id: Id,
    pub name: String,
    pub ground: Grid,
    pub decoration: Grid,
    pub above: Grid,
    pub music: Option<MusicRef>,
    pub things: Vec<ThingDef>,
    pub swaps: Vec<Swap>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThingDef {
    pub name: String,
    #[serde(flatten)]
    pub thing: Thing,
}

/// Tile numbers row by row, as in a world's layers: 0 is empty, `n` is tile `n - 1`. In the JSON
/// one string per row, numbers separated by spaces, so a map stays readable and diffable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Grid(pub Vec<Vec<u16>>);

impl Serialize for Grid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let rows: Vec<String> = self
            .0
            .iter()
            .map(|row| row.iter().map(u16::to_string).collect::<Vec<_>>().join(" "))
            .collect();
        rows.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Grid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let rows = Vec::<String>::deserialize(deserializer)?;
        let grid = rows
            .iter()
            .map(|row| row.split_whitespace().map(str::parse).collect())
            .collect::<Result<Vec<Vec<u16>>, _>>()
            .map_err(serde::de::Error::custom)?;
        Ok(Grid(grid))
    }
}

/// What painting puts down: one tile, or a block of them picked from the tileset, repeated
/// when it covers more. 0 in a block is a hole that keeps what's there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brush {
    pub width: u16,
    pub height: u16,
    pub tiles: Vec<u16>,
}

impl Brush {
    pub fn one(tile: u16) -> Brush {
        Brush {
            width: 1,
            height: 1,
            tiles: vec![tile],
        }
    }

    /// The tile for `pos` when the brush is laid down from `origin`; None for a hole.
    pub fn tile(&self, origin: Pos, pos: Pos) -> Option<u16> {
        let x = (pos.x as i32 - origin.x as i32).rem_euclid(self.width as i32) as usize;
        let y = (pos.y as i32 - origin.y as i32).rem_euclid(self.height as i32) as usize;
        let tile = self.tiles[y * self.width as usize + x];
        (tile > 0 || self.tiles.len() == 1).then_some(tile)
    }

    /// Where a block goes for `pos` in a stroke that started at `origin`: in steps of its size,
    /// so dragging lays blocks side by side.
    pub fn snap(&self, origin: Pos, pos: Pos) -> Pos {
        let step = |o: u16, p: u16, size: u16| {
            let d = (p as i32 - o as i32).div_euclid(size as i32) * size as i32;
            (o as i32 + d).max(0) as u16
        };
        Pos::new(
            step(origin.x, pos.x, self.width),
            step(origin.y, pos.y, self.height),
        )
    }
}

impl Grid {
    pub fn filled(width: usize, height: usize, tile: u16) -> Grid {
        Grid(vec![vec![tile; width]; height])
    }

    pub fn width(&self) -> usize {
        self.0.first().map_or(0, Vec::len)
    }

    pub fn height(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, at: Pos) -> Option<u16> {
        self.0.get(at.y as usize)?.get(at.x as usize).copied()
    }

    pub fn set(&mut self, at: Pos, tile: u16) {
        if let Some(cell) = self
            .0
            .get_mut(at.y as usize)
            .and_then(|row| row.get_mut(at.x as usize))
        {
            *cell = tile;
        }
    }

    /// Fills the area of same tiles around `at` (left, right, up, down) with `brush`, repeated
    /// from `at`.
    pub fn flood(&mut self, at: Pos, brush: &Brush) {
        let Some(old) = self.get(at) else { return };
        if brush.tiles == [old] {
            return;
        }
        let mut area = HashSet::new();
        let mut todo = VecDeque::from([at]);
        while let Some(pos) = todo.pop_front() {
            if self.get(pos) != Some(old) || !area.insert(pos) {
                continue;
            }
            for dir in adventure_format::Dir::ALL {
                if let Some(next) = pos.step(dir) {
                    todo.push_back(next);
                }
            }
        }
        for pos in area {
            self.paint(pos, brush.tile(at, pos));
        }
    }

    /// Fills the rectangle between two corners with `brush`, repeated from its top left.
    pub fn rect(&mut self, a: Pos, b: Pos, brush: &Brush) {
        let from = Pos::new(a.x.min(b.x), a.y.min(b.y));
        for y in from.y..=a.y.max(b.y) {
            for x in from.x..=a.x.max(b.x) {
                let pos = Pos::new(x, y);
                self.paint(pos, brush.tile(from, pos));
            }
        }
    }

    /// The whole brush with its top left at `at`.
    pub fn stamp(&mut self, at: Pos, brush: &Brush) {
        for y in 0..brush.height {
            for x in 0..brush.width {
                if let (Some(px), Some(py)) = (at.x.checked_add(x), at.y.checked_add(y)) {
                    let pos = Pos::new(px, py);
                    self.paint(pos, brush.tile(at, pos));
                }
            }
        }
    }

    /// Sets a cell unless `tile` is a brush's hole.
    fn paint(&mut self, at: Pos, tile: Option<u16>) {
        if let Some(tile) = tile {
            self.set(at, tile);
        }
    }

    /// Cuts or extends to `width` × `height`, keeping the top left; new cells get `tile`.
    pub fn resize(&mut self, width: usize, height: usize, tile: u16) {
        self.0.resize(height, vec![tile; width]);
        for row in &mut self.0 {
            row.resize(width, tile);
        }
    }

    pub fn flatten(&self) -> Vec<u16> {
        self.0.concat()
    }
}

impl MapDef {
    pub fn width(&self) -> u16 {
        self.ground.width() as u16
    }

    pub fn height(&self) -> u16 {
        self.ground.height() as u16
    }

    pub fn layer(&self, kind: LayerKind) -> &Grid {
        match kind {
            LayerKind::Ground => &self.ground,
            LayerKind::Decoration => &self.decoration,
            LayerKind::Above => &self.above,
        }
    }

    pub fn layer_mut(&mut self, kind: LayerKind) -> &mut Grid {
        match kind {
            LayerKind::Ground => &mut self.ground,
            LayerKind::Decoration => &mut self.decoration,
            LayerKind::Above => &mut self.above,
        }
    }

    /// Resizes all layers; new ground gets `tile`. Things and swaps that end up off the map are
    /// removed; returns how many things were.
    pub fn resize(&mut self, width: u16, height: u16, tile: u16) -> usize {
        let (w, h) = (width as usize, height as usize);
        self.ground.resize(w, h, tile);
        self.decoration.resize(w, h, 0);
        self.above.resize(w, h, 0);
        let before = self.things.len();
        self.things
            .retain(|thing| thing.thing.at.x < width && thing.thing.at.y < height);
        self.swaps
            .retain(|swap| swap.at.x < width && swap.at.y < height);
        before - self.things.len()
    }
}

impl Project {
    pub fn load(dir: &Path) -> Result<Project, String> {
        let path = dir.join(FILE);
        let text =
            fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let error = |e: serde_json::Error| format!("{}: {e}", path.display());
        let mut value: serde_json::Value = serde_json::from_str(&text).map_err(error)?;
        // Music stored as notes becomes MIDI files, once
        let mut old = Vec::new();
        if let Some(music) = value.get_mut("music").and_then(|m| m.as_array_mut()) {
            for (i, entry) in music.iter_mut().enumerate() {
                if entry.get("parts").is_some() {
                    let song: OldMusic = serde_json::from_value(entry.clone()).map_err(error)?;
                    *entry = serde_json::to_value(placeholder(&song.name)).expect("encodes");
                    old.push((i, song.parts));
                }
            }
        }
        let mut project: Project = serde_json::from_value(value).map_err(error)?;
        if project.version != VERSION {
            return Err(format!(
                "{} is version {}, this editor reads {VERSION}",
                path.display(),
                project.version
            ));
        }
        if !old.is_empty() {
            for (i, parts) in old {
                let name = project.music[i].name.clone();
                project.music[i] = write_song(dir, &name, &Music { parts })?;
            }
            project.save(dir)?;
        }
        Ok(project)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).expect("projects always encode");
        let path = dir.join(FILE);
        fs::write(&path, text + "\n").map_err(|e| format!("can't write {}: {e}", path.display()))
    }

    pub fn new_id(&mut self) -> Id {
        self.next_id += 1;
        Id(self.next_id)
    }

    pub fn map_index(&self, id: Id) -> Option<usize> {
        self.maps.iter().position(|map| map.id == id)
    }

    /// A new map of `width` × `height`, its ground filled with `tile`.
    pub fn add_map(&mut self, width: u16, height: u16, tile: u16) -> usize {
        let id = self.new_id();
        let (w, h) = (width as usize, height as usize);
        self.maps.push(MapDef {
            id,
            name: format!("Map {}", self.maps.len() + 1),
            ground: Grid::filled(w, h, tile),
            decoration: Grid::filled(w, h, 0),
            above: Grid::filled(w, h, 0),
            music: None,
            things: Vec::new(),
            swaps: Vec::new(),
        });
        self.maps.len() - 1
    }

    /// Removes map `index`, unless the world starts there or a door leads there.
    pub fn remove_map(&mut self, index: usize) -> Result<(), String> {
        let id = self.maps[index].id;
        if self.start.map == id {
            return Err("the world starts on this map".into());
        }
        let led_to = self.maps.iter().any(|map| {
            map.things.iter().any(|thing| {
                let rules = thing.thing.on_bump.iter().chain(&thing.thing.on_step);
                rules
                    .flat_map(|rule| &rule.then)
                    .any(|action| matches!(action, adventure_format::Action::Teleport(place) if place.map == id))
            })
        });
        if led_to {
            return Err("a door leads to this map".into());
        }
        self.maps.remove(index);
        Ok(())
    }

    /// Removes tile `index`: where maps used it they become empty, and the tiles after it move up.
    pub fn remove_tile(&mut self, index: usize) {
        self.tiles.remove(index);
        let number = index as u16 + 1;
        let renumber = |n: u16| match n {
            n if n == number => 0,
            n if n > number => n - 1,
            n => n,
        };
        for map in &mut self.maps {
            for kind in LayerKind::ALL {
                for row in &mut map.layer_mut(kind).0 {
                    for cell in row {
                        *cell = renumber(*cell);
                    }
                }
            }
            map.swaps
                .retain(|swap| swap.tile.map(|t| t.0 as usize) != Some(index));
            for swap in &mut map.swaps {
                if let Some(tile) = &mut swap.tile {
                    if tile.0 as usize > index {
                        tile.0 -= 1;
                    }
                }
            }
        }
    }

    /// How often sprite `index` is used: as the player, an item's icon, an object's picture or a
    /// picture in a bubble.
    pub fn sprite_uses(&self, index: usize) -> usize {
        let sprite = SpriteRef(index as u16);
        let heroes = self.heroes.iter().filter(|&&h| h == sprite).count();
        let icons = self.items.iter().filter(|i| i.icon == sprite).count();
        let objects: usize = self
            .maps
            .iter()
            .flat_map(|m| &m.things)
            .map(|t| {
                let look = (t.thing.look == Some(sprite)) as usize;
                let bubbles = t
                    .thing
                    .on_bump
                    .iter()
                    .chain(&t.thing.on_step)
                    .flat_map(|r| &r.then)
                    .filter(|a| **a == adventure_format::Action::Bubble(sprite))
                    .count();
                look + bubbles
            })
            .sum();
        heroes + icons + objects
    }

    /// Removes sprite `index`, which nothing uses any more; the sprites after it move up. Its file
    /// stays in the folder.
    pub fn remove_sprite(&mut self, index: usize) {
        self.sprites.remove(index);
        let shift = |s: &mut SpriteRef| {
            if s.0 as usize > index {
                s.0 -= 1;
            }
        };
        self.heroes.iter_mut().for_each(shift);
        self.items.iter_mut().for_each(|i| shift(&mut i.icon));
        for thing in self.maps.iter_mut().flat_map(|m| &mut m.things) {
            if let Some(look) = &mut thing.thing.look {
                shift(look);
            }
            let rules = thing
                .thing
                .on_bump
                .iter_mut()
                .chain(&mut thing.thing.on_step);
            for action in rules.flat_map(|r| &mut r.then) {
                if let adventure_format::Action::Bubble(sprite) = action {
                    shift(sprite);
                }
            }
        }
    }

    /// How often sound `index` is used: as tiles' step sounds and in objects' rules.
    pub fn sound_uses(&self, index: usize) -> usize {
        let sound = SoundRef(index as u16);
        let steps = self
            .tiles
            .iter()
            .filter(|t| t.step_sound == Some(sound))
            .count();
        let rules = self
            .maps
            .iter()
            .flat_map(|m| &m.things)
            .flat_map(|t| t.thing.on_bump.iter().chain(&t.thing.on_step))
            .flat_map(|r| &r.then)
            .filter(|a| **a == adventure_format::Action::Sound(sound))
            .count();
        steps + rules
    }

    /// Removes sound `index`, which nothing uses any more; the sounds after it move up. Its file
    /// stays in the folder.
    pub fn remove_sound(&mut self, index: usize) {
        self.sounds.remove(index);
        let shift = |s: &mut SoundRef| {
            if s.0 as usize > index {
                s.0 -= 1;
            }
        };
        for tile in &mut self.tiles {
            if let Some(sound) = &mut tile.step_sound {
                shift(sound);
            }
        }
        for thing in self.maps.iter_mut().flat_map(|m| &mut m.things) {
            let rules = thing
                .thing
                .on_bump
                .iter_mut()
                .chain(&mut thing.thing.on_step);
            for action in rules.flat_map(|r| &mut r.then) {
                if let adventure_format::Action::Sound(sound) = action {
                    shift(sound);
                }
            }
        }
    }

    /// Removes music `index`, which no map plays any more; the music after it moves up.
    pub fn remove_music(&mut self, index: usize) {
        self.music.remove(index);
        for map in &mut self.maps {
            if let Some(music) = &mut map.music {
                if music.0 as usize > index {
                    music.0 -= 1;
                }
            }
        }
    }

    /// The maps that play music `index`.
    pub fn music_uses(&self, index: usize) -> usize {
        let music = Some(MusicRef(index as u16));
        self.maps.iter().filter(|m| m.music == music).count()
    }

    /// The tileset cells no tile uses.
    pub fn unused_cells(&self, cells: usize) -> Vec<u16> {
        let used: HashSet<u16> = self.tiles.iter().flat_map(|t| t.cells.clone()).collect();
        (0..cells as u16).filter(|c| !used.contains(c)).collect()
    }
}

/// A song entry for music that's yet to be written as a file.
fn placeholder(name: &str) -> MusicDef {
    MusicDef {
        name: name.to_string(),
        file: String::new(),
        melody: 0,
        bass: None,
        melody_wave: Wave::Square,
        bass_wave: Wave::Triangle,
    }
}

/// The song entry for `music`, written into `music/` as a MIDI file named after `name`.
pub fn write_song(dir: &Path, name: &str, music: &Music) -> Result<MusicDef, String> {
    let folder = dir.join(MUSIC);
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let file = file_name(&folder, name, "mid");
    let path = folder.join(&file);
    fs::write(&path, crate::midi::write(music))
        .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    Ok(song_entry(name, file, music))
}

/// The song entry that plays `music` from `file`, a MIDI file [`crate::midi::write`] made of it.
pub fn song_entry(name: &str, file: String, music: &Music) -> MusicDef {
    MusicDef {
        file,
        bass: (music.parts.len() > 1).then_some(1),
        melody_wave: music.parts.first().map_or(Wave::Square, |p| p.wave),
        bass_wave: music.parts.get(1).map_or(Wave::Triangle, |p| p.wave),
        ..placeholder(name)
    }
}

/// A plain file name made from `stem` that isn't taken in `folder` yet: lowercase letters,
/// digits, `-` and `_`.
pub fn file_name(folder: &Path, stem: &str, extension: &str) -> String {
    let base: String = stem
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = if base.is_empty() { "file".into() } else { base };
    let mut name = format!("{base}.{extension}");
    let mut n = 2;
    while folder.join(&name).exists() {
        name = format!("{base}-{n}.{extension}");
        n += 1;
    }
    name
}

/// A uid nobody else will have.
pub fn new_uid() -> u64 {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_nanos());
    hasher.write_u128(nanos);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Grid {
        Grid(
            rows.iter()
                .map(|row| row.bytes().map(|b| (b - b'0') as u16).collect())
                .collect(),
        )
    }

    #[test]
    fn grid_json_has_a_string_per_row() {
        let g = grid(&["120", "003"]);
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(json, r#"["1 2 0","0 0 3"]"#);
        assert_eq!(serde_json::from_str::<Grid>(&json).unwrap(), g);
        assert!(serde_json::from_str::<Grid>(r#"["1 x"]"#).is_err());
    }

    #[test]
    fn music_as_notes_becomes_midi_files_on_open() {
        let dir = std::env::temp_dir().join(format!("adventure-old-music-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (mut project, _) = crate::export::starter();
        project.music.clear();
        let mut json = serde_json::to_value(&project).unwrap();
        json["music"] = serde_json::json!([{
            "name": "Old tune",
            "parts": [
                {"wave": "Sine", "notes": [{"hz": 440, "ms": 500}, {"hz": 0, "ms": 500}]},
                {"wave": "Triangle", "notes": [{"hz": 110, "ms": 1000}]}
            ]
        }]);
        fs::write(dir.join(FILE), json.to_string()).unwrap();
        let project = Project::load(&dir).unwrap();
        let song = &project.music[0];
        assert_eq!(song.file, "old-tune.mid");
        assert_eq!((song.melody, song.bass), (0, Some(1)));
        assert_eq!(
            (song.melody_wave, song.bass_wave),
            (Wave::Sine, Wave::Triangle)
        );
        assert!(dir.join(MUSIC).join("old-tune.mid").exists());
        // Saved, so the next open finds the file
        assert_eq!(Project::load(&dir).unwrap(), project);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sounds_are_notes_or_files_in_json() {
        let notes: SoundDef =
            serde_json::from_str(r#"{"name":"Meow","notes":[{"hz":880,"ms":50}]}"#).unwrap();
        assert_eq!(
            notes.source,
            SoundSource::Notes {
                notes: vec![Note::new(880, 50)]
            }
        );
        let file: SoundDef = serde_json::from_str(r#"{"name":"Moo","file":"cow.wav"}"#).unwrap();
        assert_eq!(
            file.source,
            SoundSource::File {
                file: "cow.wav".into()
            }
        );
        assert_eq!(
            serde_json::to_string(&file).unwrap(),
            r#"{"name":"Moo","file":"cow.wav"}"#
        );
    }

    #[test]
    fn flood_fills_the_same_tiles_around() {
        let mut g = grid(&["1121", "1221", "1111"]);
        g.flood(Pos::new(0, 0), &Brush::one(3));
        assert_eq!(g, grid(&["3323", "3223", "3333"]));
        g.flood(Pos::new(2, 0), &Brush::one(3));
        assert_eq!(g, grid(&["3333", "3333", "3333"]));
    }

    #[test]
    fn blocks_repeat_and_keep_their_holes() {
        // A 2 × 2 block whose bottom right is a hole
        let block = Brush {
            width: 2,
            height: 2,
            tiles: vec![1, 2, 3, 0],
        };
        let mut g = grid(&["9999", "9999", "9999"]);
        g.stamp(Pos::new(1, 1), &block);
        assert_eq!(g, grid(&["9999", "9129", "9399"]));
        let mut g = grid(&["9999", "9999", "9999"]);
        g.rect(Pos::new(3, 2), Pos::new(0, 0), &block);
        assert_eq!(g, grid(&["1212", "3939", "1212"]));
        let mut g = grid(&["5555", "5555", "7777"]);
        g.flood(Pos::new(1, 0), &block);
        assert_eq!(g, grid(&["2121", "5353", "7777"]));
        // Dragging lays blocks side by side from where the stroke began
        assert_eq!(block.snap(Pos::new(3, 3), Pos::new(6, 2)), Pos::new(5, 1));
        assert_eq!(block.snap(Pos::new(1, 1), Pos::new(0, 0)), Pos::new(0, 0));
    }

    #[test]
    fn rect_and_resize() {
        let mut g = grid(&["000", "000"]);
        g.rect(Pos::new(2, 1), Pos::new(1, 0), &Brush::one(5));
        assert_eq!(g, grid(&["055", "055"]));
        g.resize(4, 3, 1);
        assert_eq!(g, grid(&["0551", "0551", "1111"]));
        g.resize(2, 1, 1);
        assert_eq!(g, grid(&["05"]));
    }
}
