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
//! What counts as "a use" is [`item_has_use`]: the question "does this item do
//! anything for THIS target", answered by the same per-tool predicates the chain's
//! arms test (`tool_uses`: a hoe tills only the top of dirt or grass, shears cut
//! only a sheep, and so on), so the two cannot drift. It errs toward the main hand:
//! an item the list does not know keeps today's behaviour, so the worst a missing
//! entry costs is an off-hand item that will not fire.
//!
//! [`hand_with_use`] is the decision as an `Option<Hand>` (which hand, if either, has
//! a use for the target); [`use_hand`] and [`either_hand_has_use`] are its two
//! readings, the second for anything that should stand down when a hand's item has
//! a use (a shield is not raised against a door or a mob).
//!
//! Since v101 a joined Mirror client's use packets carry the hand; a Strict
//! seat keeps [`Hand::Main`] for its uses (a BRIDGE in `game_loop`), though its
//! shots take their round off-hand first on both copies, as every seat's do.

use crate::block::{self, BlockId};
use crate::crafting::ToolType;
use crate::item::{material_as_placeable_block, Item, ItemStack, MaterialId};

/// Which hand an action uses. On the wire since v101 (a joiner's use packets,
/// `protocol::EditHand`, `UseTag`): wire-stable, APPEND ONLY, Main = 0,
/// Off = 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// The face of the block clicked, as `PlayerSlot::target_face` (the hoe, a seed
    /// or a reed take the top face only). `[0, 0, 0]` when nothing is aimed at.
    pub face: [i32; 3],
    /// The play mode cannot edit the world here (`!PlayMode::can_edit_world`): the
    /// edit arms (hoe, eraser, salt, bone meal, fertiliser) decline.
    pub edit_blocked: bool,
    /// The species of the mob under the crosshair (shears cut a sheep, not a cow).
    pub mob_kind: Option<crate::mob::MobType>,
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

/// Blocks a right-click does something with besides placing against them (a
/// screen, a toggle, a trade, a bell...): the chain's block arms. A shield is not
/// raised against one (it is the click's other use), and a hand item that wants to
/// place still places. Errs toward fewer entries: a block missing here only means
/// the shield rises as the arm runs, since raising never stops the chain.
pub fn block_has_use(b: BlockId) -> bool {
    block_takes_held_item(b)
        || crate::chest::ChestTier::from_block(b).is_some()
        || crate::block_shape::is_toggleable(b)
        || matches!(
            b,
            block::CRAFTING_TABLE
                | block::FURNACE
                | block::FURNACE_LIT
                | block::DISPENSER
                | block::DROPPER
                | block::BED
                | block::BLASTING_KEG
                | block::DRAFTING_TABLE
                | block::REPAIR_BENCH
                | block::VENDOR_BLOCK
                | block::TIP_JAR
                | block::BAZAAR_BLOCK
                | block::AUCTION_BLOCK
                | block::BOUNTY_BOARD
                | block::OAK_SIGN
                | block::VILLAGE_BELL
                | block::MARKET_BELL
                | block::SITE_BOARD
                | block::GRAVE
                | block::COMMONS_STONE
                | block::ARCHITECT_PLAQUE
                | block::FEAST_MARKER
                | block::LATENT_PRINT
        )
}

/// The hand whose item has a use for `target`, in the click's order, or `None` when
/// neither does: the main hand when its item has one (or the aimed block takes
/// whatever is held, [`block_takes_held_item`]), else the off-hand when its item
/// does. An empty hand has no use. The one decision behind [`use_hand`] and
/// [`either_hand_has_use`].
pub fn hand_with_use(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> Option<Hand> {
    if target.block.is_some_and(block_takes_held_item) {
        return Some(Hand::Main);
    }
    if main.is_some_and(|m| item_has_use(m, target)) {
        return Some(Hand::Main);
    }
    match off {
        // BRIDGE: a Plan's Inspect dialog is keyed on a hotbar slot, so a Plan
        // in the off-hand never takes the click — replace when the dialog can
        // name the off-hand.
        Some(Item::Plan(_)) => None,
        // A steed's pack goes on from the main hand only (a joiner's
        // `PackRequest` names the hotbar slot, `pack_attempt`): aimed at a
        // mob, a chest in the off-hand never takes the click, in local play as
        // joined (off-hand review M2).
        Some(Item::Block(block::CHEST)) if target.mob => None,
        Some(o) if item_has_use(o, target) => Some(Hand::Off),
        _ => None,
    }
}

/// The hand a right-click uses: the main hand unless its item has no use for
/// `target` and the off-hand item does (Minecraft's rule). An empty main hand
/// has no use. A block that takes the held item ([`block_takes_held_item`])
/// always keeps the main hand. [`hand_with_use`], defaulting to the main hand.
pub fn use_hand(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> Hand {
    hand_with_use(main, off, target).unwrap_or(Hand::Main)
}

/// Does either hand's item have a use for `target` (or does the block take the
/// held item)? The question a thing that only acts when the click is otherwise
/// unclaimed asks (a raised shield: a door or a mob should win). A shield in a hand
/// answers for itself too ([`item_has_use`]: its use is the raise, so it has one
/// wherever no block or mob claims the click).
pub fn either_hand_has_use(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> bool {
    hand_with_use(main, off, target).is_some()
}

/// Does a held use button raise a shield against `target`? Only when the click's
/// use is the shield's: a shield in a hand, neither hand with another use for what
/// the crosshair is on (a door, a chest or a mob under it wins, and so does a
/// torch about to be placed), and the hand that has the use ([`use_hand`]) holding
/// the shield. A shield alone in the main hand aimed at a door has no use there
/// ([`item_has_use`]), and the main hand being the fallback hand does not make it
/// one. The one decision behind the frame path's raise latch.
pub fn shield_raises(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> bool {
    if !(crate::shield::is_shield(main) || crate::shield::is_shield(off)) || !either_hand_has_use(main, off, target) {
        return false;
    }
    crate::shield::is_shield(match use_hand(main, off, target) {
        Hand::Main => main,
        Hand::Off => off,
    })
}

/// Which hands swing when the player acts: `(main, off)`. Mining and attacking
/// (`break_block`) always swing the main hand; a use (`place_block`) swings the
/// hand [`use_hand`] chose, so a right-click that routes to the off-hand swings
/// the left hand and leaves the right still. First-person feedback only.
pub fn swing_hands(break_block: bool, place_block: bool, place_uses_off_hand: bool) -> (bool, bool) {
    (break_block || (place_block && !place_uses_off_hand), place_block && place_uses_off_hand)
}

/// Does right-click do anything with `item` for `target`? See the module doc:
/// every tool and target-specific material asks the predicate its arm asks
/// (`tool_uses`), so a hoe aimed at stone has no use and the off-hand item gets
/// the click. Errs toward "yes" only for what no predicate can tell.
pub fn item_has_use(item: &Item, target: &UseTarget) -> bool {
    use crate::tool_uses as uses;
    match item {
        // A placeable block needs something to place against.
        Item::Block(_) => target.block.is_some(),
        Item::Plan(_) => true,
        // Armour is equipped from the inventory screen, not by right-click.
        Item::Armour(_) => false,
        Item::Tool(tool) => match tool.tool_type {
            ToolType::Bow => target.arrows,
            ToolType::Slingshot => target.rubber_balls,
            ToolType::Hoe => uses::hoe_tills(target.block, target.face, target.edit_blocked),
            ToolType::FlintAndSteel => uses::flint_lights(target.block),
            // A hive is the block case (`block_takes_held_item`); otherwise a sheep.
            ToolType::Shears => target.mob_kind.is_some_and(uses::shears_cut),
            ToolType::FishingRod => uses::rod_answers(),
            ToolType::Eraser => uses::eraser_erases(target.block, target.edit_blocked),
            ToolType::DraftingStamp => uses::stamp_captures(target.block),
            // Melee and digging tools have no right-click use (the chain has no
            // shovel path or axe strip).
            ToolType::Pickaxe | ToolType::Axe | ToolType::Sword | ToolType::Shovel => false,
            // Off-hand release (slice 5) — a shield's use is the raise: what a click
            // does when nothing else claims it. A mob, a chest, a door and the other
            // block arms still win.
            ToolType::Shield => !target.mob && !target.block.is_some_and(block_has_use),
        },
        Item::Material(m) => {
            if item.is_food() {
                return target.wants_to_eat || target.mob;
            }
            let (block, blocked) = (target.block, target.edit_blocked);
            // Any material a block can be made of is placed against any block.
            let placeable = block.is_some() && material_as_placeable_block(*m).is_some();
            match *m {
                // The chain reads each of these wherever it is aimed: a Lead (a mob
                // or a post), the buckets (a pond, a cow), the whistle.
                MaterialId::Lead
                | MaterialId::Bucket
                | MaterialId::WaterBucket
                | MaterialId::LavaBucket
                | MaterialId::RecallWhistle => true,
                // Target-specific: the arm's own test.
                MaterialId::MagnesiumFirestarter => uses::flint_lights(block) || placeable,
                MaterialId::Bonemeal => uses::bonemeal_grows(block, blocked) || placeable,
                MaterialId::Fertiliser => uses::fertiliser_grows(block, blocked) || placeable,
                MaterialId::Salt => uses::salt_paths(block, blocked) || placeable,
                MaterialId::PapyrusReed => uses::reed_plants(block, target.face) || placeable,
                // A Stick feeds a campfire; it is not for a mob.
                MaterialId::Stick => matches!(block, Some(block::CAMPFIRE | block::CAMPFIRE_UNLIT)) || placeable,
                // Seeds go on tilled soil, a cart on a rail (any block); any other
                // material is for a mob (feeding or taming it) or nothing.
                _ => {
                    placeable
                        || (crate::use_edits::sown_crop(*m).is_some() && uses::seeds_sown(block, target.face))
                        || (block.is_some() && crate::cart::cart_hull_for_item(*m).is_some())
                        || target.mob
                }
            }
        }
    }
}

/// The block a click with `item` would place (a block, or a material
/// flagged placeable), without taking it: `Inventory::hotbar_placeable_id`'s
/// rule for any one stack.
pub fn placeable_id(item: &Item) -> Option<BlockId> {
    match item {
        Item::Block(b) => Some(*b),
        Item::Material(m) => material_as_placeable_block(*m),
        _ => None,
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
/// The one rule for the slot's click: `window::apply` runs it for
/// [`crate::window::WindowClick::Offhand`], so a joined client's click and the
/// server's copy of it (v101) are the same function.
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
    #[test]
    fn an_off_hand_use_swings_the_left_hand_only() {
        use super::swing_hands;
        assert_eq!(swing_hands(false, true, true), (false, true), "a right-click routed to the off-hand");
        assert_eq!(swing_hands(false, true, false), (true, false), "a right-click on the main hand");
        assert_eq!(swing_hands(true, false, false), (true, false), "mining swings the main hand");
        assert_eq!(swing_hands(true, false, true), (true, false), "the off-hand never mines");
        assert_eq!(swing_hands(true, true, true), (true, true), "both buttons: both hands");
        assert_eq!(swing_hands(false, false, true), (false, false), "no click, no swing");
    }

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

    // --- Off-hand release (slice 5): which click raises a shield ---

    #[test]
    fn a_shield_in_the_off_hand_takes_a_click_the_main_item_has_no_use_for() {
        let sword = tool(ToolType::Sword);
        let shield = tool(ToolType::Shield);
        assert_eq!(use_hand(Some(&sword), Some(&shield), &stone_target()), Hand::Off, "aimed at stone");
        assert_eq!(use_hand(Some(&sword), Some(&shield), &UseTarget::default()), Hand::Off, "aimed at the air");
    }

    #[test]
    fn a_shield_in_the_main_hand_keeps_the_click_in_the_open() {
        let shield = tool(ToolType::Shield);
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&shield), Some(&torch), &UseTarget::default()), Hand::Main);
        assert!(item_has_use(&shield, &stone_target()));
    }

    #[test]
    fn a_chest_a_door_or_a_mob_under_the_crosshair_wins_over_the_shield() {
        let shield = tool(ToolType::Shield);
        let sword = tool(ToolType::Sword);
        for b in [block::CHEST, block::CRAFTING_TABLE, block::OAK_DOOR, block::FURNACE, block::BED] {
            let t = UseTarget { block: Some(b), ..UseTarget::default() };
            assert!(!item_has_use(&shield, &t), "{b}: the block's own use wins");
            assert_eq!(use_hand(Some(&sword), Some(&shield), &t), Hand::Main, "{b}: not routed to the shield");
        }
        let mob = UseTarget { mob: true, ..UseTarget::default() };
        assert!(!item_has_use(&shield, &mob), "a mob wins");
        assert_eq!(use_hand(Some(&sword), Some(&shield), &mob), Hand::Main);
    }

    #[test]
    fn a_block_in_hand_places_and_leaves_the_shield_down() {
        let torch = Item::Block(block::TORCH);
        let shield = tool(ToolType::Shield);
        // Aimed at a block: the torch has a place to go, so it keeps the click.
        assert_eq!(use_hand(Some(&torch), Some(&shield), &stone_target()), Hand::Main);
        // Aimed at the air the torch has nowhere to go: the shield rises.
        assert_eq!(use_hand(Some(&torch), Some(&shield), &UseTarget::default()), Hand::Off);
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

    /// Off-hand review M2: a steed's pack goes on from the main hand only
    /// (`PackRequest` names the hotbar slot), so a chest in the off-hand
    /// never takes a click aimed at a mob, whatever block lies behind it.
    /// Local play and a joined client both route by this rule.
    #[test]
    fn a_chest_in_the_off_hand_never_takes_a_click_on_a_mob() {
        let pick = tool(ToolType::Pickaxe);
        let chest = Item::Block(block::CHEST);
        let mob_before_stone = UseTarget { mob: true, ..stone_target() };
        assert_eq!(use_hand(Some(&pick), Some(&chest), &mob_before_stone), Hand::Main);
        assert_eq!(use_hand(None, Some(&chest), &mob_before_stone), Hand::Main);
        // Aimed at a block alone, the off-hand chest is still placed.
        assert_eq!(use_hand(Some(&pick), Some(&chest), &stone_target()), Hand::Off);
        // Another block in the off-hand is unchanged at a mob.
        let torch = Item::Block(block::TORCH);
        assert_eq!(use_hand(Some(&pick), Some(&torch), &mob_before_stone), Hand::Off);
    }

    // --- Review 1 finding 3: a use is a use for THIS target ---

    fn aimed(b: BlockId, face: [i32; 3]) -> UseTarget {
        UseTarget { block: Some(b), face, ..UseTarget::default() }
    }

    const UP: [i32; 3] = [0, 1, 0];
    const SIDE: [i32; 3] = [1, 0, 0];

    /// A tool in the main hand and a torch in the off-hand: which hand the click goes to.
    fn with_a_torch_off(main: Item, target: &UseTarget) -> Hand {
        use_hand(Some(&main), Some(&Item::Block(block::TORCH)), target)
    }

    #[test]
    fn a_hoe_on_stone_hands_the_click_to_the_torch_but_still_tills_dirt_and_grass() {
        let hoe = || tool(ToolType::Hoe);
        assert_eq!(with_a_torch_off(hoe(), &aimed(block::STONE, UP)), Hand::Off, "stone: the torch is placed");
        assert_eq!(with_a_torch_off(hoe(), &aimed(block::GRASS, UP)), Hand::Main, "grass top: the hoe tills");
        assert_eq!(with_a_torch_off(hoe(), &aimed(block::DIRT, UP)), Hand::Main, "dirt top: the hoe tills");
        assert_eq!(with_a_torch_off(hoe(), &aimed(block::GRASS, SIDE)), Hand::Off, "a side face is not tilled");
        let no_edit = UseTarget { edit_blocked: true, ..aimed(block::GRASS, UP) };
        assert_eq!(with_a_torch_off(hoe(), &no_edit), Hand::Off, "a mode that cannot edit tills nothing");
    }

    #[test]
    fn shovel_shears_flint_and_eraser_decline_what_they_cannot_act_on() {
        let stone = aimed(block::STONE, UP);
        for t in [ToolType::Shovel, ToolType::Shears, ToolType::FlintAndSteel, ToolType::Eraser] {
            assert_eq!(with_a_torch_off(tool(t), &stone), Hand::Off, "{t:?} on stone");
        }
        // And keep the click where they do act.
        assert_eq!(with_a_torch_off(tool(ToolType::FlintAndSteel), &aimed(block::CAMPFIRE_UNLIT, UP)), Hand::Main);
        assert_eq!(with_a_torch_off(tool(ToolType::Eraser), &aimed(block::BLUEPRINT_PAPER, UP)), Hand::Main);
        let sheep = UseTarget { mob: true, mob_kind: Some(crate::mob::MobType::Sheep), ..stone };
        let cow = UseTarget { mob: true, mob_kind: Some(crate::mob::MobType::Cow), ..stone };
        assert_eq!(with_a_torch_off(tool(ToolType::Shears), &sheep), Hand::Main, "shears on a sheep");
        assert_eq!(with_a_torch_off(tool(ToolType::Shears), &cow), Hand::Off, "shears on a cow: the off-hand gets the click");
        assert_eq!(with_a_torch_off(tool(ToolType::Shears), &aimed(block::BEE_HIVE, SIDE)), Hand::Main, "a hive takes the held item");
    }

    #[test]
    fn a_stamp_and_a_rod_answer_every_click_so_they_keep_it() {
        let stone = aimed(block::STONE, UP);
        assert_eq!(with_a_torch_off(tool(ToolType::DraftingStamp), &stone), Hand::Main);
        assert_eq!(with_a_torch_off(tool(ToolType::FishingRod), &stone), Hand::Main);
        assert_eq!(with_a_torch_off(tool(ToolType::FishingRod), &UseTarget::default()), Hand::Main);
    }

    #[test]
    fn target_specific_materials_decline_the_wrong_target() {
        let mat = |m| Item::Material(m);
        let stone = aimed(block::STONE, UP);
        for m in [MaterialId::Salt, MaterialId::Bonemeal, MaterialId::Fertiliser, MaterialId::PapyrusReed, MaterialId::MagnesiumFirestarter] {
            assert_eq!(with_a_torch_off(mat(m), &stone), Hand::Off, "{m:?} on stone");
        }
        assert_eq!(with_a_torch_off(mat(MaterialId::Salt), &aimed(block::GRASS, UP)), Hand::Main);
        assert_eq!(with_a_torch_off(mat(MaterialId::Bonemeal), &aimed(block::GRASS, UP)), Hand::Main);
        assert_eq!(with_a_torch_off(mat(MaterialId::Fertiliser), &aimed(block::WHEAT_STAGE_0, UP)), Hand::Main);
        assert_eq!(with_a_torch_off(mat(MaterialId::PapyrusReed), &aimed(block::SAND, UP)), Hand::Main);
        assert_eq!(with_a_torch_off(mat(MaterialId::PapyrusReed), &aimed(block::SAND, SIDE)), Hand::Off, "a side face");
        assert_eq!(with_a_torch_off(mat(MaterialId::MagnesiumFirestarter), &aimed(block::CAMPFIRE_UNLIT, UP)), Hand::Main);
        let seed = crate::item::Item::Material(MaterialId::WheatSeeds);
        assert!(crate::use_edits::sown_crop(MaterialId::WheatSeeds).is_some(), "the test needs real seeds");
        assert_eq!(with_a_torch_off(seed.clone(), &stone), Hand::Off, "seeds on stone");
        assert_eq!(with_a_torch_off(seed, &aimed(block::TILLED_SOIL, UP)), Hand::Main, "seeds on tilled soil");
    }

    /// The decision as an `Option`, and "does either hand have a use" derived from it.
    #[test]
    fn hand_with_use_names_the_hand_and_either_hand_has_use_says_whether_any_does() {
        let (hoe, torch, sword, shield) = (tool(ToolType::Hoe), Item::Block(block::TORCH), tool(ToolType::Sword), tool(ToolType::Shield));
        let stone = aimed(block::STONE, UP);
        assert_eq!(hand_with_use(Some(&hoe), Some(&torch), &stone), Some(Hand::Off));
        assert_eq!(hand_with_use(Some(&hoe), Some(&torch), &aimed(block::GRASS, UP)), Some(Hand::Main));
        assert_eq!(hand_with_use(Some(&hoe), None, &stone), None, "nothing to do");
        assert_eq!(hand_with_use(Some(&sword), Some(&sword), &stone), None);
        assert!(either_hand_has_use(Some(&hoe), Some(&torch), &stone));
        assert!(!either_hand_has_use(Some(&hoe), None, &stone));
        // A door under the crosshair: the hand items with no use leave the click
        // to the door, so a shield beside a sword has no other use there either.
        let door = aimed(block::OAK_DOOR, UP);
        assert!(!either_hand_has_use(Some(&sword), Some(&shield), &door), "a door wins over a shield: neither hand takes it");
        assert!(either_hand_has_use(Some(&sword), Some(&shield), &UseTarget::default()), "in the open the shield has its use");
        // A block that takes the held item counts as the main hand's.
        assert_eq!(hand_with_use(None, Some(&torch), &aimed(block::COMPOSTER, UP)), Some(Hand::Main));
    }

    /// Review 2 finding 5: the target's own use wins over raising. A shield alone in
    /// the main hand aimed at a door or a mob does NOT raise (the main hand being the
    /// fallback hand is not a use); on air or stone it does.
    #[test]
    fn a_shield_raises_only_where_the_target_has_no_other_use() {
        let (shield, sword, pick, torch) =
            (tool(ToolType::Shield), tool(ToolType::Sword), tool(ToolType::Pickaxe), Item::Block(block::TORCH));
        let door = aimed(block::OAK_DOOR, UP);
        let chest = aimed(block::CHEST, UP);
        let mob = UseTarget { mob: true, mob_kind: Some(crate::mob::MobType::Cow), ..UseTarget::default() };
        // A main-hand shield, an empty off-hand.
        assert!(!shield_raises(Some(&shield), None, &door), "a door toggles, the shield stays down");
        assert!(!shield_raises(Some(&shield), None, &chest), "a chest opens");
        assert!(!shield_raises(Some(&shield), None, &mob), "a mob under the crosshair");
        assert!(shield_raises(Some(&shield), None, &UseTarget::default()), "air");
        assert!(shield_raises(Some(&shield), None, &stone_target()), "stone");
        // The same from the off-hand.
        assert!(!shield_raises(Some(&sword), Some(&shield), &door));
        assert!(!shield_raises(Some(&sword), Some(&shield), &mob));
        assert!(shield_raises(Some(&sword), Some(&shield), &UseTarget::default()));
        assert!(shield_raises(Some(&pick), Some(&shield), &stone_target()));
        // No shield, nothing to raise.
        assert!(!shield_raises(Some(&sword), None, &UseTarget::default()));
        assert!(!shield_raises(None, None, &stone_target()));
        // Another item with a use on this target takes the click: a torch about to be
        // placed on stone is placed, and on air the shield behind it raises.
        assert!(!shield_raises(Some(&torch), Some(&shield), &stone_target()), "the torch is placed");
        assert!(shield_raises(Some(&torch), Some(&shield), &UseTarget::default()), "nothing to place against: the shield");
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
