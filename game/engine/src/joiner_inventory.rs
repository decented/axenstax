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
//!   (`joiner_actions::take_owed`, the same function the client runs); and,
//!   C2a, the food of an accepted `ItemAction::Eat` (`item_actions`);
//! - C2b: a Q-drop is taken (`ItemAction::Drop`, owed; the item becomes a
//!   real ground item);
//! - C3a-2a: every window op the client applies — slot moves, drags, sort,
//!   locks, trash, armour equip, the craft result click, opening a screen,
//!   its auto-refill setting — is applied to the server's copy of the window
//!   by the same rule (`window_ops`), and a hit the server lands wears the
//!   server's copy of the armour;
//! - C3c-1: a block-edit use (a bucket filled or emptied, a seed or reed,
//!   bone meal, fertiliser, salt, an Eraser, a rubber tap, a hoe) takes what
//!   it used, wears its tool and adds its product (`use_edits`, by the use
//!   tag its edit carries; tallied in [`PossessionTally::use_mirrored`]).
//!
//! Still the client's alone (the shadow does not see them): the inventory it
//! joined with, face-attachment recovery, and the local uses C3c-2 and C3c-3
//! carry (a bow, a fishing rod, a campfire lit, Plans; a break's and a
//! swing's tool wear is mirrored since C3a-2b; the full list of gaps, which
//! must close before enforcement: Spec 04 §4.2e). So the shadow
//! drifts, and the possession check on placements is LOG-ONLY for one
//! release ([`PossessionTally`]): it counts and logs a mismatch (debug; a
//! warning at most once a minute per player), never refuses or corrects.
//! Enforcement waits for the remaining gains to reach the server (merges 2
//! and 3).

use crate::block::BlockId;
use crate::crafting::Tool;
use crate::inventory::Inventory;
use crate::item::Item;
use crate::protocol::MinedBlock;

/// Ticks between two possession-mismatch WARNINGS for one player (one minute
/// at 20 TPS; review LOW-6 — the shadow starts empty, so a building joiner
/// mismatches all the time this release). Every mismatch in between is a
/// debug line ([`mismatch_log_level`]), counted and reported with the next
/// warning.
pub const MISMATCH_LOG_INTERVAL_TICKS: u64 = 60 * 20;

/// What an accepted edit from a server-simulated player is to its inventory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JoinerEdit {
    /// A survival break its client mined (`InputPacket.mined`): the server
    /// yields it with the tool the client mined with.
    Break { tool: Option<Tool> },
    /// A plain block-item placement: checked against the shadow's held slot,
    /// and one consumed from it.
    Place,
    /// Neither: creative, a non-block placement its client sent no use tag
    /// for (flint and steel, a friction stick; a tagged use is mirrored by
    /// `use_edits` instead, C3c-1), a meta-only toggle, or a side effect of
    /// the client's own sim. Counted, not checked.
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

/// A block whose break yields anything (review LOW-1): the shared break
/// rules' own test (`break_drops::yields_drops`, FU2) — not an empty cell, a
/// fluid, fire or smoke. A survival break can target LAVA and FIRE and dig
/// them up, but they are no items, for a joiner as in single-player; a tag on
/// an empty, water or smoke cell is a modified client's. Either way the edit
/// is no `Break` and yields nothing.
fn minable(b: BlockId) -> bool {
    crate::break_drops::yields_drops(b)
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
///   replacement) is a `Break` — if the block is one a break can mine
///   (not a fluid, fire or smoke: [`minable`]). Untagged emptying edits are never one: a
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
        if minable(old) && (new == AIR || crate::growth::is_crop(old)) {
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

/// The verdict on wearing a joiner's tool in the shadow ([`wear_tool`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WearCheck {
    /// The slot held that tool; it wore (and is gone from the slot if that
    /// was its last use, `broke`).
    Worn { broke: bool },
    /// The slot holds no tool of that type and material. Nothing wore.
    Mismatched,
}

/// C3a-2b — wear `tool`, the one a joiner's break or swing used, in the
/// shadow's slot `slot`: the client's own rule (`Inventory::use_tool_at`,
/// which every client break arm reaches through `use_hotbar_tool`), run on
/// the same slot the client used. A slot that doesn't hold a tool of the same
/// type and material (a tool is the same tool by those; durability is what
/// wears) wears nothing and is a [`WearCheck::Mismatched`]; log-only, like the
/// placement check — the shadow doesn't yet see the inventory the joiner
/// arrived with or its slot moves.
pub fn wear_tool(inv: &mut Inventory, slot: usize, tool: &Tool) -> WearCheck {
    let holds = matches!(
        inv.slot(slot).map(|s| &s.item),
        Some(Item::Tool(t)) if t.tool_type == tool.tool_type && t.material == tool.material
    );
    if !holds {
        return WearCheck::Mismatched;
    }
    match inv.use_tool_at(slot) {
        Some(info) => WearCheck::Worn { broke: info.just_broke },
        None => WearCheck::Mismatched,
    }
}

/// The level a possession-mismatch line is logged at, given what
/// [`PossessionTally::note_mismatch`] said: a warning when one is due (at most
/// one a minute per player), otherwise a debug line.
pub fn mismatch_log_level(due: Option<u32>) -> log::Level {
    if due.is_some() {
        log::Level::Warn
    } else {
        log::Level::Debug
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
    /// C3a-2a — window ops applied to the server's copy of the window.
    pub window_ops: u32,
    /// C3a-fix-2 B-L4 — of them, ops the server's rule refused (`Refused`,
    /// `NeedsTable`) after which its window digest equals the client's: the
    /// client's rule refused too (a Trash with an empty cursor, a paint over
    /// a slot that won't take it, a Result with no recipe). Benign; the
    /// window stays as the rule leaves it, on both sides. Not counted for a
    /// creative joiner.
    pub window_noop: u32,
    /// C3a-2a — of them, ops the server's rule refused but the client's
    /// digest says it did NOT (the digests differ): a table the client still
    /// sees, a click a modified client sent. Not counted for a creative
    /// joiner.
    pub window_refused: u32,
    /// C3b-1 — container ops (`WireWindowOp::Container`) refused because no
    /// container was open on the server, or its cell was gone or out of
    /// reach (C3b-fix-a: or the client's mirror showed another container, a
    /// claim counted above its stack, or a believed deposit went past the
    /// joiner's bound). Container convergence, not lockstep (C-L4).
    pub container_refused: u32,
    /// C3b-1 — `WindowSlotSet` corrections sent after a container op whose
    /// result differed (someone else got there first, or a refusal's
    /// revert). C3b-fix-a (C-L4) — container convergence, tallied apart from
    /// `window_mismatch` (per-joiner lockstep, the C3d gate).
    pub container_corrected: u32,
    /// C3b-1 (v77) — units a joiner put into a shared container, by its
    /// claims, that the server's copy of its inventory didn't hold: believed
    /// deposits (BRIDGE until C3d; a locally fished item, unmirrored until
    /// C3c, is the honest case).
    pub container_believed: u32,
    /// C3b-fix-e (C-M1) — units a container op put in by the client's claim
    /// of a tool or armour piece the server's copy holds only at another
    /// durability (worn on the client by a use not mirrored until C3c),
    /// paired with the copy's own piece of that kind that its own run of the
    /// op deposited: a swap — the container keeps the claimed piece and the
    /// copy gives up its own (`window_ops::serve_container`). Each costs the
    /// believed bound like a believed unit, but is tallied here, not in
    /// `container_believed` (C3d gate 2's counter), so honest drift doesn't
    /// pollute it.
    pub durability_swap: u32,
    /// C3b-2-fix (M2) — units an accepted block use (`ItemAction::UseBlock`)
    /// took that the server's copy of the joiner's window didn't hold:
    /// believed within the joiner's bound (`window_ops::believe_pay`; BRIDGE
    /// until C3d).
    pub use_believed: u32,
    /// C3b-2-fix (M2) — block uses refused because their believed take went
    /// past the joiner's bound.
    pub use_refused: u32,
    /// C3b-fix-c (B-M1) — units a container op's correction took short from
    /// the server's copy of the window: the phantom had already gone some
    /// other way (placed, dropped, eaten, crafted) before the correction
    /// landed. Owed (`container_window::CorrectionDebt`), and a dupe unless a
    /// later correction give pays it; it conserves nothing until C3d.
    /// C3b-fix-e (L4) — the NET shortfall: a later correction give that pays
    /// such a debt (the phantom was deposited back inside the round trip)
    /// takes what it paid off again (`window_events::note_correction_paid`),
    /// so honest fast looting leaves it at 0 and what stays counted is a
    /// shortfall nothing paid.
    pub correction_short: u32,
    /// C3a-2a — ops after which the server's window digest differed from the
    /// one the client sent (log-only). Not counted for a creative joiner,
    /// whose item browser gives stay local until C3c.
    pub window_mismatch: u32,
    /// C3a-2a — the kind of the first op that mismatched.
    pub first_window_mismatch: Option<crate::window_ops::OpKind>,
    /// C3a-2a — `ItemAction::Craft`s ignored: unused since v75 (the craft is
    /// the window's result click).
    pub crafts_ignored: u32,
    /// C2b — Q-drops spawned as ground items.
    pub drops: u32,
    /// C2b-fix — units a grant (a break's yield, an interaction's product, a
    /// server pickup) gave the client that the shadow had no room for. Counted,
    /// never spilled: the client holds them.
    pub grant_overflow: u32,
    /// C3a-2b — tool wear the shadow couldn't apply: a break's or swing's
    /// tool the shadow's slot didn't hold ([`wear_tool`]). Logged at debug
    /// only; never refused.
    pub wear_mismatch: u32,
    /// C3c-1 — block-edit uses (`use_edits`) mirrored on the copy: a legal
    /// outcome whose cost, tool and product all matched.
    pub use_mirrored: u32,
    /// C3c-1 — uses that didn't mirror cleanly: an outcome the use's rule
    /// can't produce, an item the copy didn't hold, a tool not in the slot.
    /// Applied all the same (log-only). C3c-1-fix (L-1) — honest drift (an
    /// outcome the server's world had moved on from, mirrored all the same)
    /// and a tag that can't explain its edit (classified as an ordinary
    /// edit) count here too.
    pub use_mismatch: u32,
    /// C3c-1-fix (M-2) — units of a use's `product − unfit` the server's
    /// copy had no room for: the client holds them; counted, never spawned.
    pub use_copy_overflow: u32,
    /// C3c-1-fix (M-2) — units of a use's `unfit` (the part the client's bag
    /// couldn't take) the copy couldn't corroborate, spawned believed within
    /// the joiner's believed bound (`window_ops::BelievedBucket`; BRIDGE
    /// until C3d).
    pub use_unfit_believed: u32,
    /// C3c-1-fix (M-2) — uses whose unfit spawn went past that bound: nothing
    /// was spawned.
    pub use_unfit_refused: u32,
    /// C3c-1-fix (M-4) — use-tagged edits refused (reach, a plot, the play
    /// mode, a door's top half with no door below): the joiner was told, and
    /// undoes them.
    pub use_edit_refused: u32,
    /// C3c-1 — the use kinds already logged a mismatch for
    /// (`use_edits::UseKind::bit`): one line per kind per connection.
    uses_logged: u16,
    /// Mismatches since the last warning.
    suppressed: u32,
    /// When the last warning went out.
    last_log_tick: Option<u64>,
}

impl PossessionTally {
    /// Count a mismatch on tick `now`. `Some(n)` when a warning is due: `n`
    /// more mismatches were counted (and logged at debug only) since the last
    /// one.
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

    /// C3a-2b — count a tool-wear verdict: a mismatch is tallied (the shadow
    /// wore nothing), a wear is not.
    pub fn note_wear(&mut self, check: WearCheck) {
        if check == WearCheck::Mismatched {
            self.wear_mismatch = self.wear_mismatch.saturating_add(1);
        }
    }

    /// C3c-1 — count a use's verdict (`miss` = `None` for a clean mirror).
    /// `true` when this mismatch is the first of its kind this connection,
    /// so it is logged.
    pub fn note_use(&mut self, kind: crate::use_edits::UseKind, miss: Option<crate::use_edits::UseMiss>) -> bool {
        if miss.is_none() {
            self.use_mirrored = self.use_mirrored.saturating_add(1);
            return false;
        }
        self.use_mismatch = self.use_mismatch.saturating_add(1);
        let first = self.uses_logged & kind.bit() == 0;
        self.uses_logged |= kind.bit();
        first
    }

    /// C3a-2a — count a window op whose digest differed; `true` for the
    /// first one this connection.
    pub fn note_window_mismatch(&mut self, kind: crate::window_ops::OpKind) -> bool {
        self.window_mismatch = self.window_mismatch.saturating_add(1);
        let first = self.first_window_mismatch.is_none();
        self.first_window_mismatch.get_or_insert(kind);
        first
    }

    /// The one-line summary logged when `label` leaves, or `None` if nothing
    /// was counted.
    pub fn summary(&self, label: &str) -> Option<String> {
        let window = [self.window_ops, self.window_noop, self.window_refused, self.window_mismatch];
        let c2b = [self.drops, self.grant_overflow];
        let c3b = [
            self.container_refused,
            self.container_corrected,
            self.container_believed,
            self.durability_swap,
            self.correction_short,
            self.use_believed,
            self.use_refused,
        ];
        let uses = [
            self.use_mirrored,
            self.use_mismatch,
            self.use_copy_overflow,
            self.use_unfit_believed,
            self.use_unfit_refused,
            self.use_edit_refused,
        ];
        if [self.breaks, self.matched, self.mismatched, self.unchecked, self.crafts_ignored, self.wear_mismatch]
            .iter()
            .chain(&uses)
            .chain(&window)
            .chain(&c2b)
            .chain(&c3b)
            .all(|&n| n == 0)
        {
            return None;
        }
        let mut line = format!(
            "{label} left — possession check (log-only): {} placement(s) matched, {} mismatched \
             (placements or interactions), {} edit(s) unchecked; {} break(s) yielded by the server",
            self.matched, self.mismatched, self.unchecked, self.breaks
        );
        if self.wear_mismatch > 0 {
            line.push_str(&format!(
                "; {} tool use(s) the shadow's slot didn't hold a tool for",
                self.wear_mismatch
            ));
        }
        if uses.iter().any(|&n| n > 0) {
            line.push_str(&format!(
                "; {} block-edit use(s) mirrored, {} use(s) that didn't match the server's copy, \
                 {} product unit(s) the copy had no room for, {} unfit unit(s) spawned believed, \
                 {} unfit spawn(s) past the believed bound, {} use edit(s) refused",
                self.use_mirrored,
                self.use_mismatch,
                self.use_copy_overflow,
                self.use_unfit_believed,
                self.use_unfit_refused,
                self.use_edit_refused
            ));
        }
        if window.iter().any(|&n| n > 0) {
            let first = self.first_window_mismatch.map_or(String::new(), |k| format!(" (first: {})", k.label()));
            line.push_str(&format!(
                "; {} window op(s) mirrored, {} no-op(s) both rules refused, {} refused by the server alone, \
                 {} digest mismatch(es){first}",
                self.window_ops, self.window_noop, self.window_refused, self.window_mismatch
            ));
        }
        if c3b.iter().any(|&n| n > 0) {
            line.push_str(&format!(
                "; {} container op(s) refused, {} container correction(s) sent, {} unit(s) deposited believed, \
                 {} drifted tool(s) or armour piece(s) deposited as a swap, \
                 {} unit(s) a container correction took short (net of later gives); {} unit(s) a block use took believed, \
                 {} block use(s) refused past the believed bound",
                self.container_refused,
                self.container_corrected,
                self.container_believed,
                self.durability_swap,
                self.correction_short,
                self.use_believed,
                self.use_refused
            ));
        }
        if c2b.iter().any(|&n| n > 0) {
            line.push_str(&format!(
                "; {} Q-drop(s) spawned, {} granted unit(s) didn't fit",
                self.drops, self.grant_overflow
            ));
        }
        if self.crafts_ignored > 0 {
            line.push_str(&format!("; {} pre-v75 craft message(s) ignored", self.crafts_ignored));
        }
        Some(line)
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

    /// Review LOW-1 / FU2 — lava and fire dug up are no items (the shared
    /// `break_drops::yields_drops`), and water, smoke and an empty cell are
    /// never mined: a `mined` tag on one is no break and yields nothing.
    #[test]
    fn a_mined_tag_on_a_fluid_fire_smoke_or_air_cell_is_no_break() {
        let t = tag(Some(Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond)));
        for old in [block::WATER, block::LAVA, block::FIRE, block::CAMPFIRE_SMOKE, block::AIR] {
            assert_eq!(classify(old, block::AIR, Some(&t), Hand::Empty, false), JoinerEdit::Unchecked, "{old}");
        }
        // A minable block beside them still is.
        assert!(matches!(classify(block::STONE, block::AIR, Some(&t), Hand::Empty, false), JoinerEdit::Break { .. }));
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
        assert!(!t.summary("Visitor").unwrap().contains("craft"), "no craft part until one is counted");
    }

    /// C3a-2a (was C2b's craft tally) — window ops (mirrored, no-ops,
    /// refused by the server alone, digest mismatches with the first one's kind), Q-drops,
    /// grants that didn't fit and ignored pre-v75 crafts are counted and
    /// summarised when the player leaves.
    #[test]
    fn window_ops_drops_and_ignored_crafts_are_counted_in_the_summary() {
        use crate::window_ops::OpKind;
        let mut t = PossessionTally::default();
        t.crafts_ignored = 1;
        assert!(t.summary("Crafter").is_some(), "an ignored craft alone is worth a line");
        t.window_ops = 12;
        t.window_noop = 3;
        t.window_refused = 2;
        assert!(t.note_window_mismatch(OpKind::Result), "the first mismatch");
        assert!(!t.note_window_mismatch(OpKind::Sort));
        assert_eq!(t.first_window_mismatch, Some(OpKind::Result), "the first kind is kept");
        t.drops = 5;
        t.grant_overflow = 7;
        t.correction_short = 4;
        t.durability_swap = 2;
        t.use_believed = 6;
        t.use_refused = 2;
        let line = t.summary("Crafter").unwrap();
        assert!(line.contains("4 unit(s) a container correction took short"), "{line}");
        assert!(line.contains("2 drifted tool(s) or armour piece(s) deposited as a swap"), "{line}");
        assert!(line.contains("6 unit(s) a block use took believed, 2 block use(s) refused past the believed bound"), "{line}");
        assert!(
            line.contains(
                "12 window op(s) mirrored, 3 no-op(s) both rules refused, 2 refused by the server alone, \
                 2 digest mismatch(es) (first: result)"
            ),
            "{line}"
        );
        assert!(line.contains("5 Q-drop(s) spawned"), "{line}");
        assert!(line.contains("7 granted unit(s) didn't fit"), "{line}");
        assert!(line.contains("1 pre-v75 craft message(s) ignored"), "{line}");
    }

    /// Review LOW-6 — the shadow starts empty, so a building joiner
    /// mismatches all the time this release: every mismatch is a debug line,
    /// and at most one a minute per player is a warning.
    #[test]
    fn a_mismatch_warns_at_most_once_a_minute_and_is_otherwise_a_debug_line() {
        assert_eq!(MISMATCH_LOG_INTERVAL_TICKS, 60 * 20, "one minute at 20 TPS");
        let mut t = PossessionTally::default();
        let first = t.note_mismatch(5_000);
        assert_eq!(mismatch_log_level(first), log::Level::Warn);
        let soon = t.note_mismatch(5_000 + MISMATCH_LOG_INTERVAL_TICKS - 1);
        assert_eq!(mismatch_log_level(soon), log::Level::Debug, "still inside the minute");
        let later = t.note_mismatch(5_000 + MISMATCH_LOG_INTERVAL_TICKS);
        assert_eq!(mismatch_log_level(later), log::Level::Warn);
        assert_eq!(later, Some(1), "it reports the one it held back");
    }
}
