//! What each right-click arm of the chain accepts, as one pure predicate per tool
//! or material, shared by the arm and by the hand rule.
//!
//! [`crate::hands::use_hand`] gives the click to the off-hand when the main-hand
//! item "has no use for this target". That question must have the arm's own answer,
//! or the two drift: a hoe aimed at stone claimed the click (the old rule said "a
//! hoe has some use") while its arm declined, and the torch in the other hand never
//! got its turn. So each arm in `game_loop` asks the function here for its target
//! test, and [`crate::hands::item_has_use`] asks the same function.
//!
//! A predicate takes the plain values the arm already has (the block under the
//! crosshair, the face clicked, whether the play mode can edit) so it needs no
//! world. What a predicate cannot see stays the arm's own refusal: the Commons
//! rule (it toasts), a water rule, a full bag. Where an arm claims every click
//! (a fishing rod's cast or its "aim at water" toast, the Drafting Stamp) the
//! predicate says so.

use crate::block::{self, BlockId};
use crate::mob::MobType;

/// The face of a block that points up: the only one a hoe, a seed or a reed takes.
pub const FACE_UP: [i32; 3] = [0, 1, 0];

/// A hoe tills the top face of `target` (dirt or grass) when the play mode can edit
/// (`use_edits::tills`, the rule the server judges a joiner's till by).
pub fn hoe_tills(target: Option<BlockId>, face: [i32; 3], edit_blocked: bool) -> bool {
    !edit_blocked && face == FACE_UP && target.is_some_and(crate::use_edits::tills)
}

/// An Eraser lifts a Blueprint Paper tile (`use_edits::erases`).
pub fn eraser_erases(target: Option<BlockId>, edit_blocked: bool) -> bool {
    !edit_blocked && target.is_some_and(crate::use_edits::erases)
}

/// Shears take the wool of a sheep (`mob_interact::shear`, `local_mob_click::plan_right_click`).
/// A hive is the other use: `hands::block_takes_held_item` keeps that click on the
/// main hand.
pub fn shears_cut(kind: MobType) -> bool {
    kind == MobType::Sheep
}

/// Flint and steel strikes an unlit campfire (`block_use::light_campfire`).
///
/// BRIDGE: `game_loop`'s keg and fire arms for flint and steel (a blasting keg's
/// fuse, a flammable block's fire) sit INSIDE the campfire branch and cannot run
/// (found by the off-hand review fixes; not changed here). Until they are lifted
/// out, flint and steel has a use only at a campfire, and this says so; lifting
/// them means adding them here and calling this from them.
pub fn flint_lights(target: Option<BlockId>) -> bool {
    target == Some(block::CAMPFIRE_UNLIT)
}

/// The Drafting Stamp captures the structure over the paper below ANY block it is
/// aimed at: its arm claims every click on a block (a toast says why when it
/// cannot).
pub fn stamp_captures(target: Option<BlockId>) -> bool {
    target.is_some()
}

/// A fishing rod's arm answers every click: it casts, reels, or says "aim at water".
/// (Minecraft's rod likewise always casts.)
pub fn rod_answers() -> bool {
    true
}

/// Salt turns dirt, grass or snow into a salt path (`snowfall::target_block_accepts_salt_path`).
pub fn salt_paths(target: Option<BlockId>, edit_blocked: bool) -> bool {
    !edit_blocked && target.is_some_and(crate::snowfall::target_block_accepts_salt_path)
}

/// Bone meal grows tall grass on grass and advances a crop (the two arms).
pub fn bonemeal_grows(target: Option<BlockId>, edit_blocked: bool) -> bool {
    !edit_blocked && target.is_some_and(|b| b == block::GRASS || crate::growth::is_crop(b))
}

/// Fertiliser advances a crop.
pub fn fertiliser_grows(target: Option<BlockId>, edit_blocked: bool) -> bool {
    !edit_blocked && target.is_some_and(crate::growth::is_crop)
}

/// A papyrus reed is planted on the top face of dirt, grass or sand. (The arm's
/// further water check stays its own silent refusal.)
pub fn reed_plants(target: Option<BlockId>, face: [i32; 3]) -> bool {
    face == FACE_UP && target.is_some_and(|b| matches!(b, block::DIRT | block::GRASS | block::SAND))
}

/// A seed is sown on the top face of tilled soil. Tilled soil's top-face arm
/// takes every click there, seeds or not.
pub fn seeds_sown(target: Option<BlockId>, face: [i32; 3]) -> bool {
    face == FACE_UP && target == Some(block::TILLED_SOIL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hoe_tills_only_the_top_of_dirt_or_grass_where_edits_are_allowed() {
        for b in [block::DIRT, block::GRASS] {
            assert!(hoe_tills(Some(b), FACE_UP, false), "{b}");
            assert!(!hoe_tills(Some(b), [1, 0, 0], false), "a side face");
            assert!(!hoe_tills(Some(b), FACE_UP, true), "a mode that cannot edit");
        }
        assert!(!hoe_tills(Some(block::STONE), FACE_UP, false));
        assert!(!hoe_tills(None, FACE_UP, false));
    }

    #[test]
    fn the_other_arms_accept_what_they_act_on_and_nothing_else() {
        assert!(eraser_erases(Some(block::BLUEPRINT_PAPER), false) && !eraser_erases(Some(block::STONE), false));
        assert!(shears_cut(MobType::Sheep) && !shears_cut(MobType::Cow));
        assert!(flint_lights(Some(block::CAMPFIRE_UNLIT)) && !flint_lights(Some(block::STONE)) && !flint_lights(Some(block::OAK_LOG)));
        assert!(stamp_captures(Some(block::STONE)) && !stamp_captures(None));
        assert!(salt_paths(Some(block::GRASS), false) && !salt_paths(Some(block::STONE), false));
        assert!(bonemeal_grows(Some(block::GRASS), false) && !bonemeal_grows(Some(block::STONE), false));
        assert!(fertiliser_grows(Some(block::WHEAT_STAGE_0), false) && !fertiliser_grows(Some(block::GRASS), false));
        assert!(reed_plants(Some(block::SAND), FACE_UP) && !reed_plants(Some(block::SAND), [0, 0, 1]) && !reed_plants(Some(block::STONE), FACE_UP));
        assert!(seeds_sown(Some(block::TILLED_SOIL), FACE_UP) && !seeds_sown(Some(block::DIRT), FACE_UP));
    }
}
