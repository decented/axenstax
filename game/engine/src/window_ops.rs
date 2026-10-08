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
//!
//! **C3b-1 (v77) — shared containers.** A container op
//! (`WireWindowOp::Container`) acts on the REAL container the joiner has
//! open (`ServerPlayer::open_container`, opened by `HostedServer` on
//! `OpenContainer`), as a shared container. The container is everyone's, so
//! it doesn't follow any one client's order: the server re-runs the op over
//! the player slots the client CLAIMS it held before it
//! (`WindowOpPacket::claims`, `container_window::ClaimedWindow`) and the
//! real container — the container's new contents are that run's, and a
//! player slot whose result differs from what the client predicted (run the
//! same way over the container as last sent to it, [`SentContainer`]) is
//! corrected to that run's result: relative to the client's own state, never
//! to the server's copy, which drifts until C3d. The correction
//! (`WindowSlotSet`) also carries the real values of the container slots the
//! op involved where they differ from what the client's prediction left in
//! its mirror; its player part is a numbered
//! window event (`window_events::WindowEvent::SetSlots`). The server's own
//! copy applies the op as usual, over a copy of the container, and tallies
//! what the client deposited that it didn't hold. [`container_push`] sends
//! what others changed in an open container, once a tick.

use std::cell::Cell;

use crate::container_window::{self, ClaimedWindow, ContainerClick, ContainerData, ContainerKind};
use crate::item::ItemStack;
use crate::protocol::{
    BlockChange, EditHand, FurnaceView, WindowOpPacket, WindowSlotSetPacket, WireSlot, WireWindowOp, WireWindowSlot,
};
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

/// One logged window op: the op, the window's digest after it, its order
/// stamp, and (C3b-1, a container op) the slots the client's apply changed
/// and its claims (the player slots it acts on, as they were before it).
#[derive(Clone, Debug, PartialEq)]
pub struct LoggedOp {
    pub op: WireWindowOp,
    pub digest: u32,
    pub touched: Vec<WireWindowSlot>,
    pub claims: Vec<(WireWindowSlot, WireSlot)>,
    stamp: u64,
}

/// A client's window ops waiting to be sent, oldest first, each with the
/// window's digest after it.
#[derive(Debug, Default)]
pub struct OpLog {
    pending: Vec<LoggedOp>,
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
        self.record_container(op, digest, Vec::new(), Vec::new());
    }

    /// C3b-1 — log a container op just applied, with the window's digest
    /// after it (container included), the slots it changed and its claims.
    pub fn record_container(
        &mut self,
        op: WireWindowOp,
        digest: u32,
        touched: Vec<WireWindowSlot>,
        claims: Vec<(WireWindowSlot, WireSlot)>,
    ) {
        self.pending.push(LoggedOp { op, digest, touched, claims, stamp: order_stamp() });
    }

    /// Everything logged, oldest first, for a joined client to send. A
    /// change of the auto-refill setting (now `auto_refill`) since the last
    /// op goes last, with the window's digest now (`digest_now`).
    pub fn take(&mut self, auto_refill: bool, digest_now: u32) -> Vec<LoggedOp> {
        self.sync_auto_refill(auto_refill, || digest_now);
        std::mem::take(&mut self.pending)
    }

    /// C3a-fix-1 (B-L1) — the ops logged before the unsent edit stamped
    /// `first_edit` (every op, if there is none), oldest first; the rest
    /// stay, to go after the input that carries the edit.
    pub fn take_before(&mut self, first_edit: Option<u64>) -> Vec<LoggedOp> {
        let cut = match first_edit {
            Some(edit) => self.pending.iter().position(|l| l.stamp > edit).unwrap_or(self.pending.len()),
            None => self.pending.len(),
        };
        self.pending.drain(..cut).collect()
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
    OpenContainer,
    Container,
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
            WireWindowOp::OpenContainer { .. } => OpKind::OpenContainer,
            WireWindowOp::Container(_) => OpKind::Container,
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
            OpKind::OpenContainer => "open container",
            OpKind::Container => "container",
        }
    }
}

/// What applying one op did to the server's window.
#[derive(Clone, Debug, PartialEq)]
pub struct Served {
    /// The rule's result for a click; `None` for an open or a setting. For
    /// a container op, the server's own copy's result.
    pub result: Option<ClickResult>,
    /// The server's window digest after the op (C3b-1: a container op's
    /// covers the real container).
    pub digest: u32,
    /// C3b-1 — a container op refused before the rule ran: no container
    /// open, its cell gone, or beyond the server body's reach.
    pub container_refused: bool,
    /// C3b-1 — the correction a container op earns, if any.
    pub correction: Option<Correction>,
    /// C3b-1 — the open furnace's progress after a container op.
    pub furnace: Option<FurnaceView>,
    /// C3b-1 — units a container op put into the real container, by the
    /// client's claims, beyond what the server's own copy of its window gave
    /// it: believed deposits (a locally fished item, unmirrored until C3c).
    pub believed: u32,
}


impl Served {
    /// The rule refused the click (`Refused`, `NeedsTable`, `PlanStays`).
    pub fn refused(&self) -> bool {
        self.result.as_ref().is_some_and(|r| !r.ok())
    }
}

/// C3b-1 — the correction a container op earns (`WindowSlotSet {
/// Correction }`).
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    /// The player slots whose result differs from the client's prediction,
    /// with the result of the op re-run over the client's claims and the
    /// real container: a numbered window event
    /// (`window_events::WindowEvent::SetSlots`), applied on both sides in
    /// the client's order.
    pub player: Vec<(WireWindowSlot, Option<ItemStack>)>,
    /// Everything to send: those player slots, then the container slots the
    /// op involved at their real values (shared state, applied at once).
    pub sets: Vec<(WireWindowSlot, WireSlot)>,
}

/// C3b-1 — how long a container slot the server pushed or corrected to a
/// joiner may still be on its way (two seconds: past any round trip a game
/// is playable over). Until then the joiner may have acted on the slot's
/// value from before, and [`serve_op`] judges its prediction both ways.
pub const IN_FLIGHT_TICKS: u64 = 40;

/// C3b-1 — what the server last sent a joiner of the container it has open
/// (`ContainerOpened`, then every correction and push): its contents, by
/// value (the client's prediction of each op is run over them, [`serve_op`]),
/// and a furnace's progress. Own ops update it for the slots they involved,
/// so a joiner's own clicks don't come back as pushes.
#[derive(Clone, Debug)]
pub struct SentContainer {
    pub kind: ContainerKind,
    pub contents: ContainerData,
    pub furnace: Option<FurnaceView>,
    /// Per slot, while a push or correction of it may still be on its way
    /// ([`IN_FLIGHT_TICKS`]): the tick it was last sent, and its value
    /// before the first of those sends.
    in_flight: Vec<Option<(u64, Option<ItemStack>)>>,
}

impl SentContainer {
    /// What `ContainerOpened` sent.
    pub fn new(kind: ContainerKind, contents: ContainerData, furnace: Option<FurnaceView>) -> Self {
        let in_flight = vec![None; contents.as_ref().len()];
        SentContainer { kind, contents, furnace, in_flight }
    }

    /// Slot `i` was sent as `value` on tick `now` (a push or a correction):
    /// it may not reach the joiner for a while.
    fn send(&mut self, i: usize, value: Option<ItemStack>, now: u64) {
        let was = self.contents.as_ref().get(i).cloned();
        if let Some(slot) = self.in_flight.get_mut(i) {
            let before = match slot.take() {
                Some((at, before)) if now.saturating_sub(at) <= IN_FLIGHT_TICKS => before,
                _ => was,
            };
            *slot = Some((now, before));
        }
        self.contents.set(i, value);
    }

    /// Slot `i` is now `value` by the joiner's own op: its prediction holds
    /// it already.
    fn settle(&mut self, i: usize, value: Option<ItemStack>) {
        self.contents.set(i, value);
    }

    /// The contents as the joiner may still be seeing them on tick `now`:
    /// every slot sent within [`IN_FLIGHT_TICKS`] at its value before. `None`
    /// when nothing is on its way.
    fn as_seen_lately(&self, now: u64) -> Option<ContainerData> {
        let mut lately: Option<ContainerData> = None;
        for (i, slot) in self.in_flight.iter().enumerate() {
            if let Some((at, before)) = slot
                && now.saturating_sub(*at) <= IN_FLIGHT_TICKS
            {
                lately.get_or_insert_with(|| self.contents.clone()).set(i, before.clone());
            }
        }
        lately
    }
}

/// Apply joiner `sp`'s window op `pkt` to the server's copy of its window,
/// on `world` (`creative`: the world's mode; `registry` decodes a container
/// op's claims; `now`: the server tick). Mirrored for a body in the world, dead or alive
/// (`item_actions::can_mirror`: the client acted before it heard of its
/// death); `None` — nothing applied — for anyone else (a local slot, a seat
/// still joining, one that has left).
///
/// A click is `window::apply` at the server's station, from the server
/// body's eye; a close that returned everything resets the station to the
/// player's grid, as the client's screen closes. `OpenPlayer` and
/// `OpenTable` set the station; `SetAutoRefill` sets the setting.
///
/// C3b-1 — `Close`, `OpenPlayer` and `OpenTable` also close the open
/// container; `OpenContainer` changes no window (the caller opens the
/// container); a container op is [`serve_container`].
pub fn serve_op(
    sp: &mut ServerPlayer,
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    creative: bool,
    pkt: &WindowOpPacket,
    now: u64,
) -> Option<Served> {
    if !crate::item_actions::can_mirror(sp) {
        return None;
    }
    let op = &pkt.op;
    let eye = sp.player.eye_pos();
    let mut station = sp.station;
    // B-L2: the server's table verdict is kinder than the client's, so it is
    // a superset of an honest client's.
    let lately = sp.table_gone_ticks.is_none_or(|n| n <= window::SERVER_TABLE_GRACE_TICKS);
    let ctx = ClickCtx::new(creative, station, eye, |c| world.get_block(c[0], c[1], c[2])).with_server_slack(lately);
    if let WireWindowOp::Container(click) = op {
        let served = serve_container(sp, world, registry, &ctx.with_shared(true), click, pkt, now);
        sp.last_window_op_seq = pkt.op_seq;
        return Some(served);
    }
    let mut view = WindowMut {
        inv: &mut sp.inventory,
        armour: &mut sp.armour,
        cursor: &mut sp.cursor,
        grid: &mut sp.craft_grid,
        container: None,
    };
    let result = match op {
        WireWindowOp::Click(click) => {
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
        WireWindowOp::OpenContainer { .. } | WireWindowOp::Container(_) => None,
    };
    let digest = window::digest(&view, station);
    let closes = matches!(
        op,
        WireWindowOp::Click(WindowClick::Close) | WireWindowOp::OpenPlayer | WireWindowOp::OpenTable { .. }
    );
    if closes {
        sp.open_container = None;
        sp.container_sent = None;
    }
    sp.station = station;
    sp.last_window_op_seq = pkt.op_seq;
    Some(Served { result, digest, container_refused: false, correction: None, furnace: None, believed: 0 })
}

/// C3b-1 (v77) — joiner `sp`'s container op `click` on the REAL container
/// it has open, as a shared container (`ctx.shared`).
///
/// Refused before the rule (`container_refused`) when none is open, its cell
/// no longer holds it (then it is closed), or it stands beyond the server
/// body's reach, judged with the server's slack
/// (`container_window::container_in_server_reach`). Nothing moved, so the
/// player slots the client changed go back to the values it claims they had
/// (a correction).
///
/// Otherwise the op runs three times, over the same rule
/// (`container_window::apply_container`):
/// - **R, the shared truth:** over the player slots the client claims it
///   held before it (`pkt.claims`, every other slot a blocker,
///   `container_window::ClaimedWindow`) and the REAL container. The real
///   container keeps R's result. A deposit of an item the server's copy of
///   the window doesn't hold is BELIEVED: the container receives the
///   claimed item and `believed` counts it.
///   BRIDGE: C3d refuses and corrects from the server's window — replace
///   when the flip lands (C3d). The same fabrication class as a claimed
///   Q-drop (Spec 04 §4.2e).
/// - **P, the client's prediction as the server sees it:** the same claims
///   over the container as last sent to the joiner ([`SentContainer`]), and
///   — while a push or correction of some slot may still be on its way
///   ([`IN_FLIGHT_TICKS`]) — over the container as the joiner may still be
///   seeing it (a race it lost before the winner's push reached it).
/// - **The server's own copy** (`ServerPlayer`'s window) applies the op as
///   usual, over a copy of the container as it was: log-only, compared by
///   digest (the real container's), its shortfall tallied (`believed`).
///
/// The view the client predicted on is the one whose window (the server's
/// copy with the claimed slots at that view's results, and that view's
/// container) digests as the client's did; a drift confined to the claimed
/// slots doesn't hide it. When neither matches (a drift elsewhere), both
/// views are judged.
///
/// The correction: every claimed player slot where R differs from P (on a
/// judged view), at R's value — relative to the client's own pre-op state,
/// never the server's drifted copy, and only where the real container
/// changed the outcome (so a drift never earns one by itself, and no
/// correction touches a slot the op didn't claim); then every container
/// slot the op involved (either side's changes and the click's named slots)
/// whose real value differs from what P left in the client's mirror. Not on
/// a digest mismatch alone: a drifted joiner would be corrected on every op,
/// and a correction landing after its next click on the same slot would
/// overwrite that click. The container slots it involved are recorded as
/// sent at their real values (in flight, [`IN_FLIGHT_TICKS`], if corrected).
fn serve_container(
    sp: &mut ServerPlayer,
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    ctx: &ClickCtx,
    click: &ContainerClick,
    pkt: &WindowOpPacket,
    now: u64,
) -> Served {
    use crate::window::slot_print;
    let station = sp.station;
    let eye = sp.player.eye_pos();
    let open = sp.open_container.zip(sp.container_sent.as_ref().map(|s| s.kind));
    let real_pre = open.and_then(|(cell, kind)| container_window::container_at(world, cell, kind)).map(ContainerData::of);
    let in_reach = open.is_some_and(|(cell, _)| container_window::container_in_server_reach(eye, cell));
    let (Some((cell, kind)), Some(real_pre), true, Some(sent)) = (open, real_pre.clone(), in_reach, sp.container_sent.as_ref())
    else {
        if open.is_some() && real_pre.is_none() {
            sp.open_container = None;
            sp.container_sent = None;
        }
        return refused_container_op(sp, registry, pkt);
    };
    // P — the client's prediction, over the container as last sent to it;
    // and, while a push or correction may still be on its way, over the
    // container as it may still be seeing it.
    let mut predicted = sent.contents.clone();
    let mut p_win = ClaimedWindow::from_claims(&pkt.claims, registry);
    let p = p_win.apply(predicted.as_mut(), click, ctx);
    let lately = sent.as_seen_lately(now).map(|mut c| {
        let mut w = ClaimedWindow::from_claims(&pkt.claims, registry);
        let applied = w.apply(c.as_mut(), click, ctx);
        (w, applied, c)
    });
    // Which view the client predicted on: the one whose window — the
    // server's copy with the claimed slots at that prediction's results —
    // digests as the client's did. A drift confined to the claimed slots
    // (a locally caught fish being deposited) doesn't hide it; a drift
    // elsewhere does, and then both views are judged.
    let view_digest = |w: &ClaimedWindow, c: &ContainerData| {
        let (mut inv, mut armour, mut cursor, mut grid) =
            (sp.inventory.clone(), sp.armour, sp.cursor.clone(), sp.craft_grid.clone());
        w.overlay(&mut inv, &mut armour, &mut cursor, &mut grid);
        window::digest_with(&inv, &armour, &cursor, &grid, station, Some(c.as_ref()))
    };
    let mut views: Vec<(&ClaimedWindow, &ContainerData)> = vec![(&p_win, &predicted)];
    if let Some((w, _, c)) = lately.as_ref() {
        views.push((w, c));
    }
    if let Some(&seen) = views.iter().find(|(w, c)| view_digest(w, c) == pkt.digest) {
        views = vec![seen];
    }
    // R — the shared truth, over the real container.
    let mut r_win = ClaimedWindow::from_claims(&pkt.claims, registry);
    let Some(real) = container_window::container_at_mut(world, cell, kind) else {
        return refused_container_op(sp, registry, pkt);
    };
    let r = r_win.apply(real, click, ctx);
    // The server's own copy of the window, over a copy of the container.
    let mut own_copy = real_pre.clone();
    let own = {
        let mut view = WindowMut {
            inv: &mut sp.inventory,
            armour: &mut sp.armour,
            cursor: &mut sp.cursor,
            grid: &mut sp.craft_grid,
            container: Some(own_copy.as_mut()),
        };
        container_window::apply_container(&mut view, click, ctx)
    };
    let Some(real) = container_window::container_at(world, cell, kind) else {
        return refused_container_op(sp, registry, pkt);
    };
    let believed = gained_beyond(real_pre.as_ref(), real, own_copy.as_ref());
    let digest = window::digest_with(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, station, Some(real));
    // A claimed player slot is corrected where R's result differs from the
    // client's prediction (on every view it may have predicted on).
    let player: Vec<(WireWindowSlot, Option<ItemStack>)> = r_win
        .claimed_values()
        .into_iter()
        .filter(|(at, rv)| {
            views.iter().any(|(w, _)| w.stack(*at).is_some_and(|pv| slot_print(pv.as_ref()) != slot_print(rv.as_ref())))
        })
        .collect();
    let mut involved: Vec<WireWindowSlot> = Vec::new();
    let lately_touched = lately.as_ref().map(|(_, a, _)| a.touched.as_slice()).unwrap_or_default();
    for at in pkt
        .touched
        .iter()
        .chain(&r.touched)
        .chain(&p.touched)
        .chain(lately_touched)
        .copied()
        .chain(container_window::named_slots(click))
    {
        if !container_window::is_player_slot(at) && !involved.contains(&at) {
            involved.push(at);
        }
    }
    let involved: Vec<(WireWindowSlot, Option<&ItemStack>)> =
        involved.into_iter().filter_map(|at| slot_at(real, at).map(|v| (at, v))).collect();
    // A container slot the op involved is corrected where the real one
    // differs from what the client's prediction left in its mirror.
    let mispredicted: Vec<(WireWindowSlot, Option<&ItemStack>)> = involved
        .iter()
        .copied()
        .filter(|(at, v)| {
            views.iter().any(|(_, c)| slot_at(c.as_ref(), *at).is_none_or(|pv| slot_print(pv) != slot_print(*v)))
        })
        .collect();
    let furnace = real.furnace_view();
    let corrected: Vec<WireWindowSlot> = mispredicted.iter().map(|(at, _)| *at).collect();
    let mut sets: Vec<(WireWindowSlot, WireSlot)> =
        player.iter().map(|(at, s)| (*at, s.as_ref().map(crate::inventory::stack_to_wire))).collect();
    sets.extend(mispredicted.iter().map(|(at, v)| (*at, v.map(crate::inventory::stack_to_wire))));
    let correction = (!sets.is_empty()).then_some(Correction { player, sets });
    // The slots it involved are as the joiner will hold them: its own
    // prediction, or (corrected) on their way.
    let involved: Vec<(WireWindowSlot, Option<ItemStack>)> = involved.iter().map(|(at, v)| (*at, v.cloned())).collect();
    if let Some(sent) = sp.container_sent.as_mut() {
        for (at, v) in involved {
            let WireWindowSlot::Container(i) = at else { continue };
            if corrected.contains(&at) {
                sent.send(usize::from(i), v, now);
            } else {
                sent.settle(usize::from(i), v);
            }
        }
        if correction.is_some() {
            sent.furnace = furnace;
        }
    }
    Served { result: Some(own.result), digest, container_refused: false, correction, furnace, believed }
}

/// Container slot `at` of `c` (`None` for a player slot or one past the
/// end).
fn slot_at(c: container_window::ContainerRef<'_>, at: WireWindowSlot) -> Option<Option<&ItemStack>> {
    match at {
        WireWindowSlot::Container(i) if usize::from(i) < c.len() => Some(c.get(usize::from(i))),
        _ => None,
    }
}

/// C3b-1 — a container op refused before the rule ran: nothing moved, so
/// the player slots the client changed (`pkt.touched`) go back to the values
/// it claims they had (a Plan the client holds is never named: it keeps it).
fn refused_container_op(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, pkt: &WindowOpPacket) -> Served {
    let player: Vec<(WireWindowSlot, Option<ItemStack>)> = ClaimedWindow::from_claims(&pkt.claims, registry)
        .claimed_values()
        .into_iter()
        .filter(|(at, _)| pkt.touched.contains(at))
        .collect();
    let sets: Vec<(WireWindowSlot, WireSlot)> =
        player.iter().map(|(at, s)| (*at, s.as_ref().map(crate::inventory::stack_to_wire))).collect();
    let digest = window::digest_with(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station, None);
    Served {
        result: Some(ClickResult::Refused),
        digest,
        container_refused: true,
        correction: (!sets.is_empty()).then_some(Correction { player, sets }),
        furnace: None,
        believed: 0,
    }
}

/// C3b-1 — units of each item the real container gained from `before` to
/// `real` beyond what the server's own copy (`own`, from the same `before`)
/// gained: what a container op put in by the client's claims that the
/// server's copy of the window didn't hold (believed deposits).
fn gained_beyond(before: container_window::ContainerRef, real: container_window::ContainerRef, own: container_window::ContainerRef) -> u32 {
    use std::collections::HashMap;
    // One stack's item, digested without its count (`window::slot_print`).
    let key = |s: &ItemStack| window::slot_print(Some(&ItemStack { item: s.item.clone(), count: 1 }));
    let mut gain: HashMap<u32, i64> = HashMap::new();
    for (c, sign) in [(real, 1i64), (own, -1), (before, 0)] {
        for i in 0..c.len() {
            if let Some(s) = c.get(i) {
                *gain.entry(key(s)).or_default() += sign * i64::from(s.count);
            }
        }
    }
    gain.values().map(|&g| g.max(0)).sum::<i64>().min(i64::from(u32::MAX)) as u32
}

/// C3b-1 — what changed in joiner `sp`'s open container since it was last
/// sent ([`SentContainer`]): the container slots whose contents differ, and
/// a furnace's progress if it moved, as a `WindowSlotSet { Changed }` (no
/// window event: container slots only). This is what everyone but the
/// joiner's own ops did: another player, a hopper, the furnace cooking, a
/// host's click on its lent world. `None` when nothing changed. A container
/// whose cell no longer holds it is closed (the client closes its screen by
/// the same rule, seeing the block go). `now`: the server tick.
pub fn container_push(sp: &mut ServerPlayer, world: &World, now: u64) -> Option<WindowSlotSetPacket> {
    let cell = sp.open_container?;
    let kind = sp.container_sent.as_ref()?.kind;
    let Some(c) = container_window::container_at(world, cell, kind) else {
        sp.open_container = None;
        sp.container_sent = None;
        return None;
    };
    let sent = sp.container_sent.as_mut()?;
    let was = sent.contents.as_ref().prints();
    let mut sets: Vec<(WireWindowSlot, WireSlot)> = Vec::new();
    for (i, print) in c.prints().into_iter().enumerate() {
        if was.get(i) == Some(&print) {
            continue;
        }
        let Ok(at) = u8::try_from(i) else { continue };
        sets.push((WireWindowSlot::Container(at), c.get(i).map(crate::inventory::stack_to_wire)));
        sent.send(i, c.get(i).cloned(), now);
    }
    let furnace = c.furnace_view();
    if sets.is_empty() && furnace == sent.furnace {
        return None;
    }
    sent.furnace = furnace;
    Some(WindowSlotSetPacket {
        op_seq_applied: sp.last_window_op_seq,
        reason: crate::protocol::slot_set_reason::CHANGED,
        sets,
        furnace,
        window_event: 0,
    })
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
        let taken = log.take(false, 13);
        assert!(taken.iter().all(|l| l.touched.is_empty()), "no op here touched a container");
        assert_eq!(
            taken.into_iter().map(|l| (l.op, l.digest)).collect::<Vec<_>>(),
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
        let again: Vec<_> = log.take(false, 2).into_iter().map(|l| (l.op, l.digest)).collect();
        assert_eq!(again, vec![(WireWindowOp::SetAutoRefill { on: false }, 2)]);
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
        let pairs = |ops: Vec<LoggedOp>| ops.into_iter().map(|l| (l.op, l.digest)).collect::<Vec<_>>();
        assert_eq!(pairs(log.take_before(edits.first_stamp())), vec![(WireWindowOp::Click(WindowClick::Close), 1)]);
        assert_eq!(pairs(log.take_before(edits.first_stamp())), vec![], "the rest wait for the input");
        let (sent, _) = edits.take((0, 0, 0));
        assert_eq!(sent.len(), 2);
        assert_eq!(edits.first_stamp(), None);
        assert_eq!(
            pairs(log.take_before(edits.first_stamp())),
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
            WireWindowOp::OpenContainer { cell: [0, 0, 0] },
            WireWindowOp::Container(ContainerClick::TakeAll),
        ];
        let labels: Vec<&str> = ops.iter().map(|op| OpKind::of(op).label()).collect();
        assert_eq!(
            labels,
            ["result", "close", "open inventory", "open table", "auto-refill", "open container", "container"]
        );
    }
}
