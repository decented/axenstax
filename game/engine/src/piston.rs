//! Pistons (P11) — Electricity-powered block pushers.
//!
//! A `PISTON` faces a direction stored in its block meta (0..5). When the
//! Electricity grid powers it, it **extends**: it places a `PISTON_HEAD` in the
//! cell directly ahead and shoves the contiguous column of *pushable* blocks in
//! that direction by one, into empty space. When power drops it **retracts** (a
//! plain piston does NOT pull the pushed block back — sticky pull is a follow-up).
//!
//! Safety: only plain full-cube building/terrain blocks are pushable (a strict
//! allow-list) — never block-entities, fluids, bedrock, Satori, or pistons — so a
//! push can never move or orphan block-entity data. v1 is a plain piston with no
//! arm-shape mesh and no animation (instant move).

use crate::block::{self, BlockId};
use crate::meta::Facing;
use crate::power::Pos;
use crate::world::World;

/// Max blocks a piston shoves in one push (Minecraft parity).
pub const MAX_PUSH: usize = 12;

/// Unit step in the piston's push direction. Reads the canonical facing field
/// (low 3 bits) so the encoding is shared with every other directional block.
pub fn facing_delta(meta: u8) -> (i32, i32, i32) {
    crate::meta::facing(meta).offset()
}

/// Pick the piston's facing from the placer's look direction (pitch chooses
/// up/down). The result is written into the block's facing field on place.
pub fn facing_from_look(fx: f32, fy: f32, fz: f32) -> Facing {
    if fy.abs() > fx.abs() && fy.abs() > fz.abs() {
        if fy > 0.0 { Facing::Up } else { Facing::Down }
    } else if fx.abs() > fz.abs() {
        if fx > 0.0 { Facing::East } else { Facing::West }
    } else if fz > 0.0 {
        Facing::South
    } else {
        Facing::North
    }
}

/// Whether a block can be SHOVED by a piston. Strict allow-list of plain
/// full-cube building/terrain blocks — deliberately excludes block-entities,
/// fluids, bedrock, pistons, crops, etc. — so a push can never corrupt anything.
pub fn is_pushable(id: BlockId) -> bool {
    use block::*;
    matches!(
        id,
        STONE | COBBLESTONE | DIRT | GRASS | SAND | GRAVEL | SANDSTONE
            | LIMESTONE | MARBLE | GRANITE | SLATE
            | COAL_BLOCK | IRON_BLOCK | DIAMOND_BLOCK
            | BONE_BLOCK | HAY_BALE | AMETHYST_BLOCK | GLASS
            | OAK_LOG | BIRCH_LOG | SPRUCE_LOG | JUNGLE_LOG
            | ACACIA_LOG | DARK_OAK_LOG | RUBBER_LOG
            | OAK_PLANKS | BIRCH_PLANKS | SPRUCE_PLANKS | JUNGLE_PLANKS
            | ACACIA_PLANKS | DARK_OAK_PLANKS | RUBBER_PLANKS
            | OAK_LEAVES | BIRCH_LEAVES | SPRUCE_LEAVES | JUNGLE_LEAVES
            | ACACIA_LEAVES | DARK_OAK_LEAVES | RUBBER_LEAVES
    )
}

/// The contiguous column of pushable blocks starting at `front` (stepping by
/// `delta`), if the piston can extend: `Some(column)` when the cell after the
/// column is AIR and the column is ≤ [`MAX_PUSH`]; `None` if blocked by a
/// non-pushable block or the column is too long. An empty column (front is AIR)
/// returns `Some(vec![])` — the piston just places its head into the gap.
pub fn pushable_column(world: &World, front: Pos, delta: (i32, i32, i32)) -> Option<Vec<Pos>> {
    let mut col = Vec::new();
    let mut c = front;
    loop {
        let b = world.get_block(c.0, c.1, c.2);
        if b == block::AIR {
            return Some(col);
        }
        if !is_pushable(b) {
            return None;
        }
        col.push(c);
        if col.len() > MAX_PUSH {
            return None;
        }
        c = (c.0 + delta.0, c.1 + delta.1, c.2 + delta.2);
    }
}

/// Extend a piston at `piston` facing `meta`: shove the pushable column ahead by
/// one and place the head in the vacated front cell. No-op (empty result) if the
/// way is blocked. Returns the cells that changed (for remesh + broadcast).
pub fn extend(world: &mut World, piston: Pos, meta: u8) -> Vec<Pos> {
    let d = facing_delta(meta);
    let front = (piston.0 + d.0, piston.1 + d.1, piston.2 + d.2);
    let mut changed = Vec::new();
    let Some(col) = pushable_column(world, front, d) else {
        return changed; // blocked — stay retracted
    };
    // Shift the column far→near so a cell is always written into freed space.
    for &from in col.iter().rev() {
        let to = (from.0 + d.0, from.1 + d.1, from.2 + d.2);
        let b = world.get_block(from.0, from.1, from.2);
        let m = world.meta_at(from.0, from.1, from.2);
        // Spec 06 §2.2 — the player-placed flag travels with the block (mirrors
        // `falling_blocks`). Without this, shoving a placed block resets it to
        // "natural", laundering the anti-farming bit; shoving a natural block
        // could likewise inherit a stale placed bit from the destination cell.
        let placed = world.is_placed(from.0, from.1, from.2);
        world.set_block(to.0, to.1, to.2, b);
        world.set_meta(to, m);
        world.set_placed(to.0, to.1, to.2, placed);
        world.set_block(from.0, from.1, from.2, block::AIR);
        world.set_placed(from.0, from.1, from.2, false);
        changed.push(to);
        changed.push(from);
    }
    // Place the arm in the cell directly ahead (now air), carrying the facing.
    // The head is a system block — never player-placed.
    world.set_block(front.0, front.1, front.2, block::PISTON_HEAD);
    world.set_meta(front, meta);
    world.set_placed(front.0, front.1, front.2, false);
    changed.push(front);
    changed
}

/// True for either piston variant (plain or sticky).
pub fn is_piston(id: BlockId) -> bool {
    id == block::PISTON || id == block::STICKY_PISTON
}

/// Retract a piston: remove its head if extended. A `sticky` piston also pulls
/// the single block stuck to the head back into the vacated cell (Minecraft
/// parity — sticky pulls exactly one pushable block; a plain piston pulls
/// nothing). Returns the changed cells.
pub fn retract(world: &mut World, piston: Pos, meta: u8, sticky: bool) -> Vec<Pos> {
    let d = facing_delta(meta);
    let front = (piston.0 + d.0, piston.1 + d.1, piston.2 + d.2);
    if world.get_block(front.0, front.1, front.2) != block::PISTON_HEAD {
        return Vec::new();
    }
    let mut changed = vec![front];
    if sticky {
        // The block stuck to the head sits one cell beyond it.
        let beyond = (front.0 + d.0, front.1 + d.1, front.2 + d.2);
        let b = world.get_block(beyond.0, beyond.1, beyond.2);
        if is_pushable(b) {
            let m = world.meta_at(beyond.0, beyond.1, beyond.2);
            // Spec 06 §2.2 — carry the player-placed flag with the pulled block.
            let placed = world.is_placed(beyond.0, beyond.1, beyond.2);
            // Pull it back into the head cell; clear where it came from.
            world.set_block(front.0, front.1, front.2, b);
            world.set_meta(front, m);
            world.set_placed(front.0, front.1, front.2, placed);
            world.set_block(beyond.0, beyond.1, beyond.2, block::AIR);
            world.set_placed(beyond.0, beyond.1, beyond.2, false);
            changed.push(beyond);
            return changed;
        }
    }
    // Plain retract (or sticky with nothing to pull): clear the head.
    world.set_block(front.0, front.1, front.2, block::AIR);
    world.set_meta(front, 0); // don't leave the head's facing on the air cell
    world.set_placed(front.0, front.1, front.2, false);
    changed
}

/// Find every piston block (plain or sticky) in the loaded chunks.
pub fn collect_piston_positions(world: &World) -> Vec<Pos> {
    let mut out = Vec::new();
    for ((cx, cy, cz), chunk) in world.iter_chunks() {
        for ly in 0..crate::chunk::CHUNK_SIZE {
            for lz in 0..crate::chunk::CHUNK_SIZE {
                for lx in 0..crate::chunk::CHUNK_SIZE {
                    if is_piston(chunk.get(lx, ly, lz)) {
                        out.push((
                            cx * crate::chunk::CHUNK_SIZE as i32 + lx as i32,
                            cy * crate::chunk::CHUNK_SIZE as i32 + ly as i32,
                            cz * crate::chunk::CHUNK_SIZE as i32 + lz as i32,
                        ));
                    }
                }
            }
        }
    }
    out
}

/// One piston pass: extend newly-powered pistons, retract newly-unpowered ones.
/// "Extended" is derived from whether a `PISTON_HEAD` sits in front, so no extra
/// state is stored. Returns the changed cells for the caller to remesh/broadcast.
pub fn tick_pistons(world: &mut World) -> Vec<Pos> {
    let mut changed = Vec::new();
    for p in collect_piston_positions(world) {
        let meta = world.meta_at(p.0, p.1, p.2);
        let d = facing_delta(meta);
        let front = (p.0 + d.0, p.1 + d.1, p.2 + d.2);
        let powered = crate::power::is_block_powered(world, p);
        let extended = world.get_block(front.0, front.1, front.2) == block::PISTON_HEAD;
        let sticky = world.get_block(p.0, p.1, p.2) == block::STICKY_PISTON;
        if powered && !extended {
            changed.extend(extend(world, p, meta));
        } else if !powered && extended {
            changed.extend(retract(world, p, meta, sticky));
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_from_look_maps_to_offsets() {
        assert_eq!(facing_from_look(0.0, 1.0, 0.0), Facing::Up);
        assert_eq!(facing_from_look(0.0, -1.0, 0.0), Facing::Down);
        assert_eq!(facing_from_look(1.0, 0.0, 0.0), Facing::East);
        assert_eq!(facing_from_look(-1.0, 0.0, 0.0), Facing::West);
        assert_eq!(facing_from_look(0.0, 0.0, 1.0), Facing::South);
        assert_eq!(facing_from_look(0.0, 0.0, -1.0), Facing::North);
        // facing_delta reads that facing field back to the matching unit step.
        assert_eq!(facing_delta(Facing::East.to_bits()), (1, 0, 0));
        assert_eq!(facing_delta(Facing::Up.to_bits()), (0, 1, 0));
    }

    #[test]
    fn pushable_allow_list_is_safe() {
        assert!(is_pushable(block::STONE));
        assert!(is_pushable(block::OAK_PLANKS));
        assert!(!is_pushable(block::AIR));
        assert!(!is_pushable(block::BEDROCK));
        assert!(!is_pushable(block::CHEST)); // never push a block-entity
        assert!(!is_pushable(block::WATER));
        assert!(!is_pushable(block::PISTON));
        assert!(!is_pushable(block::PISTON_HEAD));
        // Wiki audit 2026-07-09, finding #3 — the module doc comment
        // (line 10) has always promised Satori is never pushable ("so
        // your storage is always safe"), but SATORI_BLOCK was actually
        // in the allow-list below, contradicting it.
        assert!(!is_pushable(block::SATORI_BLOCK));
    }

    #[test]
    fn pushable_column_stops_at_air_and_refuses_blocked() {
        let mut w = World::new();
        // Piston at (0,5,0) facing +X (meta 5). Front (1,5,0)=stone, (2,5,0)=air.
        w.set_block(1, 5, 0, block::STONE);
        let col = pushable_column(&w, (1, 5, 0), (1, 0, 0)).expect("air after the stone");
        assert_eq!(col, vec![(1, 5, 0)]);
        // Bedrock in front → not pushable → None.
        w.set_block(1, 5, 0, block::BEDROCK);
        assert!(pushable_column(&w, (1, 5, 0), (1, 0, 0)).is_none());
        // Air in front → empty column (piston just heads into the gap).
        w.set_block(1, 5, 0, block::AIR);
        assert_eq!(pushable_column(&w, (1, 5, 0), (1, 0, 0)), Some(vec![]));
    }

    #[test]
    fn extend_shoves_the_column_and_places_the_head() {
        let mut w = World::new();
        // Piston at (0,5,0) facing +X. A stone block right in front.
        w.set_block(0, 5, 0, block::PISTON);
        w.set_meta((0, 5, 0), 5);
        w.set_block(1, 5, 0, block::STONE);
        // (2,5,0) is air — there's room to push.
        extend(&mut w, (0, 5, 0), 5);
        assert_eq!(w.get_block(2, 5, 0), block::STONE, "stone shoved one cell forward");
        assert_eq!(w.get_block(1, 5, 0), block::PISTON_HEAD, "head fills the vacated cell");
    }

    #[test]
    fn extend_carries_the_player_placed_flag() {
        let mut w = World::new();
        // Piston at (0,5,0) facing +X (meta 5). A PLAYER-PLACED stone in front;
        // (2,5,0) is air so there's room to push.
        w.set_block(0, 5, 0, block::PISTON);
        w.set_meta((0, 5, 0), 5);
        w.place_player_block(1, 5, 0, block::STONE);
        assert!(w.is_placed(1, 5, 0));
        extend(&mut w, (0, 5, 0), 5);
        // The placed bit must travel with the shoved block (anti-farming,
        // Spec 06 §2.2) — not get laundered into a "natural" block.
        assert_eq!(w.get_block(2, 5, 0), block::STONE);
        assert!(w.is_placed(2, 5, 0), "shoved placed block stays placed");
        assert!(!w.is_placed(1, 5, 0), "vacated cell (now the head) is not placed");
    }

    #[test]
    fn sticky_retract_carries_the_player_placed_flag() {
        let mut w = World::new();
        w.set_block(0, 5, 0, block::STICKY_PISTON);
        w.set_meta((0, 5, 0), 5);
        w.place_player_block(1, 5, 0, block::STONE);
        // Extend pushes the placed stone to (2,5,0); sticky retract pulls it back.
        extend(&mut w, (0, 5, 0), 5);
        assert!(w.is_placed(2, 5, 0), "placed bit survived the push");
        retract(&mut w, (0, 5, 0), 5, true);
        assert_eq!(w.get_block(1, 5, 0), block::STONE, "sticky pulled the stone back");
        assert!(w.is_placed(1, 5, 0), "pulled placed block stays placed");
        assert!(!w.is_placed(2, 5, 0), "its old cell is cleared");
    }

    #[test]
    fn extend_then_retract_removes_the_head() {
        let mut w = World::new();
        w.set_block(0, 5, 0, block::PISTON);
        w.set_meta((0, 5, 0), 5);
        // Nothing in front → extend just places the head into the air gap.
        extend(&mut w, (0, 5, 0), 5);
        assert_eq!(w.get_block(1, 5, 0), block::PISTON_HEAD);
        retract(&mut w, (0, 5, 0), 5, false);
        assert_eq!(w.get_block(1, 5, 0), block::AIR, "head removed on retract");
    }

    #[test]
    fn sticky_piston_pulls_its_block_back_on_retract() {
        let mut w = World::new();
        // Sticky piston at (0,5,0) facing +X; a stone block in front, air beyond.
        w.set_block(0, 5, 0, block::STICKY_PISTON);
        w.set_meta((0, 5, 0), 5);
        w.set_block(1, 5, 0, block::STONE);
        // Extend: stone shoved to (2,5,0), head fills (1,5,0).
        extend(&mut w, (0, 5, 0), 5);
        assert_eq!(w.get_block(2, 5, 0), block::STONE);
        assert_eq!(w.get_block(1, 5, 0), block::PISTON_HEAD);
        // Sticky retract: the stone is pulled back into (1,5,0), (2,5,0) clears.
        retract(&mut w, (0, 5, 0), 5, true);
        assert_eq!(w.get_block(1, 5, 0), block::STONE, "sticky pulls the block back");
        assert_eq!(w.get_block(2, 5, 0), block::AIR, "the block's old cell clears");
    }

    #[test]
    fn plain_retract_leaves_the_pushed_block_behind() {
        let mut w = World::new();
        w.set_block(0, 5, 0, block::PISTON);
        w.set_meta((0, 5, 0), 5);
        w.set_block(1, 5, 0, block::STONE);
        extend(&mut w, (0, 5, 0), 5);
        // Plain retract (sticky=false): the head clears but the stone stays put.
        retract(&mut w, (0, 5, 0), 5, false);
        assert_eq!(w.get_block(1, 5, 0), block::AIR, "head removed");
        assert_eq!(w.get_block(2, 5, 0), block::STONE, "plain piston does NOT pull it back");
    }

    #[test]
    fn sticky_with_nothing_to_pull_just_clears_the_head() {
        let mut w = World::new();
        w.set_block(0, 5, 0, block::STICKY_PISTON);
        w.set_meta((0, 5, 0), 5);
        // Nothing in front → extend places the head into air; beyond is air too.
        extend(&mut w, (0, 5, 0), 5);
        assert_eq!(w.get_block(1, 5, 0), block::PISTON_HEAD);
        retract(&mut w, (0, 5, 0), 5, true);
        assert_eq!(w.get_block(1, 5, 0), block::AIR, "nothing to pull → head clears");
    }

    #[test]
    fn extend_refuses_when_no_room() {
        let mut w = World::new();
        w.set_block(0, 5, 0, block::PISTON);
        w.set_meta((0, 5, 0), 5);
        // Stone then bedrock — the column can't move (bedrock isn't pushable).
        w.set_block(1, 5, 0, block::STONE);
        w.set_block(2, 5, 0, block::BEDROCK);
        let changed = extend(&mut w, (0, 5, 0), 5);
        assert!(changed.is_empty(), "blocked piston does nothing");
        assert_eq!(w.get_block(1, 5, 0), block::STONE, "stone unmoved");
        assert_eq!(w.get_block(2, 5, 0), block::BEDROCK);
    }
}
