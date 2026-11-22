//! Adventure's `.adv` world file: what a world is, and reading, writing and checking it.
//!
//! A file is [`MAGIC`], then [`VERSION`] as a little-endian `u16`, then the [`World`] encoded with
//! [postcard](https://docs.rs/postcard). Only files of this version are read; a world in an older
//! file has to be written again. The format is described in docs/CONTRACTS.md.
//!
//! Maps, things, items and flags have a stable [`Id`], handed out once and never reused within a
//! world, so they keep meaning the same thing after the world was changed and written again.
//! Everything else is referred to by its position in the world's lists ([`TileRef`], [`SpriteRef`],
//! [`SoundRef`], [`MusicRef`]).

use std::collections::HashSet;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The first bytes of every `.adv` file.
pub const MAGIC: &[u8; 3] = b"ADV";
/// The version of the format, after [`MAGIC`].
pub const VERSION: u16 = 1;

/// A tile, and every picture of a sprite, is `TILE` × `TILE` pixels.
pub const TILE: usize = 16;
/// Pixels in one tile or sprite picture.
pub const TILE_PIXELS: usize = TILE * TILE;
/// The colour of transparent pixels (magenta), in sprites and in tiles on the upper layers.
pub const TRANSPARENT: u16 = 0xF81F;

/// The largest map, in tiles, in either direction.
pub const MAX_MAP_SIDE: u16 = 256;
/// The longest title, in bytes.
pub const MAX_TITLE: usize = 200;
/// The largest thumbnail, in pixels.
pub const MAX_THUMBNAIL: (u16, u16) = (160, 120);
/// The most pictures a tile, or a sprite per direction, can have.
pub const MAX_FRAMES: usize = 4;
/// The most parts a piece of music can have.
pub const MAX_PARTS: usize = 2;

// ---- World -----------------------------------------------------------------------------------

/// A whole world: everything needed to play it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
    /// Chosen at random once per world, and kept when it's changed: tells worlds apart, whatever
    /// their files are called.
    pub uid: u64,
    /// For the grown-ups (web page, logs); never shown to the player.
    pub title: String,
    /// What the player picks the world by.
    pub thumbnail: Picture,
    /// The characters the player can be, with 4 directions each. With one, there's no choice.
    pub heroes: Vec<SpriteRef>,
    pub start: Place,
    pub tiles: Vec<Tile>,
    /// Characters, items and pictures for bubbles.
    pub sprites: Vec<Sprite>,
    pub items: Vec<Item>,
    /// The flags that exist. They have no names in the file.
    pub flags: Vec<Id>,
    /// Sound effects.
    pub sounds: Vec<Melody>,
    pub music: Vec<Music>,
    pub maps: Vec<Map>,
}

/// A stable ID: handed out once, never reused within a world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Id(pub u32);

/// A position in [`World::tiles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileRef(pub u16);
/// A position in [`World::sprites`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpriteRef(pub u16);
/// A position in [`World::sounds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SoundRef(pub u16);
/// A position in [`World::music`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MusicRef(pub u16);

// ---- Pictures --------------------------------------------------------------------------------

/// RGB565 pixels, row by row. Stored as raw little-endian bytes, which is smaller and much faster
/// to read than postcard's variable-length numbers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pixels(pub Vec<u16>);

impl Serialize for Pixels {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = self
            .0
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect();
        serializer.serialize_bytes(&bytes)
    }
}

impl<'de> Deserialize<'de> for Pixels {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(deserializer)?;
        let (pairs, rest) = bytes.as_chunks::<2>();
        if !rest.is_empty() {
            return Err(D::Error::custom("odd number of pixel bytes"));
        }
        Ok(Pixels(
            pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect(),
        ))
    }
}

/// A picture of any size, e.g. the thumbnail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    pub width: u16,
    pub height: u16,
    pub pixels: Pixels,
}

/// One kind of ground, wall or decoration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    /// 1 to [`MAX_FRAMES`] pictures; more than one makes it animated.
    pub frames: Vec<Pixels>,
    /// How long each picture shows, when animated.
    pub frame_ms: u16,
    /// The player and characters can't walk onto it.
    pub solid: bool,
    /// Played when the player steps onto it, e.g. grass rustling.
    pub step_sound: Option<SoundRef>,
}

/// A character, an item or a picture for bubbles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sprite {
    /// 1 (e.g. an apple) or 4, in the order of [`Dir`].
    pub directions: u8,
    /// Pictures per direction, 1 to [`MAX_FRAMES`]: the first standing, the others walking.
    pub frames: u8,
    /// `directions` × `frames` pictures, direction by direction.
    pub pixels: Pixels,
}

impl Sprite {
    /// The pixels of one picture.
    pub fn picture(&self, direction: Dir, frame: usize) -> &[u16] {
        let direction = if self.directions == 4 {
            direction as usize
        } else {
            0
        };
        let index = direction * self.frames as usize + frame.min(self.frames as usize - 1);
        &self.pixels.0[index * TILE_PIXELS..(index + 1) * TILE_PIXELS]
    }
}

// ---- Maps ------------------------------------------------------------------------------------

/// One place to walk around in, e.g. a village or the inside of a house.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Map {
    pub id: Id,
    /// In tiles.
    pub width: u16,
    pub height: u16,
    pub ground: Layer,
    pub decoration: Layer,
    /// Drawn over the player and the characters, e.g. treetops.
    pub above: Layer,
    pub music: Option<MusicRef>,
    pub things: Vec<Thing>,
    /// Tiles that change while a flag is set.
    pub swaps: Vec<Swap>,
}

impl Map {
    pub fn layer(&self, kind: LayerKind) -> &Layer {
        match kind {
            LayerKind::Ground => &self.ground,
            LayerKind::Decoration => &self.decoration,
            LayerKind::Above => &self.above,
        }
    }

    pub fn contains(&self, at: Pos) -> bool {
        at.x < self.width && at.y < self.height
    }
}

/// `width` × `height` tile numbers, row by row: 0 is empty, `n` is `TileRef(n - 1)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer(pub Vec<u16>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerKind {
    Ground,
    Decoration,
    Above,
}

impl LayerKind {
    pub const ALL: [LayerKind; 3] = [LayerKind::Ground, LayerKind::Decoration, LayerKind::Above];
}

/// While the flag `when` is set, the tile at `at` on `layer` is `tile` (None: empty). The last
/// matching swap wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swap {
    pub when: Id,
    pub layer: LayerKind,
    pub at: Pos,
    pub tile: Option<TileRef>,
}

/// A tile position on a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub x: u16,
    pub y: u16,
}

impl Pos {
    pub const fn new(x: u16, y: u16) -> Self {
        Pos { x, y }
    }

    /// The neighbouring position in `dir`, if it isn't off the top or left edge.
    pub fn step(self, dir: Dir) -> Option<Pos> {
        let (dx, dy) = dir.delta();
        Some(Pos {
            x: self.x.checked_add_signed(dx)?,
            y: self.y.checked_add_signed(dy)?,
        })
    }
}

/// A position on a map, and which way the player faces there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub map: Id,
    pub at: Pos,
    pub facing: Dir,
}

/// A direction, in the order of a sprite's pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dir {
    Down,
    Up,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Down, Dir::Up, Dir::Left, Dir::Right];

    pub fn delta(self) -> (i16, i16) {
        match self {
            Dir::Down => (0, 1),
            Dir::Up => (0, -1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

// ---- Things on a map -------------------------------------------------------------------------

/// A character, an item, a door or anything else that does something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thing {
    pub id: Id,
    pub at: Pos,
    /// None: invisible, e.g. the spot in front of a door.
    pub look: Option<SpriteRef>,
    /// Solid things are bumped into (characters); the others are walked onto (items, doors).
    pub solid: bool,
    pub motion: Motion,
    /// The thing is only there while this holds.
    pub shown_if: Condition,
    /// When the player bumps into it, or presses A facing it.
    pub on_bump: Vec<Rule>,
    /// When the player walks onto it.
    pub on_step: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Motion {
    Still,
    /// Walks around at random, at most `radius` tiles from where it was placed.
    Wander {
        radius: u8,
    },
    /// Walks these steps over and over.
    Path(Vec<Dir>),
}

/// The first rule whose condition holds runs its actions, one after the other. A last rule with
/// [`Condition::Always`] is the "otherwise".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub when: Condition,
    pub then: Vec<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    Always,
    /// The player carries at least `count` of `item`.
    Has {
        item: Id,
        count: u16,
    },
    Flag(Id),
    NotFlag(Id),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Sound(SoundRef),
    /// A picture in a bubble above the thing.
    Bubble(SpriteRef),
    /// The item's picture in a bubble, with a dot for each one missing to `count`.
    BubbleMissing {
        item: Id,
        count: u16,
    },
    Animate(Animation),
    Give {
        item: Id,
        count: u16,
    },
    Take {
        item: Id,
        count: u16,
    },
    SetFlag(Id),
    ClearFlag(Id),
    /// The thing disappears for good, e.g. a picked-up apple.
    Hide(Id),
    /// This thing follows the player from now on; one at a time.
    Join,
    /// Takes the player there, e.g. through a door.
    Teleport(Place),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Animation {
    Jump,
    Spin,
    Shake,
}

/// Something the player can carry, shown as icons at the top of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: Id,
    pub icon: SpriteRef,
}

// ---- Sound -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// 0: a pause.
    pub hz: u16,
    pub ms: u16,
}

impl Note {
    pub const fn new(hz: u16, ms: u16) -> Self {
        Note { hz, ms }
    }
}

/// A sound effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Melody(pub Vec<Note>);

/// Background music: 1 to [`MAX_PARTS`] parts of equal length, played together and looped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Music {
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub wave: Wave,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wave {
    Sine,
    Square,
    Triangle,
}

// ---- Reading and writing ---------------------------------------------------------------------

#[derive(Debug)]
pub enum Error {
    /// Doesn't start with [`MAGIC`].
    NotAdv,
    /// Another version of the format.
    Version(u16),
    Decode(postcard::Error),
    /// Decoded, but doesn't make sense, e.g. a reference to a sprite that doesn't exist.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::NotAdv => write!(f, "not an .adv file"),
            Error::Version(found) => {
                write!(f, "format version {found}, this program reads {VERSION}")
            }
            Error::Decode(error) => write!(f, "can't decode: {error}"),
            Error::Invalid(reason) => write!(f, "invalid world: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// What comes after [`MAGIC`] and a matching [`VERSION`].
fn body(bytes: &[u8]) -> Result<&[u8], Error> {
    let rest = bytes.strip_prefix(MAGIC).ok_or(Error::NotAdv)?;
    let (version, rest) = rest.split_first_chunk::<2>().ok_or(Error::NotAdv)?;
    let version = u16::from_le_bytes(*version);
    if version != VERSION {
        return Err(Error::Version(version));
    }
    Ok(rest)
}

/// The first fields of a [`World`], which can be read without the rest, e.g. to show a list of
/// worlds: they're encoded first, in this order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Head {
    pub uid: u64,
    pub title: String,
    pub thumbnail: Picture,
}

impl Head {
    /// The most bytes the head of a file can take, header included: enough to read for
    /// [`Head::from_bytes`]. Variable-length numbers take at most 10 bytes (u64), 5 (lengths) or 3
    /// (u16).
    pub const MAX_BYTES: usize = MAGIC.len()
        + 2
        + 10
        + 5
        + MAX_TITLE
        + 3
        + 3
        + 5
        + 2 * MAX_THUMBNAIL.0 as usize * MAX_THUMBNAIL.1 as usize;

    /// Reads the head of an `.adv` file; `bytes` may end anywhere after it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Head, Error> {
        let (head, _) = postcard::take_from_bytes::<Head>(body(bytes)?).map_err(Error::Decode)?;
        let thumb = &head.thumbnail;
        let (max_w, max_h) = MAX_THUMBNAIL;
        let fits = (1..=max_w).contains(&thumb.width) && (1..=max_h).contains(&thumb.height);
        if !fits || thumb.pixels.0.len() != thumb.width as usize * thumb.height as usize {
            return Err(Error::Invalid("broken thumbnail".into()));
        }
        if head.title.len() > MAX_TITLE {
            return Err(Error::Invalid("title too long".into()));
        }
        Ok(head)
    }
}

impl World {
    /// Reads and checks an `.adv` file.
    pub fn from_bytes(bytes: &[u8]) -> Result<World, Error> {
        let world: World = postcard::from_bytes(body(bytes)?).map_err(Error::Decode)?;
        world.validate()?;
        Ok(world)
    }

    /// The `.adv` file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(VERSION.to_le_bytes());
        // Only fails for types serde can't describe, which World doesn't have
        bytes.extend(postcard::to_allocvec(self).expect("World always encodes"));
        bytes
    }

    pub fn map(&self, id: Id) -> Option<&Map> {
        self.maps.iter().find(|map| map.id == id)
    }

    pub fn item(&self, id: Id) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Checks everything that decoding can't: that every reference points somewhere, that sizes
    /// match, that IDs are unique. A world that passes can be played without panicking.
    pub fn validate(&self) -> Result<(), Error> {
        Check { world: self }.world().map_err(Error::Invalid)
    }
}

struct Check<'a> {
    world: &'a World,
}

type Checked = Result<(), String>;

fn ensure(ok: bool, reason: impl FnOnce() -> String) -> Checked {
    if ok {
        Ok(())
    } else {
        Err(reason())
    }
}

fn unique(ids: impl IntoIterator<Item = Id>, what: &str) -> Checked {
    let mut seen = HashSet::new();
    for id in ids {
        ensure(seen.insert(id), || format!("{what} id {} used twice", id.0))?;
    }
    Ok(())
}

impl Check<'_> {
    fn world(&self) -> Checked {
        let w = self.world;
        ensure(w.title.len() <= MAX_TITLE, || "title too long".into())?;
        let (max_w, max_h) = MAX_THUMBNAIL;
        let thumb = &w.thumbnail;
        ensure(
            (1..=max_w).contains(&thumb.width) && (1..=max_h).contains(&thumb.height),
            || format!("thumbnail is {}x{}", thumb.width, thumb.height),
        )?;
        ensure(
            thumb.pixels.0.len() == thumb.width as usize * thumb.height as usize,
            || "thumbnail has the wrong number of pixels".into(),
        )?;

        for (i, tile) in w.tiles.iter().enumerate() {
            ensure((1..=MAX_FRAMES).contains(&tile.frames.len()), || {
                format!("tile {i} has {} pictures", tile.frames.len())
            })?;
            ensure(
                tile.frames.iter().all(|frame| frame.0.len() == TILE_PIXELS),
                || format!("tile {i} has a picture of the wrong size"),
            )?;
            ensure(tile.frames.len() == 1 || tile.frame_ms > 0, || {
                format!("tile {i} is animated without frame_ms")
            })?;
            if let Some(sound) = tile.step_sound {
                self.sound(sound)?;
            }
        }
        for (i, sprite) in w.sprites.iter().enumerate() {
            ensure(matches!(sprite.directions, 1 | 4), || {
                format!("sprite {i} has {} directions", sprite.directions)
            })?;
            ensure((1..=MAX_FRAMES).contains(&(sprite.frames as usize)), || {
                format!("sprite {i} has {} pictures per direction", sprite.frames)
            })?;
            let pictures = sprite.directions as usize * sprite.frames as usize;
            ensure(sprite.pixels.0.len() == pictures * TILE_PIXELS, || {
                format!("sprite {i} has the wrong number of pixels")
            })?;
        }

        ensure(!w.heroes.is_empty(), || "no heroes".into())?;
        for &hero in &w.heroes {
            self.sprite(hero)?;
            ensure(w.sprites[hero.0 as usize].directions == 4, || {
                format!("hero sprite {} doesn't have 4 directions", hero.0)
            })?;
        }

        unique(w.items.iter().map(|item| item.id), "item")?;
        for item in &w.items {
            self.sprite(item.icon)?;
        }
        unique(w.flags.iter().copied(), "flag")?;

        for (i, music) in w.music.iter().enumerate() {
            ensure((1..=MAX_PARTS).contains(&music.parts.len()), || {
                format!("music {i} has {} parts", music.parts.len())
            })?;
            let length = |part: &Part| part.notes.iter().map(|n| n.ms as u32).sum::<u32>();
            let first = length(&music.parts[0]);
            ensure(first > 0, || format!("music {i} is empty"))?;
            ensure(music.parts.iter().all(|part| length(part) == first), || {
                format!("music {i} has parts of different lengths")
            })?;
        }

        ensure(!w.maps.is_empty(), || "no maps".into())?;
        unique(w.maps.iter().map(|map| map.id), "map")?;
        unique(
            w.maps
                .iter()
                .flat_map(|map| map.things.iter().map(|t| t.id)),
            "thing",
        )?;
        for map in &w.maps {
            self.map(map)?;
        }
        self.place(&w.start)
    }

    fn map(&self, map: &Map) -> Checked {
        let id = map.id.0;
        ensure(
            (1..=MAX_MAP_SIDE).contains(&map.width) && (1..=MAX_MAP_SIDE).contains(&map.height),
            || format!("map {id} is {}x{}", map.width, map.height),
        )?;
        let cells = map.width as usize * map.height as usize;
        for kind in LayerKind::ALL {
            let layer = &map.layer(kind).0;
            ensure(layer.len() == cells, || {
                format!("map {id}: {kind:?} layer has the wrong size")
            })?;
            ensure(
                layer.iter().all(|&n| n as usize <= self.world.tiles.len()),
                || format!("map {id}: {kind:?} layer uses a tile that doesn't exist"),
            )?;
        }
        if let Some(music) = map.music {
            ensure((music.0 as usize) < self.world.music.len(), || {
                format!("map {id} plays music {} that doesn't exist", music.0)
            })?;
        }
        for swap in &map.swaps {
            self.flag(swap.when)?;
            ensure(map.contains(swap.at), || {
                format!("map {id}: a swap is off the map")
            })?;
            if let Some(tile) = swap.tile {
                ensure((tile.0 as usize) < self.world.tiles.len(), || {
                    format!("map {id}: a swap uses tile {} that doesn't exist", tile.0)
                })?;
            }
        }
        for thing in &map.things {
            let tid = thing.id.0;
            ensure(map.contains(thing.at), || {
                format!("thing {tid} is off its map")
            })?;
            if let Some(look) = thing.look {
                self.sprite(look)?;
            }
            self.condition(&thing.shown_if)?;
            if let Motion::Path(steps) = &thing.motion {
                ensure(!steps.is_empty(), || {
                    format!("thing {tid} has an empty path")
                })?;
            }
            for rule in thing.on_bump.iter().chain(&thing.on_step) {
                self.condition(&rule.when)?;
                for action in &rule.then {
                    self.action(action)
                        .map_err(|reason| format!("thing {tid}: {reason}"))?;
                }
            }
        }
        Ok(())
    }

    fn action(&self, action: &Action) -> Checked {
        match action {
            Action::Sound(sound) => self.sound(*sound),
            Action::Bubble(sprite) => self.sprite(*sprite),
            Action::BubbleMissing { item, .. }
            | Action::Give { item, .. }
            | Action::Take { item, .. } => self.item(*item),
            Action::SetFlag(flag) | Action::ClearFlag(flag) => self.flag(*flag),
            Action::Hide(thing) => ensure(
                self.world
                    .maps
                    .iter()
                    .any(|map| map.things.iter().any(|t| t.id == *thing)),
                || format!("hides thing {} that doesn't exist", thing.0),
            ),
            Action::Teleport(place) => self.place(place),
            Action::Animate(_) | Action::Join => Ok(()),
        }
    }

    fn condition(&self, condition: &Condition) -> Checked {
        match condition {
            Condition::Always => Ok(()),
            Condition::Has { item, .. } => self.item(*item),
            Condition::Flag(flag) | Condition::NotFlag(flag) => self.flag(*flag),
        }
    }

    fn place(&self, place: &Place) -> Checked {
        let map = self
            .world
            .map(place.map)
            .ok_or_else(|| format!("map {} doesn't exist", place.map.0))?;
        ensure(map.contains(place.at), || {
            format!(
                "({}, {}) is off map {}",
                place.at.x, place.at.y, place.map.0
            )
        })
    }

    fn sprite(&self, sprite: SpriteRef) -> Checked {
        ensure((sprite.0 as usize) < self.world.sprites.len(), || {
            format!("sprite {} doesn't exist", sprite.0)
        })
    }

    fn sound(&self, sound: SoundRef) -> Checked {
        ensure((sound.0 as usize) < self.world.sounds.len(), || {
            format!("sound {} doesn't exist", sound.0)
        })
    }

    fn item(&self, item: Id) -> Checked {
        ensure(self.world.item(item).is_some(), || {
            format!("item {} doesn't exist", item.0)
        })
    }

    fn flag(&self, flag: Id) -> Checked {
        ensure(self.world.flags.contains(&flag), || {
            format!("flag {} doesn't exist", flag.0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(colour: u16) -> Pixels {
        Pixels(vec![colour; TILE_PIXELS])
    }

    /// One 3x2 map with a wall tile, a hero and an apple.
    fn tiny() -> World {
        let map = Id(1);
        let apple = Id(2);
        World {
            uid: 42,
            title: "Tiny".into(),
            thumbnail: Picture {
                width: 2,
                height: 1,
                pixels: Pixels(vec![1, 2]),
            },
            heroes: vec![SpriteRef(0)],
            start: Place {
                map,
                at: Pos::new(0, 0),
                facing: Dir::Down,
            },
            tiles: vec![
                Tile {
                    frames: vec![picture(0x07E0)],
                    frame_ms: 0,
                    solid: false,
                    step_sound: None,
                },
                Tile {
                    frames: vec![picture(0x8410)],
                    frame_ms: 0,
                    solid: true,
                    step_sound: None,
                },
            ],
            sprites: vec![
                Sprite {
                    directions: 4,
                    frames: 1,
                    pixels: Pixels(vec![0xFFFF; 4 * TILE_PIXELS]),
                },
                Sprite {
                    directions: 1,
                    frames: 1,
                    pixels: picture(0xF800),
                },
            ],
            items: vec![Item {
                id: apple,
                icon: SpriteRef(1),
            }],
            flags: vec![Id(3)],
            sounds: vec![Melody(vec![Note::new(880, 50)])],
            music: vec![Music {
                parts: vec![
                    Part {
                        wave: Wave::Square,
                        notes: vec![Note::new(440, 200)],
                    },
                    Part {
                        wave: Wave::Triangle,
                        notes: vec![Note::new(0, 100), Note::new(220, 100)],
                    },
                ],
            }],
            maps: vec![Map {
                id: map,
                width: 3,
                height: 2,
                ground: Layer(vec![1, 1, 2, 1, 1, 2]),
                decoration: Layer(vec![0; 6]),
                above: Layer(vec![0; 6]),
                music: Some(MusicRef(0)),
                things: vec![Thing {
                    id: Id(4),
                    at: Pos::new(1, 1),
                    look: Some(SpriteRef(1)),
                    solid: false,
                    motion: Motion::Still,
                    shown_if: Condition::Always,
                    on_bump: vec![],
                    on_step: vec![Rule {
                        when: Condition::NotFlag(Id(3)),
                        then: vec![
                            Action::Give {
                                item: apple,
                                count: 1,
                            },
                            Action::Hide(Id(4)),
                            Action::Sound(SoundRef(0)),
                        ],
                    }],
                }],
                swaps: vec![Swap {
                    when: Id(3),
                    layer: LayerKind::Ground,
                    at: Pos::new(2, 0),
                    tile: None,
                }],
            }],
        }
    }

    #[test]
    fn round_trip() {
        let world = tiny();
        world.validate().unwrap();
        let bytes = world.to_bytes();
        assert_eq!(&bytes[..5], b"ADV\x01\x00");
        assert_eq!(World::from_bytes(&bytes).unwrap(), world);
    }

    #[test]
    fn head_without_the_rest() {
        let mut world = tiny();
        world.thumbnail = Picture {
            width: MAX_THUMBNAIL.0,
            height: MAX_THUMBNAIL.1,
            pixels: Pixels(vec![7; MAX_THUMBNAIL.0 as usize * MAX_THUMBNAIL.1 as usize]),
        };
        world.title = "x".repeat(MAX_TITLE);
        world.uid = u64::MAX;
        let bytes = world.to_bytes();
        let head = Head::from_bytes(&bytes[..Head::MAX_BYTES.min(bytes.len())]).unwrap();
        assert_eq!(
            (head.uid, &head.title, &head.thumbnail),
            (world.uid, &world.title, &world.thumbnail)
        );
        assert!(matches!(
            Head::from_bytes(&bytes[..300]),
            Err(Error::Decode(_))
        ));
    }

    #[test]
    fn pixels_are_raw_bytes() {
        let bytes = postcard::to_allocvec(&Pixels(vec![0x1234, 0xF81F])).unwrap();
        assert_eq!(bytes, [4, 0x34, 0x12, 0x1F, 0xF8]);
        assert!(postcard::from_bytes::<Pixels>(&[3, 1, 2, 3]).is_err());
    }

    #[test]
    fn rejects_other_files() {
        assert!(matches!(
            World::from_bytes(b"PK\x03\x04"),
            Err(Error::NotAdv)
        ));
        assert!(matches!(World::from_bytes(b"ADV"), Err(Error::NotAdv)));
        let mut bytes = tiny().to_bytes();
        bytes[3] = 2;
        assert!(matches!(World::from_bytes(&bytes), Err(Error::Version(2))));
        let bytes = tiny().to_bytes();
        assert!(matches!(
            World::from_bytes(&bytes[..bytes.len() - 10]),
            Err(Error::Decode(_))
        ));
    }

    fn invalid(change: impl FnOnce(&mut World)) -> String {
        let mut world = tiny();
        change(&mut world);
        match world.validate() {
            Err(Error::Invalid(reason)) => reason,
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn finds_broken_references() {
        assert!(invalid(|w| w.maps[0].ground.0[0] = 3).contains("tile"));
        assert!(invalid(|w| w.maps[0].decoration.0.pop().map(drop).unwrap()).contains("size"));
        assert!(invalid(|w| w.heroes = vec![SpriteRef(1)]).contains("4 directions"));
        assert!(invalid(|w| w.heroes.clear()).contains("heroes"));
        assert!(invalid(|w| w.start.at = Pos::new(3, 0)).contains("off map"));
        assert!(invalid(|w| w.flags.clear()).contains("flag 3"));
        assert!(invalid(|w| w.items.push(w.items[0])).contains("used twice"));
        assert!(
            invalid(|w| w.music[0].parts[1].notes.pop().map(drop).unwrap())
                .contains("different lengths")
        );
        assert!(invalid(|w| w.maps[0].things[0].on_step[0]
            .then
            .push(Action::Hide(Id(9))))
        .contains("thing 9"));
        assert!(invalid(|w| w.maps[0].things[0].on_step[0]
            .then
            .push(Action::Teleport(Place {
                map: Id(7),
                at: Pos::new(0, 0),
                facing: Dir::Up,
            })))
        .contains("map 7"));
        assert!(invalid(|w| w.sprites[1].pixels.0.pop().map(drop).unwrap()).contains("pixels"));
    }

    #[test]
    fn sprite_pictures() {
        let mut pixels = Vec::new();
        for picture in 0..8u16 {
            pixels.extend([picture; TILE_PIXELS]);
        }
        let sprite = Sprite {
            directions: 4,
            frames: 2,
            pixels: Pixels(pixels),
        };
        assert_eq!(sprite.picture(Dir::Down, 0)[0], 0);
        assert_eq!(sprite.picture(Dir::Up, 1)[0], 3);
        assert_eq!(sprite.picture(Dir::Right, 5)[0], 7);
    }
}
