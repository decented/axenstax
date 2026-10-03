//! Spec 35 Tool Repair integration tests.
//!
//! Exercises the inventory material-count/consume helpers + the
//! repair quote→apply flow end-to-end over a real Inventory. Pure
//! quote/restore math is covered in `src/repair.rs`.

use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::repair;

#[test]
fn count_material_sums_across_slots() {
    let mut inv = Inventory::new();
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 5 });
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 3 });
    inv.add_item(ItemStack { item: Item::Material(MaterialId::Diamond), count: 2 });
    assert_eq!(inv.count_material(MaterialId::IronIngot), 8);
    assert_eq!(inv.count_material(MaterialId::Diamond), 2);
    assert_eq!(inv.count_material(MaterialId::Satori), 0);
}

#[test]
fn consume_material_removes_exact_count_across_stacks() {
    let mut inv = Inventory::new();
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 5 });
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 5 });
    let consumed = inv.consume_material(MaterialId::IronIngot, 7);
    assert_eq!(consumed, 7);
    assert_eq!(inv.count_material(MaterialId::IronIngot), 3);
}

#[test]
fn consume_material_caps_at_available() {
    let mut inv = Inventory::new();
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 2 });
    let consumed = inv.consume_material(MaterialId::IronIngot, 10);
    assert_eq!(consumed, 2, "can't consume more than available");
    assert_eq!(inv.count_material(MaterialId::IronIngot), 0);
}

#[test]
fn full_repair_flow_restores_tool_and_consumes_iron() {
    // A half-broken iron pickaxe + 5 iron ingots → full repair.
    let mut inv = Inventory::new();
    let mut tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
    tool.durability = 0; // fully broken (max 250)
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 5 });

    let mat = repair::repair_material_for(&tool).expect("iron tool repairs with iron");
    let have = inv.count_material(mat);
    let quote = repair::repair_quote(tool.durability, tool.max_durability(), have);
    // 250 missing, 5 ingots × 50 = 250 → full repair, 5 consumed.
    assert_eq!(quote.durability_restored, 250);
    assert_eq!(quote.material_consumed, 5);

    let consumed = inv.consume_material(mat, quote.material_consumed);
    assert_eq!(consumed, 5);
    repair::apply_repair(&mut tool, &quote);
    assert_eq!(tool.durability, tool.max_durability());
    assert_eq!(inv.count_material(MaterialId::IronIngot), 0);
}

#[test]
fn partial_repair_when_short_on_material() {
    // Broken iron pickaxe + only 2 ingots → partial (100 of 250).
    let mut inv = Inventory::new();
    let mut tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
    tool.durability = 0;
    inv.add_item(ItemStack { item: Item::Material(MaterialId::IronIngot), count: 2 });

    let mat = repair::repair_material_for(&tool).unwrap();
    let have = inv.count_material(mat);
    let quote = repair::repair_quote(tool.durability, tool.max_durability(), have);
    assert_eq!(quote.durability_restored, 100); // 2 × 50
    assert_eq!(quote.material_consumed, 2);

    inv.consume_material(mat, quote.material_consumed);
    repair::apply_repair(&mut tool, &quote);
    assert_eq!(tool.durability, 100);
    assert_eq!(inv.count_material(MaterialId::IronIngot), 0);
}

#[test]
fn repair_never_exceeds_max_durability() {
    // Nearly-full diamond pickaxe; plenty of diamonds → cap at max.
    let mut inv = Inventory::new();
    let mut tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond);
    let max = tool.max_durability();
    tool.durability = max - 10; // missing 10
    inv.add_item(ItemStack { item: Item::Material(MaterialId::Diamond), count: 10 });

    let mat = repair::repair_material_for(&tool).unwrap();
    let have = inv.count_material(mat);
    let quote = repair::repair_quote(tool.durability, max, have);
    assert_eq!(quote.durability_restored, 10, "only restore the missing 10");
    assert_eq!(quote.material_consumed, 1, "1 diamond covers 50 > 10");

    inv.consume_material(mat, quote.material_consumed);
    repair::apply_repair(&mut tool, &quote);
    assert_eq!(tool.durability, max);
    assert_eq!(inv.count_material(MaterialId::Diamond), 9, "only 1 consumed");
}

#[test]
fn wood_and_stone_tools_have_no_repair_material() {
    let wood = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
    let stone = Tool::new(ToolType::Axe, ToolMaterial::Stone);
    assert_eq!(repair::repair_material_for(&wood), None);
    assert_eq!(repair::repair_material_for(&stone), None);
}
