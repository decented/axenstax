//! Historical Pivot Sub-Foundation 2 (HP-2) — Chest block-entity.
//!
//! 27-slot (3 rows × 9 cols) storage container. Right-click opens the
//! `chest_ui.rs` dialog; break drops all contents at the chest's world
//! position via `cleanup_chest`. State persisted via
//! `BlockEntityData::Chest(ChestData)` (see `world.rs`).
//!
//! Single-player anyone-can-touch model — matches Vendor Block. Per-chest
//! ownership / Charter gating is explicitly out of scope for HP-2; that
//! ships when multiplayer storage lands.
//!
//! Spec: `docs/foundations/2026-05-22-historical-pivot-wild-animals.md`.

use serde::{Deserialize, Serialize};

use crate::item::ItemStack;

/// Slots in a wood (tier-0) chest. 3 rows × 9 cols — Minecraft-standard.
pub const CHEST_SLOTS: usize = 27;

/// #15 — storage tier. The wood `CHEST` is tier 0; higher tiers add capacity
/// and (top tiers) auto-collect. Materials: Gold is omitted (it doesn't exist
/// in the engine) — see `docs/foundations/2026-06-16-tiered-storage.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChestTier {
    #[default]
    Wood,
    Copper,
    Iron,
    Diamond,
    Satori,
}

impl ChestTier {
    /// All tiers, lowest → highest (drives registration + recipe tables).
    pub const ALL: [ChestTier; 5] = [
        ChestTier::Wood,
        ChestTier::Copper,
        ChestTier::Iron,
        ChestTier::Diamond,
        ChestTier::Satori,
    ];

    /// Rows of 9 slots. The capacity ladder (numbers = playtest-tunable).
    pub fn rows(self) -> usize {
        match self {
            ChestTier::Wood => 3,
            ChestTier::Copper => 4,
            ChestTier::Iron => 5,
            ChestTier::Diamond => 6,
            ChestTier::Satori => 8,
        }
    }

    /// Total slots = rows × 9.
    pub fn slots(self) -> usize {
        self.rows() * 9
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ChestTier::Wood => "Chest",
            ChestTier::Copper => "Copper Chest",
            ChestTier::Iron => "Iron Chest",
            ChestTier::Diamond => "Diamond Chest",
            ChestTier::Satori => "Satori Chest",
        }
    }

    /// The block id placed for this tier.
    pub fn block_id(self) -> crate::block::BlockId {
        use crate::block;
        match self {
            ChestTier::Wood => block::CHEST,
            ChestTier::Copper => block::COPPER_CHEST,
            ChestTier::Iron => block::IRON_CHEST,
            ChestTier::Diamond => block::DIAMOND_CHEST,
            ChestTier::Satori => block::SATORI_CHEST,
        }
    }

    /// The tier a block id represents, or `None` if it isn't a chest.
    pub fn from_block(b: crate::block::BlockId) -> Option<ChestTier> {
        ChestTier::ALL.into_iter().find(|t| t.block_id() == b)
    }

    /// Whether a placed chest of this tier hoovers nearby dropped items.
    pub fn auto_collects(self) -> bool {
        matches!(self, ChestTier::Diamond | ChestTier::Satori)
    }
}

/// Per-chest state. `slots` mirrors the player inventory layout shape
/// (`Option<ItemStack>`) so the chest UI can use the same item-stack
/// click helpers without translation.
///
/// `#[serde(default)]` on each field would help forward-compat, but
/// `ChestData` is shipped at the same time as the new `BlockEntityData::
/// Chest` variant — there's no older save format to worry about
/// (deserialising old saves will simply have no `Chest` entries, which
/// is the correct empty state).
///
/// `PartialEq` (2026-07-06, Task 11) — a Donkey/Mule cargo pack embeds a
/// `ChestData` on `HorseData`, and `SavedTamedPetData` derives `PartialEq`
/// for its wire round-trip tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChestData {
    pub slots: Vec<Option<ItemStack>>,
    /// #15 — storage tier. `#[serde(default)]` ⇒ pre-#15 saves load as `Wood`
    /// (tier 0), matching their 27-slot `slots` length. Append-only.
    #[serde(default)]
    pub tier: ChestTier,
}

impl Default for ChestData {
    fn default() -> Self {
        Self {
            slots: vec![None; CHEST_SLOTS],
            tier: ChestTier::Wood,
        }
    }
}

impl ChestData {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh chest sized to `tier`'s capacity.
    pub fn for_tier(tier: ChestTier) -> Self {
        Self {
            slots: vec![None; tier.slots()],
            tier,
        }
    }

    /// Number of non-empty slots — used by the Bear food-raid scanner
    /// to know whether the chest has anything worth raiding.
    pub fn occupied(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// True when at least one slot holds a `is_food()`-true item. Used
    /// by the Bear food-raid scanner — only chests with food in them
    /// register as a Bear target.
    pub fn has_food(&self) -> bool {
        self.slots.iter().any(|s| {
            s.as_ref().is_some_and(|st| st.item.is_food())
        })
    }

    /// Remove one unit of food from the first slot that has any. The
    /// stack count decrements by 1; if it hits 0 the slot empties.
    /// Returns the removed stack (always count = 1) or `None` if no
    /// food was present.
    ///
    /// Used by the Bear's RaidChest transition: it pulls a single
    /// food item and gains satiety; the chest stays intact.
    pub fn take_one_food(&mut self) -> Option<ItemStack> {
        for slot in self.slots.iter_mut() {
            let take = match slot {
                Some(stack) if stack.item.is_food() => {
                    let mut taken = stack.clone();
                    taken.count = 1;
                    if stack.count > 1 {
                        stack.count -= 1;
                    } else {
                        *slot = None;
                    }
                    Some(taken)
                }
                _ => None,
            };
            if take.is_some() {
                return take;
            }
        }
        None
    }

    /// Try to deposit a stack into the chest. Stacks onto an existing
    /// compatible slot first (up to u8::MAX per slot); leftover spills
    /// into the first empty slot. Returns the unplaced remainder (count
    /// 0 if everything fit). Used by the future hopper / quick-deposit
    /// path; the v1 chest UI does its own slot-mutation directly.
    pub fn try_insert(&mut self, mut stack: ItemStack) -> ItemStack {
        // Cap each slot at the item's max_stack (64), not u8::MAX. Stacking to
        // 255 was a cap violation that, once withdrawn onto a normal inventory
        // slot, overflowed the u8 add in click_inventory_slot (engine audit
        // 2026-06-04, A). Overflow spills into further slots.
        let max = stack.item.max_stack();
        for slot in self.slots.iter_mut() {
            if stack.count == 0 {
                break;
            }
            if let Some(existing) = slot
                && existing.item.can_stack_with(&stack.item) && existing.count < max {
                    let room = max - existing.count;
                    let take = stack.count.min(room);
                    existing.count += take;
                    stack.count -= take;
                }
        }
        for slot in self.slots.iter_mut() {
            if stack.count == 0 {
                break;
            }
            if slot.is_none() {
                let take = stack.count.min(max);
                let mut placed = stack.clone();
                placed.count = take;
                *slot = Some(placed);
                stack.count -= take;
            }
        }
        stack
    }
}

/// Spec-mirrored cleanup helper. Removes the block-entity entry at
/// `pos` and returns every non-empty slot's contents so the caller can
/// spawn them as ItemEntities at the chest's world position. Idempotent
/// — safe on cells that were never chests.
pub fn cleanup_chest(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<ItemStack> {
    let mut spill: Vec<ItemStack> = Vec::new();
    if let Some(c) = world.chest_at((x, y, z)) {
        for slot in &c.slots {
            if let Some(stack) = slot.as_ref()
                && stack.count > 0 {
                    spill.push(stack.clone());
                }
        }
    }
    world.remove_block_entity((x, y, z));
    spill
}

/// #15 — radius (blocks) within which an auto-collecting chest hoovers dropped
/// items. Measured from the chest block centre.
pub const CHEST_ABSORB_RADIUS: f32 = 4.0;

/// #15 auto-collect: pull nearby dropped `ItemEntity`s into auto-collecting
/// chests (top tiers). Each item goes to the nearest in-range auto-collect
/// chest; fully-absorbed items are despawned, partial absorbs leave the
/// remainder on the ground. Returns the count fully absorbed.
///
/// Bounded: only auto-collecting chests participate, and the caller throttles
/// the call. A spatial index would replace the O(items × chests) scan if the
/// chest count ever grows large — fine for single-player counts now.
pub fn tick_chest_autocollect(world: &mut crate::world::World, ecs: &mut hecs::World) -> u32 {
    use crate::entity::{ItemEntity, Position};
    // Auto-collecting chest positions (block centres).
    let chests: Vec<(i32, i32, i32)> = world
        .iter_chests()
        .filter(|(_, c)| c.tier.auto_collects())
        .map(|(p, _)| p)
        .collect();
    if chests.is_empty() {
        return 0;
    }
    // First pass: pair each item with the nearest in-range auto-collect chest.
    let mut actions: Vec<(hecs::Entity, (i32, i32, i32))> = Vec::new();
    for (id, (pos, _item)) in ecs.query::<(&Position, &ItemEntity)>().iter() {
        let p = pos.0;
        let mut best: Option<((i32, i32, i32), f32)> = None;
        for &c in &chests {
            let centre = glam::Vec3::new(c.0 as f32 + 0.5, c.1 as f32 + 0.5, c.2 as f32 + 0.5);
            let d = (centre - p).length();
            if d <= CHEST_ABSORB_RADIUS && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((c, d));
            }
        }
        if let Some((c, _)) = best {
            actions.push((id, c));
        }
    }
    // Second pass: absorb.
    let mut absorbed = 0u32;
    let mut to_despawn: Vec<hecs::Entity> = Vec::new();
    for (id, cpos) in actions {
        // Read the item's stack, then route it into the chest.
        let stack = match ecs.get::<&ItemEntity>(id) {
            Ok(item) => item.stack.clone(),
            Err(_) => continue,
        };
        if let Some(chest) = world.chest_at_mut(cpos) {
            let leftover = chest.try_insert(stack);
            if leftover.count == 0 {
                absorbed += 1;
                to_despawn.push(id);
            } else if let Ok(mut item) = ecs.get::<&mut ItemEntity>(id) {
                item.stack = leftover;
            }
        }
    }
    for id in to_despawn {
        let _ = ecs.despawn(id);
    }
    absorbed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, ItemStack, MaterialId};

    fn bread() -> ItemStack { ItemStack::new_material(MaterialId::Bread, 1) }
    fn iron() -> ItemStack { ItemStack::new_material(MaterialId::IronIngot, 1) }

    #[test]
    fn default_chest_has_27_empty_slots() {
        let c = ChestData::default();
        assert_eq!(c.slots.len(), CHEST_SLOTS);
        assert!(c.slots.iter().all(|s| s.is_none()));
        assert_eq!(c.occupied(), 0);
        assert!(!c.has_food());
    }

    #[test]
    fn occupied_counts_non_empty_slots_only() {
        let mut c = ChestData::new();
        c.slots[0] = Some(bread());
        c.slots[10] = Some(iron());
        assert_eq!(c.occupied(), 2);
    }

    #[test]
    fn has_food_true_when_any_slot_holds_food() {
        let mut c = ChestData::new();
        c.slots[5] = Some(iron());
        assert!(!c.has_food(), "iron is not food");
        c.slots[20] = Some(bread());
        assert!(c.has_food(), "bread is food");
    }

    #[test]
    fn take_one_food_decrements_stack_returns_one() {
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 3));
        let taken = c.take_one_food().expect("food present");
        assert_eq!(taken.count, 1);
        assert!(matches!(taken.item, Item::Material(MaterialId::Bread)));
        assert_eq!(c.slots[0].as_ref().unwrap().count, 2);
    }

    #[test]
    fn take_one_food_clears_slot_when_last_unit_taken() {
        let mut c = ChestData::new();
        c.slots[7] = Some(bread());
        c.take_one_food().expect("food present");
        assert!(c.slots[7].is_none(), "slot must empty");
    }

    #[test]
    fn take_one_food_returns_none_when_no_food() {
        let mut c = ChestData::new();
        c.slots[0] = Some(iron());
        assert!(c.take_one_food().is_none());
    }

    #[test]
    fn take_one_food_skips_non_food_slots() {
        let mut c = ChestData::new();
        c.slots[0] = Some(iron());
        c.slots[1] = Some(bread());
        let taken = c.take_one_food().expect("food present after iron slot");
        assert!(matches!(taken.item, Item::Material(MaterialId::Bread)));
        assert!(c.slots[0].is_some(), "iron untouched");
        assert!(c.slots[1].is_none(), "bread slot emptied");
    }

    #[test]
    fn try_insert_stacks_onto_compatible_slot() {
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 5));
        let leftover = c.try_insert(ItemStack::new_material(MaterialId::Bread, 10));
        assert_eq!(leftover.count, 0);
        assert_eq!(c.slots[0].as_ref().unwrap().count, 15);
    }

    #[test]
    fn try_insert_spills_into_empty_slot_when_stack_full() {
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, u8::MAX));
        let leftover = c.try_insert(ItemStack::new_material(MaterialId::Bread, 10));
        assert_eq!(leftover.count, 0);
        assert_eq!(c.slots[1].as_ref().unwrap().count, 10);
    }

    #[test]
    fn try_insert_returns_remainder_when_chest_is_full() {
        let mut c = ChestData::new();
        for slot in c.slots.iter_mut() {
            *slot = Some(ItemStack::new_material(MaterialId::Bread, u8::MAX));
        }
        let leftover = c.try_insert(ItemStack::new_material(MaterialId::IronIngot, 4));
        assert_eq!(leftover.count, 4, "no room left");
    }

    #[test]
    fn try_insert_caps_slot_at_max_stack_not_255() {
        // Blocks/materials cap at 64. Inserting 100 into an empty chest must
        // put 64 in the first slot and spill 36 into the next — NOT pile 100
        // into one slot (the old u8::MAX cap, which then overflowed the u8 add
        // in click_inventory_slot when withdrawn).
        let mut c = ChestData::new();
        let leftover = c.try_insert(ItemStack::new_block(crate::block::STONE, 100));
        assert_eq!(leftover.count, 0, "all 100 placed across slots");
        assert_eq!(c.slots[0].as_ref().unwrap().count, 64, "slot capped at max_stack(64)");
        assert_eq!(c.slots[1].as_ref().unwrap().count, 36, "overflow spilled to the next slot");
    }

    #[test]
    fn chest_data_round_trips_via_bincode() {
        let mut c = ChestData::new();
        c.slots[0] = Some(bread());
        c.slots[26] = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 32));
        let bytes = bincode::serialize(&c).unwrap();
        let back: ChestData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.slots.len(), CHEST_SLOTS);
        assert_eq!(back.slots[0].as_ref().unwrap().count, 1);
        assert_eq!(back.slots[26].as_ref().unwrap().count, 32);
        assert!(back.slots[10].is_none());
    }

    #[test]
    fn cleanup_chest_empties_returns_contents_and_is_idempotent() {
        let mut world = crate::world::World::new();
        let mut c = ChestData::new();
        c.slots[0] = Some(bread());
        c.slots[5] = Some(iron());
        world.insert_chest((10, 70, 10), c);

        let spill = cleanup_chest(&mut world, 10, 70, 10);
        assert_eq!(spill.len(), 2);
        assert!(world.chest_at((10, 70, 10)).is_none());

        let spill2 = cleanup_chest(&mut world, 10, 70, 10);
        assert!(spill2.is_empty(), "second cleanup is a no-op");
    }

    // ---- #15 storage tiers ----------------------------------------------

    #[test]
    fn tier_slot_ladder_increases() {
        assert_eq!(ChestTier::Wood.slots(), 27);
        assert_eq!(ChestTier::Copper.slots(), 36);
        assert_eq!(ChestTier::Iron.slots(), 45);
        assert_eq!(ChestTier::Diamond.slots(), 54);
        assert_eq!(ChestTier::Satori.slots(), 72);
        // Strictly monotonic.
        for w in ChestTier::ALL.windows(2) {
            assert!(w[1].slots() > w[0].slots(), "{:?} > {:?}", w[1], w[0]);
        }
    }

    #[test]
    fn tier_block_id_round_trips() {
        for t in ChestTier::ALL {
            assert_eq!(ChestTier::from_block(t.block_id()), Some(t));
        }
        // Wood maps to the legacy CHEST block.
        assert_eq!(ChestTier::Wood.block_id(), crate::block::CHEST);
        // A non-chest block is not a tier.
        assert_eq!(ChestTier::from_block(crate::block::STONE), None);
    }

    #[test]
    fn only_top_tiers_auto_collect() {
        assert!(!ChestTier::Wood.auto_collects());
        assert!(!ChestTier::Copper.auto_collects());
        assert!(!ChestTier::Iron.auto_collects());
        assert!(ChestTier::Diamond.auto_collects());
        assert!(ChestTier::Satori.auto_collects());
    }

    #[test]
    fn autocollect_absorbs_nearby_items_into_top_tier_chests() {
        use crate::entity::{spawn_item, ItemEntity};
        let mut world = crate::world::World::new();
        world.insert_chest((10, 64, 10), ChestData::for_tier(ChestTier::Diamond));
        let mut ecs = hecs::World::new();
        // An item ~2 blocks away — inside the absorb radius.
        spawn_item(
            &mut ecs,
            glam::Vec3::new(11.5, 64.5, 11.0),
            ItemStack::new_material(MaterialId::IronIngot, 5),
            1,
        );
        let absorbed = tick_chest_autocollect(&mut world, &mut ecs);
        assert_eq!(absorbed, 1, "the nearby item is absorbed");
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 0, "item despawned");
        let chest = world.chest_at((10, 64, 10)).unwrap();
        let total: u32 = chest.slots.iter().flatten().map(|s| s.count as u32).sum();
        assert_eq!(total, 5, "all 5 ingots landed in the chest");
    }

    #[test]
    fn autocollect_ignores_low_tiers_and_out_of_range_items() {
        use crate::entity::{spawn_item, ItemEntity};
        let mut world = crate::world::World::new();
        world.insert_chest((0, 64, 0), ChestData::for_tier(ChestTier::Iron)); // not auto
        world.insert_chest((50, 64, 50), ChestData::for_tier(ChestTier::Satori)); // auto but far
        let mut ecs = hecs::World::new();
        spawn_item(
            &mut ecs,
            glam::Vec3::new(0.5, 64.5, 0.5),
            ItemStack::new_material(MaterialId::IronIngot, 3),
            1,
        );
        let absorbed = tick_chest_autocollect(&mut world, &mut ecs);
        assert_eq!(absorbed, 0, "iron is not auto-collect; satori is out of range");
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 1, "item stays on the ground");
    }

    #[test]
    fn for_tier_sizes_the_slot_vec_and_defaults_wood() {
        let c = ChestData::for_tier(ChestTier::Iron);
        assert_eq!(c.slots.len(), 45);
        assert_eq!(c.tier, ChestTier::Iron);
        assert_eq!(ChestData::default().tier, ChestTier::Wood);
        assert_eq!(ChestTier::default(), ChestTier::Wood);
    }
}
