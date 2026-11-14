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
//! a use. A shield's raise is the decision's last claimant ([`shield_raises`]): a
//! use of the other hand, a door or chest, or a mob a hand really does something
//! to (`mob_uses`) wins over it, and a hostile mob never does.
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
    /// The sneak button is held (the horse family feeds only while sneaking).
    pub sneak: bool,
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
    /// The mob under the crosshair: its species and the state its actions need
    /// (`mob_uses`). `None` with `mob` set is a mob nothing is known about, which no
    /// held item does anything to.
    pub mob_facts: Option<crate::mob_uses::MobFacts>,
    /// An EMPTY bucket's fluid ray reaches a water or lava source it can fill from,
    /// and the play mode can edit (`GameState::bucket_fill_in_reach`, the ray
    /// `try_bucket_fill` casts). That ray passes through nothing else, so it is a
    /// separate question from `block` (whose ray passes through water) and from the
    /// mob: a fish under the crosshair does not take it away.
    pub bucket_fills: bool,
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

/// Who takes the click and why: the one decision behind [`hand_with_use`],
/// [`use_hand`], [`either_hand_has_use`] and [`shield_raises`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    /// A hand's item (not a raised shield) does something here, or the block or mob
    /// aimed at takes the click: this hand runs the chain.
    Use(Hand),
    /// Nothing else claims the click and this hand holds a shield: it is raised.
    Raise(Hand),
}

/// Does the hand holding `item` (`None` = empty) do anything at all for `target`:
/// first the whole mob chain for that hand ([`crate::mob_uses::hand_mob_use`]: the
/// chain's own order, a refusing harvest ending only the mob arms), then the item's
/// uses that are not about a mob (a block placed, food eaten, a bucket filled at
/// water). A shield's raise is not counted here: it is the last claimant.
fn hand_acts(item: Option<&Item>, target: &UseTarget) -> bool {
    use crate::mob_uses::{hand_mob_use, MobUse};
    if target.mob_facts.is_some_and(|f| hand_mob_use(item, &f, target.sneak) == MobUse::Acts) {
        return true;
    }
    item.is_some_and(|i| !crate::shield::is_shield(Some(i)) && non_mob_use(i, target))
}

/// In the click's order: the block that takes whatever is held, then each hand in
/// turn, the main hand's WHOLE action first (the mob chain in its own order, then
/// its other uses) and only if it has nothing, the off-hand's, and only then a
/// shield (the main hand's first). A shield's own use, the raise, is the last
/// claimant: another hand's real use, a door or a chest, and a mob that a hand does
/// something to all win over it, and nothing else does. An empty off-hand never
/// takes a click (a pet is commanded by an empty MAIN hand).
fn pick(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> Option<Pick> {
    if target.block.is_some_and(block_takes_held_item) {
        return Some(Pick::Use(Hand::Main));
    }
    if hand_acts(main, target) {
        return Some(Pick::Use(Hand::Main));
    }
    match off {
        None => {}
        // BRIDGE: a Plan's Inspect dialog is keyed on a hotbar slot, so a Plan
        // in the off-hand never takes the click — replace when the dialog can
        // name the off-hand.
        Some(Item::Plan(_)) => {}
        // A steed's pack goes on from the main hand only (a joiner's
        // `PackRequest` names the hotbar slot, `pack_attempt`): aimed at a
        // mob, a chest in the off-hand never takes the click, in local play as
        // joined (off-hand review M2).
        Some(Item::Block(block::CHEST)) if target.mob => {}
        Some(o) if hand_acts(Some(o), target) => return Some(Pick::Use(Hand::Off)),
        Some(_) => {}
    }
    // Nothing else wants the click. A shield raises unless a block with a use of its
    // own is aimed at (a door, a chest...); a mob under the crosshair that nothing
    // held works on does not stop it (the enemy in front of you).
    if target.block.is_some_and(block_has_use) {
        return None;
    }
    [(Hand::Main, main), (Hand::Off, off)]
        .into_iter()
        .find(|(_, i)| crate::shield::is_shield(*i))
        .map(|(h, _)| Pick::Raise(h))
}

/// The hand whose item has a use for `target`, in the click's order, or `None` when
/// neither does: the main hand when its item has one (or the aimed block takes
/// whatever is held, [`block_takes_held_item`]), else the off-hand when its item
/// does; a mob that takes the click whatever is held (a steed, a villager) or an
/// empty hand's command to its own pet goes to the main hand. A shield counts last,
/// for its raise. An empty hand has no other use. The one decision behind
/// [`use_hand`] and [`either_hand_has_use`].
pub fn hand_with_use(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> Option<Hand> {
    pick(main, off, target).map(|p| match p {
        Pick::Use(h) | Pick::Raise(h) => h,
    })
}

/// The hand a right-click uses: the main hand unless its item has no use for
/// `target` and the off-hand item does (Minecraft's rule). An empty main hand
/// has no use. A block that takes the held item ([`block_takes_held_item`])
/// always keeps the main hand. [`hand_with_use`], defaulting to the main hand.
pub fn use_hand(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> Hand {
    hand_with_use(main, off, target).unwrap_or(Hand::Main)
}

/// Does either hand's item have a use for `target` (or does the block or mob take
/// the click)? The question a thing that only acts when the click is otherwise
/// unclaimed asks. A shield in a hand answers for itself too (its use is the raise,
/// so it has one wherever no block or mob claims the click).
#[cfg(test)]
pub fn either_hand_has_use(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> bool {
    hand_with_use(main, off, target).is_some()
}

/// Does a use button pressed on `target` raise a shield? Only when the click's use
/// is the shield's and nothing else's: a shield in a hand, the other hand with no
/// use here (a torch about to be placed on stone is placed, a shield main with a
/// torch off on stone places the torch; on air the shield raises), no door or chest
/// aimed at, and no mob a hand really does something to (feeding, taming, a Lead,
/// milking, shearing, riding, trading, a pet's command: `mob_uses`). A hostile mob,
/// or one nothing held does anything to, never takes the click. The decision is made
/// when the press starts ([`shield_press`] keeps it up while the button is held).
pub fn shield_raises(main: Option<&Item>, off: Option<&Item>, target: &UseTarget) -> bool {
    matches!(pick(main, off, target), Some(Pick::Raise(_)))
}

/// What the shield does this frame: `raised` is the state the frame applies,
/// `held_over` says the press was already decided as a raise and is held on (not
/// the press frame): the chain must not act on whatever is under the crosshair now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldHold {
    pub raised: bool,
    pub held_over: bool,
}

/// Where a seat's use press stands: the classification is made once per press and
/// kept until the button is released, so nothing re-asks the target while it stays
/// held (a cow fed on the press frame is in love on the next, and wheat no longer
/// claims it: that must not turn the press into a raise).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Press {
    /// The button is up.
    #[default]
    Idle,
    /// The press was classified: `raised` is its outcome (a decision to keep the
    /// shield down included).
    Decided { raised: bool },
}

/// One frame of the use button, given last frame's [`Press`]: the target check
/// (`raises_here`, [`shield_raises`], asked only on the press frame, never again
/// while the button stays held) decides when the press starts; once decided it
/// stands (a raise stays up whatever comes under the crosshair, a decision to keep
/// the shield down stays down) until release, and a new press asks the target
/// again. A shield no longer in a hand lowers the raise and it stays down for the
/// press. Pure, so a test pins it without a window.
pub fn shield_press(
    prev: Press,
    holding: bool,
    shield_in_hand: bool,
    raises_here: impl FnOnce() -> bool,
) -> (ShieldHold, Press) {
    if !holding {
        return (ShieldHold { raised: false, held_over: false }, Press::Idle);
    }
    match prev {
        Press::Idle => {
            let raised = shield_in_hand && raises_here();
            (ShieldHold { raised, held_over: false }, Press::Decided { raised })
        }
        Press::Decided { raised } => {
            let up = raised && shield_in_hand;
            (ShieldHold { raised: up, held_over: up }, Press::Decided { raised: up })
        }
    }
}

/// The off-hand item a seat's click can really use. A Strict seat keeps the main
/// hand for its uses (a BRIDGE in `game_loop`: its typed actions carry no hand), so
/// an off-hand action that is not a shield can never run there and must not claim
/// the click; an off-hand shield can still be raised. Raise selection and execution
/// ask the same rule.
pub fn usable_off_hand(strict: bool, off: Option<&Item>) -> Option<&Item> {
    if strict && !crate::shield::is_shield(off) { None } else { off }
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
/// the click. Errs toward "yes" only for what no predicate can tell. A mob under
/// the crosshair counts ([`crate::mob_uses::hand_mob_use`]); a shield's use is its
/// raise, wherever no block with a use of its own is aimed at.
#[cfg(test)]
pub fn item_has_use(item: &Item, target: &UseTarget) -> bool {
    if crate::shield::is_shield(Some(item)) {
        return !target.block.is_some_and(block_has_use);
    }
    hand_acts(Some(item), target)
}

/// What `item` does that is not about a mob: the non-mob half of [`item_has_use`],
/// the part a failed mob use must never erase (a bucket's fill at water beside a
/// fish or a cow just milked).
fn non_mob_use(item: &Item, target: &UseTarget) -> bool {
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
            // A hive is the block case (`block_takes_held_item`); a sheep is the mob's.
            ToolType::Shears => false,
            ToolType::FishingRod => uses::rod_answers(),
            ToolType::Eraser => uses::eraser_erases(target.block, target.edit_blocked),
            ToolType::DraftingStamp => uses::stamp_captures(target.block),
            // Melee and digging tools have no right-click use (the chain has no
            // shovel path or axe strip).
            ToolType::Pickaxe | ToolType::Axe | ToolType::Sword | ToolType::Shovel => false,
            // A shield's use is the raise, which [`pick`] asks last.
            ToolType::Shield => false,
        },
        Item::Material(m) => {
            if item.is_food() {
                return target.wants_to_eat;
            }
            let (block, blocked) = (target.block, target.edit_blocked);
            // Any material a block can be made of is placed against any block.
            let placeable = block.is_some() && material_as_placeable_block(*m).is_some();
            match *m {
                // A Lead goes on a fence post (a mob is the mob chain's).
                MaterialId::Lead => block.is_some_and(block::is_fence_post),
                // An empty bucket fills at a pond (the fluid ray, `bucket_fills`) or
                // taps a rubber log (`game_loop`'s block arm); a hive is the block
                // case. Milking is the mob chain's.
                MaterialId::Bucket => target.bucket_fills || (block == Some(block::RUBBER_LOG) && !blocked),
                // These read the click wherever it is aimed.
                MaterialId::WaterBucket | MaterialId::LavaBucket | MaterialId::RecallWhistle => true,
                // Target-specific: the arm's own test.
                MaterialId::MagnesiumFirestarter => uses::flint_lights(block) || placeable,
                MaterialId::Bonemeal => uses::bonemeal_grows(block, blocked) || placeable,
                MaterialId::Fertiliser => uses::fertiliser_grows(block, blocked) || placeable,
                MaterialId::Salt => uses::salt_paths(block, blocked) || placeable,
                MaterialId::PapyrusReed => uses::reed_plants(block, target.face) || placeable,
                // A Stick feeds a campfire; it is not for a mob.
                MaterialId::Stick => matches!(block, Some(block::CAMPFIRE | block::CAMPFIRE_UNLIT)) || placeable,
                // Seeds go on tilled soil, a cart on a rail (any block); any other
                // material is for a mob that takes it or nothing.
                _ => {
                    placeable
                        || (crate::use_edits::sown_crop(*m).is_some() && uses::seeds_sown(block, target.face))
                        || (block.is_some() && crate::cart::cart_hull_for_item(*m).is_some())
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
    fn a_chest_or_a_door_under_the_crosshair_wins_over_the_shield() {
        let shield = tool(ToolType::Shield);
        let sword = tool(ToolType::Sword);
        for b in [block::CHEST, block::CRAFTING_TABLE, block::OAK_DOOR, block::FURNACE, block::BED] {
            let t = UseTarget { block: Some(b), ..UseTarget::default() };
            assert!(!item_has_use(&shield, &t), "{b}: the block's own use wins");
            assert_eq!(use_hand(Some(&sword), Some(&shield), &t), Hand::Main, "{b}: not routed to the shield");
        }
    }

    /// A target with a mob of `kind` (in the default state) under the crosshair.
    fn at_mob(kind: crate::mob::MobType) -> UseTarget {
        UseTarget { mob: true, mob_facts: Some(crate::mob_uses::MobFacts::of(kind)), ..UseTarget::default() }
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
        let cow = at_mob(crate::mob::MobType::Cow);
        assert_eq!(use_hand(Some(&wheat), Some(&torch), &cow), Hand::Main);
        // Not a cow's food: the plain material is nothing to the mob.
        let ingot = Item::Material(MaterialId::IronIngot);
        assert!(!item_has_use(&ingot, &cow));
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
        let sheep = UseTarget { block: stone.block, face: stone.face, ..at_mob(crate::mob::MobType::Sheep) };
        let cow = UseTarget { block: stone.block, face: stone.face, ..at_mob(crate::mob::MobType::Cow) };
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

    /// The target's own use wins over raising, and a mob wins only when a hand really
    /// does something to it (decision A): a hostile mob never takes the click.
    #[test]
    fn a_shield_raises_only_where_the_target_has_no_other_use() {
        let (shield, sword, pick, torch) =
            (tool(ToolType::Shield), tool(ToolType::Sword), tool(ToolType::Pickaxe), Item::Block(block::TORCH));
        let door = aimed(block::OAK_DOOR, UP);
        let chest = aimed(block::CHEST, UP);
        // A main-hand shield, an empty off-hand.
        assert!(!shield_raises(Some(&shield), None, &door), "a door toggles, the shield stays down");
        assert!(!shield_raises(Some(&shield), None, &chest), "a chest opens");
        assert!(shield_raises(Some(&shield), None, &UseTarget::default()), "air");
        assert!(shield_raises(Some(&shield), None, &stone_target()), "stone");
        // The same from the off-hand.
        assert!(!shield_raises(Some(&sword), Some(&shield), &door));
        assert!(shield_raises(Some(&sword), Some(&shield), &UseTarget::default()));
        assert!(shield_raises(Some(&pick), Some(&shield), &stone_target()));
        // No shield, nothing to raise.
        assert!(!shield_raises(Some(&sword), None, &UseTarget::default()));
        assert!(!shield_raises(None, None, &stone_target()));
        // Another hand's item with a use on this target takes the click: a torch about
        // to be placed on stone is placed, and on air the shield raises.
        assert!(!shield_raises(Some(&torch), Some(&shield), &stone_target()), "the torch is placed");
        assert!(shield_raises(Some(&torch), Some(&shield), &UseTarget::default()), "nothing to place against: the shield");
    }

    /// Review 3 finding 5: with a shield in one hand, the OTHER hand's real use wins,
    /// whichever hand it is. Shield main and a torch off, aimed at stone: the torch is
    /// placed (the off-hand takes the click); on air it raises.
    #[test]
    fn a_shield_in_the_main_hand_yields_to_the_off_hands_real_use() {
        let (shield, torch) = (tool(ToolType::Shield), Item::Block(block::TORCH));
        assert!(!shield_raises(Some(&shield), Some(&torch), &stone_target()), "the torch is placed");
        assert_eq!(hand_with_use(Some(&shield), Some(&torch), &stone_target()), Some(Hand::Off));
        assert_eq!(use_hand(Some(&shield), Some(&torch), &stone_target()), Hand::Off, "the click is the torch's");
        assert!(shield_raises(Some(&shield), Some(&torch), &UseTarget::default()), "on air the shield raises");
        assert_eq!(hand_with_use(Some(&shield), Some(&torch), &UseTarget::default()), Some(Hand::Main));
        // Food in the off-hand when hungry, a hoe on grass: the same.
        let bread = Item::Material(MaterialId::Bread);
        let hungry = UseTarget { wants_to_eat: true, ..UseTarget::default() };
        assert!(!shield_raises(Some(&shield), Some(&bread), &hungry), "a hungry player eats");
        assert!(shield_raises(Some(&shield), Some(&bread), &UseTarget::default()), "not hungry: the shield");
        let hoe = tool(ToolType::Hoe);
        assert!(!shield_raises(Some(&shield), Some(&hoe), &aimed(block::GRASS, UP)), "the hoe tills");
    }

    // --- Decision A: a mob wins only when a hand really does something to it ---

    #[test]
    fn a_shield_raises_at_a_hostile_mob_from_either_hand() {
        use crate::mob::MobType;
        let (shield, sword) = (tool(ToolType::Shield), tool(ToolType::Sword));
        for kind in [MobType::Brigand, MobType::Marauder, MobType::Berserker] {
            let t = at_mob(kind);
            assert!(shield_raises(Some(&shield), None, &t), "{kind:?}: shield main");
            assert!(shield_raises(Some(&sword), Some(&shield), &t), "{kind:?}: shield off");
            assert_eq!(hand_with_use(Some(&sword), Some(&shield), &t), Some(Hand::Off));
            // A block behind the enemy changes nothing for a plain wall.
            let before_stone = UseTarget { block: Some(block::STONE), ..t };
            assert!(shield_raises(Some(&sword), Some(&shield), &before_stone), "{kind:?}: in front of stone");
        }
    }

    #[test]
    fn a_cow_is_fed_when_a_hand_holds_its_food_and_otherwise_the_shield_raises() {
        use crate::mob::MobType;
        let (shield, pick) = (tool(ToolType::Shield), tool(ToolType::Pickaxe));
        let wheat = Item::Material(MaterialId::Wheat);
        let cow = at_mob(MobType::Cow);
        // Wheat in the other hand: the cow is fed from that hand, the shield stays down.
        assert!(!shield_raises(Some(&shield), Some(&wheat), &cow), "shield main, wheat off");
        assert_eq!(hand_with_use(Some(&shield), Some(&wheat), &cow), Some(Hand::Off), "the off-hand feeds");
        assert!(!shield_raises(Some(&wheat), Some(&shield), &cow), "shield off, wheat main");
        assert_eq!(hand_with_use(Some(&wheat), Some(&shield), &cow), Some(Hand::Main));
        // A calf, or a cow in love, takes no wheat: the shield raises.
        let calf = UseTarget { mob_facts: Some(crate::mob_uses::MobFacts { can_breed: false, ..crate::mob_uses::MobFacts::of(MobType::Cow) }), ..cow };
        assert!(shield_raises(Some(&shield), Some(&wheat), &calf));
        // A pickaxe does nothing to a cow: the shield raises.
        assert!(shield_raises(Some(&pick), Some(&shield), &cow), "pickaxe main, shield off");
        assert!(shield_raises(Some(&shield), Some(&pick), &cow), "shield main, pickaxe off");
        assert!(shield_raises(Some(&shield), None, &cow), "nothing held does anything to it");
        // A bucket milks a ready cow; a cow milked a moment ago is not milked.
        let bucket = Item::Material(MaterialId::Bucket);
        assert!(!shield_raises(Some(&shield), Some(&bucket), &cow));
        let milked = UseTarget { mob_facts: Some(crate::mob_uses::MobFacts { product_ready: false, ..crate::mob_uses::MobFacts::of(MobType::Cow) }), ..cow };
        assert!(shield_raises(Some(&shield), Some(&bucket), &milked));
        // A Lead ties a passive mob, never a brigand.
        let lead = Item::Material(MaterialId::Lead);
        assert!(!shield_raises(Some(&shield), Some(&lead), &cow));
        assert!(shield_raises(Some(&shield), Some(&lead), &at_mob(MobType::Brigand)));
        // Shears cut a ready sheep and nothing else.
        let shears = tool(ToolType::Shears);
        assert!(!shield_raises(Some(&shield), Some(&shears), &at_mob(MobType::Sheep)));
        assert!(shield_raises(Some(&shield), Some(&shears), &cow));
    }

    #[test]
    fn a_villager_a_steed_and_your_own_pet_take_the_click_from_a_shield() {
        use crate::mob::MobType;
        let (shield, sword) = (tool(ToolType::Shield), tool(ToolType::Sword));
        // A villager: trade wins whatever is held, and the main hand runs the chain.
        let villager = at_mob(MobType::Villager);
        assert!(!shield_raises(Some(&sword), Some(&shield), &villager), "trade wins");
        assert!(!shield_raises(Some(&shield), None, &villager), "trade wins from a main-hand shield too");
        assert_eq!(use_hand(Some(&sword), Some(&shield), &villager), Hand::Main);
        // A horse: riding wins.
        for kind in [MobType::Horse, MobType::Donkey, MobType::Nostrich] {
            assert!(!shield_raises(Some(&sword), Some(&shield), &at_mob(kind)), "{kind:?}: riding wins");
        }
        // Your own tamed wolf: an empty hand's command wins; wild or someone else's does not.
        let own = |kind| UseTarget {
            mob_facts: Some(crate::mob_uses::MobFacts { tamed: true, own_pet: true, ..crate::mob_uses::MobFacts::of(kind) }),
            ..at_mob(kind)
        };
        assert!(!shield_raises(None, Some(&shield), &own(MobType::Wolf)), "the command wins");
        assert_eq!(hand_with_use(None, Some(&shield), &own(MobType::Wolf)), Some(Hand::Main), "the empty main hand runs it");
        assert!(!shield_raises(None, Some(&shield), &own(MobType::Cat)), "a companion's command wins");
        assert!(shield_raises(None, Some(&shield), &at_mob(MobType::Wolf)), "a wild wolf is an enemy-in-front, not a pet");
        // A shield held in the main hand cannot command (the pet reads an EMPTY hand), so it raises.
        assert!(shield_raises(Some(&shield), None, &own(MobType::Wolf)));
        // A tethered mob has its Lead taken off whatever is held.
        let tied = UseTarget {
            mob_facts: Some(crate::mob_uses::MobFacts { tethered: true, ..crate::mob_uses::MobFacts::of(MobType::Cow) }),
            ..at_mob(MobType::Cow)
        };
        assert!(!shield_raises(Some(&sword), Some(&shield), &tied));
    }

    // --- Review 4: hand choice at mobs ---

    fn facts_at(f: crate::mob_uses::MobFacts) -> UseTarget {
        UseTarget { mob: true, mob_facts: Some(f), ..UseTarget::default() }
    }

    /// Finding 1: the main hand's whole mob chain goes before the off-hand's. Wheat in
    /// the main hand and a Lead in the off-hand, a plain right-click on a free horse
    /// (or a villager): the main hand's click mounts (opens trade) and the Lead is
    /// not spent on the horse.
    #[test]
    fn the_main_hands_whole_mob_chain_goes_before_the_off_hand() {
        use crate::mob::MobType;
        let (wheat, lead, sword, shield) =
            (Item::Material(MaterialId::Wheat), Item::Material(MaterialId::Lead), tool(ToolType::Sword), tool(ToolType::Shield));
        for kind in [MobType::Horse, MobType::Donkey, MobType::Villager] {
            let t = at_mob(kind);
            assert_eq!(hand_with_use(Some(&wheat), Some(&lead), &t), Some(Hand::Main), "{kind:?}: wheat main, Lead off");
            assert_eq!(use_hand(Some(&wheat), Some(&lead), &t), Hand::Main, "{kind:?}");
            assert_eq!(use_hand(Some(&sword), Some(&lead), &t), Hand::Main, "{kind:?}: a sword main does not hand the click to the Lead either");
            assert_eq!(use_hand(Some(&shield), Some(&lead), &t), Hand::Main, "{kind:?}: nor a shield");
        }
        // Where the main hand has nothing at a mob, the off-hand's real use still gets the click.
        let cow = at_mob(MobType::Cow);
        assert_eq!(hand_with_use(Some(&sword), Some(&lead), &cow), Some(Hand::Off), "a Lead ties a cow from the off-hand");
        // And in the chain's own order within a hand: the main Lead ties a cow before the off wheat feeds.
        assert_eq!(hand_with_use(Some(&lead), Some(&wheat), &cow), Some(Hand::Main));
        // Sneaking, wheat feeds a horse (the feed arm comes before the mount arm, in the same hand).
        let sneaking = UseTarget { sneak: true, ..at_mob(MobType::Horse) };
        assert_eq!(hand_with_use(Some(&wheat), Some(&lead), &sneaking), Some(Hand::Main));
    }

    /// Finding 2: a bucket's fluid-ray fill survives a mob under the crosshair. An empty
    /// bucket main, a shield off, a fish under the crosshair and a water source on the
    /// fluid ray: the bucket keeps the click and the shield does not raise. The same
    /// beside a cow that was just milked.
    #[test]
    fn a_buckets_fill_survives_a_mob_under_the_crosshair() {
        use crate::mob::MobType;
        let (bucket, shield) = (Item::Material(MaterialId::Bucket), tool(ToolType::Shield));
        let with_water = |t: UseTarget| UseTarget { bucket_fills: true, ..t };
        let fish = with_water(at_mob(MobType::Fish));
        assert_eq!(hand_with_use(Some(&bucket), Some(&shield), &fish), Some(Hand::Main), "the bucket fills");
        assert!(!shield_raises(Some(&bucket), Some(&shield), &fish), "the shield does not raise");
        let milked = with_water(facts_at(crate::mob_uses::MobFacts { product_ready: false, ..crate::mob_uses::MobFacts::of(MobType::Cow) }));
        assert_eq!(hand_with_use(Some(&bucket), Some(&shield), &milked), Some(Hand::Main), "beside a cow just milked");
        assert!(!shield_raises(Some(&bucket), Some(&shield), &milked));
        // No water on the ray: the bucket has no use at the fish, and the shield raises.
        assert!(shield_raises(Some(&bucket), Some(&shield), &at_mob(MobType::Fish)));
        // A ready cow is still milked (the mob's own use), with or without water.
        assert_eq!(hand_with_use(Some(&bucket), Some(&shield), &at_mob(MobType::Cow)), Some(Hand::Main));
        // The block uses the fill path shares: a tappable log, unless the mode cannot edit.
        let log = UseTarget { block: Some(block::RUBBER_LOG), ..UseTarget::default() };
        assert_eq!(hand_with_use(Some(&bucket), Some(&shield), &log), Some(Hand::Main));
        assert!(shield_raises(Some(&bucket), Some(&shield), &UseTarget { edit_blocked: true, ..log }));
        // From the off-hand too.
        assert_eq!(hand_with_use(Some(&shield), Some(&bucket), &fish), Some(Hand::Off));
    }

    /// Finding 3: whether a hand can take a Lead off depends on the item it holds and on
    /// whether THAT hand's harvest refused, worked out per hand. Shears main (the wool is
    /// still growing) and a shield off, on a tethered sheep: the click is not swallowed,
    /// the Lead comes off through the off-hand and the shield does not raise. Shears that
    /// can cut keep the click, as the chain has it.
    #[test]
    fn a_hand_whose_harvest_refused_hands_the_lead_to_the_other_hand() {
        use crate::mob::MobType;
        let (shears, shield, sword) = (tool(ToolType::Shears), tool(ToolType::Shield), tool(ToolType::Sword));
        let tied = |ready| facts_at(crate::mob_uses::MobFacts { tethered: true, product_ready: ready, ..crate::mob_uses::MobFacts::of(MobType::Sheep) });
        let growing = tied(false);
        assert_eq!(hand_with_use(Some(&shears), Some(&shield), &growing), Some(Hand::Off), "the Lead comes off through the off-hand");
        assert_eq!(use_hand(Some(&shears), Some(&shield), &growing), Hand::Off);
        assert!(!shield_raises(Some(&shears), Some(&shield), &growing), "the shield does not raise");
        assert_eq!(hand_with_use(Some(&shears), Some(&sword), &growing), Some(Hand::Off), "any non-Lead item takes it off");
        // Nothing to take the click: not swallowed by the refused hand, not claimed by an empty off-hand.
        assert_eq!(hand_with_use(Some(&shears), None, &growing), None);
        // The existing order when the shears can cut.
        assert_eq!(hand_with_use(Some(&shears), Some(&shield), &tied(true)), Some(Hand::Main));
        // A shield main on the growing sheep detaches from the main hand (it is not shears).
        assert_eq!(hand_with_use(Some(&shield), Some(&shears), &growing), Some(Hand::Main));
        // A Lead in a hand never takes a Lead off: the other hand does.
        let lead = Item::Material(MaterialId::Lead);
        let tied_brigand = facts_at(crate::mob_uses::MobFacts { tethered: true, ..crate::mob_uses::MobFacts::of(MobType::Brigand) });
        assert_eq!(hand_with_use(Some(&lead), Some(&sword), &tied_brigand), Some(Hand::Off));
    }

    // --- Decision B: hold persistence ---

    #[test]
    fn once_raised_the_shield_stays_up_across_target_changes_until_the_button_is_released() {
        let ask = |answer: bool| move || answer;
        let up = ShieldHold { raised: true, held_over: false };
        let down = ShieldHold { raised: false, held_over: false };
        // The press: the target check decides.
        assert_eq!(shield_press(Press::Idle, true, true, ask(true)), (up, Press::Decided { raised: true }));
        assert_eq!(shield_press(Press::Idle, true, true, ask(false)), (down, Press::Decided { raised: false }), "a press on a door never raises");
        // Held: whatever comes under the crosshair (a cow that takes the click, a door), it stays up,
        // and the target is not asked again.
        let held = Press::Decided { raised: true };
        let (h, p) = shield_press(held, true, true, || panic!("the target is asked once per press"));
        assert_eq!((h, p), (ShieldHold { raised: true, held_over: true }, held));
        // Release lowers it, and the next press asks the target again.
        assert_eq!(shield_press(held, false, true, ask(true)), (down, Press::Idle));
        assert_eq!(shield_press(Press::Idle, true, true, ask(true)).0, up);
        // No shield in a hand (swapped away mid-hold): nothing to hold up, and it stays down for the press.
        let (h, p) = shield_press(held, true, false, ask(true));
        assert_eq!((h, p), (down, Press::Decided { raised: false }));
        assert_eq!(shield_press(p, true, true, || panic!("not asked again")).0, down);
        // A whole walk: raise on air, turn over a door and a cow, release.
        let mut press = Press::Idle;
        for (holding, raises_here, want) in [(true, true, true), (true, false, true), (true, false, true), (true, true, true), (false, false, false), (true, false, false)] {
            let (hold, next) = shield_press(press, holding, true, ask(raises_here));
            press = next;
            assert_eq!(hold.raised, want);
        }
    }

    /// Review 4 finding 5: the press is classified once. A shield main and wheat off,
    /// a press on a breedable cow: the cow is fed and the shield stays down for the whole
    /// press, even though on the next frame the cow is in love and wheat no longer claims
    /// it. Release and press again: the shield raises.
    #[test]
    fn a_press_decided_as_a_feed_never_becomes_a_raise_until_released() {
        use crate::mob::MobType;
        let (shield, wheat) = (tool(ToolType::Shield), Item::Material(MaterialId::Wheat));
        let breedable = at_mob(MobType::Cow);
        let in_love = UseTarget { mob_facts: Some(crate::mob_uses::MobFacts { can_breed: false, ..crate::mob_uses::MobFacts::of(MobType::Cow) }), ..breedable };
        let asks = |t: &UseTarget| shield_raises(Some(&shield), Some(&wheat), t);
        assert!(!asks(&breedable) && asks(&in_love), "the premise: the same cow, fed, would now raise");
        let frame = |press, target: &UseTarget, holding| shield_press(press, holding, true, || asks(target));
        let (hold, press) = frame(Press::Idle, &breedable, true);
        assert!(!hold.raised, "the press frame feeds");
        // Still held: the cow is in love now. The shield stays down.
        let (hold, press) = frame(press, &in_love, true);
        assert!(!hold.raised, "same press: still down");
        let (hold, press) = frame(press, &in_love, true);
        assert!(!hold.raised, "and the next frame");
        // Release, press again on the cow in love: the shield raises.
        let (hold, press) = frame(press, &in_love, false);
        assert_eq!((hold.raised, press), (false, Press::Idle));
        let (hold, _) = frame(press, &in_love, true);
        assert!(hold.raised, "a new press asks again");
    }

    /// Review 4 finding 6: a Strict seat's off-hand counts only as a shield, so a
    /// shield main and a torch off, clicking stone, raises (the torch cannot be
    /// placed on a Strict seat) while an unrestricted seat places the torch.
    #[test]
    fn a_strict_seat_raises_a_shield_past_an_off_hand_use_it_cannot_run() {
        let (shield, torch, sword) = (tool(ToolType::Shield), Item::Block(block::TORCH), tool(ToolType::Sword));
        let raises = |strict, main: &Item, off: &Item| shield_raises(Some(main), usable_off_hand(strict, Some(off)), &stone_target());
        assert!(!raises(false, &shield, &torch), "a Mirror seat places the torch");
        assert!(raises(true, &shield, &torch), "a Strict seat raises: the torch never runs there");
        // An off-hand shield is still a raise candidate on a Strict seat.
        assert!(raises(true, &sword, &shield));
        assert_eq!(usable_off_hand(true, Some(&shield)), Some(&shield));
        assert_eq!(usable_off_hand(true, Some(&torch)), None);
        assert_eq!(usable_off_hand(false, Some(&torch)), Some(&torch));
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
