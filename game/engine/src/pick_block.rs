//! Pick block (Spec 05 §8.2): middle-click the block under the crosshair and
//! its item lands in your hand.
//!
//! Two pure halves, no `GameState`:
//!
//! - [`pick_item_for`] — a block's item form. Built on the mappings the
//!   engine already has rather than a parallel table: the placement inverse
//!   (`item::material_as_placeable_block`, `use_edits::sown_crop`, the
//!   papyrus reed) run backwards, and the normalisations the break drop
//!   already makes (`BlockRegistry::mine_drop`: lit campfire to unlit,
//!   turning wheel to idle, deepslate variants to the canonical block).
//!   Where the mine drop is the *raw* thing (ore to coal, log to green log)
//!   the pick wants the *placeable* thing, so those return the block.
//! - [`plan_pick`] — what to do to the inventory, per play mode: a
//!   [`PickPlan`] over the hotbar and bag. `game_loop` applies it through the
//!   same paths the inventory screen uses (window clicks for the survival
//!   swap, the creative give for the creative take), so a joiner behaves
//!   exactly as if it had done it by hand in the UI.

use crate::block::{self, BlockId};
use crate::inventory::Inventory;
use crate::item::{material_as_placeable_block, Item, ItemStack, MaterialId};
use crate::play_mode::PlayMode;
use crate::window::{BAG_START, SLOTS};

/// Hotbar slots (inventory slots `0..HOTBAR`; the bag is `BAG_START..SLOTS`).
const HOTBAR: usize = BAG_START;

/// Every seed `use_edits::sown_crop` sows into tilled soil. A table test
/// checks no material outside this list sows, so it cannot drift.
const SOWN_SEEDS: [MaterialId; 9] = [
    MaterialId::WheatSeeds,
    MaterialId::Carrot,
    MaterialId::Potato,
    MaterialId::CornSeeds,
    MaterialId::CottonSeeds,
    MaterialId::HempSeeds,
    MaterialId::CornflowerSeeds,
    MaterialId::FieldPoppySeeds,
    MaterialId::ButtercupSeeds,
];

/// Blocks that deliberately pick nothing, each with its reason. Crop families
/// with no planting item are covered by [`no_planting_item`].
pub const PICKS_NOTHING: &[(BlockId, &str)] = &[
    (block::AIR, "nothing there"),
    (block::WATER, "a fluid: buckets scoop it, nothing places it as a block item"),
    (block::LAVA, "a fluid: buckets scoop it, nothing places it as a block item"),
    (block::FIRE, "no item form: it is punched out, never carried"),
    (block::CAMPFIRE_SMOKE, "a render-only smoke pillar, not a block anyone holds"),
    (block::BRIGAND_HIDEOUT_BANNER, "a hideout marker whose identity lives in a world side-table; a carried copy would be a forgery"),
    (block::PISTON_HEAD, "the extended arm is transient and does not say plain or sticky: aim at the piston body"),
    (block::LATENT_PRINT, "a developing print placed from a Plan, which carries per-instance data a bare item cannot"),
    (block::CYANOTYPE_PRINT, "a print hung from a Plan, which carries per-instance data a bare item cannot"),
];

/// The farm and worldgen crops that have no planting item: sugar beet,
/// beetroot, pumpkin stems and berry bushes are placed by worldgen and grown
/// in place (`sown_crop` has no seed for them, and their "seeds" are never
/// sown), so no item places them back.
fn no_planting_item(b: BlockId) -> bool {
    (block::SUGAR_BEET_STAGE_0..=block::BEETROOT_STAGE_3).contains(&b)
        || (block::PUMPKIN_STEM_0..=block::PUMPKIN_STEM_4).contains(&b)
        || (block::BERRY_BUSH_0..=block::BERRY_BUSH_3).contains(&b)
}

/// Why `b` picks nothing, or `None` if it picks an item.
pub fn picks_nothing_reason(b: BlockId) -> Option<&'static str> {
    if let Some((_, why)) = PICKS_NOTHING.iter().find(|(id, _)| *id == b) {
        return Some(why);
    }
    no_planting_item(b).then_some("a worldgen crop with no planting item: nothing places it back")
}

/// What placing `item` makes: the placement inverse the tests and the
/// survival matcher share. `Block` places itself; a material places as the
/// block `material_as_placeable_block` names, the crop its seed sows
/// (`use_edits::sown_crop`), or papyrus' first stage.
pub fn places_as(item: &Item) -> Option<BlockId> {
    match item {
        Item::Block(b) => Some(*b),
        Item::Material(MaterialId::PapyrusReed) => Some(block::PAPYRUS_STAGE_0),
        Item::Material(m) => material_as_placeable_block(*m).or_else(|| crate::use_edits::sown_crop(*m)),
        Item::Tool(_) | Item::Plan(_) | Item::Armour(_) => None,
    }
}

/// The idle/unlit/canonical block a variant is carried as. The normalisations
/// the break drop already makes (campfire, furnace, wheels, windmill,
/// deepslate, tapped rubber log) plus the lit electrical twins that drop does
/// not fold.
pub fn canonical_block(b: BlockId) -> BlockId {
    match b {
        block::CAMPFIRE => block::CAMPFIRE_UNLIT,
        block::FURNACE_LIT => block::FURNACE,
        block::WATER_WHEEL_TURNING => block::WATER_WHEEL,
        block::WINDMILL_TURNING => block::WINDMILL,
        block::PURE_DEEPSLATE_THIN | block::PURE_DEEPSLATE_HEALTHY | block::PURE_DEEPSLATE_FAT => block::PURE_DEEPSLATE,
        block::RUBBER_LOG_TAPPED => block::RUBBER_LOG,
        block::CABLE_LIT => block::CABLE,
        block::ELECTRIC_LAMP_LIT => block::ELECTRIC_LAMP,
        block::STEAM_GENERATOR_LIT => block::STEAM_GENERATOR,
        other => other,
    }
}

/// The seed (or plantable item) of a crop stage block, by walking each
/// sowable crop's growth ladder (`growth::next_stage`) from its first stage.
/// The wild flowers double as their crop's last stage; they are placeable
/// blocks in their own right, so [`pick_item_for`] handles them first.
fn crop_item(b: BlockId) -> Option<Item> {
    let ladder_holds = |first: BlockId| {
        let mut at = first;
        loop {
            if at == b {
                return true;
            }
            match crate::growth::next_stage(at) {
                Some(next) => at = next,
                None => return false,
            }
        }
    };
    if ladder_holds(block::PAPYRUS_STAGE_0) {
        return Some(Item::Material(MaterialId::PapyrusReed));
    }
    SOWN_SEEDS
        .iter()
        .find(|&&m| crate::use_edits::sown_crop(m).is_some_and(ladder_holds))
        .map(|&m| Item::Material(m))
}

/// The material that places as `b` (the economy-block items and saplings are
/// stored as materials), if any. The log materials place as an oak log but an
/// oak log is itself a placeable block, so they are alternates, not the pick
/// ([`alternates_for`]).
fn placing_material(b: BlockId) -> Option<MaterialId> {
    if b == block::OAK_LOG {
        return None;
    }
    crate::inventory_explorer::ALL_MATERIAL_IDS
        .iter()
        .copied()
        .find(|&m| material_as_placeable_block(m) == Some(b))
}

/// The item that places `block` back: its item form. `None` for the blocks
/// in [`PICKS_NOTHING`] and the crops with no planting item.
///
/// Examples: a wheat crop gives wheat seeds, papyrus its reed, a lit cable
/// the cable, a campfire the unlit campfire, a sapling its sapling item, ore
/// the ore block (not its drop), leaves the leaves.
pub fn pick_item_for(b: BlockId) -> Option<Item> {
    if picks_nothing_reason(b).is_some() {
        return None;
    }
    // The wild flowers are crop stages too, but they place as themselves.
    if !matches!(b, block::CORNFLOWER | block::FIELD_POPPY | block::BUTTERCUP)
        && let Some(seed) = crop_item(b)
    {
        return Some(seed);
    }
    if let Some(m) = placing_material(b) {
        return Some(Item::Material(m));
    }
    Some(Item::Block(canonical_block(b)))
}

/// Other items that place as the same block as `item`, for the survival
/// search: an oak log is carried as green, seasoned or kiln-dried log
/// material (they place as an oak log), so looking at a log with only green
/// logs in the bag still picks. Empty for most items.
pub fn alternates_for(item: &Item) -> Vec<Item> {
    let Some(target) = places_as(item) else { return Vec::new() };
    crate::inventory_explorer::ALL_MATERIAL_IDS
        .iter()
        .map(|&m| Item::Material(m))
        .filter(|candidate| candidate != item && places_as(candidate) == Some(target))
        .collect()
}

/// What a pick does to the inventory.
#[derive(Clone, Debug, PartialEq)]
pub enum PickPlan {
    /// Nothing happens: spectator, no pickable block, or (survival) the item
    /// is nowhere in the inventory.
    Nothing,
    /// The item is already in the hotbar: select that slot.
    Select(usize),
    /// Survival: the item is in the bag at `bag_slot`; swap it with the
    /// selected hotbar slot (the old hotbar item goes where it came from).
    Swap { bag_slot: usize },
    /// Creative: put `stack` into hotbar `slot` and select it. `replaces` is
    /// true when the slot held something else (a full hotbar).
    Give { slot: usize, stack: ItemStack, replaces: bool },
}

fn first_slot(inv: &Inventory, range: std::ops::Range<usize>, matches: impl Fn(&Item) -> bool) -> Option<usize> {
    range.into_iter().find(|&i| inv.slot(i).is_some_and(|s| s.count > 0 && matches(&s.item)))
}

/// Survival and Adventure: the picked item must already be carried. Prefers
/// the selected slot, then the lowest hotbar slot, then the lowest bag slot.
/// Items that place as the same block count (`alternates_for`), the exact
/// item first.
fn plan_survival(inv: &Inventory, selected: usize, item: &Item) -> PickPlan {
    let mut wanted = vec![item.clone()];
    wanted.extend(alternates_for(item));
    for want in &wanted {
        let is = |held: &Item| held == want;
        if first_slot(inv, selected..selected + 1, is).is_some() {
            return PickPlan::Select(selected);
        }
        if let Some(slot) = first_slot(inv, 0..HOTBAR, is) {
            return PickPlan::Select(slot);
        }
        if let Some(bag_slot) = first_slot(inv, BAG_START..SLOTS, is) {
            return PickPlan::Swap { bag_slot };
        }
    }
    PickPlan::Nothing
}

/// Creative: already in the hotbar, select it; else a full stack into the
/// selected slot if empty; else into the first empty hotbar slot; else (full
/// hotbar) replace the selected slot.
fn plan_creative(inv: &Inventory, selected: usize, item: &Item) -> PickPlan {
    if let Some(slot) = first_slot(inv, selected..selected + 1, |h| h == item).or_else(|| first_slot(inv, 0..HOTBAR, |h| h == item)) {
        return PickPlan::Select(slot);
    }
    let stack = ItemStack { item: item.clone(), count: item.max_stack() };
    if inv.slot(selected).is_none() {
        return PickPlan::Give { slot: selected, stack, replaces: false };
    }
    if let Some(slot) = (0..HOTBAR).find(|&i| inv.slot(i).is_none()) {
        return PickPlan::Give { slot, stack, replaces: false };
    }
    PickPlan::Give { slot: selected, stack, replaces: true }
}

/// The inventory change for picking `item` in `mode` with hotbar slot
/// `selected` held. Spectator never picks.
pub fn plan_pick(mode: PlayMode, inv: &Inventory, selected: usize, item: &Item) -> PickPlan {
    let selected = selected.min(HOTBAR - 1);
    match mode {
        PlayMode::Spectator => PickPlan::Nothing,
        PlayMode::Creative => plan_creative(inv, selected, item),
        PlayMode::Survival | PlayMode::Adventure => plan_survival(inv, selected, item),
    }
}

/// [`plan_pick`] for the block under the crosshair.
pub fn plan_pick_block(mode: PlayMode, inv: &Inventory, selected: usize, aimed: BlockId) -> PickPlan {
    match pick_item_for(aimed) {
        Some(item) => plan_pick(mode, inv, selected, &item),
        None => PickPlan::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockRegistry;

    fn stone(n: u8) -> Option<ItemStack> {
        Some(ItemStack::new_block(block::STONE, n))
    }
    fn dirt(n: u8) -> Option<ItemStack> {
        Some(ItemStack::new_block(block::DIRT, n))
    }
    fn glass(n: u8) -> Option<ItemStack> {
        Some(ItemStack::new_block(block::GLASS, n))
    }
    fn item(b: BlockId) -> Item {
        Item::Block(b)
    }

    /// The block family a block shares with what places it back: its
    /// canonical form (lit to unlit, tapped to plain, …) and, for a crop
    /// stage, the first stage of its ladder.
    fn family(b: BlockId) -> BlockId {
        let b = canonical_block(b);
        if matches!(b, block::CORNFLOWER | block::FIELD_POPPY | block::BUTTERCUP) {
            return b;
        }
        let mut first = b;
        // Walk back: the stage whose successor is `b`.
        for _ in 0..8 {
            let prev = (0..400u16).find(|&p| crate::growth::next_stage(p) == Some(first));
            match prev {
                Some(p) => first = p,
                None => break,
            }
        }
        first
    }

    // ── Point 2: block to item ──────────────────────────────────────────

    #[test]
    fn table_every_registered_block_picks_an_item_that_places_back_or_has_a_reason() {
        let reg = BlockRegistry::new();
        let mut picked = 0;
        let mut nothing = 0;
        for id in 0..reg.len() as BlockId {
            match (pick_item_for(id), picks_nothing_reason(id)) {
                (Some(it), None) => {
                    let placed = places_as(&it).unwrap_or_else(|| panic!("{} picks {it:?}, which places nothing", reg.get(id).name));
                    assert_eq!(
                        family(placed),
                        family(id),
                        "{} picks {it:?}, which places {} instead",
                        reg.get(id).name,
                        reg.get(placed).name
                    );
                    picked += 1;
                }
                (None, Some(why)) => {
                    assert!(!why.is_empty());
                    nothing += 1;
                }
                (a, b) => panic!("{}: pick {a:?} disagrees with reason {b:?}", reg.get(id).name),
            }
        }
        assert!(picked > 280, "the table covered {picked} picking blocks");
        assert_eq!(nothing, PICKS_NOTHING.len() + 4 + 4 + 5 + 4, "the picks-nothing set is exactly the listed blocks plus the four no-planting-item crops");
    }

    #[test]
    fn every_listed_nothing_block_is_registered() {
        let reg = BlockRegistry::new();
        for (id, why) in PICKS_NOTHING {
            assert!((*id as usize) < reg.len(), "{why}");
        }
    }

    #[test]
    fn sowable_seeds_are_exactly_the_listed_ones() {
        for &m in crate::inventory_explorer::ALL_MATERIAL_IDS {
            assert_eq!(
                crate::use_edits::sown_crop(m).is_some(),
                SOWN_SEEDS.contains(&m),
                "{m:?}: SOWN_SEEDS drifted from use_edits::sown_crop"
            );
        }
    }

    #[test]
    fn crops_give_their_seed_at_every_stage() {
        use block::*;
        for (stages, want) in [
            (vec![WHEAT_STAGE_0, WHEAT_STAGE_1, WHEAT_STAGE_2, WHEAT_STAGE_3], Item::Material(MaterialId::WheatSeeds)),
            (vec![CARROT_STAGE_0, CARROT_STAGE_3], Item::Material(MaterialId::Carrot)),
            (vec![POTATO_STAGE_0, POTATO_STAGE_3], Item::Material(MaterialId::Potato)),
            (vec![CORN_STAGE_1, CORN_STAGE_3], Item::Material(MaterialId::CornSeeds)),
            (vec![COTTON_STAGE_2, COTTON_STAGE_3], Item::Material(MaterialId::CottonSeeds)),
            (vec![HEMP_STAGE_0, HEMP_STAGE_3], Item::Material(MaterialId::HempSeeds)),
            (vec![PAPYRUS_STAGE_0, PAPYRUS_STAGE_3], Item::Material(MaterialId::PapyrusReed)),
            (vec![CORNFLOWER_STAGE_0, CORNFLOWER_STAGE_2], Item::Material(MaterialId::CornflowerSeeds)),
            (vec![FIELD_POPPY_STAGE_1], Item::Material(MaterialId::FieldPoppySeeds)),
            (vec![BUTTERCUP_STAGE_0], Item::Material(MaterialId::ButtercupSeeds)),
        ] {
            for s in stages {
                assert_eq!(pick_item_for(s), Some(want.clone()), "stage block {s}");
            }
        }
    }

    #[test]
    fn mature_wild_flowers_pick_the_flower_block() {
        for f in [block::CORNFLOWER, block::FIELD_POPPY, block::BUTTERCUP] {
            assert_eq!(pick_item_for(f), Some(item(f)));
        }
    }

    #[test]
    fn variants_pick_the_idle_form() {
        for (variant, idle) in [
            (block::CABLE_LIT, block::CABLE),
            (block::ELECTRIC_LAMP_LIT, block::ELECTRIC_LAMP),
            (block::STEAM_GENERATOR_LIT, block::STEAM_GENERATOR),
            (block::CAMPFIRE, block::CAMPFIRE_UNLIT),
            (block::FURNACE_LIT, block::FURNACE),
            (block::WATER_WHEEL_TURNING, block::WATER_WHEEL),
            (block::WINDMILL_TURNING, block::WINDMILL),
            (block::PURE_DEEPSLATE_FAT, block::PURE_DEEPSLATE),
            (block::RUBBER_LOG_TAPPED, block::RUBBER_LOG),
        ] {
            assert_eq!(pick_item_for(variant), Some(item(idle)));
        }
    }

    #[test]
    fn canonical_blocks_agree_with_the_break_drop_where_it_normalises() {
        // The pick reuses the break drop's own normalisations. Where the drop
        // folds a variant into a block, the canonical form is that block.
        let reg = BlockRegistry::new();
        for variant in [
            block::CAMPFIRE,
            block::FURNACE_LIT,
            block::WATER_WHEEL_TURNING,
            block::WINDMILL_TURNING,
            block::PURE_DEEPSLATE_THIN,
            block::PURE_DEEPSLATE_HEALTHY,
            block::PURE_DEEPSLATE_FAT,
        ] {
            assert_eq!(reg.mine_drop(variant).item, item(canonical_block(variant)), "{}", reg.get(variant).name);
        }
    }

    #[test]
    fn ore_logs_and_leaves_pick_the_block_not_the_drop() {
        for b in [
            block::COAL_ORE,
            block::IRON_ORE,
            block::DIAMOND_ORE,
            block::DEEPSLATE_IRON_ORE,
            block::COPPER_ORE,
            block::ROCK_SALT,
            block::OAK_LOG,
            block::BIRCH_LOG,
            block::OAK_LEAVES,
            block::STONE,
            block::SALT_PATH,
        ] {
            assert_eq!(pick_item_for(b), Some(item(b)));
        }
    }

    #[test]
    fn saplings_and_economy_blocks_pick_their_material() {
        assert_eq!(pick_item_for(block::SAPLING_BIRCH), Some(Item::Material(MaterialId::BirchSapling)));
        assert_eq!(pick_item_for(block::BOUNTY_BOARD), Some(Item::Material(MaterialId::BountyBoardItem)));
        assert_eq!(pick_item_for(block::TIP_JAR), Some(Item::Material(MaterialId::TipJarItem)));
    }

    #[test]
    fn doors_beds_and_torches_pick_themselves_whatever_their_state() {
        // A door's halves, a bed's halves and a wall torch are one block id
        // each (the half or facing is placement metadata), so the block IS
        // the item form.
        for b in [block::OAK_DOOR, block::BED, block::TORCH] {
            assert_eq!(pick_item_for(b), Some(item(b)));
        }
    }

    #[test]
    fn air_fluids_fire_and_planting_less_crops_pick_nothing() {
        for b in [
            block::AIR,
            block::WATER,
            block::LAVA,
            block::FIRE,
            block::CAMPFIRE_SMOKE,
            block::SUGAR_BEET_STAGE_3,
            block::BEETROOT_STAGE_0,
            block::PUMPKIN_STEM_4,
            block::BERRY_BUSH_3,
        ] {
            assert_eq!(pick_item_for(b), None, "block {b}");
        }
    }

    #[test]
    fn oak_log_has_the_log_materials_as_alternates() {
        let alts = alternates_for(&item(block::OAK_LOG));
        for m in [MaterialId::GreenLog, MaterialId::SeasonedLog, MaterialId::KilnDriedLog] {
            assert!(alts.contains(&Item::Material(m)), "{m:?}");
        }
        assert!(alternates_for(&item(block::STONE)).is_empty());
    }

    // ── Point 3: survival and adventure ────────────────────────────────

    #[test]
    fn survival_item_already_in_the_hotbar_selects_that_slot() {
        let mut inv = Inventory::new();
        inv.set_slot(2, stone(5));
        inv.set_slot(5, dirt(5));
        for mode in [PlayMode::Survival, PlayMode::Adventure] {
            assert_eq!(plan_pick(mode, &inv, 0, &item(block::STONE)), PickPlan::Select(2));
            assert_eq!(plan_pick(mode, &inv, 5, &item(block::DIRT)), PickPlan::Select(5));
        }
    }

    #[test]
    fn survival_bag_item_swaps_with_the_selected_slot() {
        let mut inv = Inventory::new();
        inv.set_slot(0, dirt(3));
        inv.set_slot(20, stone(9));
        assert_eq!(plan_pick(PlayMode::Survival, &inv, 0, &item(block::STONE)), PickPlan::Swap { bag_slot: 20 });
    }

    #[test]
    fn survival_item_nowhere_does_nothing() {
        let mut inv = Inventory::new();
        inv.set_slot(0, dirt(3));
        assert_eq!(plan_pick(PlayMode::Survival, &inv, 0, &item(block::STONE)), PickPlan::Nothing);
        assert_eq!(plan_pick_block(PlayMode::Survival, &inv, 0, block::STONE), PickPlan::Nothing);
    }

    #[test]
    fn survival_prefers_the_hotbar_over_the_bag() {
        let mut inv = Inventory::new();
        inv.set_slot(20, stone(9));
        inv.set_slot(7, stone(1));
        assert_eq!(plan_pick(PlayMode::Survival, &inv, 0, &item(block::STONE)), PickPlan::Select(7));
    }

    #[test]
    fn survival_swap_into_an_empty_selected_slot() {
        let mut inv = Inventory::new();
        inv.set_slot(30, glass(2));
        assert_eq!(plan_pick(PlayMode::Survival, &inv, 4, &item(block::GLASS)), PickPlan::Swap { bag_slot: 30 });
    }

    #[test]
    fn survival_green_logs_answer_for_an_oak_log() {
        let mut inv = Inventory::new();
        inv.set_slot(12, Some(ItemStack::new_material(MaterialId::GreenLog, 4)));
        assert_eq!(plan_pick_block(PlayMode::Survival, &inv, 0, block::OAK_LOG), PickPlan::Swap { bag_slot: 12 });
        // The exact item still wins over an alternate.
        inv.set_slot(30, Some(ItemStack::new_block(block::OAK_LOG, 2)));
        assert_eq!(plan_pick_block(PlayMode::Survival, &inv, 0, block::OAK_LOG), PickPlan::Swap { bag_slot: 30 });
    }

    #[test]
    fn survival_a_non_pickable_block_does_nothing_even_with_items_about() {
        let mut inv = Inventory::new();
        inv.set_slot(0, stone(1));
        assert_eq!(plan_pick_block(PlayMode::Survival, &inv, 0, block::WATER), PickPlan::Nothing);
        assert_eq!(plan_pick_block(PlayMode::Survival, &inv, 0, block::AIR), PickPlan::Nothing);
    }

    // ── Point 4: creative ──────────────────────────────────────────────

    #[test]
    fn creative_item_in_the_hotbar_selects_it() {
        let mut inv = Inventory::new();
        inv.set_slot(3, stone(1));
        assert_eq!(plan_pick(PlayMode::Creative, &inv, 0, &item(block::STONE)), PickPlan::Select(3));
    }

    #[test]
    fn creative_gives_a_full_stack_to_an_empty_selected_slot() {
        let inv = Inventory::new();
        assert_eq!(
            plan_pick(PlayMode::Creative, &inv, 4, &item(block::STONE)),
            PickPlan::Give { slot: 4, stack: ItemStack::new_block(block::STONE, 64), replaces: false }
        );
    }

    #[test]
    fn creative_moves_to_the_first_empty_hotbar_slot_when_the_selected_one_is_taken() {
        let mut inv = Inventory::new();
        inv.set_slot(0, dirt(1));
        inv.set_slot(1, dirt(1));
        assert_eq!(
            plan_pick(PlayMode::Creative, &inv, 0, &item(block::STONE)),
            PickPlan::Give { slot: 2, stack: ItemStack::new_block(block::STONE, 64), replaces: false }
        );
    }

    #[test]
    fn creative_with_a_full_hotbar_replaces_the_selected_slot() {
        let mut inv = Inventory::new();
        for i in 0..9 {
            inv.set_slot(i, dirt(1));
        }
        assert_eq!(
            plan_pick(PlayMode::Creative, &inv, 6, &item(block::STONE)),
            PickPlan::Give { slot: 6, stack: ItemStack::new_block(block::STONE, 64), replaces: true }
        );
    }

    #[test]
    fn creative_ignores_the_bag_and_wants_the_exact_item() {
        let mut inv = Inventory::new();
        inv.set_slot(20, stone(9));
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::GreenLog, 4)));
        assert!(matches!(plan_pick(PlayMode::Creative, &inv, 1, &item(block::STONE)), PickPlan::Give { slot: 1, .. }));
        assert!(matches!(plan_pick_block(PlayMode::Creative, &inv, 1, block::OAK_LOG), PickPlan::Give { slot: 1, .. }));
    }

    #[test]
    fn creative_a_crop_gives_a_full_stack_of_seeds() {
        let inv = Inventory::new();
        assert_eq!(
            plan_pick_block(PlayMode::Creative, &inv, 0, block::WHEAT_STAGE_2),
            PickPlan::Give { slot: 0, stack: ItemStack::new_material(MaterialId::WheatSeeds, 64), replaces: false }
        );
    }

    // ── Point 5: spectator ─────────────────────────────────────────────

    #[test]
    fn spectator_never_picks() {
        let mut inv = Inventory::new();
        inv.set_slot(0, stone(1));
        assert_eq!(plan_pick_block(PlayMode::Spectator, &inv, 0, block::STONE), PickPlan::Nothing);
        assert_eq!(plan_pick_block(PlayMode::Spectator, &Inventory::new(), 0, block::DIRT), PickPlan::Nothing);
    }

    // ── The survival swap, run through the real inventory window ───────

    /// Apply the survival swap as `game_loop` does: three left clicks on the
    /// real window rule (bag slot, hotbar slot, bag slot again).
    fn swap_through_the_window(inv: &mut Inventory, selected: usize, bag_slot: usize) {
        use crate::window::WindowClick;
        let mut ui = crate::craft_ui::CraftingUi::new();
        let mut armour = [None; 4];
        let mut click = |ui: &mut crate::craft_ui::CraftingUi, slot: usize| {
            ui.apply_click(inv, &mut armour, &WindowClick::Slot { slot, right: false }, false, glam::Vec3::ZERO, |_| block::AIR);
        };
        click(&mut ui, bag_slot);
        click(&mut ui, selected);
        // An empty selected slot takes the stack outright: nothing left to put back.
        if ui.cursor_item.is_some() {
            click(&mut ui, bag_slot);
        }
        assert!(ui.cursor_item.is_none(), "the cursor ends empty");
    }

    #[test]
    fn the_swap_sends_the_old_hotbar_item_to_where_the_picked_one_came_from() {
        let mut inv = Inventory::new();
        inv.set_slot(3, dirt(7));
        inv.set_slot(22, stone(9));
        swap_through_the_window(&mut inv, 3, 22);
        assert_eq!(inv.slot(3), stone(9).as_ref());
        assert_eq!(inv.slot(22), dirt(7).as_ref());
    }

    #[test]
    fn the_swap_into_an_empty_selected_slot_leaves_the_bag_slot_empty() {
        let mut inv = Inventory::new();
        inv.set_slot(22, stone(9));
        swap_through_the_window(&mut inv, 3, 22);
        assert_eq!(inv.slot(3), stone(9).as_ref());
        assert!(inv.slot(22).is_none());
    }
}
