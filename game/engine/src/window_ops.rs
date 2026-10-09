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
//! each edit (C3b-fix-d: every edit, not just an input's first) are stamped
//! from one order clock ([`order_stamp`]; the edits wait in
//! [`PendingEdits`]). At the tick's send
//! the ops logged before the input's first edit go before the input and the
//! rest after it ([`OpLog::take_before`], `GameState::network_send_input`);
//! a request sent mid-frame (`EntityAttack`, `EntityInteract`, `ItemAction`,
//! `DeviceInteract`) with no edit unsent first sends the ops logged before
//! it (`GameState::flush_ops_before_edits`); with edits unsent (C3b-fix-b) it
//! waits and goes right after the input carrying them, in the op log's order
//! (`GameState::send_request`). So a Close then a Q-drop in one tick reach
//! the server as Close, Drop; a placement then E as the input, then
//! `OpenPlayer`; a placement then a Q-drop as the input, then the drop.
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
//! real container (R); the container's new contents are that run's.
//!
//! **C3b-fix-a (v78) — corrections that can't duplicate or lose.**
//! - Every container view the server sends (the opened container, every
//!   push, every correction's container slots) is a numbered window event,
//!   so the server knows EXACTLY which view each op was predicted on
//!   ([`ContainerViews::seen`], C-M1) and runs the client's prediction (P)
//!   on it.
//! - A correction moves items, never sets a player slot (C-H1): the client
//!   is told R − P by item ("take N of X", "give N of X",
//!   `container_window::ItemDelta`), resolved on its window as it is when
//!   the correction lands, and the server's copy applies only R − own, its
//!   own run's difference, never a claimed value.
//! - The phantom ledger: while a correction is on its way, what it will
//!   take back is debited from every later op's claims before R runs
//!   (`window_events::phantom`), so an item the server already refused is
//!   never believed again; a take that finds nothing is owed against the
//!   next give of that item (`container_window::CorrectionDebt`).
//! - A refused op's revert has no server-side effect (C-M2); believed
//!   deposits are bounded per joiner ([`BelievedBucket`], C-L3). Since
//!   C3b-2-fix (M2) the same bound pays a block use's believed take
//!   ([`believe_pay`]).
//! - C3b-fix-c — that holds for container ops. A phantom spent another way
//!   (placed, dropped, eaten, crafted) before its correction lands is a real
//!   item until C3d; the correction's short take shows it
//!   (`correction_short`). A correction's take is the one owed search
//!   (`joiner_actions::take_owed_search`: an exact match first everywhere,
//!   C3b-fix-e L1, the armour slots too), and a believed deposit counts as
//!   held only what that take can pay ([`believed_units`]).
//! - C3b-fix-e (C-M1) — a tool or armour piece deposited at a durability
//!   the copy doesn't hold (worn on the client by a use not mirrored until
//!   C3c), while the copy's own run deposits its own piece of that kind, is
//!   a swap ([`durability_swaps`]): the copy gives that piece up, so honest
//!   drift duplicates nothing; it costs the bound and is tallied
//!   `durability_swap`.
//!
//! [`container_push`] sends what others changed in an open container, once
//! a tick.

use std::cell::Cell;

use crate::container_window::{self, ClaimedWindow, ContainerClick, ContainerData, ContainerKind, ItemDelta};
use crate::item::ItemStack;
use crate::protocol::{
    BlockChange, EditHand, FurnaceView, UseTag, WindowOpPacket, WindowSlotSetPacket, WireSlot, WireWindowOp,
    WireWindowSlot,
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
/// stamp, (C3b-1, a container op) the slots the client's apply changed and
/// its claims (the player slots it acts on, as they were before it), and
/// (C3b-fix-a) the client's own verdict on it.
#[derive(Clone, Debug, PartialEq)]
pub struct LoggedOp {
    pub op: WireWindowOp,
    pub digest: u32,
    pub touched: Vec<WireWindowSlot>,
    pub claims: Vec<(WireWindowSlot, WireSlot)>,
    /// A-L2/A-L3 — the client's `ClickResult::ok()` (`true` for an op that
    /// isn't a click): `WindowOpPacket::client_ok`.
    pub client_ok: bool,
    stamp: u64,
}

impl LoggedOp {
    /// The packet that sends it as op `op_seq`, the client having applied
    /// window events up to `events_applied`.
    pub fn packet(self, op_seq: u32, events_applied: u32) -> WindowOpPacket {
        WindowOpPacket {
            op_seq,
            op: self.op,
            digest: self.digest,
            events_applied,
            touched: self.touched,
            claims: self.claims,
            client_ok: self.client_ok,
        }
    }
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
            self.record(WireWindowOp::SetAutoRefill { on }, digest(), true);
        }
    }

    /// Log an op just applied, with the window's digest after it and the
    /// client's own verdict on it (`ClickResult::ok()`; `true` for an op
    /// that isn't a click).
    pub fn record(&mut self, op: WireWindowOp, digest: u32, client_ok: bool) {
        self.record_container(op, digest, Vec::new(), Vec::new(), client_ok);
    }

    /// C3b-1 — log a container op just applied, with the window's digest
    /// after it (container included), the slots it changed, its claims and
    /// the client's verdict.
    pub fn record_container(
        &mut self,
        op: WireWindowOp,
        digest: u32,
        touched: Vec<WireWindowSlot>,
        claims: Vec<(WireWindowSlot, WireSlot)>,
        client_ok: bool,
    ) {
        self.pending.push(LoggedOp { op, digest, touched, claims, client_ok, stamp: order_stamp() });
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

    /// C3c-1 — the order stamp of the first op still waiting, if any: the
    /// send cuts its edits there, so an edit made after it goes in a later
    /// input, after it (`RemoteClient::note_order_cut`).
    pub fn first_stamp(&self) -> Option<u64> {
        self.pending.first().map(|l| l.stamp)
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
/// - its order stamp ([`order_stamp`]), so the ops logged before the first
///   go ahead of the input that carries it (B-L1); C3b-fix-d (A-L3) — every
///   edit's own, not just the first's: an edit that waits for a later input
///   (`RemoteClient`'s carry-over) keeps it, so what was made before it is
///   not held behind it;
/// - C-M1 — the hotbar slot and hand each was made with
///   (`InputPacket.edit_hands`). The client stamps them just before its
///   hotbar selection changes and at the send ([`Self::stamp_hands`]): an
///   edit takes the selection it was made under, whatever the player
///   scrolled to before the input went out;
/// - C3c-1 — a use's tag ([`UseTag`], `InputPacket.use_tags`), kept beside
///   the edit it was made with from the moment it is made, so it never
///   travels without it.
#[derive(Debug, Default)]
pub struct PendingEdits {
    edits: Vec<BlockChange>,
    /// The hands of the first `hands.len()` edits.
    hands: Vec<EditHand>,
    /// Each edit's order stamp, in step with `edits` (C3b-fix-d, A-L3).
    stamps: Vec<u64>,
    /// C3c-1 — each edit's use tag (`None` for every edit but a use's), in
    /// step with `edits`.
    uses: Vec<Option<UseTag>>,
}

impl PendingEdits {
    pub fn push(&mut self, edit: BlockChange) {
        self.stamps.push(order_stamp());
        self.edits.push(edit);
        self.uses.push(None);
    }

    /// C3c-1 — push a use's edit with its tag (`use_edits::tag`, stamped
    /// before the use spent its item).
    pub fn push_use(&mut self, edit: BlockChange, tag: UseTag) {
        self.push(edit);
        if let Some(last) = self.uses.last_mut() {
            *last = Some(tag);
        }
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
        self.stamps.first().copied()
    }

    /// Every edit without a hand yet was made with `hand` (the selection is
    /// about to change, or the input is going out).
    pub fn stamp_hands(&mut self, hand: EditHand) {
        self.hands.resize(self.edits.len(), hand);
    }

    /// Take everything for the input going out, the edits not stamped yet
    /// made with `hand`: the edits, their hands, their order stamps and
    /// (C3c-1) their use tags, in step.
    #[allow(clippy::type_complexity)]
    pub fn take(&mut self, hand: EditHand) -> (Vec<BlockChange>, Vec<EditHand>, Vec<u64>, Vec<Option<UseTag>>) {
        self.stamp_hands(hand);
        let taken = std::mem::take(self);
        (taken.edits, taken.hands, taken.stamps, taken.uses)
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
#[derive(Clone, Debug)]
pub struct Served {
    /// The rule's result for a click; `None` for an open or a setting. For
    /// a container op, the server's own copy's result.
    pub result: Option<ClickResult>,
    /// The server's window digest after the op (C3b-1: a container op's
    /// covers the container — v78: as the client's mirror shows it after its
    /// own prediction, so the comparison is per-joiner lockstep only, C-L4).
    pub digest: u32,
    /// C3b-1 — a container op refused before the rule ran: no container
    /// open, its cell gone, beyond the server body's reach, not the
    /// container the client's mirror shows, a claim above its stack, or a
    /// believed deposit past the bound (`over_bound`).
    pub container_refused: bool,
    /// C3b-1 — the correction a container op earns, if any (v78: a refused
    /// op's revert too).
    pub correction: Option<Correction>,
    /// C3b-1 — units a container op put into the real container, by the
    /// client's claims, beyond what the server's own copy of its window held:
    /// believed deposits (a locally fished item, unmirrored until C3c).
    /// C3b-fix-e (C-M1) — the swaps (`swapped`) are not among them.
    pub believed: u32,
    /// C3b-fix-e (C-M1) — of the units the op put in beyond what the copy
    /// held, those that were a swap: a tool or armour piece claimed at a
    /// durability the copy doesn't hold, paired with the copy's own piece of
    /// that kind that its own run deposited ([`durability_swaps`]). Charged
    /// to the believed bound like `believed`, tallied apart
    /// (`PossessionTally::durability_swap`).
    pub swapped: u32,
    /// C3b-fix-a (C-L3) — units a container op would have put in believed
    /// beyond the joiner's bound ([`BelievedBucket`]): the op was refused.
    pub over_bound: u32,
}

impl Served {
    /// The rule refused the click (`Refused`, `NeedsTable`, `PlanStays`).
    pub fn refused(&self) -> bool {
        self.result.as_ref().is_some_and(|r| !r.ok())
    }

    fn plain(result: Option<ClickResult>, digest: u32) -> Self {
        Served { result, digest, container_refused: false, correction: None, believed: 0, swapped: 0, over_bound: 0 }
    }
}

/// C3b-fix-a — a container as a joined client's mirror shows it: the
/// opened container and every view sent since, and the client's own
/// predictions.
#[derive(Clone, Debug)]
pub struct MirrorView {
    pub cell: [i32; 3],
    pub kind: ContainerKind,
    /// Its slots, and a furnace's progress (`FurnaceData`).
    pub contents: ContainerData,
}

/// C3b-fix-a (v78, C-M1) — one container view the server sends a joiner, a
/// numbered window event (`window_events::WindowEvent::ContainerView`, or a
/// correction's container part). It changes the client's mirror, never its
/// window; the server applies it to its model of that mirror
/// ([`ContainerViews::seen`]) when the client reports it applied it.
#[derive(Clone, Debug)]
pub enum ViewEvent {
    /// `ContainerOpened`: the mirror opens on these contents.
    Opened(MirrorView),
    /// A `WindowSlotSet`'s container part: these container slots at these
    /// values, and a furnace's progress if it moved.
    Sets { sets: Vec<(usize, Option<ItemStack>)>, furnace: Option<FurnaceView> },
}

impl ViewEvent {
    /// Apply this view to a mirror (`None`: none open, which a set skips).
    pub fn apply(&self, mirror: &mut Option<MirrorView>) {
        match self {
            ViewEvent::Opened(view) => *mirror = Some(view.clone()),
            ViewEvent::Sets { sets, furnace } => {
                let Some(m) = mirror.as_mut() else { return };
                for (i, stack) in sets {
                    m.contents.set(*i, stack.clone());
                }
                if let (Some(f), ContainerData::Furnace(data)) = (furnace, &mut m.contents) {
                    f.apply_to(data);
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, ViewEvent::Sets { sets, furnace: None } if sets.is_empty())
    }
}

/// C3b-fix-a (v78) — the correction a container op earns
/// (`WindowSlotSet { Correction }`, a numbered window event,
/// `window_events::WindowEvent::Correction`).
#[derive(Clone, Debug)]
pub struct Correction {
    /// The client's change, by item (C-H1): what the op gave it when re-run
    /// over its claims and the real container, minus what its own
    /// prediction gave it over the view it predicted on (R − P). A refused
    /// op's revert: its prediction undone (−P).
    pub client: ItemDelta,
    /// The server's copy's change, by item, applied when the client reports
    /// the event: R minus what its own run of the op gave it (R − own) —
    /// never the client's claimed values (C-M2). `None` for a refused op:
    /// the server's copy never moved.
    pub own: Option<ItemDelta>,
    /// The container slots the op involved whose real value differs from
    /// what the client's mirror will show (a refused op: the slots it
    /// touched and named), and a furnace's progress if it differs.
    pub view: ViewEvent,
}

impl Correction {
    /// The packet that carries it as window event `window_event`.
    pub fn packet(&self, op_seq_applied: u32, window_event: u32) -> WindowSlotSetPacket {
        let (sets, furnace) = match &self.view {
            ViewEvent::Sets { sets, furnace } => (wire_sets(sets), *furnace),
            ViewEvent::Opened(_) => (Vec::new(), None),
        };
        let (take, give) = self.client.to_wire();
        WindowSlotSetPacket {
            op_seq_applied,
            reason: crate::protocol::slot_set_reason::CORRECTION,
            sets,
            furnace,
            window_event,
            take,
            give,
        }
    }

    /// Does it correct the client at all (an item to move, or a container
    /// slot or progress to set)? What `container_corrected` counts.
    pub fn corrects_client(&self) -> bool {
        !self.client.is_empty() || !self.view.is_empty()
    }
}

/// Container slots on the wire.
fn wire_sets(sets: &[(usize, Option<ItemStack>)]) -> Vec<(WireWindowSlot, WireSlot)> {
    sets.iter()
        .filter_map(|(i, s)| u8::try_from(*i).ok().map(|at| (WireWindowSlot::Container(at), s.as_ref().map(crate::inventory::stack_to_wire))))
        .collect()
}

/// C3b-fix-a (C-L3) — the believed-units bound per joiner: units a joiner
/// may put into shared containers that the server's copy of its window
/// doesn't hold (a locally caught fish, honest until C3c mirrors fishing).
pub const BELIEVED_BUCKET_UNITS: u32 = 64;
/// ... refilled at this many units a second.
pub const BELIEVED_REFILL_PER_SECOND: u32 = 4;
/// Server ticks a second.
const TICKS_PER_SECOND: u32 = 20;

/// C3c-2-fix (M2) — the believed-AMMO bound per joiner: arrows and rubber
/// balls a `Shoot` spends that the server's copy of its window doesn't hold.
/// Its own bucket, drained by shots alone: on the main bound (refilled at 4
/// a second) a joiner shooting at the 2.5-a-second cadence never ran dry,
/// so believed ammo was unbounded.
pub const BELIEVED_AMMO_UNITS: u32 = 16;
/// ... refilled one unit every this many server ticks (4 s): a believed
/// burst of 16, then one shot in ten.
pub const BELIEVED_AMMO_REFILL_TICKS: u32 = 80;

/// A per-joiner bucket of believed units: `UNITS` deep, refilled one unit
/// every `TICKS_PER_UNIT` server ticks, all or nothing per take. The
/// believed bound ([`BelievedBucket`], C3b-fix-a C-L3) and the believed-ammo
/// bound ([`BelievedAmmo`], C3c-2-fix M2).
#[derive(Clone, Copy, Debug)]
pub struct Bucket<const UNITS: u32, const TICKS_PER_UNIT: u32> {
    /// In ticks of refill (`TICKS_PER_UNIT` a unit).
    level: u32,
    /// The server tick of the last refill.
    at: Option<u64>,
}

/// C3b-fix-a (C-L3) — a joiner's believed-units bucket: [`BELIEVED_BUCKET_UNITS`]
/// deep, refilled at [`BELIEVED_REFILL_PER_SECOND`]. A container op whose
/// believed deposit it can't pay is refused and corrected; a block use's
/// believed pay or tool, and (C3c-2-fix M2) a request's believed weapon, rod
/// or Firestarter, likewise.
pub type BelievedBucket = Bucket<BELIEVED_BUCKET_UNITS, { TICKS_PER_SECOND / BELIEVED_REFILL_PER_SECOND }>;

/// C3c-2-fix (M2) — a joiner's believed-ammo bucket ([`BELIEVED_AMMO_UNITS`]
/// deep, a unit every [`BELIEVED_AMMO_REFILL_TICKS`]).
pub type BelievedAmmo = Bucket<BELIEVED_AMMO_UNITS, BELIEVED_AMMO_REFILL_TICKS>;

impl<const UNITS: u32, const TICKS_PER_UNIT: u32> Default for Bucket<UNITS, TICKS_PER_UNIT> {
    fn default() -> Self {
        Bucket { level: UNITS * TICKS_PER_UNIT, at: None }
    }
}

impl<const UNITS: u32, const TICKS_PER_UNIT: u32> Bucket<UNITS, TICKS_PER_UNIT> {
    /// The level on server tick `now`, refilled since the last take.
    fn level_at(&self, now: u64) -> u32 {
        let cap = UNITS * TICKS_PER_UNIT;
        let ticks = self.at.map_or(0, |at| now.saturating_sub(at)).min(u64::from(cap)) as u32;
        self.level.saturating_add(ticks).min(cap)
    }

    /// Would the bucket pay `units` on server tick `now`? Takes nothing.
    pub fn can_take(&self, units: u32, now: u64) -> bool {
        units.saturating_mul(TICKS_PER_UNIT) <= self.level_at(now)
    }

    /// Pay `units` on server tick `now`, if the bucket holds them.
    pub fn try_take(&mut self, units: u32, now: u64) -> bool {
        self.level = self.level_at(now);
        self.at = Some(now);
        let cost = units.saturating_mul(TICKS_PER_UNIT);
        if cost > self.level {
            return false;
        }
        self.level -= cost;
        true
    }

    /// Whole units it holds now (before this tick's refill).
    #[cfg(test)]
    pub fn units(&self) -> u32 {
        self.level / TICKS_PER_UNIT
    }
}

/// C3b-fix-a (v78) — one joiner's containers on the server
/// (`ServerPlayer::container_sent`).
#[derive(Clone, Debug, Default)]
pub struct ContainerViews {
    /// The kind of the real container `ServerPlayer::open_container` names.
    pub open_kind: Option<ContainerKind>,
    /// C-M1 — the joiner's mirror as its client holds it after the last
    /// container op the server served and the views it reported applied:
    /// the opened container, every push and correction it applied, and its
    /// own predictions. A container op's prediction (P) is run on exactly
    /// this, after the views its `events_applied` reports are applied
    /// (`window_events::apply_through`). Cleared by the client's own close
    /// (in its order), never by the server's: its mirror stays until it
    /// closes it.
    pub seen: Option<MirrorView>,
    /// C-L3 — the believed-units bound.
    pub believed: BelievedBucket,
    /// C3c-2-fix (M2) — the believed-ammo bound (shots alone).
    pub believed_ammo: BelievedAmmo,
    /// C-L4 — the window event of the last refused op's revert: an op made
    /// before the client applied it ran on a window the server's copy never
    /// had (the prediction it undoes), which is not a lockstep mismatch.
    pub revert_event: u32,
}

/// What joiner `sp`'s mirror will show once every container view sent to
/// it lands: the model ([`ContainerViews::seen`]), then the views still
/// waiting, oldest first. Pushes and corrections are diffed against it.
pub fn latest_view(sp: &ServerPlayer) -> Option<MirrorView> {
    let mut view = sp.container_sent.seen.clone();
    for ev in crate::window_events::waiting_views(sp) {
        ev.apply(&mut view);
    }
    view
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
/// C3b-fix-a (A-L2) — a `Result` click (it reads the table verdict, where
/// the server's slack is kinder than the client's rule) is applied only when
/// the client's own rule accepted it (`WindowOpPacket::client_ok`): the
/// server never crafts what the client refused, so a stuck forced close
/// can't craft on one side only. C3b-fix-c (B-M4) — an `Autofill` the
/// client refused is not skipped: a refused Autofill has already returned
/// the grid and the cursor to the bag (step 1, `window::autofill`), so the
/// server runs the rule with its table grace off
/// (`ClickCtx::with_server_slack(false)`) and moves what the client's did.
/// The half-block reach slack stays (that toggle keeps it), so a client
/// that refused for reach within the slack still diverges (rare).
///
/// C3b-1 — `Close`, `OpenPlayer` and `OpenTable` also close the open
/// container (and the model of the client's mirror: the client's screen
/// closes with them); `OpenContainer` changes no window (the caller opens
/// the container); a container op is [`serve_container`].
pub fn serve_op(
    sp: &mut ServerPlayer,
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    creative: bool,
    pkt: &WindowOpPacket,
    now: u64,
    tables_gone: &mut TablesGone,
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
        let served = serve_container(sp, world, registry, &ctx.with_shared(true), click, pkt, now, creative);
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
            let result = match click {
                WindowClick::Result if !pkt.client_ok => ClickResult::Refused,
                // C3b-fix-c (B-M4) — a refused Autofill already moved the
                // grid and cursor back: the rule runs without the table
                // grace, so it moves what the client's did.
                WindowClick::Autofill { .. } if !pkt.client_ok => window::apply(&mut view, click, &ctx.clone().with_server_slack(false)),
                _ => window::apply(&mut view, click, &ctx),
            };
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
            // C3b-fix-b (A-L1) — the grace is for a table that WAS there: an
            // open at a cell that isn't a crafting table gets none (counted
            // past it from the start; `watch_table` only counts up).
            // C3b-fix-d (A-L5) — a table the server saw go within the grace
            // was there: the open gets what is left of it.
            sp.table_gone_ticks = if world.get_block(cell[0], cell[1], cell[2]) == crate::block::CRAFTING_TABLE {
                None
            } else {
                tables_gone.note(world, now);
                Some(tables_gone.gone_for(*cell, now).unwrap_or(window::SERVER_TABLE_GRACE_TICKS.saturating_add(1)))
            };
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
        sp.container_sent.open_kind = None;
        sp.container_sent.seen = None;
    }
    sp.station = station;
    sp.last_window_op_seq = pkt.op_seq;
    Some(Served::plain(result, digest))
}

/// Why a container op is refused before its rule runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// No container open on the server, or its cell gone.
    NotOpen,
    /// Beyond the server body's reach, with the slack.
    OutOfReach,
    /// The client's mirror shows another container (or none).
    OtherView,
    /// A claimed stack above its item's `max_stack()` (C-L3).
    OverStack,
}

/// C3b-1 (v77) / C3b-fix-a (v78) — joiner `sp`'s container op `click` on
/// the REAL container it has open, as a shared container (`ctx.shared`).
///
/// **P, the client's prediction:** the op over the player slots it claims
/// it held before it (`pkt.claims`, every other slot a blocker,
/// `container_window::ClaimedWindow`) and EXACTLY the view of the container
/// it predicted on ([`ContainerViews::seen`], C-M1: every container view is a
/// numbered window event, applied to the model up to the op's
/// `events_applied`).
///
/// Refused before the rule ([`refused_container_op`]) when no container is
/// open or its cell no longer holds it (then it is closed), it stands beyond
/// the server body's reach (`container_window::container_in_server_reach`),
/// the client's mirror shows another container, or a claim counts above its
/// stack (C-L3).
///
/// Otherwise:
/// - **R, the shared truth:** the op over the client's claims less the
///   phantom ledger (`window_events::phantom`: what corrections still on
///   their way take back — items the server already refused, never believed
///   again, C-H1) and the REAL container. A deposit of an item the server's
///   copy of the window doesn't hold is BELIEVED (`believed`), within the
///   joiner's bound ([`BelievedBucket`]; creative is unbounded): past it the
///   op is refused. BRIDGE: C3d refuses and corrects from the server's
///   window — replace when the flip lands (C3d). The same fabrication class
///   as a claimed Q-drop (Spec 04 §4.2e). C3b-fix-e (C-M1) — a believed tool
///   or armour piece the copy holds only at another durability, whose own
///   run deposits its piece of that kind in the same op, is a SWAP
///   ([`durability_swaps`]): the copy gives up its piece, charged and
///   tallied apart (`swapped`).
/// - **The server's own copy** (`ServerPlayer`'s window) applies the op as
///   usual over a copy of the view the client predicted on (own): in
///   lockstep exactly the client's prediction, so its digest — taken with
///   the container as the client's mirror shows it — compares lockstep only.
///
/// The correction, by item (C-H1): the client's change R − P, and the server
/// copy's R − own, both from what the container lost (the rules only move
/// items between the two); then every container slot the op involved
/// (either side's changes and the click's named slots) whose real value
/// differs from what the client's mirror will show
/// ([`latest_view`]), and a furnace's progress. When nothing reaches the
/// client, the server's copy applies its change at once. Never on a digest
/// mismatch alone.
#[allow(clippy::too_many_arguments)]
fn serve_container(
    sp: &mut ServerPlayer,
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    ctx: &ClickCtx,
    click: &ContainerClick,
    pkt: &WindowOpPacket,
    now: u64,
    creative: bool,
) -> Served {
    let eye = sp.player.eye_pos();
    let seen = sp.container_sent.seen.clone();
    let predicted = seen.as_ref().map(|v| {
        let mut w = ClaimedWindow::from_claims(&pkt.claims, registry);
        let mut after = v.contents.clone();
        let applied = w.apply(after.as_mut(), click, ctx);
        Predicted { window: w, after, applied }
    });
    let open = sp.open_container.zip(sp.container_sent.open_kind);
    let real_pre = open.and_then(|(cell, kind)| container_window::container_at(world, cell, kind)).map(ContainerData::of);
    if open.is_some() && real_pre.is_none() {
        sp.open_container = None;
        sp.container_sent.open_kind = None;
    }
    let refusal = match (open, &real_pre, &seen) {
        (None, ..) | (_, None, _) => Some(Refusal::NotOpen),
        (Some((cell, _)), ..) if !container_window::container_in_server_reach(eye, cell) => Some(Refusal::OutOfReach),
        (Some((cell, kind)), _, Some(v)) if v.cell != cell || v.kind != kind => Some(Refusal::OtherView),
        (_, _, None) => Some(Refusal::OtherView),
        _ if !container_window::claims_fit_stacks(&pkt.claims, registry) => Some(Refusal::OverStack),
        _ => None,
    };
    if let Some(why) = refusal {
        log::debug!("{}'s container op {} refused: {why:?}", sp.display_name, pkt.op_seq);
        return refused_container_op(sp, world, pkt, click, seen, predicted, 0);
    }
    let (Some((cell, kind)), Some(real_pre), Some(seen), Some(p)) = (open, real_pre, seen, predicted) else {
        // The checks above leave none of these empty.
        return refused_container_op(sp, world, pkt, click, None, None, 0);
    };
    // R — the claims less the phantom ledger, over a copy of the real
    // container first (a deposit past the believed bound is refused).
    let mut r_win = ClaimedWindow::from_claims(&pkt.claims, registry);
    for (item, n) in crate::window_events::phantom(sp) {
        r_win.debit(&item, n);
    }
    let mut r_after = real_pre.clone();
    let r = r_win.apply(r_after.as_mut(), click, ctx);
    let r_gain = container_window::player_gain(real_pre.as_ref(), r_after.as_ref());
    let believed_by_item = believed_units(sp, &r_gain);
    let believed: u32 = believed_by_item.iter().map(|(_, n)| *n).fold(0, u32::saturating_add);
    if believed > 0 && !creative && !sp.container_sent.believed.try_take(believed, now) {
        log::debug!("{}'s container op {} refused: {believed} believed unit(s) past the bound", sp.display_name, pkt.op_seq);
        return refused_container_op(sp, world, pkt, click, Some(seen), Some(p), believed);
    }
    if let Some(mut real) = container_window::container_at_mut(world, cell, kind) {
        real.replace_with(&r_after);
    }
    // The server's own copy, over the view the client predicted on.
    let mut own_after = seen.contents.clone();
    let own = {
        let mut view = WindowMut {
            inv: &mut sp.inventory,
            armour: &mut sp.armour,
            cursor: &mut sp.cursor,
            grid: &mut sp.craft_grid,
            container: Some(own_after.as_mut()),
        };
        container_window::apply_container(&mut view, click, ctx)
    };
    let p_gain = container_window::player_gain(seen.contents.as_ref(), p.after.as_ref());
    let own_gain = container_window::player_gain(seen.contents.as_ref(), own_after.as_ref());
    let hint = |item: &crate::item::Item| p.window.slot_holding(item).unwrap_or(0);
    let client = ItemDelta::from_counts(&container_window::counts_minus(&r_gain, &p_gain), hint);
    // The server's copy gives up what R put in, less the believed units it
    // never held (tallied, not owed).
    let believed_back: container_window::ItemCounts =
        believed_by_item.iter().map(|(item, n)| (item.clone(), -i64::from(*n))).collect();
    let own_net = container_window::counts_minus(&container_window::counts_minus(&r_gain, &own_gain), &believed_back);
    // C3b-fix-e (C-M1) — a believed tool or armour piece the copy holds only
    // at another durability, deposited by its own run in the same op, is a
    // swap: the copy gives up its own piece (the give-back no longer returns
    // it) and the container keeps the claimed one.
    let swaps = durability_swaps(&believed_by_item, &own_gain, &own_net);
    let swapped: u32 = swaps.iter().map(|(_, n)| u32::try_from(*n).unwrap_or(u32::MAX)).fold(0, u32::saturating_add);
    let own_net = container_window::counts_minus(&own_net, &swaps);
    let own_delta = ItemDelta::from_counts(&own_net, hint);
    // The model: the client's mirror after its own prediction.
    sp.container_sent.seen = Some(MirrorView { cell, kind, contents: p.after.clone() });
    let involved = involved_slots(pkt, click, [&r.touched, &p.applied.touched]);
    let view = view_correction(sp, r_after.as_ref(), &involved);
    let digest = window::digest_with(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station, Some(p.after.as_ref()));
    let correction = Correction { client, own: Some(own_delta), view };
    let correction = if correction.corrects_client() {
        Some(correction)
    } else {
        // Nothing reaches the client: its window is its prediction, so the
        // server's copy takes its own change now (none in lockstep).
        if let Some(own) = correction.own.as_ref().filter(|d| !d.is_empty()) {
            crate::window_events::apply_own_now(sp, own, now);
        }
        None
    };
    Served { result: Some(own.result), digest, container_refused: false, correction, believed: believed.saturating_sub(swapped), swapped, over_bound: 0 }
}

/// C3b-fix-e (C-M1) — the swaps in a container op. A tool or armour piece
/// the op put in BELIEVED (`believed`: claimed at a durability the server's
/// copy doesn't hold — worn on the client by a use not mirrored until C3c,
/// or a modified client's "repair") is paired, one unit for one, with a unit
/// of the same kind (`joiner_actions::same_item`) at another durability that
/// the copy's own run of the op gave up (`own_gain`'s negative counts) and
/// its give-back would return (`own_net`'s positive counts: R didn't put
/// that exact piece in). Each pair is a swap: the container keeps the claimed
/// piece, the copy gives up its own (the caller drops these units from the
/// give-back), so honest drift conserves — one hoe in the world, not two
/// (C3b-fix-c verify C-M1). A swap still costs the believed bound, so a
/// modified client's repair stays bounded; it is tallied
/// `durability_swap`, not `container_believed`. A believed unit with no
/// partner stays believed. Returns the copy's paired units, as positive
/// counts. Only a tool or armour piece can pair: for any other item a match
/// by kind is the same item.
fn durability_swaps(
    believed: &[(crate::item::Item, u32)],
    own_gain: &container_window::ItemCounts,
    own_net: &container_window::ItemCounts,
) -> container_window::ItemCounts {
    let mut partners: Vec<(crate::item::Item, i64)> = own_gain
        .iter()
        .filter(|(_, n)| *n < 0)
        .map(|(item, n)| {
            let back = own_net.iter().find(|(i, _)| i == item).map_or(0, |(_, m)| (*m).max(0));
            (item.clone(), n.abs().min(back))
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    let mut swaps: container_window::ItemCounts = Vec::new();
    for (item, n) in believed {
        let mut left = i64::from(*n);
        for (own, k) in partners.iter_mut() {
            if left == 0 {
                break;
            }
            if own != item && crate::joiner_actions::same_item(own, item) && *k > 0 {
                let m = left.min(*k);
                *k -= m;
                left -= m;
                match swaps.iter_mut().find(|(i, _)| i == own) {
                    Some((_, c)) => *c += m,
                    None => swaps.push((own.clone(), m)),
                }
            }
        }
    }
    swaps
}

/// The client's prediction of a container op, run by the server.
struct Predicted {
    /// The claimed window after it.
    window: ClaimedWindow,
    /// The container after it.
    after: ContainerData,
    applied: container_window::ContainerApplied,
}

/// The container slots a container op involved: the ones the client says
/// it touched (`pkt.touched`), the ones each run changed, and the ones the
/// click names.
fn involved_slots<const N: usize>(pkt: &WindowOpPacket, click: &ContainerClick, runs: [&Vec<WireWindowSlot>; N]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let named = container_window::named_slots(click);
    for at in pkt.touched.iter().chain(runs.into_iter().flatten()).chain(&named) {
        if let WireWindowSlot::Container(i) = *at
            && !out.contains(&usize::from(i))
        {
            out.push(usize::from(i));
        }
    }
    out
}

/// The container part of a correction: each of `involved` whose real value
/// (`real`) differs from what joiner `sp`'s mirror will show
/// ([`latest_view`]), at its real value, and with them a furnace's progress
/// if it differs. Progress alone earns no correction: the cook's push
/// carries it.
fn view_correction(sp: &ServerPlayer, real: container_window::ContainerRef, involved: &[usize]) -> ViewEvent {
    let latest = latest_view(sp);
    let shown = latest.as_ref().map(|v| v.contents.as_ref());
    let print = |c: Option<container_window::ContainerRef>, i: usize| c.map(|c| window::slot_print(c.get(i)));
    let sets: Vec<(usize, Option<ItemStack>)> = involved
        .iter()
        .copied()
        .filter(|&i| i < real.len() && print(Some(real), i) != print(shown, i))
        .map(|i| (i, real.get(i).cloned()))
        .collect();
    let furnace = real
        .furnace_view()
        .filter(|f| !sets.is_empty() && shown.and_then(|c| c.furnace_view()) != Some(*f));
    ViewEvent::Sets { sets, furnace }
}

/// Units a container op's R put into the real container (`r_gain`'s
/// negative counts) beyond what the server's copy of joiner `sp`'s window
/// holds of each item: believed deposits, item by item (none listed at 0).
///
/// C3b-fix-c (B-M3) — `held` counts only what the own take can pay: exactly
/// the correction take's places (`joiner_actions::take_correction`: the 36
/// slots, the grid, the cursor and the armour slots), in the window as it
/// will be once its waiting events land (`window_events::effective_window`:
/// a waiting correction's take is already off it, so a phantom never counts
/// as held), and by exact identity (`==`): a tool or armour piece counts only
/// at its own durability, so a claimed piece unlike any the copy holds is
/// believed (bounded, tallied) instead of paid with a different one — or,
/// when the copy's own run deposited its own piece of that kind in the same
/// op, swapped for it (C3b-fix-e, [`durability_swaps`]).
fn believed_units(sp: &ServerPlayer, r_gain: &container_window::ItemCounts) -> Vec<(crate::item::Item, u32)> {
    let w = crate::window_events::effective_window(sp);
    r_gain
        .iter()
        .filter(|(_, n)| *n < 0)
        .map(|(item, n)| (item.clone(), n.unsigned_abs().saturating_sub(held_units(&w, item)).min(u64::from(u32::MAX)) as u32))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// Units of `item` window `w` holds by exact identity (`==`) in the places
/// an owed take can pay from: the 36 slots, the grid, the cursor and the
/// armour slots ([`believed_units`], [`believe_pay`]).
fn held_units(w: &crate::window_events::EffectiveWindow, item: &crate::item::Item) -> u64 {
    let of = |s: Option<&ItemStack>| s.filter(|s| &s.item == item).map_or(0, |s| u64::from(s.count));
    let inv: u64 = w.inv.slots_iter().map(of).sum();
    let grid: u64 = w.grid.iter().flatten().map(|c| of(c.as_ref())).sum();
    let armour = w.armour.iter().flatten().filter(|p| &crate::item::Item::Armour(**p) == item).count() as u64;
    inv + grid + of(w.cursor.as_ref()) + armour
}

/// C3b-2-fix (M2) — joiner `sp`'s request pays `n` of `item` (an accepted
/// block use's take, `HostedServer::serve_block_use`). The part the server's
/// copy of its window can't cover — counted as [`believed_units`] counts: in
/// the window as it will be once its waiting events land
/// (`window_events::effective_window`, so an earlier request's pending take
/// is already off it), by exact identity — is BELIEVED, charged to the same
/// per-joiner bound as believed container deposits ([`BelievedBucket`]).
/// `Ok(believed)` (0 when the copy covers it all); `Err(short)` when the
/// bound can't pay the `short` units: the caller refuses the request.
/// BRIDGE: C3d refuses a pay the server's window can't cover — replace when
/// the flip lands (C3d gate list, design doc).
pub fn believe_pay(sp: &mut ServerPlayer, item: &crate::item::Item, n: u32, now: u64) -> Result<u32, u32> {
    let short = pay_short(sp, item, n);
    if short == 0 {
        return Ok(0);
    }
    if sp.container_sent.believed.try_take(short, now) {
        Ok(short)
    } else {
        Err(short)
    }
}

/// C3c-2-fix (M2) — of `n` of `item` joiner `sp`'s request pays, the units
/// the server's copy of its window can't cover ([`believe_pay`]'s count;
/// nothing is charged).
pub fn pay_short(sp: &ServerPlayer, item: &crate::item::Item, n: u32) -> u32 {
    let w = crate::window_events::effective_window(sp);
    u64::from(n).saturating_sub(held_units(&w, item)).min(u64::from(u32::MAX)) as u32
}

/// C3c-2-fix (M2) — 1 when the server's copy of joiner `sp`'s window holds no
/// tool of `tool`'s type and material ([`believe_wear`]'s test; nothing is
/// charged), else 0.
pub fn wear_short(sp: &ServerPlayer, tool: &crate::crafting::Tool) -> u32 {
    let w = crate::window_events::effective_window(sp);
    let like = |s: Option<&ItemStack>| {
        matches!(s.map(|s| &s.item), Some(crate::item::Item::Tool(t)) if t.tool_type == tool.tool_type && t.material == tool.material)
    };
    let held = w.inv.slots_iter().any(like) || w.grid.iter().flatten().any(|c| like(c.as_ref())) || like(w.cursor.as_ref());
    u32::from(!held)
}

/// C3c-1-fix (L-3) — joiner `sp`'s accepted block use wears `tool` and takes
/// nothing (`block_use::Used` with `pay` 0 and `wear`: shears on a hive). A
/// tool the server's copy of its window doesn't hold — no tool of that type
/// and material anywhere an owed take looks (the 36 slots, the grid, the
/// cursor), in the window as it will be once its waiting events land
/// (`window_events::effective_window`) — is BELIEVED: 1 charged to the same
/// per-joiner bound as believed deposits and block-use pays
/// ([`BelievedBucket`]). By type and material, not exact identity: a tool is
/// the same tool by those (`joiner_inventory::wear_tool`), and its durability
/// drifts honestly. `Ok(believed)` (0 when the copy holds one); `Err(1)` past
/// the bound: the caller refuses the use. BRIDGE: C3d refuses a use with a
/// tool the server's window doesn't hold — replace when the flip lands.
pub fn believe_wear(sp: &mut ServerPlayer, tool: &crate::crafting::Tool, now: u64) -> Result<u32, u32> {
    if wear_short(sp, tool) == 0 {
        return Ok(0);
    }
    if sp.container_sent.believed.try_take(1, now) {
        Ok(1)
    } else {
        Err(1)
    }
}

/// C3b-1 / C3b-fix-a (C-M2, C-L1) — a container op refused before its rule
/// ran. The server's copy never moved, and neither did the real container.
/// The client's prediction (P, over the view it predicted on) is undone by
/// item — a numbered event with NO server-side effect (`own: None`) — and
/// the container slots it touched and named go back to their real values
/// where its mirror would show otherwise, if that container still exists.
/// With no view to predict on (no container was ever opened for it), there
/// is nothing to undo: a client can't hold a mirror the server never sent.
/// `over_bound`: the believed units that refused it, if that was why.
fn refused_container_op(
    sp: &mut ServerPlayer,
    world: &World,
    pkt: &WindowOpPacket,
    click: &ContainerClick,
    seen: Option<MirrorView>,
    predicted: Option<Predicted>,
    over_bound: u32,
) -> Served {
    let mut correction = None;
    if let (Some(v), Some(p)) = (seen, predicted) {
        let undo: container_window::ItemCounts =
            container_window::player_gain(v.contents.as_ref(), p.after.as_ref()).into_iter().map(|(item, n)| (item, -n)).collect();
        let client = ItemDelta::from_counts(&undo, |item| p.window.slot_holding(item).unwrap_or(0));
        sp.container_sent.seen = Some(MirrorView { contents: p.after.clone(), ..v.clone() });
        let view = match container_window::container_at(world, v.cell, v.kind) {
            Some(real) => view_correction(sp, real, &involved_slots(pkt, click, [&p.applied.touched])),
            None => ViewEvent::Sets { sets: Vec::new(), furnace: None },
        };
        let c = Correction { client, own: None, view };
        correction = c.corrects_client().then_some(c);
    }
    let shown = sp.container_sent.seen.as_ref().map(|v| v.contents.as_ref());
    let digest = window::digest_with(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station, shown);
    Served { result: Some(ClickResult::Refused), digest, container_refused: true, correction, believed: 0, swapped: 0, over_bound }
}

/// C3b-1 / C3b-fix-a — what changed in joiner `sp`'s open container beyond
/// what its mirror will show once every view sent lands ([`latest_view`]):
/// the container slots whose contents differ, and a furnace's progress if it
/// moved, as a `WindowSlotSet { Changed }` — v78: a numbered window event
/// (`window_events::WindowEvent::ContainerView`), queued here, so the
/// client's next op names the view it predicted on (C-M1). This is what
/// everyone but the joiner's own ops did: another player, a hopper, the
/// furnace cooking, a host's click on its lent world. `None` when nothing
/// changed. A container whose cell no longer holds it is closed on the
/// server (the client closes its screen by the same rule, seeing the block
/// go). `now`: the server tick.
pub fn container_push(sp: &mut ServerPlayer, world: &World, now: u64) -> Option<WindowSlotSetPacket> {
    let cell = sp.open_container?;
    let kind = sp.container_sent.open_kind?;
    let Some(c) = container_window::container_at(world, cell, kind) else {
        sp.open_container = None;
        sp.container_sent.open_kind = None;
        return None;
    };
    let latest = latest_view(sp).filter(|v| v.cell == cell && v.kind == kind)?;
    let shown = latest.contents.as_ref();
    let sets: Vec<(usize, Option<ItemStack>)> = (0..c.len())
        .filter(|&i| window::slot_print(c.get(i)) != window::slot_print(shown.get(i)))
        .map(|i| (i, c.get(i).cloned()))
        .collect();
    let furnace = c.furnace_view().filter(|f| shown.furnace_view() != Some(*f));
    if sets.is_empty() && furnace.is_none() {
        return None;
    }
    let wire = wire_sets(&sets);
    let event = crate::window_events::queue(
        sp,
        crate::window_events::WindowEvent::ContainerView(ViewEvent::Sets { sets, furnace }),
        now,
    );
    Some(WindowSlotSetPacket {
        op_seq_applied: sp.last_window_op_seq,
        reason: crate::protocol::slot_set_reason::CHANGED,
        sets: wire,
        furnace,
        window_event: event,
        take: Vec::new(),
        give: Vec::new(),
    })
}

/// C3b-fix-d (A-L5) — most cells a server's [`TablesGone`] remembers; past
/// it the oldest is forgotten.
pub const MAX_TABLES_GONE: usize = 256;

/// C3b-fix-d (A-L5) — a server's memory of the cells that stopped being a
/// crafting table within the last [`window::SERVER_TABLE_GRACE_TICKS`], each
/// with the tick it went (`GameServer::tables_gone`): taken in from its
/// world's log (`World::take_tables_gone`, every table removal on the world
/// the server runs on, whoever made it) once a tick and at each `OpenTable`
/// ([`Self::note`]), pruned as they age, at most [`MAX_TABLES_GONE`]. An
/// `OpenTable` at one of them gets the grace that is left ([`serve_op`]):
/// the joiner's world, a few ticks behind, still showed the table another
/// player broke. Any other cell that isn't a table gets none.
#[derive(Debug, Default)]
pub struct TablesGone {
    /// Oldest first.
    cells: std::collections::VecDeque<([i32; 3], u64)>,
    /// Has this server taken its world's log in yet? The first take only
    /// drops what is there: a lending host's world keeps its log on after a
    /// server stops (nothing turns it off), so it can hold tables broken in
    /// solo play long before this server started.
    started: bool,
}

impl TablesGone {
    /// Take in the cells `world` logged since the last call as gone at tick
    /// `now` (the first time, turn its log on and drop what it already
    /// held), and forget every cell past the grace.
    pub fn note(&mut self, world: &mut World, now: u64) {
        world.track_tables_gone();
        let logged = world.take_tables_gone();
        if !std::mem::replace(&mut self.started, true) {
            return;
        }
        for (x, y, z) in logged {
            if self.cells.len() >= MAX_TABLES_GONE {
                self.cells.pop_front();
            }
            self.cells.push_back(([x, y, z], now));
        }
        let grace = u64::from(window::SERVER_TABLE_GRACE_TICKS);
        while self.cells.front().is_some_and(|&(_, at)| now.saturating_sub(at) > grace) {
            self.cells.pop_front();
        }
    }

    /// How many ticks before `now` the table at `cell` went (the last time),
    /// if that is inside the grace.
    pub fn gone_for(&self, cell: [i32; 3], now: u64) -> Option<u8> {
        let ago = self.cells.iter().rev().find(|(c, _)| *c == cell).map(|&(_, at)| now.saturating_sub(at))?;
        u8::try_from(ago).ok().filter(|&n| n <= window::SERVER_TABLE_GRACE_TICKS)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cells.len()
    }
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
/// no-op when the client's own rule refused too, a refusal when it didn't)
/// and a digest that differs from the client's is a mismatch (the first
/// one's kind is kept, and it is logged at info; the rest at debug).
/// Log-only.
///
/// C3a-fix-1 (decision 2) — the first comparison after join is not tallied:
/// it is recorded as the baseline (`WindowEvents::baseline`), the window the
/// joiner arrived with, which no wire carries yet (the sidecar's join sync
/// will).
///
/// C3b-fix-a — `window_mismatch` is per-joiner lockstep only, the C3d gate
/// (C-L4): a container op refused before its rule (tallied
/// `container_refused` by the caller), and an op made before the client
/// applied a refused op's revert ([`ContainerViews::revert_event`]: it ran
/// on a prediction the server's copy never made), are container
/// convergence, not mismatches. A-L3: no-op versus refusal splits on the
/// client's own verdict (`client_ok`), not on the digests, which drift
/// elsewhere would split wrongly.
pub fn note_served(sp: &mut ServerPlayer, pkt: &WindowOpPacket, served: &Served, creative: bool) {
    let tally = &mut sp.possession;
    tally.window_ops = tally.window_ops.saturating_add(1);
    if creative || served.container_refused {
        return;
    }
    if served.refused() {
        // B-L4: refused on both sides is a benign no-op; refused here alone
        // is not.
        if pkt.client_ok {
            tally.window_refused = tally.window_refused.saturating_add(1);
        } else {
            tally.window_noop = tally.window_noop.saturating_add(1);
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
    if matched || sp.container_sent.revert_event > pkt.events_applied {
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
        log.record(WireWindowOp::OpenPlayer, 11, true);
        log.sync_auto_refill(true, || panic!("unchanged: no digest is read"));
        log.record(WireWindowOp::Click(WindowClick::Sort), 12, true);
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
        log.record(WireWindowOp::OpenPlayer, 1, true);
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
        log.record(WireWindowOp::Click(WindowClick::Close), 1, true);
        edits.push(BlockChange { x: 1, y: 2, z: 3, new_block: 4, meta: 0 });
        log.record(WireWindowOp::OpenPlayer, 2, true);
        edits.push(BlockChange { x: 5, y: 2, z: 3, new_block: 4, meta: 0 });
        log.record(WireWindowOp::Click(WindowClick::Sort), 3, true);
        let pairs = |ops: Vec<LoggedOp>| ops.into_iter().map(|l| (l.op, l.digest)).collect::<Vec<_>>();
        assert_eq!(pairs(log.take_before(edits.first_stamp())), vec![(WireWindowOp::Click(WindowClick::Close), 1)]);
        assert_eq!(pairs(log.take_before(edits.first_stamp())), vec![], "the rest wait for the input");
        let (sent, _, stamps, _) = edits.take((0, 0, 0));
        assert_eq!(sent.len(), 2);
        assert!(stamps[0] < stamps[1], "each edit keeps its own stamp (C3b-fix-d, A-L3)");
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
        let (sent, hands, _, _) = edits.take((8, 1, 3));
        assert_eq!(sent.iter().map(|b| b.x).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert_eq!(hands, vec![(2, 1, 7), (2, 1, 7), (5, 4, 9), (8, 1, 3)]);
        assert!(edits.is_empty() && edits.take((0, 0, 0)).1.is_empty());
        edits.push(edit(9));
        edits.clear();
        assert_eq!((edits.len(), edits.first_stamp()), (0, None));
    }

    /// C3b-fix-d (A-L5) — a world a server runs on logs every crafting table
    /// that goes (and nothing else); the server remembers each for the grace,
    /// then forgets it, and never holds more than its cap.
    #[test]
    fn a_table_that_goes_is_remembered_for_the_grace_then_forgotten() {
        use crate::block::{AIR, CRAFTING_TABLE, DIRT, STONE};
        let mut world = World::new();
        world.set_block(1, 70, 1, CRAFTING_TABLE);
        world.set_block(1, 70, 1, AIR);
        assert!(world.take_tables_gone().is_empty(), "not tracking: nothing logged");
        let mut gone = TablesGone::default();
        gone.note(&mut world, 100);
        world.set_block(2, 70, 2, CRAFTING_TABLE);
        world.set_block(2, 70, 2, STONE);
        world.set_block(3, 70, 3, DIRT);
        world.set_block(3, 70, 3, AIR);
        gone.note(&mut world, 101);
        assert_eq!(gone.gone_for([2, 70, 2], 101), Some(0), "taken in as gone now");
        assert_eq!(gone.gone_for([2, 70, 2], 104), Some(3));
        assert_eq!(gone.gone_for([3, 70, 3], 104), None, "never a table");
        assert_eq!(gone.gone_for([1, 70, 1], 104), None, "went before the server tracked the world");
        let grace = u64::from(window::SERVER_TABLE_GRACE_TICKS);
        assert_eq!(gone.gone_for([2, 70, 2], 101 + grace), Some(window::SERVER_TABLE_GRACE_TICKS));
        assert_eq!(gone.gone_for([2, 70, 2], 102 + grace), None, "past the grace");
        gone.note(&mut world, 102 + grace);
        assert_eq!(gone.len(), 0, "pruned as it ages");
        let n = MAX_TABLES_GONE as i32 + 10;
        for x in 0..n {
            world.set_block(x, 71, 0, CRAFTING_TABLE);
            world.set_block(x, 71, 0, AIR);
        }
        gone.note(&mut world, 200);
        assert_eq!(gone.len(), MAX_TABLES_GONE, "bounded");
        assert_eq!(gone.gone_for([n - 1, 71, 0], 200), Some(0), "the newest kept");
        assert_eq!(gone.gone_for([0, 71, 0], 200), None, "the oldest dropped");
        // A new server on a world whose log stayed on (a lending host that
        // stopped, played solo, and hosts again): what was logged before it
        // started is not taken in as gone now.
        world.set_block(5, 70, 5, CRAFTING_TABLE);
        world.set_block(5, 70, 5, AIR);
        let mut fresh = TablesGone::default();
        fresh.note(&mut world, 1);
        assert_eq!(fresh.gone_for([5, 70, 5], 1), None, "broken before this server started");
        assert_eq!(fresh.len(), 0);
    }

    /// C3b-fix-a (C-L3) — the believed bucket: 64 units deep, 4 a second
    /// back, all or nothing per op.
    #[test]
    fn the_believed_bucket_holds_sixty_four_and_refills_four_a_second() {
        let mut b = BelievedBucket::default();
        assert_eq!(b.units(), BELIEVED_BUCKET_UNITS);
        assert!(b.try_take(60, 100));
        assert!(!b.try_take(5, 100), "4 left: 5 is refused whole");
        assert!(b.try_take(4, 100));
        assert!(!b.try_take(1, 104), "4 ticks refill 0.8 of a unit");
        assert!(b.try_take(1, 105), "5 ticks, one unit");
        assert!(b.try_take(4, 125), "a second, four");
        assert!(b.try_take(64, 10_000), "full again, never past full");
        assert!(!b.try_take(1, 10_000));
    }

    /// C3c-2-fix (M2) — the believed-ammo bucket: 16 deep, one unit back
    /// every 80 ticks (4 s); `can_take` looks without taking.
    #[test]
    fn the_believed_ammo_bucket_holds_sixteen_and_refills_one_every_four_seconds() {
        let mut b = BelievedAmmo::default();
        assert_eq!(b.units(), BELIEVED_AMMO_UNITS);
        assert!(b.try_take(16, 100));
        assert!(!b.can_take(1, 179), "79 ticks: not yet");
        assert!(b.can_take(1, 180) && b.can_take(1, 180), "looking takes nothing");
        assert!(b.try_take(1, 180));
        assert!(!b.try_take(1, 180));
        assert!(b.try_take(16, 180 + 16 * 80));
        assert!(!b.can_take(1, 180 + 16 * 80));
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
