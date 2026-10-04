//! Hoppers (P7 gap-closure) — item transport between containers.
//!
//! A `HOPPER` block is a **pure conduit**: every [`HOPPER_INTERVAL_TICKS`] it
//! moves one item from the container directly above it into the container
//! directly below it. v1 connects chests (above → below), enough to drain one
//! chest into another or chain hoppers into a sorter. The hopper holds nothing
//! itself, so there's no block-entity and no save-format change.
//!
//! The slot helpers are pure + unit-tested; [`tick_hoppers`] runs the scan on
//! the interval cadence and applies the moves with sequential chest borrows. It
//! is called by the client loop (single-player / LAN host) and, on a dedicated
//! server, by `GameServer::tick` (T1-3) — one implementation, two callers.

use crate::chest::ChestData;
use crate::item::{Item, ItemStack};
use crate::world::World;

/// Ticks between hopper transfers (8 @ 20 TPS = Minecraft's hopper cadence).
pub const HOPPER_INTERVAL_TICKS: u64 = 8;

/// Two item stacks are the same *kind* for merging iff they're the same block
/// or the same material. Tools/plans/armour carry per-instance state and never
/// merge (they go to a fresh slot). `Item` has no `PartialEq`, hence this.
fn same_kind(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::Block(x), Item::Block(y)) => x == y,
        (Item::Material(x), Item::Material(y)) => x == y,
        _ => false,
    }
}

/// First non-empty slot of `chest` as `(index, item)`.
pub fn first_item(chest: &ChestData) -> Option<(usize, Item)> {
    chest.slots.iter().enumerate().find_map(|(i, s)| {
        s.as_ref()
            .filter(|st| st.count > 0)
            .map(|st| (i, st.item.clone()))
    })
}

/// Where one unit of `item` can land in `chest`: a matching stack with room,
/// else the first empty slot. `None` if the chest is full for this item.
pub fn dest_slot(chest: &ChestData, item: &Item) -> Option<usize> {
    let max = item.max_stack();
    chest
        .slots
        .iter()
        .position(|s| s.as_ref().is_some_and(|st| same_kind(&st.item, item) && st.count < max))
        .or_else(|| chest.slots.iter().position(|s| s.is_none()))
}

/// Remove one unit from `chest`'s slot `idx`, clearing the slot if it empties.
pub fn take_one(chest: &mut ChestData, idx: usize) {
    if let Some(st) = chest.slots.get_mut(idx).and_then(|s| s.as_mut()) {
        st.count = st.count.saturating_sub(1);
        if st.count == 0 {
            chest.slots[idx] = None;
        }
    }
}

/// Add one unit of `item` to `chest`'s slot `idx` (merge if matching, else set).
pub fn put_one(chest: &mut ChestData, idx: usize, item: Item) {
    match chest.slots.get_mut(idx) {
        Some(slot @ None) => *slot = Some(ItemStack { item, count: 1 }),
        Some(Some(st)) => st.count += 1,
        None => {}
    }
}

/// Resolve the hopper-connectable container at `pos`: a chest of any tier, or
/// a dispenser/dropper (2026-07-04 — their inventory IS an embedded
/// `ChestData`, so hoppers auto-feed them, the MC auto-farm idiom).
pub fn container_at(world: &World, pos: (i32, i32, i32)) -> Option<&ChestData> {
    world
        .chest_at(pos)
        .or_else(|| world.dispenser_at(pos).map(|d| &d.chest))
}

/// Mutable [`container_at`].
pub fn container_at_mut(world: &mut World, pos: (i32, i32, i32)) -> Option<&mut ChestData> {
    if world.chest_at(pos).is_some() {
        return world.chest_at_mut(pos);
    }
    world.dispenser_at_mut(pos).map(|d| &mut d.chest)
}

/// Scan loaded chunks for `HOPPER` blocks that have a container (chest OR
/// dispenser/dropper) both directly above and directly below. Returns
/// `(above_pos, below_pos)` transfer pairs.
pub fn collect_hopper_transfers(world: &World) -> Vec<((i32, i32, i32), (i32, i32, i32))> {
    let mut out = Vec::new();
    for ((cx, cy, cz), chunk) in world.iter_chunks() {
        for ly in 0..crate::chunk::CHUNK_SIZE {
            for lz in 0..crate::chunk::CHUNK_SIZE {
                for lx in 0..crate::chunk::CHUNK_SIZE {
                    if chunk.get(lx, ly, lz) != crate::block::HOPPER {
                        continue;
                    }
                    let wx = cx * crate::chunk::CHUNK_SIZE as i32 + lx as i32;
                    let wy = cy * crate::chunk::CHUNK_SIZE as i32 + ly as i32;
                    let wz = cz * crate::chunk::CHUNK_SIZE as i32 + lz as i32;
                    let above = (wx, wy + 1, wz);
                    let below = (wx, wy - 1, wz);
                    if container_at(world, above).is_some()
                        && container_at(world, below).is_some()
                    {
                        out.push((above, below));
                    }
                }
            }
        }
    }
    out
}

/// One hopper pass (P7; shared 2026-10-05, T1-3): on the
/// [`HOPPER_INTERVAL_TICKS`] cadence, move ONE item from the container above
/// each hopper into the container below it. Sequential borrows (read source,
/// compute dest, then put + take) avoid aliasing the world. Gated on `tick`
/// internally, so it is safe to call every tick.
///
/// The ONE implementation: the client loop and the dedicated server
/// (`block_machines.rs`) both call this. Container contents are block-entity
/// state, not blocks, so a transfer queues no `BlockChange`.
pub fn tick_hoppers(world: &mut World, tick: u64) {
    if !tick.is_multiple_of(HOPPER_INTERVAL_TICKS) {
        return;
    }
    for (above, below) in collect_hopper_transfers(world) {
        // Chest OR dispenser/dropper on either end (container_at).
        let Some((src_idx, item)) = container_at(world, above).and_then(first_item) else {
            continue;
        };
        let Some(dst_idx) = container_at(world, below).and_then(|c| dest_slot(c, &item)) else {
            continue; // destination full for this item
        };
        if let Some(c) = container_at_mut(world, below) {
            put_one(c, dst_idx, item);
        }
        if let Some(c) = container_at_mut(world, above) {
            take_one(c, src_idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MaterialId;

    fn chest_with(items: &[(MaterialId, u8)]) -> ChestData {
        let mut c = ChestData::new();
        for (i, (m, n)) in items.iter().enumerate() {
            c.slots[i] = Some(ItemStack::new_material(*m, *n));
        }
        c
    }

    #[test]
    fn first_item_skips_empty_slots() {
        let mut c = ChestData::new();
        c.slots[2] = Some(ItemStack::new_material(MaterialId::Coal, 5));
        let (idx, item) = first_item(&c).expect("has an item");
        assert_eq!(idx, 2);
        assert!(matches!(item, Item::Material(MaterialId::Coal)));
    }

    #[test]
    fn dest_slot_merges_onto_matching_stack() {
        let c = chest_with(&[(MaterialId::Coal, 5)]);
        let slot = dest_slot(&c, &Item::Material(MaterialId::Coal)).unwrap();
        assert_eq!(slot, 0, "should stack onto the existing coal");
    }

    #[test]
    fn dest_slot_uses_empty_when_no_match() {
        let c = chest_with(&[(MaterialId::Coal, 5)]);
        let slot = dest_slot(&c, &Item::Material(MaterialId::Diamond)).unwrap();
        assert_eq!(slot, 1, "diamond can't stack on coal, goes to next empty");
    }

    #[test]
    fn full_chest_has_no_dest() {
        // Fill every slot with a max diamond stack; another diamond can't fit
        // (diamonds stack to 64, all slots are full diamond stacks).
        let mut c = ChestData::new();
        for s in c.slots.iter_mut() {
            *s = Some(ItemStack::new_material(MaterialId::Diamond, 64));
        }
        assert!(dest_slot(&c, &Item::Material(MaterialId::Coal)).is_none());
    }

    #[test]
    fn move_one_drains_source_and_fills_dest() {
        let mut from = chest_with(&[(MaterialId::Coal, 2)]);
        let mut to = ChestData::new();
        let (idx, item) = first_item(&from).unwrap();
        let dst = dest_slot(&to, &item).unwrap();
        put_one(&mut to, dst, item);
        take_one(&mut from, idx);
        assert_eq!(from.slots[0].as_ref().unwrap().count, 1, "one coal left in source");
        assert_eq!(to.slots[0].as_ref().unwrap().count, 1, "one coal moved to dest");
    }

    #[test]
    fn take_one_clears_slot_at_zero() {
        let mut c = chest_with(&[(MaterialId::Coal, 1)]);
        take_one(&mut c, 0);
        assert!(c.slots[0].is_none(), "single item -> slot cleared");
    }
}
