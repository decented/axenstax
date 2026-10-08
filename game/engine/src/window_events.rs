//! C3a-fix-1 (protocol v76) — the server's changes to a joiner's window are
//! applied in the order the CLIENT applied them
//! (`docs/foundations/2026-10-07-c3-server-owned-inventory.md` §2; Spec 04
//! §4.2g).
//!
//! The server changes a joiner's window by itself four ways: a grant (a
//! break's yield, an interaction's product, a pickup; C3b-2, a block use's
//! gain), the owed take of an accepted request (an eat, a D2b interaction;
//! C3b-2, a block use's take), an armour-wear hit and an accepted swing's
//! weapon wear (C3b-2: and shears' wear on a hive, `WearWeapon` on an
//! `ItemActionOutcome` with `wear_held`). C3b-1 (v77) added a fifth, a container op's
//! correction. C3b-fix-a (v78) makes it a change by item, never by slot
//! ([`WindowEvent::Correction`]: take N of X, give N of X, resolved where
//! the client applies it), and numbers every container VIEW the server sends
//! too ([`WindowEvent::ContainerView`]: an opened container and every push;
//! a correction's container slots ride with it): such an event changes no
//! window on the server, it only advances the count, so the server knows
//! exactly which view a container op was predicted on (C-M1;
//! `window_ops::ContainerViews`). The client applies each when its packet
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
use crate::protocol::InventoryGrantPacket;
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
#[derive(Clone, Debug)]
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
    /// C3b-fix-a (v78, C-M1) — a container view sent to the joiner (an
    /// opened container, or a push of what others changed): it changes the
    /// server's model of the client's mirror (`window_ops::ContainerViews`)
    /// when applied, and no window.
    ContainerView(crate::window_ops::ViewEvent),
    /// C3b-fix-a (v78, C-H1) — a container op's correction
    /// (`WindowSlotSet { Correction }`): the server's copy applies its own
    /// delta (R − own; nothing for a refused op), by item, with the same
    /// debt rule the client resolves its delta (R − P) by
    /// (`container_window::CorrectionDebt`); its container slots go to the
    /// model of the client's mirror. While it waits, its client delta is the
    /// phantom ledger ([`phantom`]).
    Correction(Box<crate::window_ops::Correction>),
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
    /// C3b-fix-b (B-M2) — units a `GrantUnfit` claimed within its grant that
    /// the server could not back with anything: neither overflow the grant
    /// never landed nor units its copy of the window still held. Never
    /// spawned.
    pub unfit_unbacked: u32,
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
    /// C3b-fix-a — what the server's copy owes from correction takes it
    /// couldn't pay (the client keeps the same ledger,
    /// [`WindowInbox::debt`]).
    pub debt: crate::container_window::CorrectionDebt,
    /// C3b-fix-c (B-L6) — corrections the valve applied without the
    /// client's word (`forced`), each with its number and its client delta:
    /// still part of the phantom ledger ([`phantom`]) until the client
    /// reports it applied them ([`apply_through`]). At most
    /// [`MAX_PENDING_EVENTS`] (the oldest go first).
    forced_corrections: VecDeque<(u32, crate::container_window::ItemDelta)>,
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
        if t.unfit_returned > 0 || t.unfit_clamped > 0 || t.unfit_unbacked > 0 {
            line.push_str(&format!(
                "; {} unfit granted unit(s) given back as ground items, {} claimed beyond the grant, \
                 {} not backed by anything the server held",
                t.unfit_returned, t.unfit_clamped, t.unfit_unbacked
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
    /// A correction: what the server's own delta did (`None`: a refused
    /// op's, which changes nothing here).
    Corrected(Option<crate::container_window::DebtApplied>),
    /// A container view: no window changed.
    Viewed,
    /// A grant: how many units the window held.
    Granted { landed: u8 },
    /// A take: how many it paid.
    Took { taken: u8 },
    /// Armour wore.
    Wore,
    /// A weapon's wear.
    Weapon(crate::joiner_inventory::WearCheck),
}

/// Apply `event` to `parts` (with the copy's correction `debt`) by the
/// client's own rule.
fn apply_to(parts: &mut Parts, debt: &mut crate::container_window::CorrectionDebt, event: &WindowEvent) -> Effect {
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
        WindowEvent::ContainerView(_) => Effect::Viewed,
        WindowEvent::Correction(c) => {
            Effect::Corrected(c.own.as_ref().map(|own| debt.apply(parts.inv, parts.grid, parts.cursor, parts.armour, own)))
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
    // C3b-fix-c (B-L6) — a forced correction the client has now applied
    // leaves the phantom ledger.
    sp.window_events.forced_corrections.retain(|(seq, _)| *seq > acked);
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
    match (apply_to(&mut parts, &mut sp.window_events.debt, &w.event), w.event) {
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
        (Effect::Viewed, WindowEvent::ContainerView(view)) => view.apply(&mut sp.container_sent.seen),
        (Effect::Corrected(applied), WindowEvent::Correction(c)) => {
            c.view.apply(&mut sp.container_sent.seen);
            if let Some(a) = applied.as_ref() {
                note_correction_short(sp, &a.short, now);
            }
            note_correction_gives(sp, w.seq, &c, applied.map(|a| a.settled));
            if forced {
                let ledger = &mut sp.window_events.forced_corrections;
                if ledger.len() >= MAX_PENDING_EVENTS {
                    ledger.pop_front();
                }
                ledger.push_back((w.seq, c.client));
            }
        }
        _ => {}
    }
}

/// C3b-fix-c (B-M1, decision 6) — a container op's correction took short
/// from joiner `sp`'s server copy (`short`, item by item: owed now): the
/// phantom went some other way — placed, dropped, eaten, crafted — before
/// the correction landed. Tallied `correction_short` and logged like an owed
/// take's shortfall (rate-limited). Log-only: it conserves nothing until C3d
/// judges every spend against the effective window.
pub fn note_correction_short(sp: &mut ServerPlayer, short: &[(Item, u32)], now: u64) {
    for (item, n) in short {
        sp.possession.correction_short = sp.possession.correction_short.saturating_add(*n);
        let due = sp.possession.note_mismatch(now);
        log::log!(
            crate::joiner_inventory::mismatch_log_level(due),
            "possession check (log-only): a container correction took {n} × {item:?} short from {}'s window \
             (spent another way before it landed) — owed{}",
            sp.display_name,
            crate::hosted_server::held_back_note(due),
        );
    }
}

/// C3b-fix-c — apply a container op's own delta (R − own) to joiner `sp`'s
/// server copy now, with the copy's correction debt (`window_ops`: when
/// nothing reaches the client, its window already is its prediction), and
/// tally a take that fell short ([`note_correction_short`]).
pub fn apply_own_now(sp: &mut ServerPlayer, own: &crate::container_window::ItemDelta, now: u64) {
    let applied = sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, own);
    note_correction_short(sp, &applied.short, now);
}

/// A correction applied to the server's copy: each give of the CLIENT's
/// delta is remembered as a grant (one entry per give, in order), so the
/// client's `GrantUnfit` for this event names it — one report per give, in
/// order, a give that fit reported as 0 when a later one didn't (they match
/// [`return_unfit`]'s first unreturned grant of the event). The server's own
/// gives that didn't fit count as `grant_overflow`, as a grant's do.
///
/// C3b-fix-c (B-M2, decision 4) — what a client give "landed" comes from the
/// server's own unfit: per item, `server_unfit` = Σ (own give − settled),
/// spent over the client's gives of that item in order, so a give records
/// `landed = count − min(server_unfit_left, count)`. Every other unit of an
/// accepted op's give is one the server's copy holds (R moved it out of the
/// real container into the copy, or the copy never gave it up), so a
/// `GrantUnfit` for it is backed only by what `return_unfit` can take back
/// from the copy, and only the server's true overflow is spawned without a
/// take. A refused op's revert gives back what the server's copy never gave
/// up: `landed = count`.
fn note_correction_gives(sp: &mut ServerPlayer, seq: u32, c: &crate::window_ops::Correction, settled: Option<Vec<u8>>) {
    let mut server_unfit: Vec<(Item, u32)> = Vec::new();
    if let (Some(own), Some(settled)) = (c.own.as_ref(), settled.as_ref()) {
        for (g, s) in own.give.iter().zip(settled) {
            let overflow = g.count.saturating_sub(*s);
            sp.possession.grant_overflow = sp.possession.grant_overflow.saturating_add(u32::from(overflow));
            match server_unfit.iter_mut().find(|(i, _)| i == &g.item) {
                Some((_, n)) => *n += u32::from(overflow),
                None => server_unfit.push((g.item.clone(), u32::from(overflow))),
            }
        }
    }
    let grants = &mut sp.window_events.grants;
    for g in &c.client.give {
        let unfit = match c.own {
            None => 0,
            Some(_) => match server_unfit.iter_mut().find(|(i, _)| i == &g.item) {
                Some((_, left)) => {
                    let u = (*left).min(u32::from(g.count)) as u8;
                    *left -= u32::from(u);
                    u
                }
                None => 0,
            },
        };
        if grants.len() >= RECENT_GRANTS {
            grants.pop_front();
        }
        grants.push_back(AppliedGrant { seq, stack: g.clone(), landed: g.count - unfit, returned: false });
    }
}

/// C3b-fix-a (C-H1) — the phantom ledger: what joiner `sp`'s corrections
/// still on their way will take back from its client, item by item (their
/// client deltas' takes, net of their gives). A later container op's claims
/// still hold those units — items the server already refused — so the
/// server debits them before re-running the op (`window_ops::serve_op`):
/// they are never believed again. Read after `apply_through`, so every
/// correction still waiting is one the op was made before. C3b-fix-c (B-L6)
/// — so is every correction the valve forced that the client hasn't
/// reported applying yet (`WindowEvents::forced_corrections`): its client
/// still holds the phantom.
pub fn phantom(sp: &ServerPlayer) -> Vec<(Item, u32)> {
    let mut net: Vec<(Item, i64)> = Vec::new();
    let waiting = sp.window_events.waiting.iter().filter_map(|w| match &w.event {
        WindowEvent::Correction(c) => Some(&c.client),
        _ => None,
    });
    for client in sp.window_events.forced_corrections.iter().map(|(_, d)| d).chain(waiting) {
        for (item, n) in client.counts() {
            match net.iter_mut().find(|(i, _)| i == &item) {
                Some((_, m)) => *m += n,
                None => net.push((item, n)),
            }
        }
    }
    net.into_iter().filter(|(_, n)| *n < 0).map(|(item, n)| (item, n.unsigned_abs() as u32)).collect()
}

/// C3b-fix-a (C-M1) — the container views waiting for joiner `sp`'s
/// client's word, oldest first (an opened container, a push, a correction's
/// container slots): with the model of its mirror
/// (`window_ops::ContainerViews::seen`), what its mirror will show once they
/// land.
pub fn waiting_views(sp: &ServerPlayer) -> impl Iterator<Item = &crate::window_ops::ViewEvent> {
    sp.window_events.waiting.iter().filter_map(|w| match &w.event {
        WindowEvent::ContainerView(v) => Some(v),
        WindowEvent::Correction(c) => Some(&c.view),
        _ => None,
    })
}

/// Joiner `sp`'s 36 slots as they will be once every waiting event is
/// applied: what a decision made now about what the joiner holds must read
/// (C3d's refusals; today the pickup fit during a `GrantUnfit` hold). Changes
/// nothing and tallies nothing.
pub fn effective_inventory(sp: &ServerPlayer) -> Inventory {
    effective_window(sp).inv
}

/// C3b-fix-c — joiner `sp`'s whole window (36 slots, armour, cursor, grid)
/// as it will be once every waiting event is applied ([`effective_inventory`]
/// for all four parts): what the believed bound counts
/// (`window_ops::believed_units`). Changes nothing and tallies nothing.
pub fn effective_window(sp: &ServerPlayer) -> EffectiveWindow {
    let mut w = EffectiveWindow {
        inv: sp.inventory.clone(),
        armour: sp.armour,
        cursor: sp.cursor.clone(),
        grid: sp.craft_grid.clone(),
    };
    if sp.window_events.waiting.is_empty() {
        return w;
    }
    let mut debt = sp.window_events.debt.clone();
    let mut parts = Parts { inv: &mut w.inv, armour: &mut w.armour, cursor: &mut w.cursor, grid: &mut w.grid };
    for ev in &sp.window_events.waiting {
        apply_to(&mut parts, &mut debt, &ev.event);
    }
    w
}

/// A copy of a joiner's window with its waiting events applied
/// ([`effective_window`]).
#[derive(Clone)]
pub struct EffectiveWindow {
    pub inv: Inventory,
    pub armour: [Option<ArmourItem>; 4],
    pub cursor: Option<ItemStack>,
    pub grid: CraftGrid,
}

/// D-M2 — joiner `sp`'s client says `count` of the stack granted by window
/// event `event` didn't fit. Clamped to what that grant gave (one report per
/// grant; an event it can't name gives nothing, all tallied
/// `unfit_clamped`). The server's copy gives back what it holds of the grant
/// beyond what the client kept — none in lockstep, where its own `add_item`
/// left the same part out — and the overflow the client confirmed stops
/// counting as `grant_overflow`.
///
/// C3b-fix-b (B-M2) — what is returned as a ground item is only what came
/// back: the grant's overflow (the part its `add_item` never landed: the
/// pickup removed that item from the world and nothing else holds it) plus
/// what the server's copy gave up (`take_owed_window` took it). A claim the
/// server cannot back with either (the units moved on, say deposited in a
/// chest) spawns nothing for the gap, tallied `unfit_unbacked`: the report
/// cannot make items. Returns the stack to spawn at the joiner's feet
/// ([`spawn_unfit`]) and starts the pickup hold ([`UNFIT_HOLD_TICKS`]).
///
/// C3b-fix-c (A-L4) — the take back is the one owed search, an exact match
/// first (`joiner_actions::take_owed_search`), and the stack spawned is the
/// instance it took (a worn tool comes back worn), never the grant's own —
/// except the overflow, which never landed and is the grant's.
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
    let kept = g.stack.count - n;
    let give_back = g.landed.saturating_sub(kept);
    let overflow = g.stack.count - g.landed;
    let item = g.stack.item.clone();
    sp.possession.grant_overflow = sp.possession.grant_overflow.saturating_sub(u32::from(overflow.min(n)));
    // C3b-fix-c (A-L4) — the exact granted item first, by the one owed
    // search (no hint: the client names no slot), and what comes back is
    // what was taken.
    let taken = if give_back > 0 {
        crate::joiner_actions::take_owed_search(
            &mut sp.inventory,
            Some(&mut sp.craft_grid),
            Some(&mut sp.cursor),
            None,
            None,
            &item,
            give_back,
        )
    } else {
        Vec::new()
    };
    let taken_units = taken.iter().map(|s| s.count).fold(0u8, u8::saturating_add);
    let ev = &mut sp.window_events;
    let backed = n.min(overflow.saturating_add(taken_units));
    ev.tally.unfit_unbacked = ev.tally.unfit_unbacked.saturating_add(u32::from(n - backed));
    if backed == 0 {
        return None;
    }
    ev.tally.unfit_returned = ev.tally.unfit_returned.saturating_add(u32::from(backed));
    ev.unfit_hold_until = now + UNFIT_HOLD_TICKS;
    // One stack: the overflow is the grant's own instance (it never landed
    // anywhere), the rest the instances taken. They are one item: a tool or
    // armour piece stacks to one (overflow or taken, never both), and any
    // other item matched by kind is the same item.
    let item = taken.first().map_or(item, |s| s.item.clone());
    Some(ItemStack { item, count: backed })
}

/// C3b-fix-b (B-M2) — put the stack [`return_unfit`] gave back on the ground
/// at joiner `player_index`'s feet, thrown like its own Q-drop: it is the
/// dropper, so it can't take it again for `ITEM_DROP_PICKUP_DELAY_TICKS` (a
/// grant refused for want of room would otherwise come straight back, again
/// and again, wherever the server's copy has room the client's lacks), while
/// anyone else may take it at once.
pub fn spawn_unfit(ecs: &mut hecs::World, feet: glam::Vec3, stack: ItemStack, player_index: usize) {
    crate::entity::spawn_thrown_item(
        ecs,
        feet + glam::Vec3::new(0.0, 0.4, 0.0),
        glam::Vec3::new(0.0, 0.22, 0.0),
        stack,
        player_index.min(usize::from(u8::MAX)) as u8,
    );
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
            // C3b-fix-a (v78) — every container view is an event: an opened
            // mirror (0 for a refusal), a push, a correction.
            InboxItem::Outcome(RequestOutcome::SlotSet(set)) => set.window_event,
            InboxItem::Outcome(RequestOutcome::ContainerOpened(o)) => o.window_event,
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
    /// C3b-fix-a — what this client's window owes from correction takes it
    /// couldn't pay (`container_window::CorrectionDebt`; the server keeps
    /// the same ledger for its copy, [`WindowEvents::debt`]). Per session.
    pub debt: crate::container_window::CorrectionDebt,
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

    /// Nothing waits to be applied (the debt is not a carrier).
    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty() && self.grants.is_empty() && self.armour.is_empty() && self.ack.is_none()
    }

    /// Everything waiting, in the order it arrived: an item with an event
    /// number goes before every item with a higher one; an outcome that is
    /// no event keeps its place among the outcomes (their order is the
    /// requests', `JoinerActions::take`) — where it falls against grants and
    /// wear no number says, and it changes no window. Then the
    /// acknowledgement.
    ///
    /// MUST: an event-0 carrier never touches player slots (B-L3). It is
    /// merged by queue position only, so it can run ahead of a lower-numbered
    /// grant or armour item that arrived before it; that commutes today
    /// because a refusal, `ContainerOpened` and a container-only
    /// `WindowSlotSet` touch container state, never the inventory, armour,
    /// cursor or grid. A new unnumbered carrier that changes any of those
    /// must be numbered instead.
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

    /// B-M2 — the modified client's scenario: a grant lands and is acked, the
    /// units move on in the server's copy (a real move into a chest), then a
    /// `GrantUnfit` claims them all. Nothing came back, so nothing is
    /// spawned, and the gap is tallied.
    #[test]
    fn a_grant_unfit_for_units_that_moved_on_spawns_nothing() {
        let mut sp = joiner();
        let seq = queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::COBBLESTONE, 64)), 0);
        apply_through(&mut sp, seq, 0);
        assert_eq!(sp.inventory.slot(0).map(|s| s.count), Some(64), "landed");
        sp.inventory.set_slot(0, None);
        assert_eq!(return_unfit(&mut sp, seq, 64, 10), None, "no stack from nothing");
        let t = sp.window_events.tally;
        assert_eq!((t.unfit_returned, t.unfit_unbacked), (0, 64));
        assert!(sp.window_events.grants_whole(10), "and no pickup hold for a stack that was never thrown");
    }

    /// B-M2 — a claim is backed by the grant's overflow (never landed: the
    /// pickup took that item out of the world) plus whatever the server's
    /// copy still holds; the rest is the gap.
    #[test]
    fn a_grant_unfit_spawns_the_overflow_and_what_the_copy_gave_up_and_no_more() {
        let full = |sp: &mut ServerPlayer| {
            for i in 0..36 {
                sp.inventory.set_slot(i, Some(ItemStack::new_block(crate::block::STONE, 64)));
            }
            sp.inventory.set_slot(7, Some(bread(61)));
        };
        // Honest drift: 3 landed on the server's copy, 4 overflowed; the
        // client, full, kept none: 7 back.
        let mut sp = joiner();
        full(&mut sp);
        let seq = queue(&mut sp, WindowEvent::Grant(bread(7)), 0);
        apply_through(&mut sp, seq, 0);
        assert_eq!(return_unfit(&mut sp, seq, 7, 0), Some(bread(7)));
        assert_eq!(sp.inventory.slot(7), Some(&bread(61)), "the 3 that landed are taken back");
        assert_eq!(sp.window_events.tally.unfit_unbacked, 0);
        // Modified: the bread moved on first (the owed search is by item, so
        // it takes the grant's units wherever the copy still holds that item);
        // only the overflow is real.
        let mut sp = joiner();
        full(&mut sp);
        let seq = queue(&mut sp, WindowEvent::Grant(bread(7)), 0);
        apply_through(&mut sp, seq, 0);
        sp.inventory.set_slot(7, None);
        assert_eq!(return_unfit(&mut sp, seq, 7, 0), Some(bread(4)), "the overflow, no more");
        assert_eq!((sp.window_events.tally.unfit_returned, sp.window_events.tally.unfit_unbacked), (4, 3));
    }

    /// C3b-fix-c (A-L4) — a `GrantUnfit` for a granted tool takes the
    /// granted instance back (an exact match first) and spawns what it took:
    /// a worn pickaxe of the same kind is neither taken in its place nor
    /// swapped for a fresh one.
    #[test]
    fn a_tool_grant_unfit_spawns_the_instance_it_took() {
        let fresh = Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond);
        let worn = Tool { durability: fresh.durability - 100, ..fresh };
        let mut sp = joiner();
        sp.inventory.set_slot(0, Some(ItemStack::new_tool(worn)));
        let seq = queue(&mut sp, WindowEvent::Grant(ItemStack::new_tool(fresh)), 0);
        apply_through(&mut sp, seq, 0);
        assert_eq!(return_unfit(&mut sp, seq, 1, 0), Some(ItemStack::new_tool(fresh)));
        assert_eq!(sp.inventory.slot(0), Some(&ItemStack::new_tool(worn)), "its own worn pickaxe stays");
        // The granted one moved on; a worn one is all the copy holds: it is
        // what comes back, never a fresh one.
        let mut sp = joiner();
        let seq = queue(&mut sp, WindowEvent::Grant(ItemStack::new_tool(fresh)), 0);
        apply_through(&mut sp, seq, 0);
        sp.inventory.set_slot(0, Some(ItemStack::new_tool(worn)));
        assert_eq!(return_unfit(&mut sp, seq, 1, 0), Some(ItemStack::new_tool(worn)), "what it took");
        assert!(sp.inventory.slot(0).is_none());
    }

    /// C3b-fix-c (B-L6) — a correction the valve forced stays in the phantom
    /// ledger until the client reports it applied it.
    #[test]
    fn a_forced_correction_stays_in_the_phantom_ledger_until_the_client_applies_it() {
        let mut sp = joiner();
        let stone = ItemStack::new_block(crate::block::STONE, 16);
        let client = crate::container_window::ItemDelta { take: vec![(0, stone.clone())], give: vec![] };
        let c = crate::window_ops::Correction {
            client: client.clone(),
            own: Some(client),
            view: crate::window_ops::ViewEvent::Sets { sets: Vec::new(), furnace: None },
        };
        let seq = queue(&mut sp, WindowEvent::Correction(Box::new(c)), 0);
        assert_eq!(phantom(&sp), vec![(stone.item.clone(), 16)]);
        apply_overdue(&mut sp, EVENT_ACK_TIMEOUT_TICKS);
        assert_eq!(sp.window_events.tally.forced, 1);
        assert_eq!(phantom(&sp), vec![(stone.item.clone(), 16)], "forced, but the client hasn't applied it");
        apply_through(&mut sp, seq, EVENT_ACK_TIMEOUT_TICKS + 1);
        assert!(phantom(&sp).is_empty(), "applied by the client: gone");
    }

    /// B-M2 — the stack goes down thrown from the joiner: it is the dropper,
    /// so it waits out the Q-drop delay; anyone else may take it at once.
    #[test]
    fn the_unfit_stack_is_thrown_by_the_joiner_with_the_drop_delay() {
        let mut ecs = hecs::World::new();
        spawn_unfit(&mut ecs, glam::Vec3::new(1.0, 70.0, 1.0), bread(3), 2);
        let (dropper, delay, stack) = ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .map(|(_, i)| (i.dropper, i.pickup_delay, i.stack.clone()))
            .next()
            .unwrap();
        assert_eq!(dropper, Some(2));
        assert_eq!(delay, crate::entity::ITEM_DROP_PICKUP_DELAY_TICKS);
        assert_eq!(stack, bread(3));
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
        RequestOutcome::Item(ItemActionOutcomePacket { seq, accepted: true, consume_held: 1, note: 0, window_event, wear_held: false })
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
