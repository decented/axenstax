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
}
