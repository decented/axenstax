//! Spec 39 Server Bazaar integration tests.
//!
//! Exercises the sell flow over a real Inventory + the trade-value
//! floor. Pure sell_quote maths in `src/bazaar.rs`.

use crate::bazaar;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};

#[test]
fn sell_quote_matches_trade_value_times_count() {
    let stack = ItemStack { item: Item::Material(MaterialId::IronIngot), count: 4 };
    let unit = stack.item.trade_value().unwrap();
    assert_eq!(bazaar::sell_quote(&stack), Some(unit * 4));
}

#[test]
fn selling_held_stack_consumes_it_and_quote_is_positive() {
    // Simulate the game-loop sell: read the held stack's quote, then
    // take it from the inventory. The quote must be > 0 and the slot
    // must empty afterwards.
    let mut inv = Inventory::new();
    inv.set_slot(0, Some(ItemStack { item: Item::Material(MaterialId::Diamond), count: 3 }));
    let held = inv.hotbar_slot(0).cloned().unwrap();
    let quote = bazaar::sell_quote(&held).expect("diamond stack has a floor");
    assert!(quote > 0);
    let sold = inv.take_slot(0);
    assert!(sold.is_some());
    assert_eq!(sold.unwrap().count, 3);
    assert!(inv.hotbar_slot(0).is_none(), "slot emptied after sale");
}

#[test]
fn untradeable_held_item_has_no_quote() {
    let mut inv = Inventory::new();
    // AIR as a held block — trade_value None.
    inv.set_slot(0, Some(ItemStack { item: Item::Block(crate::block::AIR), count: 1 }));
    let held = inv.hotbar_slot(0).cloned().unwrap();
    assert_eq!(bazaar::sell_quote(&held), None);
}

#[test]
fn empty_hand_has_no_quote() {
    let inv = Inventory::new();
    assert!(inv.hotbar_slot(0).is_none());
    // The game-loop maps a None held stack to no quote — nothing to sell.
}

#[test]
fn full_stack_floor_scales() {
    // A full stack of a tier-0 item still has a meaningful floor.
    let stack = ItemStack { item: Item::Material(MaterialId::Stick), count: 64 };
    let unit = stack.item.trade_value().unwrap();
    assert_eq!(bazaar::sell_quote(&stack), Some(unit * 64));
}
