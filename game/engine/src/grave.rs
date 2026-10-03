//! #47 — Graves. On death (Survival, when `keep_inventory` is off), the player's
//! 36-slot inventory is snapshotted into a recoverable GRAVE block instead of
//! scattered as item entities. Recovery restores each stack to its **original
//! inventory slot** (Corpse-mod parity).
//!
//! Design: the grave holds a 36-entry `slots` Vec **index-aligned to the player
//! inventory** — `slots[i]` is whatever was in inventory slot `i`. So
//! restore-to-original-slot is a plain index walk; no separate slot-map needed.
//! Reuses the `chest`/block-entity persistence pattern (a new
//! `BlockEntityData::Grave` + a `graves: Vec<SavedGrave>` save list).

use crate::inventory::Inventory;
use crate::item::ItemStack;

/// Number of slots a grave mirrors — the full player inventory (hotbar + main).
pub const GRAVE_SLOTS: usize = 36;

/// A death container: the snapshotted inventory + when it was made.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GraveData {
    /// Index-aligned to the player's 36 inventory slots at death.
    pub slots: Vec<Option<ItemStack>>,
    /// World tick the grave was created (for a future cosmetic-decay timer).
    pub created_tick: u32,
}

impl GraveData {
    /// Build a grave from a 36-entry inventory snapshot (index-aligned). Shorter
    /// snapshots are padded; longer are truncated, so the stored Vec is always
    /// exactly `GRAVE_SLOTS` long.
    pub fn from_snapshot(mut snapshot: Vec<Option<ItemStack>>, created_tick: u32) -> Self {
        snapshot.resize(GRAVE_SLOTS, None);
        Self { slots: snapshot, created_tick }
    }

    /// True once every slot has been recovered (the grave can be removed).
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|s| s.is_none())
    }

    /// Every non-empty stack still in the grave (for break-spill).
    pub fn remaining(&self) -> Vec<ItemStack> {
        self.slots.iter().flatten().cloned().collect()
    }

    /// Restore the grave into `inv`, putting each stack back in its **original**
    /// slot when that slot is free, else best-effort into the first free slot.
    /// Anything that doesn't fit stays in the grave (count-conserving). Returns
    /// `true` if the grave is now empty.
    pub fn restore_to_inventory(&mut self, inv: &mut Inventory) -> bool {
        for i in 0..GRAVE_SLOTS.min(self.slots.len()) {
            let Some(stack) = self.slots[i].take() else { continue };
            // Prefer the original slot if it's free.
            if i < 36 && inv.slot(i).is_none() {
                inv.set_slot(i, Some(stack));
                continue;
            }
            // Else fold into the inventory wherever it fits; keep the remainder.
            if let Some(remainder) = inv.add_item(stack) {
                self.slots[i] = Some(remainder);
            }
        }
        self.is_empty()
    }
}

/// Find a safe cell to place a grave near a death at `(x, y, z)` — one that won't
/// destroy a real build and isn't in the void or a hazard. `is_safe(x,y,z)`
/// returns true when a grave may occupy that cell (typically: in-bounds + the
/// current block is air/replaceable + not lava). Searches the death cell, then
/// straight up (you died falling / in liquid), then a small outward ring per
/// level. Returns `None` only if nothing nearby is safe (caller then falls back
/// to the legacy scatter). Pure — `is_safe` is injected, so it's unit-testable.
pub fn find_safe_grave_pos(
    is_safe: impl Fn(i32, i32, i32) -> bool,
    x: i32,
    y: i32,
    z: i32,
) -> Option<[i32; 3]> {
    // 1) The death cell itself.
    if is_safe(x, y, z) {
        return Some([x, y, z]);
    }
    // 2) Straight up a few blocks (escape lava/liquid/a tight pocket).
    for dy in 1..=6 {
        if is_safe(x, y + dy, z) {
            return Some([x, y + dy, z]);
        }
    }
    // 3) Outward rings at the death level and just above (void/edge cases).
    for dy in 0..=4i32 {
        for r in 1..=3i32 {
            for dx in -r..=r {
                for dz in -r..=r {
                    // Only the ring boundary at radius r (interior covered by smaller r).
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    if is_safe(x + dx, y + dy, z + dz) {
                        return Some([x + dx, y + dy, z + dz]);
                    }
                }
            }
        }
    }
    None
}

/// Remove the grave block-entity at `(x, y, z)` and return its remaining
/// contents so the caller can spill them as item entities (break path). Mirrors
/// `chest::cleanup_chest`. Idempotent on non-grave cells.
pub fn cleanup_grave(world: &mut crate::world::World, x: i32, y: i32, z: i32) -> Vec<ItemStack> {
    let spill = world
        .grave_at((x, y, z))
        .map(|g| g.remaining())
        .unwrap_or_default();
    world.remove_block_entity((x, y, z));
    spill
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::item::ItemStack;

    fn snap_with(stone_at: &[usize]) -> Vec<Option<ItemStack>> {
        let mut v = vec![None; GRAVE_SLOTS];
        for &i in stone_at {
            v[i] = Some(ItemStack::new_block(block::STONE, 5));
        }
        v
    }

    #[test]
    fn from_snapshot_is_always_36_long() {
        let g = GraveData::from_snapshot(vec![None; 3], 0);
        assert_eq!(g.slots.len(), GRAVE_SLOTS);
        let g2 = GraveData::from_snapshot(vec![None; 50], 0);
        assert_eq!(g2.slots.len(), GRAVE_SLOTS);
    }

    #[test]
    fn restore_returns_each_stack_to_its_original_slot() {
        let mut g = GraveData::from_snapshot(snap_with(&[0, 5, 20]), 7);
        let mut inv = Inventory::new();
        let empty = g.restore_to_inventory(&mut inv);
        assert!(empty, "all slots free → grave emptied");
        assert_eq!(inv.slot(0).map(|s| s.count), Some(5));
        assert_eq!(inv.slot(5).map(|s| s.count), Some(5));
        assert_eq!(inv.slot(20).map(|s| s.count), Some(5));
        assert!(g.is_empty());
    }

    #[test]
    fn restore_conserves_count_when_original_slot_taken() {
        // Original slot 0 is occupied by a different stack; the grave's slot-0
        // stone must still land somewhere (count conserved), not vanish.
        let mut g = GraveData::from_snapshot(snap_with(&[0]), 0);
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::DIRT, 1)));
        let empty = g.restore_to_inventory(&mut inv);
        assert!(empty);
        let stone: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .filter(|s| matches!(&s.item, crate::item::Item::Block(b) if *b == block::STONE))
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(stone, 5, "stone preserved into a fallback slot");
    }

    #[test]
    fn restore_keeps_unfit_items_in_the_grave() {
        // Fill the inventory with non-stacking tools so the grave's stone can't
        // land — it must STAY in the grave (no loss), grave not empty.
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut g = GraveData::from_snapshot(snap_with(&[10]), 0);
        let mut inv = Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        let empty = g.restore_to_inventory(&mut inv);
        assert!(!empty, "nothing fit → grave retains the stone");
        assert_eq!(g.remaining().len(), 1, "stone still in grave");
    }

    #[test]
    fn safe_pos_prefers_the_death_cell_when_safe() {
        let pos = find_safe_grave_pos(|_, _, _| true, 4, 64, -2);
        assert_eq!(pos, Some([4, 64, -2]));
    }

    #[test]
    fn safe_pos_searches_upward_out_of_a_hazard() {
        // Death cell + the two above are unsafe (e.g. lava column); the 3rd up is air.
        let safe_y = 67;
        let pos = find_safe_grave_pos(|_, y, _| y >= safe_y, 0, 64, 0);
        assert_eq!(pos, Some([0, safe_y, 0]));
    }

    #[test]
    fn safe_pos_falls_back_outward_when_column_blocked() {
        // The whole column at x=0 is unsafe; only a neighbour at x=1 is safe.
        let pos = find_safe_grave_pos(|x, _, _| x == 1, 0, 64, 0);
        assert!(pos.is_some());
        assert_eq!(pos.unwrap()[0], 1, "found the safe neighbour column");
    }

    #[test]
    fn safe_pos_none_when_nothing_is_safe() {
        assert_eq!(find_safe_grave_pos(|_, _, _| false, 0, 64, 0), None);
    }
}
