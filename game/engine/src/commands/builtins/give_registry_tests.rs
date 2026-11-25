//! Coverage of the `/give` registry fallback: every registered block and material can be given by
//! its display name, so a block added to the game is giveable without anyone adding a table row.

use super::give::resolve_item;
use super::give_names::NOT_GIVEABLE_BLOCKS;
use crate::block::{self, BlockRegistry};
use crate::inventory_explorer::ALL_MATERIAL_IDS;
use crate::item::Item;

/// Every item the fallback is answerable for: each registered block but the technical ones, each
/// material, with its display name.
fn everything_giveable(registry: &BlockRegistry) -> Vec<(String, Item)> {
    let blocks = (0..registry.len() as block::BlockId)
        .filter(|id| !NOT_GIVEABLE_BLOCKS.contains(id) && !block::is_reserved(*id))
        .map(Item::Block);
    let materials = ALL_MATERIAL_IDS.iter().map(|&m| Item::Material(m));
    blocks.chain(materials).map(|item| (item.name(registry), item)).collect()
}

#[test]
fn every_registered_block_and_material_resolves_by_its_display_name() {
    let registry = BlockRegistry::new();
    let all = everything_giveable(&registry);
    assert!(all.len() > 500, "only {} giveable items", all.len());
    let mut missing = Vec::new();
    for (label, _) in &all {
        if resolve_item(label, 1).is_err() {
            missing.push(label.clone());
        }
    }
    assert!(missing.is_empty(), "{} items have no /give name by their display name: {missing:?}", missing.len());
}

/// Display names that give a different item than the one they name, and why. Each is an old name
/// the tables keep (pinned by the golden list), or the item form of a block, and the item they
/// name is still reachable: by `OTHER` where one is given.
const SHADOWED: &[(&str, &str)] = &[
    // `campfire` is the unlit one.
    ("Campfire", "campfire_lit"),
    // `mill` is the windmill.
    ("Mill", "grain_mill"),
    // `iron_chest` and its kin are the chestplates.
    ("Iron Chest", "iron_chest_block"),
    ("Diamond Chest", "diamond_chest_block"),
    ("Satori Chest", "satori_chest_block"),
    // `copper`, `amethyst` and `pumpkin` name the block (the ore, the cluster, the plant).
    ("Copper", "raw_copper"),
    ("Amethyst", "amethyst_gem"),
    ("Pumpkin", "pumpkin_food"),
    // The block keeps `copper_cable`; the material is `coppercable`.
    ("Copper Cable", "coppercable"),
];

/// The economy blocks are placed from an item form (`BountyBoardItem` ...), and that item form is
/// what their name gives: nobody holds the bare block.
const ITEM_FORM_BLOCKS: &[&str] = &["Bounty Board", "Tip Jar", "Repair Bench", "Plot Marker", "Market Bell", "Auction Block", "Bazaar Block"];

#[test]
fn a_display_name_gives_that_item_unless_it_is_listed_as_shadowed() {
    let registry = BlockRegistry::new();
    let all = everything_giveable(&registry);
    let mut mismatched = Vec::new();
    for (label, item) in &all {
        let got = resolve_item(label, 1).map(|s| s.item);
        if got.as_ref() == Ok(item) {
            continue;
        }
        let listed = SHADOWED.iter().any(|(l, _)| l == label) || (ITEM_FORM_BLOCKS.contains(&label.as_str()) && matches!(item, Item::Block(_)));
        if !listed {
            mismatched.push(format!("{label}: wanted {item:?}, gave {got:?}"));
        }
    }
    assert!(mismatched.is_empty(), "{} display names give another item:\n{}", mismatched.len(), mismatched.join("\n"));
    // Each shadowed item is given by its other name.
    for (label, other) in SHADOWED {
        let wanted: Vec<&Item> = all.iter().filter(|(l, _)| l == label).map(|(_, i)| i).collect();
        let given = resolve_item(other, 1).unwrap_or_else(|e| panic!("{other}: {e}")).item;
        assert!(wanted.contains(&&given), "{other} gives {given:?}, not an item called {label}");
    }
}

#[test]
fn the_registry_name_works_with_or_without_the_namespace_in_any_case() {
    for typed in ["genesis:spruce_planks", "spruce_planks", "Spruce Planks", "SPRUCE-PLANKS", "spruce planks"] {
        assert_eq!(resolve_item(typed, 1).unwrap().item, Item::Block(block::SPRUCE_PLANKS), "{typed}");
    }
    assert_eq!(resolve_item("spruce planks", 5).unwrap().count, 5);
}

#[test]
fn deepslate_gives_the_pure_deepslate_block() {
    for typed in ["deepslate", "pure deepslate", "pure_deepslate", "Pure Deepslate"] {
        assert_eq!(resolve_item(typed, 1).unwrap().item, Item::Block(block::PURE_DEEPSLATE), "{typed}");
    }
    // Cobbled Deepslate (what mining pure deepslate drops) is its own block, by its own name.
    for typed in ["cobbled deepslate", "cobbled_deepslate", "genesis:cobbled_deepslate"] {
        assert_eq!(resolve_item(typed, 1).unwrap().item, Item::Block(block::COBBLED_DEEPSLATE), "{typed}");
    }
}

#[test]
fn the_technical_blocks_are_not_given_and_the_list_says_why_each_is_there() {
    for id in NOT_GIVEABLE_BLOCKS {
        let name = BlockRegistry::new().get(*id).name;
        assert!(resolve_item(name, 1).is_err(), "{name} must not be giveable by its registry name");
    }
    for typed in ["air", "genesis:air", "piston_head", "worn_path", "feast_marker", "blighted_crop"] {
        assert!(resolve_item(typed, 1).is_err(), "{typed} must not be giveable");
    }
}

#[test]
fn the_ids_held_for_tier_3_are_never_given_or_offered() {
    let registry = BlockRegistry::new();
    let names = super::give::item_names();
    for id in block::RESERVED_FIRST..=block::RESERVED_LAST {
        let name = registry.get(id).name;
        let local = name.rsplit_once(':').map_or(name, |(_, rest)| rest);
        assert!(resolve_item(name, 1).is_err(), "{name} must not be giveable by its registry name");
        assert!(resolve_item(local, 1).is_err(), "{local} must not be giveable");
        assert!(!resolve_item(&Item::Block(id).name(&registry), 1).is_ok_and(|s| s.item == Item::Block(id)), "{id} must not be giveable by its display name");
        assert!(!names.iter().any(|n| n == local), "{local} must not be offered by Tab");
    }
}

#[test]
fn a_genuine_typo_still_gets_a_hint_and_nonsense_still_fails() {
    let err = resolve_item("spruce_plank", 1);
    assert!(err.is_err(), "a one-letter slip is not an exact name");
    let msg = err.unwrap_err();
    assert!(msg.contains("Did you mean 'spruce_planks'"), "{msg}");
    let err = resolve_item("nonsense_thing", 1).unwrap_err();
    assert!(err.starts_with("unknown item: 'nonsense_thing'"), "{err}");
}

#[test]
fn the_names_the_fallback_added_are_suggested_for_tab_completion() {
    let has = |n: &str| super::give::item_names().iter().any(|x| x == n);
    for name in ["spruce_planks", "spruce_log", "spruce_leaves", "pure_deepslate", "granite", "obsidian"] {
        assert!(has(name), "{name} is not offered by Tab");
    }
}
