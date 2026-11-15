//! Shared inventory split for a death: the 36 inventory slots and the off-hand
//! item move into the grave (held things drop). Armour, the cursor stack and
//! the craft grid are not passed in, so a death never sees them (worn things
//! stay).
use crate::{
    inventory::Inventory,
    item::{Item, ItemStack},
};
pub fn split_window(
    inv: &Inventory,
    keep_plans: bool,
) -> ([Option<ItemStack>; 36], [Option<ItemStack>; 36]) {
    let mut grave = std::array::from_fn(|_| None);
    let mut kept = std::array::from_fn(|_| None);
    for i in 0..36 {
        let stack = inv.slot(i).cloned();
        if keep_plans
            && stack
                .as_ref()
                .is_some_and(|s| matches!(s.item, Item::Plan(_)))
        {
            kept[i] = stack;
        } else {
            grave[i] = stack;
        }
    }
    (grave, kept)
}

/// Bury a body's held things: move the bag (`inv`) and the off-hand into the
/// returned grave snapshot, `GRAVE_SLOTS` long and index-aligned (0-35 the bag,
/// `GRAVE_OFFHAND_SLOT` the off-hand), leaving behind only what a death keeps.
/// `keep_plans` (a joiner's rule) leaves Plans where they are, in the bag or in
/// the off-hand. Both death sites (the local one in `game_loop`, the joiner's
/// on the authority) bury through this one rule.
pub fn bury(
    inv: &mut Inventory,
    offhand: &mut Option<ItemStack>,
    keep_plans: bool,
) -> Vec<Option<ItemStack>> {
    let (grave, kept) = split_window(inv, keep_plans);
    for (at, stack) in kept.into_iter().enumerate() {
        inv.set_slot(at, stack);
    }
    let mut snapshot: Vec<Option<ItemStack>> = grave.into();
    let keeps = keep_plans
        && offhand
            .as_ref()
            .is_some_and(|s| matches!(s.item, Item::Plan(_)));
    snapshot.push(if keeps { None } else { offhand.take() });
    debug_assert_eq!(snapshot.len(), crate::grave::GRAVE_SLOTS);
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joiner_keeps_plans_and_local_death_moves_them() {
        let mut inv = Inventory::new();
        inv.set_slot(2, Some(ItemStack::new_block(crate::block::STONE, 7)));
        inv.set_slot(
            8,
            Some(ItemStack {
                item: Item::Plan(crate::satoshi::starter_hut_plan()),
                count: 1,
            }),
        );
        let (grave, kept) = split_window(&inv, true);
        assert_eq!(grave[2].as_ref().unwrap().count, 7);
        assert!(grave[8].is_none());
        assert!(kept[8].is_some());
        let (grave, kept) = split_window(&inv, false);
        assert!(grave[8].is_some());
        assert!(kept.iter().all(Option::is_none));
    }

    fn shield() -> ItemStack {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood))
    }

    /// Held things drop, worn things stay: the off-hand item goes into the
    /// grave with the bag, and the armour is still worn.
    #[test]
    fn the_off_hand_item_goes_into_the_grave_and_the_armour_stays_on() {
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        slot.inventory.set_slot(2, Some(ItemStack::new_block(crate::block::STONE, 7)));
        slot.offhand = Some(shield());
        slot.armour_slots[0] = Some(crate::armour::ArmourItem::new(
            crate::armour::ArmourSlot::Helmet,
            crate::armour::ArmourMaterial::Iron,
        ));
        let grave = bury(&mut slot.inventory, &mut slot.offhand, false);
        assert_eq!(grave.len(), crate::grave::GRAVE_SLOTS);
        assert_eq!(grave[2].as_ref().map(|s| s.count), Some(7), "the bag goes to the grave");
        assert_eq!(grave[crate::grave::GRAVE_OFFHAND_SLOT], Some(shield()), "so does the off-hand item");
        assert!(slot.offhand.is_none(), "the hand is empty");
        assert!(slot.inventory.slot(2).is_none());
        assert!(slot.armour_slots[0].is_some(), "the armour is still worn");
    }

    #[test]
    fn a_joiner_keeps_a_plan_held_in_the_off_hand() {
        let plan = ItemStack { item: Item::Plan(crate::satoshi::starter_hut_plan()), count: 1 };
        let mut inv = Inventory::new();
        let mut off = Some(plan.clone());
        let grave = bury(&mut inv, &mut off, true);
        assert!(grave.iter().all(Option::is_none));
        assert_eq!(off, Some(plan.clone()), "the keep-Plans rule covers the off-hand");
        let mut off = Some(plan.clone());
        let grave = bury(&mut inv, &mut off, false);
        assert_eq!(grave[crate::grave::GRAVE_OFFHAND_SLOT], Some(plan), "a local death buries it");
        assert!(off.is_none());
    }
}
