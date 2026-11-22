//! An editor project: a folder with `world.json`, the tileset `tiles.png` and one PNG per sprite in
//! `sprites/`. The JSON names everything for the editor, refers to pictures by file and to tiles by
//! their cell in the tileset, and uses adventure-format's own types for the rest.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;

use adventure_format::{
    Id, LayerKind, Melody, MusicRef, Note, Part, Place, Pos, SoundRef, SpriteRef, Swap, Thing,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const FILE: &str = "world.json";
pub const TILESET: &str = "tiles.png";
pub const SPRITES: &str = "sprites";
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
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicDef {
    pub name: String,
    pub parts: Vec<Part>,
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

    /// Fills the area of same tiles around `at` (left, right, up, down) with `tile`.
    pub fn flood(&mut self, at: Pos, tile: u16) {
        let Some(old) = self.get(at) else { return };
        if old == tile {
            return;
        }
        let mut todo = VecDeque::from([at]);
        while let Some(pos) = todo.pop_front() {
            if self.get(pos) != Some(old) {
                continue;
            }
            self.set(pos, tile);
            for dir in adventure_format::Dir::ALL {
                if let Some(next) = pos.step(dir) {
                    todo.push_back(next);
                }
            }
        }
    }

    /// Fills the rectangle between two corners.
    pub fn rect(&mut self, a: Pos, b: Pos, tile: u16) {
        for y in a.y.min(b.y)..=a.y.max(b.y) {
            for x in a.x.min(b.x)..=a.x.max(b.x) {
                self.set(Pos::new(x, y), tile);
            }
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
        let project: Project =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if project.version != VERSION {
            return Err(format!(
                "{} is version {}, this editor reads {VERSION}",
                path.display(),
                project.version
            ));
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

    /// The tileset cells no tile uses.
    pub fn unused_cells(&self, cells: usize) -> Vec<u16> {
        let used: HashSet<u16> = self.tiles.iter().flat_map(|t| t.cells.clone()).collect();
        (0..cells as u16).filter(|c| !used.contains(c)).collect()
    }
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

/// Sounds as the format has them.
pub fn melodies(sounds: &[SoundDef]) -> Vec<Melody> {
    sounds.iter().map(|s| Melody(s.notes.clone())).collect()
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
    fn flood_fills_the_same_tiles_around() {
        let mut g = grid(&["1121", "1221", "1111"]);
        g.flood(Pos::new(0, 0), 3);
        assert_eq!(g, grid(&["3323", "3223", "3333"]));
        g.flood(Pos::new(2, 0), 3);
        assert_eq!(g, grid(&["3333", "3333", "3333"]));
    }

    #[test]
    fn rect_and_resize() {
        let mut g = grid(&["000", "000"]);
        g.rect(Pos::new(2, 1), Pos::new(1, 0), 5);
        assert_eq!(g, grid(&["055", "055"]));
        g.resize(4, 3, 1);
        assert_eq!(g, grid(&["0551", "0551", "1111"]));
        g.resize(2, 1, 1);
        assert_eq!(g, grid(&["05"]));
    }
}
