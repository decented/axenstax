//! Live cargo transitions and runtime revision metadata, without a stock copy.
use crate::{chest::{ChestData, ChestTier, CHEST_SLOTS}, item::{Item, ItemStack}, protocol::WireSlot};

/// Retained on the same entity across pack removal/re-equipping; never a save
/// identity and never reset while that generational entity remains alive.
#[derive(Clone, Copy, Debug)]
pub struct PackRevision(pub u64);

pub fn revision(ecs: &hecs::World, entity: hecs::Entity) -> u64 {
    ecs.get::<&PackRevision>(entity).map_or(1, |revision| revision.0)
}

pub fn project(pack: &ChestData, registry: &crate::block::BlockRegistry) -> Result<ChestData, String> {
    if pack.tier != ChestTier::Wood || pack.slots.len() != CHEST_SLOTS { return Err("A pack must have 27 wooden slots".into()); }
    let slots = pack.slots.iter().map(|slot| slot.as_ref().map(|stack| crate::staged_window::exact_marker(stack, registry).ok_or("Cargo cannot be projected exactly".to_string())).transpose()).collect::<Result<Vec<_>, _>>()?;
    Ok(ChestData { slots, tier: ChestTier::Wood })
}

/// One attempt's exact patch. Plans which stay inside the pack are moved
/// from their live owned slots, so sorting legacy cargo cannot drop a body.
/// This scratch value is discarded after the attempt, never another ledger.
pub struct CargoCommit {
    before: [WireSlot; CHEST_SLOTS],
    after: Vec<Option<ItemStack>>,
    plan_sources: [Option<usize>; CHEST_SLOTS],
}

impl CargoCommit {
    pub fn prepare(current: &ChestData, candidate: ChestData) -> Result<Self, String> {
        if current.slots.len() != CHEST_SLOTS || candidate.slots.len() != CHEST_SLOTS || current.tier != ChestTier::Wood || candidate.tier != ChestTier::Wood { return Err("Cargo changed its shape".into()); }
        let before = std::array::from_fn(|at| current.slots[at].as_ref().map(crate::inventory::stack_to_wire));
        let mut plan_sources = [None; CHEST_SLOTS];
        let mut used = [false; CHEST_SLOTS];
        for (destination, slot) in candidate.slots.iter().enumerate() {
            if let Some(ItemStack { item: Item::Plan(body), count }) = slot {
                if *count != 1 { return Err("A cargo Plan must be one item".into()); }
                let marker = crate::plan::marker(body);
                let developed = body.develop_state.is_developed();
                let source = current.slots.iter().enumerate().find_map(|(at, original)| {
                    match original {
                        Some(ItemStack { item: Item::Plan(original), count: 1 }) if !used[at] && crate::plan::marker(original) == marker && original.develop_state.is_developed() == developed => Some(at),
                        _ => None,
                    }
                }).ok_or("A cargo transition attempted Plan transfer")?;
                used[source] = true;
                plan_sources[destination] = Some(source);
            }
        }
        // The shared rule cannot move a Plan out either. Every original body
        // remains in exactly one slot of the candidate before anything moves.
        if current.slots.iter().enumerate().any(|(at, slot)| matches!(slot, Some(ItemStack { item: Item::Plan(_), .. })) && !used[at]) {
            return Err("A cargo transition attempted to release a Plan".into());
        }
        Ok(Self { before, after: candidate.slots, plan_sources })
    }

    pub fn changed(&self) -> bool {
        self.before.iter().zip(&self.after).any(|(before, after)| *before != after.as_ref().map(crate::inventory::stack_to_wire))
    }
    pub fn matches(&self, current: &ChestData) -> bool {
        current.tier == ChestTier::Wood && current.slots.len() == CHEST_SLOTS && current.slots.iter().zip(&self.before).all(|(slot, before)| slot.as_ref().map(crate::inventory::stack_to_wire) == *before)
    }
    /// Called only after live generation/revision/permission and every
    /// reservation passed. A changed live pack refuses without any write.
    pub fn commit(mut self, current: &mut ChestData) -> bool {
        if !self.matches(current) { return false; }
        let mut original = std::mem::replace(&mut current.slots, std::mem::take(&mut self.after));
        for (destination, source) in self.plan_sources.iter().enumerate() {
            if let Some(source) = source { current.slots[destination] = original[*source].take(); }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn d2c2_live_cargo_patch_refuses_a_race_without_overwriting_the_new_stack() {
        let registry = crate::block::BlockRegistry::new();
        let mut live = ChestData::new();
        live.slots[0] = Some(ItemStack::new_block(crate::block::STONE, 3));
        let mut candidate = project(&live, &registry).unwrap();
        candidate.slots[0] = None;
        let patch = CargoCommit::prepare(&live, candidate).unwrap();
        live.slots[0] = Some(ItemStack::new_block(crate::block::STONE, 4));
        assert!(!patch.commit(&mut live));
        assert_eq!(live.slots[0].as_ref().unwrap().count, 4);
    }
    #[test]
    fn d2c2_local_full_plan_moves_inside_live_cargo_without_body_loss() {
        let registry = crate::block::BlockRegistry::new();
        let mut live = ChestData::new();
        let body = crate::plan::PlanData::debug_3x3_stone();
        let bytes = bincode::serialize(&body).unwrap();
        live.slots[8] = Some(ItemStack { item: Item::Plan(body), count: 1 });
        let mut candidate = project(&live, &registry).unwrap();
        candidate.slots.swap(8, 0);
        let patch = CargoCommit::prepare(&live, candidate).unwrap();
        assert!(patch.changed());
        assert!(patch.commit(&mut live));
        let Item::Plan(body) = &live.slots[0].as_ref().unwrap().item else { panic!("body") };
        assert_eq!(bincode::serialize(body).unwrap(), bytes);
        assert!(live.slots[8].is_none());
    }
}
