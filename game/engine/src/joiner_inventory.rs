//! C1 (2026-10-07) — a joiner's inventory as the SERVER keeps it: merge 1 of 3
//! of inventory authority (owner O-7 #2: per-npub persistence must save the
//! server's copy, never a client-asserted snapshot).
//!
//! `ServerPlayer.inventory` is a maintained SHADOW of a server-simulated
//! player's inventory, fed by every change the server itself decides:
//!
//! - gains: server-side pickups (`entity::tick_item_pickups`), the drops of
//!   the joiner's breaks (`break_drops`, yielded by the server), and the
//!   products of its accepted interactions (D2b) — each also sent to the
//!   client as an `InventoryGrant`;
//! - consumes: a plain block placement takes one from the held hotbar slot
//!   ([`check_placement`]), and an accepted interaction takes what its
//!   `InteractOutcome.consume_held` says, owed from wherever the item is
//!   (`joiner_actions::take_owed`, the same function the client runs).
//!
//! Still the client's alone (the shadow does not see them): the inventory it
//! joined with, crafting, chests and furnaces, Q-drops, eating, tool and
//! armour wear, armour equip, client-side pickups, face-attachment and
//! drying-rack recovery, and moving stacks between slots. So the shadow
//! drifts, and the possession check on placements is LOG-ONLY for one
//! release ([`PossessionTally`]): it counts and logs a mismatch, never
//! refuses or corrects. Enforcement waits for the remaining gains to reach
//! the server (merges 2 and 3).

use crate::block::BlockId;
use crate::crafting::Tool;
use crate::inventory::Inventory;
use crate::item::Item;
use crate::protocol::MinedBlock;

/// Ticks between two possession-mismatch log lines for one player (5 s at
/// 20 TPS). Mismatches in between are counted and reported with the next.
pub const MISMATCH_LOG_INTERVAL_TICKS: u64 = 100;

/// What an accepted edit from a server-simulated player is to its inventory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JoinerEdit {
    /// A survival break its client mined (`InputPacket.mined`): the server
    /// yields it with the tool the client mined with.
    Break { tool: Option<Tool> },
    /// A plain block-item placement: checked against the shadow's held slot,
    /// and one consumed from it.
    Place,
    /// Neither: creative, a non-block placement (bucket, seeds, flint, bone
    /// meal, a hoe's tilling…), a meta-only toggle, or a side effect of the
    /// client's own sim. Counted, not checked.
    Unchecked,
}

/// A block no item places: what a non-block placement or a sim leaves in a
/// cell (fluids from a bucket, fire from flint, smoke, a crop from seeds, a
/// piston's arm). With a held block-item that places exactly this block it
/// is still a plain placement (a farmed flower's mature block is also a
/// flower item).
fn side_effect_block(b: BlockId) -> bool {
    crate::block::is_fluid(b)
        || matches!(b, crate::block::FIRE | crate::block::CAMPFIRE_SMOKE | crate::block::PISTON_HEAD)
        || crate::growth::is_crop(b)
}

/// The block an item places, if it places one (`Inventory::hotbar_placeable_id`'s rule).
pub fn placeable_block(item: &Item) -> Option<BlockId> {
    match item {
        Item::Block(b) => Some(*b),
        Item::Material(m) => crate::item::material_as_placeable_block(*m),
        Item::Tool(_) | Item::Plan(_) | Item::Armour(_) => None,
    }
}

/// What a player's input says is in hand, as far as its `held_kind`/`held_id`
/// pair tells: blocks and materials survive the pair; a tool keeps only its
/// tier (so it is known to be a tool, not which); armour and a Plan read as
/// an empty hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Empty,
    /// A block-item, or a material that places as this block.
    Places(BlockId),
    /// Anything else: a tool, a bucket, seeds, flint, bone meal…
    Other,
}

impl Hand {
    /// Decode the input's held pair (`InputPacket.held_kind`/`held_id`).
    pub fn from_wire(kind: u8, id: u16, registry: &crate::block::BlockRegistry) -> Self {
        if kind == crate::protocol::item_kind::EMPTY {
            return Hand::Empty;
        }
        match crate::inventory::item_from_ref(kind, id, registry).as_ref().and_then(placeable_block) {
            Some(b) => Hand::Places(b),
            None => Hand::Other,
        }
    }
}

/// Classify an accepted edit `old → new` from a server-simulated player.
///
/// `mined` is the client's tag for the cell, if it sent one; `hand` what its
/// input says is in hand; `creative` the world's mode.
///
/// - A tagged cell whose edit empties it (or leaves a harvested crop's
///   replacement) is a `Break`. Untagged emptying edits are never one: a
///   bucket scoop, an Eraser, a lifted Latent Print, or a cell the client's
///   own pistons or kegs cleared would otherwise mint drops.
/// - An edit filling an empty (or water) cell is a `Place` when the hand
///   holds a block-item or nothing (the last of a stack leaves the hand
///   empty), unless the block is one no item places and the hand isn't
///   holding exactly it. A hand holding a non-placeable (tool, bucket, seeds,
///   flint, bone meal) makes it `Unchecked`.
pub fn classify(
    old: BlockId,
    new: BlockId,
    mined: Option<&MinedBlock>,
    hand: Hand,
    creative: bool,
) -> JoinerEdit {
    use crate::block::{AIR, WATER};
    if creative || old == new {
        return JoinerEdit::Unchecked;
    }
    if let Some(m) = mined {
        if old != AIR && (new == AIR || crate::growth::is_crop(old)) {
            let tool = match crate::inventory::item_from_wire_full(&m.tool) {
                Some(Item::Tool(t)) => Some(t),
                _ => None,
            };
            return JoinerEdit::Break { tool };
        }
        return JoinerEdit::Unchecked;
    }
    if !(old == AIR || old == WATER) || new == AIR {
        return JoinerEdit::Unchecked;
    }
    match hand {
        Hand::Places(b) if b == new => JoinerEdit::Place,
        Hand::Places(_) | Hand::Empty if !side_effect_block(new) => JoinerEdit::Place,
        _ => JoinerEdit::Unchecked,
    }
}

/// The possession check's verdict on one plain placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceCheck {
    /// The shadow's held slot places this block; one was taken from it.
    Matched,
    /// It doesn't: `held` is what it places, if anything. Nothing taken.
    Mismatched { held: Option<BlockId> },
}

/// C1 — a joiner's plain placement of `placed` against the shadow's held
/// hotbar slot (`hotbar_placeable_id`). On a match the shadow consumes one
/// from that slot exactly as the client's placement did
/// (`take_placeable_from_hotbar`, auto-refill included). On a mismatch it
/// takes nothing — the check is log-only and never corrects the shadow.
pub fn check_placement(inv: &mut Inventory, hotbar_slot: usize, placed: BlockId) -> PlaceCheck {
    match inv.hotbar_placeable_id(hotbar_slot) {
        Some(b) if b == placed => {
            inv.take_placeable_from_hotbar(hotbar_slot);
            PlaceCheck::Matched
        }
        held => PlaceCheck::Mismatched { held },
    }
}

/// Per-connection counters of the log-only possession check, readable by
/// tests and summarised in the server log when the player leaves
/// ([`Self::summary`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PossessionTally {
    /// Breaks the server yielded.
    pub breaks: u32,
    /// Plain placements the shadow's held slot backed.
    pub matched: u32,
    /// Plain placements it didn't back, and accepted interactions whose
    /// `consume_held` it couldn't pay (logged, accepted).
    pub mismatched: u32,
    /// Edits not checked: creative, non-block placements, meta toggles, side
    /// effects, a tagged break whose edit didn't match the server's yield.
    pub unchecked: u32,
    /// Mismatches since the last log line.
    suppressed: u32,
    /// When the last log line went out.
    last_log_tick: Option<u64>,
}

impl PossessionTally {
    /// Count a mismatch on tick `now`. `Some(n)` when a log line is due:
    /// `n` more mismatches were counted (and not logged) since the last one.
    pub fn note_mismatch(&mut self, now: u64) -> Option<u32> {
        self.mismatched = self.mismatched.saturating_add(1);
        let due = self
            .last_log_tick
            .is_none_or(|t| now.saturating_sub(t) >= MISMATCH_LOG_INTERVAL_TICKS);
        if !due {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.last_log_tick = Some(now);
        Some(std::mem::take(&mut self.suppressed))
    }

    /// The one-line summary logged when `label` leaves, or `None` if nothing
    /// was counted.
    pub fn summary(&self, label: &str) -> Option<String> {
        if [self.breaks, self.matched, self.mismatched, self.unchecked].iter().all(|&n| n == 0) {
            return None;
        }
        Some(format!(
            "{label} left — possession check (log-only): {} placement(s) matched, {} mismatched \
             (placements or interactions), {} edit(s) unchecked; {} break(s) yielded by the server",
            self.matched, self.mismatched, self.unchecked, self.breaks
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::crafting::{ToolMaterial, ToolType};
    use crate::item::{ItemStack, MaterialId};
    use crate::protocol::WireItem;

    /// `item` in hand, decoded as the server decodes an input's held pair.
    fn hand(item: &Item) -> Hand {
        let (kind, id) = crate::inventory::item_to_ref(item).to_wire();
        Hand::from_wire(kind, id, &crate::block::BlockRegistry::new())
    }

    fn tag(tool: Option<Tool>) -> MinedBlock {
        MinedBlock {
            x: 0,
            y: 0,
            z: 0,
            tool: tool.map_or(WireItem::None, |t| crate::inventory::item_to_wire_full(&Item::Tool(t))),
        }
    }

    #[test]
    fn only_a_mined_cell_is_a_break_and_it_carries_its_tool() {
        let pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
        let t = tag(Some(pick));
        match classify(block::STONE, block::AIR, Some(&t), Hand::Empty, false) {
            JoinerEdit::Break { tool: Some(got) } => {
                assert_eq!((got.tool_type, got.material), (ToolType::Pickaxe, ToolMaterial::Stone));
            }
            other => panic!("expected a break with its pickaxe, got {other:?}"),
        }
        // A harvested crop leaves its replacement, still a break.
        assert!(matches!(
            classify(block::WHEAT_STAGE_3, block::TILLED_SOIL, Some(&tag(None)), Hand::Empty, false),
            JoinerEdit::Break { tool: None }
        ));
        // The same emptying edits, untagged, are never breaks: a bucket
        // scoop, an Eraser, a piston or keg the client's own sim ran.
        let bucket = Item::Material(MaterialId::Bucket);
        assert_eq!(classify(block::LAVA, block::AIR, None, hand(&bucket), false), JoinerEdit::Unchecked);
        assert_eq!(classify(block::STONE, block::AIR, None, Hand::Empty, false), JoinerEdit::Unchecked);
        // Creative never yields.
        assert_eq!(classify(block::STONE, block::AIR, Some(&t), Hand::Empty, true), JoinerEdit::Unchecked);
    }

    #[test]
    fn a_block_item_filling_an_empty_cell_is_a_placement() {
        let stone = Item::Block(block::STONE);
        assert_eq!(classify(block::AIR, block::STONE, None, hand(&stone), false), JoinerEdit::Place);
        assert_eq!(classify(block::WATER, block::STONE, None, hand(&stone), false), JoinerEdit::Place);
        // Logs place from their material.
        let log = Item::Material(MaterialId::GreenLog);
        assert_eq!(classify(block::AIR, block::OAK_LOG, None, hand(&log), false), JoinerEdit::Place);
        // The last of a stack leaves the hand empty.
        assert_eq!(classify(block::AIR, block::STONE, None, Hand::Empty, false), JoinerEdit::Place);
        // Non-block placements and side effects are unchecked.
        let bucket = Item::Material(MaterialId::WaterBucket);
        assert_eq!(classify(block::AIR, block::WATER, None, hand(&bucket), false), JoinerEdit::Unchecked);
        let seeds = Item::Material(MaterialId::WheatSeeds);
        assert_eq!(classify(block::AIR, block::WHEAT_STAGE_0, None, hand(&seeds), false), JoinerEdit::Unchecked);
        assert_eq!(classify(block::AIR, block::CAMPFIRE_SMOKE, None, Hand::Empty, false), JoinerEdit::Unchecked);
        let pick = Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Wood));
        assert_eq!(classify(block::AIR, block::STONE, None, hand(&pick), false), JoinerEdit::Unchecked);
        // Not into an occupied cell, and never in creative.
        assert_eq!(classify(block::DIRT, block::TILLED_SOIL, None, Hand::Empty, false), JoinerEdit::Unchecked);
        assert_eq!(classify(block::AIR, block::STONE, None, hand(&stone), true), JoinerEdit::Unchecked);
    }

    #[test]
    fn a_matched_placement_consumes_one_and_a_mismatch_consumes_nothing() {
        let mut inv = Inventory::new();
        inv.set_slot(2, Some(ItemStack::new_block(block::STONE, 3)));
        assert_eq!(check_placement(&mut inv, 2, block::STONE), PlaceCheck::Matched);
        assert_eq!(inv.slot(2).unwrap().count, 2);
        assert_eq!(
            check_placement(&mut inv, 2, block::DIRT),
            PlaceCheck::Mismatched { held: Some(block::STONE) }
        );
        assert_eq!(inv.slot(2).unwrap().count, 2, "log-only: never corrected");
        assert_eq!(check_placement(&mut inv, 5, block::DIRT), PlaceCheck::Mismatched { held: None });
    }

    #[test]
    fn mismatch_logs_are_rate_limited_and_count_what_they_skip() {
        let mut t = PossessionTally::default();
        assert_eq!(t.note_mismatch(1_000), Some(0), "the first one is logged");
        assert_eq!(t.note_mismatch(1_001), None);
        assert_eq!(t.note_mismatch(1_050), None);
        assert_eq!(t.note_mismatch(1_000 + MISMATCH_LOG_INTERVAL_TICKS), Some(2));
        assert_eq!(t.mismatched, 4, "every mismatch is counted");
        assert!(PossessionTally::default().summary("Visitor").is_none());
        assert!(t.summary("Visitor").unwrap().contains("4 mismatched"));
    }
}
