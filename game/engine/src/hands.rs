//! Hands: which hand a right-click uses, and the swap. Pure rules, no world.
//!
//! The off-hand (`PlayerSlot.offhand`) is Minecraft's second hand: any item can
//! sit there. Right-click uses the main hand (the selected hotbar slot) first;
//! only when the main item has no use for what the player is aiming at does the
//! off-hand item's use run (a torch placed while a pickaxe is held, food eaten
//! while a sword is held). [`use_hand`] is that one decision; the right-click
//! chain in `game_loop` takes it once per click and reads the item of the
//! chosen hand through `PlayerSlot::held_for_use`.
//!
//! What counts as "a use" is [`item_has_use`]: a list of what the chain's arms
//! actually do with a held item, so it must grow with them. It errs toward the
//! main hand: an item the list does not know keeps today's behaviour, so the
//! worst a missing entry costs is an off-hand item that will not fire.
//!
//! Local players only for now: a joined client's use packets do not carry a
//! hand yet, so `game_loop` takes [`Hand::Main`] while joined (slice 2 of the
//! off-hand plan, `docs/` plan file `offhand-plan.md`).

use crate::block::{self, BlockId};
use crate::crafting::ToolType;
use crate::item::{material_as_placeable_block, Item, ItemStack, MaterialId};

/// Which hand an action uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hand {
    /// The selected hotbar slot.
    #[default]
    Main,
    /// `PlayerSlot.offhand`.
    Off,
}

/// What the player is aiming at and what state they are in: everything
/// [`use_hand`] needs besides the two items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UseTarget {
    /// The block under the crosshair (`PlayerSlot::target_block`), if any.
    pub block: Option<BlockId>,
    /// A mob is under the crosshair (feeding, taming, a Lead, milking...).
    pub mob: bool,
    /// Hungry or hurt: food has a use.
    pub wants_to_eat: bool,
    /// The inventory holds an Arrow (a bow can fire).
    pub arrows: bool,
    /// The inventory holds a Rubber Ball (a slingshot can fire).
    pub rubber_balls: bool,
}

/// Blocks whose right-click arm in `game_loop` takes or reads the HELD item
/// whatever it is: a Steam Generator's fuel, a hive's bucket or shears, a
/// Composter's scraps, an Item Frame's item, a campfire's fuel or food, a drying
/// rack's, the Feast Table's stack, and a Track (freight, or an empty hand to
/// ride). Aimed at one, the main hand keeps the click (Minecraft's block use
/// runs with the main hand first), empty or not, so a torch in the off-hand is
/// never fed to a composter or hung in a frame while a pickaxe is held. Add a
/// block here when its arm gains a read of the held item.
pub fn block_takes_held_item(b: BlockId) -> bool {
    matches!(
        b,
        block::STEAM_GENERATOR
            | block::STEAM_GENERATOR_LIT
            | block::BEE_HIVE
            | block::COMPOSTER
            | block::ITEM_FRAME
            | block::CAMPFIRE
            | block::CAMPFIRE_UNLIT
            | block::DRYING_RACK
            | block::FEAST_TABLE
            | crate::rail::TRACK
    )
}

/// The hand a right-click uses: the main hand unless its item has no use for
/// `target` and the off-hand item does (Minecraft's rule). An empty main hand
/// has no use. A block that takes the held item ([`block_takes_held_item`])
/// always keeps the main hand.
pub fn use_hand(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> Hand {
    if target.block.is_some_and(block_takes_held_item) {
        return Hand::Main;
    }
    let main_used = main.is_some_and(|m| item_has_use(m, target));
    if main_used {
        return Hand::Main;
    }
    match off {
        // BRIDGE: a Plan's Inspect dialog is keyed on a hotbar slot, so a Plan
        // in the off-hand never takes the click — replace when the dialog can
        // name the off-hand.
        Some(o) if !matches!(o, Item::Plan(_)) && item_has_use(o, target) => Hand::Off,
        _ => Hand::Main,
    }
}

/// Materials a right-click does something with wherever they are aimed (the
/// right-click chain reads each of them): a Lead, bonemeal and fertiliser, a
/// reed, the buckets, salt, the whistle and the firestarter.
fn is_general_use_material(m: MaterialId) -> bool {
    matches!(
        m,
        MaterialId::Lead
            | MaterialId::Bonemeal
            | MaterialId::Fertiliser
            | MaterialId::PapyrusReed
            | MaterialId::Bucket
            | MaterialId::WaterBucket
            | MaterialId::LavaBucket
            | MaterialId::Salt
            | MaterialId::RecallWhistle
            | MaterialId::MagnesiumFirestarter
    )
}

/// Does right-click do anything with `item` for `target`? See the module doc:
/// this lists the chain's arms and errs toward "yes" for anything it cannot
/// tell, except the ordinary tools and materials that plainly have no use.
pub fn item_has_use(item: &Item, target: &UseTarget) -> bool {
    match item {
        // A placeable block needs something to place against.
        Item::Block(_) => target.block.is_some(),
        Item::Plan(_) => true,
        // Armour is equipped from the inventory screen, not by right-click.
        Item::Armour(_) => false,
        Item::Tool(tool) => match tool.tool_type {
            ToolType::Bow => target.arrows,
            ToolType::Slingshot => target.rubber_balls,
            ToolType::Hoe
            | ToolType::FlintAndSteel
            | ToolType::Shears
            | ToolType::FishingRod
            | ToolType::Eraser
            | ToolType::DraftingStamp => true,
            // Melee and digging tools have no right-click use.
            ToolType::Pickaxe | ToolType::Axe | ToolType::Sword | ToolType::Shovel => false,
        },
        Item::Material(m) => {
            if item.is_food() {
                return target.wants_to_eat || target.mob;
            }
            if is_general_use_material(*m)
                || (target.block.is_some()
                    && (material_as_placeable_block(*m).is_some()
                        || crate::use_edits::sown_crop(*m).is_some()
                        || crate::cart::cart_hull_for_item(*m).is_some()))
            {
                return true;
            }
            // A Stick feeds a campfire; any other material is for a mob
            // (feeding or taming it) or nothing.
            if *m == MaterialId::Stick {
                return matches!(target.block, Some(block::CAMPFIRE | block::CAMPFIRE_UNLIT));
            }
            target.mob
        }
    }
}

/// Swap the main-hand slot and the off-hand (the F key, a tap on the off-hand
/// box). Nothing to swap (both empty) changes nothing. Returns whether
/// anything moved.
pub fn swap(main: &mut Option<ItemStack>, off: &mut Option<ItemStack>) -> bool {
    if main.is_none() && off.is_none() {
        return false;
    }
    std::mem::swap(main, off);
    true
}

/// A click on the off-hand slot in the inventory screen, with the cursor's
/// stack: Minecraft's slot rules, as every inventory slot (`window::click_slot`).
/// Left works on the whole stack (pick up, put down, merge up to the max, or
/// swap); right works on one (pick up the ceil-half, put one down, or add
/// one). Lossless: nothing is created or destroyed. Any item may sit here.
///
/// BRIDGE: this is the window rule for a local player only. A joined client's
/// window is the server's copy, which has no off-hand slot yet — replace with a
/// `WindowClick::Offhand` in `window::apply` when the slot reaches the wire
/// (slice 2 of the off-hand plan).
pub fn click_offhand(off: &mut Option<ItemStack>, cursor: &mut Option<ItemStack>, right: bool) {
    let Some(carried) = cursor.take() else {
        // Empty cursor: pick up. Left = whole, right = ceil-half.
        if let Some(stack) = off.take() {
            if right {
                let take = stack.count / 2 + stack.count % 2;
                let leave = stack.count - take;
                *cursor = Some(ItemStack { item: stack.item.clone(), count: take });
                if leave > 0 {
                    *off = Some(ItemStack { item: stack.item, count: leave });
                }
            } else {
                *cursor = Some(stack);
            }
        }
        return;
    };
    match off.take() {
        // Empty slot: right puts one down, left the whole stack.
        None => {
            if right {
                *off = Some(ItemStack { item: carried.item.clone(), count: 1 });
                if carried.count > 1 {
                    *cursor = Some(ItemStack { item: carried.item, count: carried.count - 1 });
                }
            } else {
                *off = Some(carried);
            }
        }
        // Same item: merge up to the stack max.
        Some(existing) if existing.item.can_stack_with(&carried.item) => {
            let room = carried.item.max_stack().saturating_sub(existing.count);
            let add = if right { carried.count.min(room).min(1) } else { carried.count.min(room) };
            *off = Some(ItemStack { item: carried.item.clone(), count: existing.count + add });
            if add < carried.count {
                *cursor = Some(ItemStack { item: carried.item, count: carried.count - add });
            }
        }
        // A different item: swap (both buttons).
        Some(existing) => {
            *off = Some(carried);
            *cursor = Some(existing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial};

    fn tool(t: ToolType) -> Item {
        Item::Tool(Tool::new(t, ToolMaterial::Iron))
    }

    fn stone_target() -> UseTarget {
        UseTarget { block: Some(block::STONE), ..UseTarget::default() }
    }

    #[test]
    fn a_pickaxe_in_the_main_hand_hands_the_click_to_a_torch_in_the_off_hand() {
        let pick = tool(ToolType::Pickaxe);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&pick), Some(&torch), &stone_target()), Hand::Off);
    }

    #[test]
    fn an_empty_main_hand_hands_the_click_to_the_off_hand() {
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(None, Some(&torch), &stone_target()), Hand::Off);
    }

    #[test]
    fn a_block_in_the_main_hand_places_from_the_main_hand() {
        let stone = Item::Block(block::STONE);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&stone), Some(&torch), &stone_target()), Hand::Main);
    }

    #[test]
    fn food_in_the_main_hand_eats_from_the_main_hand_when_hungry() {
        let bread = Item::Material(MaterialId::Bread);
        assert!(bread.is_food(), "the test needs a real food");
        let torch = Item::Block(block::TORCH);
        let hungry = UseTarget { wants_to_eat: true, ..stone_target() };
        assert_eq!(use_hand(Some(&bread), Some(&torch), &hungry), Hand::Main);
    }

    #[test]
    fn full_and_unhurt_food_has_no_use_so_the_off_hand_goes() {
        let bread = Item::Material(MaterialId::Bread);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&bread), Some(&torch), &stone_target()), Hand::Off);
    }

    #[test]
    fn food_in_the_off_hand_is_eaten_when_the_main_item_has_no_use() {
        let sword = tool(ToolType::Sword);
        let bread = Item::Material(MaterialId::Bread);
        let hungry = UseTarget { wants_to_eat: true, ..UseTarget::default() };
        assert_eq!(use_hand(Some(&sword), Some(&bread), &hungry), Hand::Off);
        // Not hungry: nothing to eat, so the main hand keeps the click.
        assert_eq!(use_hand(Some(&sword), Some(&bread), &UseTarget::default()), Hand::Main);
    }

    #[test]
    fn nothing_in_the_off_hand_means_the_main_hand() {
        let pick = tool(ToolType::Pickaxe);
        assert_eq!(use_hand(Some(&pick), None, &stone_target()), Hand::Main);
        assert_eq!(use_hand(None, None, &stone_target()), Hand::Main);
    }

    #[test]
    fn a_useless_off_hand_item_never_steals_the_click() {
        let pick = tool(ToolType::Pickaxe);
        let ingot = Item::Material(MaterialId::IronIngot);
        assert_eq!(use_hand(Some(&pick), Some(&ingot), &stone_target()), Hand::Main);
        // Aimed at the sky, a torch has nothing to be placed against.
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&pick), Some(&torch), &UseTarget::default()), Hand::Main);
    }

    #[test]
    fn a_bow_needs_an_arrow_to_hold_the_click() {
        let bow = tool(ToolType::Bow);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&bow), Some(&torch), &stone_target()), Hand::Off);
        let armed = UseTarget { arrows: true, ..stone_target() };
        assert_eq!(use_hand(Some(&bow), Some(&torch), &armed), Hand::Main);
    }

    #[test]
    fn a_stick_only_has_a_use_at_a_campfire() {
        let stick = Item::Material(MaterialId::Stick);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&stick), Some(&torch), &stone_target()), Hand::Off);
        let campfire = UseTarget { block: Some(block::CAMPFIRE), ..UseTarget::default() };
        assert_eq!(use_hand(Some(&stick), Some(&torch), &campfire), Hand::Main);
    }

    #[test]
    fn a_lead_keeps_the_click_wherever_it_is_aimed() {
        let lead = Item::Material(MaterialId::Lead);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&lead), Some(&torch), &UseTarget::default()), Hand::Main);
    }

    #[test]
    fn a_mob_under_the_crosshair_makes_a_plain_material_useful() {
        let wheat = Item::Material(MaterialId::Wheat);
        let torch = Item::Block(block::TORCH);
        let cow = UseTarget { mob: true, ..UseTarget::default() };
        assert_eq!(use_hand(Some(&wheat), Some(&torch), &cow), Hand::Main);
    }

    #[test]
    fn a_block_that_takes_the_held_item_keeps_the_main_hand_whatever_it_is() {
        let torch = Item::Block(block::TORCH);
        let pick = tool(ToolType::Pickaxe);
        let sword = tool(ToolType::Sword);
        let bread = Item::Material(MaterialId::Bread);
        let wheat = Item::Material(MaterialId::Wheat);
        for b in [
            block::STEAM_GENERATOR,
            block::STEAM_GENERATOR_LIT,
            block::BEE_HIVE,
            block::COMPOSTER,
            block::ITEM_FRAME,
            block::CAMPFIRE,
            block::CAMPFIRE_UNLIT,
            block::DRYING_RACK,
            block::FEAST_TABLE,
            crate::rail::TRACK,
        ] {
            assert!(block_takes_held_item(b));
            let at = UseTarget { block: Some(b), ..UseTarget::default() };
            for main in [&pick, &sword, &bread, &wheat] {
                assert_eq!(use_hand(Some(main), Some(&torch), &at), Hand::Main, "block {b}, main {main:?}");
            }
            assert_eq!(use_hand(None, Some(&torch), &at), Hand::Main, "block {b}: an empty hand keeps the click (a Track mounts)");
        }
        // Full and unhurt food at a campfire or a Feast Table is not stolen by the torch.
        let at_feast = UseTarget { block: Some(block::FEAST_TABLE), ..UseTarget::default() };
        assert_eq!(use_hand(Some(&bread), Some(&torch), &at_feast), Hand::Main);
        // An ordinary block does not.
        assert!(!block_takes_held_item(block::STONE));
        assert_eq!(use_hand(Some(&pick), Some(&torch), &stone_target()), Hand::Off);
    }

    #[test]
    fn swap_trades_the_two_hands_and_ignores_two_empty_ones() {
        let mut main = Some(ItemStack::new_block(block::STONE, 5));
        let mut off = Some(ItemStack::new_block(block::TORCH, 12));
        assert!(swap(&mut main, &mut off));
        assert_eq!(main, Some(ItemStack::new_block(block::TORCH, 12)));
        assert_eq!(off, Some(ItemStack::new_block(block::STONE, 5)));

        // One empty hand: the item moves across.
        let mut main = None;
        let mut off = Some(ItemStack::new_block(block::TORCH, 3));
        assert!(swap(&mut main, &mut off));
        assert_eq!(main, Some(ItemStack::new_block(block::TORCH, 3)));
        assert_eq!(off, None);

        let (mut a, mut b): (Option<ItemStack>, Option<ItemStack>) = (None, None);
        assert!(!swap(&mut a, &mut b));
    }

    #[test]
    fn the_off_hand_slot_picks_up_puts_down_merges_and_swaps() {
        let stone = |n| Some(ItemStack::new_block(block::STONE, n));
        let torch = |n| Some(ItemStack::new_block(block::TORCH, n));

        // Put a whole stack down on an empty slot; pick it back up.
        let (mut off, mut cur) = (None, stone(10));
        click_offhand(&mut off, &mut cur, false);
        assert_eq!((off.clone(), cur.clone()), (stone(10), None));
        click_offhand(&mut off, &mut cur, false);
        assert_eq!((off.clone(), cur.clone()), (None, stone(10)));

        // Right-click: one down, then one more added; pick up the half.
        let (mut off, mut cur) = (None, stone(10));
        click_offhand(&mut off, &mut cur, true);
        assert_eq!((off.clone(), cur.clone()), (stone(1), stone(9)));
        click_offhand(&mut off, &mut cur, true);
        assert_eq!((off.clone(), cur.clone()), (stone(2), stone(8)));
        let (mut off, mut cur) = (stone(7), None);
        click_offhand(&mut off, &mut cur, true);
        assert_eq!((off.clone(), cur.clone()), (stone(3), stone(4)), "the cursor takes the larger half");

        // Merge past the stack max leaves the remainder on the cursor.
        let max = ItemStack::new_block(block::STONE, 1).item.max_stack();
        let (mut off, mut cur) = (stone(max - 2), stone(5));
        click_offhand(&mut off, &mut cur, false);
        assert_eq!((off.clone(), cur.clone()), (stone(max), stone(3)));

        // A different item swaps, losing nothing.
        let (mut off, mut cur) = (torch(4), stone(6));
        click_offhand(&mut off, &mut cur, false);
        assert_eq!((off, cur), (stone(6), torch(4)));

        // Nothing on either side stays nothing.
        let (mut off, mut cur): (Option<ItemStack>, Option<ItemStack>) = (None, None);
        click_offhand(&mut off, &mut cur, false);
        assert_eq!((off, cur), (None, None));
    }
}
