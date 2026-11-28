//! A small world built by hand, for the tests and the examples: a village with a cat, four apples,
//! a horse at a gate and a dog behind it, and grandma's house, where grandma has the fifth apple.
//! The horse opens the gate for five apples.

use adventure_format::*;

pub const VILLAGE: Id = Id(1);
pub const HOUSE: Id = Id(2);
pub const APPLE: Id = Id(10);
pub const GATE_OPEN: Id = Id(20);
pub const GIFT_GIVEN: Id = Id(21);
pub const DOG: Id = Id(100);

/// `0xRRGGBB` as RGB565.
const fn rgb(hex: u32) -> u16 {
    let (r, g, b) = ((hex >> 16) & 0xFF, (hex >> 8) & 0xFF, hex & 0xFF);
    ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) as u16
}

const GRASS: u16 = rgb(0x6abe30);
const GRASS_DARK: u16 = rgb(0x4f9a26);
const PATH: u16 = rgb(0xe2b878);
const PATH_DARK: u16 = rgb(0xc9975c);
const WATER: u16 = rgb(0x3f7fd8);
const WAVE: u16 = rgb(0x9cc8ff);
const BARK: u16 = rgb(0x7a4a24);
const BARK_DARK: u16 = rgb(0x5c361a);
const LEAF: u16 = rgb(0x2f7a2a);
const LEAF_LIGHT: u16 = rgb(0x45a33a);
const ROOF: u16 = rgb(0xc8443c);
const ROOF_DARK: u16 = rgb(0x99302a);
const WALL: u16 = rgb(0xf2e3c6);
const WALL_DARK: u16 = rgb(0xd4bf98);
const GLASS: u16 = rgb(0x7fc8ff);
const WOOD: u16 = rgb(0x8a5a2b);
const WOOD_DARK: u16 = rgb(0x6a4220);
const KNOB: u16 = rgb(0xffd34d);
const FENCE: u16 = rgb(0xf4f0e6);
const FENCE_DARK: u16 = rgb(0xb9ab8f);
const FLOOR: u16 = rgb(0xc99a62);
const FLOOR_DARK: u16 = rgb(0xa87a46);
const INNER: u16 = rgb(0x8f6b8a);
const INNER_DARK: u16 = rgb(0x6c4c68);
const SHEET: u16 = rgb(0xf2f2f2);
const BLANKET: u16 = rgb(0xd94b6a);
const RUG: u16 = rgb(0x4a7fc0);
const RUG_LIGHT: u16 = rgb(0x7fb0e8);
const MAT: u16 = rgb(0x5a8a3a);
const MAT_LIGHT: u16 = rgb(0x76a64f);
const PINK: u16 = rgb(0xff5a7a);
const YELLOW: u16 = rgb(0xffe066);
const ORANGE: u16 = rgb(0xff8a00);
const SKIN: u16 = rgb(0xffd2a8);
const HAIR: u16 = rgb(0x6b3e1e);
const EYE: u16 = rgb(0x222222);
const MOUTH: u16 = rgb(0xee8888);
const DRESS: u16 = rgb(0xff6fa8);
const LEGS: u16 = rgb(0x3a4a8a);
const CAT: u16 = rgb(0xf0a040);
const CAT_DARK: u16 = rgb(0xc87a24);
const DOG_FUR: u16 = rgb(0xa0663a);
const DOG_DARK: u16 = rgb(0x6b3f20);
const COLLAR: u16 = rgb(0xe53b3b);
const HORSE: u16 = rgb(0x8b5a2b);
const MANE: u16 = rgb(0x3a2412);
const GREY: u16 = rgb(0xd8d8d8);
const PURPLE: u16 = rgb(0x8a5ac8);
const APPLE_RED: u16 = rgb(0xe33b3b);
const APPLE_SHINE: u16 = rgb(0xff9a9a);
const STEM: u16 = rgb(0x5a3a1a);
const HEART: u16 = rgb(0xe5395c);

/// A 16 × 16 picture, painted rectangle by rectangle.
#[derive(Clone)]
struct Pic(Vec<u16>);

impl Pic {
    fn new(colour: u16) -> Pic {
        Pic(vec![colour; TILE_PIXELS])
    }

    fn clear() -> Pic {
        Pic::new(TRANSPARENT)
    }

    fn rect(mut self, x: usize, y: usize, w: usize, h: usize, colour: u16) -> Pic {
        for row in y..(y + h).min(TILE) {
            for col in x..(x + w).min(TILE) {
                self.0[row * TILE + col] = colour;
            }
        }
        self
    }

    /// `rows` of characters, each pixel `scale` × `scale`; characters not in `palette` are left out.
    fn pattern(
        mut self,
        rows: &[&str],
        x: usize,
        y: usize,
        scale: usize,
        palette: &[(char, u16)],
    ) -> Pic {
        for (row, line) in rows.iter().enumerate() {
            for (col, c) in line.chars().enumerate() {
                if let Some(&(_, colour)) = palette.iter().find(|(key, _)| *key == c) {
                    self = self.rect(x + col * scale, y + row * scale, scale, scale, colour);
                }
            }
        }
        self
    }

    fn mirror(mut self) -> Pic {
        for row in self.0.as_chunks_mut::<TILE>().0 {
            row.reverse();
        }
        self
    }

    /// Only rows `from..to`; the others become transparent.
    fn rows(mut self, from: usize, to: usize) -> Pic {
        for (row, line) in self.0.as_chunks_mut::<TILE>().0.iter_mut().enumerate() {
            if !(from..to).contains(&row) {
                line.fill(TRANSPARENT);
            }
        }
        self
    }

    /// Paints `top` over this picture, leaving out its transparent pixels.
    fn under(mut self, top: &Pic) -> Pic {
        for (pixel, &over) in self.0.iter_mut().zip(&top.0) {
            if over != TRANSPARENT {
                *pixel = over;
            }
        }
        self
    }

    fn pixels(self) -> Pixels {
        Pixels(self.0)
    }
}

// ---- Tiles -----------------------------------------------------------------------------------

fn grass() -> Pic {
    Pic::new(GRASS)
        .rect(3, 4, 1, 2, GRASS_DARK)
        .rect(11, 2, 1, 2, GRASS_DARK)
        .rect(7, 11, 1, 2, GRASS_DARK)
        .rect(13, 12, 1, 2, GRASS_DARK)
}

fn path() -> Pic {
    Pic::new(PATH)
        .rect(4, 5, 1, 1, PATH_DARK)
        .rect(12, 10, 2, 1, PATH_DARK)
        .rect(8, 2, 1, 1, PATH_DARK)
        .rect(2, 13, 1, 1, PATH_DARK)
}

fn water(frame: usize) -> Pic {
    let shift = frame * 2;
    Pic::new(WATER)
        .rect(2 + shift, 4, 4, 1, WAVE)
        .rect(9 - shift, 10, 4, 1, WAVE)
}

fn trunk() -> Pic {
    Pic::clear()
        .rect(6, 6, 4, 10, BARK)
        .rect(6, 6, 1, 10, BARK_DARK)
        .rect(4, 14, 8, 2, BARK)
}

/// A treetop on the tile over a trunk. Its upper part is drawn above the player, its lowest rows
/// below, so that the player's feet stay visible behind the tree.
fn crown() -> Pic {
    Pic::clear()
        .rect(2, 3, 12, 13, LEAF)
        .rect(1, 6, 14, 8, LEAF)
        .rect(4, 1, 8, 3, LEAF)
        .rect(4, 4, 3, 2, LEAF_LIGHT)
        .rect(9, 7, 2, 2, LEAF_LIGHT)
        .rect(5, 10, 2, 1, LEAF_LIGHT)
}

/// A whole small tree in one tile, for the edge of the map.
fn tree() -> Pic {
    Pic::clear()
        .rect(6, 10, 4, 6, BARK)
        .rect(6, 10, 1, 6, BARK_DARK)
        .rect(2, 1, 12, 9, LEAF)
        .rect(1, 3, 14, 5, LEAF)
        .rect(4, 0, 8, 2, LEAF)
        .rect(4, 2, 3, 2, LEAF_LIGHT)
        .rect(9, 4, 2, 2, LEAF_LIGHT)
}

fn flowers() -> Pic {
    Pic::clear()
        .rect(5, 8, 1, 3, GRASS_DARK)
        .rect(11, 12, 1, 3, GRASS_DARK)
        .rect(4, 5, 3, 3, PINK)
        .rect(5, 6, 1, 1, YELLOW)
        .rect(10, 9, 3, 3, YELLOW)
        .rect(11, 10, 1, 1, ORANGE)
}

fn roof() -> Pic {
    (0..4).fold(Pic::new(ROOF), |pic, i| {
        pic.rect(0, i * 4 + 3, 16, 1, ROOF_DARK)
    })
}

fn wall() -> Pic {
    Pic::new(WALL)
        .rect(0, 15, 16, 1, WALL_DARK)
        .rect(4, 4, 8, 6, GLASS)
        .rect(7, 4, 2, 6, WALL_DARK)
        .rect(4, 6, 8, 1, WALL_DARK)
}

fn door() -> Pic {
    Pic::new(WALL)
        .rect(4, 3, 8, 13, WOOD)
        .rect(4, 3, 8, 1, WOOD_DARK)
        .rect(10, 9, 1, 1, KNOB)
}

fn fence() -> Pic {
    Pic::clear()
        .rect(1, 4, 3, 10, FENCE)
        .rect(12, 4, 3, 10, FENCE)
        .rect(0, 6, 16, 2, FENCE)
        .rect(0, 10, 16, 2, FENCE)
        .rect(0, 8, 16, 1, FENCE_DARK)
        .rect(0, 12, 16, 1, FENCE_DARK)
}

fn gate_closed() -> Pic {
    (0..4)
        .fold(Pic::clear().rect(1, 3, 14, 11, WOOD), |pic, i| {
            pic.rect(3 + i * 3, 3, 1, 11, WOOD_DARK)
        })
        .rect(1, 8, 14, 1, WOOD_DARK)
}

fn gate_open() -> Pic {
    Pic::clear()
        .rect(0, 4, 2, 10, FENCE)
        .rect(14, 4, 2, 10, FENCE)
}

fn floor() -> Pic {
    (0..4)
        .fold(Pic::new(FLOOR), |pic, i| {
            pic.rect(0, i * 4 + 3, 16, 1, FLOOR_DARK)
        })
        .rect(7, 0, 1, 4, FLOOR_DARK)
        .rect(3, 4, 1, 4, FLOOR_DARK)
        .rect(11, 8, 1, 4, FLOOR_DARK)
        .rect(6, 12, 1, 4, FLOOR_DARK)
}

fn inner_wall() -> Pic {
    Pic::new(INNER)
        .rect(0, 12, 16, 4, INNER_DARK)
        .rect(3, 3, 1, 1, INNER_DARK)
        .rect(11, 6, 1, 1, INNER_DARK)
}

fn bed() -> Pic {
    Pic::clear()
        .rect(1, 1, 14, 14, SHEET)
        .rect(1, 7, 14, 8, BLANKET)
        .rect(1, 7, 14, 1, ROOF_DARK)
        .rect(3, 2, 10, 4, 0xFFFF)
}

fn rug() -> Pic {
    floor().rect(0, 2, 16, 12, RUG).rect(2, 4, 12, 8, RUG_LIGHT)
}

fn mat() -> Pic {
    floor().rect(2, 4, 12, 10, MAT).rect(4, 6, 8, 6, MAT_LIGHT)
}

// ---- Sprites ---------------------------------------------------------------------------------

fn girl(dir: Dir, frame: usize) -> Pic {
    let pic = Pic::clear()
        .rect(4, 1, 8, 3, HAIR)
        .rect(3, 2, 1, 6, HAIR)
        .rect(12, 2, 1, 6, HAIR)
        .rect(4, 4, 8, 5, SKIN);
    let pic = match dir {
        Dir::Up => pic.rect(4, 4, 8, 4, HAIR),
        Dir::Left => pic
            .rect(5, 6, 1, 1, EYE)
            .rect(8, 6, 1, 1, EYE)
            .rect(11, 3, 1, 5, HAIR),
        Dir::Right => pic
            .rect(7, 6, 1, 1, EYE)
            .rect(10, 6, 1, 1, EYE)
            .rect(4, 3, 1, 5, HAIR),
        Dir::Down => pic
            .rect(6, 6, 1, 1, EYE)
            .rect(9, 6, 1, 1, EYE)
            .rect(7, 8, 2, 1, MOUTH),
    };
    let (left, right) = match frame {
        1 => (2, 3),
        2 => (3, 2),
        _ => (3, 3),
    };
    pic.rect(4, 9, 8, 4, DRESS)
        .rect(3, 12, 10, 1, DRESS)
        .rect(3, 9, 1, 3, SKIN)
        .rect(12, 9, 1, 3, SKIN)
        .rect(5, 13, 2, left, LEGS)
        .rect(9, 13, 2, right, LEGS)
}

/// A four-legged animal facing left (or towards the viewer); right is the mirror image.
fn animal(dir: Dir, frame: usize, draw: fn(Dir, usize) -> Pic) -> Pic {
    match dir {
        Dir::Right => draw(Dir::Left, frame).mirror(),
        _ => draw(dir, frame),
    }
}

fn cat(dir: Dir, frame: usize) -> Pic {
    let legs = if frame == 1 { [3, 10] } else { [4, 9] };
    let pic = Pic::clear()
        .rect(3, 8, 9, 6, CAT)
        .rect(legs[0], 14, 2, 2, CAT)
        .rect(legs[1], 14, 2, 2, CAT)
        .rect(4, 3, 7, 6, CAT)
        .rect(4, 1, 2, 2, CAT)
        .rect(9, 1, 2, 2, CAT)
        .rect(6, 9, 3, 1, CAT_DARK)
        .rect(5, 11, 5, 1, CAT_DARK)
        .rect(12, 5, 2, 7, CAT)
        .rect(12, 5, 2, 1, CAT_DARK);
    match dir {
        Dir::Up => pic,
        Dir::Left => pic
            .rect(4, 5, 1, 1, EYE)
            .rect(8, 5, 1, 1, EYE)
            .rect(6, 7, 1, 1, MOUTH),
        _ => pic
            .rect(5, 5, 1, 1, EYE)
            .rect(9, 5, 1, 1, EYE)
            .rect(7, 7, 1, 1, MOUTH),
    }
}

fn dog(dir: Dir, frame: usize) -> Pic {
    let legs = if frame == 1 { [2, 11] } else { [3, 10] };
    let pic = Pic::clear()
        .rect(2, 8, 11, 5, DOG_FUR)
        .rect(legs[0], 13, 2, 3, DOG_FUR)
        .rect(legs[1], 13, 2, 3, DOG_FUR)
        .rect(4, 2, 8, 7, DOG_FUR)
        .rect(3, 3, 2, 5, DOG_DARK)
        .rect(11, 3, 2, 5, DOG_DARK)
        .rect(13, 6, 2, 3, DOG_FUR)
        .rect(5, 9, 6, 1, COLLAR);
    match dir {
        Dir::Up => pic,
        Dir::Left => pic
            .rect(5, 5, 1, 1, EYE)
            .rect(8, 5, 1, 1, EYE)
            .rect(6, 7, 2, 1, EYE),
        _ => pic
            .rect(6, 5, 1, 1, EYE)
            .rect(9, 5, 1, 1, EYE)
            .rect(7, 7, 2, 1, EYE),
    }
}

fn horse(dir: Dir, _frame: usize) -> Pic {
    let right = Pic::clear()
        .rect(1, 6, 12, 6, HORSE)
        .rect(2, 12, 2, 4, HORSE)
        .rect(5, 12, 2, 4, HORSE)
        .rect(10, 12, 2, 4, HORSE)
        .rect(10, 1, 4, 8, HORSE)
        .rect(12, 3, 3, 3, HORSE)
        .rect(9, 1, 1, 7, MANE)
        .rect(0, 6, 1, 5, MANE)
        .rect(13, 3, 1, 1, EYE)
        .rect(11, 0, 1, 2, HORSE);
    match dir {
        Dir::Left | Dir::Down => right.mirror(),
        Dir::Right | Dir::Up => right,
    }
}

fn grandma(dir: Dir, _frame: usize) -> Pic {
    let pic = Pic::clear()
        .rect(5, 0, 6, 2, GREY)
        .rect(4, 2, 8, 2, GREY)
        .rect(4, 4, 8, 5, SKIN)
        .rect(3, 3, 1, 4, GREY)
        .rect(12, 3, 1, 4, GREY)
        .rect(3, 9, 10, 7, PURPLE)
        .rect(5, 10, 6, 4, SHEET);
    match dir {
        Dir::Up => pic.rect(4, 4, 8, 4, GREY),
        Dir::Left => pic.rect(5, 6, 1, 1, EYE).rect(8, 6, 1, 1, EYE),
        Dir::Right => pic.rect(7, 6, 1, 1, EYE).rect(10, 6, 1, 1, EYE),
        Dir::Down => pic
            .rect(6, 6, 1, 1, EYE)
            .rect(9, 6, 1, 1, EYE)
            .rect(7, 8, 2, 1, MOUTH),
    }
}

fn apple() -> Pic {
    Pic::clear()
        .rect(4, 6, 8, 7, APPLE_RED)
        .rect(5, 5, 6, 9, APPLE_RED)
        .rect(6, 7, 2, 2, APPLE_SHINE)
        .rect(8, 3, 1, 3, STEM)
        .rect(9, 3, 3, 2, LEAF_LIGHT)
}

fn heart() -> Pic {
    let rows = [
        ".XX.XX.", "XXXXXXX", "XXXXXXX", ".XXXXX.", "..XXX..", "...X...",
    ];
    Pic::clear().pattern(&rows, 1, 2, 2, &[('X', HEART)])
}

fn sprite(directions: &[Dir], frames: usize, draw: impl Fn(Dir, usize) -> Pic) -> Sprite {
    let mut pixels = Vec::new();
    for &dir in directions {
        for frame in 0..frames {
            pixels.extend(draw(dir, frame).0);
        }
    }
    Sprite {
        directions: directions.len() as u8,
        frames: frames as u8,
        pixels: Pixels(pixels),
    }
}

// ---- The world -------------------------------------------------------------------------------

// Tiles, in World::tiles order; a layer holds the position + 1
const T_GRASS: u16 = 1;
const T_PATH: u16 = 2;
const T_WATER: u16 = 3;
const T_TREE: u16 = 4;
const T_TRUNK: u16 = 5;
const T_CANOPY: u16 = 6;
const T_FLOWERS: u16 = 7;
const T_ROOF: u16 = 8;
const T_WALL: u16 = 9;
const T_DOOR: u16 = 10;
const T_FENCE: u16 = 11;
const T_GATE: u16 = 12;
const T_GATE_OPEN: u16 = 13;
const T_FLOOR: u16 = 14;
const T_INNER: u16 = 15;
const T_BED: u16 = 16;
const T_RUG: u16 = 17;
const T_MAT: u16 = 18;
const T_CROWN_BOTTOM: u16 = 19;
/// Where a treetop is split between above and below the player
const CROWN_SPLIT: usize = 12;

// Sprites, in World::sprites order
const S_GIRL: SpriteRef = SpriteRef(0);
const S_CAT: SpriteRef = SpriteRef(1);
const S_DOG: SpriteRef = SpriteRef(2);
const S_HORSE: SpriteRef = SpriteRef(3);
const S_GRANDMA: SpriteRef = SpriteRef(4);
const S_APPLE: SpriteRef = SpriteRef(5);
const S_HEART: SpriteRef = SpriteRef(6);

// Sounds, in World::sounds order
const PICKUP: SoundRef = SoundRef(0);
const MEOW: SoundRef = SoundRef(1);
const WOOF: SoundRef = SoundRef(2);
const NEIGH: SoundRef = SoundRef(3);
const HMM: SoundRef = SoundRef(4);
const JINGLE: SoundRef = SoundRef(5);
const SONG: SoundRef = SoundRef(6);
const DOOR: SoundRef = SoundRef(7);

const VILLAGE_ROWS: [&str; 14] = [
    "TTTTTTTTTTTTTTTTTTTT",
    "T..f..a..TTT....~~~T",
    "T.RRR..........a~~~T",
    "T.HDH...........~~~T",
    "T..=............~~~T",
    "T..=======..f....~~T",
    "T.a......=.c......~T",
    "T....f...=...Y..Y..T",
    "T......P.=h....a...T",
    "TFFFFFFFFgFFFFFFFFFT",
    "T..f.....=....f....T",
    "T.....f..=.........T",
    "T...f....=..d...f..T",
    "TTTTTTTTTTTTTTTTTTTT",
];

const HOUSE_ROWS: [&str; 7] = [
    "WWWWWWWWWW",
    "WbwwwwwwwW",
    "WwwwwwmwwW",
    "WwwrrrwwwW",
    "WwwrrrwwwW",
    "WwwwwwwwwW",
    "WWWWOWWWWW",
];

fn always(then: Vec<Action>) -> Vec<Rule> {
    vec![Rule {
        when: Condition::Always,
        then,
    }]
}

fn thing(id: Id, x: usize, y: usize, look: Option<SpriteRef>, solid: bool) -> Thing {
    Thing {
        id,
        at: Pos::new(x as u16, y as u16),
        look,
        solid,
        motion: Motion::Still,
        shown_if: Condition::Always,
        on_bump: Vec::new(),
        on_step: Vec::new(),
    }
}

/// Builds a map from rows of characters, one per tile.
struct MapBuilder {
    width: usize,
    ground: Vec<u16>,
    decoration: Vec<u16>,
    above: Vec<u16>,
    things: Vec<Thing>,
    next_id: u32,
}

impl MapBuilder {
    fn new(rows: &[&str], first_id: u32) -> Self {
        let cells = rows.len() * rows[0].len();
        MapBuilder {
            width: rows[0].len(),
            ground: vec![0; cells],
            decoration: vec![0; cells],
            above: vec![0; cells],
            things: Vec::new(),
            next_id: first_id,
        }
    }

    fn id(&mut self) -> Id {
        self.next_id += 1;
        Id(self.next_id)
    }

    fn set(&mut self, x: usize, y: usize, ground: u16, decoration: u16) {
        self.ground[y * self.width + x] = ground;
        self.decoration[y * self.width + x] = decoration;
    }

    fn finish(self, id: Id, music: MusicRef, swaps: Vec<Swap>) -> Map {
        let height = self.ground.len() / self.width;
        Map {
            id,
            width: self.width as u16,
            height: height as u16,
            ground: Layer(self.ground),
            decoration: Layer(self.decoration),
            above: Layer(self.above),
            music: Some(music),
            things: self.things,
            swaps,
        }
    }
}

fn village() -> Map {
    let mut b = MapBuilder::new(&VILLAGE_ROWS, 100);
    for (y, row) in VILLAGE_ROWS.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            match c {
                '=' => b.set(x, y, T_PATH, 0),
                '~' => b.set(x, y, T_WATER, 0),
                'T' => b.set(x, y, T_GRASS, T_TREE),
                'Y' => {
                    b.set(x, y, T_GRASS, T_TRUNK);
                    b.above[(y - 1) * b.width + x] = T_CANOPY;
                    b.decoration[(y - 1) * b.width + x] = T_CROWN_BOTTOM;
                }
                'f' => b.set(x, y, T_GRASS, T_FLOWERS),
                'R' => b.set(x, y, T_ROOF, 0),
                'H' => b.set(x, y, T_WALL, 0),
                'F' => b.set(x, y, T_GRASS, T_FENCE),
                'g' => b.set(x, y, T_PATH, T_GATE),
                'D' => {
                    b.set(x, y, T_DOOR, 0);
                    let mut door = thing(b.id(), x, y, None, false);
                    door.on_step = always(vec![
                        Action::Sound(DOOR),
                        Action::Teleport(Place {
                            map: HOUSE,
                            at: Pos::new(4, 5),
                            facing: Dir::Up,
                        }),
                    ]);
                    b.things.push(door);
                }
                _ => b.set(x, y, T_GRASS, 0),
            }
            match c {
                'a' => {
                    let id = b.id();
                    let mut apple = thing(id, x, y, Some(S_APPLE), false);
                    apple.on_step = always(vec![
                        Action::Sound(PICKUP),
                        Action::Give {
                            item: APPLE,
                            count: 1,
                        },
                        Action::Hide(id),
                    ]);
                    b.things.push(apple);
                }
                'c' => {
                    let mut cat = thing(b.id(), x, y, Some(S_CAT), true);
                    cat.motion = Motion::Wander { radius: 2 };
                    cat.on_bump = always(vec![
                        Action::Sound(MEOW),
                        Action::Bubble(S_HEART),
                        Action::Animate(Animation::Spin),
                    ]);
                    b.things.push(cat);
                }
                'h' => {
                    let mut horse = thing(b.id(), x, y, Some(S_HORSE), true);
                    horse.on_bump = vec![
                        Rule {
                            when: Condition::Flag(GATE_OPEN),
                            then: vec![Action::Sound(NEIGH), Action::Bubble(S_HEART)],
                        },
                        Rule {
                            when: Condition::Has {
                                item: APPLE,
                                count: 5,
                            },
                            then: vec![
                                Action::Take {
                                    item: APPLE,
                                    count: 5,
                                },
                                Action::SetFlag(GATE_OPEN),
                                Action::Sound(JINGLE),
                                Action::Bubble(S_HEART),
                                Action::Animate(Animation::Jump),
                            ],
                        },
                        Rule {
                            when: Condition::Always,
                            then: vec![
                                Action::Sound(HMM),
                                Action::BubbleMissing {
                                    item: APPLE,
                                    count: 5,
                                },
                                Action::Animate(Animation::Shake),
                            ],
                        },
                    ];
                    b.things.push(horse);
                }
                'd' => {
                    let mut dog = thing(DOG, x, y, Some(S_DOG), true);
                    dog.motion = Motion::Wander { radius: 1 };
                    dog.on_bump = always(vec![
                        Action::Sound(WOOF),
                        Action::Join,
                        Action::Bubble(S_HEART),
                    ]);
                    b.things.push(dog);
                }
                _ => {}
            }
        }
    }
    let gate = Swap {
        when: GATE_OPEN,
        layer: LayerKind::Decoration,
        at: Pos::new(9, 9),
        tile: Some(TileRef(T_GATE_OPEN - 1)),
    };
    b.finish(VILLAGE, MusicRef(0), vec![gate])
}

fn house() -> Map {
    let mut b = MapBuilder::new(&HOUSE_ROWS, 200);
    for (y, row) in HOUSE_ROWS.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            match c {
                'W' => b.set(x, y, T_INNER, 0),
                'b' => b.set(x, y, T_FLOOR, T_BED),
                'r' => b.set(x, y, T_RUG, 0),
                'O' => {
                    b.set(x, y, T_MAT, 0);
                    let mut exit = thing(b.id(), x, y, None, false);
                    exit.on_step = always(vec![
                        Action::Sound(DOOR),
                        Action::Teleport(Place {
                            map: VILLAGE,
                            at: Pos::new(3, 4),
                            facing: Dir::Down,
                        }),
                    ]);
                    b.things.push(exit);
                }
                _ => b.set(x, y, T_FLOOR, 0),
            }
            if c == 'm' {
                let mut grandma = thing(b.id(), x, y, Some(S_GRANDMA), true);
                grandma.on_bump = vec![
                    Rule {
                        when: Condition::NotFlag(GIFT_GIVEN),
                        then: vec![
                            Action::Give {
                                item: APPLE,
                                count: 1,
                            },
                            Action::SetFlag(GIFT_GIVEN),
                            Action::Sound(PICKUP),
                            Action::Bubble(S_APPLE),
                            Action::Animate(Animation::Jump),
                        ],
                    },
                    Rule {
                        when: Condition::Always,
                        then: vec![Action::Sound(SONG), Action::Bubble(S_HEART)],
                    },
                ];
                b.things.push(grandma);
            }
        }
    }
    b.finish(HOUSE, MusicRef(1), Vec::new())
}

/// The house with a tree and the girl on the path, 64 × 48.
fn thumbnail() -> Picture {
    let scene = [
        [grass().under(&tree()), grass(), roof(), roof()],
        [grass().under(&flowers()), grass(), wall(), door()],
        [
            path(),
            path().under(&girl(Dir::Down, 0)),
            path(),
            grass().under(&flowers()),
        ],
    ];
    let mut pixels = vec![0; 64 * 48];
    for (ty, row) in scene.iter().enumerate() {
        for (tx, pic) in row.iter().enumerate() {
            for y in 0..TILE {
                for x in 0..TILE {
                    pixels[(ty * TILE + y) * 64 + tx * TILE + x] = pic.0[y * TILE + x];
                }
            }
        }
    }
    Picture {
        width: 64,
        height: 48,
        pixels: Pixels(pixels),
    }
}

fn notes(notes: &[(u16, u16)]) -> Vec<Note> {
    notes.iter().map(|&(hz, ms)| Note::new(hz, ms)).collect()
}

/// The cat's meow, as a recording, so that the sample world has one: 0.45 s at 8 kHz, a tone
/// sliding up and back down, with a few overtones, swelling in and fading out.
fn meow() -> Samples {
    const RATE: u16 = 8000;
    let length = RATE as usize * 45 / 100;
    let mut phase = 0.0f32;
    let samples = (0..length)
        .map(|i| {
            let t = i as f32 / length as f32;
            // Up from 500 Hz to 800 Hz at a third, then down to 450 Hz
            let hz = if t < 0.33 {
                500.0 + 300.0 * t / 0.33
            } else {
                800.0 - 350.0 * (t - 0.33) / 0.67
            };
            phase = (phase + hz / f32::from(RATE)).fract();
            let turn = std::f32::consts::TAU * phase;
            let tone = turn.sin() + 0.5 * (2.0 * turn).sin() + 0.25 * (3.0 * turn).sin();
            let envelope = (t / 0.1).min(1.0) * ((1.0 - t) / 0.3).min(1.0);
            (tone / 1.75 * envelope * 110.0) as i8
        })
        .collect();
    Samples {
        rate: RATE,
        samples,
    }
}

/// The sample world.
pub fn world() -> World {
    let tile = |pic: Pic, solid: bool| Tile {
        frames: vec![pic.pixels()],
        frame_ms: 0,
        solid,
        step_sound: None,
    };
    let tiles = vec![
        tile(grass(), false),
        tile(path(), false),
        Tile {
            frames: (0..3).map(|frame| water(frame).pixels()).collect(),
            frame_ms: 400,
            solid: true,
            step_sound: None,
        },
        tile(tree(), true),
        tile(trunk(), true),
        tile(crown().rows(0, CROWN_SPLIT), false),
        tile(flowers(), false),
        tile(roof(), true),
        tile(wall(), true),
        tile(door(), false),
        tile(fence(), true),
        tile(gate_closed(), true),
        tile(gate_open(), false),
        tile(floor(), false),
        tile(inner_wall(), true),
        tile(bed(), true),
        tile(rug(), false),
        tile(mat(), false),
        tile(crown().rows(CROWN_SPLIT, TILE), false),
    ];
    let four = &Dir::ALL;
    let sprites = vec![
        sprite(four, 3, girl),
        sprite(four, 2, |dir, frame| animal(dir, frame, cat)),
        sprite(four, 2, |dir, frame| animal(dir, frame, dog)),
        sprite(four, 1, horse),
        sprite(four, 1, grandma),
        sprite(&[Dir::Down], 1, |_, _| apple()),
        sprite(&[Dir::Down], 1, |_, _| heart()),
    ];
    let sounds = [
        &[(880, 70), (1320, 120)][..],
        // MEOW: a recording instead, see meow()
        &[],
        &[(330, 80), (0, 60), (330, 80)],
        &[(620, 100), (560, 100), (500, 100), (440, 150)],
        &[(220, 120), (196, 200)],
        &[(523, 120), (659, 120), (784, 120), (1047, 240)],
        &[
            (659, 200),
            (587, 200),
            (523, 200),
            (587, 200),
            (659, 200),
            (659, 200),
            (659, 400),
        ],
        &[(420, 80), (300, 80), (200, 120)],
    ]
    .iter()
    .enumerate()
    .map(|(i, melody)| match SoundRef(i as u16) {
        MEOW => Sound::Samples(meow()),
        _ => Sound::Notes(notes(melody)),
    })
    .collect();
    let music = vec![
        Music {
            parts: vec![
                Part {
                    wave: Wave::Square,
                    notes: notes(&[
                        (523, 300),
                        (659, 300),
                        (784, 300),
                        (659, 300),
                        (698, 300),
                        (880, 300),
                        (784, 300),
                        (0, 300),
                    ]),
                },
                Part {
                    wave: Wave::Triangle,
                    notes: notes(&[(131, 600), (175, 600), (131, 600), (196, 600)]),
                },
            ],
        },
        Music {
            parts: vec![Part {
                wave: Wave::Sine,
                notes: notes(&[
                    (659, 400),
                    (784, 400),
                    (880, 400),
                    (784, 400),
                    (659, 400),
                    (587, 400),
                    (523, 800),
                ]),
            }],
        },
    ];
    World {
        uid: 0x5A4D_504C_4520_0001,
        title: "Sample village".into(),
        thumbnail: thumbnail(),
        heroes: vec![S_GIRL],
        start: Place {
            map: VILLAGE,
            at: Pos::new(7, 8),
            facing: Dir::Down,
        },
        tiles,
        sprites,
        items: vec![Item {
            id: APPLE,
            icon: S_APPLE,
        }],
        flags: vec![GATE_OPEN, GIFT_GIVEN],
        sounds,
        music,
        maps: vec![village(), house()],
    }
}
