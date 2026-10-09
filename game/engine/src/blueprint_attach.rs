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
/// shared rule allows it ([`attach_rule`]: the target face is Top, the target
/// block is solid and that face is bare); false (no-op) otherwise.
/// Zero-height — no block placed, no BlockChange. (Capture comes later.)
pub fn lay_blank_blueprint_paper(
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    target_block: (i32, i32, i32),
    target_face: [i32; 3],
) -> bool {
    let Some(face) = crate::mesh::Face::from_normal(target_face) else { return false };
    attach(world, registry, target_block, face.index(), crate::block::BLUEPRINT_PAPER).is_ok()
}

/// C3c-3b — why a block can't go on a face as an attachment ([`attach_rule`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachRefusal {
    /// The held block is neither a wallpaper nor Blueprint Paper.
    NotAttachable,
    /// The target block isn't solid (never AIR or water).
    NotSolid,
    /// That face already carries an attachment.
    FaceCovered,
    /// Blank paper lies flat on floors only: the face isn't Top.
    NotAFloor,
}

impl AttachRefusal {
    /// The note a joiner's refused `Attach` carries.
    pub fn note(self) -> crate::item_actions::ItemNote {
        use crate::item_actions::ItemNote;
        match self {
            AttachRefusal::NotAttachable => ItemNote::NothingToTake,
            AttachRefusal::NotSolid => ItemNote::NotThatBlock,
            AttachRefusal::FaceCovered => ItemNote::FaceCovered,
            AttachRefusal::NotAFloor => ItemNote::NotAFloor,
        }
    }
}

/// C3c-3b — the one rule for painting wallpaper and laying blank paper, run
/// by every seat (single-player's and a host's arms, and the server for a
/// joiner's `ItemAction::Attach`): what the block `held` puts on face
/// `face_idx` (`mesh::Face::index`) of the block at `pos` — a wallpaper
/// block its `Wallpaper`, Blueprint Paper a `BlueprintBlank` — if the target
/// is solid, the face is bare, and (blank paper) the face is Top. Reach,
/// plots and the play mode are the caller's.
pub fn attach_rule(
    world: &World,
    registry: &crate::block::BlockRegistry,
    pos: (i32, i32, i32),
    face_idx: usize,
    held: crate::block::BlockId,
) -> Result<FaceAttachment, AttachRefusal> {
    let att = if crate::block::is_wallpaper(held) {
        FaceAttachment::Wallpaper(held)
    } else if held == crate::block::BLUEPRINT_PAPER {
        FaceAttachment::BlueprintBlank
    } else {
        return Err(AttachRefusal::NotAttachable);
    };
    if face_idx >= 6 || (att == FaceAttachment::BlueprintBlank && face_idx != crate::mesh::Face::Top.index()) {
        return Err(AttachRefusal::NotAFloor);
    }
    if !registry.is_solid(world.get_block(pos.0, pos.1, pos.2)) {
        return Err(AttachRefusal::NotSolid);
    }
    if world.face_attachment_at(pos, face_idx).is_some() {
        return Err(AttachRefusal::FaceCovered);
    }
    Ok(att)
}

/// C3c-3b — [`attach_rule`], and on success the attachment is set (logged
/// for the server's stream like every setter). The caller takes the item.
pub fn attach(
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    pos: (i32, i32, i32),
    face_idx: usize,
    held: crate::block::BlockId,
) -> Result<(), AttachRefusal> {
    let att = attach_rule(world, registry, pos, face_idx, held)?;
    world.set_face_attachment(pos, face_idx, att);
    Ok(())
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

/// C3c-3b — the one rule for what a broken block's attachments give back:
/// take every face attachment off the block at `pos` (each removal logged for
/// the server's stream) and return what each recovers to
/// (`recovered_item_for`), in face order. Single-player and a host's break
/// arms put them in the breaker's inventory (overflow dropped at the block);
/// the server, for a joiner's break, spills them as ground items at the cell
/// (`HostedServer::spill_broken_attachments`) — a laid Blueprint as a ground
/// Plan with the server's real body. A joined client calls neither: its copy
/// loses them when the server's stream says so, and nothing is granted twice.
pub fn take_recoverable_attachments(world: &mut World, pos: (i32, i32, i32)) -> Vec<crate::item::Item> {
    world.remove_face_attachments_at(pos).iter().flatten().map(recovered_item_for).collect()
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

    // ── C3c-3r / C3c-3b ──

    fn attachments_on(world: &mut World, pos: (i32, i32, i32)) {
        world.set_face_attachment(pos, 0, FaceAttachment::Blueprint(Box::new(PlanData::debug_3x3_stone())));
        world.set_face_attachment(pos, 2, FaceAttachment::Wallpaper(block::OAK_PLANKS));
        world.set_face_attachment(pos, 4, FaceAttachment::BlueprintBlank);
    }

    /// C3c-3b — a break recovers every attachment once, in face order, and
    /// each removal is logged for the server's stream.
    #[test]
    fn a_break_recovers_every_attachment_once_and_logs_each() {
        let (mut world, _r, pos) = solid_floor();
        attachments_on(&mut world, pos);
        world.track_attachment_changes();
        let items = take_recoverable_attachments(&mut world, pos);
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], crate::item::Item::Plan(p) if p.name == PlanData::debug_3x3_stone().name), "the real Plan");
        assert_eq!(items[1], crate::item::Item::Block(block::OAK_PLANKS));
        assert_eq!(items[2], crate::item::Item::Block(block::BLUEPRINT_PAPER));
        assert_eq!(world.take_attachment_changes(), vec![(pos, 0), (pos, 2), (pos, 4)]);
        assert!(take_recoverable_attachments(&mut world, pos).is_empty(), "nothing a second time");
    }

    /// C3c-3b — the shared attach rule: a wallpaper on any bare face of a
    /// solid block, blank paper on a bare Top face only; anything else held
    /// attaches nothing.
    #[test]
    fn the_attach_rule_wants_a_solid_block_a_bare_face_and_paper_on_top() {
        let (mut world, registry, pos) = solid_floor();
        let top = crate::mesh::Face::Top.index();
        let north = crate::mesh::Face::North.index();
        assert_eq!(attach_rule(&world, &registry, pos, north, block::WALLPAPER_RED), Ok(FaceAttachment::Wallpaper(block::WALLPAPER_RED)));
        assert_eq!(attach_rule(&world, &registry, pos, top, block::BLUEPRINT_PAPER), Ok(FaceAttachment::BlueprintBlank));
        assert_eq!(attach_rule(&world, &registry, pos, north, block::BLUEPRINT_PAPER), Err(AttachRefusal::NotAFloor));
        assert_eq!(attach_rule(&world, &registry, pos, north, block::STONE), Err(AttachRefusal::NotAttachable));
        assert_eq!(attach_rule(&world, &registry, (pos.0, pos.1 + 1, pos.2), north, block::WALLPAPER_RED), Err(AttachRefusal::NotSolid));
        assert_eq!(attach(&mut world, &registry, pos, top, block::WALLPAPER_RED), Ok(()));
        assert_eq!(attach_rule(&world, &registry, pos, top, block::BLUEPRINT_PAPER), Err(AttachRefusal::FaceCovered));
        assert_eq!(attach_rule(&world, &registry, pos, top, block::WALLPAPER_BLUE), Err(AttachRefusal::FaceCovered));
        assert!(!lay_blank_blueprint_paper(&mut world, &registry, pos, [0, 1, 0]), "blank paper never covers a wallpaper");
        assert_eq!(world.face_attachment_at(pos, top), Some(&FaceAttachment::Wallpaper(block::WALLPAPER_RED)));
    }
}
