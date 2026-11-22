//! Turning a project into a world, and the starter project made from the engine's sample world.

use std::fs;
use std::path::Path;

use adventure_engine::{sample, Game, HEIGHT, WIDTH};
use adventure_format::{Dir, Item, Layer, Map, Music, Picture, Pixels, Sprite, Tile, World, TILE};

use crate::images::Rgba;
use crate::project::{
    self, FlagDef, Grid, ItemDef, MapDef, MusicDef, Project, SoundDef, SpriteDef, ThingDef, TileDef,
};

/// The pictures of a project, as far as they could be read.
pub struct Assets {
    pub tileset: Result<Rgba, String>,
    /// One per sprite of the project
    pub sprites: Vec<Result<Rgba, String>>,
}

impl Assets {
    pub fn load(dir: &Path, project: &Project) -> Assets {
        Assets {
            tileset: Rgba::load(&dir.join(project::TILESET)),
            sprites: project
                .sprites
                .iter()
                .map(|sprite| Rgba::load(&dir.join(project::SPRITES).join(&sprite.file)))
                .collect(),
        }
    }
}

/// The world a project makes, checked; its thumbnail is the view at the start.
pub fn world(project: &Project, assets: &Assets) -> Result<World, String> {
    let tileset = assets.tileset.as_ref().map_err(Clone::clone)?;
    let tiles = project
        .tiles
        .iter()
        .map(|def| {
            let frames = def
                .cells
                .iter()
                .map(|&cell| {
                    tileset.cell(cell as usize).map(Pixels).ok_or_else(|| {
                        format!("tile \"{}\": the tileset has no cell {cell}", def.name)
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Tile {
                frames,
                frame_ms: def.frame_ms,
                solid: def.solid,
                step_sound: def.step_sound,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sprites = project
        .sprites
        .iter()
        .zip(&assets.sprites)
        .map(|(def, image)| sprite(def, image))
        .collect::<Result<Vec<_>, String>>()?;
    let maps = project
        .maps
        .iter()
        .map(|map| Map {
            id: map.id,
            width: map.width(),
            height: map.height(),
            ground: Layer(map.ground.flatten()),
            decoration: Layer(map.decoration.flatten()),
            above: Layer(map.above.flatten()),
            music: map.music,
            things: map.things.iter().map(|t| t.thing.clone()).collect(),
            swaps: map.swaps.clone(),
        })
        .collect();
    let mut world = World {
        uid: project.uid,
        title: project.title.clone(),
        thumbnail: Picture {
            width: 1,
            height: 1,
            pixels: Pixels(vec![0]),
        },
        heroes: project.heroes.clone(),
        start: project.start,
        tiles,
        sprites,
        items: project
            .items
            .iter()
            .map(|item| Item {
                id: item.id,
                icon: item.icon,
            })
            .collect(),
        flags: project.flags.iter().map(|flag| flag.id).collect(),
        sounds: project::melodies(&project.sounds),
        music: project
            .music
            .iter()
            .map(|music| Music {
                parts: music.parts.clone(),
            })
            .collect(),
        maps,
    };
    world.validate().map_err(|e| e.to_string())?;
    world.thumbnail = start_view(&world);
    Ok(world)
}

/// What the player sees at the start.
fn start_view(world: &World) -> Picture {
    let mut pixels = vec![0; WIDTH * HEIGHT];
    if let Ok(game) = Game::new(world.clone(), 0) {
        game.draw(&mut pixels);
    }
    Picture {
        width: WIDTH as u16,
        height: HEIGHT as u16,
        pixels: Pixels(pixels),
    }
}

fn sprite(def: &SpriteDef, image: &Result<Rgba, String>) -> Result<Sprite, String> {
    let image = image.as_ref().map_err(Clone::clone)?;
    let (frames, directions) = image.cells();
    let shape_ok = image.width % TILE == 0
        && image.height % TILE == 0
        && matches!(directions, 1 | 4)
        && (1..=4).contains(&frames);
    if !shape_ok {
        return Err(format!(
            "sprite \"{}\" is {}x{}: it needs 1 or 4 rows (down, up, left, right) of 1 to 4 pictures of 16x16",
            def.name, image.width, image.height
        ));
    }
    let mut pixels = Vec::new();
    for cell in 0..frames * directions {
        pixels.extend(image.cell(cell).expect("within the picture"));
    }
    Ok(Sprite {
        directions: directions as u8,
        frames: frames as u8,
        pixels: Pixels(pixels),
    })
}

// ---- The starter project ---------------------------------------------------------------------

/// Names for the sample world's tiles and sprites, in their order.
const TILE_NAMES: [&str; 19] = [
    "Grass",
    "Path",
    "Water",
    "Tree",
    "Trunk",
    "Treetop",
    "Flowers",
    "Roof",
    "Wall",
    "Door",
    "Fence",
    "Gate",
    "Open gate",
    "Floor",
    "Inner wall",
    "Bed",
    "Rug",
    "Mat",
    "Treetop (low)",
];
const SPRITE_NAMES: [&str; 7] = ["Girl", "Cat", "Dog", "Horse", "Grandma", "Apple", "Heart"];
const SOUND_NAMES: [&str; 8] = [
    "Pickup", "Meow", "Woof", "Neigh", "Hmm", "Jingle", "Song", "Door",
];
const FLAG_NAMES: [&str; 2] = ["Gate open", "Gift given"];
const MAP_NAMES: [&str; 2] = ["Village", "Grandma's house"];
const TILESET_COLUMNS: usize = 8;

fn name(names: &[&str], index: usize, fallback: &str) -> String {
    names.get(index).map_or_else(
        || format!("{fallback} {}", index + 1),
        |name| name.to_string(),
    )
}

/// A new project with the sample world's tiles, characters, maps and things to start from.
pub fn starter() -> (Project, Assets) {
    let world = sample::world();

    let total: usize = world.tiles.iter().map(|tile| tile.frames.len()).sum();
    let mut tileset = Rgba::new(
        TILESET_COLUMNS * TILE,
        total.div_ceil(TILESET_COLUMNS) * TILE,
    );
    let mut cell = 0u16;
    let tiles = world
        .tiles
        .iter()
        .enumerate()
        .map(|(i, tile)| {
            let cells = tile
                .frames
                .iter()
                .map(|frame| {
                    let n = cell as usize;
                    let (x, y) = (n % TILESET_COLUMNS * TILE, n / TILESET_COLUMNS * TILE);
                    tileset.paste(&frame.0, TILE, x, y);
                    cell += 1;
                    cell - 1
                })
                .collect();
            TileDef {
                name: name(&TILE_NAMES, i, "Tile"),
                cells,
                frame_ms: tile.frame_ms,
                solid: tile.solid,
                step_sound: tile.step_sound,
            }
        })
        .collect();

    let mut sprite_images = Vec::new();
    let sprites = world
        .sprites
        .iter()
        .enumerate()
        .map(|(i, sprite)| {
            let (frames, directions) = (sprite.frames as usize, sprite.directions as usize);
            let mut image = Rgba::new(frames * TILE, directions * TILE);
            for (d, &dir) in Dir::ALL.iter().take(directions).enumerate() {
                for f in 0..frames {
                    image.paste(sprite.picture(dir, f), TILE, f * TILE, d * TILE);
                }
            }
            sprite_images.push(Ok(image));
            let name = name(&SPRITE_NAMES, i, "Sprite");
            let file = format!("{}.png", name.to_lowercase().replace(' ', "-"));
            SpriteDef { name, file }
        })
        .collect::<Vec<_>>();

    let sprite_name = |look: Option<adventure_format::SpriteRef>| match look {
        Some(look) => sprites[look.0 as usize].name.clone(),
        None => "Door".to_string(),
    };
    let maps = world
        .maps
        .iter()
        .enumerate()
        .map(|(i, map)| {
            let grid = |layer: &Layer| {
                Grid(
                    layer
                        .0
                        .chunks(map.width as usize)
                        .map(<[u16]>::to_vec)
                        .collect(),
                )
            };
            MapDef {
                id: map.id,
                name: name(&MAP_NAMES, i, "Map"),
                ground: grid(&map.ground),
                decoration: grid(&map.decoration),
                above: grid(&map.above),
                music: map.music,
                things: map
                    .things
                    .iter()
                    .map(|thing| ThingDef {
                        name: sprite_name(thing.look),
                        thing: thing.clone(),
                    })
                    .collect(),
                swaps: map.swaps.clone(),
            }
        })
        .collect::<Vec<_>>();

    let ids = maps
        .iter()
        .flat_map(|map| std::iter::once(map.id).chain(map.things.iter().map(|t| t.thing.id)))
        .chain(world.items.iter().map(|item| item.id))
        .chain(world.flags.iter().copied());
    let next_id = ids.map(|id| id.0).max().unwrap_or(0);

    let project = Project {
        version: project::VERSION,
        uid: project::new_uid(),
        title: "My world".into(),
        tiles,
        heroes: world.heroes.clone(),
        start: world.start,
        items: world
            .items
            .iter()
            .map(|item| ItemDef {
                id: item.id,
                name: sprites[item.icon.0 as usize].name.clone(),
                icon: item.icon,
            })
            .collect(),
        flags: world
            .flags
            .iter()
            .enumerate()
            .map(|(i, &id)| FlagDef {
                id,
                name: name(&FLAG_NAMES, i, "Flag"),
            })
            .collect(),
        sounds: world
            .sounds
            .iter()
            .enumerate()
            .map(|(i, melody)| SoundDef {
                name: name(&SOUND_NAMES, i, "Sound"),
                notes: melody.0.clone(),
            })
            .collect(),
        music: world
            .music
            .iter()
            .enumerate()
            .map(|(i, music)| MusicDef {
                name: name(&MAP_NAMES, i, "Music"),
                parts: music.parts.clone(),
            })
            .collect(),
        sprites,
        maps,
        next_id,
    };
    let assets = Assets {
        tileset: Ok(tileset),
        sprites: sprite_images,
    };
    (project, assets)
}

/// Creates the folder `dir` with the starter project in it, titled after the folder. An existing
/// folder is only used if it's empty.
pub fn write_starter(dir: &Path) -> Result<Project, String> {
    let in_use = fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
    if in_use {
        return Err(format!(
            "{} already exists and isn't empty: choose a new name",
            dir.display()
        ));
    }
    let (mut project, assets) = starter();
    if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
        project.title = name.to_string();
    }
    let sprites = dir.join(project::SPRITES);
    fs::create_dir_all(&sprites).map_err(|e| format!("{}: {e}", sprites.display()))?;
    assets
        .tileset
        .as_ref()
        .expect("made above")
        .save(&dir.join(project::TILESET))?;
    for (def, image) in project.sprites.iter().zip(&assets.sprites) {
        image
            .as_ref()
            .expect("made above")
            .save(&sprites.join(&def.file))?;
    }
    project.save(dir)?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starter_makes_the_sample_world() {
        let (project, assets) = starter();
        let mut made = world(&project, &assets).unwrap();
        let sample = sample::world();
        assert_eq!(made.thumbnail.width, WIDTH as u16);
        // A new world of its own, with its own title and the start as its picture
        made.uid = sample.uid;
        made.title = sample.title.clone();
        made.thumbnail = sample.thumbnail.clone();
        assert_eq!(made, sample);
    }

    #[test]
    fn starter_files_round_trip() {
        let dir = std::env::temp_dir().join(format!("adventure-starter-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let written = write_starter(&dir).unwrap();
        assert!(
            written.title.starts_with("adventure-starter-"),
            "titled after the folder"
        );
        assert!(write_starter(&dir).is_err(), "not into a folder in use");
        let loaded = Project::load(&dir).unwrap();
        assert_eq!(loaded, written);
        let assets = Assets::load(&dir, &loaded);
        let from_files = world(&loaded, &assets).unwrap();
        let (project, assets) = starter();
        let mut in_memory = world(&project, &assets).unwrap();
        in_memory.uid = from_files.uid;
        assert_eq!(from_files.tiles, in_memory.tiles);
        assert_eq!(from_files.sprites, in_memory.sprites);
        assert_eq!(from_files.maps, in_memory.maps);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explains_a_sprite_of_the_wrong_shape() {
        let (project, mut assets) = starter();
        assets.sprites[0] = Ok(Rgba::new(48, 32));
        let error = world(&project, &assets).unwrap_err();
        assert!(error.contains("\"Girl\" is 48x32"), "{error}");
    }

    #[test]
    fn removing_a_tile_renumbers_the_maps() {
        let (mut project, assets) = starter();
        let grass = project.maps[0]
            .ground
            .get(adventure_format::Pos::new(1, 1))
            .unwrap();
        assert_eq!(grass, 1);
        project.remove_tile(1); // Path
        assert_eq!(
            project.maps[0].ground.get(adventure_format::Pos::new(1, 1)),
            Some(1)
        );
        assert_eq!(
            project.maps[0].ground.get(adventure_format::Pos::new(3, 4)),
            Some(0),
            "path is gone"
        );
        assert_eq!(
            project.maps[0]
                .ground
                .get(adventure_format::Pos::new(17, 1)),
            Some(2),
            "water moved up"
        );
        // The gate's swap pointed at "Open gate", which moved up too
        assert_eq!(
            project.maps[0].swaps[0].tile,
            Some(adventure_format::TileRef(11))
        );
        world(&project, &assets).unwrap();
    }
}
