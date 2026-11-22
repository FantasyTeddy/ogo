//! Saving a game and going on with it later, also after the world was changed.

use std::collections::HashMap;

use adventure_format::{Error, Id, Place, SpriteRef, World};
use serde::{Deserialize, Serialize};

use crate::{Body, Game, Pet};

/// The first bytes of a saved game.
pub const MAGIC: &[u8; 3] = b"ADS";
/// The version of the save format, after [`MAGIC`].
pub const VERSION: u16 = 1;

/// Where a player is in a world and what they have done there. Refers to the world only by stable
/// IDs, so it still fits after the world was changed and written again: [`Game::resume`] keeps what
/// still exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Save {
    /// The world's [`World::uid`].
    pub uid: u64,
    /// A position in [`World::heroes`].
    pub hero: u16,
    pub place: Place,
    pub items: Vec<(Id, u16)>,
    pub flags: Vec<Id>,
    pub hidden: Vec<Id>,
    pub pet: Option<Id>,
}

impl Save {
    /// [`MAGIC`], [`VERSION`] as a little-endian u16, then the save encoded with postcard.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(VERSION.to_le_bytes());
        bytes.extend(postcard::to_allocvec(self).expect("Save always encodes"));
        bytes
    }

    /// None if it isn't a save of this version.
    pub fn from_bytes(bytes: &[u8]) -> Option<Save> {
        let rest = bytes.strip_prefix(MAGIC)?;
        let (version, rest) = rest.split_first_chunk::<2>()?;
        if u16::from_le_bytes(*version) != VERSION {
            return None;
        }
        postcard::from_bytes(rest).ok()
    }
}

impl Game {
    pub fn save(&self) -> Save {
        let mut items: Vec<(Id, u16)> = self
            .items
            .iter()
            .filter(|(_, &count)| count > 0)
            .map(|(&id, &count)| (id, count))
            .collect();
        items.sort();
        let mut flags: Vec<Id> = self.flags.iter().copied().collect();
        flags.sort();
        let mut hidden: Vec<Id> = self.hidden.iter().copied().collect();
        hidden.sort();
        // While going through a door, where it leads
        let place = match self.fade {
            Some((_, place)) => place,
            None => self.place(),
        };
        Save {
            uid: self.world.uid,
            hero: self.hero as u16,
            place,
            items,
            flags,
            hidden,
            pet: self.pet(),
        }
    }

    /// Goes on with a saved game. Keeps what still fits the world: items, flags and things that
    /// still exist, and the place if it's still somewhere to stand; otherwise the player starts at
    /// the world's start. A save of another world starts this one from the beginning.
    pub fn resume(world: World, save: &Save) -> Result<Game, Error> {
        if save.uid != world.uid {
            return Game::new(world, 0);
        }
        let mut game = Game::new(world, save.hero as usize)?;
        let world = &game.world;
        let looks: HashMap<Id, Option<SpriteRef>> = world
            .maps
            .iter()
            .flat_map(|map| map.things.iter().map(|thing| (thing.id, thing.look)))
            .collect();
        game.items = save
            .items
            .iter()
            .filter(|(id, _)| world.item(*id).is_some())
            .copied()
            .collect();
        game.flags = save
            .flags
            .iter()
            .filter(|flag| world.flags.contains(flag))
            .copied()
            .collect();
        game.hidden = save
            .hidden
            .iter()
            .filter(|id| looks.contains_key(id))
            .copied()
            .collect();
        game.pet = save
            .pet
            .filter(|id| looks.contains_key(id) && !game.hidden.contains(id))
            .map(|id| Pet {
                id,
                look: looks[&id],
                body: Body::at(save.place.at, save.place.facing),
            });

        let start = game.world.start;
        let fits = game
            .world
            .map(save.place.map)
            .is_some_and(|map| map.contains(save.place.at));
        game.restart_in(if fits { save.place } else { start });
        if fits && !game.player_can_enter(save.place.at) {
            game.restart_in(start);
        }
        Ok(game)
    }

    /// Enters `place` as if the game had just started there.
    fn restart_in(&mut self, place: Place) {
        self.events.clear();
        self.music = None;
        self.enter(place);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, APPLE, DOG, GATE_OPEN, HOUSE, VILLAGE};
    use crate::{Buttons, Event};
    use adventure_format::{Dir, Pos};

    fn played() -> Game {
        let mut game = Game::new(sample::world(), 0).unwrap();
        game.items.insert(APPLE, 3);
        game.flags.insert(GATE_OPEN);
        game.hidden.insert(Id(101));
        game.pet = Some(Pet {
            id: DOG,
            look: None,
            body: Body::at(Pos::new(3, 4), Dir::Down),
        });
        game.restart_in(Place {
            map: HOUSE,
            at: Pos::new(2, 2),
            facing: Dir::Left,
        });
        game
    }

    #[test]
    fn bytes_round_trip() {
        let save = played().save();
        assert_eq!(Save::from_bytes(&save.to_bytes()), Some(save.clone()));
        assert_eq!(Save::from_bytes(b"ADV\x01\x00"), None);
        let mut bytes = save.to_bytes();
        bytes[3] = 9;
        assert_eq!(Save::from_bytes(&bytes), None);
    }

    #[test]
    fn resumes_where_it_was() {
        let save = played().save();
        let mut game = Game::resume(sample::world(), &save).unwrap();
        assert_eq!(game.save(), save);
        assert_eq!(game.place().map, HOUSE);
        assert_eq!(game.pet(), Some(DOG));
        // Music for the house, once
        assert_eq!(
            game.step(Buttons::default()),
            [Event::Music(Some(adventure_format::MusicRef(1)))]
        );
    }

    #[test]
    fn keeps_what_still_fits() {
        let save = played().save();
        let mut world = sample::world();
        // The apples, the gate's flag and the dog are gone; grandma's house got smaller
        world.items.clear();
        for map in &mut world.maps {
            for thing in &mut map.things {
                thing.on_step.clear();
                thing.on_bump.clear();
            }
            map.things.retain(|thing| thing.id != DOG);
            map.swaps.clear();
        }
        world.flags.retain(|&flag| flag != GATE_OPEN);
        let house = world.maps.iter_mut().find(|map| map.id == HOUSE).unwrap();
        house.width = 2;
        house.things.retain(|thing| thing.at.x < 2);
        for layer in [&mut house.ground, &mut house.decoration, &mut house.above] {
            layer.0 = layer
                .0
                .chunks(10)
                .flat_map(|row| row[..2].to_vec())
                .collect();
        }
        world.validate().unwrap();

        let game = Game::resume(world, &save).unwrap();
        let resumed = game.save();
        assert_eq!(resumed.items, []);
        assert_eq!(resumed.flags, []);
        assert_eq!(resumed.hidden, [Id(101)]);
        assert_eq!(resumed.pet, None);
        assert_eq!(
            resumed.place.map, VILLAGE,
            "(2, 2) is no longer in the house"
        );
    }

    #[test]
    fn not_into_a_wall() {
        let mut save = played().save();
        save.place = Place {
            map: VILLAGE,
            at: Pos::new(0, 0),
            facing: Dir::Up,
        };
        let game = Game::resume(sample::world(), &save).unwrap();
        assert_eq!(game.place().at, sample::world().start.at);
    }

    #[test]
    fn another_world_starts_fresh() {
        let mut save = played().save();
        save.uid += 1;
        let game = Game::resume(sample::world(), &save).unwrap();
        assert_eq!(game.count(APPLE), 0);
        assert_eq!(game.place(), sample::world().start);
    }
}
