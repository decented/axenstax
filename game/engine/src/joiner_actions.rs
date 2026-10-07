//! A joiner's swings and right-clicks on the server's mobs (MP-D2b, Spec 04
//! §4.2d), and its item actions (C2a, §4.2f: eating, sleeping), awaiting the
//! server's word.
//!
//! A joiner's inventory is its own until phase C, but the server decides
//! whether a swing, an interaction or an item action happened. So the client
//! sends the request (`EntityAttack` / `EntityInteract` / `ItemAction`) and
//! remembers what it was made with; when the outcome (`InteractOutcome` /
//! `ItemActionOutcome`) comes back accepted it takes the consumed items (or
//! wears the weapon). A refused outcome changes nothing. Products (milk, the
//! Lead back, wool and loot picked up) arrive as `InventoryGrant`s, never
//! through here. All three requests share one sequence: the server reads and
//! answers them in the order sent, on one ordered stream.
//!
//! Review D2b LOW-1 — an accepted outcome is OWED. What it used comes out of
//! the slot the request was made from if that slot still holds it, and
//! otherwise from wherever the item now is (a bucket dragged to another
//! slot while the request was in flight is still the bucket the cow filled).
//! And an item can't be spent twice: a request that would use an item is
//! sent only while the inventory holds one more than its requests in flight
//! already claim ([`JoinerActions::can_afford`]), so one bucket can't milk
//! two cows on a slow link.
//!
//! FU verify N4 — a claim ends on the server's liveness, never on a clock:
//! each request remembers the sequence number of the input sent after it
//! (`RemoteClient::next_input_seq`), and its claim ends once a `StateUpdate`
//! acknowledges that input ([`JoinerActions::acknowledged`]). The server reads
//! a client's packets in order and answers a request the moment it reads it,
//! before that tick's broadcast, so by then the answer has arrived or never
//! will (one skipped over a per-type budget). While the server is silent — a
//! stalled host — the claim holds, however long: a wall-clock expiry (FU1's
//! ten seconds) let one bucket milk two cows after a longer stall. Leaving
//! the world forgets every request ([`JoinerActions::clear`], `world_exit`; a
//! reconnect is always a leave and a new join).
//!
//! The ordering this rests on (C2a verify L4): every server-to-client packet
//! shares ONE ordered stream (QUIC's single bi stream, a WebSocket, the
//! channel transport), and the server sends an outcome inline while it reads
//! the request, before the `StateUpdate` that acknowledges the input after
//! it — so the outcome always arrives before the acknowledgement that ends
//! its claim. Move `StateUpdate` onto an unreliable or separate channel and
//! the ack could overtake the outcome: the claim would end early and a second
//! request could spend the same item. Pinned over the channel transport by
//! `test_integration::joiner_hunger`
//! (`an_outcome_arrives_before_the_state_update_that_acknowledges_the_input_after_it`);
//! Spec 04 §4.2d.
//!
//! C2b — what the client HOLDS, for both rules above, is its 36 slots, then
//! the crafting grid, then the cursor (the crafting UI holds items outside
//! the 36 slots while it is open): an owed outcome is paid from the first of
//! them that has the item ([`take_owed_held`]; C2a verify L6 — food carried on
//! the cursor when its `Eat` was accepted used to go unpaid), and a claim
//! counts all three ([`JoinerActions::can_afford`]). And a joined client's
//! own uses respect the claims: a Q-drop or a craft that would spend an item
//! a request in flight needs does nothing ([`JoinerActions::can_spend`],
//! [`JoinerActions::may_craft`]). A `Craft` or `Drop` takes a request number
//! too but is never answered ([`JoinerActions::unanswered`]).

use std::collections::VecDeque;

use crate::craft_ui::CraftingUi;
use crate::inventory::Inventory;
use crate::item::Item;
use crate::mob::MobType;
use crate::protocol::{InteractKind, InteractOutcomePacket, ItemActionOutcomePacket};

/// Requests kept waiting for an answer. The server answers every request it
/// reads, in the order sent (one past its per-tick budget waits for its next
/// tick, FU1); one it never answers — lost with a connection, or skipped by a
/// per-type budget no honest client reaches — is forgotten when a later one
/// is answered ([`JoinerActions::take`]), and the oldest waiting entry is
/// forgotten once this many are outstanding.
pub const MAX_PENDING: usize = 64;

/// What a request asked the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A swing at a mob (`EntityAttack`).
    Swing,
    /// A one-shot interaction with a mob, or a Lead on a fence post
    /// (`EntityInteract`).
    Interact(InteractKind),
    /// C2a — eat one of the food in hand (`ItemAction::Eat`).
    Eat,
    /// C2a — sleep in the bed at `bed` (`ItemAction::Sleep`).
    Sleep { bed: [i32; 3] },
}

/// One request awaiting its outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    /// What was asked for.
    pub kind: Asked,
    /// The mob's species (the note's wording); `None` for a Lead on a fence
    /// post, which names no mob, and for an item action.
    pub mob: Option<MobType>,
    /// Where the held item was.
    pub hotbar_slot: usize,
    /// What it was (`None` = an empty hand).
    pub held: Option<Item>,
}

/// One remembered request.
#[derive(Debug)]
struct Entry {
    seq: u32,
    /// The sequence number of the input sent after this request; its claim
    /// ends once the server acknowledges that input (N4).
    ends_at_input: u64,
    /// Still claiming its item ([`JoinerActions::can_afford`]).
    claims: bool,
    request: Pending,
}

/// The joiner's outstanding requests. Empty unless joined.
#[derive(Default)]
pub struct JoinerActions {
    next_seq: u32,
    /// Oldest first.
    pending: VecDeque<Entry>,
}

impl JoinerActions {
    /// Remember a request; returns the `seq` to send it under.
    /// `next_input_seq` is the sequence number of the input this client
    /// sends next (`RemoteClient::next_input_seq`): the request goes out
    /// ahead of it, so once the server acknowledges that input it has read
    /// (and answered, or skipped) the request.
    pub fn record(&mut self, request: Pending, next_input_seq: u64) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(Entry {
            seq: self.next_seq,
            ends_at_input: next_input_seq,
            claims: true,
            request,
        });
        self.next_seq
    }

    /// C2b — the sequence number for a request the server never answers
    /// (`ItemAction::Craft`, `ItemAction::Drop`): the shared sequence moves
    /// on, and nothing waits for an outcome or claims an item.
    pub fn unanswered(&mut self) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        self.next_seq
    }

    /// The request `seq` answered, if this client is waiting for it. Every
    /// request sent before it and still waiting is forgotten too: the server
    /// reads requests in the order they were sent and answers each at once,
    /// on the same ordered stream, so one it left unanswered never will be —
    /// and must not keep claiming the item it would have used
    /// ([`Self::can_afford`]).
    pub fn take(&mut self, seq: u32) -> Option<Pending> {
        let at = self.pending.iter().position(|e| e.seq == seq)?;
        self.pending.drain(..at);
        self.pending.pop_front().map(|e| e.request)
    }

    /// A `StateUpdate` says the server has applied every input up to
    /// `last_acked_input` (N4): every request sent ahead of one of them has
    /// been read — answered, or skipped for good — so it claims nothing any
    /// more. The entry stays: its answer, already queued in the same poll,
    /// is still applied.
    pub fn acknowledged(&mut self, last_acked_input: u64) {
        for e in &mut self.pending {
            if e.ends_at_input <= last_acked_input {
                e.claims = false;
            }
        }
    }

    /// Forget everything (the session ended: leaving the world, and so every
    /// reconnect — `world_exit`).
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// May a request asking for `kind` made with `held` go out now (review
    /// D2b LOW-1)? Yes when it uses nothing; otherwise only while the client
    /// holds (`inv`, plus `ui`'s grid and cursor, C2b) more of the item than
    /// the requests still claiming would use (N4: a request claims until the
    /// server acknowledges the input sent after it).
    pub fn can_afford(&self, inv: &Inventory, ui: &CraftingUi, kind: Asked, held: Option<&Item>) -> bool {
        let need = uses(kind);
        if need == 0 {
            return true;
        }
        let Some(item) = held else { return false };
        self.can_spend(inv, ui, item, u32::from(need))
    }

    /// C2b — may the client spend `n` of `item` itself (a Q-drop, a craft)?
    /// Only if what it holds afterwards (`inv`, plus `ui`'s grid and cursor)
    /// still covers every claim of the requests in flight on that item.
    pub fn can_spend(&self, inv: &Inventory, ui: &CraftingUi, item: &Item, n: u32) -> bool {
        count_held(inv, ui, item) >= self.claimed(item).saturating_add(n)
    }

    /// C2b — may the result-slot click craft? It consumes one from every
    /// non-empty grid cell, so each distinct ingredient is spent once per
    /// cell holding it ([`Self::can_spend`]).
    pub fn may_craft(&self, inv: &Inventory, ui: &CraftingUi) -> bool {
        let mut spent: Vec<(&Item, u32)> = Vec::new();
        for stack in ui.grid.iter().flatten().flatten() {
            match spent.iter_mut().find(|(item, _)| same_item(item, &stack.item)) {
                Some((_, n)) => *n += 1,
                None => spent.push((&stack.item, 1)),
            }
        }
        spent.iter().all(|(item, n)| self.can_spend(inv, ui, item, *n))
    }

    /// How many of `item` the requests still claiming would use.
    fn claimed(&self, item: &Item) -> u32 {
        self.pending
            .iter()
            .filter(|e| e.claims)
            .filter(|e| e.request.held.as_ref().is_some_and(|h| same_item(h, item)))
            .map(|e| u32::from(uses(e.request.kind)))
            .sum()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Is an `Eat` still in flight (C2a verify M1)? A joiner sends no new one
    /// while it is: the entry waits for the outcome, and stops counting once
    /// the server has passed it by ([`Self::acknowledged`]), so a lost
    /// request can't block eating for good.
    pub fn eat_in_flight(&self) -> bool {
        self.pending.iter().any(|e| e.claims && e.request.kind == Asked::Eat)
    }
}

/// What an outcome did to the inventory.
#[derive(Debug, Default)]
pub struct Applied {
    /// The weapon's wear, for the "tool worn / broke" feedback (accepted
    /// swings only).
    pub wear: Option<crate::inventory::ToolUseInfo>,
    /// Items taken from the held stack.
    pub consumed: u8,
}

/// How many of the held item a request asking for `kind` uses when accepted
/// (the server's `consume_held` for it): breeding food, taming food, a
/// bucket, a Lead on a mob or a post, an eaten food. Shearing, taking a Lead
/// off, a pet command, a swing and a sleep use none.
pub fn uses(kind: Asked) -> u8 {
    match kind {
        Asked::Interact(
            InteractKind::Feed
            | InteractKind::Tame
            | InteractKind::Milk
            | InteractKind::LeadAttach
            | InteractKind::LeadToPost { .. },
        )
        | Asked::Eat => 1,
        Asked::Interact(InteractKind::Shear | InteractKind::LeadDetach | InteractKind::SitToggle)
        | Asked::Swing
        | Asked::Sleep { .. } => 0,
    }
}

/// Is `a` the item `b` was, for an outcome's purposes? A tool is the same
/// tool by type and material, and an armour piece by slot and material
/// (durability is what wears; the shadow's copy never does — C2b verify L6).
fn same_item(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::Tool(a), Item::Tool(b)) => a.tool_type == b.tool_type && a.material == b.material,
        (Item::Armour(a), Item::Armour(b)) => a.slot == b.slot && a.material == b.material,
        (a, b) => a == b,
    }
}

/// How many of `item` the whole inventory holds.
fn count_of(inv: &Inventory, item: &Item) -> u32 {
    inv.slots_iter()
        .flatten()
        .filter(|s| same_item(&s.item, item))
        .map(|s| u32::from(s.count))
        .sum()
}

/// C2b — how many of `item` the client holds: its 36 slots, the crafting
/// grid and the cursor.
// BRIDGE: replaced when C3a's server window holds the grid and cursor.
fn count_held(inv: &Inventory, ui: &CraftingUi, item: &Item) -> u32 {
    let outside: u32 = ui
        .grid
        .iter()
        .flatten()
        .chain(std::iter::once(&ui.cursor_item))
        .flatten()
        .filter(|s| same_item(&s.item, item))
        .map(|s| u32::from(s.count))
        .sum();
    count_of(inv, item) + outside
}

/// Where `held` is now: the slot `slot` it was used from if that still holds
/// it, else the first slot (of all 36) that does.
fn where_now(inv: &crate::inventory::Inventory, slot: usize, held: &Item) -> Option<usize> {
    let holds = |i: usize| inv.slot(i).is_some_and(|s| same_item(&s.item, held));
    if holds(slot) {
        return Some(slot);
    }
    (0..36).find(|&i| holds(i))
}

/// Take what an accepted interaction OWES (review D2b LOW-1): `n` of `held`,
/// each from `slot` if it still holds one, else from wherever one now is.
/// Returns how many were taken (fewer only when the inventory runs out).
///
/// One rule for both copies of a joiner's inventory: the client runs it on
/// its own ([`apply_outcome`], [`apply_item_outcome`]) and the server on its
/// shadow of it (C1, `joiner_inventory`), for the same accepted outcome.
pub fn take_owed(inv: &mut crate::inventory::Inventory, slot: usize, held: &Item, n: u8) -> u8 {
    let mut taken = 0;
    for _ in 0..n {
        let Some(at) = where_now(inv, slot, held) else { break };
        let Some(mut stack) = inv.take_slot(at) else { break };
        stack.count = stack.count.saturating_sub(1);
        if stack.count > 0 {
            inv.set_slot(at, Some(stack));
        }
        taken += 1;
    }
    taken
}

/// C2b decision 5 — the client's owed payment: `n` of `held`, from its 36
/// slots first ([`take_owed`]: the request's slot if it still holds one,
/// else wherever one is), then the crafting grid (row-major), then the
/// cursor. Returns how many were taken. The grid's result is recomputed if a
/// cell was taken from. (The server's shadow has no grid: it runs
/// [`take_owed`].)
// BRIDGE: replaced when C3a's server window holds the grid and cursor.
pub fn take_owed_held(inv: &mut Inventory, ui: &mut CraftingUi, slot: usize, held: &Item, n: u8) -> u8 {
    let mut taken = take_owed(inv, slot, held, n);
    let mut from_grid = false;
    while taken < n {
        let cell = match ui.grid.iter_mut().flatten().find(|c| c.as_ref().is_some_and(|s| same_item(&s.item, held))) {
            Some(cell) => {
                from_grid = true;
                cell
            }
            None if ui.cursor_item.as_ref().is_some_and(|s| same_item(&s.item, held)) => &mut ui.cursor_item,
            None => break,
        };
        if let Some(stack) = cell.as_mut() {
            stack.count = stack.count.saturating_sub(1);
            if stack.count == 0 {
                *cell = None;
            }
        }
        taken += 1;
    }
    if from_grid {
        ui.update_result();
    }
    taken
}

/// Apply an outcome to the joiner's inventory: an accepted swing wears the
/// weapon (`Inventory::use_tool_at`, the single-player rule — a swing that
/// found its target wears it, damage or not); an accepted interaction takes
/// `consume_held` of the item it was made with. Nothing on a refusal. An
/// accepted outcome is owed (review D2b LOW-1): taken from the request's slot
/// if it still holds the item, else from wherever the item now is — the 36
/// slots, then `ui`'s grid, then its cursor (C2b, [`take_owed_held`]); only
/// an item the client no longer holds at all goes unpaid.
pub fn apply_outcome(
    inv: &mut Inventory,
    ui: &mut CraftingUi,
    request: &Pending,
    outcome: &InteractOutcomePacket,
) -> Applied {
    let mut applied = Applied::default();
    if !outcome.accepted {
        return applied;
    }
    let Some(held) = request.held.as_ref() else {
        return applied;
    };
    match request.kind {
        Asked::Swing => {
            if let Some(at) = where_now(inv, request.hotbar_slot, held) {
                applied.wear = inv.use_tool_at(at);
            }
        }
        Asked::Interact(_) => {
            applied.consumed = take_owed_held(inv, ui, request.hotbar_slot, held, outcome.consume_held)
        }
        // Not an interaction's answer.
        Asked::Eat | Asked::Sleep { .. } => {}
    }
    applied
}

/// C2a — apply an item action's outcome to the joiner's inventory: an
/// accepted one takes `consume_held` of what it claimed (the eaten food),
/// owed like an interaction's ([`take_owed_held`]: the 36 slots, then `ui`'s
/// grid, then its cursor — C2a verify L6). Nothing on a refusal. Returns how
/// many were taken.
pub fn apply_item_outcome(
    inv: &mut Inventory,
    ui: &mut CraftingUi,
    request: &Pending,
    outcome: &ItemActionOutcomePacket,
) -> u8 {
    if !outcome.accepted || !matches!(request.kind, Asked::Eat | Asked::Sleep { .. }) {
        return 0;
    }
    let Some(held) = request.held.as_ref() else {
        return 0;
    };
    take_owed_held(inv, ui, request.hotbar_slot, held, outcome.consume_held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::inventory::Inventory;
    use crate::item::{ItemStack, MaterialId};

    fn outcome(seq: u32, accepted: bool, consume_held: u8) -> InteractOutcomePacket {
        InteractOutcomePacket { seq, entity: 1, kind: None, accepted, consume_held, note: 0 }
    }

    fn inv_with(slot: usize, stack: ItemStack) -> Inventory {
        let mut inv = Inventory::new();
        inv.set_slot(slot, Some(stack));
        inv
    }

    fn sword() -> Item {
        Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))
    }

    /// C2b verify L6 — a worn armour piece pays from the unworn shadow copy.
    #[test]
    fn a_worn_armour_piece_is_paid_from_the_unworn_shadow_copy() {
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        let fresh = ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron);
        let mut worn = fresh;
        worn.durability = worn.durability.saturating_sub(9);
        assert_ne!(fresh, worn);
        let mut inv = inv_with(3, ItemStack { item: Item::Armour(fresh), count: 1 });
        assert_eq!(take_owed(&mut inv, 0, &Item::Armour(worn), 1), 1, "the worn piece pays from the unworn copy");
        assert!(inv.slot(3).is_none());
        // A different slot or material is a different piece.
        let mut inv = inv_with(3, ItemStack { item: Item::Armour(fresh), count: 1 });
        let boots = ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Iron);
        assert_eq!(take_owed(&mut inv, 0, &Item::Armour(boots), 1), 0);
    }

    fn durability(inv: &Inventory, slot: usize) -> u32 {
        match &inv.hotbar_slot(slot).unwrap().item {
            Item::Tool(t) => t.durability as u32,
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_weapon_wears_only_on_a_confirmed_swing() {
        let mut inv = inv_with(2, ItemStack { item: sword(), count: 1 });
        let before = durability(&inv, 2);
        let swing = Pending { kind: Asked::Swing, mob: Some(MobType::Cow), hotbar_slot: 2, held: Some(sword()) };
        let refused = apply_outcome(&mut inv, &mut CraftingUi::new(), &swing, &outcome(1, false, 0));
        assert!(refused.wear.is_none());
        assert_eq!(durability(&inv, 2), before, "a refused swing wears nothing");
        let confirmed = apply_outcome(&mut inv, &mut CraftingUi::new(), &swing, &outcome(1, true, 0));
        assert!(confirmed.wear.is_some());
        assert_eq!(durability(&inv, 2), before - 1);
    }

    #[test]
    fn an_accepted_interaction_takes_the_items_and_a_refused_one_takes_nothing() {
        let wheat = Item::Material(MaterialId::Wheat);
        let mut inv = inv_with(0, ItemStack { item: wheat.clone(), count: 5 });
        let feed = Pending {
            kind: Asked::Interact(InteractKind::Feed),
            mob: Some(MobType::Cow),
            hotbar_slot: 0,
            held: Some(wheat),
        };
        assert_eq!(apply_outcome(&mut inv, &mut CraftingUi::new(), &feed, &outcome(1, false, 1)).consumed, 0);
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 5);
        assert_eq!(apply_outcome(&mut inv, &mut CraftingUi::new(), &feed, &outcome(1, true, 1)).consumed, 1);
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 4);
    }

    /// Review D2b LOW-1 — an accepted outcome is owed: the bucket dragged to
    /// another slot while the request was in flight is the bucket that went
    /// (this test used to pin the opposite — the swap kept the bucket AND
    /// earned the milk). What took the old slot is never charged.
    #[test]
    fn an_accepted_outcome_is_owed_wherever_the_item_went() {
        let bucket = Item::Material(MaterialId::Bucket);
        let mut inv = inv_with(3, ItemStack::new_material(MaterialId::Bone, 4));
        inv.set_slot(20, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        let milk = Pending {
            kind: Asked::Interact(InteractKind::Milk),
            mob: Some(MobType::Cow),
            hotbar_slot: 3,
            held: Some(bucket),
        };
        assert_eq!(apply_outcome(&mut inv, &mut CraftingUi::new(), &milk, &outcome(1, true, 1)).consumed, 1);
        assert_eq!(inv.hotbar_slot(3).unwrap().count, 4, "the bones that took its slot are untouched");
        assert!(inv.slot(20).is_none(), "the bucket went from where it is now");
        // Nothing to take any more: nothing taken, no panic.
        assert_eq!(apply_outcome(&mut inv, &mut CraftingUi::new(), &milk, &outcome(1, true, 1)).consumed, 0);
    }

    /// Review D2b LOW-1 — a swing confirmed after the sword moved wears that
    /// sword, wherever it is.
    #[test]
    fn a_confirmed_swing_wears_the_sword_where_it_went() {
        let mut inv = inv_with(2, ItemStack::new_material(MaterialId::Wheat, 3));
        inv.set_slot(30, Some(ItemStack { item: sword(), count: 1 }));
        let swing = Pending { kind: Asked::Swing, mob: Some(MobType::Cow), hotbar_slot: 2, held: Some(sword()) };
        let durability_at = |inv: &Inventory| match &inv.slot(30).unwrap().item {
            Item::Tool(t) => t.durability,
            _ => unreachable!(),
        };
        let before = durability_at(&inv);
        assert!(apply_outcome(&mut inv, &mut CraftingUi::new(), &swing, &outcome(1, true, 0)).wear.is_some());
        assert_eq!(durability_at(&inv), before - 1);
        assert_eq!(inv.hotbar_slot(2).unwrap().count, 3, "the wheat in its old slot is untouched");
    }

    /// Review D2b LOW-1 — one bucket can't be spent twice: while a Milk
    /// request is in flight, a second one with the same (only) bucket isn't
    /// sent; with two buckets it is. A request that uses nothing always goes.
    #[test]
    fn an_item_already_claimed_by_a_request_in_flight_cannot_be_claimed_again() {
        let bucket = Item::Material(MaterialId::Bucket);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bucket, 1));
        let mut a = JoinerActions::default();
        let milk = Asked::Interact(InteractKind::Milk);
        assert!(a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)));
        let first = a.record(
            Pending { kind: milk, mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(bucket.clone()) },
            1,
        );
        assert!(!a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)), "the only bucket is spoken for");
        assert!(a.can_afford(&inv, &CraftingUi::new(), Asked::Interact(InteractKind::Shear), None), "shearing uses nothing");
        let two = inv_with(0, ItemStack::new_material(MaterialId::Bucket, 2));
        assert!(a.can_afford(&two, &CraftingUi::new(), milk, Some(&bucket)), "a second bucket is free");
        // Answered (refused or not): the claim is gone.
        a.take(first);
        assert!(a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)));
        assert!(!a.can_afford(&inv, &CraftingUi::new(), milk, None), "an empty hand has nothing to spend");
    }

    /// Review D2b LOW-1 — a request the server never answered stops claiming
    /// its item once a LATER request is answered: the server answers in
    /// order, so it never will be.
    #[test]
    fn an_unanswered_request_stops_claiming_once_a_later_one_is_answered() {
        let bucket = Item::Material(MaterialId::Bucket);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bucket, 1));
        let mut a = JoinerActions::default();
        let milk = Asked::Interact(InteractKind::Milk);
        let dropped = a.record(
            Pending { kind: milk, mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(bucket.clone()) },
            1,
        );
        let swing =
            a.record(Pending { kind: Asked::Swing, mob: Some(MobType::Cow), hotbar_slot: 1, held: None }, 1);
        assert!(!a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)), "claimed while it might still be answered");
        assert!(a.take(swing).is_some());
        assert!(a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)), "the server answered past it: it never will be");
        assert!(a.take(dropped).is_none(), "forgotten");
        assert_eq!(a.len(), 0);
    }

    /// FU verify N4 — a claim ends on the server's liveness, not a clock.
    /// FU1's ten-second expiry let one bucket milk cow A, the host stall past
    /// ten seconds, and the same bucket milk cow B; both were then accepted
    /// (two milk buckets from one). Now: milk A with the only bucket, then
    /// 300 ticks of inputs with no `StateUpdate` (the host stalled) — B is
    /// still refused; once a `StateUpdate` acknowledges the input sent after
    /// A, A has been read (answered or skipped for good) and the bucket is
    /// free. An answer that still comes is applied, and leaving the world
    /// forgets every claim.
    #[test]
    fn a_claim_holds_while_the_server_is_silent_and_ends_with_its_acknowledgement() {
        let bucket = Item::Material(MaterialId::Bucket);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bucket, 1));
        let milk = Asked::Interact(InteractKind::Milk);
        let req = || Pending { kind: milk, mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(bucket.clone()) };
        let mut a = JoinerActions::default();
        // Milk A, sent ahead of input 7.
        let cow_a = a.record(req(), 7);
        assert!(!a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)));
        // 300 ticks: inputs 7..307 go out, the stalled host acknowledges
        // nothing past input 6 (sent before the request).
        for _ in 0..300 {
            a.acknowledged(6);
        }
        assert!(
            !a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)),
            "however long the server is silent, the only bucket stays spoken for"
        );
        // The host resumes: its StateUpdate acknowledges input 7.
        a.acknowledged(7);
        assert!(a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)), "read and answered: the bucket is free");
        // The answer, queued in the same poll, is still applied.
        assert_eq!(a.take(cow_a).map(|p| p.kind), Some(milk));

        // A request skipped (never answered) stops claiming the same way.
        let skipped = a.record(req(), 400);
        a.acknowledged(450);
        assert!(a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)));
        assert!(a.take(skipped).is_some(), "the entry stays for a late answer");

        // Leaving the world (and so every reconnect) forgets every claim.
        let mut a = JoinerActions::default();
        a.record(req(), 1);
        assert!(!a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)));
        a.clear();
        assert!(a.can_afford(&inv, &CraftingUi::new(), milk, Some(&bucket)));
        assert_eq!(a.len(), 0);
    }

    /// C2a — eating claims the food like a Feed does (one carrot can't be
    /// eaten and fed to a pig at once); a sleep claims nothing.
    #[test]
    fn eating_claims_its_food_alongside_interactions_and_a_sleep_claims_nothing() {
        let carrot = Item::Material(MaterialId::Carrot);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Carrot, 1));
        let mut a = JoinerActions::default();
        assert!(a.can_afford(&inv, &CraftingUi::new(), Asked::Eat, Some(&carrot)));
        let eat = a.record(Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(carrot.clone()) }, 3);
        assert!(!a.can_afford(&inv, &CraftingUi::new(), Asked::Eat, Some(&carrot)), "the only carrot is being eaten");
        assert!(!a.can_afford(&inv, &CraftingUi::new(), Asked::Interact(InteractKind::Feed), Some(&carrot)));
        assert!(a.can_afford(&inv, &CraftingUi::new(), Asked::Sleep { bed: [0, 64, 0] }, None), "sleeping uses nothing");
        assert_eq!(uses(Asked::Eat), 1);
        assert_eq!(uses(Asked::Sleep { bed: [0, 0, 0] }), 0);
        assert_eq!(uses(Asked::Swing), 0);
        assert!(a.take(eat).is_some());
    }

    #[test]
    fn an_accepted_eat_takes_the_food_and_a_refused_one_nothing() {
        let bread = Item::Material(MaterialId::Bread);
        let mut inv = inv_with(4, ItemStack::new_material(MaterialId::Bread, 3));
        let eat = Pending { kind: Asked::Eat, mob: None, hotbar_slot: 4, held: Some(bread) };
        let out = |accepted, consume_held| ItemActionOutcomePacket { seq: 1, accepted, consume_held, note: 0 };
        assert_eq!(apply_item_outcome(&mut inv, &mut CraftingUi::new(), &eat, &out(false, 0)), 0);
        assert_eq!(inv.hotbar_slot(4).unwrap().count, 3);
        assert_eq!(apply_item_outcome(&mut inv, &mut CraftingUi::new(), &eat, &out(true, 1)), 1);
        assert_eq!(inv.hotbar_slot(4).unwrap().count, 2);
        // An interaction's answer never pays an item action, nor the reverse.
        assert_eq!(apply_outcome(&mut inv, &mut CraftingUi::new(), &eat, &outcome(1, true, 1)).consumed, 0);
        assert_eq!(inv.hotbar_slot(4).unwrap().count, 2);
    }

    // ─── C2b: the claims gate and the wider owed payment ────────────────

    fn milk_pending(a: &mut JoinerActions) -> u32 {
        let bucket = Item::Material(MaterialId::Bucket);
        a.record(
            Pending {
                kind: Asked::Interact(InteractKind::Milk),
                mob: Some(MobType::Cow),
                hotbar_slot: 0,
                held: Some(bucket),
            },
            1,
        )
    }

    /// Decision 4 — a pending Milk claims the only bucket: the client's
    /// Q-drop of it (`can_spend(bucket, 1)`, the gate the Q-drop site asks)
    /// does nothing until the claim ends; with a second bucket it goes.
    #[test]
    fn a_pending_milk_claim_on_the_only_bucket_stops_its_q_drop() {
        let bucket = Item::Material(MaterialId::Bucket);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bucket, 1));
        let ui = CraftingUi::new();
        let mut a = JoinerActions::default();
        assert!(a.can_spend(&inv, &ui, &bucket, 1), "nothing claims it yet");
        let milk = milk_pending(&mut a);
        assert!(!a.can_spend(&inv, &ui, &bucket, 1), "the Q-drop is gated");
        let two = inv_with(0, ItemStack::new_material(MaterialId::Bucket, 2));
        assert!(a.can_spend(&two, &ui, &bucket, 1), "a spare bucket may go");
        assert!(a.can_spend(&inv, &ui, &Item::Material(MaterialId::Wheat), 0));
        a.take(milk);
        assert!(a.can_spend(&inv, &ui, &bucket, 1), "answered: free again");
    }

    /// Decision 4 — a pending Feed claims the only wheat... and the player
    /// moved it into the crafting grid (with two more) for bread: the result
    /// click, which would consume one from each of the three cells, is gated.
    /// Without the claim it crafts; a craft that doesn't touch wheat is
    /// never gated by it.
    #[test]
    fn a_pending_feed_claim_on_the_only_wheat_stops_a_craft_that_would_consume_it() {
        let wheat = Item::Material(MaterialId::Wheat);
        let inv = Inventory::new();
        let mut ui = CraftingUi::new();
        ui.open_table_crafting([0, 64, 0]);
        for c in 0..3 {
            ui.grid[1][c] = Some(ItemStack::new_material(MaterialId::Wheat, 1));
        }
        ui.update_result();
        assert!(ui.result.is_some(), "three wheat make bread");
        let mut a = JoinerActions::default();
        assert!(a.may_craft(&inv, &ui));
        let feed = a.record(
            Pending { kind: Asked::Interact(InteractKind::Feed), mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(wheat) },
            1,
        );
        assert!(!a.may_craft(&inv, &ui), "crafting would spend the wheat the feed needs");
        // A fourth wheat in the inventory covers the claim: the craft goes.
        let spare = inv_with(9, ItemStack::new_material(MaterialId::Wheat, 1));
        assert!(a.may_craft(&spare, &ui));
        // A craft from other items is never gated by the wheat claim.
        let mut planks = CraftingUi::new();
        planks.open_player_crafting();
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            planks.grid[r][c] = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 1));
        }
        assert!(a.may_craft(&inv, &planks));
        a.take(feed);
        assert!(a.may_craft(&inv, &ui));
    }

    /// Decision 5 — a claim counts the grid and the cursor too: the only
    /// bucket on the cursor is still claimed by a Milk in flight.
    #[test]
    fn a_claim_counts_the_crafting_grid_and_the_cursor() {
        let bucket = Item::Material(MaterialId::Bucket);
        let inv = Inventory::new();
        let mut ui = CraftingUi::new();
        let milk = Asked::Interact(InteractKind::Milk);
        let mut a = JoinerActions::default();
        assert!(!a.can_afford(&inv, &ui, milk, Some(&bucket)), "no bucket anywhere");
        ui.cursor_item = Some(ItemStack::new_material(MaterialId::Bucket, 1));
        assert!(a.can_afford(&inv, &ui, milk, Some(&bucket)), "the cursor's bucket counts");
        milk_pending(&mut a);
        assert!(!a.can_afford(&inv, &ui, milk, Some(&bucket)));
        ui.grid[0][0] = Some(ItemStack::new_material(MaterialId::Bucket, 1));
        assert!(a.can_afford(&inv, &ui, milk, Some(&bucket)), "and so does the grid's");
    }

    /// Decision 5 (and C2a verify L6) — an accepted outcome is paid from the
    /// crafting grid, then the cursor, when the 36 slots lack the item: a
    /// D2b Milk from a bucket in the grid, an Eat from bread on the cursor.
    /// The 36 slots always pay first.
    #[test]
    fn an_accepted_outcome_is_paid_from_the_grid_then_the_cursor() {
        let bucket = Item::Material(MaterialId::Bucket);
        let bread = Item::Material(MaterialId::Bread);
        let mut inv = Inventory::new();
        let mut ui = CraftingUi::new();
        ui.open_player_crafting();
        // Milk: the bucket was moved into the grid while the request flew.
        ui.grid[1][1] = Some(ItemStack::new_material(MaterialId::Bucket, 1));
        ui.cursor_item = Some(ItemStack::new_material(MaterialId::Bucket, 1));
        let milk = Pending {
            kind: Asked::Interact(InteractKind::Milk),
            mob: Some(MobType::Cow),
            hotbar_slot: 0,
            held: Some(bucket),
        };
        assert_eq!(apply_outcome(&mut inv, &mut ui, &milk, &outcome(1, true, 1)).consumed, 1);
        assert!(ui.grid[1][1].is_none(), "paid from the grid first");
        assert!(ui.cursor_item.is_some(), "the cursor's bucket is untouched");
        assert_eq!(apply_outcome(&mut inv, &mut ui, &milk, &outcome(1, true, 1)).consumed, 1);
        assert!(ui.cursor_item.is_none(), "then from the cursor");
        assert_eq!(apply_outcome(&mut inv, &mut ui, &milk, &outcome(1, true, 1)).consumed, 0, "then nothing");

        // Eat: the bread is on the cursor mid-drag when the outcome lands.
        ui.cursor_item = Some(ItemStack::new_material(MaterialId::Bread, 2));
        let eat = Pending { kind: Asked::Eat, mob: None, hotbar_slot: 4, held: Some(bread) };
        let out = ItemActionOutcomePacket { seq: 1, accepted: true, consume_held: 1, note: 0 };
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &eat, &out), 1);
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(1), "paid from the cursor");
        // With bread back in the 36 slots, they pay first.
        inv.set_slot(20, Some(ItemStack::new_material(MaterialId::Bread, 1)));
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &eat, &out), 1);
        assert!(inv.slot(20).is_none());
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(1));
    }

    /// C2b verify L7 — an `Eat` owed is paid from the crafting grid, the same
    /// function as the cursor case: the bread was dragged into the grid while
    /// the request flew.
    #[test]
    fn an_eat_owed_is_paid_from_the_grid() {
        let bread = Item::Material(MaterialId::Bread);
        let mut inv = Inventory::new();
        let mut ui = CraftingUi::new();
        ui.open_player_crafting();
        ui.grid[0][1] = Some(ItemStack::new_material(MaterialId::Bread, 2));
        let eat = Pending { kind: Asked::Eat, mob: None, hotbar_slot: 4, held: Some(bread) };
        let out = ItemActionOutcomePacket { seq: 1, accepted: true, consume_held: 1, note: 0 };
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &eat, &out), 1);
        assert_eq!(ui.grid[0][1].as_ref().map(|s| s.count), Some(1), "one bite taken from the grid cell");
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &eat, &out), 1);
        assert!(ui.grid[0][1].is_none());
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &eat, &out), 0, "then nothing");
    }

    /// Decision 5 — paying from the grid recomputes the result shown.
    #[test]
    fn paying_from_the_grid_updates_the_crafting_result() {
        let wheat = Item::Material(MaterialId::Wheat);
        let mut inv = Inventory::new();
        let mut ui = CraftingUi::new();
        ui.open_table_crafting([0, 64, 0]);
        for c in 0..3 {
            ui.grid[1][c] = Some(ItemStack::new_material(MaterialId::Wheat, 1));
        }
        ui.update_result();
        assert!(ui.result.is_some());
        let feed = Pending { kind: Asked::Interact(InteractKind::Feed), mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(wheat) };
        assert_eq!(apply_outcome(&mut inv, &mut ui, &feed, &outcome(1, true, 1)).consumed, 1);
        assert!(ui.result.is_none(), "two wheat make nothing");
    }

    /// Decision 6 — a Craft or Drop moves the shared sequence on but waits
    /// for nothing and claims nothing.
    #[test]
    fn an_unanswered_request_takes_a_number_and_waits_for_nothing() {
        let mut a = JoinerActions::default();
        let s1 = milk_pending(&mut a);
        let s2 = a.unanswered();
        let s3 = milk_pending(&mut a);
        assert_eq!((s2, s3), (s1 + 1, s1 + 2));
        assert_eq!(a.len(), 2, "nothing waits for the craft or drop");
        assert!(a.take(s2).is_none());
    }

    #[test]
    fn pending_requests_are_matched_by_seq_and_bounded() {
        let mut a = JoinerActions::default();
        let req = |slot| Pending { kind: Asked::Swing, mob: Some(MobType::Pig), hotbar_slot: slot, held: None };
        let s1 = a.record(req(1), 1);
        let s2 = a.record(req(2), 1);
        assert_ne!(s1, s2);
        assert_eq!(a.take(s2).unwrap().hotbar_slot, 2);
        assert!(a.take(s2).is_none(), "answered once");
        for _ in 0..MAX_PENDING + 5 {
            a.record(req(0), 1);
        }
        assert_eq!(a.len(), MAX_PENDING);
        assert!(a.take(s1).is_none(), "the oldest unanswered request was forgotten");
    }
}
