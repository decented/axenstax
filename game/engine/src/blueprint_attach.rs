//! Spec 38 — laying a blueprint Plan flat on a floor.
//!
//! A Latent blueprint is painted as a paper-thin `FaceAttachment::Blueprint`
//! on the TOP face of the targeted floor block. It has zero height and zero
//! collision: nothing is placed in the air cell above, and no `BlockChange`
//! is emitted (attachments aren't blocks — this matches the wallpaper paint
//! path). This is the fix for the old "block too tall" bug, where laying a
//! Latent plan placed a full `LATENT_PRINT` cube in the cell above the floor.

use crate::world::{FaceAttachment, World};

/// Lay a Latent blueprint plan flat on the TOP face of the targeted floor
/// block. Returns true (and sets the attachment) iff the target face is Top
/// and the target block is solid; false (no-op) otherwise. Render-only,
/// zero-height — no block placed, no BlockChange.
pub fn lay_blueprint_on_floor(
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    target_block: (i32, i32, i32),
    target_face: [i32; 3],
    plan: crate::plan::PlanData,
) -> bool {
    // Blueprints lay flat on floors only — the targeted face must be Top.
    if target_face != [0, 1, 0] {
        return false;
    }
    // The block we paint onto must be solid (it normally is, since it was a
    // raycast hit, but guard anyway so we never attach to AIR/WATER).
    let target_id = world.get_block(target_block.0, target_block.1, target_block.2);
    if !registry.is_solid(target_id) {
        return false;
    }
    world.set_face_attachment(
        target_block,
        crate::mesh::Face::Top.index(),
        FaceAttachment::Blueprint(Box::new(plan)),
    );
    true
}

/// Lay blank cream draughting paper flat on the TOP face of the targeted
/// floor block. Returns true (and sets a BlueprintBlank attachment) iff the
/// target face is Top and the target block is solid; false (no-op) otherwise.
/// Zero-height — no block placed, no BlockChange. (Capture comes later.)
pub fn lay_blank_blueprint_paper(
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    target_block: (i32, i32, i32),
    target_face: [i32; 3],
) -> bool {
    // Blank paper lays flat on floors only — the targeted face must be Top.
    if target_face != [0, 1, 0] {
        return false;
    }
    // Only attach to a solid block (never AIR/WATER).
    let target_id = world.get_block(target_block.0, target_block.1, target_block.2);
    if !registry.is_solid(target_id) {
        return false;
    }
    world.set_face_attachment(
        target_block,
        crate::mesh::Face::Top.index(),
        FaceAttachment::BlueprintBlank,
    );
    true
}

/// Decide which inventory item a peeled/destroyed face attachment recovers to.
/// DRYs the three peel/destroy sites in `game_loop.rs`:
/// - `Wallpaper(b)`     → `Item::Block(b)` (the wallpaper block)
/// - `BlueprintBlank`   → `Item::Block(BLUEPRINT_PAPER)` (the cream sheet back)
/// - `Blueprint(plan)`  → `Item::Plan(*plan)` (the captured-plan paper back)
pub fn recovered_item_for(att: &FaceAttachment) -> crate::item::Item {
    match att {
        FaceAttachment::Wallpaper(b) => crate::item::Item::Block(*b),
        FaceAttachment::BlueprintBlank => {
            crate::item::Item::Block(crate::block::BLUEPRINT_PAPER)
        }
        FaceAttachment::Blueprint(plan) => crate::item::Item::Plan((**plan).clone()),
    }
}

/// C3c-3r — take every face attachment off the block at `pos` and return what
/// each recovers to. A single-player or host seat gets all of them
/// (`recovered_item_for`). A JOINED client's copy of a laid Blueprint is the
/// server's to own (C3c-3b): it grants nothing for one and leaves it standing
/// in the client's world copy. Wallpaper and blank paper are unchanged.
pub fn take_recoverable_attachments(
    world: &mut World,
    pos: (i32, i32, i32),
    joined: bool,
) -> Vec<crate::item::Item> {
    let mut items = Vec::new();
    for (face_idx, att) in world.remove_face_attachments_at(pos).into_iter().enumerate() {
        match att {
            Some(att @ FaceAttachment::Blueprint(_)) if joined => {
                world.set_face_attachment(pos, face_idx, att);
            }
            Some(att) => items.push(recovered_item_for(&att)),
            None => {}
        }
    }
    items
}

/// Resolve the blueprint paper tile beneath a clicked block by scanning straight
/// down its column. Returns the first `(x, y, z)` at or below `clicked` whose TOP
/// face carries a `BlueprintBlank` attachment — scanning from `clicked.1` down to
/// `clicked.1 - plan::MAX_HEIGHT` (the capture height cap) — or `None` if no
/// papered tile is found in that span. Used by the Drafting Stamp: stamp any block
/// of your build and it finds the paper underneath. Returns the clicked block
/// itself if it is the papered tile (you stamped the paper directly).
pub fn paper_tile_under(world: &World, clicked: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
    let (x, y0, z) = clicked;
    for y in (y0 - crate::plan::MAX_HEIGHT..=y0).rev() {
        if matches!(
            world.face_attachment_at((x, y, z), crate::mesh::Face::Top.index()),
            Some(FaceAttachment::BlueprintBlank)
        ) {
            return Some((x, y, z));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::plan::PlanData;
    use crate::world::World;

    fn solid_floor() -> (World, BlockRegistry, (i32, i32, i32)) {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        let pos = (3, 64, 5);
        world.set_block(pos.0, pos.1, pos.2, block::STONE);
        (world, registry, pos)
    }

    #[test]
    fn lays_on_top_face_of_solid_block_no_cube_above() {
        let (mut world, registry, pos) = solid_floor();
        let laid = lay_blueprint_on_floor(
            &mut world,
            &registry,
            pos,
            [0, 1, 0],
            PlanData::debug_3x3_stone(),
        );
        assert!(laid, "laying on a Top face of a solid block should succeed");

        // Attachment painted on the Top face.
        let att = world.face_attachment_at(pos, crate::mesh::Face::Top.index());
        assert!(
            matches!(att, Some(FaceAttachment::Blueprint(_))),
            "expected a Blueprint attachment on the top face, got {att:?}"
        );

        // No cube placed in the air cell above — this is the height-bug fix.
        assert_eq!(
            world.get_block(pos.0, pos.1 + 1, pos.2),
            block::AIR,
            "cell above the floor must stay AIR (zero-height attachment)"
        );
    }

    #[test]
    fn non_top_face_is_a_noop() {
        let (mut world, registry, pos) = solid_floor();
        let laid = lay_blueprint_on_floor(
            &mut world,
            &registry,
            pos,
            [1, 0, 0],
            PlanData::debug_3x3_stone(),
        );
        assert!(!laid, "laying on a non-Top face should be a no-op");
        assert!(
            world
                .face_attachment_at(pos, crate::mesh::Face::Top.index())
                .is_none(),
            "no attachment should be set on a non-Top lay"
        );
    }

    #[test]
    fn non_solid_target_is_a_noop() {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        let pos = (3, 64, 5); // left as AIR
        let laid = lay_blueprint_on_floor(
            &mut world,
            &registry,
            pos,
            [0, 1, 0],
            PlanData::debug_3x3_stone(),
        );
        assert!(!laid, "laying on a non-solid (AIR) target should be a no-op");
        assert!(
            world
                .face_attachment_at(pos, crate::mesh::Face::Top.index())
                .is_none(),
            "no attachment should be set on a non-solid lay"
        );
    }

    #[test]
    fn blank_lays_on_top_face_of_solid_block_no_cube_above() {
        let (mut world, registry, pos) = solid_floor();
        let laid = lay_blank_blueprint_paper(&mut world, &registry, pos, [0, 1, 0]);
        assert!(laid, "blank paper should lay on a Top face of a solid block");

        let att = world.face_attachment_at(pos, crate::mesh::Face::Top.index());
        assert!(
            matches!(att, Some(FaceAttachment::BlueprintBlank)),
            "expected a BlueprintBlank attachment on the top face, got {att:?}"
        );

        // Zero-height — the cell above stays AIR (no BLUEPRINT_PAPER cube).
        assert_eq!(
            world.get_block(pos.0, pos.1 + 1, pos.2),
            block::AIR,
            "cell above the floor must stay AIR (zero-height attachment)"
        );
    }

    #[test]
    fn blank_on_non_top_face_is_a_noop() {
        let (mut world, registry, pos) = solid_floor();
        let laid = lay_blank_blueprint_paper(&mut world, &registry, pos, [1, 0, 0]);
        assert!(!laid, "blank paper on a non-Top face should be a no-op");
        assert!(
            world
                .face_attachment_at(pos, crate::mesh::Face::Top.index())
                .is_none(),
            "no attachment should be set on a non-Top blank lay"
        );
    }

    #[test]
    fn blank_on_non_solid_target_is_a_noop() {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        let pos = (3, 64, 5); // left as AIR
        let laid = lay_blank_blueprint_paper(&mut world, &registry, pos, [0, 1, 0]);
        assert!(!laid, "blank paper on a non-solid (AIR) target should be a no-op");
        assert!(
            world
                .face_attachment_at(pos, crate::mesh::Face::Top.index())
                .is_none(),
            "no attachment should be set on a non-solid blank lay"
        );
    }

    // ── paper_tile_under tests ────────────────────────────────────────────────

    /// Helper: set a stone floor block with a BlueprintBlank on its Top face.
    fn place_paper(world: &mut World, pos: (i32, i32, i32)) {
        world.set_block(pos.0, pos.1, pos.2, block::STONE);
        world.set_face_attachment(pos, crate::mesh::Face::Top.index(), FaceAttachment::BlueprintBlank);
    }

    #[test]
    fn paper_tile_under_scan_down_hit() {
        // Paper at y=64; solid build block at y=66.
        let mut world = World::new();
        place_paper(&mut world, (0, 64, 0));
        world.set_block(0, 66, 0, block::STONE); // build block sitting above the paper
        assert_eq!(
            super::paper_tile_under(&world, (0, 66, 0)),
            Some((0, 64, 0)),
            "should resolve paper two blocks below the clicked build block"
        );
    }

    #[test]
    fn paper_tile_under_miss_no_paper_in_column() {
        // No paper anywhere in this column.
        let world = World::new();
        assert_eq!(
            super::paper_tile_under(&world, (5, 66, 5)),
            None,
            "should return None when no papered tile exists in the column"
        );
    }

    #[test]
    fn paper_tile_under_clicked_block_is_the_paper() {
        // Stamp directly on the paper itself.
        let mut world = World::new();
        place_paper(&mut world, (2, 70, 2));
        assert_eq!(
            super::paper_tile_under(&world, (2, 70, 2)),
            Some((2, 70, 2)),
            "clicking the papered tile directly should return that tile"
        );
    }

    #[test]
    fn paper_tile_under_out_of_range() {
        // Paper is deeper than MAX_HEIGHT below the click → should not be found.
        let mut world = World::new();
        let paper_y = 10_i32;
        let click_y = paper_y + crate::plan::MAX_HEIGHT + 1;
        place_paper(&mut world, (0, paper_y, 0));
        assert_eq!(
            super::paper_tile_under(&world, (0, click_y, 0)),
            None,
            "paper deeper than MAX_HEIGHT below the click should not be found"
        );
    }

    #[test]
    fn paper_tile_under_highest_of_two() {
        // Two papered tiles in the same column; scanning down from click should
        // return the HIGHER one (i.e. first encountered scanning downward).
        let mut world = World::new();
        place_paper(&mut world, (0, 64, 0));
        place_paper(&mut world, (0, 68, 0));
        // Click at y=70, well above both papers.
        assert_eq!(
            super::paper_tile_under(&world, (0, 70, 0)),
            Some((0, 68, 0)),
            "should return the highest papered tile (first found scanning down)"
        );
    }

    #[test]
    fn recovered_item_covers_all_three_variants() {
        // Wallpaper recovers as its own block.
        assert!(matches!(
            recovered_item_for(&FaceAttachment::Wallpaper(block::WALLPAPER_RED)),
            crate::item::Item::Block(b) if b == block::WALLPAPER_RED
        ));
        // Blank cream paper recovers as a BLUEPRINT_PAPER sheet.
        assert!(matches!(
            recovered_item_for(&FaceAttachment::BlueprintBlank),
            crate::item::Item::Block(b) if b == block::BLUEPRINT_PAPER
        ));
        // A captured plan recovers as a Plan item carrying the same plan.
        let plan = PlanData::debug_3x3_stone();
        let recovered = recovered_item_for(&FaceAttachment::Blueprint(Box::new(plan.clone())));
        assert!(matches!(recovered, crate::item::Item::Plan(_)));
        if let crate::item::Item::Plan(p) = recovered {
            assert_eq!(p.name, plan.name, "recovered plan should carry the same data");
        }
    }

    // ── C3c-3r ──

    fn attachments_on(world: &mut World, pos: (i32, i32, i32)) {
        world.set_face_attachment(pos, 0, FaceAttachment::Blueprint(Box::new(PlanData::debug_3x3_stone())));
        world.set_face_attachment(pos, 2, FaceAttachment::Wallpaper(block::OAK_PLANKS));
        world.set_face_attachment(pos, 4, FaceAttachment::BlueprintBlank);
    }

    #[test]
    fn a_seat_that_owns_its_world_recovers_every_attachment() {
        let (mut world, _r, pos) = solid_floor();
        attachments_on(&mut world, pos);
        let items = take_recoverable_attachments(&mut world, pos, false);
        assert_eq!(items.len(), 3);
        assert!(items.iter().any(|i| matches!(i, crate::item::Item::Plan(_))));
        assert!(world.remove_face_attachments_at(pos).iter().all(Option::is_none), "all removed");
    }

    #[test]
    fn a_joiner_recovers_wallpaper_and_paper_but_leaves_a_blueprint_standing() {
        let (mut world, _r, pos) = solid_floor();
        attachments_on(&mut world, pos);
        let items = take_recoverable_attachments(&mut world, pos, true);
        assert_eq!(items.len(), 2, "wallpaper and blank paper only: {items:?}");
        assert!(!items.iter().any(|i| matches!(i, crate::item::Item::Plan(_))), "no Plan granted");
        assert!(matches!(world.face_attachment_at(pos, 0), Some(FaceAttachment::Blueprint(_))), "left in place");
        assert!(world.face_attachment_at(pos, 2).is_none() && world.face_attachment_at(pos, 4).is_none());
    }
}
