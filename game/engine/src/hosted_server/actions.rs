//! One joined typed action queue. Mutation admission is independent of movement.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::{ActionIntent, ActionPacket, ActionReceipt, AppliedActionAck};

impl HostedServer {
    pub(super) fn strict_slot(&self, i: usize) -> bool {
        self.server.players.get(i).is_some_and(|p| p.server_simulated && p.inventory_mode == InventoryMode::Strict)
    }
    pub(super) fn reject_legacy_inventory(&mut self, i: usize) {
        self.fail_action_history(i, "Strict inventory mutation requires an allocated action identity");
    }
    fn fail_action_history(&mut self, i: usize, reason: &str) {
        if let Some(left) = self.release_slot(i, Some(reason)) { self.send_to_joined_except(i, &left); }
    }
    /// Until complete canonical recovery is connected, a paused action is
    /// explicitly failed closed. Owned client history survives the real reject.
    fn pause_typed_action(&mut self, i: usize, reason: &str) {
        if let Some(frontier) = self.server.players.get_mut(i).and_then(|p| p.action_frontier.as_mut()) { let _ = frontier.prepare(); }
        self.fail_action_history(i, reason);
    }
    pub(super) fn receive_applied_action_ack(&mut self, i: usize, payload: &[u8]) {
        let Ok(ack) = protocol::safe_deserialize::<AppliedActionAck>(payload) else { return self.fail_action_history(i, "Invalid applied action acknowledgement") };
        let Some(current) = self.server.players.get(i).and_then(|p| p.action_frontier.as_ref()).map(|f| f.header(ack.through, ack.included_events)) else { return self.fail_action_history(i, "Applied action acknowledgement has no joined session") };
        if ack.session == current.session && ack.epoch <= current.epoch && ack.life <= current.life && (ack.epoch < current.epoch || ack.life < current.life) { return; }
        if ack.session != current.session || ack.epoch != current.epoch || ack.life != current.life { return self.fail_action_history(i, "Foreign or future applied action acknowledgement"); }
        let valid = self.server.players.get_mut(i).is_some_and(|sp| {
            sp.action_results.as_mut().is_some_and(|results| results.acknowledge(ack, sp.window_events.last_number()).is_ok())
        });
        if !valid { self.fail_action_history(i, "Foreign or regressing applied action acknowledgement"); }
    }
    pub(super) fn receive_typed_action(&mut self, i: usize, payload: &[u8]) {
        use crate::action_queue::{QueueError, QueueResult};
        let Ok(packet) = ActionPacket::decode(payload) else { return self.fail_action_history(i, "Invalid typed action") };
        let Some(sp) = self.server.players.get_mut(i) else { return };
        let Some(frontier) = &sp.action_frontier else { return self.fail_action_history(i, "Typed action has no joined session") };
        let current = frontier.header(packet.action.seq, packet.action.events_applied);
        if packet.action.session == current.session && packet.action.epoch <= current.epoch && packet.action.life <= current.life && (packet.action.epoch < current.epoch || packet.action.life < current.life) { return; }
        let result = sp.action_queue.as_mut().expect("issued action queue").push(frontier, packet.action, payload.to_vec().into_boxed_slice());
        match result {
            Ok(QueueResult::Queued | QueueResult::QueuedDuplicate) => {},
            Ok(QueueResult::TerminalDuplicate) => {
                if sp.action_results.as_mut().and_then(|r| r.repeat(packet.action)).is_none() {
                    self.pause_typed_action(i, "Old terminal action has no retained exact result; rejoin required");
                }
            }
            Err(QueueError::Paused) => {},
            Err(QueueError::Gap | QueueError::Count | QueueError::Bytes) => self.pause_typed_action(i, "Action history reached a recovery boundary; rejoin required"),
            Err(_) => self.fail_action_history(i, "Conflicting, foreign or exhausted action history"),
        }
    }
    pub(super) fn process_typed_actions(&mut self, i: usize, windows: &mut usize, items: &mut usize) {
        loop {
            if self.disconnected[i] { return; }
            let Some(sp) = self.server.players.get(i) else { return };
            let Some(queued) = sp.action_queue.as_ref().and_then(|q| q.front()) else { return };
            let Ok(packet) = ActionPacket::decode(&queued.payload) else { return self.fail_action_history(i, "Retained action could not decode") };
            let is_window = matches!(packet.intent, ActionIntent::Window(_));
            if (is_window && *windows >= MAX_WINDOW_OPS_PER_TICK) || (!is_window && *items >= MAX_ITEM_ACTIONS_PER_TICK) { return; }
            if is_window { *windows += 1; } else { *items += 1; }
            let now = self.server.tick_counter;
            let sp = &mut self.server.players[i];
            if !sp.window_events.accepts_boundary(packet.action.events_applied) {
                return self.pause_typed_action(i, "Action names an unavailable window boundary; rejoin required");
            }
            crate::window_events::apply_through(sp, packet.action.events_applied, now);
            if !sp.is_present_and_alive() || sp.window_events.death_replace.is_some() {
                return self.refuse_typed_action(i, packet.action);
            }
            match packet.intent {
                ActionIntent::CancelledNoop => {
                    if !self.complete_typed_noop(i, packet.action, ActionFate::CancelledNoop) { return; }
                }
                ActionIntent::Window(op) if self.strict_slot(i) => {
                    if !self.serve_strict_window(i, packet.action, &op) { return; }
                }
                ActionIntent::Item(req) if self.strict_slot(i) => {
                    if let protocol::ItemAction::Shoot { weapon, hotbar_slot, held_kind, held_id, held_full, yaw, pitch, charge } = req.action {
                        if !self.serve_strict_shot(i, packet.action, req.seq, weapon, hotbar_slot, (held_kind, held_id, held_full), Aim { yaw, pitch, charge }) { return; }
                    } else { return self.pause_typed_action(i, "Typed item handler is awaiting authoritative integration; rejoin required"); }
                }
                _ => return self.pause_typed_action(i, "Typed mutation handler is awaiting authoritative integration; rejoin required"),
            }
        }
    }
    fn prepare_typed_result(&mut self, i: usize, action: ActionHeader, fate: ActionFate, included_events: u32) -> Option<crate::action_results::PreparedResult> {
        let receipt = ActionReceipt { action, fate, included_events };
        let cells = match self.server.players[i].station { crate::window::Station::Table { cell } => vec![cell].into_boxed_slice(), _ => Box::new([]) };
        self.server.players[i].action_results.as_mut()?.prepare(receipt, cells).ok()
    }
    fn complete_typed_result(&mut self, i: usize, action: ActionHeader, fate: ActionFate, result: crate::action_results::PreparedResult) {
        let sp = &mut self.server.players[i];
        let frontier = sp.action_frontier.as_mut().expect("issued frontier");
        frontier.terminal(action, fate).expect("only the exact queue head commits");
        sp.action_queue.as_mut().expect("issued queue").pop_terminal(frontier).expect("terminal queue head is retained");
        sp.action_results.as_mut().expect("issued results").commit(result);
    }
    fn complete_typed_noop(&mut self, i: usize, action: ActionHeader, fate: ActionFate) -> bool {
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, fate, included) else {
            self.pause_typed_action(i, "Terminal result history is full; rejoin required"); return false;
        };
        self.complete_typed_result(i, action, fate, result);
        true
    }
    fn refuse_typed_action(&mut self, i: usize, action: ActionHeader) {
        if self.complete_typed_noop(i, action, ActionFate::Refused) { self.pause_typed_action(i, "Action refused at an authoritative frontier; rejoin required"); }
    }
    fn serve_strict_window(&mut self, i: usize, action: ActionHeader, op: &protocol::WindowOpPacket) -> bool {
        let protocol::WireWindowOp::Click(click) = &op.op else {
            self.pause_typed_action(i, "Typed window configuration is awaiting integration; rejoin required"); return false;
        };
        let sp = &self.server.players[i];
        let ctx = crate::window::ClickCtx::new(self.server.play_mode.is_creative(), sp.station, sp.player.eye_pos(), |cell| self.server.world.get_block(cell[0], cell[1], cell[2]));
        let Ok(mut staged) = crate::window_events::stage_strict_click(sp, &self.server.registry, click, &ctx) else {
            self.pause_typed_action(i, "Window source or pending fit is uncertain; rejoin required"); return false;
        };
        let station = crate::window::station_after(sp.station, click, &staged.raw.result);
        let digest = crate::window::digest(&staged.raw.after.view(), station);
        if (matches!(click, crate::window::WindowClick::Result | crate::window::WindowClick::Autofill { .. }) && staged.raw.result.ok() != op.client_ok) || digest != op.digest {
            self.refuse_typed_action(i, action); return false;
        }
        let included = sp.window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, "Window result history is full; rejoin required"); return false;
        };
        let (inv, armour, cursor, grid) = staged.raw.after.into_parts();
        let sp = &mut self.server.players[i];
        sp.inventory = inv; sp.armour = armour; sp.cursor = cursor; sp.craft_grid = grid; sp.station = station;
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        true
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
        let Ok(mut effective) = crate::window_events::strict_effective_marker(sp, &self.server.registry) else {
            self.pause_typed_action(i, "Shot source lineage is uncertain; rejoin required"); return false;
        };
        let effective_view = effective.view();
        let Some(ItemStack { item: Item::Tool(surviving), count: 1 }) = effective_view.inv.slot(slot as usize) else {
            self.pause_typed_action(i, "The historical shot weapon no longer survives; rejoin required"); return false;
        };
        if surviving.tool_type != historical.tool_type || surviving.material != historical.material {
            self.pause_typed_action(i, "The historical shot weapon has changed; rejoin required"); return false;
        }
        let events = vec![
            crate::window_events::WindowEvent::Take { slot: slot as usize, item: Item::Material(crate::shot::ammo_for(weapon)), n: 1, how: "on a shot" },
            crate::window_events::WindowEvent::WearWeapon { slot: slot as usize, tool: historical },
        ].into_boxed_slice();
        if crate::window_events::stage_strict_effects(sp, &self.server.registry, &events).is_err() {
            self.pause_typed_action(i, "Shot payment or pending fit is uncertain; rejoin required"); return false;
        }
        let Ok(prepared_events) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
            self.pause_typed_action(i, "Whole shot event reservation is unavailable; rejoin required"); return false;
        };
        let included = prepared_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, "Shot result history is full; rejoin required"); return false;
        };
        let outcome = protocol::ItemActionOutcomePacket { seq: request_seq, accepted: true, consume_held: 1, note: 0, window_event: included, wear_held: true, bite_after: 0 };
        let packet = protocol::serialize_packet(protocol::PacketType::ItemActionOutcome, &outcome);
        if packet.len() > crate::action_journal::MAX_PAGE_BYTES {
            self.pause_typed_action(i, "Shot outcome exceeds its reservation; rejoin required"); return false;
        }
        let sp = &self.server.players[i];
        let shooter = crate::entity::Shooter { who: crate::combat::Attacker::Remote { slot: i, generation: sp.attach_gen }, owner_key: sp.pet_owner_key().unwrap_or_default() };
        let launch = crate::shot::launch(weapon, sp.player.eye_pos(), crate::camera::forward_from(aim.yaw, aim.pitch), aim.charge);
        // Every pay/source/fit/event/result allocation above precedes BOTH the
        // projectile and the numbered event registration. No belief bucket.
        crate::shot::spawn(&mut self.server.ecs, &launch, Some(shooter));
        let sp = &mut self.server.players[i];
        sp.next_shot_tick = sp.next_shot_tick.max(now).saturating_add(crate::shot::SHOT_COOLDOWN_TICKS);
        prepared_events.commit(sp, now);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.send_to_joined_slot(i, &packet);
        true
    }
}
