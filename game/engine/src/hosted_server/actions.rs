//! One joined typed action queue. Mutation admission is independent of movement.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::{ActionIntent, ActionPacket, ActionReceipt, AppliedActionAck};
use crate::strict_admission::{RefusalKind, StrictTeardown};

impl HostedServer {
    pub(super) fn strict_slot(&self, i: usize) -> bool {
        self.server.players.get(i).is_some_and(|p| p.server_simulated && p.inventory_mode == InventoryMode::Strict)
    }
    pub(super) fn reject_legacy_inventory(&mut self, i: usize) {
        self.strict_bridge_teardown(i, StrictTeardown::Untyped, "untyped inventory mutation on a Strict connection");
    }
    /// BRIDGE: recover instead of teardown — replace when C3d recovery/replay is wired.
    ///
    /// The ONE place a joined Strict session is closed for a refusal, a
    /// contest, a cap, an uncertainty or a protocol fault: logged with the
    /// developer detail, the player told `cause`'s plain sentence
    /// (`strict_admission::StrictTeardown::reason`) as the `JoinReject`
    /// reason. Departure settles and saves as any leave does; owned client
    /// history survives the reject.
    pub(super) fn strict_bridge_teardown(&mut self, i: usize, cause: StrictTeardown, detail: &str) {
        log::warn!("Strict session in slot {i} closed ({cause:?}): {detail}");
        #[cfg(test)]
        teardown_detail::note(detail);
        if let Some(left) = self.release_slot(i, Some(cause.reason())) { self.send_to_joined_except(i, &left); }
    }
    /// Until complete canonical recovery is connected, a paused action is
    /// explicitly failed closed ([`Self::strict_bridge_teardown`]).
    pub(super) fn pause_typed_action(&mut self, i: usize, cause: StrictTeardown, detail: &str) {
        if let Some(frontier) = self.server.players.get_mut(i).and_then(|p| p.action_frontier.as_mut()) { let _ = frontier.prepare(); }
        self.strict_bridge_teardown(i, cause, detail);
    }
    pub(super) fn receive_applied_action_ack(&mut self, i: usize, payload: &[u8]) {
        let Ok(ack) = protocol::safe_deserialize::<AppliedActionAck>(payload) else { return self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Invalid applied action acknowledgement") };
        let Some(current) = self.server.players.get(i).and_then(|p| p.action_frontier.as_ref()).map(|f| f.header(ack.through, ack.included_events)) else { return self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Applied action acknowledgement has no joined session") };
        if ack.session == current.session && ack.epoch <= current.epoch && ack.life <= current.life && (ack.epoch < current.epoch || ack.life < current.life) { return; }
        if ack.session != current.session || ack.epoch != current.epoch || ack.life != current.life { return self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Foreign or future applied action acknowledgement"); }
        let valid = self.server.players.get_mut(i).is_some_and(|sp| {
            sp.action_results.as_mut().is_some_and(|results| results.acknowledge(ack, sp.window_events.last_number()).is_ok())
        });
        if !valid { self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Foreign or regressing applied action acknowledgement"); }
    }
    pub(super) fn receive_typed_action(&mut self, i: usize, payload: &[u8]) {
        use crate::action_queue::{QueueError, QueueResult};
        let Ok(packet) = ActionPacket::decode(payload) else { return self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Invalid typed action") };
        let Some(sp) = self.server.players.get_mut(i) else { return };
        let Some(frontier) = &sp.action_frontier else { return self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Typed action has no joined session") };
        let current = frontier.header(packet.action.seq, packet.action.events_applied);
        if packet.action.session == current.session && packet.action.epoch <= current.epoch && packet.action.life <= current.life && (packet.action.epoch < current.epoch || packet.action.life < current.life) { return; }
        let result = sp.action_queue.as_mut().expect("issued action queue").push(frontier, packet.action, payload.to_vec().into_boxed_slice());
        match result {
            Ok(QueueResult::Queued | QueueResult::QueuedDuplicate) => {},
            Ok(QueueResult::TerminalDuplicate) => {
                if sp.action_results.as_mut().and_then(|r| r.repeat(packet.action)).is_none() {
                    self.pause_typed_action(i, StrictTeardown::FellBehind, "Old terminal action has no retained exact result");
                }
            }
            Err(QueueError::Paused) => {},
            Err(QueueError::Gap | QueueError::Count | QueueError::Bytes) => self.pause_typed_action(i, StrictTeardown::FellBehind, "Action history reached a recovery boundary"),
            Err(_) => self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Conflicting, foreign or exhausted action history"),
        }
    }
    pub(super) fn process_typed_actions(&mut self, i: usize, windows: &mut usize, items: &mut usize, budget: &mut EditTickBudget) {
        loop {
            if self.disconnected[i] { return; }
            let Some(sp) = self.server.players.get(i) else { return };
            let Some(queued) = sp.action_queue.as_ref().and_then(|q| q.front()) else { return };
            let Ok(packet) = ActionPacket::decode(&queued.payload) else { return self.strict_bridge_teardown(i, StrictTeardown::Unreadable, "Retained action could not decode") };
            let is_window = matches!(packet.intent, ActionIntent::Window(_));
            // C3d run 6 — a typed edit counts against the tick's edit budget
            // (the same `MAX_BLOCK_CHANGES_PER_TICK` legacy edits share), not
            // the item counter; past it the head waits for the next tick.
            if matches!(packet.intent, ActionIntent::Edit { .. }) {
                if budget.edits >= MAX_BLOCK_CHANGES_PER_TICK { return; }
                budget.edits += 1;
            } else {
                if (is_window && *windows >= MAX_WINDOW_OPS_PER_TICK) || (!is_window && *items >= MAX_ITEM_ACTIONS_PER_TICK) { return; }
                if is_window { *windows += 1; } else { *items += 1; }
            }
            let now = self.server.tick_counter;
            let sp = &mut self.server.players[i];
            let base = packet.action.events_applied;
            // Configuration (`SetAutoRefill`, `OpenPlayer`, `OpenTable`)
            // touches no contents, and a `Replace` keeps the refill setting
            // and station on both copies: it stays independent of a waiting
            // `Replace` and of death (`serve_strict_config`).
            let config = matches!(&packet.intent, ActionIntent::Window(op) if matches!(op.op, protocol::WireWindowOp::SetAutoRefill { .. } | protocol::WireWindowOp::OpenPlayer | protocol::WireWindowOp::OpenTable { .. }));
            // Run 5 (coordinator decision) — a stale base is superseded, not
            // a breach: one made before a whole-window `Replace` (join or
            // death sync) its client hadn't applied. Never a future base
            // (`last_replace <= last_sent`).
            // Run 6 (run-5 review LOW-2) — configuration too: a config op
            // made before the `Replace` is served on the server's current
            // copy without applying through its base, and without the
            // contents digest (`serve_strict_config`).
            let stale_base = sp.window_events.predates_replace(base);
            let boundary_ok = sp.window_events.accepts_boundary(base);
            if !boundary_ok && !stale_base {
                return self.pause_typed_action(i, StrictTeardown::FellBehind, "Action names an unavailable window boundary");
            }
            if boundary_ok && !(config && stale_base) { crate::window_events::apply_through(sp, base, now); }
            if !self.strict_events_hold(i) { return; }
            let sp = &self.server.players[i];
            // ...and so is any non-configuration action from a dead body.
            // `Superseded`: terminal, zero external effect (no window,
            // container, projectile, payment or event), never torn down.
            //
            // Run 6 (coordinator rule, run-5 review M1): every `Superseded`
            // comes with an authoritative correction. A stale base's is the
            // `Replace` it predates; a dead body's on a current base (after
            // the death sync) gets one fresh whole-window correction
            // `Replace`, at most one outstanding: actions based before it
            // are stale and supersede without another.
            //
            // Run 9 (run-6 review LOW-3) — a superseded `Edit` is corrected
            // in the world too: its predicted cell (and a door's pair) is
            // sent back by the refusal's path, and its row carries the cell.
            if !config && (stale_base || !sp.is_present_and_alive()) {
                let cells = match &packet.intent {
                    ActionIntent::Edit { change, .. } => {
                        let cells = self.edit_cells(change);
                        self.send_back_refused_edit(change, budget);
                        cells
                    }
                    _ => Vec::new(),
                };
                if !self.complete_typed_noop_at(i, packet.action, ActionFate::Superseded, &cells) { return; }
                if !stale_base { self.queue_correction_replace(i); }
                continue;
            }
            match packet.intent {
                ActionIntent::CancelledNoop => {
                    if !self.complete_typed_noop(i, packet.action, ActionFate::CancelledNoop) { return; }
                }
                ActionIntent::Window(op) if self.strict_slot(i) => {
                    if !self.serve_strict_window(i, packet.action, &op) { return; }
                }
                // C3d run 6 — a plain placement or a mined break
                // (`strict_edit.rs`); a use-tagged edit still waits for its
                // typed class.
                ActionIntent::Edit { change, hand, mined, use_tag: None, old, .. } if self.strict_slot(i) => {
                    if !self.serve_strict_edit(i, packet.action, &change, hand, mined.as_ref(), old, budget) { return; }
                }
                ActionIntent::Item(req) if self.strict_slot(i) => {
                    if let protocol::ItemAction::Shoot { weapon, hotbar_slot, held_kind, held_id, held_full, yaw, pitch, charge } = req.action {
                        if !self.serve_strict_shot(i, packet.action, req.seq, weapon, hotbar_slot, (held_kind, held_id, held_full), Aim { yaw, pitch, charge }) { return; }
                    } else { return self.pause_typed_action(i, StrictTeardown::Untyped, "Typed item handler is awaiting authoritative integration"); }
                }
                _ => return self.pause_typed_action(i, StrictTeardown::Untyped, "Typed mutation handler is awaiting authoritative integration"),
            }
        }
    }
    /// Run 6 — one whole-window correction `Replace` (window, station, lock
    /// mask, debt), re-arming the death/join `Replace` path; none while one
    /// is still unacknowledged.
    ///
    /// C3d run 14 (run-13 review LOW-3) — a dead body's correction is a live
    /// correction for the bound: the same per-seat rate bound
    /// (`strict_admission::CorrectionRate`) and the same reserved fit as
    /// [`Self::queue_live_correction_replace`], so a client that never
    /// respawns can't draw a whole-window sync per superseded action.
    pub(super) fn queue_correction_replace(&mut self, i: usize) {
        if !self.admit_correction_replace(i) { return; }
        let sp = &self.server.players[i];
        let fold = crate::window_events::effective_window_and_debt(sp);
        let n = self.send_inventory_sync(i, fold.window, fold.debt);
        self.server.players[i].window_events.death_replace = Some(n);
    }
    pub(super) fn prepare_typed_result(&mut self, i: usize, action: ActionHeader, fate: ActionFate, included_events: u32) -> Option<crate::action_results::PreparedResult> {
        self.prepare_typed_result_at(i, action, fate, included_events, &[])
    }
    /// [`Self::prepare_typed_result`] whose row also carries the action's
    /// own world cells (run 9, run-6 review LOW-3: an `Edit`'s cell and a
    /// door's pair), beside the station's table cell.
    pub(super) fn prepare_typed_result_at(&mut self, i: usize, action: ActionHeader, fate: ActionFate, included_events: u32, extra: &[[i32; 3]]) -> Option<crate::action_results::PreparedResult> {
        let receipt = ActionReceipt { action, fate, included_events };
        let mut cells: Vec<[i32; 3]> = match self.server.players[i].station { crate::window::Station::Table { cell } => vec![cell], _ => Vec::new() };
        for cell in extra { if !cells.contains(cell) { cells.push(*cell); } }
        self.server.players[i].action_results.as_mut()?.prepare(receipt, cells.into_boxed_slice()).ok()
    }
    pub(super) fn complete_typed_result(&mut self, i: usize, action: ActionHeader, fate: ActionFate, result: crate::action_results::PreparedResult) {
        let sp = &mut self.server.players[i];
        let frontier = sp.action_frontier.as_mut().expect("issued frontier");
        frontier.terminal(action, fate).expect("only the exact queue head commits");
        sp.action_queue.as_mut().expect("issued queue").pop_terminal(frontier).expect("terminal queue head is retained");
        sp.action_results.as_mut().expect("issued results").commit(result);
    }
    pub(super) fn complete_typed_noop(&mut self, i: usize, action: ActionHeader, fate: ActionFate) -> bool {
        self.complete_typed_noop_at(i, action, fate, &[])
    }
    pub(super) fn complete_typed_noop_at(&mut self, i: usize, action: ActionHeader, fate: ActionFate, cells: &[[i32; 3]]) -> bool {
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result_at(i, action, fate, included, cells) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Terminal result history is full"); return false;
        };
        self.complete_typed_result(i, action, fate, result);
        true
    }
    pub(super) fn refuse_typed_action(&mut self, i: usize, action: ActionHeader) {
        self.refuse_typed_action_at(i, action, &[], RefusalKind::Rule);
    }
    /// [`Self::refuse_typed_action`] whose row carries the action's cells.
    ///
    /// C3d run 12 (S1) — on a Strict seat a refusal is CORRECTED, not torn
    /// down: its `Refused` row, then ONE whole-window correction `Replace`
    /// ([`Self::queue_live_correction_replace`]). Every in-flight action
    /// after it has an older base, so it is `Superseded` with zero effect,
    /// its cell sent back. Only for refusals whose client prediction a
    /// `Replace` plus the cell send-back fully undoes: typed edits (every
    /// Drift included), personal clicks and a refused Autofill, shots,
    /// `OpenPlayer`/`OpenTable`. The rest call
    /// [`Self::refuse_typed_action_then_teardown`].
    pub(super) fn refuse_typed_action_at(&mut self, i: usize, action: ActionHeader, cells: &[[i32; 3]], kind: RefusalKind) {
        if !self.strict_slot(i) {
            if self.complete_typed_noop_at(i, action, ActionFate::Refused, cells) { self.pause_typed_action(i, StrictTeardown::Unconfirmed, "action refused at its frontier"); }
            return;
        }
        self.server.players[i].strict_refusals.note(kind);
        if !self.complete_typed_noop_at(i, action, ActionFate::Refused, cells) { return; }
        // BRIDGE: the refused action's suffix is superseded, not replayed (Decision 16) — replace in S3 (recovery bundle + replay)
        self.queue_live_correction_replace(i);
    }
    /// A refusal a `Replace` can't undo — a container open or click (the
    /// `Replace` carries no container view) or `SetAutoRefill` (a `Replace`
    /// keeps the refill setting): still the bridge teardown.
    pub(super) fn refuse_typed_action_then_teardown(&mut self, i: usize, action: ActionHeader) {
        self.refuse_then_teardown_for(i, action, StrictTeardown::Unconfirmed, "action refused at its frontier");
    }
    /// C3d run 13 (run-12 review LOW-2) — the server's OWN window can't be
    /// represented exactly (`StrictStageError::RawUnrepresentable`): a
    /// `Replace` would ship the same window back and the next action would
    /// refuse again, so it is refused, then torn down with its own reason.
    pub(super) fn refuse_raw_unrepresentable(&mut self, i: usize, action: ActionHeader) {
        self.refuse_then_teardown_for(i, action, StrictTeardown::Unrepresentable, "the server's own window can't be represented exactly");
    }
    fn refuse_then_teardown_for(&mut self, i: usize, action: ActionHeader, cause: StrictTeardown, detail: &str) {
        if self.strict_slot(i) { self.server.players[i].strict_refusals.note(RefusalKind::Rule); }
        if self.complete_typed_noop(i, action, ActionFate::Refused) { self.pause_typed_action(i, cause, detail); }
    }
    /// C3d run 12 (S1) — a LIVE Strict seat's correction `Replace`: the
    /// whole window of [`Self::queue_correction_replace`], but it takes no
    /// death-only flag (`death_replace` keeps meaning a death sync, which
    /// gates requests); the neutral "a correction `Replace` waits" flag is
    /// [`crate::window_events::WindowEvents::replace_waiting`]. At most one
    /// waits. One that doesn't fit the pending window events (count and
    /// retained bytes) is never forced in: the seat is torn down.
    ///
    /// Run 13 (run-12 review LOW-1) — the room is RESERVED in the deque
    /// before it is measured (`window_events::reserve_strict_replace`), so
    /// the `push_back` can't grow it past the byte bound and make the next
    /// honest action fall behind. Run 13 (LOW-3) — at most
    /// `strict_admission::MAX_CORRECTIONS_PER_WINDOW` per
    /// `CORRECTION_WINDOW_TICKS`; the next is a `FellBehind` teardown.
    pub(super) fn queue_live_correction_replace(&mut self, i: usize) {
        if !self.admit_correction_replace(i) { return; }
        let fold = crate::window_events::effective_window_and_debt(&self.server.players[i]);
        self.send_inventory_sync(i, fold.window, fold.debt);
    }
    /// Shared by both correction paths: `false` when one already waits
    /// (nothing queued), or when the seat was torn down at the rate bound or
    /// the reserved fit. On `true` the deque's room is reserved.
    fn admit_correction_replace(&mut self, i: usize) -> bool {
        let now = self.server.tick_counter;
        let sp = &mut self.server.players[i];
        if sp.window_events.replace_waiting() { return false; }
        if !sp.strict_corrections.admit(now) {
            self.strict_bridge_teardown(i, StrictTeardown::FellBehind, CORRECTION_RATE_DETAIL);
            return false;
        }
        let fold = crate::window_events::effective_window_and_debt(sp);
        let probe = crate::window_events::WindowEvent::Replace { window: Box::new(fold.window), debt: fold.debt, station: sp.station };
        if !crate::window_events::reserve_strict_replace(sp, &probe) {
            self.strict_bridge_teardown(i, StrictTeardown::FellBehind, CORRECTION_FIT_DETAIL);
            return false;
        }
        true
    }
    /// A terminal zero-effect fate (`Refused`, or `NormalisedAcceptedNoop`
    /// for a contested shared container) is retained first; the
    /// authoritative correction that must follow is the bridge teardown
    /// until recovery is wired.
    pub(super) fn terminal_then_correct(&mut self, i: usize, action: ActionHeader, fate: ActionFate, detail: &str) {
        if self.complete_typed_noop(i, action, fate) { self.pause_typed_action(i, StrictTeardown::Unconfirmed, detail); }
    }
    fn serve_strict_window(&mut self, i: usize, action: ActionHeader, op: &protocol::WindowOpPacket) -> bool {
        let protocol::WireWindowOp::Click(click) = &op.op else {
            // Configuration, container entry and container clicks:
            // `strict_container.rs` (C3d shared containers).
            return self.serve_strict_window_op(i, action, op);
        };
        let sp = &self.server.players[i];
        // The server's EXISTING transition (review M2): `serve_op`'s context
        // (table reach slack and grace) and rule (the client's verdict on a
        // Result or Autofill), shared, never a second copy.
        let ctx = crate::window_ops::server_click_ctx(sp, &self.server.world, self.server.play_mode.is_creative());
        let staging = if matches!(click, crate::window::WindowClick::Autofill { .. }) && !op.client_ok {
            // Run-2 review LOW-2: a refused Autofill stages exactly the safe
            // subset its client did (no change, or step 1 only).
            match crate::window_events::stage_strict_refused_autofill(sp, &self.server.registry, click, op.digest) {
                Ok(Some(staged)) => Ok(staged),
                Ok(None) => { self.refuse_typed_action(i, action); return false; }
                Err(e) => Err(e),
            }
        } else {
            crate::window_events::stage_strict_click(sp, &self.server.registry, click, &ctx, op.client_ok)
        };
        let mut staged = match staging {
            Ok(staged) => staged,
            Err(crate::window_events::StrictStageError::Debt) => return self.strict_debt_at_staging(i, "window click"),
            Err(crate::window_events::StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(crate::window_events::StrictStageError::Unrepresentable) => {
                // Review L1: never commit a window that would drop or clamp a stack.
                self.refuse_typed_action(i, action); return false;
            }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Window source or pending fit is uncertain"); return false; }
        };
        let station = crate::window::station_after(sp.station, click, &staged.raw.result);
        let digest = crate::window::digest(&staged.raw.after.view(), station);
        if digest != op.digest {
            self.refuse_typed_action(i, action); return false;
        }
        let included = sp.window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Window result history is full"); return false;
        };
        let (inv, armour, cursor, grid) = staged.raw.after.into_parts();
        let sp = &mut self.server.players[i];
        sp.inventory = inv; sp.armour = armour; sp.cursor = cursor; sp.craft_grid = grid; sp.station = station;
        sp.last_window_op_seq = op.op_seq;
        // Mirror parity (`window_ops::serve_op`): the client's own close
        // closes its container view, in its order.
        if crate::window_ops::op_closes_container_view(&op.op) {
            crate::window_ops::close_container_view(sp);
        }
        #[cfg(test)]
        commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.strict_fold_holds(i, "window click")
    }
    fn serve_strict_shot(&mut self, i: usize, action: ActionHeader, request_seq: u32, weapon: protocol::ShotWeapon, slot: u8, claim: (u8, u16, protocol::WireItem), aim: Aim) -> bool {
        use crate::item::{Item, ItemStack};
        let now = self.server.tick_counter;
        let sp = &self.server.players[i];
        if slot >= 9 || !aim.yaw.is_finite() || !aim.pitch.is_finite() {
            self.refuse_typed_action(i, action); return false;
        }
        let Some(ItemStack { item: Item::Tool(historical), count: 1 }) = sp.inventory.slot(slot as usize) else {
            self.refuse_typed_action(i, action); return false;
        };
        let historical = *historical;
        let item = Item::Tool(historical);
        if crate::shot::weapon_of(Some(&item)) != Some(weapon) || crate::inventory::item_to_ref(&item).to_wire() != (claim.0, claim.1) || crate::inventory::item_to_wire_full(&item) != claim.2 {
            self.refuse_typed_action(i, action); return false;
        }
        if now.saturating_add(crate::shot::SHOT_JITTER_TICKS) < sp.next_shot_tick {
            self.refuse_typed_action(i, action); return false;
        }
        let mut effective = match crate::window_events::strict_effective_marker(sp, &self.server.registry) {
            Ok(effective) => effective,
            Err(crate::window_events::StrictStageError::Debt) => return self.strict_debt_at_staging(i, "shot"),
            Err(crate::window_events::StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(crate::window_events::StrictStageError::Unrepresentable) => { self.refuse_typed_action(i, action); return false; }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Shot source lineage is uncertain"); return false; }
        };
        let effective_view = effective.view();
        let Some(ItemStack { item: Item::Tool(surviving), count: 1 }) = effective_view.inv.slot(slot as usize) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "The historical shot weapon no longer survives"); return false;
        };
        if surviving.tool_type != historical.tool_type || surviving.material != historical.material {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "The historical shot weapon has changed"); return false;
        }
        let events = vec![
            crate::window_events::WindowEvent::Take { slot: slot as usize, item: Item::Material(crate::shot::ammo_for(weapon)), n: 1, how: "on a shot" },
            crate::window_events::WindowEvent::WearWeapon { slot: slot as usize, tool: historical },
        ].into_boxed_slice();
        match crate::window_events::stage_strict_effects(sp, &self.server.registry, &events) {
            Ok(()) => {}
            Err(crate::window_events::StrictStageError::Debt) => return self.strict_debt_at_staging(i, "shot"),
            Err(crate::window_events::StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(crate::window_events::StrictStageError::Unrepresentable) => { self.refuse_typed_action(i, action); return false; }
            // C3d run 13 — the ammo isn't there to pay (raw R, or after the
            // pending prefix): deterministic on this state, so a refusal
            // the correction `Replace` undoes (the client spent nothing).
            Err(crate::window_events::StrictStageError::MissingPayment) => { self.refuse_typed_action(i, action); return false; }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Shot payment or pending fit is uncertain"); return false; }
        }
        // The outcome's number is the second event's: known before anything
        // is reserved, so no reservation waits on a fallible serialisation.
        let included = sp.window_events.last_number().checked_add(2);
        let Some(included) = included else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Window event numbers are exhausted"); return false;
        };
        let outcome = protocol::ItemActionOutcomePacket { seq: request_seq, accepted: true, consume_held: 1, note: 0, window_event: included, wear_held: true, bite_after: 0 };
        let packet = protocol::serialize_packet(protocol::PacketType::ItemActionOutcome, &outcome);
        if packet.len() > crate::action_journal::MAX_PAGE_BYTES {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Shot outcome exceeds its reservation"); return false;
        }
        let Ok(prepared_events) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Whole shot event reservation is unavailable"); return false;
        };
        debug_assert_eq!(prepared_events.last_number(), included);
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            // The events' reservation is released exactly: no capacity leaks.
            prepared_events.abandon(&mut self.server.players[i]);
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Shot result history is full"); return false;
        };
        let sp = &self.server.players[i];
        let shooter = crate::entity::Shooter { who: crate::combat::Attacker::Remote { slot: i, generation: sp.attach_gen }, owner_key: sp.pet_owner_key().unwrap_or_default() };
        let launch = crate::shot::launch(weapon, sp.player.eye_pos(), crate::camera::forward_from(aim.yaw, aim.pitch), aim.charge);
        // Every pay/source/fit/event/result allocation above precedes BOTH the
        // projectile and the numbered event registration. No belief bucket.
        crate::shot::spawn(&mut self.server.ecs, &launch, Some(shooter));
        let sp = &mut self.server.players[i];
        sp.next_shot_tick = sp.next_shot_tick.max(now).saturating_add(crate::shot::SHOT_COOLDOWN_TICKS);
        prepared_events.commit(sp, now);
        #[cfg(test)]
        commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.send_to_joined_slot(i, &packet);
        self.strict_fold_holds(i, "shot")
    }

    /// Decision 10 — staging found an obligation in a Strict session's fold.
    /// Strict never creates one, so this is an invariant breach detected
    /// before any effect: logged, nothing committed, the action paused at
    /// its frontier (fail-closed teardown until recovery is wired).
    pub(super) fn strict_debt_at_staging(&mut self, i: usize, path: &str) -> bool {
        log::error!(
            "Strict inventory invariant breached: slot {i} carries an obligation at a Strict {path}'s staging; nothing committed"
        );
        self.pause_typed_action(i, StrictTeardown::Obligation, "This world's strict inventory check found an obligation it can't account for");
        false
    }

    /// Decision 10 — Strict never creates debt. After every Strict commit the
    /// effective fold (live debt, pending corrections, session history,
    /// `ServerPlayer::obligations_proven_zero`) must prove zero. A breach
    /// seen after a commit is logged and the session torn down with a
    /// reason. Never a panic, never a write-off: the departure save carries
    /// whatever the fold holds.
    pub(super) fn strict_fold_holds(&mut self, i: usize, path: &str) -> bool {
        let Some(sp) = self.server.players.get(i) else { return false };
        // Run-2 review LOW-1: the SAME predicate staging uses.
        if crate::window_events::strict_leaves_no_unpaid_obligation(sp, &self.server.registry) { return true; }
        log::error!(
            "Strict inventory invariant breached: {} carries an obligation after a Strict {path} commit; tearing the session down",
            sp.display_name
        );
        self.strict_bridge_teardown(i, StrictTeardown::Obligation, "This world's strict inventory check found an obligation it can't account for");
        false
    }

    /// C3d (run 4) — a Strict copy applied a numbered Take short
    /// (`window_events::apply_front`): Strict never queues one its window
    /// can't pay, so this is an invariant breach, never log-only. Torn down
    /// with the reason through the one BRIDGE; `false` when it was. Checked
    /// where a Strict action applies its boundary (before anything executes
    /// on top of it) and after every tick's overdue valve, which covers any
    /// other application point by the end of that tick.
    pub(super) fn strict_events_hold(&mut self, i: usize) -> bool {
        let Some(sp) = self.server.players.get_mut(i) else { return true };
        let Some(event) = sp.window_events.take_strict_short_take() else { return true };
        if self.disconnected.get(i).copied().unwrap_or(true) { return false; }
        self.strict_bridge_teardown(i, StrictTeardown::Obligation, &format!("window event {event} took short of what Strict admitted"));
        false
    }

    /// Decision 10 — a Strict seat's departure (after `settle_leaving_life`):
    /// a non-empty fold is an invariant breach, logged. The save that
    /// follows records it as it stands; nothing is written off.
    pub(super) fn note_strict_departure(&self, i: usize) {
        if self.strict_slot(i) && !self.server.players[i].obligations_proven_zero() {
            log::error!(
                "Strict inventory invariant breached: {} departs carrying an obligation; saved as it stands, never written off",
                self.server.players[i].display_name
            );
        }
    }
}

/// C3d run 14 (run-13 review LOW-5) — the internal detail of the two
/// correction-bound teardowns (the player sees `FellBehind`'s sentence).
pub(super) const CORRECTION_RATE_DETAIL: &str = "the live-correction rate bound: more than 8 correction Replaces in 100 ticks";
pub(super) const CORRECTION_FIT_DETAIL: &str = "a refusal's correction Replace doesn't fit the pending window events";

/// C3d run 14 (run-13 review LOW-5) — test builds record each teardown's
/// internal detail on the test's thread, so a test tells its teardowns
/// apart by reason, not by context.
#[cfg(test)]
pub(super) mod teardown_detail {
    thread_local! { static LAST: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
    pub fn note(detail: &str) { LAST.with(|l| *l.borrow_mut() = Some(detail.to_string())); }
    /// The detail of this thread's latest teardown.
    pub fn last() -> String { LAST.with(|l| l.borrow().clone().unwrap_or_default()) }
}

/// Test-only fault seam: make a Strict commit leave an obligation in the
/// fold, so the post-commit guard ([`HostedServer::strict_fold_holds`]) can
/// be pinned through the real dispatch.
#[cfg(test)]
pub(super) mod commit_fault {
    thread_local! {
        pub static OWE_AFTER_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// Run-2 review LOW-1: queue a pending Take the window can't pay.
        pub static SHORT_TAKE_AFTER_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    pub fn after_commit(sp: &mut crate::server::ServerPlayer) {
        if SHORT_TAKE_AFTER_COMMIT.with(|f| f.get()) {
            let take = crate::window_events::WindowEvent::Take { slot: 35, item: crate::item::Item::Block(crate::block::BEDROCK), n: 1, how: "fault seam" };
            crate::window_events::queue(sp, take, 0);
        }
        if OWE_AFTER_COMMIT.with(|f| f.get()) {
            let stone = crate::item::ItemStack { item: crate::item::Item::Block(crate::block::STONE), count: 1 };
            let take = crate::container_window::ItemDelta { take: vec![(0, stone)], give: vec![] };
            sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, &take);
        }
    }
}
