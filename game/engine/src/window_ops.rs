//! C3a-2a (protocol v75) — the server mirrors a joiner's inventory window,
//! click for click (`docs/foundations/2026-10-07-c3-server-owned-inventory.md`
//! §2; Spec 04 §4.2g).
//!
//! **Client.** Every window transition the inventory screen applies — each
//! click, the close included (`CraftingUi::apply_click`, `CraftingUi::close`),
//! and each open — is logged in an [`OpLog`] with the window's digest after
//! it (`window::digest`). Its auto-refill setting is an op too, logged at
//! join and whenever it changes, ahead of the op it first applies to.
//! Single-player and a host's own slots send nothing.
//!
//! **One ordered send path (C3a-fix-1, B-L1).** A joined client sends its
//! ops, its edits and its requests in the order it made them. Each op and
//! the first edit of each input are stamped from one order clock
//! ([`order_stamp`]; the edits wait in [`PendingEdits`]). At the tick's send
//! the ops logged before the input's first edit go before the input and the
//! rest after it ([`OpLog::take_before`], `GameState::network_send_input`);
//! a request sent mid-frame (`EntityAttack`, `EntityInteract`, `ItemAction`,
//! `DeviceInteract`) first sends the ops logged before any unsent edit
//! (`GameState::flush_ops_before_edits`). So a Close then a Q-drop in one
//! tick reach the server as Close, Drop; a placement then E as the input,
//! then `OpenPlayer`.
//!
//! **Server.** [`serve_op`] applies the op to its copy of that joiner's
//! window (`ServerPlayer`'s 36 slots, armour, cursor, craft grid and
//! station) by the same rule, `window::apply`, judging a table's reach from
//! the server's body in the server's world. [`note_served`] compares the
//! digests and tallies (`PossessionTally`; C3a-fix-1: the first comparison
//! after join is the baseline, not a mismatch). It is log-only: nothing is
//! refused, nothing is sent back. A rule refusal leaves the server's window
//! as the rule leaves it, exactly as on the client. Before an op, the server
//! applies its own window events the client had applied by then
//! (`window_events`, `WindowOpPacket.events_applied`) and checks the op
//! sequence (`window_events::WindowEvents::note_op_seq`).

use std::cell::Cell;

use crate::protocol::{BlockChange, EditHand, WindowOpPacket, WireWindowOp};
use crate::server::ServerPlayer;
use crate::window::{self, ClickCtx, ClickResult, Station, WindowClick, WindowMut};
use crate::world::World;

thread_local! {
    /// The client's order clock ([`order_stamp`]).
    static ORDER: Cell<u64> = const { Cell::new(0) };
}

/// C3a-fix-1 — a stamp from the client's order clock: strictly increasing on
/// this thread. A window op ([`OpLog`]) and the first unsent edit
/// ([`PendingEdits`]) are each stamped when made, so the send can tell which
/// came first. Per thread, not per game: a client's game loop runs on one
/// thread, and another game on the same thread (a test's host and joiner)
/// only skips numbers, which keeps every client's own order.
pub fn order_stamp() -> u64 {
    ORDER.with(|c| {
        let next = c.get().wrapping_add(1);
        c.set(next);
        next
    })
}

/// One logged op: the op, the window's digest after it, and its order stamp.
#[derive(Debug)]
struct Logged {
    op: WireWindowOp,
    digest: u32,
    stamp: u64,
}

/// A client's window ops waiting to be sent, oldest first, each with the
/// window's digest after it.
#[derive(Debug, Default)]
pub struct OpLog {
    pending: Vec<Logged>,
    /// The auto-refill setting last logged this session; `None` until the
    /// first, so a session always starts by sending it.
    auto_refill: Option<bool>,
}

impl OpLog {
    /// Before an op is applied: if the window's auto-refill setting `on`
    /// isn't the one last logged, log `SetAutoRefill` first, with the digest
    /// of the window as it stands (`digest`, read only then): the server
    /// applies it before the op, to the window as it was.
    pub fn sync_auto_refill(&mut self, on: bool, digest: impl FnOnce() -> u32) {
        if self.auto_refill != Some(on) {
            self.auto_refill = Some(on);
            self.record(WireWindowOp::SetAutoRefill { on }, digest());
        }
    }

    /// Log an op just applied, with the window's digest after it.
    pub fn record(&mut self, op: WireWindowOp, digest: u32) {
        self.pending.push(Logged { op, digest, stamp: order_stamp() });
    }

    /// Everything logged, oldest first, for a joined client to send. A
    /// change of the auto-refill setting (now `auto_refill`) since the last
    /// op goes last, with the window's digest now (`digest_now`).
    pub fn take(&mut self, auto_refill: bool, digest_now: u32) -> Vec<(WireWindowOp, u32)> {
        self.sync_auto_refill(auto_refill, || digest_now);
        std::mem::take(&mut self.pending).into_iter().map(|l| (l.op, l.digest)).collect()
    }

    /// C3a-fix-1 (B-L1) — the ops logged before the unsent edit stamped
    /// `first_edit` (every op, if there is none), oldest first; the rest
    /// stay, to go after the input that carries the edit.
    pub fn take_before(&mut self, first_edit: Option<u64>) -> Vec<(WireWindowOp, u32)> {
        let cut = match first_edit {
            Some(edit) => self.pending.iter().position(|l| l.stamp > edit).unwrap_or(self.pending.len()),
            None => self.pending.len(),
        };
        self.pending.drain(..cut).map(|l| (l.op, l.digest)).collect()
    }

    /// Drop everything (not joined, so nothing is sent) and forget the
    /// setting, so the next joined session starts by sending it.
    pub fn discard(&mut self) {
        self.pending.clear();
        self.auto_refill = None;
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

/// C3a-fix-1 — a client's block edits not sent yet (`GameState`'s
/// `pending_block_changes`), with what the send needs to know of each:
/// - the order stamp of the first ([`order_stamp`]), so the ops logged before
///   it go ahead of the input that carries it (B-L1);
/// - C-M1 — the hotbar slot and hand each was made with
///   (`InputPacket.edit_hands`). The client stamps them just before its
///   hotbar selection changes and at the send ([`Self::stamp_hands`]): an
///   edit takes the selection it was made under, whatever the player
///   scrolled to before the input went out.
#[derive(Debug, Default)]
pub struct PendingEdits {
    edits: Vec<BlockChange>,
    /// The hands of the first `hands.len()` edits.
    hands: Vec<EditHand>,
    /// The order stamp of the first edit since the last take.
    first: Option<u64>,
}

impl PendingEdits {
    pub fn push(&mut self, edit: BlockChange) {
        if self.edits.is_empty() {
            self.first = Some(order_stamp());
        }
        self.edits.push(edit);
    }

    pub fn extend(&mut self, edits: impl IntoIterator<Item = BlockChange>) {
        for edit in edits {
            self.push(edit);
        }
    }

    /// Drop everything unsent (the world was left or reset).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.edits.len()
    }

    #[cfg(test)]
    pub fn iter(&self) -> std::slice::Iter<'_, BlockChange> {
        self.edits.iter()
    }

    /// The order stamp of the first unsent edit, if any.
    pub fn first_stamp(&self) -> Option<u64> {
        self.first
    }

    /// Every edit without a hand yet was made with `hand` (the selection is
    /// about to change, or the input is going out).
    pub fn stamp_hands(&mut self, hand: EditHand) {
        self.hands.resize(self.edits.len(), hand);
    }

    /// Take everything for the input going out, the edits not stamped yet
    /// made with `hand`: the edits and their hands, in step.
    pub fn take(&mut self, hand: EditHand) -> (Vec<BlockChange>, Vec<EditHand>) {
        self.stamp_hands(hand);
        let taken = std::mem::take(self);
        (taken.edits, taken.hands)
    }
}

/// What kind of window op it was: the tally keeps the first mismatch's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Slot,
    Grid,
    Armour,
    Result,
    Trash,
    DragDistribute,
    DragGather,
    Sort,
    ToggleLock,
    Autofill,
    Close,
    OpenPlayer,
    OpenTable,
    SetAutoRefill,
}

impl OpKind {
    pub fn of(op: &WireWindowOp) -> Self {
        match op {
            WireWindowOp::Click(click) => match click {
                WindowClick::Slot { .. } => OpKind::Slot,
                WindowClick::Grid { .. } => OpKind::Grid,
                WindowClick::Armour { .. } => OpKind::Armour,
                WindowClick::Result => OpKind::Result,
                WindowClick::Trash => OpKind::Trash,
                WindowClick::DragDistribute { .. } => OpKind::DragDistribute,
                WindowClick::DragGather { .. } => OpKind::DragGather,
                WindowClick::Sort => OpKind::Sort,
                WindowClick::ToggleLock { .. } => OpKind::ToggleLock,
                WindowClick::Autofill { .. } => OpKind::Autofill,
                WindowClick::Close => OpKind::Close,
            },
            WireWindowOp::OpenPlayer => OpKind::OpenPlayer,
            WireWindowOp::OpenTable { .. } => OpKind::OpenTable,
            WireWindowOp::SetAutoRefill { .. } => OpKind::SetAutoRefill,
        }
    }

    /// Its name in the server log.
    pub fn label(self) -> &'static str {
        match self {
            OpKind::Slot => "slot",
            OpKind::Grid => "grid",
            OpKind::Armour => "armour",
            OpKind::Result => "result",
            OpKind::Trash => "trash",
            OpKind::DragDistribute => "drag distribute",
            OpKind::DragGather => "drag gather",
            OpKind::Sort => "sort",
            OpKind::ToggleLock => "lock",
            OpKind::Autofill => "autofill",
            OpKind::Close => "close",
            OpKind::OpenPlayer => "open inventory",
            OpKind::OpenTable => "open table",
            OpKind::SetAutoRefill => "auto-refill",
        }
    }
}

/// What applying one op did to the server's window.
#[derive(Clone, Debug, PartialEq)]
pub struct Served {
    /// The rule's result for a click; `None` for an open or a setting.
    pub result: Option<ClickResult>,
    /// The server's window digest after the op.
    pub digest: u32,
}

impl Served {
    /// The rule refused the click (`Refused`, `NeedsTable`).
    pub fn refused(&self) -> bool {
        self.result.as_ref().is_some_and(|r| !r.ok())
    }
}

/// Apply joiner `sp`'s window op `op` to the server's copy of its window, on
/// `world` (`creative`: the world's mode). Mirrored for a body in the world,
/// dead or alive (`item_actions::can_mirror`: the client acted before it
/// heard of its death); `None` — nothing applied — for anyone else (a local
/// slot, a seat still joining, one that has left).
///
/// A click is `window::apply` at the server's station, from the server
/// body's eye; a close that returned everything resets the station to the
/// player's grid, as the client's screen closes. `OpenPlayer` and
/// `OpenTable` set the station; `SetAutoRefill` sets the setting.
pub fn serve_op(sp: &mut ServerPlayer, world: &World, creative: bool, op: &WireWindowOp) -> Option<Served> {
    if !crate::item_actions::can_mirror(sp) {
        return None;
    }
    let eye = sp.player.eye_pos();
    let mut station = sp.station;
    let mut view = WindowMut {
        inv: &mut sp.inventory,
        armour: &mut sp.armour,
        cursor: &mut sp.cursor,
        grid: &mut sp.craft_grid,
        container: None,
    };
    let result = match op {
        WireWindowOp::Click(click) => {
            // B-L2: the server's table verdict is kinder than the client's,
            // so it is a superset of an honest client's.
            let lately = sp.table_gone_ticks.is_none_or(|n| n <= window::SERVER_TABLE_GRACE_TICKS);
            let ctx = ClickCtx::new(creative, station, eye, |c| world.get_block(c[0], c[1], c[2]))
                .with_server_slack(lately);
            let result = window::apply(&mut view, click, &ctx);
            station = window::station_after(station, click, &result);
            Some(result)
        }
        WireWindowOp::OpenPlayer => {
            station = Station::Player;
            sp.table_gone_ticks = None;
            None
        }
        WireWindowOp::OpenTable { cell } => {
            station = Station::Table { cell: *cell };
            sp.table_gone_ticks = None;
            None
        }
        WireWindowOp::SetAutoRefill { on } => {
            view.inv.auto_refill = *on;
            None
        }
    };
    let digest = window::digest(&view, station);
    sp.station = station;
    Some(Served { result, digest })
}

/// B-L2 — once a tick, for a joiner whose open screen is a crafting table's:
/// count the ticks its cell has not been a crafting table
/// (`ServerPlayer::table_gone_ticks`), so [`serve_op`] can grant a table that
/// just changed a few ticks' grace ([`window::SERVER_TABLE_GRACE_TICKS`]): the
/// client acts on a world that is a few ticks behind the server's.
pub fn watch_table(sp: &mut ServerPlayer, world: &World) {
    sp.table_gone_ticks = match sp.station {
        Station::Table { cell } if world.get_block(cell[0], cell[1], cell[2]) != crate::block::CRAFTING_TABLE => {
            Some(sp.table_gone_ticks.map_or(1, |n| n.saturating_add(1)))
        }
        _ => None,
    };
}

/// Tally op `pkt` as served (`served`) on joiner `sp`'s possession counters:
/// every op is counted; unless `creative`, a rule refusal is counted (a
/// no-op when the client's digest agrees, a refusal when it doesn't) and a
/// digest that differs from the client's is a mismatch (the first one's
/// kind is kept, and it is logged at info; the rest at debug). Log-only.
///
/// C3a-fix-1 (decision 2) — the first comparison after join is not tallied:
/// it is recorded as the baseline (`WindowEvents::baseline`), the window the
/// joiner arrived with, which no wire carries yet (the sidecar's join sync
/// will).
pub fn note_served(sp: &mut ServerPlayer, pkt: &WindowOpPacket, served: &Served, creative: bool) {
    let tally = &mut sp.possession;
    tally.window_ops = tally.window_ops.saturating_add(1);
    if creative {
        return;
    }
    if served.refused() {
        // B-L4: refused on both sides (the client's digest is the server's:
        // its rule refused too) is a benign no-op; refused here alone is not.
        if served.digest == pkt.digest {
            tally.window_noop = tally.window_noop.saturating_add(1);
        } else {
            tally.window_refused = tally.window_refused.saturating_add(1);
        }
    }
    let matched = served.digest == pkt.digest;
    if sp.window_events.baseline.is_none() {
        sp.window_events.baseline = Some(matched);
        if !matched {
            log::info!(
                "window mirror (log-only): {}'s window differs at its first op after join ({}): recorded as the baseline, not tallied",
                sp.display_name,
                OpKind::of(&pkt.op).label(),
            );
        }
        return;
    }
    if matched {
        return;
    }
    let kind = OpKind::of(&pkt.op);
    let level = if tally.note_window_mismatch(kind) { log::Level::Info } else { log::Level::Debug };
    log::log!(
        level,
        "window mirror (log-only): {}'s window differs after op {} ({}): client digest {:08x}, server {:08x}",
        sp.display_name,
        pkt.op_seq,
        kind.label(),
        pkt.digest,
        served.digest,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_starts_by_logging_auto_refill_then_only_its_changes() {
        let mut log = OpLog::default();
        log.sync_auto_refill(true, || 11);
        log.record(WireWindowOp::OpenPlayer, 11);
        log.sync_auto_refill(true, || panic!("unchanged: no digest is read"));
        log.record(WireWindowOp::Click(WindowClick::Sort), 12);
        assert_eq!(
            log.take(false, 13),
            vec![
                (WireWindowOp::SetAutoRefill { on: true }, 11),
                (WireWindowOp::OpenPlayer, 11),
                (WireWindowOp::Click(WindowClick::Sort), 12),
                (WireWindowOp::SetAutoRefill { on: false }, 13),
            ],
            "a change since the last op goes last, with the digest now"
        );
        assert!(log.take(false, 13).is_empty(), "nothing new");
        // Not joined: dropped, and the next session sends the setting again.
        log.record(WireWindowOp::OpenPlayer, 1);
        log.discard();
        assert_eq!(log.len(), 0);
        assert_eq!(log.take(false, 2), vec![(WireWindowOp::SetAutoRefill { on: false }, 2)]);
    }

    /// C3a-fix-1 (B-L1) — the ops logged before the first unsent edit go
    /// before the input, the rest after it.
    #[test]
    fn ops_split_at_the_first_unsent_edit() {
        let mut log = OpLog::default();
        let mut edits = PendingEdits::default();
        log.record(WireWindowOp::Click(WindowClick::Close), 1);
        edits.push(BlockChange { x: 1, y: 2, z: 3, new_block: 4, meta: 0 });
        log.record(WireWindowOp::OpenPlayer, 2);
        edits.push(BlockChange { x: 5, y: 2, z: 3, new_block: 4, meta: 0 });
        log.record(WireWindowOp::Click(WindowClick::Sort), 3);
        assert_eq!(log.take_before(edits.first_stamp()), vec![(WireWindowOp::Click(WindowClick::Close), 1)]);
        assert_eq!(log.take_before(edits.first_stamp()), vec![], "the rest wait for the input");
        let (sent, _) = edits.take((0, 0, 0));
        assert_eq!(sent.len(), 2);
        assert_eq!(edits.first_stamp(), None);
        assert_eq!(
            log.take_before(edits.first_stamp()),
            vec![(WireWindowOp::OpenPlayer, 2), (WireWindowOp::Click(WindowClick::Sort), 3)],
            "with no edit unsent, everything goes"
        );
    }

    /// C3a-fix-1 (C-M1) — an edit keeps the hand it was made with: hands are
    /// stamped before the selection changes, the rest at the send.
    #[test]
    fn each_unsent_edit_keeps_the_hand_it_was_made_with() {
        let mut edits = PendingEdits::default();
        let edit = |x| BlockChange { x, y: 70, z: 0, new_block: 1, meta: 0 };
        edits.push(edit(1));
        edits.push(edit(2));
        edits.stamp_hands((2, 1, 7));
        edits.extend([edit(3)]);
        edits.stamp_hands((5, 4, 9));
        edits.stamp_hands((6, 0, 0));
        edits.push(edit(4));
        let (sent, hands) = edits.take((8, 1, 3));
        assert_eq!(sent.iter().map(|b| b.x).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert_eq!(hands, vec![(2, 1, 7), (2, 1, 7), (5, 4, 9), (8, 1, 3)]);
        assert!(edits.is_empty() && edits.take((0, 0, 0)).1.is_empty());
        edits.push(edit(9));
        edits.clear();
        assert_eq!((edits.len(), edits.first_stamp()), (0, None));
    }

    #[test]
    fn every_op_kind_has_a_label() {
        let ops = [
            WireWindowOp::Click(WindowClick::Result),
            WireWindowOp::Click(WindowClick::Close),
            WireWindowOp::OpenPlayer,
            WireWindowOp::OpenTable { cell: [0, 0, 0] },
            WireWindowOp::SetAutoRefill { on: true },
        ];
        let labels: Vec<&str> = ops.iter().map(|op| OpKind::of(op).label()).collect();
        assert_eq!(labels, ["result", "close", "open inventory", "open table", "auto-refill"]);
    }
}
