//! pt1124 — attached blocks drop when what holds them goes (Spec 02 §1
//! "Support", Spec 05 §3).
//!
//! * **The table.** `BlockRegistry::attachment` names what each attached block
//!   needs: a torch the solid block it was placed against ([`torch_meta`]
//!   records that face; a torch placed before it records none and keeps the old
//!   rule, any solid neighbour), a standing lantern the cell below, a hanging
//!   lantern the cell above (a solid block or another lantern, so a lantern can
//!   hang under a lantern). Every other block stands on its own.
//! * **The queue.** Every block change outside world-gen passes
//!   `World::set_block`, which logs the cell (`World::take_support_cells`). A
//!   player's break, a blast, a piston, a falling block, fire, a fluid and a
//!   joiner's edit on the server all land there alike, so there is no
//!   per-cause hook to forget.
//! * **The pass.** [`tick_support`] runs beside the falling blocks (5 Hz) on
//!   whichever side owns the world: a standalone client, a lending host's
//!   server, a dedicated server. A joined client discards its log: its server
//!   drops the blocks and sends the change and the item. It checks each logged
//!   cell and its six neighbours, decides every drop against the world as it
//!   stood at the start of the pass, then clears them: the cell goes to AIR
//!   (no meta, not player-placed), its light is taken out, and the block's own
//!   mining drop comes back to the caller to spawn. A cleared cell is logged in
//!   turn, so a chain (a lantern under a lantern, a torch on a block that
//!   itself dropped) falls one link per pass.
//!   A bulk edit past the log's bound marks whole sections instead, which a
//!   later pass rescans ([`MAX_SECTIONS_PER_PASS`] a pass), so no change is
//!   lost.
//! * **Saving and loading** (review 3 items 6 and 7). The log is runtime-only
//!   and a save neither keeps nor settles it: Save and Quit leaves a chain's
//!   hanging links where they hang. A load instead queues every player-placed
//!   torch and lantern in a section it brings in from a save (or back from the
//!   evicted store) for one check (`World::note_loaded_for_support`), so a
//!   collapse a save cut resumes after the load, each link falling once as one
//!   item. World-gen torches carry no placed bit and are never queued (some
//!   bundled village plans hang one in mid-air), a generated, never-saved
//!   section is never loaded, and a pushed chunk (a joined client's) is not the
//!   authority's world.
//! * **Old worlds.** So a player's torch already floating in a saved world
//!   falls at the next load; a world-gen one stays until something next to it
//!   changes.

use crate::block::{self, Attachment, BlockId, BlockRegistry};
use crate::item::ItemStack;
use crate::world::World;

/// Most logged cells one pass checks; the rest wait for the next pass.
pub const MAX_CELLS_PER_PASS: usize = 2048;

/// Most overflowed sections (`World::take_support_sections`) one pass
/// rescans, once the cell log has been drained below a full pass.
pub const MAX_SECTIONS_PER_PASS: usize = 4;

/// Most passes [`settle`] runs: a chain falls one link a pass, and no chain is
/// taller than the world.
#[cfg(test)]
pub const MAX_SETTLE_PASSES: usize = 1024;

/// The state and aux bits that mark a torch's meta as recording its support
/// face (the facing bits): a combination no other block's stale byte is likely
/// to leave in a cell, and 0 (every torch placed before) is "unknown".
const TORCH_ORIENTED_STATE: u8 = 0b11;
const TORCH_ORIENTED_AUX: u8 = 0b101;

/// pt1124 review 11 — the meta a torch placed against the face `face` (the
/// clicked face's outward normal: the torch is at the clicked block plus
/// `face`, so its support is at minus `face`) stamps. `None` for a face that is
/// not one of the six.
pub fn torch_meta(face: [i32; 3]) -> Option<u8> {
    let toward = [-face[0], -face[1], -face[2]];
    if toward.iter().map(|c| c.abs()).sum::<i32>() != 1 {
        return None;
    }
    let f = crate::block_shape::facing_from_face_normal(toward);
    let m = crate::meta::with_facing(0, f);
    Some(crate::meta::with_aux(crate::meta::with_state(m, TORCH_ORIENTED_STATE), TORCH_ORIENTED_AUX))
}

/// The direction of the block a torch with meta `meta` was placed against, or
/// `None` for a torch that recorded none (placed before review 11, by a
/// structure or a `/we` write): those keep the any-neighbour rule.
pub fn torch_support_dir(meta: u8) -> Option<(i32, i32, i32)> {
    (crate::meta::state(meta) == TORCH_ORIENTED_STATE && crate::meta::aux(meta) == TORCH_ORIENTED_AUX)
        .then(|| crate::meta::facing(meta).offset())
}

const SIDES: [(i32, i32, i32); 6] = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

/// One attached block a pass dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct Dropped {
    pub cell: (i32, i32, i32),
    /// The block that was there.
    pub block: BlockId,
    /// Its mining drop (a torch, a lantern), for the caller to spawn at the cell.
    pub item: ItemStack,
}

impl Dropped {
    /// The change that tells a client the cell is empty now.
    pub fn air_change(&self) -> crate::protocol::BlockChange {
        let (x, y, z) = self.cell;
        crate::protocol::BlockChange { x, y, z, new_block: block::AIR, meta: 0 }
    }
}

fn add(c: (i32, i32, i32), d: (i32, i32, i32)) -> (i32, i32, i32) {
    (c.0 + d.0, c.1 + d.1, c.2 + d.2)
}

/// Whether the neighbour at `n` counts as holding something. A cell in a
/// column this world holds nothing of (not loaded here) holds: what is there
/// is unknown, and a missing column must never knock a torch off.
fn holds(world: &World, registry: &BlockRegistry, n: (i32, i32, i32), hanging: bool) -> bool {
    if world.column_top_y(n.0, n.2) < 0 {
        return true;
    }
    let b = world.get_block(n.0, n.1, n.2);
    registry.is_solid(b) || (hanging && b == block::LANTERN)
}

/// Whether the block at `cell` (block `id`, meta `meta`) has what it needs.
/// `true` for any block with no attachment.
pub fn is_supported(world: &World, registry: &BlockRegistry, cell: (i32, i32, i32), id: BlockId, meta: u8) -> bool {
    match registry.attachment(id, meta) {
        None => true,
        Some(Attachment::AnySide) => SIDES.iter().any(|&d| holds(world, registry, add(cell, d), false)),
        Some(Attachment::Toward(d)) => holds(world, registry, add(cell, d), false),
        Some(Attachment::Below) => holds(world, registry, add(cell, (0, -1, 0)), false),
        Some(Attachment::Above) => holds(world, registry, add(cell, (0, 1, 0)), true),
    }
}

/// One support pass: check up to [`MAX_CELLS_PER_PASS`] logged cells and their
/// neighbours, clear every attached block that lost its support, and return
/// what dropped (cell order). The caller spawns each `item` at its cell and
/// tells whoever draws the world (a broadcast `BlockChange` to AIR, a remesh).
pub fn tick_support(world: &mut World, registry: &BlockRegistry) -> Vec<Dropped> {
    let logged = world.take_support_cells(MAX_CELLS_PER_PASS);
    // Review 9 — the sections a bulk edit overflowed into, once the cell log
    // has been drained below a full pass.
    let sections = if logged.len() < MAX_CELLS_PER_PASS {
        world.take_support_sections(MAX_SECTIONS_PER_PASS)
    } else {
        Vec::new()
    };
    // Review 3 items 6 and 7 — the placed torches and lanterns a load brought
    // in, in the room the cell log left: each checked itself (not its
    // neighbours, so a world-gen torch beside one is not judged by this).
    let rechecks = world.take_support_rechecks(MAX_CELLS_PER_PASS.saturating_sub(logged.len()));
    if logged.is_empty() && sections.is_empty() && rechecks.is_empty() {
        return Vec::new();
    }
    let mut candidates: std::collections::BTreeSet<(i32, i32, i32)> = rechecks.into_iter().collect();
    for &cell in &logged {
        candidates.extend(std::iter::once(cell).chain(SIDES.iter().map(|&d| add(cell, d))));
    }
    // A section's own cells and the ring round it (an attached block just
    // outside it may have lost a support inside it).
    let cs = crate::chunk::CHUNK_SIZE as i32;
    for &(sx, sy, sz) in &sections {
        for x in sx * cs - 1..=sx * cs + cs {
            for y in sy * cs - 1..=sy * cs + cs {
                for z in sz * cs - 1..=sz * cs + cs {
                    if registry.attachment(world.get_block(x, y, z), 0).is_some() {
                        candidates.insert((x, y, z));
                    }
                }
            }
        }
    }
    // Decide every drop against the world as it stands now, then clear them.
    let mut doomed: std::collections::BTreeSet<(i32, i32, i32)> = std::collections::BTreeSet::new();
    for c in candidates {
        let id = world.get_block(c.0, c.1, c.2);
        if registry.attachment(id, 0).is_none() {
            continue;
        }
        let meta = world.meta_at(c.0, c.1, c.2);
        if !is_supported(world, registry, c, id, meta) {
            doomed.insert(c);
        }
    }
    let mut dropped = Vec::with_capacity(doomed.len());
    for cell in doomed {
        let (x, y, z) = cell;
        let id = world.get_block(x, y, z);
        world.set_block(x, y, z, block::AIR);
        world.set_meta(cell, 0);
        world.set_placed(x, y, z, false);
        crate::lighting::update_for_block_change(world, cell, id, block::AIR, registry);
        dropped.push(Dropped { cell, block: id, item: registry.mine_drop(id) });
    }
    dropped
}

/// Run support passes until nothing waits (at most [`MAX_SETTLE_PASSES`]) and
/// return everything that dropped, in pass order. Tests only: the game runs
/// one pass a support tick and never settles at a save (review 3 items 6 and 7;
/// a save between two passes keeps the hanging links, and the load resumes them).
#[cfg(test)]
pub fn settle(world: &mut World, registry: &BlockRegistry) -> Vec<Dropped> {
    let mut all = Vec::new();
    for _ in 0..MAX_SETTLE_PASSES {
        if !world.support_pending() {
            break;
        }
        all.extend(tick_support(world, registry));
    }
    all
}

/// Spawn each dropped block's item at its cell, as a break's drop lands.
pub fn spawn_drops(ecs: &mut hecs::World, dropped: &[Dropped]) {
    for d in dropped {
        if d.item.count == 0 {
            continue;
        }
        let (x, y, z) = d.cell;
        crate::entity::spawn_item(
            ecs,
            glam::Vec3::new(x as f32 + 0.5, y as f32 + 0.25, z as f32 + 0.5),
            d.item.clone(),
            (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ (z as u32).wrapping_mul(83_492_791),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockRegistry, AIR, LANTERN, STONE, TORCH};

    const HIGH: i32 = 100;

    /// A world with a stone floor round (0, HIGH, 0), so every column near the
    /// cells under test is held (`holds` treats an empty column as unknown).
    fn world() -> World {
        let mut w = World::new();
        for x in -4..=4 {
            for z in -4..=4 {
                w.set_block(x, HIGH - 10, z, STONE);
            }
        }
        w.discard_support_log();
        w
    }

    fn hanging() -> u8 {
        crate::lantern::placement_meta([0, -1, 0])
    }

    #[test]
    fn a_torch_holds_on_any_solid_side_and_drops_with_none() {
        let reg = BlockRegistry::new();
        for d in SIDES {
            let mut w = world();
            let wall = add((0, HIGH, 0), d);
            w.set_block(wall.0, wall.1, wall.2, STONE);
            w.set_block(0, HIGH, 0, TORCH);
            assert!(is_supported(&w, &reg, (0, HIGH, 0), TORCH, 0), "a torch holds on {d:?}");
            assert!(tick_support(&mut w, &reg).is_empty(), "nothing drops while {d:?} holds");
            w.set_block(wall.0, wall.1, wall.2, AIR);
            let dropped = tick_support(&mut w, &reg);
            assert_eq!(dropped, vec![Dropped { cell: (0, HIGH, 0), block: TORCH, item: ItemStack::new_block(TORCH, 1) }]);
            assert_eq!(w.get_block(0, HIGH, 0), AIR);
        }
    }

    #[test]
    fn a_standing_lantern_needs_the_block_below_and_a_hanging_one_the_block_above() {
        let reg = BlockRegistry::new();
        // Standing: a wall beside it is no help.
        let mut w = world();
        w.set_block(0, HIGH - 1, 0, STONE);
        w.set_block(1, HIGH, 0, STONE);
        w.set_block(0, HIGH, 0, LANTERN);
        assert!(tick_support(&mut w, &reg).is_empty());
        w.set_block(0, HIGH - 1, 0, AIR);
        let dropped = tick_support(&mut w, &reg);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].item, ItemStack::new_block(LANTERN, 1));
        assert_eq!(w.get_block(0, HIGH, 0), AIR);
        // Hanging: the floor below is no help.
        let mut w = world();
        w.set_block(0, HIGH + 1, 0, STONE);
        w.set_block(0, HIGH - 1, 0, STONE);
        w.set_block(0, HIGH, 0, LANTERN);
        w.set_meta((0, HIGH, 0), hanging());
        assert!(tick_support(&mut w, &reg).is_empty());
        w.set_block(0, HIGH + 1, 0, AIR);
        assert_eq!(tick_support(&mut w, &reg).len(), 1, "the hanging lantern drops");
        assert_eq!(w.get_block(0, HIGH, 0), AIR);
        assert_eq!(w.meta_at(0, HIGH, 0), 0, "the cell keeps no hanging bit");
    }

    #[test]
    fn a_chain_of_hanging_lanterns_falls_one_link_per_pass() {
        let reg = BlockRegistry::new();
        let mut w = world();
        w.set_block(0, HIGH + 1, 0, STONE);
        for y in [HIGH, HIGH - 1] {
            w.set_block(0, y, 0, LANTERN);
            w.set_meta((0, y, 0), hanging());
        }
        assert!(tick_support(&mut w, &reg).is_empty(), "a lantern holds the lantern under it");
        w.set_block(0, HIGH + 1, 0, AIR);
        let first = tick_support(&mut w, &reg);
        assert_eq!(first.iter().map(|d| d.cell).collect::<Vec<_>>(), vec![(0, HIGH, 0)]);
        assert_eq!(w.get_block(0, HIGH - 1, 0), LANTERN, "decided against the world at the start of the pass");
        let second = tick_support(&mut w, &reg);
        assert_eq!(second.iter().map(|d| d.cell).collect::<Vec<_>>(), vec![(0, HIGH - 1, 0)]);
        assert!(tick_support(&mut w, &reg).is_empty());
    }

    #[test]
    fn a_torch_on_sand_drops_when_the_sand_falls() {
        let reg = BlockRegistry::new();
        let mut w = world();
        let mut water = crate::water::WaterSystem::new();
        w.set_block(0, HIGH, 0, block::SAND);
        w.set_block(0, HIGH + 1, 0, TORCH);
        w.discard_support_log();
        let falls = crate::falling_blocks::tick_falling_blocks(&mut w, &reg, &mut water, &[glam::Vec3::new(0.0, HIGH as f32, 0.0)], 15);
        assert!(!falls.is_empty(), "the sand falls");
        let dropped = tick_support(&mut w, &reg);
        assert_eq!(dropped.iter().map(|d| d.cell).collect::<Vec<_>>(), vec![(0, HIGH + 1, 0)]);
    }

    #[test]
    fn the_light_goes_with_the_torch_and_a_missing_column_holds() {
        let reg = BlockRegistry::new();
        let mut w = world();
        w.set_block(0, HIGH - 1, 0, STONE);
        w.set_block(0, HIGH, 0, TORCH);
        crate::lighting::update_for_block_change(&mut w, (0, HIGH, 0), AIR, TORCH, &reg);
        assert!(w.block_light_at(0, HIGH + 1, 0) > 0, "the torch lights the cell above it");
        w.set_block(0, HIGH - 1, 0, AIR);
        assert_eq!(tick_support(&mut w, &reg).len(), 1);
        assert_eq!(w.block_light_at(0, HIGH + 1, 0), 0, "no light where the torch was");
        // A torch whose only neighbour column is not held here keeps its place.
        let mut w = World::new();
        w.set_block(15, HIGH - 10, 15, STONE); // the torch's own column is held
        w.set_block(15, HIGH, 15, TORCH);
        assert!(is_supported(&w, &reg, (15, HIGH, 15), TORCH, 0), "the column at x 16 is unknown, so it holds");
    }

    /// pt1124 review 11 — a torch placed on a floor drops when the floor goes, even with
    /// a wall beside it; a torch from before (meta 0, no face) keeps the any-neighbour
    /// rule; a wall torch holds while only its floor goes.
    #[test]
    fn a_torch_needs_the_face_it_was_placed_against() {
        let reg = BlockRegistry::new();
        let floor_torch = torch_meta([0, 1, 0]).unwrap();
        let wall_torch = torch_meta([-1, 0, 0]).unwrap(); // clicked the west face of the wall at +x
        for (meta, drops, what) in [(floor_torch, true, "a floor torch"), (0, false, "a torch from before"), (wall_torch, false, "a wall torch")] {
            let mut w = world();
            w.set_block(0, HIGH - 1, 0, STONE);
            w.set_block(1, HIGH, 0, STONE);
            w.set_block(0, HIGH, 0, TORCH);
            w.set_meta((0, HIGH, 0), meta);
            assert!(tick_support(&mut w, &reg).is_empty(), "{what} holds");
            w.set_block(0, HIGH - 1, 0, AIR);
            let dropped = tick_support(&mut w, &reg);
            assert_eq!(!dropped.is_empty(), drops, "{what}: floor gone");
            assert_eq!(w.get_block(0, HIGH, 0) == TORCH, !drops, "{what}");
        }
        // The wall torch goes with its wall.
        let mut w = world();
        w.set_block(1, HIGH, 0, STONE);
        w.set_block(0, HIGH - 1, 0, STONE);
        w.set_block(0, HIGH, 0, TORCH);
        w.set_meta((0, HIGH, 0), wall_torch);
        w.discard_support_log();
        w.set_block(1, HIGH, 0, AIR);
        assert_eq!(tick_support(&mut w, &reg).len(), 1, "a wall torch drops with its wall, floor or not");
        assert_eq!(torch_meta([0, 0, 0]), None);
    }

    /// pt1124 review 9 — an edit past the log's bound is not lost: its section is
    /// rescanned by a later pass, so a lantern whose support went in the overflow drops.
    #[test]
    fn a_support_lost_past_the_log_bound_is_found_by_a_section_rescan() {
        let reg = BlockRegistry::new();
        let mut w = world();
        w.set_block(0, HIGH - 1, 0, STONE);
        w.set_block(0, HIGH, 0, LANTERN);
        w.discard_support_log();
        for i in 0..crate::world::MAX_SUPPORT_LOG as i32 {
            w.set_block(i % 64 - 32, HIGH + 20 + i / 4096, (i / 64) % 64 - 32, STONE);
        }
        assert_eq!(w.support_log_len(), crate::world::MAX_SUPPORT_LOG);
        w.set_block(0, HIGH - 1, 0, AIR); // over the bound: not a logged cell
        assert_eq!(w.support_sections_len(), 1, "its section is marked instead");
        let dropped = settle(&mut w, &reg);
        assert_eq!(dropped.iter().map(|d| d.cell).collect::<Vec<_>>(), vec![(0, HIGH, 0)], "the lantern drops, once");
        assert!(!w.support_pending());
        assert_eq!(w.get_block(0, HIGH, 0), AIR);
    }

    #[test]
    fn untouched_blocks_are_left_alone_and_the_log_is_bounded() {
        let reg = BlockRegistry::new();
        let mut w = world();
        // A floating torch already in the world (placed with no neighbour change
        // logged since) stays: the pass only looks where something changed.
        w.set_block(0, HIGH, 0, TORCH);
        w.discard_support_log();
        assert!(tick_support(&mut w, &reg).is_empty());
        assert_eq!(w.get_block(0, HIGH, 0), TORCH);
        for i in 0..(crate::world::MAX_SUPPORT_LOG as i32 + 10) {
            w.set_block(i % 64 - 32, HIGH + 20 + i / 4096, (i / 64) % 64 - 32, STONE);
        }
        assert_eq!(w.support_log_len(), crate::world::MAX_SUPPORT_LOG);
        let _ = tick_support(&mut w, &reg);
        assert_eq!(w.support_log_len(), crate::world::MAX_SUPPORT_LOG - MAX_CELLS_PER_PASS);
    }
}
