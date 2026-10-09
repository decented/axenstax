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
//! counts all three ([`JoinerActions::can_afford`]). C3a-2a — the server
//! holds the same window (it mirrors every window op) and pays an accepted
//! outcome from its copy by the same search ([`take_owed_window`]). C3a-fix-1
//! — an outcome that changes the window (a take, a swing's wear) carries a
//! window event number, applied by the client in arrival order
//! (`window_events::WindowInbox`) and by the server once the client reports
//! it: so both pay at the same point among the client's window ops. And a joined client's
//! own uses respect the claims: a Q-drop or a craft that would spend an item
//! a request in flight needs does nothing ([`JoinerActions::can_spend`],
//! [`JoinerActions::may_craft`]). A `Drop` takes a request number too but is
//! never answered ([`JoinerActions::unanswered`]); since C3a-2a a craft is
//! the window's result click, sent as a window op, not a request.

use std::collections::VecDeque;

use crate::craft_ui::CraftingUi;
use crate::armour::ArmourItem;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};
use crate::mob::MobType;
use crate::protocol::{InteractKind, InteractOutcomePacket, ItemActionOutcomePacket};

/// Requests kept waiting for an answer. The server answers every request it
/// reads, in the order sent (one past its per-tick budget waits for its next
/// tick, FU1); one it never answers — lost with a connection, or skipped by a
/// per-type budget no honest client reaches — is forgotten when a later one
/// is answered ([`JoinerActions::take`]), and one entry is forgotten once
/// this many are outstanding — C3b-fix-e (L5): never one whose claim still
/// holds while another will do ([`JoinerActions::record`]).
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
    /// C3b-2 — right-click the `kind` block (a composter, drying rack,
    /// campfire, item frame or hive) at `cell` (`ItemAction::UseBlock`),
    /// claiming `claim` of the held item (`block_use::claim`: what the rule
    /// could take).
    UseBlock { cell: [i32; 3], kind: crate::block_use::UseKind, claim: u8 },
    /// C3c-2 — shoot the `weapon` of `material` in the request's hotbar slot
    /// (`ItemAction::Shoot`). Unlike every other request, [`Pending::held`]
    /// is what it SPENDS — one of its ammo (`shot::ammo_for`) — so the claim
    /// is on the ammo; the weapon wears ([`apply_use_wear`]). C3c-2-fix (L3)
    /// — it claims one unit of the weapon's wear too
    /// ([`JoinerActions::can_afford`]).
    Shoot { weapon: crate::protocol::ShotWeapon, material: crate::crafting::ToolMaterial },
    /// C3c-2 — place the cart in hand on the rail at `cell`
    /// (`ItemAction::PlaceCart`), claiming the one cart.
    PlaceCart { cell: [i32; 3] },
    /// C3c-2 — light the unlit campfire at `cell` with `lighter` (sent as a
    /// campfire `ItemAction::UseBlock`; the client remembers it was a
    /// lighting for the feedback). Claims the stick, or flint and steel's
    /// wear; the Firestarter nothing.
    Light { cell: [i32; 3], lighter: crate::block_use::Lighter },
    /// C3c-2 — cast the rod in hand (`ItemAction::Cast`): claims nothing
    /// (C3c-2-fix L4: recorded as non-claiming, and only while the ledger has
    /// room without ending a claim).
    Cast,
    /// C3c-2 — reel the line in (`ItemAction::Reel`): claims nothing (as a
    /// cast); the rod wears on a catch.
    Reel,
    /// C3c-3b — paint the wallpaper or lay the blank paper in hand on face
    /// `face` of the block at `cell` (`ItemAction::Attach`), claiming the one
    /// block (none is taken in creative: the outcome's `consume_held` is 0).
    Attach { cell: [i32; 3], face: u8 },
    /// C3c-3b — peel the attachment off face `face` of the block at `cell`
    /// (`ItemAction::Detach`): claims nothing; the item comes as a grant.
    Detach { cell: [i32; 3], face: u8 },
}

impl Asked {
    /// C3c-2 — the weapon a `Shoot` wears, as an item ([`same_item`] knows
    /// it by type and material, whatever its durability).
    pub fn shot_weapon(self) -> Option<Item> {
        let Asked::Shoot { weapon, material } = self else { return None };
        let tool_type = match weapon {
            crate::protocol::ShotWeapon::Bow => crate::crafting::ToolType::Bow,
            crate::protocol::ShotWeapon::Slingshot => crate::crafting::ToolType::Slingshot,
        };
        Some(Item::Tool(crate::crafting::Tool::new(tool_type, material)))
    }
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

/// C3b-fix-d (A-L1) — the `ends_at_input` of a request queued behind unsent
/// edits (`GameState::send_request`): it rides behind no input yet, so no
/// acknowledgement ends its claim until it is sent and rebased
/// ([`JoinerActions::rebase`]), or discarded unsent and released
/// ([`JoinerActions::release`]).
pub const UNTIL_SENT: u64 = u64::MAX;

/// C3b-fix-e (L7) — what the send path did with a request
/// ([`JoinerActions::settle`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestFate {
    /// It went out from the queue, ahead of input `next_input_seq`
    /// (`RemoteClient::next_input_seq` right after the send).
    Sent { next_input_seq: u64 },
    /// It was dropped from the queue unsent: the link closed.
    Discarded,
    /// There was no connection to send it on.
    NoConnection,
}

/// One remembered request.
#[derive(Debug)]
struct Entry {
    seq: u32,
    /// The sequence number of the input sent after this request; its claim
    /// ends once the server acknowledges that input (N4). [`UNTIL_SENT`]
    /// while the request waits in the queue.
    ends_at_input: u64,
    /// Still claiming its item ([`JoinerActions::can_afford`]): not yet
    /// read by the server (its input not acknowledged). For a request that
    /// claims no item it only says "in flight" ([`JoinerActions::eat_in_flight`]).
    claims: bool,
    request: Pending,
}

impl Entry {
    /// C3c-2-fix (L4) — does this entry hold an item claim right now: still
    /// unread, and made to use an item ([`claims_an_item`])? A request that
    /// claims nothing never does, so the eviction picks it first and it never
    /// fills the ledger with claims.
    fn claims_now(&self) -> bool {
        self.claims && claims_an_item(&self.request)
    }
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
    /// (and answered, or skipped) the request. C3b-fix-d (A-L1) — a request
    /// that will wait in the queue is recorded with [`UNTIL_SENT`] instead.
    ///
    /// C3b-fix-e (L5) — at [`MAX_PENDING`] one entry is forgotten to make
    /// room, never one whose claim still holds while another will do
    /// ([`Self::evict_one`]); a caller asks [`Self::can_afford`] first, which
    /// says no while every entry claims. C3c-2-fix (L4) — a request that
    /// claims nothing ([`claims_an_item`]: a cast, a reel, a Firestarter
    /// lighting, a swing) counts as NON-claiming ([`Entry::claims_now`]), so
    /// the eviction picks it first and it never fills the ledger with claims;
    /// and it too asks first (`can_afford` says no for it as well while every
    /// entry claims), so it can't end a queued claim.
    pub fn record(&mut self, request: Pending, next_input_seq: u64) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        if self.pending.len() >= MAX_PENDING {
            self.evict_one();
        }
        self.pending.push_back(Entry {
            seq: self.next_seq,
            ends_at_input: next_input_seq,
            claims: true,
            request,
        });
        self.next_seq
    }

    /// C3b-fix-e (L5) — forget one entry to make room (the ledger is at
    /// [`MAX_PENDING`]), the oldest of the first kind there is:
    /// 1. one that claims nothing: its claim has ended (the server has read
    ///    it; its answer, if still to come, is lost), or (C3c-2-fix L4) it
    ///    never claimed (a swing, a sleep, a shear, a cast: only its answer
    ///    is lost);
    /// 2. one already sent (not [`UNTIL_SENT`]): its claim ends within a
    ///    round trip anyway;
    /// 3. the oldest of all — every entry is queued and claims an item, which
    ///    only a caller that records without asking can meet (every
    ///    request that asks is refused first, [`Self::can_afford`]; a swing
    ///    and a sleep still record without asking).
    ///
    /// So a queued request's claim — which lives until it is sent, however
    /// long a carry-over keeps it waiting — is never ended early by the
    /// requests made behind it (C3b-fix-d verify L5: a queued Eat evicted by
    /// 64 swings let a Q-drop of its bread pass, and the server ate it AND
    /// dropped it).
    fn evict_one(&mut self) {
        let rank = |e: &Entry| -> u8 {
            match (e.claims_now(), e.ends_at_input == UNTIL_SENT) {
                (false, _) => 1,
                (true, false) => 2,
                (true, true) => 3,
            }
        };
        let at = (0..self.pending.len()).min_by_key(|&i| (rank(&self.pending[i]), i));
        if let Some(at) = at {
            self.pending.remove(at);
        }
    }

    /// Is the ledger full with every entry still claiming
    /// ([`Self::can_afford`]: no room for a new claim without ending one)?
    fn full_of_claims(&self) -> bool {
        self.pending.len() >= MAX_PENDING && self.pending.iter().all(Entry::claims_now)
    }

    /// C2b — the sequence number for a request the server never answers
    /// (`ItemAction::Drop`): the shared sequence moves
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

    /// C3b-fix-d (A-L1) — request `seq`, queued until now, was just sent
    /// ahead of input `next_input_seq` (`RemoteClient::next_input_seq` at the
    /// send): its claim ends once the server acknowledges that input, as a
    /// request sent at once does. No entry (an unanswered `Drop`, or one
    /// forgotten past [`MAX_PENDING`]): nothing to do.
    pub fn rebase(&mut self, seq: u32, next_input_seq: u64) {
        if let Some(e) = self.pending.iter_mut().find(|e| e.seq == seq) {
            e.ends_at_input = next_input_seq;
        }
    }

    /// C3b-fix-d (A-L1) — request `seq` was discarded unsent (the link
    /// closed, or there was none): it will never be answered, so it is
    /// forgotten and its items are free at once.
    pub fn release(&mut self, seq: u32) {
        self.pending.retain(|e| e.seq != seq);
    }

    /// C3b-fix-e (L7) — the send path's one decision about request `seq`'s
    /// claim, by what became of it (`GameState::flush_window_ops`,
    /// `GameState::send_request`): sent from the queue ahead of input
    /// `next_input_seq` → its claim ends with that input's acknowledgement
    /// ([`Self::rebase`]); discarded from the queue (the link closed) or made
    /// with no connection to send it on → it will never be answered, so it
    /// is released at once ([`Self::release`]). `None` (a `DeviceInteract`,
    /// which claims nothing): nothing to settle. A request sent at once was
    /// recorded with the input it goes ahead of and needs none of this.
    pub fn settle(&mut self, seq: Option<u32>, fate: RequestFate) {
        let Some(seq) = seq else { return };
        match fate {
            RequestFate::Sent { next_input_seq } => self.rebase(seq, next_input_seq),
            RequestFate::Discarded | RequestFate::NoConnection => self.release(seq),
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
    ///
    /// C3b-fix-e (L5) — and, for one that would claim, only while the ledger
    /// has room for it without ending a claim: at [`MAX_PENDING`] with every
    /// entry still claiming, the answer is no, so the use does nothing here,
    /// exactly as when it can't be afforded. C3c-2-fix (L4) — one that claims
    /// nothing (a cast, a reel, a Firestarter lighting) too: recorded, it
    /// would end a queued claim just the same.
    ///
    /// C3c-2-fix (L3) — a shot also claims one unit of its weapon's WEAR: it
    /// goes only while the client's bows (or slingshots) of that kind have
    /// more uses left than the shots in flight will wear ([`wear_held`]), so
    /// a bow on its last use fires once, not again for a round trip after it
    /// breaks.
    pub fn can_afford(&self, inv: &Inventory, ui: &CraftingUi, kind: Asked, held: Option<&Item>) -> bool {
        if self.full_of_claims() {
            return false;
        }
        if let Some(weapon) = kind.shot_weapon()
            && wear_held(inv, &weapon) <= self.wear_claimed(&weapon)
        {
            return false;
        }
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
    /// C3c-2-fix (L3) — for a weapon, its wear too: the uses left after `n`
    /// of it go must still cover the shots in flight.
    pub fn can_spend(&self, inv: &Inventory, ui: &CraftingUi, item: &Item, n: u32) -> bool {
        if count_held(inv, ui, item) < self.claimed(item).saturating_add(n) {
            return false;
        }
        let worn = self.wear_claimed(item);
        worn == 0 || wear_held(inv, item).saturating_sub(n.saturating_mul(uses_left(item))) >= worn
    }

    /// C3c-2-fix (L3) — the units of wear the shots still claiming will take
    /// from weapons of `weapon`'s kind (one each).
    fn wear_claimed(&self, weapon: &Item) -> u32 {
        self.pending
            .iter()
            .filter(|e| e.claims)
            .filter(|e| e.request.kind.shot_weapon().is_some_and(|w| same_item(&w, weapon)))
            .count() as u32
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

    /// C3c-3b — is a request asking exactly `kind` still waiting for its
    /// answer (an `Attach` or `Detach` of the same face: a held click sends
    /// it once)?
    pub fn is_asking(&self, kind: Asked) -> bool {
        self.pending.iter().any(|e| e.request.kind == kind)
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
        | Asked::Eat
        | Asked::Shoot { .. }
        | Asked::PlaceCart { .. }
        | Asked::Attach { .. } => 1,
        Asked::Interact(InteractKind::Shear | InteractKind::LeadDetach | InteractKind::SitToggle)
        | Asked::Swing
        | Asked::Sleep { .. }
        | Asked::Cast
        | Asked::Reel
        | Asked::Detach { .. } => 0,
        Asked::UseBlock { claim, .. } => claim,
        Asked::Light { lighter, .. } => u8::from(lighter != crate::block_use::Lighter::Firestarter),
    }
}

/// C3b-fix-e (L5) — does `request` claim an item while in flight (it uses
/// one, and was made with one in hand)?
fn claims_an_item(request: &Pending) -> bool {
    uses(request.kind) > 0 && request.held.is_some()
}

/// C3c-2-fix (L3) — the uses a tool has left before it breaks (a tool at 0
/// still has the one that breaks it); 0 for anything else.
fn uses_left(item: &Item) -> u32 {
    match item {
        Item::Tool(t) => u32::from(t.durability).max(1),
        _ => 0,
    }
}

/// C3c-2-fix (L3) — the uses left in every tool of `tool`'s kind in the
/// client's 36 slots: where a shot's wear lands ([`where_now`], on both
/// copies; a bow on the cursor or in the grid can't be worn).
fn wear_held(inv: &Inventory, tool: &Item) -> u32 {
    inv.slots_iter()
        .flatten()
        .filter(|s| same_item(&s.item, tool))
        .map(|s| uses_left(&s.item).saturating_mul(u32::from(s.count)))
        .sum()
}

/// Is `a` the item `b` was, for an outcome's purposes? A tool is the same
/// tool by type and material, and an armour piece by slot and material
/// (durability is what wears; the shadow's copy never does — C2b verify L6).
/// C3c-3a — a Plan is the same Plan by its marker (`plan::PlanData::same_plan`):
/// the server's copy holds a joiner's Plan as a marker placeholder, so an
/// owed take of a Plan (a hung print's) finds it by marker, in the exact
/// pass and the kind pass of [`take_owed_search`] alike.
pub(crate) fn same_item(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::Tool(a), Item::Tool(b)) => a.tool_type == b.tool_type && a.material == b.material,
        (Item::Armour(a), Item::Armour(b)) => a.slot == b.slot && a.material == b.material,
        (Item::Plan(a), Item::Plan(b)) => a.same_plan(b),
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
/// grid and the cursor (the window the server mirrors since C3a-2a).
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
/// it, else the first slot (of all 36) that does (a tool by type and
/// material, an armour piece by slot and material). One rule for both copies
/// of a joiner's window (C3a-2a): the client wears an accepted swing's
/// weapon there ([`apply_outcome`]), and the server wears its copy there
/// (`hosted_server::wear_joiner_weapon`), so a weapon moved off its hotbar
/// slot while the swing was in flight wears the same piece on both sides.
/// (C3b-fix-c: the owed takes no longer use it; they search with
/// [`take_owed_search`], an exact match first.)
pub fn where_now(inv: &crate::inventory::Inventory, slot: usize, held: &Item) -> Option<usize> {
    let holds = |i: usize| inv.slot(i).is_some_and(|s| same_item(&s.item, held));
    if holds(slot) {
        return Some(slot);
    }
    (0..36).find(|&i| holds(i))
}

/// C3b-fix-c (decision 2) — THE owed-take search, one fn for every take of a
/// joiner's window, run the same way by both copies of it (a LOCKSTEP rule:
/// client and server must call this, or they diverge). It looks, in order:
/// the hint slot `hint`, then the 36 slots, then the crafting grid
/// (row-major), then the cursor, then the armour slots — each only where the
/// caller passes it. C3b-fix-e (L1) — first for an exact match (`==`,
/// durability included) in every one of those places, and only then, in the
/// same order, for a match by kind ([`same_item`]): so a tool owed is that
/// tool wherever it can be told apart, never the player's own one of its
/// kind, not even one moved into the hint slot (C3b-fix verify B-L1, A-L4;
/// C3b-fix-c verify L1). Takes up to `n` units, one at a time, and returns
/// the units it took, as stacks of equal items in the order taken: a tool or
/// armour piece can differ from `held` by its durability.
pub(crate) fn take_owed_search(
    inv: &mut Inventory,
    mut grid: Option<&mut crate::window::CraftGrid>,
    mut cursor: Option<&mut Option<ItemStack>>,
    mut armour: Option<&mut [Option<ArmourItem>; 4]>,
    hint: Option<usize>,
    held: &Item,
    n: u8,
) -> Vec<ItemStack> {
    let mut taken: Vec<ItemStack> = Vec::new();
    for _ in 0..n {
        let Some(item) = take_one_owed(inv, grid.as_deref_mut(), cursor.as_deref_mut(), armour.as_deref_mut(), hint, held)
        else {
            break;
        };
        match taken.last_mut() {
            Some(last) if last.item == item => last.count += 1,
            _ => taken.push(ItemStack { item, count: 1 }),
        }
    }
    taken
}

/// One unit of [`take_owed_search`]: the item it took, or `None` when no
/// place it searches holds `held`. C3b-fix-e (L1) — two passes over the same
/// places in the same order (the hint slot, the 36 slots, the grid, the
/// cursor, the armour slots): an exact match (`==`) first EVERYWHERE, and
/// only then one of its kind ([`same_item`]). So no place's kind match beats
/// another place's exact one — the hint slot's included, where a worn tool
/// swapped in inside the round trip was taken for the fresh phantom.
/// C3c-3a — a Plan matches exactly by its marker (`plan::PlanData::same_plan`):
/// the copy's marker placeholder is the Plan the claim names.
fn take_one_owed(
    inv: &mut Inventory,
    mut grid: Option<&mut crate::window::CraftGrid>,
    mut cursor: Option<&mut Option<ItemStack>>,
    mut armour: Option<&mut [Option<ArmourItem>; 4]>,
    hint: Option<usize>,
    held: &Item,
) -> Option<Item> {
    let exact = |s: &ItemStack| match (&s.item, held) {
        (Item::Plan(a), Item::Plan(b)) => a.same_plan(b),
        (a, b) => a == b,
    };
    let kind = |s: &ItemStack| same_item(&s.item, held);
    let passes: [&dyn Fn(&ItemStack) -> bool; 2] = [&exact, &kind];
    passes.into_iter().find_map(|matches| {
        take_one_where(inv, grid.as_deref_mut(), cursor.as_deref_mut(), armour.as_deref_mut(), hint, matches)
    })
}

/// One pass of [`take_one_owed`]: the first place, in the search's order,
/// holding a stack `matches` accepts gives up one unit.
fn take_one_where(
    inv: &mut Inventory,
    grid: Option<&mut crate::window::CraftGrid>,
    cursor: Option<&mut Option<ItemStack>>,
    armour: Option<&mut [Option<ArmourItem>; 4]>,
    hint: Option<usize>,
    matches: &dyn Fn(&ItemStack) -> bool,
) -> Option<Item> {
    let slot = hint
        .filter(|&h| h < 36 && inv.slot(h).is_some_and(matches))
        .or_else(|| (0..36).find(|&i| inv.slot(i).is_some_and(matches)));
    if let Some(at) = slot {
        let mut stack = inv.take_slot(at)?;
        let item = stack.item.clone();
        stack.count = stack.count.saturating_sub(1);
        if stack.count > 0 {
            inv.set_slot(at, Some(stack));
        }
        return Some(item);
    }
    if let Some(grid) = grid
        && let Some(k) = (0..9).find(|&k| grid[k / 3][k % 3].as_ref().is_some_and(matches))
    {
        return take_one_from(&mut grid[k / 3][k % 3]);
    }
    if let Some(cursor) = cursor
        && cursor.as_ref().is_some_and(matches)
    {
        return take_one_from(cursor);
    }
    let armour = armour?;
    let at = (0..armour.len()).find(|&i| armour[i].is_some_and(|p| matches(&ItemStack { item: Item::Armour(p), count: 1 })))?;
    armour[at].take().map(Item::Armour)
}

/// Take one unit from `cell`: its item.
fn take_one_from(cell: &mut Option<ItemStack>) -> Option<Item> {
    let stack = cell.as_mut()?;
    let item = stack.item.clone();
    stack.count = stack.count.saturating_sub(1);
    if stack.count == 0 {
        *cell = None;
    }
    Some(item)
}

/// Units in what [`take_owed_search`] took.
fn units(taken: &[ItemStack]) -> u8 {
    taken.iter().map(|s| s.count).fold(0, u8::saturating_add)
}

/// Take what an accepted interaction OWES (review D2b LOW-1): `n` of `held`,
/// each from `slot` if it still holds one, else from wherever one now is in
/// the 36 slots (C3b-fix-c: an exact match first, [`take_owed_search`]).
/// Returns how many were taken (fewer only when the inventory runs out).
///
/// The 36-slot part of [`take_owed_window`], which both copies of a
/// joiner's window run for an accepted outcome. Also the server's rule for a
/// Q-drop's item (`item_actions::serve_drop`).
pub fn take_owed(inv: &mut crate::inventory::Inventory, slot: usize, held: &Item, n: u8) -> u8 {
    units(&take_owed_search(inv, None, None, None, Some(slot), held, n))
}

/// The owed payment (C2b decision 5; C3a-2a: one rule for both copies of a
/// joiner's window): `n` of `held`, from the 36 slots first ([`take_owed`]:
/// the request's slot if it still holds one, else wherever one is), then the
/// crafting grid (row-major), then the cursor — an exact match first in all
/// of them, then one of its kind ([`take_owed_search`], C3b-fix-c; C3b-fix-e
/// L1). Returns how many were taken.
/// The client runs it on its own window ([`take_owed_held`]) when the
/// outcome arrives, the server on its copy (`hosted_server::shadow_take_owed`,
/// a `window_events::WindowEvent::Take`) once the client reports it applied
/// that outcome (C3a-fix-1): the same point in the client's order on both
/// sides, whatever window ops crossed the outcome in flight.
pub fn take_owed_window(
    inv: &mut Inventory,
    grid: &mut crate::window::CraftGrid,
    cursor: &mut Option<crate::item::ItemStack>,
    slot: usize,
    held: &Item,
    n: u8,
) -> u8 {
    units(&take_owed_search(inv, Some(grid), Some(cursor), None, Some(slot), held, n))
}

/// C3b-fix-c (decision 2) — a correction's take
/// (`container_window::CorrectionDebt::apply`, run by both copies of a
/// joiner's window): [`take_owed_window`]'s places, then the armour slots, so
/// a phantom the player equipped inside the correction's round trip is taken
/// off on both sides (C3b-fix verify B-M1 scenario 2). Player-visible: a
/// correction can take an equipped piece off (Spec 05).
pub fn take_correction(
    inv: &mut Inventory,
    grid: &mut crate::window::CraftGrid,
    cursor: &mut Option<crate::item::ItemStack>,
    armour: &mut [Option<ArmourItem>; 4],
    hint: usize,
    held: &Item,
    n: u8,
) -> u8 {
    units(&take_owed_search(inv, Some(grid), Some(cursor), Some(armour), Some(hint), held, n))
}

/// The client's owed payment: [`take_owed_window`] on its window (`inv`, and
/// `ui`'s grid and cursor), then the result shown is recomputed.
pub fn take_owed_held(inv: &mut Inventory, ui: &mut CraftingUi, slot: usize, held: &Item, n: u8) -> u8 {
    let taken = take_owed_window(inv, &mut ui.grid, &mut ui.cursor_item, slot, held, n);
    ui.update_result();
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
        Asked::Eat
        | Asked::Sleep { .. }
        | Asked::UseBlock { .. }
        | Asked::Shoot { .. }
        | Asked::PlaceCart { .. }
        | Asked::Light { .. }
        | Asked::Cast
        | Asked::Reel
        | Asked::Attach { .. }
        | Asked::Detach { .. } => {}
    }
    applied
}

/// C2a — apply an item action's outcome to the joiner's inventory: an
/// accepted one takes `consume_held` of what it claimed (the eaten food;
/// C3b-2, what a block use took; C3c-2, a shot's ammo — searched from the
/// weapon's slot, which never holds it, so by the shared search alone — and
/// a placed cart), owed like an interaction's
/// ([`take_owed_held`]: the 36 slots, then `ui`'s grid, then its cursor —
/// C2a verify L6). Nothing on a refusal. Returns how many were taken.
pub fn apply_item_outcome(
    inv: &mut Inventory,
    ui: &mut CraftingUi,
    request: &Pending,
    outcome: &ItemActionOutcomePacket,
) -> u8 {
    if !outcome.accepted
        || !matches!(
            request.kind,
            Asked::Eat
                | Asked::Sleep { .. }
                | Asked::UseBlock { .. }
                | Asked::Light { .. }
                | Asked::Shoot { .. }
                | Asked::PlaceCart { .. }
                | Asked::Attach { .. }
        )
    {
        return 0;
    }
    let Some(held) = request.held.as_ref() else {
        return 0;
    };
    take_owed_held(inv, ui, request.hotbar_slot, held, outcome.consume_held)
}

/// C3b-2 — an accepted block use that wore its tool (`wear_held`: shears on
/// a hive; C3c-2 flint and steel on a campfire) wears it where it now is
/// ([`where_now`]), as an accepted swing does; the server wears its copy at
/// the same point (the outcome's window event). C3c-2 — a shot wears its
/// weapon ([`Asked::shot_weapon`]), a reel that caught its rod. `None` when
/// nothing wore.
pub fn apply_use_wear(
    inv: &mut Inventory,
    request: &Pending,
    outcome: &ItemActionOutcomePacket,
) -> Option<crate::inventory::ToolUseInfo> {
    if !outcome.accepted || !outcome.wear_held {
        return None;
    }
    let tool = match request.kind {
        Asked::UseBlock { .. } | Asked::Light { .. } | Asked::Reel => request.held.clone()?,
        Asked::Shoot { .. } => request.kind.shot_weapon()?,
        _ => return None,
    };
    let at = where_now(inv, request.hotbar_slot, &tool)?;
    inv.use_tool_at(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::inventory::Inventory;
    use crate::item::{ItemStack, MaterialId};

    fn outcome(seq: u32, accepted: bool, consume_held: u8) -> InteractOutcomePacket {
        InteractOutcomePacket { seq, entity: 1, kind: None, accepted, consume_held, note: 0, window_event: 0 }
    }

    fn inv_with(slot: usize, stack: ItemStack) -> Inventory {
        let mut inv = Inventory::new();
        inv.set_slot(slot, Some(stack));
        inv
    }

    fn sword() -> Item {
        Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))
    }

    /// C3b-2 — a block use claims what its rule could take (a bucket on a
    /// hive), so a second use can't spend the same bucket; shears claim
    /// nothing and wear only on an accepted outcome that says so.
    #[test]
    fn a_block_use_claims_its_bucket_and_shears_wear_only_when_told() {
        use crate::block_use::UseKind;
        let bucket = Item::Material(MaterialId::Bucket);
        let inv = inv_with(0, ItemStack { item: bucket.clone(), count: 1 });
        let hive = |claim| Asked::UseBlock { cell: [1, 2, 3], kind: UseKind::Hive, claim };
        let mut a = JoinerActions::default();
        assert_eq!(uses(hive(1)), 1);
        assert!(a.can_afford(&inv, &CraftingUi::new(), hive(1), Some(&bucket)));
        a.record(Pending { kind: hive(1), mob: None, hotbar_slot: 0, held: Some(bucket.clone()) }, 5);
        assert!(!a.can_afford(&inv, &CraftingUi::new(), hive(1), Some(&bucket)), "the only bucket is claimed");
        let shears = Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron));
        let mut inv = inv_with(2, ItemStack { item: shears.clone(), count: 1 });
        let req = Pending { kind: hive(0), mob: None, hotbar_slot: 2, held: Some(shears) };
        let out = |accepted, wear_held| ItemActionOutcomePacket {
            seq: 1,
            accepted,
            consume_held: 0,
            note: 0,
            window_event: 7,
            wear_held,
            bite_after: 0,
        };
        assert!(apply_use_wear(&mut inv, &req, &out(false, true)).is_none(), "refused: no wear");
        assert!(apply_use_wear(&mut inv, &req, &out(true, false)).is_none(), "not told to wear");
        assert!(apply_use_wear(&mut inv, &req, &out(true, true)).is_some(), "worn");
        assert_eq!(apply_item_outcome(&mut inv, &mut CraftingUi::new(), &req, &out(true, true)), 0, "nothing taken");
    }

    /// C3c-2 — a shot claims its AMMO (one arrow in flight per arrow held),
    /// and its accepted outcome takes the arrow by the shared search and
    /// wears the bow where it now is; a refusal changes nothing.
    #[test]
    fn a_shot_claims_its_ammo_takes_it_and_wears_the_bow() {
        use crate::protocol::ShotWeapon;
        let bow = Tool::new(ToolType::Bow, ToolMaterial::Wood);
        let arrow = Item::Material(MaterialId::Arrow);
        let mut inv = inv_with(0, ItemStack::new_tool(bow));
        inv.set_slot(20, Some(ItemStack { item: arrow.clone(), count: 1 }));
        let shoot = Asked::Shoot { weapon: ShotWeapon::Bow, material: ToolMaterial::Wood };
        assert_eq!(shoot.shot_weapon(), Some(Item::Tool(bow)));
        let mut a = JoinerActions::default();
        let ui = CraftingUi::new();
        assert!(a.can_afford(&inv, &ui, shoot, Some(&arrow)));
        let req = Pending { kind: shoot, mob: None, hotbar_slot: 0, held: Some(arrow.clone()) };
        a.record(req.clone(), 3);
        assert!(!a.can_afford(&inv, &ui, shoot, Some(&arrow)), "the only arrow is in flight");
        assert!(!a.can_spend(&inv, &ui, &arrow, 1), "and can't be dropped meanwhile");
        let out = |accepted| ItemActionOutcomePacket {
            seq: 1,
            accepted,
            consume_held: 1,
            note: 0,
            window_event: 4,
            wear_held: accepted,
            bite_after: 0,
        };
        let mut ui = CraftingUi::new();
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &req, &out(false)), 0);
        assert!(apply_use_wear(&mut inv, &req, &out(false)).is_none());
        assert_eq!(apply_item_outcome(&mut inv, &mut ui, &req, &out(true)), 1);
        assert!(inv.slot(20).is_none(), "the arrow, from wherever it is");
        assert!(apply_use_wear(&mut inv, &req, &out(true)).is_some());
        assert_eq!(durability(&inv, 0), u32::from(bow.durability) - 1, "the bow wore once");
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

    /// C3b-fix-c (decision 2) — an owed take prefers the exact instance
    /// (durability included) in each place it looks: the player's own worn
    /// pickaxe isn't taken for the fresh one it owes.
    #[test]
    fn an_owed_take_prefers_the_exact_instance_over_one_of_its_kind() {
        let fresh = Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond);
        let worn = Tool { durability: fresh.durability - 100, ..fresh };
        let mut inv = Inventory::new();
        inv.set_slot(3, Some(ItemStack::new_tool(worn)));
        inv.set_slot(10, Some(ItemStack::new_tool(fresh)));
        assert_eq!(take_owed(&mut inv, 0, &Item::Tool(fresh), 1), 1);
        assert_eq!(inv.slot(3), Some(&ItemStack::new_tool(worn)), "its own worn pickaxe stays");
        assert!(inv.slot(10).is_none(), "the fresh one was taken");
        // No exact one: one of its kind still pays.
        assert_eq!(take_owed(&mut inv, 0, &Item::Tool(fresh), 1), 1);
        assert!(inv.slot(3).is_none());
    }

    /// C3b-fix-e (L1) — exact before kind EVERYWHERE: a worn pickaxe swapped
    /// into the hint slot is not taken for the fresh phantom elsewhere (the
    /// hint used to be matched by kind before any exact match, which turned
    /// worn into fresh). Then the grid, cursor and armour are searched
    /// exactly before any place is searched by kind.
    #[test]
    fn an_exact_match_anywhere_beats_one_of_its_kind_in_the_hint_slot() {
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        let fresh = Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond);
        let worn = Tool { durability: fresh.durability - 100, ..fresh };
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_tool(worn)));
        inv.set_slot(10, Some(ItemStack::new_tool(fresh)));
        // Both sides run the one search: the server's copy and the client.
        assert_eq!(take_owed(&mut inv, 0, &Item::Tool(fresh), 1), 1);
        assert_eq!(inv.slot(0), Some(&ItemStack::new_tool(worn)), "the worn one in the hint slot stays: no repair");
        assert!(inv.slot(10).is_none(), "the fresh phantom was taken");
        // The fresh one on the cursor, the worn one in the hint slot and a
        // worn one in the grid: the cursor's exact match wins.
        let mut inv = inv_with(0, ItemStack::new_tool(worn));
        let mut grid: crate::window::CraftGrid = Default::default();
        grid[0][0] = Some(ItemStack::new_tool(worn));
        let mut cursor = Some(ItemStack::new_tool(fresh));
        assert_eq!(take_owed_window(&mut inv, &mut grid, &mut cursor, 0, &Item::Tool(fresh), 1), 1);
        assert_eq!(cursor, None, "the exact one on the cursor");
        assert!(inv.slot(0).is_some() && grid[0][0].is_some(), "neither worn one was touched");
        // An equipped exact chestplate beats a worn one in the hint slot.
        let plate = ArmourItem::new(ArmourSlot::Chestplate, ArmourMaterial::Iron);
        let mut worn_plate = plate;
        worn_plate.durability = worn_plate.durability.saturating_sub(7);
        let mut inv = inv_with(0, ItemStack { item: Item::Armour(worn_plate), count: 1 });
        let mut armour = [None, Some(plate), None, None];
        let (mut grid, mut cursor) = (Default::default(), None);
        assert_eq!(take_correction(&mut inv, &mut grid, &mut cursor, &mut armour, 0, &Item::Armour(plate), 1), 1);
        assert_eq!(armour[1], None, "the exact piece, taken off");
        assert!(inv.slot(0).is_some(), "the worn one in the hint slot stays");
        // No exact one anywhere: the hint slot's kind match pays first.
        let mut inv = inv_with(0, ItemStack::new_tool(worn));
        inv.set_slot(4, Some(ItemStack::new_tool(worn)));
        assert_eq!(take_owed(&mut inv, 4, &Item::Tool(fresh), 1), 1);
        assert!(inv.slot(4).is_none() && inv.slot(0).is_some(), "by kind, the hint first");
    }

    /// C3b-fix-c (decision 2) — a correction's take searches the hint, the
    /// 36 slots, the grid, the cursor, then the armour slots: an equipped
    /// phantom is taken; the other owed takes never reach armour.
    #[test]
    fn a_correction_take_reaches_the_armour_slots_and_an_owed_take_does_not() {
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        let plate = ArmourItem::new(ArmourSlot::Chestplate, ArmourMaterial::Iron);
        let mut inv = Inventory::new();
        let mut grid: crate::window::CraftGrid = Default::default();
        let mut cursor = None;
        let mut armour = [None, Some(plate), None, None];
        assert_eq!(take_owed_window(&mut inv, &mut grid, &mut cursor, 0, &Item::Armour(plate), 1), 0);
        assert_eq!(armour[1], Some(plate));
        assert_eq!(take_correction(&mut inv, &mut grid, &mut cursor, &mut armour, 0, &Item::Armour(plate), 1), 1);
        assert_eq!(armour[1], None, "taken off");
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

    /// C3b-fix-d (A-L1) — a request queued behind unsent edits claims until
    /// it is SENT: the acknowledgements of the inputs it waits behind end
    /// nothing, however many. Once sent (rebased to the input after it) it
    /// claims until that input is acknowledged, like any request.
    #[test]
    fn a_queued_request_claims_until_it_is_sent_then_until_the_input_after_it() {
        let bread = Item::Material(MaterialId::Bread);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bread, 1));
        let ui = CraftingUi::new();
        let mut a = JoinerActions::default();
        let eat = a.record(Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(bread.clone()) }, UNTIL_SENT);
        // A long carry-over: inputs 10..=14 go out ahead of it, and the
        // server acknowledges every one.
        for acked in 10..=14 {
            a.acknowledged(acked);
        }
        assert!(a.eat_in_flight(), "still queued: nothing the server read covers it");
        assert!(!a.can_spend(&inv, &ui, &bread, 1), "a Q-drop of the bread it claims is refused");
        // Sent right after input 14: it rides ahead of input 15.
        a.rebase(eat, 15);
        a.acknowledged(14);
        assert!(!a.can_spend(&inv, &ui, &bread, 1), "an acknowledgement from before the send ends nothing");
        a.acknowledged(15);
        assert!(a.can_spend(&inv, &ui, &bread, 1), "read by the server: the bread is free");
        assert!(!a.eat_in_flight());
        assert!(a.take(eat).is_some(), "the entry stays for its answer");
        // A seq with no entry (an unanswered Drop) moves nothing.
        a.rebase(999, 1);
        assert_eq!(a.len(), 0);
    }

    /// C3b-fix-d (A-L1) — a queued request discarded unsent releases its
    /// claim at once (it will never be answered), and only its own.
    #[test]
    fn a_queued_request_discarded_unsent_releases_its_claim() {
        let bread = Item::Material(MaterialId::Bread);
        let bucket = Item::Material(MaterialId::Bucket);
        let mut inv = inv_with(0, ItemStack::new_material(MaterialId::Bread, 1));
        inv.set_slot(1, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        let ui = CraftingUi::new();
        let mut a = JoinerActions::default();
        let eat = a.record(Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(bread.clone()) }, UNTIL_SENT);
        let milk = Pending {
            kind: Asked::Interact(InteractKind::Milk),
            mob: Some(MobType::Cow),
            hotbar_slot: 1,
            held: Some(bucket.clone()),
        };
        a.record(milk, UNTIL_SENT);
        assert!(!a.can_spend(&inv, &ui, &bread, 1) && !a.can_spend(&inv, &ui, &bucket, 1));
        a.release(eat);
        assert!(a.can_spend(&inv, &ui, &bread, 1), "the discarded Eat's bread is free");
        assert!(!a.eat_in_flight());
        assert!(!a.can_spend(&inv, &ui, &bucket, 1), "the other request still claims its bucket");
        assert_eq!(a.len(), 1);
        a.release(999);
        assert_eq!(a.len(), 1, "a seq with no entry releases nothing");
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
        let out = |accepted, consume_held| ItemActionOutcomePacket { seq: 1, accepted, consume_held, note: 0, window_event: 0, wear_held: false, bite_after: 0 };
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
        ui.open_table_crafting([0, 64, 0], &Inventory::new(), &[None; 4]);
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
        planks.open_player_crafting(&Inventory::new(), &[None; 4]);
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
        ui.open_player_crafting(&Inventory::new(), &[None; 4]);
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
        let out = ItemActionOutcomePacket { seq: 1, accepted: true, consume_held: 1, note: 0, window_event: 0, wear_held: false, bite_after: 0 };
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
        ui.open_player_crafting(&Inventory::new(), &[None; 4]);
        ui.grid[0][1] = Some(ItemStack::new_material(MaterialId::Bread, 2));
        let eat = Pending { kind: Asked::Eat, mob: None, hotbar_slot: 4, held: Some(bread) };
        let out = ItemActionOutcomePacket { seq: 1, accepted: true, consume_held: 1, note: 0, window_event: 0, wear_held: false, bite_after: 0 };
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
        ui.open_table_crafting([0, 64, 0], &Inventory::new(), &[None; 4]);
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

    /// C3b-fix-e (L5) — a claim is never evicted: an Eat queued behind a
    /// long carry-over, then 64 more requests while it waits (swings, each
    /// still claiming). The ledger stays bounded by forgetting the oldest
    /// swing, never the Eat: a Q-drop of its bread is refused locally, and
    /// so is a new request that would claim an item while every entry still
    /// claims. Once the Eat is sent and read, the bread is free.
    #[test]
    fn a_full_ledger_never_evicts_a_claim_and_refuses_a_new_claiming_request() {
        let bread = Item::Material(MaterialId::Bread);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bread, 2));
        let ui = CraftingUi::new();
        let mut a = JoinerActions::default();
        let eat = a.record(Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(bread.clone()) }, UNTIL_SENT);
        let swing = Pending { kind: Asked::Swing, mob: Some(MobType::Pig), hotbar_slot: 1, held: Some(sword()) };
        let first_swing = a.record(swing.clone(), 7);
        for _ in 0..MAX_PENDING {
            a.record(swing.clone(), 7);
        }
        assert_eq!(a.len(), MAX_PENDING, "bounded");
        assert!(a.take(first_swing).is_none(), "the oldest sent swing was forgotten, not the queued Eat");
        // `take` drained nothing older than an entry it didn't find: the Eat
        // still claims one of the two loaves.
        assert!(a.eat_in_flight(), "the Eat still claims");
        assert!(a.can_spend(&inv, &ui, &bread, 1), "one loaf is unclaimed");
        assert!(!a.can_spend(&inv, &ui, &bread, 2), "a Q-drop of the claimed loaf is refused");
        // C3c-2-fix (L4) — a swing claims nothing, so it is recorded as
        // non-claiming: the ledger isn't full of claims, and a new claim may
        // go — it evicts a swing, never the queued Eat.
        assert!(a.can_afford(&inv, &ui, Asked::Interact(InteractKind::Feed), Some(&bread)), "the swings claim nothing");
        assert!(a.can_afford(&inv, &ui, Asked::Swing, Some(&sword())), "one that claims nothing may still go");
        a.record(Pending { kind: Asked::Interact(InteractKind::Feed), mob: Some(MobType::Pig), hotbar_slot: 0, held: Some(bread.clone()) }, 7);
        assert_eq!(a.len(), MAX_PENDING);
        assert!(a.eat_in_flight(), "the Eat still claims: a swing went instead");
        assert!(!a.can_spend(&inv, &ui, &bread, 1), "both loaves are claimed now");
        // The queue drains: the Eat goes out ahead of input 8, and the
        // server reads every input up to it.
        a.rebase(eat, 8);
        a.acknowledged(8);
        assert!(!a.eat_in_flight());
        assert!(a.can_spend(&inv, &ui, &bread, 2), "the claims ended: both loaves are free");
        assert!(a.can_afford(&inv, &ui, Asked::Interact(InteractKind::Feed), Some(&bread)), "and a new claim may go");
    }

    /// C3b-fix-e (L7) — the send path's one decision about a request's
    /// claim: sent (now from the queue) → rebased to the input after it;
    /// discarded unsent, or no connection to send it on → released. A
    /// request with no seq (a device interact) settles nothing.
    #[test]
    fn the_send_path_rebases_a_sent_request_and_releases_one_never_sent() {
        let bread = Item::Material(MaterialId::Bread);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bread, 1));
        let ui = CraftingUi::new();
        let eat = |a: &mut JoinerActions| {
            a.record(Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(bread.clone()) }, UNTIL_SENT)
        };
        // Sent from the queue ahead of input 12: claims until 12 is read.
        let mut a = JoinerActions::default();
        let seq = eat(&mut a);
        a.settle(Some(seq), RequestFate::Sent { next_input_seq: 12 });
        a.acknowledged(11);
        assert!(!a.can_spend(&inv, &ui, &bread, 1), "an acknowledgement from before the send ends nothing");
        a.acknowledged(12);
        assert!(a.can_spend(&inv, &ui, &bread, 1), "read: free");
        assert!(a.take(seq).is_some(), "the entry stays for its answer");
        // Discarded from the queue (the link closed): released at once.
        let mut a = JoinerActions::default();
        let seq = eat(&mut a);
        a.settle(Some(seq), RequestFate::Discarded);
        assert!(a.can_spend(&inv, &ui, &bread, 1) && a.len() == 0, "released");
        // No connection to send it on: released at once.
        let mut a = JoinerActions::default();
        let seq = eat(&mut a);
        a.settle(Some(seq), RequestFate::NoConnection);
        assert!(a.can_spend(&inv, &ui, &bread, 1) && a.len() == 0, "released");
        // No seq: nothing settles.
        let mut a = JoinerActions::default();
        eat(&mut a);
        a.settle(None, RequestFate::NoConnection);
        assert_eq!(a.len(), 1);
    }

    /// C3c-3a — an owed take of a Plan finds the server's marker placeholder
    /// by marker: with two Plans in the copy and the OTHER one in the hint
    /// slot, the named one goes, from wherever it is.
    #[test]
    fn an_owed_plan_is_taken_by_its_marker() {
        use crate::plan::{marker, DevelopState, PlanData};
        let a = PlanData::debug_3x3_stone();
        let b = PlanData { develop_state: DevelopState::Latent { exposure_ticks: 0 }, ..a.clone() };
        let (ma, mb) = (marker(&a), marker(&b));
        assert_ne!(ma, mb);
        let placeholder = |m, developed| Item::Plan(PlanData::marker_placeholder(m, developed));
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack { item: placeholder(mb, false), count: 1 }));
        inv.set_slot(5, Some(ItemStack { item: placeholder(ma, true), count: 1 }));
        // The claim decodes to A's placeholder; the hint (slot 0) holds B.
        assert_eq!(take_owed(&mut inv, 0, &placeholder(ma, true), 1), 1);
        assert!(inv.slot(5).is_none(), "A went");
        assert_eq!(inv.slot(0).map(|s| s.item.clone()), Some(placeholder(mb, false)), "B stayed");
        // A real Plan claim matches its placeholder too (by marker).
        assert_eq!(take_owed(&mut inv, 3, &Item::Plan(b.clone()), 1), 1);
        assert!(inv.slot(0).is_none());
        assert_eq!(take_owed(&mut inv, 0, &Item::Plan(b), 1), 0, "none left");
        assert!(same_item(&placeholder(ma, true), &Item::Plan(a)));
    }

    /// C3c-2-fix (L4) — every recorder asks first. With the ledger full of
    /// queued item claims, a cast, a reel or a Firestarter lighting (which
    /// claim nothing) is refused locally like a claiming request: recorded,
    /// it would evict a queued claim (and a Q-drop of that item could then
    /// pass). A request that claims nothing is recorded as non-claiming, so
    /// a claim made after it evicts IT first.
    #[test]
    fn a_cast_reel_or_firestarter_never_evicts_a_queued_claim() {
        let bread = Item::Material(MaterialId::Bread);
        let inv = inv_with(0, ItemStack::new_material(MaterialId::Bread, 64));
        let ui = CraftingUi::new();
        let rod = Item::Tool(Tool::new(ToolType::FishingRod, ToolMaterial::Wood));
        let firestarter = Item::Material(MaterialId::MagnesiumFirestarter);
        let eat = || Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(bread.clone()) };
        let mut a = JoinerActions::default();
        for _ in 0..MAX_PENDING {
            a.record(eat(), UNTIL_SENT);
        }
        assert!(!a.can_afford(&inv, &ui, Asked::Cast, Some(&rod)), "a cast would evict a queued Eat");
        assert!(!a.can_afford(&inv, &ui, Asked::Reel, Some(&rod)));
        let light = Asked::Light { cell: [1, 2, 3], lighter: crate::block_use::Lighter::Firestarter };
        assert!(!a.can_afford(&inv, &ui, light, Some(&firestarter)));
        // A cast recorded first, then 63 Eats: one more Eat may go, and the
        // cast is what it evicts.
        let mut b = JoinerActions::default();
        let cast = b.record(Pending { kind: Asked::Cast, mob: None, hotbar_slot: 1, held: Some(rod.clone()) }, UNTIL_SENT);
        let mut eats = Vec::new();
        for _ in 0..MAX_PENDING - 1 {
            eats.push(b.record(eat(), UNTIL_SENT));
        }
        assert!(b.can_afford(&inv, &ui, Asked::Eat, Some(&bread)), "the cast claims nothing");
        eats.push(b.record(eat(), UNTIL_SENT));
        assert_eq!(b.len(), MAX_PENDING);
        assert!(!b.can_spend(&inv, &ui, &bread, 1), "all 64 Eats still claim");
        b.release(cast);
        assert_eq!(b.len(), MAX_PENDING, "the cast was the one evicted");
    }

    /// C3c-2-fix (L3) — a shot claims one unit of its weapon's WEAR as well as
    /// its arrow: a bow on its last use can't fire a second shot inside the
    /// round trip (the first breaks it), nor be dropped while its shot is in
    /// flight; a bow with wear to spare fires on as before. The claim ends
    /// with the acknowledgement, as every claim does.
    #[test]
    fn a_shot_claims_its_weapons_wear_so_a_bow_on_its_last_use_fires_once() {
        use crate::protocol::ShotWeapon;
        let ui = CraftingUi::new();
        let arrow = Item::Material(MaterialId::Arrow);
        let shoot = Asked::Shoot { weapon: ShotWeapon::Bow, material: ToolMaterial::Wood };
        let shot = || Pending { kind: shoot, mob: None, hotbar_slot: 0, held: Some(arrow.clone()) };
        let mut last = Tool::new(ToolType::Bow, ToolMaterial::Wood);
        last.durability = 1;
        let mut inv = inv_with(0, ItemStack::new_tool(last));
        inv.set_slot(9, Some(ItemStack::new_material(MaterialId::Arrow, 10)));
        let mut a = JoinerActions::default();
        assert!(a.can_afford(&inv, &ui, shoot, Some(&arrow)));
        a.record(shot(), 5);
        assert!(!a.can_afford(&inv, &ui, shoot, Some(&arrow)), "its last use is claimed");
        assert!(!a.can_spend(&inv, &ui, &Item::Tool(last), 1), "nor can it be dropped meanwhile");
        assert!(a.can_spend(&inv, &ui, &arrow, 9), "the other nine arrows are free");
        a.acknowledged(5);
        assert!(a.can_afford(&inv, &ui, shoot, Some(&arrow)), "read: free again");
        // A fresh bow: shots in flight up to its uses left.
        let fresh = Tool::new(ToolType::Bow, ToolMaterial::Wood);
        let mut inv = inv_with(0, ItemStack::new_tool(fresh));
        inv.set_slot(9, Some(ItemStack::new_material(MaterialId::Arrow, 10)));
        let mut b = JoinerActions::default();
        for _ in 0..5 {
            assert!(b.can_afford(&inv, &ui, shoot, Some(&arrow)), "wear to spare");
            b.record(shot(), 5);
        }
        // A second bow of the kind carries the dropped one's claims.
        let mut two = inv.clone();
        two.set_slot(1, Some(ItemStack::new_tool(last)));
        let mut c = JoinerActions::default();
        c.record(shot(), 5);
        assert!(c.can_spend(&two, &ui, &Item::Tool(last), 1), "the fresh bow still covers the shot");
    }
}
