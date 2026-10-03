//! Buckets MC-parity — the fill/empty rules as pure decisions.
//!
//! An empty [`MaterialId::Bucket`] is filled by right-clicking a liquid
//! *source* block (flowing liquid can't be bottled, matching Minecraft),
//! yielding a [`MaterialId::WaterBucket`] or [`MaterialId::LavaBucket`]. A
//! filled bucket empties into an AIR cell, registering a new liquid source
//! there and returning the empty Bucket; water poured beside lava freezes to
//! obsidian via the existing lava sim rule (`lava.rs`).
//!
//! These two functions are the *rules* only — the orchestration (consuming the
//! held stack, mutating the world, registering the source with the liquid sim,
//! broadcasting the block change) lives in the interaction handler in
//! `game_loop.rs`. Keeping the decision pure mirrors the neighbouring
//! `papyrus::is_valid_planting_base` / `rubber::apply_tap` pattern and makes
//! the rules testable in isolation (and reusable by a future server-side path).

use crate::block::{self, BlockId};
use crate::item::{Item, MaterialId};

/// The filled bucket an empty [`MaterialId::Bucket`] becomes when used on a
/// liquid block, or `None` if this isn't a fillable interaction.
///
/// * `held` — the item in hand (must be an empty `Bucket`; a `MilkBucket` or a
///   filled bucket does not fill).
/// * `target_block` — the block being right-clicked (must be `WATER`/`LAVA`).
/// * `is_source` — whether that block is a *source* cell; flowing liquid can't
///   be bottled, so a non-source target yields `None`.
pub fn fill_result(held: &Item, target_block: BlockId, is_source: bool) -> Option<MaterialId> {
    if !matches!(held, Item::Material(MaterialId::Bucket)) || !is_source {
        return None;
    }
    match target_block {
        block::WATER => Some(MaterialId::WaterBucket),
        block::LAVA => Some(MaterialId::LavaBucket),
        _ => None,
    }
}

/// The liquid a filled bucket places when emptied into `dest_block`, paired
/// with the filled material that gets consumed, or `None` if this isn't an
/// emptyable interaction.
///
/// * `held` — the item in hand (must be a `WaterBucket` or `LavaBucket`).
/// * `dest_block` — the cell the liquid would occupy (must be `AIR`).
///
/// Returns `(liquid_block_to_place, filled_material_consumed)`.
pub fn empty_result(held: &Item, dest_block: BlockId) -> Option<(BlockId, MaterialId)> {
    if dest_block != block::AIR {
        return None;
    }
    match held {
        Item::Material(MaterialId::WaterBucket) => Some((block::WATER, MaterialId::WaterBucket)),
        Item::Material(MaterialId::LavaBucket) => Some((block::LAVA, MaterialId::LavaBucket)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat(id: MaterialId) -> Item {
        Item::Material(id)
    }

    #[test]
    fn empty_bucket_fills_from_water_source() {
        assert_eq!(
            fill_result(&mat(MaterialId::Bucket), block::WATER, true),
            Some(MaterialId::WaterBucket)
        );
    }

    #[test]
    fn empty_bucket_fills_from_lava_source() {
        assert_eq!(
            fill_result(&mat(MaterialId::Bucket), block::LAVA, true),
            Some(MaterialId::LavaBucket)
        );
    }

    #[test]
    fn flowing_liquid_cannot_be_bottled() {
        // Non-source water/lava (is_source = false) yields nothing, matching MC.
        assert_eq!(fill_result(&mat(MaterialId::Bucket), block::WATER, false), None);
        assert_eq!(fill_result(&mat(MaterialId::Bucket), block::LAVA, false), None);
    }

    #[test]
    fn fill_requires_an_empty_bucket() {
        // A milk bucket or an already-filled bucket does not refill.
        assert_eq!(fill_result(&mat(MaterialId::MilkBucket), block::WATER, true), None);
        assert_eq!(fill_result(&mat(MaterialId::WaterBucket), block::WATER, true), None);
        assert_eq!(fill_result(&mat(MaterialId::LavaBucket), block::LAVA, true), None);
    }

    #[test]
    fn fill_only_targets_liquid_blocks() {
        // A source-flagged non-liquid (shouldn't happen in practice) is inert.
        assert_eq!(fill_result(&mat(MaterialId::Bucket), block::STONE, true), None);
        assert_eq!(fill_result(&mat(MaterialId::Bucket), block::AIR, true), None);
    }

    #[test]
    fn water_bucket_empties_into_air() {
        assert_eq!(
            empty_result(&mat(MaterialId::WaterBucket), block::AIR),
            Some((block::WATER, MaterialId::WaterBucket))
        );
    }

    #[test]
    fn lava_bucket_empties_into_air() {
        assert_eq!(
            empty_result(&mat(MaterialId::LavaBucket), block::AIR),
            Some((block::LAVA, MaterialId::LavaBucket))
        );
    }

    #[test]
    fn empty_needs_an_air_cell() {
        // Can't pour a bucket into a solid (or already-liquid) cell.
        assert_eq!(empty_result(&mat(MaterialId::WaterBucket), block::STONE), None);
        assert_eq!(empty_result(&mat(MaterialId::LavaBucket), block::WATER), None);
    }

    #[test]
    fn empty_requires_a_filled_bucket() {
        // An empty Bucket or a non-bucket item pours nothing.
        assert_eq!(empty_result(&mat(MaterialId::Bucket), block::AIR), None);
        assert_eq!(empty_result(&mat(MaterialId::Stick), block::AIR), None);
    }
}
