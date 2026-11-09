//! Living world Tier 2 S9 — Satoshi's gardener lines for the two new sights:
//! a Worn Path underfoot and a Blighted Crop nearby (spec §11). The third line,
//! for a passed-on Plot, is S5's (`succession::SATOSHI_PASSED_ON`).
//!
//! Each line is said once per seat per world session, the first time that seat
//! sees the thing. Gardener voice: a grey place is a puzzle, not a fault. The
//! rule is pure (a world, the seats' feet and a record of who has heard what),
//! so the game loop only supplies positions and shows the toast.

use std::collections::HashSet;

use glam::Vec3;

use crate::block;
use crate::world::World;

/// The sights Satoshi comments on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sight {
    /// Standing on a Worn Path.
    WornPath,
    /// A Blighted Crop within [`BLIGHT_NOTICE_RADIUS`] blocks.
    BlightedCrop,
}

/// What Satoshi says on first standing on a Worn Path.
pub const WORN_PATH_LINE: &str = "A path! Feet wore that in. Places people use get paths.";

/// What Satoshi says on first seeing a Blighted Crop.
pub const BLIGHTED_CROP_LINE: &str =
    "That crop's gone grey. A field of just one plant gets sick easily. Try mixing in other plants.";

/// Blocks either side (x and z) in which a Blighted Crop is noticed.
pub const BLIGHT_NOTICE_RADIUS: i32 = 4;
/// Blocks above and below the feet in which a Blighted Crop is noticed.
pub const BLIGHT_NOTICE_HEIGHT: i32 = 2;

/// Logical ticks between looks (about a second at 20 TPS).
pub const LOOK_EVERY_TICKS: u64 = 20;

impl Sight {
    pub fn line(self) -> &'static str {
        match self {
            Sight::WornPath => WORN_PATH_LINE,
            Sight::BlightedCrop => BLIGHTED_CROP_LINE,
        }
    }
}

/// Which seats have heard which line this world session. Runtime only.
#[derive(Clone, Debug, Default)]
pub struct Told {
    heard: HashSet<(usize, Sight)>,
}

impl Told {
    /// Forget everything: a new world was entered.
    pub fn reset(&mut self) {
        self.heard.clear();
    }

    pub fn has_heard(&self, seat: usize, sight: Sight) -> bool {
        self.heard.contains(&(seat, sight))
    }
}

/// Is a Worn Path under `feet`?
pub fn on_worn_path(world: &World, feet: Vec3) -> bool {
    let (x, z) = (feet.x.floor() as i32, feet.z.floor() as i32);
    world.get_block(x, (feet.y - 0.05).floor() as i32, z) == block::WORN_PATH
}

/// Is a Blighted Crop within a few blocks of `feet`?
pub fn blight_near(world: &World, feet: Vec3) -> bool {
    let (cx, cy, cz) = (feet.x.floor() as i32, feet.y.floor() as i32, feet.z.floor() as i32);
    for dx in -BLIGHT_NOTICE_RADIUS..=BLIGHT_NOTICE_RADIUS {
        for dz in -BLIGHT_NOTICE_RADIUS..=BLIGHT_NOTICE_RADIUS {
            for dy in -BLIGHT_NOTICE_HEIGHT..=BLIGHT_NOTICE_HEIGHT {
                if world.get_block(cx + dx, cy + dy, cz + dz) == block::BLIGHTED_CROP {
                    return true;
                }
            }
        }
    }
    false
}

/// The next line due, if any: the first of `seats` (`(seat index, feet)`, the
/// living ones) that is on a Worn Path or near a Blighted Crop and has not yet
/// heard that line. Marks only the line it returns, so a second sight at the
/// same moment is said a look later, never dropped. Call it about once a second
/// and only when no other toast is showing.
pub fn next_line(world: &World, told: &mut Told, seats: &[(usize, Vec3)]) -> Option<&'static str> {
    for &(seat, feet) in seats {
        if !told.has_heard(seat, Sight::WornPath) && on_worn_path(world, feet) {
            told.heard.insert((seat, Sight::WornPath));
            return Some(Sight::WornPath.line());
        }
        if !told.has_heard(seat, Sight::BlightedCrop) && blight_near(world, feet) {
            told.heard.insert((seat, Sight::BlightedCrop));
            return Some(Sight::BlightedCrop.line());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy_lint::{banned_in, BANNED_MONEY_WORDS};

    fn world_with_ground() -> World {
        let mut w = World::new();
        for x in -8..8 {
            for z in -8..8 {
                w.set_block(x, 63, z, block::GRASS);
            }
        }
        w
    }

    const FEET: Vec3 = Vec3::new(0.5, 64.0, 0.5);

    #[test]
    fn a_worn_path_underfoot_fires_once_per_seat_and_only_on_the_right_block() {
        let mut w = world_with_ground();
        let mut told = Told::default();
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), None, "grass says nothing");
        w.set_block(0, 62, 0, block::WORN_PATH);
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), None, "a path a block down is not underfoot");
        w.set_block(0, 63, 0, block::WORN_PATH);
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), Some(WORN_PATH_LINE));
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), None, "once");
        // A second seat has not heard it yet.
        assert_eq!(next_line(&w, &mut told, &[(0, FEET), (1, FEET)]), Some(WORN_PATH_LINE));
        assert_eq!(next_line(&w, &mut told, &[(0, FEET), (1, FEET)]), None);
        // Dirt that is not a Worn Path is not one.
        let mut other = world_with_ground();
        other.set_block(0, 63, 0, block::DIRT);
        assert_eq!(next_line(&other, &mut Told::default(), &[(0, FEET)]), None);
    }

    #[test]
    fn a_blighted_crop_within_a_few_blocks_fires_once_per_seat_and_not_beyond() {
        let mut w = world_with_ground();
        let mut told = Told::default();
        w.set_block(BLIGHT_NOTICE_RADIUS + 1, 64, 0, block::BLIGHTED_CROP);
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), None, "just out of range");
        w.set_block(BLIGHT_NOTICE_RADIUS, 64, 0, block::BLIGHTED_CROP);
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), Some(BLIGHTED_CROP_LINE));
        assert_eq!(next_line(&w, &mut told, &[(0, FEET)]), None, "once");
        assert_eq!(next_line(&w, &mut told, &[(1, FEET)]), Some(BLIGHTED_CROP_LINE), "another seat");
        // Healthy wheat nearby is no blight.
        let mut healthy = world_with_ground();
        healthy.set_block(1, 64, 0, block::WHEAT_STAGE_2);
        assert_eq!(next_line(&healthy, &mut Told::default(), &[(0, FEET)]), None);
    }

    #[test]
    fn two_sights_at_once_are_said_one_look_apart_and_none_is_dropped() {
        let mut w = world_with_ground();
        w.set_block(0, 63, 0, block::WORN_PATH);
        w.set_block(2, 64, 0, block::BLIGHTED_CROP);
        let mut told = Told::default();
        let seats = [(0, FEET)];
        assert_eq!(next_line(&w, &mut told, &seats), Some(WORN_PATH_LINE));
        assert_eq!(next_line(&w, &mut told, &seats), Some(BLIGHTED_CROP_LINE));
        assert_eq!(next_line(&w, &mut told, &seats), None);
    }

    #[test]
    fn a_new_world_session_starts_everyone_fresh() {
        let mut w = world_with_ground();
        w.set_block(0, 63, 0, block::WORN_PATH);
        let mut told = Told::default();
        assert!(next_line(&w, &mut told, &[(0, FEET)]).is_some());
        told.reset();
        assert!(!told.has_heard(0, Sight::WornPath));
        assert!(next_line(&w, &mut told, &[(0, FEET)]).is_some());
    }

    #[test]
    fn the_lines_are_plain_and_money_free() {
        for line in [WORN_PATH_LINE, BLIGHTED_CROP_LINE] {
            assert!(banned_in(line, BANNED_MONEY_WORDS).is_empty(), "{line}");
            assert!(banned_in(&format!("Satoshi: {line}"), BANNED_MONEY_WORDS).is_empty());
        }
    }
}
