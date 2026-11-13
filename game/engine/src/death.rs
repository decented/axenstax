//! Shared inventory split for a death: only the 36 inventory slots move.
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

    /// The off-hand is not one of the 36 slots, so a death never sees it: like
    /// the armour, it stays on the body (it is not in the grave, and
    /// `keep_inventory` has nothing to say about it).
    #[test]
    fn the_off_hand_item_stays_on_the_body_at_death() {
        let torches = ItemStack::new_block(crate::block::TORCH, 12);
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        slot.inventory.set_slot(2, Some(ItemStack::new_block(crate::block::STONE, 7)));
        slot.offhand = Some(torches.clone());
        // The one split both death paths use (the local death in `game_loop`).
        let (grave, kept) = split_window(&slot.inventory, false);
        assert_eq!(grave[2].as_ref().map(|s| s.count), Some(7), "the bag goes to the grave");
        assert!(
            grave.iter().chain(kept.iter()).flatten().all(|s| s.item != torches.item),
            "the off-hand item is in neither the grave nor the kept window"
        );
        assert_eq!(slot.offhand, Some(torches), "and it is still in the hand");
    }
}
