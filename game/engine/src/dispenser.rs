//! Dispensers + Droppers (2026-07-04 gap-fill wave).
//!
//! Two container blocks sharing one implementation: a 9-slot inventory that,
//! on a POWER RISING EDGE, ejects its first occupied slot's item out of its
//! facing side. The DISPENSER "uses" what it can (arrows are shot as real
//! projectiles); the DROPPER always just tosses the item entity. Future
//! dispense behaviours (water bucket, bonemeal, fire charge) are new arms in
//! [`eject_decision`].
//!
//! Mechanics notes:
//! - The rising-edge latch lives HERE (`DispenserData.on`), the keg idiom —
//!   never in `PowerDeviceData`, whose bincode layout is frozen (positional
//!   fields inside `WorldSave.power_devices`).
//! - The inventory is an embedded 9-slot [`ChestData`], so the chest UI
//!   helpers (withdraw/deposit/sort) and the hopper feed work unchanged.
//! - `tick_dispensers` is pure decision-making: it pops items and returns
//!   eject orders; the caller (game loop) spawns the arrow / item entities,
//!   because entity spawning needs the ECS + audio it owns.

use serde::{Deserialize, Serialize};

use crate::block;
use crate::chest::ChestData;
use crate::chest::ChestTier;
use crate::item::{Item, ItemStack, MaterialId};
use crate::meta::Facing;
use crate::world::{BlockEntityData, World};

/// Slots in a dispenser/dropper (one 9-wide row in the dialog).
pub const DISPENSER_SLOTS: usize = 9;

/// Ejection speed (blocks/tick) for tossed items.
pub const TOSS_SPEED: f32 = 0.25;
/// Small upward lift on tossed items so they arc off the block.
pub const TOSS_LIFT: f32 = 0.1;

/// Per-block state for a Dispenser or Dropper.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispenserData {
    /// 9-slot inventory. Embedded `ChestData` so the chest UI + hopper
    /// helpers operate on it directly (tier is cosmetic-only here).
    pub chest: ChestData,
    /// Previous powered state — the rising-edge latch (keg idiom).
    #[serde(default)]
    pub on: bool,
}

impl Default for DispenserData {
    fn default() -> Self {
        Self::new()
    }
}

impl DispenserData {
    pub fn new() -> Self {
        Self {
            chest: ChestData { slots: vec![None; DISPENSER_SLOTS], tier: ChestTier::Wood },
            on: false,
        }
    }

    /// Index of the first occupied slot, if any.
    pub fn first_occupied(&self) -> Option<usize> {
        self.chest.slots.iter().position(|s| s.is_some())
    }

    /// Remove exactly one item from `idx`, returning a 1-count stack.
    pub fn take_one(&mut self, idx: usize) -> Option<ItemStack> {
        let slot = self.chest.slots.get_mut(idx)?;
        let stack = slot.as_mut()?;
        let mut taken = stack.clone();
        taken.count = 1;
        if stack.count <= 1 {
            *slot = None;
        } else {
            stack.count -= 1;
        }
        Some(taken)
    }
}

/// What the block does with the ejected item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EjectKind {
    /// DISPENSER + Arrow → shoot a real arrow projectile.
    Arrow,
    /// DISPENSER + filled bucket → pour: place this liquid source block in
    /// the facing cell (the empty Bucket goes back into the inventory).
    PlaceLiquid(crate::block::BlockId),
    /// DISPENSER + Bonemeal → bonemeal the crop in the facing cell.
    Bonemeal,
    /// DISPENSER + flint & steel / Magnesium Firestarter → light a fire in
    /// the facing cell (the igniter stays in the inventory).
    Ignite,
    /// Everything else (and every DROPPER eject) → toss an item entity.
    Toss,
}

/// Decide how `block_id` ejects `item`. Future dispense behaviours are new
/// arms HERE, not in the tick.
pub fn eject_decision(block_id: u16, item: &Item) -> EjectKind {
    if block_id == block::DISPENSER {
        match item {
            Item::Material(MaterialId::Arrow) => return EjectKind::Arrow,
            Item::Material(MaterialId::WaterBucket) => {
                return EjectKind::PlaceLiquid(block::WATER)
            }
            Item::Material(MaterialId::LavaBucket) => return EjectKind::PlaceLiquid(block::LAVA),
            Item::Material(MaterialId::Bonemeal) => return EjectKind::Bonemeal,
            // Both hand-igniters (see the flint-and-steel right-click path).
            Item::Material(MaterialId::MagnesiumFirestarter) => return EjectKind::Ignite,
            Item::Tool(t) if t.tool_type == crate::crafting::ToolType::FlintAndSteel => {
                return EjectKind::Ignite
            }
            _ => {}
        }
    }
    EjectKind::Toss
}

/// Put a stack back into the dispenser at `pos` — a failed or completed use
/// (bucket against a solid block, bonemeal on a mature crop, the returned
/// empty Bucket). Merge-or-first-empty via the hopper `dest_slot` idiom.
/// Returns `false` if the block-entity vanished or is full; the caller must
/// then toss the stack instead so items are never destroyed.
pub fn return_stack(world: &mut World, pos: (i32, i32, i32), stack: ItemStack) -> bool {
    let Some(d) = world.dispenser_at_mut(pos) else { return false };
    match crate::hopper::dest_slot(&d.chest, &stack.item) {
        Some(idx) => {
            match &mut d.chest.slots[idx] {
                Some(st) => st.count = st.count.saturating_add(stack.count),
                slot @ None => *slot = Some(stack),
            }
            true
        }
        None => false,
    }
}

/// One eject order for the caller to realise as an entity spawn.
#[derive(Clone, Debug)]
pub struct EjectOrder {
    pub pos: (i32, i32, i32),
    pub facing: Facing,
    pub stack: ItemStack,
    pub kind: EjectKind,
}

/// Advance every dispenser/dropper: latch `powered` and, on a rising edge,
/// pop the first item as an [`EjectOrder`]. Pure world-state pass — the
/// caller spawns entities and plays audio.
pub fn tick_dispensers(world: &mut World) -> Vec<EjectOrder> {
    let positions: Vec<(i32, i32, i32)> = world
        .block_entities
        .iter()
        .filter(|(_, be)| matches!(be, BlockEntityData::Dispenser(_)))
        .map(|(p, _)| *p)
        .collect();

    let mut orders = Vec::new();
    for pos in positions {
        let block_id = world.get_block(pos.0, pos.1, pos.2);
        if block_id != block::DISPENSER && block_id != block::DROPPER {
            // Block was broken/replaced under the entity — cleaned by the
            // break path; skip defensively.
            continue;
        }
        let powered = crate::power::is_block_powered(world, pos);
        let facing = crate::meta::facing(world.meta_at(pos.0, pos.1, pos.2));
        let Some(d) = world.dispenser_at_mut(pos) else { continue };
        let rising = powered && !d.on;
        d.on = powered;
        if !rising {
            continue;
        }
        let Some(idx) = d.first_occupied() else { continue };
        let Some(stack) = d.take_one(idx) else { continue };
        let kind = eject_decision(block_id, &stack.item);
        orders.push(EjectOrder { pos, facing, stack, kind });
    }
    orders
}

/// Remove the block-entity at `pos` and return its contents for spilling
/// (the `cleanup_chest` idiom — call from the break path).
pub fn cleanup_dispenser(world: &mut World, x: i32, y: i32, z: i32) -> Vec<ItemStack> {
    match world.block_entities.remove(&(x, y, z)) {
        Some(BlockEntityData::Dispenser(d)) => d.chest.slots.into_iter().flatten().collect(),
        Some(other) => {
            // Not ours — put it back untouched.
            world.block_entities.insert((x, y, z), other);
            Vec::new()
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::{PowerDeviceData, PowerDeviceKind};

    fn world_with_dispenser(block_id: u16) -> (World, (i32, i32, i32)) {
        let mut w = World::new();
        let pos = (0, 10, 0);
        w.set_block(pos.0, pos.1, pos.2, block_id);
        w.set_meta(pos, crate::meta::with_facing(0, Facing::East));
        w.insert_dispenser(pos, DispenserData::new());
        (w, pos)
    }

    /// Put an active lever right next to the dispenser so it reads powered.
    fn power_on(w: &mut World, at: (i32, i32, i32)) {
        w.set_block(at.0, at.1, at.2, block::LEVER);
        let mut lever = PowerDeviceData::new(PowerDeviceKind::Lever, Facing::Up);
        lever.on = true;
        w.insert_power_device(at, lever);
    }

    #[test]
    fn rising_edge_ejects_exactly_once() {
        let (mut w, pos) = world_with_dispenser(block::DROPPER);
        w.dispenser_at_mut(pos).unwrap().chest.slots[3] =
            Some(ItemStack::new_material(MaterialId::Bone, 5));

        // Unpowered ticks: nothing.
        assert!(tick_dispensers(&mut w).is_empty());
        assert!(tick_dispensers(&mut w).is_empty());

        power_on(&mut w, (0, 10, 1));
        let orders = tick_dispensers(&mut w);
        assert_eq!(orders.len(), 1, "rising edge fires once");
        assert_eq!(orders[0].facing, Facing::East);
        assert_eq!(orders[0].stack.count, 1, "ejects exactly one item");
        assert_eq!(orders[0].kind, EjectKind::Toss);

        // Held power: no repeat fire.
        assert!(tick_dispensers(&mut w).is_empty(), "held wire fires once, not every tick");
        // 4 remain in the slot.
        assert_eq!(
            w.dispenser_at(pos).unwrap().chest.slots[3].as_ref().unwrap().count,
            4
        );
    }

    #[test]
    fn dispenser_shoots_arrows_dropper_tosses_them() {
        let arrow = ItemStack::new_material(MaterialId::Arrow, 1);
        assert_eq!(eject_decision(block::DISPENSER, &arrow.item), EjectKind::Arrow);
        assert_eq!(eject_decision(block::DROPPER, &arrow.item), EjectKind::Toss);
        let bone = ItemStack::new_material(MaterialId::Bone, 1);
        assert_eq!(eject_decision(block::DISPENSER, &bone.item), EjectKind::Toss);
    }

    #[test]
    fn dispenser_arms_bucket_bonemeal_ignite() {
        // Campaign D (2026-07-05) — the reserved "future behaviours" arms.
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let water = Item::Material(MaterialId::WaterBucket);
        assert_eq!(eject_decision(block::DISPENSER, &water), EjectKind::PlaceLiquid(block::WATER));
        let lava = Item::Material(MaterialId::LavaBucket);
        assert_eq!(eject_decision(block::DISPENSER, &lava), EjectKind::PlaceLiquid(block::LAVA));
        let bm = Item::Material(MaterialId::Bonemeal);
        assert_eq!(eject_decision(block::DISPENSER, &bm), EjectKind::Bonemeal);
        let fas = Item::Tool(Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron));
        assert_eq!(eject_decision(block::DISPENSER, &fas), EjectKind::Ignite);
        let mag = Item::Material(MaterialId::MagnesiumFirestarter);
        assert_eq!(eject_decision(block::DISPENSER, &mag), EjectKind::Ignite);
        // Droppers never "use" — always Toss (unchanged).
        assert_eq!(eject_decision(block::DROPPER, &water), EjectKind::Toss);
        assert_eq!(eject_decision(block::DROPPER, &fas), EjectKind::Toss);
        // An empty bucket has no use-arm — plain Toss.
        let empty = Item::Material(MaterialId::Bucket);
        assert_eq!(eject_decision(block::DISPENSER, &empty), EjectKind::Toss);
    }

    #[test]
    fn return_stack_merges_or_falls_back() {
        let (mut w, pos) = world_with_dispenser(block::DISPENSER);
        w.dispenser_at_mut(pos).unwrap().chest.slots[2] =
            Some(ItemStack::new_material(MaterialId::Bonemeal, 3));
        // Merges onto the matching stack (the hopper dest_slot idiom).
        assert!(return_stack(&mut w, pos, ItemStack::new_material(MaterialId::Bonemeal, 1)));
        assert_eq!(
            w.dispenser_at(pos).unwrap().chest.slots[2].as_ref().unwrap().count,
            4
        );
        // A different item lands in the first empty slot.
        assert!(return_stack(&mut w, pos, ItemStack::new_material(MaterialId::Bucket, 1)));
        assert!(w
            .dispenser_at(pos)
            .unwrap()
            .chest
            .slots
            .iter()
            .flatten()
            .any(|s| matches!(s.item, Item::Material(MaterialId::Bucket))));
        // No block-entity → false (caller tosses instead; never destroy items).
        assert!(!return_stack(&mut w, (9, 9, 9), ItemStack::new_material(MaterialId::Bone, 1)));
    }

    #[test]
    fn empty_dispenser_latches_but_ejects_nothing() {
        let (mut w, pos) = world_with_dispenser(block::DISPENSER);
        power_on(&mut w, (0, 10, 1));
        assert!(tick_dispensers(&mut w).is_empty(), "empty: no order");
        assert!(w.dispenser_at(pos).unwrap().on, "latch still records powered");
    }

    #[test]
    fn cleanup_spills_contents() {
        let (mut w, pos) = world_with_dispenser(block::DROPPER);
        w.dispenser_at_mut(pos).unwrap().chest.slots[0] =
            Some(ItemStack::new_material(MaterialId::Bone, 5));
        w.dispenser_at_mut(pos).unwrap().chest.slots[8] =
            Some(ItemStack::new_material(MaterialId::Coal, 2));
        let spill = cleanup_dispenser(&mut w, pos.0, pos.1, pos.2);
        assert_eq!(spill.len(), 2, "both stacks spilled");
        assert!(w.dispenser_at(pos).is_none(), "entity removed");
    }

    #[test]
    fn take_one_drains_the_slot() {
        let mut d = DispenserData::new();
        d.chest.slots[0] = Some(ItemStack::new_material(MaterialId::Bone, 2));
        assert_eq!(d.take_one(0).unwrap().count, 1);
        assert_eq!(d.chest.slots[0].as_ref().unwrap().count, 1);
        assert_eq!(d.take_one(0).unwrap().count, 1);
        assert!(d.chest.slots[0].is_none(), "slot empties");
        assert!(d.take_one(0).is_none());
        assert!(d.first_occupied().is_none());
    }
}
