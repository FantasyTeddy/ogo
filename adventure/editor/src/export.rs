//! Turning a project into a world, and the starter project made from the engine's sample world.

use std::fs;
use std::path::Path;

use adventure_engine::{sample, Game, HEIGHT, WIDTH};
use adventure_format::{
    Dir, Item, Layer, Map, Music, Part, Picture, Pixels, Samples, Sound, Sprite, Tile, World, TILE,
};

use crate::audio;
use crate::images::Rgba;
use crate::midi;
use crate::project::{
    self, FlagDef, Grid, ItemDef, MapDef, MusicDef, Project, SoundDef, SoundSource, SpriteDef,
    ThingDef, TileDef,
};

/// The pictures and recorded sounds of a project, as far as they could be read.
pub struct Assets {
    pub tileset: Result<Rgba, String>,
    /// One per sprite of the project
    pub sprites: Vec<Result<Rgba, String>>,
    /// One per sound of the project; None for notes
    pub sounds: Vec<Option<Result<Samples, String>>>,
    /// One per piece of music of the project
    pub songs: Vec<Result<SongFile, String>>,
}

/// A MIDI file, and what it holds.
pub struct SongFile {
    pub bytes: Vec<u8>,
    pub song: midi::Song,
}

impl SongFile {
    pub fn load(path: &Path) -> Result<SongFile, String> {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let song = midi::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(SongFile { bytes, song })
    }

    /// Of `music`, as [`midi::write`] makes it.
    pub fn of(music: &Music) -> SongFile {
        let bytes = midi::write(music);
        let song = midi::read(&bytes).expect("written music reads back");
        SongFile { bytes, song }
    }
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
            sounds: project
                .sounds
                .iter()
                .map(|sound| match &sound.source {
                    SoundSource::Notes { .. } => None,
                    SoundSource::File { file } => {
                        Some(audio::load(&dir.join(project::SOUNDS).join(file)))
                    }
                })
                .collect(),
            songs: project
                .music
                .iter()
                .map(|music| SongFile::load(&dir.join(project::MUSIC).join(&music.file)))
                .collect(),
        }
    }
}

/// The world a project makes, checked; its thumbnail is the view at the start.
pub fn world(project: &Project, assets: &Assets) -> Result<World, String> {
    if project.heroes.is_empty() {
        return Err("choose who the player is: World settings › Pictures".into());
    }
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
        sounds: project
            .sounds
            .iter()
            .zip(&assets.sounds)
            .map(|(def, samples)| sound(def, samples))
            .collect::<Result<Vec<_>, String>>()?,
        music: project
            .music
            .iter()
            .zip(&assets.songs)
            .map(|(def, song)| music(def, song))
            .collect::<Result<Vec<_>, String>>()?,
        maps,
    };
    world.validate().map_err(|e| e.to_string())?;
    world.thumbnail = start_view(&world);
    Ok(world)
}

/// A sound as the format has it; Err says what's wrong with its file.
pub fn sound(def: &SoundDef, samples: &Option<Result<Samples, String>>) -> Result<Sound, String> {
    match (&def.source, samples) {
        (SoundSource::Notes { notes }, _) => Ok(Sound::Notes(notes.clone())),
        (SoundSource::File { .. }, Some(Ok(samples))) => Ok(Sound::Samples(samples.clone())),
        (SoundSource::File { .. }, Some(Err(error))) => {
            Err(format!("sound \"{}\": {error}", def.name))
        }
        (SoundSource::File { file }, None) => {
            Err(format!("sound \"{}\": {file} isn't loaded", def.name))
        }
    }
}

/// A piece of music as the format has it: the parts chosen from its MIDI file, a note at a time.
pub fn music(def: &MusicDef, song: &Result<SongFile, String>) -> Result<Music, String> {
    let error = |e: &str| format!("music \"{}\": {e}", def.name);
    let song = &song.as_ref().map_err(|e| error(e))?.song;
    let part = |i: usize| {
        song.groups
            .get(i)
            .ok_or_else(|| error(&format!("{} has no part {} any more", def.file, i + 1)))
    };
    let mut parts = vec![Part {
        wave: def.melody_wave,
        notes: midi::line(part(def.melody)?, true, song.length_ms),
    }];
    if let Some(bass) = def.bass {
        parts.push(Part {
            wave: def.bass_wave,
            notes: midi::line(part(bass)?, false, song.length_ms),
        });
    }
    Ok(Music { parts })
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

/// A sprite from its PNG; why not, if the picture has the wrong shape.
pub fn sprite(def: &SpriteDef, image: &Result<Rgba, String>) -> Result<Sprite, String> {
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
            .map(|(i, sound)| SoundDef {
                name: name(&SOUND_NAMES, i, "Sound"),
                source: match sound {
                    Sound::Notes(notes) => SoundSource::Notes {
                        notes: notes.clone(),
                    },
                    // The sample world has only notes
                    Sound::Samples(_) => SoundSource::Notes { notes: Vec::new() },
                },
            })
            .collect(),
        music: world
            .music
            .iter()
            .enumerate()
            .map(|(i, music)| {
                let name = name(&MAP_NAMES, i, "Music");
                let file = format!("{}.mid", name.to_lowercase().replace([' ', '\''], "-"));
                project::song_entry(&name, file, music)
            })
            .collect(),
        sprites,
        maps,
        next_id,
    };
    let assets = Assets {
        tileset: Ok(tileset),
        sprites: sprite_images,
        sounds: vec![None; project.sounds.len()],
        songs: world.music.iter().map(|m| Ok(SongFile::of(m))).collect(),
    };
    (project, assets)
}

/// Creates the folder `dir` with the starter project in it, titled after the folder. An existing
/// folder is only used if it's empty.
pub fn write_starter(dir: &Path) -> Result<Project, String> {
    let (project, assets) = starter();
    write_new(dir, project, assets)
}

/// Creates the folder `dir` with an empty world in it: one empty map, no tiles, no pictures.
pub fn write_empty(dir: &Path) -> Result<Project, String> {
    let (project, assets) = empty();
    write_new(dir, project, assets)
}

/// A world with nothing in it yet but one empty map.
pub fn empty() -> (Project, Assets) {
    let mut project = Project {
        version: project::VERSION,
        uid: project::new_uid(),
        title: "My world".into(),
        tiles: Vec::new(),
        sprites: Vec::new(),
        heroes: Vec::new(),
        start: adventure_format::Place {
            map: adventure_format::Id(0),
            at: adventure_format::Pos::new(10, 7),
            facing: Dir::Down,
        },
        items: Vec::new(),
        flags: Vec::new(),
        sounds: Vec::new(),
        music: Vec::new(),
        maps: Vec::new(),
        next_id: 0,
    };
    let map = project.add_map(20, 15, 0);
    project.maps[map].name = "Map 1".into();
    project.start.map = project.maps[map].id;
    let assets = Assets {
        // One empty cell, so there's a tileset to add tiles to
        tileset: Ok(Rgba::new(TILE, TILE)),
        sprites: Vec::new(),
        sounds: Vec::new(),
        songs: Vec::new(),
    };
    (project, assets)
}

fn write_new(dir: &Path, mut project: Project, assets: Assets) -> Result<Project, String> {
    let in_use = fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
    if in_use {
        return Err(format!(
            "{} already exists and isn't empty: choose a new name",
            dir.display()
        ));
    }
    if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
        project.title = name.to_string();
    }
    let sprites = dir.join(project::SPRITES);
    fs::create_dir_all(&sprites).map_err(|e| format!("{}: {e}", sprites.display()))?;
    let sounds = dir.join(project::SOUNDS);
    fs::create_dir_all(&sounds).map_err(|e| format!("{}: {e}", sounds.display()))?;
    let music = dir.join(project::MUSIC);
    fs::create_dir_all(&music).map_err(|e| format!("{}: {e}", music.display()))?;
    for (def, song) in project.music.iter().zip(&assets.songs) {
        let path = music.join(&def.file);
        let bytes = &song.as_ref().expect("made above").bytes;
        fs::write(&path, bytes).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    }
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
    fn an_empty_world_needs_a_player() {
        let (mut project, mut assets) = empty();
        assert_eq!(project.maps.len(), 1);
        let error = world(&project, &assets).unwrap_err();
        assert!(error.contains("player"), "{error}");
        let mut farmer = Rgba::new(TILE, TILE);
        farmer.paste(&[0x07E0; TILE * TILE], TILE, 0, 0);
        project.sprites.push(SpriteDef {
            name: "Farmer".into(),
            file: "farmer.png".into(),
        });
        assets.sprites.push(Ok(farmer));
        project.heroes = vec![adventure_format::SpriteRef(0)];
        let made = world(&project, &assets).unwrap();
        assert_eq!(
            made.sprites[0].directions, 1,
            "a front-facing player is fine"
        );
    }

    #[test]
    fn music_comes_from_the_chosen_parts() {
        let (mut project, mut assets) = starter();
        // The sample's songs became MIDI files and play as before
        let made = world(&project, &assets).unwrap();
        assert_eq!(made.music, sample::world().music);
        // A song whose bass is chosen from a part that isn't there any more
        project.music[0].bass = Some(5);
        assert!(world(&project, &assets).unwrap_err().contains("no part 6"));
        project.music[0].bass = None;
        assert_eq!(world(&project, &assets).unwrap().music[0].parts.len(), 1);
        assets.songs[1] = Err("grandma.mid: not a MIDI file".into());
        assert!(world(&project, &assets)
            .unwrap_err()
            .contains("not a MIDI file"));
    }

    #[test]
    fn recorded_sounds_go_into_the_world() {
        let (mut project, mut assets) = starter();
        project.sounds.push(SoundDef {
            name: "Moo".into(),
            source: SoundSource::File {
                file: "cow.wav".into(),
            },
        });
        let samples = Samples {
            rate: audio::RATE,
            samples: vec![1, 2, 3],
        };
        assets.sounds.push(Some(Ok(samples.clone())));
        let made = world(&project, &assets).unwrap();
        assert_eq!(made.sounds.last(), Some(&Sound::Samples(samples)));
        // A file that can't be read names the sound
        *assets.sounds.last_mut().unwrap() = Some(Err("cow.wav: it's silent".into()));
        assert!(world(&project, &assets)
            .unwrap_err()
            .contains("\"Moo\": cow.wav"));
    }

    #[test]
    fn removing_a_sprite_renumbers_the_rest() {
        use adventure_format::SpriteRef;
        let (mut project, mut assets) = starter();
        let cat = 1;
        assert_eq!(project.sprite_uses(cat), 1, "the cat's object");
        project.maps[0]
            .things
            .retain(|t| t.thing.look != Some(SpriteRef(cat as u16)));
        assert_eq!(project.sprite_uses(cat), 0);
        project.remove_sprite(cat);
        let _ = assets.sprites.remove(cat);
        assert_eq!(project.items[0].icon, SpriteRef(4), "the apple moved up");
        let horse = project.maps[0]
            .things
            .iter()
            .find(|t| t.name == "Horse")
            .unwrap();
        assert_eq!(horse.thing.look, Some(SpriteRef(2)));
        world(&project, &assets).unwrap();
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
