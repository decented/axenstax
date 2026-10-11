//! C3d — Strict window configuration and shared containers through reserved
//! actions (Decision 10; `docs/foundations/2026-10-09-c3d-admission-checkpoint.md`
//! "Shared containers").
//!
//! - **Configuration** (`OpenPlayer`, `OpenTable`, `SetAutoRefill`): staged on
//!   a marker copy of raw R at the action's boundary; the digest must match;
//!   result row before commit. `OpenPlayer`/`OpenTable` close the container
//!   view, as in Mirror (`window_ops::serve_op`).
//! - **Open** (`OpenContainer`): the shared verdict
//!   (`HostedServer::container_open_verdict`). An opened view is a numbered
//!   window event whose count and owned bytes are reserved before anything is
//!   opened or sent. A refused open is the request's answer (`OpenRefusal`),
//!   not a refused action: the client predicted no change and none happens.
//!   A view that doesn't fit the event budget is one too (`TooLarge`,
//!   run 5); a container's Plans count as their placeholders.
//! - **Container click**: run twice on raw R — once on EXACTLY the view the
//!   client predicted on (`ContainerViews::seen` at its boundary) to check its
//!   digest, once on the REAL container to commit. No claims, no believed
//!   deposit, no durability swap, no fabricated view stock. Same effect →
//!   both are committed once. A contested slot (the real container differs
//!   in a slot the click acts on) → terminal `NormalisedAcceptedNoop`,
//!   nothing committed, then the correction (the bridge teardown until
//!   recovery is wired). Whole-or-nothing: a partial contest (fewer units
//!   than predicted) is a contest too.
//! - **Pushes** to a Strict joiner reserve their numbered view first; at a
//!   cap the session fails closed (bridge).

use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::container_window::{self as cw, ContainerData};
use crate::strict_admission::StrictTeardown;
use crate::window_events::{StrictStageError, WindowEvent};
use crate::window_ops::{MirrorView, ViewEvent};

/// Whether the click had the SAME effect on the real container as on the
/// view the client predicted on: every slot either run changed held, before
/// it, what the client saw, and holds after it what the client predicted.
fn same_effect(seen: &ContainerData, predicted: &ContainerData, real_before: &ContainerData, real_after: &ContainerData) -> bool {
    let (s, p, q, r) = (seen.as_ref(), predicted.as_ref(), real_before.as_ref(), real_after.as_ref());
    if s.len() != q.len() {
        return false;
    }
    (0..s.len()).all(|i| {
        let print = |c: cw::ContainerRef, i| crate::window::slot_print(c.get(i));
        let moved = print(s, i) != print(p, i) || print(q, i) != print(r, i);
        !moved || (print(s, i) == print(q, i) && print(p, i) == print(r, i))
    })
}

impl HostedServer {
    /// A Strict window op that isn't a personal-window click.
    pub(super) fn serve_strict_window_op(&mut self, i: usize, action: ActionHeader, op: &protocol::WindowOpPacket) -> bool {
        match &op.op {
            protocol::WireWindowOp::OpenContainer { cell } => self.serve_strict_open(i, action, op, *cell),
            protocol::WireWindowOp::Container(click) => self.serve_strict_container_click(i, action, op, click),
            protocol::WireWindowOp::OpenPlayer | protocol::WireWindowOp::OpenTable { .. } | protocol::WireWindowOp::SetAutoRefill { .. } => {
                self.serve_strict_config(i, action, op)
            }
            protocol::WireWindowOp::Click(_) => {
                self.pause_typed_action(i, StrictTeardown::Unreadable, "a click reached the configuration arm");
                false
            }
        }
    }

    /// Raw R as marker scratch, or the typed refusal / pause it calls for.
    /// `correctable`: a configuration refusal is corrected by a `Replace`
    /// (C3d run 12, S1; `SetAutoRefill` since run 14); a container one tears
    /// down.
    fn strict_raw_or_stop(&mut self, i: usize, action: ActionHeader, path: &str, correctable: bool) -> Option<crate::staged_window::MarkerWindow> {
        match crate::window_events::strict_raw_marker(&self.server.players[i], &self.server.registry) {
            Ok(raw) => Some(raw),
            Err(StrictStageError::Debt) => { self.strict_debt_at_staging(i, path); None }
            Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); None }
            Err(StrictStageError::Unrepresentable) => {
                if correctable { self.refuse_typed_action(i, action) } else { self.refuse_typed_action_then_teardown(i, action) }
                None
            }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, path); None }
        }
    }

    fn serve_strict_config(&mut self, i: usize, action: ActionHeader, op: &protocol::WindowOpPacket) -> bool {
        let Some(mut raw) = self.strict_raw_or_stop(i, action, "window configuration", true) else { return false };
        let now = self.server.tick_counter;
        let sp = &self.server.players[i];
        let (mut station, mut gone, mut refill) = (sp.station, sp.table_gone_ticks, sp.inventory.auto_refill);
        match op.op {
            protocol::WireWindowOp::OpenPlayer => { station = crate::window::Station::Player; gone = None; }
            protocol::WireWindowOp::OpenTable { cell } => {
                station = crate::window::Station::Table { cell };
                gone = crate::window_ops::open_table_grace(&mut self.server.world, cell, now, &mut self.server.tables_gone);
            }
            protocol::WireWindowOp::SetAutoRefill { on } => refill = on,
            _ => unreachable!("only configuration ops reach here"),
        }
        raw.set_auto_refill(refill);
        // Run 4 (run-3 review M1) — configuration touches no contents, so it
        // is staged independently of a waiting join/death replacement: the
        // client's window at this boundary is about to be replaced wholesale,
        // so its contents digest is not compared (no Replacement uncertainty).
        // Run 6 (run-5 review LOW-2): skipped exactly for an op made before
        // the latest `Replace`, not whenever one waits.
        let stale = self.server.players[i].window_events.predates_replace(action.events_applied);
        if !stale && raw.digest_with(station, None) != op.digest {
            // C3d run 12 (S1): a `Replace` carries the station. Run 14: a
            // refused `SetAutoRefill` is corrected too. The `Replace` keeps
            // each copy's refill setting, so the client, seeing its setting's
            // `Refused` receipt, sends it again on the corrected window
            // (`RemoteClient::refill_refused`).
            self.refuse_typed_action(i, action);
            return false;
        }
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "configuration result history is full");
            return false;
        };
        let sp = &mut self.server.players[i];
        sp.station = station;
        sp.table_gone_ticks = gone;
        sp.inventory.auto_refill = refill;
        if crate::window_ops::op_closes_container_view(&op.op) {
            crate::window_ops::close_container_view(sp);
        }
        sp.last_window_op_seq = op.op_seq;
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.strict_fold_holds(i, "window configuration")
    }

    /// A refused open: the request's answer (`OpenRefusal`) on an Accepted
    /// row with no effect — the client predicted no change and none happens.
    fn answer_refused_open(&mut self, i: usize, action: ActionHeader, op_seq: u32, cell: [i32; 3], why: protocol::OpenRefusal) -> bool {
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "container open result history is full");
            return false;
        };
        self.server.players[i].last_window_op_seq = op_seq;
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        let kind = cw::ContainerKind::of_block(self.server.world.get_block(cell[0], cell[1], cell[2])).unwrap_or(cw::ContainerKind::Dispenser);
        let pkt = protocol::ContainerOpenedPacket { cell, kind, slots: Vec::new(), furnace: None, refused: Some(why), window_event: 0 };
        self.send_to_joined_slot(i, &protocol::serialize_packet(protocol::PacketType::ContainerOpened, &pkt));
        self.strict_fold_holds(i, "container open")
    }

    fn serve_strict_open(&mut self, i: usize, action: ActionHeader, op: &protocol::WindowOpPacket, cell: [i32; 3]) -> bool {
        // (An open while dead or on a base before a waiting `Replace` never
        // gets here: it is `Superseded`, `actions.rs`.)
        let Some(raw) = self.strict_raw_or_stop(i, action, "container open", false) else { return false };
        // Nothing opens until the server answers: the client's digest is
        // its window unchanged.
        if raw.digest_with(self.server.players[i].station, None) != op.digest {
            self.refuse_typed_action_then_teardown(i, action);
            return false;
        }
        let now = self.server.tick_counter;
        let kind = match self.container_open_verdict(i, cell) {
            Ok(kind) => kind,
            Err(why) => return self.answer_refused_open(i, action, op.op_seq, cell, why),
        };
        // Run 11 (run-7 item 4) — the view is read without conjuring the
        // block entity: a missing one (a lazy chest) views as the fresh one
        // `ensure_container` would create, and it is created only once the
        // view and the result row are reserved, so a refused open
        // (`TooLarge`) leaves the world exactly as it was.
        let fresh;
        let view = match cw::container_at(&self.server.world, cell, kind) {
            Some(view) => view,
            None => {
                fresh = cw::FreshContainer::of(kind);
                fresh.view()
            }
        };
        // Run 5 (run-4 review LOW-2) — the view models what the client's
        // mirror holds: a Plan as the body-less placeholder (Plans stay in
        // a shared container and digest content-free), so a host's Plan
        // costs the view its placeholder's bytes, not its body's.
        let (slots, furnace, contents) = (view.wire_slots(), view.furnace_view(), ContainerData::of(view).without_plan_bodies());
        let opened = ViewEvent::Opened(MirrorView { cell, kind, contents });
        // The numbered view (count AND owned bytes) and the result row are
        // reserved before the container opens or anything is sent. A view
        // that doesn't fit the budget is the open's typed refusal, never a
        // teardown: nothing opened.
        let Ok(prepared) = crate::window_events::prepare_strict_events(&mut self.server.players[i], vec![WindowEvent::ContainerView(opened)].into_boxed_slice()) else {
            return self.answer_refused_open(i, action, op.op_seq, cell, protocol::OpenRefusal::TooLarge);
        };
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, prepared.last_number()) else {
            prepared.abandon(&mut self.server.players[i]);
            self.pause_typed_action(i, StrictTeardown::FellBehind, "container open result history is full");
            return false;
        };
        cw::ensure_container(&mut self.server.world, cell, kind);
        let sp = &mut self.server.players[i];
        sp.open_container = Some(cell);
        sp.container_sent.open_kind = Some(kind);
        sp.last_window_op_seq = op.op_seq;
        let window_event = prepared.commit(sp, now);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        let pkt = protocol::ContainerOpenedPacket { cell, kind, slots, furnace, refused: None, window_event };
        self.send_to_joined_slot(i, &protocol::serialize_packet(protocol::PacketType::ContainerOpened, &pkt));
        self.strict_fold_holds(i, "container open")
    }

    fn serve_strict_container_click(&mut self, i: usize, action: ActionHeader, op: &protocol::WindowOpPacket, click: &cw::ContainerClick) -> bool {
        let creative = self.server.play_mode.is_creative();
        let sp = &self.server.players[i];
        let open = sp.open_container.zip(sp.container_sent.open_kind);
        let real_pre = open.and_then(|(cell, kind)| cw::container_at(&self.server.world, cell, kind)).map(ContainerData::of);
        let seen = sp.container_sent.seen.clone();
        // Stale world (no container open, its cell gone, out of reach, the
        // client's mirror shows another): a terminal refusal (Drift).
        let (Some((cell, kind)), Some(real_pre), Some(seen)) = (open, real_pre, seen) else {
            self.refuse_typed_action_then_teardown(i, action);
            return false;
        };
        if seen.cell != cell || seen.kind != kind || !cw::container_in_server_reach(sp.player.eye_pos(), cell) {
            self.refuse_typed_action_then_teardown(i, action);
            return false;
        }
        let Some(raw) = self.strict_raw_or_stop(i, action, "container click", false) else { return false };
        let sp = &self.server.players[i];
        let ctx = crate::window_ops::server_click_ctx(sp, &self.server.world, creative).with_shared(true);
        // P — the client's prediction, on exactly the view it predicted on.
        let mut predicted = seen.contents.clone();
        let p = crate::staged_window::stage_container(&raw, &mut predicted, click, &ctx);
        if p.after.digest_with(sp.station, Some(predicted.as_ref())) != op.digest {
            self.refuse_typed_action_then_teardown(i, action);
            return false;
        }
        // R — the same click on the REAL container.
        let mut real_after = real_pre.clone();
        let r = crate::staged_window::stage_container(&raw, &mut real_after, click, &ctx);
        if !same_effect(&seen.contents, &predicted, &real_pre, &real_after) || p.after.slots() != r.after.slots() {
            // Contested: another player (or a machine) won these units.
            // Nothing is committed for this action.
            self.terminal_then_correct(i, action, ActionFate::NormalisedAcceptedNoop, "contested shared container");
            return false;
        }
        match crate::window_events::admit_staged(sp, &self.server.registry, &raw, &r) {
            Ok(()) => {}
            Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "container click"),
            Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(StrictStageError::Unrepresentable) => { self.refuse_typed_action_then_teardown(i, action); return false; }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "container source or pending fit is uncertain"); return false; }
        }
        let included = sp.window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "container result history is full");
            return false;
        };
        // Commit both once: the real container and the raw personal window.
        let Some(mut real) = cw::container_at_mut(&mut self.server.world, cell, kind) else {
            unreachable!("the real container was read this action")
        };
        real.replace_with(&real_after);
        let (inv, armour, cursor, grid) = r.after.into_parts();
        let sp = &mut self.server.players[i];
        sp.inventory = inv; sp.armour = armour; sp.cursor = cursor; sp.craft_grid = grid;
        // The model of the client's mirror: its own prediction (equal to the
        // real container in every slot this click moved).
        sp.container_sent.seen = Some(MirrorView { cell, kind, contents: predicted });
        sp.last_window_op_seq = op.op_seq;
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.strict_fold_holds(i, "container click")
    }

    /// A Strict joiner's container push: its numbered view is reserved
    /// (count and owned bytes) before it is sent; at a cap, fail closed.
    pub(super) fn push_strict_container(&mut self, i: usize) {
        let now = self.server.tick_counter;
        let server = &mut self.server;
        let sp = &mut server.players[i];
        let Some((mut sets, furnace)) = crate::window_ops::container_push_sets(sp, &server.world) else { return };
        let wire = crate::window_ops::wire_sets(&sets);
        // Run 5 (LOW-2) — the view holds what the client's mirror does.
        for (_, stack) in &mut sets { cw::strip_plan_body(stack); }
        let event = WindowEvent::ContainerView(ViewEvent::Sets { sets, furnace });
        let Ok(prepared) = crate::window_events::prepare_strict_events(sp, vec![event].into_boxed_slice()) else {
            self.strict_bridge_teardown(i, StrictTeardown::FellBehind, "container push reservation is unavailable");
            return;
        };
        let number = prepared.commit(sp, now);
        let pkt = crate::window_ops::push_packet(sp, wire, furnace, number);
        self.send_to_joined_slot(i, &protocol::serialize_packet(protocol::PacketType::WindowSlotSet, &pkt));
        self.flush_retired_overflow(i);
    }
}
