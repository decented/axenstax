//! C3d run 16 (run-15 review MEDIUM-1, decided) — a second tool of the same
//! kind anywhere in the window never makes a Strict swing or shot a
//! teardown: the numbered Wear resolves to the NAMED held slot when it
//! holds the kind (`joiner_actions::strict_wear_target`, one rule for both),
//! and only then to a single match elsewhere.
use super::c3d_client_attack_tests::{attack, cow_state, durability, fate, held, rig, sword};
use super::c3d_client_edit_tests::joined;
use super::c3d_client_shot_tests::{arrows, bow, fate as shot_fate, projectiles, shoot};
use crate::action_identity::ActionFate;
use crate::crafting::ToolMaterial;
use crate::item::{Item, ItemStack};

/// Two identical iron swords, one held, one in the backpack: the swing is
/// served and the HELD one wears, on both copies; never torn down.
#[test]
fn c3d_strict_swing_with_a_spare_identical_sword_is_served() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, cow, id, at) = rig("wear-two-swords", &[(0, held(iron)), (20, held(iron))], 2.0);
    let (max, _, _) = cow_state(&j.hs, cow);
    let action = attack(&mut j, 0, id);
    super::c3d_client_attack_tests::place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    assert_eq!(cow_state(&j.hs, cow).0, max - Item::Tool(iron).attack_damage(), "the hit landed");
    j.assert_honest("two identical swords");
    assert_eq!((durability(&j.inv, 0), durability(&j.inv, 20)), (Some(iron.durability - 1), Some(iron.durability)), "the held sword wore on the client");
    assert_eq!((durability(&j.sp().inventory, 0), durability(&j.sp().inventory, 20)), (Some(iron.durability - 1), Some(iron.durability)), "and on the server copy");
}

/// Two identical bows, one held, one in the backpack: the shot is served,
/// the HELD bow wears on both copies; never torn down.
#[test]
fn c3d_strict_shot_with_a_spare_identical_bow_is_served() {
    let b = bow();
    let mut j = joined("wear-two-bows", &[(0, ItemStack { item: Item::Tool(b), count: 1 }), (1, arrows(4)), (20, ItemStack { item: Item::Tool(b), count: 1 })], true);
    let shot = shoot(&mut j, 0).expect("typed");
    j.pump();
    assert_eq!(shot_fate(&j, shot), Some(ActionFate::Accepted), "served");
    assert_eq!(projectiles(&j), 1, "one projectile");
    j.assert_honest("two identical bows");
    let wear = |inv: &crate::inventory::Inventory, at: usize| match inv.slot(at).map(|s| &s.item) { Some(Item::Tool(t)) => t.durability, _ => 0 };
    assert_eq!((wear(&j.inv, 0), wear(&j.inv, 20)), (b.durability - 1, b.durability), "the held bow wore on the client");
    assert_eq!((wear(&j.sp().inventory, 0), wear(&j.sp().inventory, 20)), (b.durability - 1, b.durability), "and on the server copy");
}

/// C3d run 17 (run-16 review LOW-5) — `strict_wear_target`'s fallback,
/// pinned: the named slot lacks the kind and exactly one piece elsewhere
/// holds it → that piece; two elsewhere → `None` (ambiguous).
#[test]
fn c3d_wear_target_falls_back_to_the_unique_piece_elsewhere() {
    let iron = Item::Tool(sword(ToolMaterial::Iron));
    let mut inv = crate::inventory::Inventory::new();
    inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
    inv.set_slot(20, Some(held(sword(ToolMaterial::Iron))));
    assert_eq!(crate::joiner_actions::strict_wear_target(&inv, 0, &iron), Some(20), "the unique piece elsewhere");
    inv.set_slot(25, Some(held(sword(ToolMaterial::Iron))));
    assert_eq!(crate::joiner_actions::strict_wear_target(&inv, 0, &iron), None, "two elsewhere: ambiguous");
}
