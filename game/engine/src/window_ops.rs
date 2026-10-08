//! C3a-2a (protocol v75) — the server mirrors a joiner's inventory window,
//! click for click (`docs/foundations/2026-10-07-c3-server-owned-inventory.md`
//! §2; Spec 04 §4.2g).
//!
//! **Client.** Every window transition the inventory screen applies — each
//! click, the close included (`CraftingUi::apply_click`, `CraftingUi::close`),
//! and each open — is logged in an [`OpLog`] with the window's digest after
//! it (`window::digest`). A joined client sends the log as `WindowOp`
//! packets at the start of every tick, before that tick's input
//! (`GameState::flush_window_ops`), so the server reads them in the order
//! the client made them relative to its edits. Its auto-refill setting is an
//! op too, logged at join and whenever it changes, ahead of the op it first
//! applies to. Single-player and a host's own slots send nothing.
//!
//! **Server.** [`serve_op`] applies the op to its copy of that joiner's
//! window (`ServerPlayer`'s 36 slots, armour, cursor, craft grid and
//! station) by the same rule, `window::apply`, judging a table's reach from
//! the server's body in the server's world. [`note_served`] compares the
//! digests and tallies (`PossessionTally`). It is log-only: nothing is
//! refused, nothing is sent back. A rule refusal leaves the server's window
//! as the rule leaves it, exactly as on the client.

use crate::protocol::{WindowOpPacket, WireWindowOp};
use crate::server::ServerPlayer;
use crate::window::{self, ClickCtx, ClickResult, Station, WindowClick, WindowMut};
use crate::world::World;

/// A client's window ops waiting to be sent, oldest first, each with the
/// window's digest after it.
#[derive(Debug, Default)]
pub struct OpLog {
    pending: Vec<(WireWindowOp, u32)>,
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
            self.pending.push((WireWindowOp::SetAutoRefill { on }, digest()));
        }
    }

    /// Log an op just applied, with the window's digest after it.
    pub fn record(&mut self, op: WireWindowOp, digest: u32) {
        self.pending.push((op, digest));
    }

    /// Everything logged, oldest first, for a joined client to send. A
    /// change of the auto-refill setting (now `auto_refill`) since the last
    /// op goes last, with the window's digest now (`digest_now`).
    pub fn take(&mut self, auto_refill: bool, digest_now: u32) -> Vec<(WireWindowOp, u32)> {
        self.sync_auto_refill(auto_refill, || digest_now);
        std::mem::take(&mut self.pending)
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
            if *click == WindowClick::Close && result.ok() {
                station = Station::Player;
            }
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
    let digest = window::digest(&view);
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
    if served.digest == pkt.digest {
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
