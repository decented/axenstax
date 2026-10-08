//! C3a-fix-1 (protocol v76) — the server's changes to a joiner's window are
//! applied in the order the CLIENT applied them
//! (`docs/foundations/2026-10-07-c3-server-owned-inventory.md` §2; Spec 04
//! §4.2g).
//!
//! The server changes a joiner's window by itself four ways: a grant (a
//! break's yield, an interaction's product, a pickup), the owed take of an
//! accepted request (an eat, a D2b interaction), an armour-wear hit and an
//! accepted swing's weapon wear. C3b-1 (v77) adds a fifth: a container op's
//! correction of player slots (`WindowSlotSet` with `window_event` set,
//! [`WindowEvent::SetSlots`]); its container slots are shared state and are
//! not an event. The client applies each when its packet
//! arrives, so a window op it applied in between reached the server after the
//! change: "change, then op" on the server, "op, then change" on the client,
//! and the two windows diverged (C3a review B-H1).
//!
//! **Server.** Each such change is a [`WindowEvent`], numbered 1, 2, 3… per
//! connection ([`queue`]) and sent on its carrier packet
//! (`InventoryGrant`, `InteractOutcome`, `ItemActionOutcome`, the
//! `ArmourWorn` `PlayerEvent`) as `window_event`. The server does NOT apply it
//! then: it waits in [`WindowEvents`] until a packet from the client reports
//! (`events_applied`) that the client applied it too, and is applied
//! ([`apply_through`]) just before that packet is processed. The content is
//! the server's; only the ORDER follows the client. There is no replay.
//! Events still waiting [`EVENT_ACK_TIMEOUT_TICKS`] after they were sent are
//! applied anyway ([`apply_overdue`], tallied `forced`), and at most
//! [`MAX_PENDING_EVENTS`] wait (past that the oldest is applied, tallied the
//! same). A decision the server makes at the time of an event (C3d's
//! refusals; today the pickup fit while a joiner's pickups follow its window)
//! reads the window with the waiting events applied ([`effective_inventory`]).
//!
//! **Client.** It applies the carriers in arrival order on the one ordered
//! stream ([`WindowInbox`]) and reports the highest event it has applied in
//! every packet the server judges against the window. It applies none while
//! it holds edits it has not sent yet, so every edit of an input was made at
//! the count the input reports (`GameState::apply_window_inbox`).

use std::collections::VecDeque;

use crate::armour::ArmourItem;
use crate::crafting::Tool;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};
use crate::protocol::{InventoryGrantPacket, WireWindowSlot};
use crate::remote_client::RequestOutcome;
use crate::server::ServerPlayer;
use crate::window::CraftGrid;

/// Ticks a window event may wait for the client's word before the server
/// applies it anyway (ten seconds; an honest client reports it within a
/// round trip and a tick).
pub const EVENT_ACK_TIMEOUT_TICKS: u64 = 200;

/// Most window events one joiner may have waiting; past it the oldest is
/// applied at once.
pub const MAX_PENDING_EVENTS: usize = 1024;

/// Applied grants remembered for a `GrantUnfit` to name ([`return_unfit`]).
/// The client reports an unfit grant the moment it applies it, so only the
/// last few can be named.
pub const RECENT_GRANTS: usize = 64;

/// D-M2 — after a `GrantUnfit`, the joiner's server-side pickups follow the
/// server's copy of its window for this long (five seconds) instead of
/// granting the whole stack (the C2b-fix BRIDGE): its client is full, so the
/// item it just gave back would otherwise be picked up, refused and given
/// back again, round and round.
pub const UNFIT_HOLD_TICKS: u64 = 100;

/// One change the server makes to a joiner's window by itself.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowEvent {
    /// `InventoryGrant`: the stack is added (`Inventory::add_item`).
    Grant(ItemStack),
    /// An accepted request's owed take (`joiner_actions::take_owed_window`):
    /// `n` of `item`, from `slot` first. `how` ends the shortfall's log line.
    Take { slot: usize, item: Item, n: u8, how: &'static str },
    /// `hits` hits wear every worn armour piece once each
    /// (`window::wear_armour`).
    WearArmour { hits: u8 },
    /// An accepted swing wears `tool` where it now is
    /// (`joiner_actions::where_now` from `slot`).
    WearWeapon { slot: usize, tool: Tool },
    /// C3b-1 (v77) — a container op's correction of player slots
    /// (`WindowSlotSet`, `window_ops::Correction::player`): each named slot
    /// takes the value the op had when re-run over the client's own claimed
    /// slots and the real container. Container slots are never in it (they
    /// are shared, set at once).
    SetSlots(Vec<(WireWindowSlot, Option<ItemStack>)>),
}

/// One event waiting for the client's word.
#[derive(Debug)]
struct Waiting {
    seq: u32,
    /// The server tick it was sent on.
    sent: u64,
    event: WindowEvent,
}

/// A grant already applied, for a `GrantUnfit` to name.
#[derive(Debug)]
struct AppliedGrant {
    seq: u32,
    stack: ItemStack,
    /// What the server's copy of the window held of it.
    landed: u8,
    /// A `GrantUnfit` named it already (one per grant).
    returned: bool,
}

/// The per-connection counters of the ordered window events, logged with the
/// possession summary when the joiner leaves ([`WindowEvents::summary`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventTally {
    /// Events numbered and sent.
    pub sent: u32,
    /// Events applied without the client's word: waited past
    /// [`EVENT_ACK_TIMEOUT_TICKS`], or pushed out past
    /// [`MAX_PENDING_EVENTS`].
    pub forced: u32,
    /// B-L6 — window ops missing from the `op_seq` sequence (a gap: an op
    /// that never arrived or didn't decode).
    pub ops_lost: u32,
    /// D-M2 — units a `GrantUnfit` gave back as real ground items.
    pub unfit_returned: u32,
    /// D-M2 — units a `GrantUnfit` claimed beyond what its grant gave (or
    /// for a grant it can't name): never spawned.
    pub unfit_clamped: u32,
}

/// One joiner's ordered window events and the window-mirror state that goes
/// with them (`ServerPlayer::window_events`, fresh per attach).
#[derive(Debug, Default)]
pub struct WindowEvents {
    /// The number of the last event sent.
    last_sent: u32,
    /// Events sent but not applied yet, oldest first.
    waiting: VecDeque<Waiting>,
    /// Grants applied lately, oldest first ([`RECENT_GRANTS`]).
    grants: VecDeque<AppliedGrant>,
    /// B-L6 — the last window op's `op_seq` (0 before the first).
    last_op_seq: u32,
    /// Decision 2 — the first digest comparison after join, recorded and not
    /// tallied: `Some(true)` if the windows agreed.
    pub baseline: Option<bool>,
    /// D-M2 — until this server tick the joiner's pickups follow its window
    /// ([`UNFIT_HOLD_TICKS`]).
    pub unfit_hold_until: u64,
    pub tally: EventTally,
}

impl WindowEvents {
    /// Events sent and not applied yet.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// The number of the last event sent (0 before the first).
    #[cfg(test)]
    pub fn last_sent(&self) -> u32 {
        self.last_sent
    }

    /// Does a pickup of this joiner grant the whole stack whatever its
    /// window holds (the C2b-fix BRIDGE), on server tick `now`? Not while a
    /// `GrantUnfit` hold lasts.
    pub fn grants_whole(&self, now: u64) -> bool {
        now >= self.unfit_hold_until
    }

    /// B-L6 — note window op `op_seq`: one past the last is in sequence; a
    /// gap is tallied as lost ops (an op that never arrived, or didn't
    /// decode). An old number (a replay) moves nothing.
    pub fn note_op_seq(&mut self, op_seq: u32) {
        let expected = self.last_op_seq.wrapping_add(1);
        if op_seq > self.last_op_seq {
            self.tally.ops_lost = self.tally.ops_lost.saturating_add(op_seq - expected);
            self.last_op_seq = op_seq;
        }
    }

    /// The one-line addition to the leave summary, or `None` if nothing was
    /// counted.
    pub fn summary(&self) -> Option<String> {
        let t = self.tally;
        if t == EventTally::default() && self.baseline != Some(false) {
            return None;
        }
        let mut line = format!(
            "window events: {} sent, {} applied without the client's word, {} window op(s) lost",
            t.sent, t.forced, t.ops_lost
        );
        if t.unfit_returned > 0 || t.unfit_clamped > 0 {
            line.push_str(&format!(
                "; {} unfit granted unit(s) given back as ground items, {} claimed beyond the grant",
                t.unfit_returned, t.unfit_clamped
            ));
        }
        if self.baseline == Some(false) {
            line.push_str("; the window differed from the first op after join (baseline)");
        }
        Some(line)
    }
}

/// The four parts of a window an event can change.
struct Parts<'a> {
    inv: &'a mut Inventory,
    armour: &'a mut [Option<ArmourItem>; 4],
    cursor: &'a mut Option<ItemStack>,
    grid: &'a mut CraftGrid,
}

/// What applying one event did, for the tallies.
enum Effect {
    /// A grant: how many units the window held.
    Granted { landed: u8 },
    /// A take: how many it paid.
    Took { taken: u8 },
    /// Armour wore.
    Wore,
    /// A weapon's wear.
    Weapon(crate::joiner_inventory::WearCheck),
    /// Slots set.
    Set,
}

/// Apply `event` to `parts` by the client's own rule.
fn apply_to(parts: &mut Parts, event: &WindowEvent) -> Effect {
    match event {
        WindowEvent::Grant(stack) => {
            let rest = parts.inv.add_item(stack.clone()).map_or(0, |r| r.count);
            Effect::Granted { landed: stack.count.saturating_sub(rest) }
        }
        WindowEvent::Take { slot, item, n, .. } => Effect::Took {
            taken: crate::joiner_actions::take_owed_window(parts.inv, parts.grid, parts.cursor, *slot, item, *n),
        },
        WindowEvent::WearArmour { hits } => {
            for _ in 0..*hits {
                crate::window::wear_armour(parts.armour);
            }
            Effect::Wore
        }
        WindowEvent::WearWeapon { slot, tool } => {
            let check = match crate::joiner_actions::where_now(parts.inv, *slot, &Item::Tool(*tool)) {
                Some(at) => crate::joiner_inventory::wear_tool(parts.inv, at, tool),
                None => crate::joiner_inventory::WearCheck::Mismatched,
            };
            Effect::Weapon(check)
        }
        WindowEvent::SetSlots(sets) => {
            for (at, stack) in sets {
                match *at {
                    WireWindowSlot::Inv(i) if usize::from(i) < crate::window::SLOTS => {
                        parts.inv.set_slot(usize::from(i), stack.clone());
                    }
                    WireWindowSlot::Armour(i) if i < 4 => match stack {
                        None => parts.armour[usize::from(i)] = None,
                        Some(ItemStack { item: Item::Armour(piece), .. }) => parts.armour[usize::from(i)] = Some(*piece),
                        Some(_) => {}
                    },
                    WireWindowSlot::Cursor => *parts.cursor = stack.clone(),
                    WireWindowSlot::Grid(r, c) if r < 3 && c < 3 => {
                        parts.grid[usize::from(r)][usize::from(c)] = stack.clone();
                    }
                    _ => {}
                }
            }
            Effect::Set
        }
    }
}

/// Number `event` for joiner `sp` and queue it, sent on server tick `now`;
/// returns the number its carrier packet carries. At
/// [`MAX_PENDING_EVENTS`] the oldest waiting event is applied first.
pub fn queue(sp: &mut ServerPlayer, event: WindowEvent, now: u64) -> u32 {
    while sp.window_events.waiting.len() >= MAX_PENDING_EVENTS {
        apply_front(sp, now, true);
    }
    let ev = &mut sp.window_events;
    ev.last_sent = ev.last_sent.wrapping_add(1);
    ev.tally.sent = ev.tally.sent.saturating_add(1);
    ev.waiting.push_back(Waiting { seq: ev.last_sent, sent: now, event });
    ev.last_sent
}

/// Apply every event joiner `sp`'s client says it has applied (`acked`:
/// a packet's `events_applied`), oldest first, before that packet is
/// processed. A number past the last sent applies all of them.
pub fn apply_through(sp: &mut ServerPlayer, acked: u32, now: u64) {
    while sp.window_events.waiting.front().is_some_and(|w| w.seq <= acked) {
        apply_front(sp, now, false);
    }
}

/// The safety valve: apply joiner `sp`'s events still waiting
/// [`EVENT_ACK_TIMEOUT_TICKS`] after they were sent (tallied `forced`).
pub fn apply_overdue(sp: &mut ServerPlayer, now: u64) {
    while sp
        .window_events
        .waiting
        .front()
        .is_some_and(|w| now.saturating_sub(w.sent) >= EVENT_ACK_TIMEOUT_TICKS)
    {
        apply_front(sp, now, true);
    }
}

/// Apply the oldest waiting event to `sp`'s window and tally it.
fn apply_front(sp: &mut ServerPlayer, now: u64, forced: bool) {
    let Some(w) = sp.window_events.waiting.pop_front() else { return };
    if forced {
        sp.window_events.tally.forced = sp.window_events.tally.forced.saturating_add(1);
    }
    let mut parts =
        Parts { inv: &mut sp.inventory, armour: &mut sp.armour, cursor: &mut sp.cursor, grid: &mut sp.craft_grid };
    match (apply_to(&mut parts, &w.event), w.event) {
        (Effect::Granted { landed }, WindowEvent::Grant(stack)) => {
            let overflow = stack.count.saturating_sub(landed);
            sp.possession.grant_overflow = sp.possession.grant_overflow.saturating_add(u32::from(overflow));
            let grants = &mut sp.window_events.grants;
            if grants.len() >= RECENT_GRANTS {
                grants.pop_front();
            }
            grants.push_back(AppliedGrant { seq: w.seq, stack, landed, returned: false });
        }
        (Effect::Took { taken }, WindowEvent::Take { item, n, how, .. }) if taken < n => {
            let due = sp.possession.note_mismatch(now);
            log::log!(
                crate::joiner_inventory::mismatch_log_level(due),
                "possession check (log-only): {} used {n} × {item:?} {how}; the server's copy \
                 of their inventory held {taken}{} — accepted",
                sp.display_name,
                crate::hosted_server::held_back_note(due),
            );
        }
        (Effect::Weapon(check), _) => sp.possession.note_wear(check),
        _ => {}
    }
}

/// Joiner `sp`'s 36 slots as they will be once every waiting event is
/// applied: what a decision made now about what the joiner holds must read
/// (C3d's refusals; today the pickup fit during a `GrantUnfit` hold). Changes
/// nothing and tallies nothing.
pub fn effective_inventory(sp: &ServerPlayer) -> Inventory {
    let mut inv = sp.inventory.clone();
    if sp.window_events.waiting.is_empty() {
        return inv;
    }
    let (mut armour, mut cursor, mut grid) = (sp.armour, sp.cursor.clone(), sp.craft_grid.clone());
    let mut parts = Parts { inv: &mut inv, armour: &mut armour, cursor: &mut cursor, grid: &mut grid };
    for w in &sp.window_events.waiting {
        apply_to(&mut parts, &w.event);
    }
    inv
}

/// D-M2 — joiner `sp`'s client says `count` of the stack granted by window
/// event `event` didn't fit. Clamped to what that grant gave (one report per
/// grant; an event it can't name gives nothing, all tallied
/// `unfit_clamped`). The server's copy gives back what it holds of the grant
/// beyond what the client kept — none in lockstep, where its own `add_item`
/// left the same part out — and the overflow the client confirmed stops
/// counting as `grant_overflow`. Returns the stack to spawn as a real ground
/// item at the joiner's feet, and starts the pickup hold
/// ([`UNFIT_HOLD_TICKS`]).
pub fn return_unfit(sp: &mut ServerPlayer, event: u32, count: u8, now: u64) -> Option<ItemStack> {
    let ev = &mut sp.window_events;
    let Some(g) = ev.grants.iter_mut().find(|g| g.seq == event && !g.returned) else {
        ev.tally.unfit_clamped = ev.tally.unfit_clamped.saturating_add(u32::from(count));
        return None;
    };
    g.returned = true;
    let n = count.min(g.stack.count);
    ev.tally.unfit_clamped = ev.tally.unfit_clamped.saturating_add(u32::from(count - n));
    if n == 0 {
        return None;
    }
    ev.tally.unfit_returned = ev.tally.unfit_returned.saturating_add(u32::from(n));
    ev.unfit_hold_until = now + UNFIT_HOLD_TICKS;
    let kept = g.stack.count - n;
    let give_back = g.landed.saturating_sub(kept);
    let overflow = g.stack.count - g.landed;
    let item = g.stack.item.clone();
    sp.possession.grant_overflow = sp.possession.grant_overflow.saturating_sub(u32::from(overflow.min(n)));
    if give_back > 0 {
        crate::joiner_actions::take_owed_window(
            &mut sp.inventory,
            &mut sp.craft_grid,
            &mut sp.cursor,
            0,
            &item,
            give_back,
        );
    }
    Some(ItemStack { item, count: n })
}

// ─── Client: the carriers, applied in arrival order ───────────────────

/// One carrier of a window event, as the client applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum InboxItem {
    /// An `InteractOutcome` or `ItemActionOutcome` (its take or swing wear,
    /// if `window_event` is set); C3b-1 — or a `ContainerOpened` (never an
    /// event), or a `WindowSlotSet` (an event when it sets player slots).
    Outcome(RequestOutcome),
    /// An `InventoryGrant`.
    Grant(InventoryGrantPacket),
    /// `ArmourWorn { hits }`, window event `event`.
    ArmourWorn { hits: u8, event: u32 },
}

impl InboxItem {
    /// The window event this item is, 0 for none.
    pub fn event(&self) -> u32 {
        match self {
            InboxItem::Outcome(RequestOutcome::Interact(o)) => o.window_event,
            InboxItem::Outcome(RequestOutcome::Item(o)) => o.window_event,
            // C3b-1 — a set of player slots is an event; opening a mirror
            // and a set of container slots only are not.
            InboxItem::Outcome(RequestOutcome::SlotSet(set)) => set.window_event,
            InboxItem::Outcome(RequestOutcome::ContainerOpened(_)) => 0,
            InboxItem::Grant(g) => g.window_event,
            InboxItem::ArmourWorn { event, .. } => *event,
        }
    }
}

/// A joined client's window-event carriers not applied yet, and the
/// `StateUpdate` acknowledgement that waits with them (its request claims
/// must not end before their outcomes are applied, joiner_actions N4).
/// `RemoteClient` decodes them into one queue per kind, each in arrival
/// order; [`Self::take_ordered`] puts them back into the one order they
/// arrived in by their event numbers (the server numbers and sends them in
/// one order, on one ordered stream).
#[derive(Debug, Default)]
pub struct WindowInbox {
    outcomes: Vec<RequestOutcome>,
    grants: Vec<InventoryGrantPacket>,
    armour: Vec<(u8, u32)>,
    ack: Option<u64>,
}

impl WindowInbox {
    /// Add one poll's carriers, each list in arrival order.
    pub fn add(&mut self, outcomes: Vec<RequestOutcome>, grants: Vec<InventoryGrantPacket>, armour: Vec<(u8, u32)>) {
        self.outcomes.extend(outcomes);
        self.grants.extend(grants);
        self.armour.extend(armour);
    }

    /// A `StateUpdate` acknowledged every input up to `acked`.
    pub fn acknowledged(&mut self, acked: u64) {
        self.ack = Some(self.ack.map_or(acked, |a| a.max(acked)));
    }

    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty() && self.grants.is_empty() && self.armour.is_empty() && self.ack.is_none()
    }

    /// Everything waiting, in the order it arrived: an item with an event
    /// number goes before every item with a higher one; an outcome that is
    /// no event keeps its place among the outcomes (their order is the
    /// requests', `JoinerActions::take`) — where it falls against grants and
    /// wear no number says, and it changes no window. Then the
    /// acknowledgement.
    pub fn take_ordered(&mut self) -> (Vec<InboxItem>, Option<u64>) {
        let mut outcomes: VecDeque<InboxItem> = self.outcomes.drain(..).map(InboxItem::Outcome).collect();
        let mut grants: VecDeque<InboxItem> = self.grants.drain(..).map(InboxItem::Grant).collect();
        let mut armour: VecDeque<InboxItem> =
            self.armour.drain(..).map(|(hits, event)| InboxItem::ArmourWorn { hits, event }).collect();
        let mut out = Vec::with_capacity(outcomes.len() + grants.len() + armour.len());
        loop {
            let pick = [&outcomes, &grants, &armour]
                .iter()
                .enumerate()
                .filter_map(|(k, q)| q.front().map(|item| (item.event(), k)))
                .min();
            let Some((_, k)) = pick else { break };
            let next = match k {
                0 => outcomes.pop_front(),
                1 => grants.pop_front(),
                _ => armour.pop_front(),
            };
            out.extend(next);
        }
        (out, self.ack.take())
    }

    /// Forget everything (not joined any more).
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armour::{ArmourMaterial, ArmourSlot};
    use crate::crafting::{ToolMaterial, ToolType};
    use crate::item::MaterialId;
    use crate::protocol::{InteractOutcomePacket, ItemActionOutcomePacket, WireItem};

    fn joiner() -> ServerPlayer {
        let mut sp = ServerPlayer::new(glam::Vec3::new(0.0, 70.0, 0.0));
        sp.server_simulated = true;
        sp
    }

    fn bread(n: u8) -> ItemStack {
        ItemStack::new_material(MaterialId::Bread, n)
    }

    #[test]
    fn an_event_waits_until_the_client_says_it_applied_it() {
        let mut sp = joiner();
        let a = queue(&mut sp, WindowEvent::Grant(bread(2)), 10);
        let b = queue(&mut sp, WindowEvent::Grant(ItemStack::new_material(MaterialId::Stick, 1)), 10);
        assert_eq!((a, b), (1, 2), "numbered 1, 2… per connection");
        assert_eq!(sp.window_events.last_sent(), 2);
        assert!(sp.inventory.slot(0).is_none(), "nothing applied on sending");
        apply_through(&mut sp, 0, 11);
        assert_eq!(sp.window_events.waiting(), 2);
        apply_through(&mut sp, 1, 11);
        assert_eq!(sp.inventory.slot(0), Some(&bread(2)), "the first, in order");
        assert_eq!(sp.window_events.waiting(), 1);
        apply_through(&mut sp, 99, 11);
        assert_eq!(sp.window_events.waiting(), 0, "a number past the last applies them all");
        assert_eq!(sp.window_events.tally, EventTally { sent: 2, ..Default::default() });
    }

    #[test]
    fn the_effective_window_has_the_waiting_events_applied() {
        let mut sp = joiner();
        sp.inventory.set_slot(3, Some(bread(1)));
        queue(&mut sp, WindowEvent::Take { slot: 3, item: Item::Material(MaterialId::Bread), n: 1, how: "by eating" }, 0);
        queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::STONE, 5)), 0);
        let eff = effective_inventory(&sp);
        assert!(eff.slot(3).is_none(), "the take, then");
        assert_eq!(eff.slot(0), Some(&ItemStack::new_block(crate::block::STONE, 5)), "the grant");
        assert_eq!(sp.inventory.slot(3), Some(&bread(1)), "the real window is untouched");
        assert_eq!(sp.possession, Default::default(), "nothing tallied");
    }

    #[test]
    fn an_unacknowledged_event_is_forced_after_the_timeout_and_the_bound() {
        let mut sp = joiner();
        queue(&mut sp, WindowEvent::Grant(bread(1)), 100);
        apply_overdue(&mut sp, 100 + EVENT_ACK_TIMEOUT_TICKS - 1);
        assert_eq!(sp.window_events.waiting(), 1, "not yet");
        apply_overdue(&mut sp, 100 + EVENT_ACK_TIMEOUT_TICKS);
        assert_eq!(sp.window_events.waiting(), 0);
        assert_eq!(sp.window_events.tally.forced, 1);
        for _ in 0..MAX_PENDING_EVENTS + 3 {
            queue(&mut sp, WindowEvent::WearArmour { hits: 1 }, 500);
        }
        assert_eq!(sp.window_events.waiting(), MAX_PENDING_EVENTS, "bounded");
        assert_eq!(sp.window_events.tally.forced, 4, "the oldest three applied, and the first");
    }

    #[test]
    fn a_take_short_of_what_it_owed_is_a_logged_mismatch() {
        let mut sp = joiner();
        sp.inventory.set_slot(0, Some(bread(1)));
        queue(&mut sp, WindowEvent::Take { slot: 0, item: Item::Material(MaterialId::Bread), n: 2, how: "by eating" }, 0);
        apply_through(&mut sp, 1, 0);
        assert!(sp.inventory.slot(0).is_none());
        assert_eq!(sp.possession.mismatched, 1);
    }

    #[test]
    fn weapon_and_armour_wear_are_events_too() {
        let mut sp = joiner();
        let sword = Tool::new(ToolType::Sword, ToolMaterial::Iron);
        sp.inventory.set_slot(4, Some(ItemStack::new_tool(sword)));
        let helmet = ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron);
        sp.armour[0] = Some(helmet);
        queue(&mut sp, WindowEvent::WearWeapon { slot: 2, tool: sword }, 0);
        queue(&mut sp, WindowEvent::WearArmour { hits: 2 }, 0);
        queue(&mut sp, WindowEvent::WearWeapon { slot: 0, tool: Tool::new(ToolType::Axe, ToolMaterial::Wood) }, 0);
        apply_through(&mut sp, 3, 0);
        let worn = match sp.inventory.slot(4).map(|s| &s.item) {
            Some(Item::Tool(t)) => t.durability,
            _ => unreachable!(),
        };
        assert_eq!(worn, sword.durability - 1, "found where it now is");
        assert_eq!(sp.armour[0].unwrap().durability, helmet.durability - 2);
        assert_eq!(sp.possession.wear_mismatch, 1, "an axe it doesn't hold");
    }

    #[test]
    fn grant_unfit_is_clamped_to_its_grant_and_gives_back_only_what_the_window_holds_beyond_the_client() {
        // Lockstep: the server's window is as full as the client's, so its
        // own add_item left the same 4 out — nothing to take back.
        let mut sp = joiner();
        for i in 0..36 {
            sp.inventory.set_slot(i, Some(ItemStack::new_block(crate::block::STONE, 64)));
        }
        sp.inventory.set_slot(7, Some(bread(61)));
        let seq = queue(&mut sp, WindowEvent::Grant(bread(7)), 0);
        apply_through(&mut sp, seq, 0);
        assert_eq!(sp.possession.grant_overflow, 4, "4 didn't fit the server's copy either (61 + 3 = 64)");
        let back = return_unfit(&mut sp, seq, 4, 50).expect("given back");
        assert_eq!(back, bread(4));
        assert_eq!(sp.inventory.slot(7), Some(&bread(64)), "nothing taken back: it held only what the client kept");
        assert_eq!(sp.possession.grant_overflow, 0, "the client confirmed that overflow");
        assert!(!sp.window_events.grants_whole(50 + UNFIT_HOLD_TICKS - 1), "the pickup hold");
        assert!(sp.window_events.grants_whole(50 + UNFIT_HOLD_TICKS));
        // A second report for the same grant, or one beyond it, gives nothing.
        assert_eq!(return_unfit(&mut sp, seq, 1, 51), None);
        let seq2 = queue(&mut sp, WindowEvent::Grant(bread(2)), 0);
        apply_through(&mut sp, seq2, 0);
        assert_eq!(return_unfit(&mut sp, seq2, 200, 52), Some(bread(2)), "clamped to the grant");
        assert_eq!(return_unfit(&mut sp, 777, 5, 52), None, "a grant it can't name");
        assert_eq!(sp.window_events.tally.unfit_returned, 6);
        assert_eq!(sp.window_events.tally.unfit_clamped, 1 + 198 + 5);
    }

    #[test]
    fn grant_unfit_takes_back_what_a_roomier_server_copy_landed() {
        // Drift: the server's copy had room, the client didn't.
        let mut sp = joiner();
        let seq = queue(&mut sp, WindowEvent::Grant(bread(4)), 0);
        apply_through(&mut sp, seq, 0);
        assert_eq!(sp.inventory.slot(0), Some(&bread(4)));
        assert_eq!(return_unfit(&mut sp, seq, 4, 0), Some(bread(4)));
        assert!(sp.inventory.slot(0).is_none(), "the server's copy follows the client's");
    }

    #[test]
    fn op_seq_gaps_are_tallied_as_lost_ops() {
        let mut ev = WindowEvents::default();
        for seq in [1, 2, 3, 6, 7, 5, 9] {
            ev.note_op_seq(seq);
        }
        assert_eq!(ev.tally.ops_lost, 2 + 1, "4 and 5 missing at 6; 8 at 9; the late 5 moves nothing");
    }

    fn interact(seq: u32, window_event: u32) -> RequestOutcome {
        RequestOutcome::Interact(InteractOutcomePacket {
            seq,
            entity: 1,
            kind: None,
            accepted: true,
            consume_held: 0,
            note: 0,
            window_event,
        })
    }

    fn eat(seq: u32, window_event: u32) -> RequestOutcome {
        RequestOutcome::Item(ItemActionOutcomePacket { seq, accepted: true, consume_held: 1, note: 0, window_event })
    }

    fn grant(window_event: u32) -> InventoryGrantPacket {
        InventoryGrantPacket {
            item_kind: crate::protocol::item_kind::MATERIAL,
            item_id: 1,
            count: 1,
            full_item: WireItem::None,
            window_event,
        }
    }

    #[test]
    fn the_inbox_gives_its_carriers_back_in_arrival_order() {
        let mut inbox = WindowInbox::default();
        // On the wire: grant 1, eat (2), armour 3, a refused swing (0), the
        // milk take (4), its milk (5), armour 6.
        inbox.add(vec![eat(10, 2), interact(11, 0), interact(12, 4)], vec![grant(1), grant(5)], vec![(1, 3), (2, 6)]);
        inbox.acknowledged(7);
        inbox.acknowledged(5);
        let (items, ack) = inbox.take_ordered();
        let order: Vec<u32> = items.iter().map(InboxItem::event).collect();
        // The refused swing (no event) keeps its place among the outcomes;
        // where it falls against the other kinds no number says, and it
        // changes no window.
        assert_eq!(order, vec![1, 2, 0, 3, 4, 5, 6]);
        assert_eq!(items[2], InboxItem::Outcome(interact(11, 0)), "the outcomes keep their own order");
        assert_eq!(ack, Some(7), "the latest acknowledgement");
        assert!(inbox.is_empty());
        // Leaving the world drops whatever waits (`world_exit`).
        inbox.add(vec![eat(1, 1)], vec![grant(2)], vec![(1, 3)]);
        inbox.acknowledged(9);
        inbox.clear();
        assert!(inbox.is_empty());
        assert_eq!(inbox.take_ordered(), (Vec::new(), None));
    }
}
