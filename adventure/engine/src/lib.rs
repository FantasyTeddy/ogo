//! Plays an Adventure world: walking, bumping, the things' rules, and drawing it all into a
//! 160 × 120 picture. Knows nothing about the device or the PC: it gets the buttons once per frame
//! ([`Game::step`]) and hands back the picture ([`Game::draw`]) and the sounds to play ([`Event`]).
//! `cargo run --example walk` plays a world on the PC.

use std::collections::{HashMap, HashSet};

use adventure_format::*;

pub mod recordings;
pub mod sample;
mod save;

pub use recordings::Recordings;
pub use save::Save;

/// The picture [`Game::draw`] paints, in pixels. The device shows it doubled.
pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 120;
/// [`Game::step`] is meant to be called this often per second.
pub const FPS: u32 = 30;

const T: i32 = TILE as i32;
/// Pixels per frame: walking, running (B) and the things' pace
const WALK: u8 = 2;
const RUN: u8 = 4;
const THING_SPEED: u8 = 1;
/// Frames of the fade through black when going through a door
const FADE: u8 = 20;
const BUBBLE_FRAMES: u16 = 70;
/// Frames between two bumps while walking into a wall
const BUMP_PAUSE: u8 = 14;
const ANIMATION_FRAMES: u8 = 12;
/// Item icons at the top of the screen
const MAX_ICONS: usize = 16;

const WHITE: u16 = 0xFFFF;
const OUTLINE: u16 = 0x2124;
const DOT: u16 = 0xE1C7;

/// The buttons held down in this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buttons {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    /// Look at or talk to what's in front.
    pub a: bool,
    /// Run.
    pub b: bool,
}

impl Buttons {
    fn held(&self, dir: Dir) -> bool {
        match dir {
            Dir::Up => self.up,
            Dir::Down => self.down,
            Dir::Left => self.left,
            Dir::Right => self.right,
        }
    }
}

/// Something for the app to do, returned by [`Game::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Sound(SoundRef),
    /// The music to play from now on (None: none). Sent at the start and when it changes.
    Music(Option<MusicRef>),
    /// The player walked into something that doesn't react: time for a soft bump sound.
    Bump,
}

/// Something that walks from tile to tile.
#[derive(Debug, Clone, Copy)]
struct Body {
    pos: Pos,
    /// Where it's walking to; `pos` while standing.
    to: Pos,
    face: Dir,
    /// Pixels of the way to `to`.
    prog: u8,
    /// Which foot goes first, alternating with every step.
    stride: bool,
}

impl Body {
    fn at(pos: Pos, face: Dir) -> Self {
        Body {
            pos,
            to: pos,
            face,
            prog: 0,
            stride: false,
        }
    }

    fn moving(&self) -> bool {
        self.pos != self.to
    }

    fn start(&mut self, to: Pos) {
        self.to = to;
        self.prog = 0;
        self.stride = !self.stride;
    }

    /// Walks on; true once it has arrived.
    fn advance(&mut self, by: u8) -> bool {
        self.prog += by;
        if self.prog >= TILE as u8 {
            self.pos = self.to;
            self.prog = 0;
            true
        } else {
            false
        }
    }

    /// Top-left corner on the map, in pixels, `prog` pixels of the way to `to`.
    fn pixel(&self, prog: u8) -> (i32, i32) {
        let along = |from: u16, to: u16| from as i32 * T + (to as i32 - from as i32) * prog as i32;
        (along(self.pos.x, self.to.x), along(self.pos.y, self.to.y))
    }

    /// Which of a sprite's pictures to show: the first standing, the others in the middle of a step.
    fn frame(&self, prog: u8, frames: u8) -> usize {
        let mid_step = self.moving() && (4..12).contains(&prog);
        match (mid_step, frames) {
            (false, _) | (true, 1) => 0,
            (true, 2) => 1,
            (true, _) => 1 + self.stride as usize,
        }
    }
}

/// A thing on the current map, while the player is there.
struct ThingState {
    body: Body,
    /// Frames until it may walk again.
    wait: u16,
    /// Steps of its path done.
    path: usize,
    animation: Option<(Animation, u8)>,
}

/// The thing that follows the player.
struct Pet {
    id: Id,
    look: Option<SpriteRef>,
    /// Walks along with the player, so it moves by the player's `prog`.
    body: Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Pet,
    Thing(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BubbleKind {
    Picture(SpriteRef),
    Missing { icon: SpriteRef, missing: u16 },
}

struct Bubble {
    who: Who,
    kind: BubbleKind,
    left: u16,
}

#[derive(Clone, Copy)]
enum Trigger {
    Bump,
    Step,
}

/// A world being played.
pub struct Game {
    world: World,
    /// A position in `world.heroes`
    hero: usize,
    /// The current map, a position in `world.maps`
    map: usize,
    player: Body,
    pet: Option<Pet>,
    things: Vec<ThingState>,
    /// The current map's layers, with the swaps of the flags that are set
    layers: [Vec<u16>; 3],
    items: HashMap<Id, u16>,
    flags: HashSet<Id>,
    hidden: HashSet<Id>,
    bubbles: Vec<Bubble>,
    /// Frames into a fade, and where it leads
    fade: Option<(u8, Place)>,
    bump_pause: u8,
    frame: u32,
    previous: Buttons,
    /// None until the first map was entered
    music: Option<Option<MusicRef>>,
    events: Vec<Event>,
    random: u32,
    /// Counts the changes worth saving
    progress: u32,
}

impl Game {
    /// Starts the world from the beginning, as `world.heroes[hero]` (the first if out of range).
    pub fn new(world: World, hero: usize) -> Result<Game, Error> {
        world.validate()?;
        let hero = if hero < world.heroes.len() { hero } else { 0 };
        let start = world.start;
        let random = (world.uid as u32 ^ (world.uid >> 32) as u32) | 1;
        let mut game = Game {
            world,
            hero,
            map: 0,
            player: Body::at(start.at, start.facing),
            pet: None,
            things: Vec::new(),
            layers: Default::default(),
            items: HashMap::new(),
            flags: HashSet::new(),
            hidden: HashSet::new(),
            bubbles: Vec::new(),
            fade: None,
            bump_pause: 0,
            frame: 0,
            previous: Buttons::default(),
            music: None,
            events: Vec::new(),
            random,
            progress: 0,
        };
        game.enter(start);
        Ok(game)
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// Where the player is (where they're walking from, while walking).
    pub fn place(&self) -> Place {
        Place {
            map: self.current().id,
            at: self.player.pos,
            facing: self.player.face,
        }
    }

    /// How many of `item` the player carries.
    pub fn count(&self, item: Id) -> u16 {
        self.items.get(&item).copied().unwrap_or(0)
    }

    pub fn flag(&self, flag: Id) -> bool {
        self.flags.contains(&flag)
    }

    /// Whether a thing was hidden for good.
    pub fn hidden(&self, thing: Id) -> bool {
        self.hidden.contains(&thing)
    }

    /// The thing following the player.
    pub fn pet(&self) -> Option<Id> {
        self.pet.as_ref().map(|pet| pet.id)
    }

    /// Changes whenever something worth saving changed: items, flags, hidden things, the pet or
    /// the map. Where on a map the player stands isn't counted.
    pub fn progress(&self) -> u32 {
        self.progress
    }

    /// One frame: moves everything on and runs the rules the player triggers.
    pub fn step(&mut self, buttons: Buttons) -> Vec<Event> {
        self.frame = self.frame.wrapping_add(1);
        let pressed_a = buttons.a && !self.previous.a;
        self.previous = buttons;

        if let Some((frames, place)) = &mut self.fade {
            *frames += 1;
            let (frames, place) = (*frames, *place);
            if frames == FADE / 2 {
                self.enter(place);
            }
            if frames >= FADE {
                self.fade = None;
            }
        }
        self.bubbles.retain_mut(|bubble| {
            bubble.left -= 1;
            bubble.left > 0
        });
        self.bump_pause = self.bump_pause.saturating_sub(1);

        for i in 0..self.things.len() {
            self.update_thing(i);
        }
        self.update_player(buttons, pressed_a);
        std::mem::take(&mut self.events)
    }

    fn current(&self) -> &Map {
        &self.world.maps[self.map]
    }

    fn enter(&mut self, place: Place) {
        self.map = self
            .world
            .maps
            .iter()
            .position(|map| map.id == place.map)
            .expect("validated: places lead to maps");
        self.player = Body::at(place.at, place.facing);
        self.things = self.world.maps[self.map]
            .things
            .iter()
            .map(|thing| ThingState {
                body: Body::at(thing.at, Dir::Down),
                wait: 30,
                path: 0,
                animation: None,
            })
            .collect();
        self.bubbles.clear();
        self.refresh_layers();

        if self.pet.is_some() {
            let behind = place
                .at
                .step(place.facing.opposite())
                .filter(|&pos| self.current().contains(pos) && !self.tile_solid(pos))
                .unwrap_or(place.at);
            if let Some(pet) = &mut self.pet {
                pet.body = Body::at(behind, place.facing);
            }
        }

        let music = self.current().music;
        if self.music != Some(music) {
            self.events.push(Event::Music(music));
            self.music = Some(music);
        }
        self.progress += 1;
    }

    fn refresh_layers(&mut self) {
        let map = &self.world.maps[self.map];
        self.layers = [
            map.ground.0.clone(),
            map.decoration.0.clone(),
            map.above.0.clone(),
        ];
        for swap in &map.swaps {
            if self.flags.contains(&swap.when) {
                let cell = swap.at.y as usize * map.width as usize + swap.at.x as usize;
                self.layers[swap.layer as usize][cell] = swap.tile.map_or(0, |tile| tile.0 + 1);
            }
        }
    }

    fn tile_at(&self, layer: LayerKind, pos: Pos) -> Option<&Tile> {
        let cell = pos.y as usize * self.current().width as usize + pos.x as usize;
        match self.layers[layer as usize][cell] {
            0 => None,
            n => Some(&self.world.tiles[n as usize - 1]),
        }
    }

    fn tile_solid(&self, pos: Pos) -> bool {
        !self.current().contains(pos)
            || LayerKind::ALL
                .iter()
                .any(|&layer| self.tile_at(layer, pos).is_some_and(|tile| tile.solid))
    }

    fn holds(&self, condition: Condition) -> bool {
        match condition {
            Condition::Always => true,
            Condition::Has { item, count } => self.count(item) >= count,
            Condition::Flag(flag) => self.flags.contains(&flag),
            Condition::NotFlag(flag) => !self.flags.contains(&flag),
        }
    }

    /// Whether thing `i` of the current map is there right now.
    fn visible(&self, i: usize) -> bool {
        let thing = &self.current().things[i];
        !self.hidden.contains(&thing.id)
            && self.pet.as_ref().is_none_or(|pet| pet.id != thing.id)
            && self.holds(thing.shown_if)
    }

    fn covers(body: &Body, pos: Pos) -> bool {
        body.pos == pos || body.to == pos
    }

    /// The solid thing standing at (or walking onto) `pos`.
    fn solid_thing_at(&self, pos: Pos) -> Option<usize> {
        (0..self.things.len()).find(|&i| {
            self.current().things[i].solid
                && self.visible(i)
                && Self::covers(&self.things[i].body, pos)
        })
    }

    fn player_can_enter(&self, pos: Pos) -> bool {
        !self.tile_solid(pos) && self.solid_thing_at(pos).is_none()
    }

    fn thing_can_enter(&self, me: usize, pos: Pos) -> bool {
        !self.tile_solid(pos)
            && !Self::covers(&self.player, pos)
            && self
                .pet
                .as_ref()
                .is_none_or(|pet| !Self::covers(&pet.body, pos))
            && !(0..self.things.len())
                .any(|i| i != me && self.visible(i) && Self::covers(&self.things[i].body, pos))
    }

    fn next_random(&mut self) -> u32 {
        // xorshift32
        let mut x = self.random;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.random = x;
        x
    }

    fn update_thing(&mut self, i: usize) {
        if !self.visible(i) {
            return;
        }
        let state = &mut self.things[i];
        if let Some((_, frames)) = &mut state.animation {
            *frames += 1;
            if *frames >= ANIMATION_FRAMES {
                state.animation = None;
            }
        }
        if state.body.moving() {
            state.body.advance(THING_SPEED);
            return;
        }
        if state.wait > 0 {
            state.wait -= 1;
            return;
        }

        let (r1, r2) = (self.next_random(), self.next_random());
        let thing = &self.world.maps[self.map].things[i];
        let pos = self.things[i].body.pos;
        let (dir, pause, on_path) = match &thing.motion {
            Motion::Still => return,
            Motion::Wander { radius } => {
                let dir = Dir::ALL[r1 as usize % 4];
                let radius = *radius as u16;
                let home = thing.at;
                let near =
                    |to: Pos| to.x.abs_diff(home.x) <= radius && to.y.abs_diff(home.y) <= radius;
                if !pos.step(dir).is_some_and(near) {
                    self.things[i].wait = 20;
                    return;
                }
                (dir, 30 + (r2 % 70) as u16, false)
            }
            Motion::Path(steps) => (steps[self.things[i].path % steps.len()], 4, true),
        };
        match pos.step(dir).filter(|&to| self.thing_can_enter(i, to)) {
            Some(to) => {
                let state = &mut self.things[i];
                state.body.face = dir;
                state.body.start(to);
                state.wait = pause;
                state.path += on_path as usize;
            }
            None => self.things[i].wait = 10,
        }
    }

    fn update_player(&mut self, buttons: Buttons, pressed_a: bool) {
        if self.player.moving() {
            let speed = if buttons.b { RUN } else { WALK };
            if self.player.advance(speed) {
                if let Some(pet) = &mut self.pet {
                    pet.body.pos = pet.body.to;
                }
                self.arrived();
            }
        }
        if self.player.moving() || self.fade.is_some() {
            return;
        }

        // Keep going the same way while that's held, otherwise take any held direction
        let face = self.player.face;
        let dir = if buttons.held(face) {
            Some(face)
        } else {
            Dir::ALL.into_iter().find(|&dir| buttons.held(dir))
        };
        let Some(dir) = dir else {
            if pressed_a {
                let ahead = self
                    .player
                    .pos
                    .step(face)
                    .and_then(|pos| self.solid_thing_at(pos));
                if let Some(i) = ahead {
                    self.bump(i);
                }
            }
            return;
        };

        self.player.face = dir;
        let next = self.player.pos.step(dir);
        if let Some(to) = next.filter(|&to| self.player_can_enter(to)) {
            let from = self.player.pos;
            self.player.start(to);
            if let Some(pet) = &mut self.pet {
                if pet.body.pos != from {
                    pet.body.face = toward(pet.body.pos, from).unwrap_or(pet.body.face);
                    pet.body.start(from);
                }
            }
        } else if self.bump_pause == 0 {
            self.bump_pause = BUMP_PAUSE;
            match next.and_then(|to| self.solid_thing_at(to)) {
                Some(i) => self.bump(i),
                None => self.events.push(Event::Bump),
            }
        }
    }

    /// The player has just stepped onto a tile.
    fn arrived(&mut self) {
        let pos = self.player.pos;
        let sound = [LayerKind::Decoration, LayerKind::Ground]
            .into_iter()
            .find_map(|layer| self.tile_at(layer, pos))
            .and_then(|tile| tile.step_sound);
        if let Some(sound) = sound {
            self.events.push(Event::Sound(sound));
        }
        let stepped: Vec<usize> = (0..self.things.len())
            .filter(|&i| {
                !self.current().things[i].solid && self.visible(i) && self.things[i].body.pos == pos
            })
            .collect();
        for i in stepped {
            self.run(i, Trigger::Step);
        }
    }

    fn bump(&mut self, i: usize) {
        let body = &mut self.things[i].body;
        if !body.moving() {
            body.face = self.player.face.opposite();
        }
        self.run(i, Trigger::Bump);
    }

    fn run(&mut self, i: usize, trigger: Trigger) {
        let thing = &self.current().things[i];
        let rules = match trigger {
            Trigger::Bump => &thing.on_bump,
            Trigger::Step => &thing.on_step,
        };
        let Some(rule) = rules.iter().find(|rule| self.holds(rule.when)) else {
            return;
        };
        for action in rule.then.clone() {
            self.act(i, action);
        }
    }

    fn act(&mut self, i: usize, action: Action) {
        if let Action::Give { .. }
        | Action::Take { .. }
        | Action::SetFlag(_)
        | Action::ClearFlag(_)
        | Action::Hide(_)
        | Action::Join = action
        {
            self.progress += 1;
        }
        match action {
            Action::Sound(sound) => self.events.push(Event::Sound(sound)),
            Action::Bubble(picture) => self.bubble(i, BubbleKind::Picture(picture)),
            Action::BubbleMissing { item, count } => {
                let missing = count.saturating_sub(self.count(item));
                let icon = self.world.item(item).expect("validated: items exist").icon;
                self.bubble(i, BubbleKind::Missing { icon, missing });
            }
            Action::Animate(animation) => self.things[i].animation = Some((animation, 0)),
            Action::Give { item, count } => {
                let carried = self.items.entry(item).or_default();
                *carried = carried.saturating_add(count);
            }
            Action::Take { item, count } => {
                let carried = self.items.entry(item).or_default();
                *carried = carried.saturating_sub(count);
            }
            Action::SetFlag(flag) => {
                self.flags.insert(flag);
                self.refresh_layers();
            }
            Action::ClearFlag(flag) => {
                self.flags.remove(&flag);
                self.refresh_layers();
            }
            Action::Hide(thing) => {
                self.hidden.insert(thing);
                if self.pet() == Some(thing) {
                    self.pet = None;
                }
            }
            Action::Join => {
                if self.visible(i) {
                    let thing = &self.current().things[i];
                    let body = self.things[i].body;
                    self.pet = Some(Pet {
                        id: thing.id,
                        look: thing.look,
                        body: Body::at(body.pos, body.face),
                    });
                    for bubble in &mut self.bubbles {
                        if bubble.who == Who::Thing(i) {
                            bubble.who = Who::Pet;
                        }
                    }
                }
            }
            Action::Teleport(place) => {
                if self.fade.is_none() {
                    self.fade = Some((0, place));
                }
            }
        }
    }

    fn bubble(&mut self, i: usize, kind: BubbleKind) {
        let who = if self.pet() == Some(self.current().things[i].id) {
            Who::Pet
        } else {
            Who::Thing(i)
        };
        self.bubbles.retain(|bubble| bubble.who != who);
        self.bubbles.push(Bubble {
            who,
            kind,
            left: BUBBLE_FRAMES,
        });
    }

    // ---- Drawing ------------------------------------------------------------------------------

    /// Paints the current picture into `out`: [`WIDTH`] × [`HEIGHT`] RGB565 pixels, row by row.
    pub fn draw(&self, out: &mut [u16]) {
        assert_eq!(out.len(), WIDTH * HEIGHT, "the picture is WIDTH x HEIGHT");
        out.fill(0);
        let map = self.current();
        let (px, py) = self.player.pixel(self.player.prog);
        // Follow the player, but don't show what's beyond the edges; centre maps smaller than the screen
        let camera = |p: i32, view: usize, size: i32| {
            let view = view as i32;
            if size <= view {
                -(view - size) / 2
            } else {
                (p + T / 2 - view / 2).clamp(0, size - view)
            }
        };
        let cx = camera(px, WIDTH, map.width as i32 * T);
        let cy = camera(py, HEIGHT, map.height as i32 * T);

        self.draw_layer(out, LayerKind::Ground, cx, cy);
        self.draw_layer(out, LayerKind::Decoration, cx, cy);

        // Everyone who walks around, the ones further down in front
        struct Figure<'a> {
            x: i32,
            y: i32,
            pixels: &'a [u16],
            flip: bool,
        }
        let sprite = |look: SpriteRef| &self.world.sprites[look.0 as usize];
        let mut figures = Vec::new();
        for (i, state) in self.things.iter().enumerate() {
            let Some(look) = self.current().things[i].look else {
                continue;
            };
            if !self.visible(i) {
                continue;
            }
            let (mut x, mut y) = state.body.pixel(state.body.prog);
            let mut face = state.body.face;
            match state.animation {
                Some((Animation::Jump, t)) => y -= (t as i32 * (ANIMATION_FRAMES - t) as i32) / 6,
                Some((Animation::Spin, t)) => {
                    face = [Dir::Down, Dir::Left, Dir::Up, Dir::Right][(t as usize / 3) % 4]
                }
                Some((Animation::Shake, t)) => x += if t % 2 == 0 { 1 } else { -1 },
                None => {}
            }
            let (pixels, flip, hop) = pose(&state.body, state.body.prog, sprite(look), face);
            figures.push(Figure {
                x,
                y: y - hop,
                pixels,
                flip,
            });
        }
        if let Some(pet) = &self.pet {
            if let Some(look) = pet.look {
                let (x, y) = pet.body.pixel(self.player.prog);
                let (pixels, flip, hop) =
                    pose(&pet.body, self.player.prog, sprite(look), pet.body.face);
                figures.push(Figure {
                    x,
                    y: y - hop,
                    pixels,
                    flip,
                });
            }
        }
        let hero = sprite(self.world.heroes[self.hero]);
        let (pixels, flip, hop) = pose(&self.player, self.player.prog, hero, self.player.face);
        figures.push(Figure {
            x: px,
            y: py - hop,
            pixels,
            flip,
        });
        figures.sort_by_key(|figure| figure.y);
        for figure in figures {
            blit(
                out,
                figure.pixels,
                TILE,
                figure.x - cx,
                figure.y - cy,
                figure.flip,
            );
        }

        self.draw_layer(out, LayerKind::Above, cx, cy);

        for bubble in &self.bubbles {
            let (x, y) = match bubble.who {
                Who::Thing(i) if self.visible(i) => {
                    let body = &self.things[i].body;
                    body.pixel(body.prog)
                }
                Who::Pet => match &self.pet {
                    Some(pet) => pet.body.pixel(self.player.prog),
                    None => continue,
                },
                Who::Thing(_) => continue,
            };
            self.draw_bubble(out, x - cx + T / 2, y - cy, bubble.kind);
        }

        self.draw_items(out);

        if let Some((frames, _)) = self.fade {
            let half = FADE / 2;
            let dark = if frames < half { frames } else { FADE - frames };
            for pixel in out.iter_mut() {
                *pixel = shade(*pixel, (half - dark) as u32, half as u32);
            }
        }
    }

    fn draw_layer(&self, out: &mut [u16], layer: LayerKind, cx: i32, cy: i32) {
        let map = self.current();
        let time_ms = self.frame as u64 * 1000 / FPS as u64;
        let first_x = cx.div_euclid(T).max(0);
        let last_x = (cx + WIDTH as i32 - 1)
            .div_euclid(T)
            .min(map.width as i32 - 1);
        let first_y = cy.div_euclid(T).max(0);
        let last_y = (cy + HEIGHT as i32 - 1)
            .div_euclid(T)
            .min(map.height as i32 - 1);
        for ty in first_y..=last_y {
            for tx in first_x..=last_x {
                let Some(tile) = self.tile_at(layer, Pos::new(tx as u16, ty as u16)) else {
                    continue;
                };
                let frame = match tile.frames.len() {
                    1 => 0,
                    n => (time_ms / tile.frame_ms as u64) as usize % n,
                };
                blit(
                    out,
                    &tile.frames[frame].0,
                    TILE,
                    tx * T - cx,
                    ty * T - cy,
                    false,
                );
            }
        }
    }

    /// A white bubble with a tail, above the point (`x`, `y`).
    fn draw_bubble(&self, out: &mut [u16], x: i32, y: i32, kind: BubbleKind) {
        let (picture, dots) = match kind {
            BubbleKind::Picture(sprite) => (sprite, 0),
            BubbleKind::Missing { icon, missing } => (icon, missing.min(9) as i32),
        };
        let width = 20 + if dots > 0 { dots * 4 + 1 } else { 0 };
        let height = 20;
        let left = (x - width / 2).clamp(1, WIDTH as i32 - width - 1);
        let top = (y - height - 4).max(1);
        fill(out, left - 1, top - 1, width + 2, height + 2, OUTLINE);
        fill(out, left, top, width, height, WHITE);
        let tail = x.clamp(left + 3, left + width - 4);
        for row in 0..3 {
            fill(
                out,
                tail - 3 + row,
                top + height + row,
                7 - 2 * row,
                1,
                OUTLINE,
            );
            fill(
                out,
                tail - 2 + row,
                top + height + row,
                5 - 2 * row,
                1,
                WHITE,
            );
        }
        fill(out, tail, top + height + 3, 1, 1, OUTLINE);
        let picture = self.world.sprites[picture.0 as usize].picture(Dir::Down, 0);
        blit(out, picture, TILE, left + 2, top + 2, false);
        for dot in 0..dots {
            fill(out, left + 20 + dot * 4, top + 9, 3, 3, DOT);
        }
    }

    /// The items carried, as small icons at the top left.
    fn draw_items(&self, out: &mut [u16]) {
        let icons: Vec<SpriteRef> = self
            .world
            .items
            .iter()
            .flat_map(|item| std::iter::repeat_n(item.icon, self.count(item.id) as usize))
            .take(MAX_ICONS)
            .collect();
        if icons.is_empty() {
            return;
        }
        let width = icons.len() * 9 + 1;
        for y in 1..12 {
            for x in 1..1 + width {
                let pixel = &mut out[y * WIDTH + x];
                *pixel = shade(*pixel, 1, 2);
            }
        }
        for (n, icon) in icons.iter().enumerate() {
            let picture = self.world.sprites[icon.0 as usize].picture(Dir::Down, 0);
            // Half size: every other pixel
            for y in 0..TILE / 2 {
                for x in 0..TILE / 2 {
                    let pixel = picture[y * 2 * TILE + x * 2];
                    if pixel != TRANSPARENT {
                        out[(2 + y) * WIDTH + 2 + n * 9 + x] = pixel;
                    }
                }
            }
        }
    }
}

/// Which of a sprite's pictures shows a body, whether mirrored, and how far it's lifted: a picture
/// without directions is mirrored to face left, one without walking pictures hops a pixel in the
/// middle of each step.
fn pose<'a>(body: &Body, prog: u8, sprite: &'a Sprite, face: Dir) -> (&'a [u16], bool, i32) {
    let frame = body.frame(prog, sprite.frames);
    let flip = sprite.directions == 1 && face == Dir::Left;
    let hop = (sprite.frames == 1 && body.moving() && (4..12).contains(&prog)) as i32;
    (sprite.picture(face, frame), flip, hop)
}

/// The direction from `from` to its neighbour `to`.
fn toward(from: Pos, to: Pos) -> Option<Dir> {
    Dir::ALL.into_iter().find(|&dir| from.step(dir) == Some(to))
}

/// Copies a picture `width` pixels wide to (`x`, `y`), mirrored if asked, leaving out transparent
/// pixels and whatever is off the picture.
fn blit(out: &mut [u16], pixels: &[u16], width: usize, x: i32, y: i32, flip: bool) {
    for (row, line) in pixels.chunks_exact(width).enumerate() {
        let oy = y + row as i32;
        if !(0..HEIGHT as i32).contains(&oy) {
            continue;
        }
        for (col, &pixel) in line.iter().enumerate() {
            let col = if flip { width - 1 - col } else { col };
            let ox = x + col as i32;
            if pixel != TRANSPARENT && (0..WIDTH as i32).contains(&ox) {
                out[oy as usize * WIDTH + ox as usize] = pixel;
            }
        }
    }
}

fn fill(out: &mut [u16], x: i32, y: i32, width: i32, height: i32, colour: u16) {
    for oy in y.max(0)..(y + height).min(HEIGHT as i32) {
        for ox in x.max(0)..(x + width).min(WIDTH as i32) {
            out[oy as usize * WIDTH + ox as usize] = colour;
        }
    }
}

/// An RGB565 colour at `num`/`den` of its brightness.
fn shade(pixel: u16, num: u32, den: u32) -> u16 {
    let (r, g, b) = (
        (pixel >> 11) as u32,
        ((pixel >> 5) & 0x3F) as u32,
        (pixel & 0x1F) as u32,
    );
    ((r * num / den) << 11 | (g * num / den) << 5 | (b * num / den)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use sample::{APPLE, DOG, GATE_OPEN, HOUSE, VILLAGE};

    fn game() -> Game {
        Game::new(sample::world(), 0).unwrap()
    }

    fn hold(dir: Dir) -> Buttons {
        let mut buttons = Buttons::default();
        match dir {
            Dir::Up => buttons.up = true,
            Dir::Down => buttons.down = true,
            Dir::Left => buttons.left = true,
            Dir::Right => buttons.right = true,
        }
        buttons
    }

    /// Steps without buttons until nothing walks or fades any more.
    fn settle(game: &mut Game) {
        for _ in 0..100 {
            if !game.player.moving() && game.fade.is_none() {
                return;
            }
            game.step(Buttons::default());
        }
        panic!("didn't settle");
    }

    /// Holds `dir` for `tiles` steps' worth of frames, then lets go.
    fn walk(game: &mut Game, dir: Dir, tiles: usize) {
        for _ in 0..tiles * (TILE / WALK as usize) {
            game.step(hold(dir));
        }
        settle(game);
    }

    fn press_a(game: &mut Game) {
        game.step(Buttons {
            a: true,
            ..Buttons::default()
        });
        game.step(Buttons::default());
    }

    fn put(game: &mut Game, x: u16, y: u16, face: Dir) {
        game.player = Body::at(Pos::new(x, y), face);
    }

    #[test]
    fn sample_world_survives_a_file() {
        let world = sample::world();
        assert_eq!(World::from_bytes(&world.to_bytes()).unwrap(), world);
    }

    #[test]
    fn starts_with_music() {
        let mut game = game();
        assert_eq!(game.place().map, VILLAGE);
        assert_eq!(
            game.step(Buttons::default()),
            [Event::Music(Some(MusicRef(0)))]
        );
        assert_eq!(game.step(Buttons::default()), []);
    }

    #[test]
    fn walls_stop_her() {
        let mut game = game();
        put(&mut game, 3, 8, Dir::Down);
        walk(&mut game, Dir::Left, 5);
        assert_eq!(game.place().at, Pos::new(1, 8));
        // Into the tree at x 0: a bump (at the latest once the pause after the last one is over)
        let events: Vec<Event> = (0..=BUMP_PAUSE)
            .flat_map(|_| game.step(hold(Dir::Left)))
            .collect();
        assert!(events.contains(&Event::Bump));
        assert_eq!(game.place().at, Pos::new(1, 8));
        // The fence and the closed gate
        put(&mut game, 9, 8, Dir::Down);
        walk(&mut game, Dir::Down, 1);
        assert_eq!(game.place().at, Pos::new(9, 8));
    }

    #[test]
    fn doors_lead_inside_and_back() {
        let mut game = game();
        put(&mut game, 3, 4, Dir::Down);
        walk(&mut game, Dir::Up, 1);
        assert_eq!(
            game.place(),
            Place {
                map: HOUSE,
                at: Pos::new(4, 5),
                facing: Dir::Up
            }
        );
        walk(&mut game, Dir::Down, 1);
        assert_eq!(
            game.place(),
            Place {
                map: VILLAGE,
                at: Pos::new(3, 4),
                facing: Dir::Down
            }
        );
    }

    #[test]
    fn apples_are_picked_up() {
        let mut game = game();
        put(&mut game, 14, 8, Dir::Right);
        walk(&mut game, Dir::Right, 1);
        assert_eq!(game.count(APPLE), 1);
        walk(&mut game, Dir::Left, 1);
        walk(&mut game, Dir::Right, 1);
        assert_eq!(game.count(APPLE), 1, "gone after the first time");
    }

    #[test]
    fn the_horse_opens_the_gate_for_five_apples() {
        let mut game = game();
        put(&mut game, 9, 8, Dir::Right);
        press_a(&mut game);
        assert!(!game.flag(GATE_OPEN));
        assert!(matches!(
            game.bubbles[..],
            [Bubble {
                kind: BubbleKind::Missing { missing: 5, .. },
                ..
            }]
        ));

        game.items.insert(APPLE, 5);
        press_a(&mut game);
        assert!(game.flag(GATE_OPEN));
        assert_eq!(game.count(APPLE), 0);
        walk(&mut game, Dir::Down, 2);
        assert_eq!(game.place().at, Pos::new(9, 10));
    }

    #[test]
    fn the_dog_follows_her() {
        let mut game = game();
        game.flags.insert(GATE_OPEN);
        put(&mut game, 11, 12, Dir::Right);
        press_a(&mut game);
        assert_eq!(game.pet(), Some(DOG));
        walk(&mut game, Dir::Left, 2);
        assert_eq!(game.pet.as_ref().unwrap().body.pos, Pos::new(10, 12));
    }

    #[test]
    fn draws_something() {
        let mut game = game();
        let mut picture = vec![0; WIDTH * HEIGHT];
        for _ in 0..40 {
            game.step(hold(Dir::Up));
            game.draw(&mut picture);
        }
        assert!(picture.iter().filter(|&&pixel| pixel != 0).count() > WIDTH * HEIGHT / 2);
    }
}
