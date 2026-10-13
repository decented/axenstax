//! C3d run 24 — a Strict seat's last paused classes: eating
//! (`ItemAction::Eat`), a power device's right-click (`ActionIntent::Device`)
//! and a creative give (`ItemAction::CreativeGive`), typed with the shape of
//! the entity interactions (`strict_interact.rs`).
//!
//! The item is the server copy's item in the NAMED hotbar slot, never the
//! claim; a slot past the hotbar or a claim that disagrees with that item
//! is a window disagreement, a corrected refusal (`refuse_typed_action`:
//! one `Replace`, inside the bound). The client predicts NOTHING for any of
//! the three (`ActionIntent::predicts_nothing`): an eat takes no food and
//! feeds nothing until the server's accepted outcome (its only local change
//! is the eat cooldown that paces its clicks, not window or world state); a
//! device right-click waits for the broadcast; a creative give waits for its
//! `InventoryGrant`. So a world fact — not hungry, inside the server's
//! eating cooldown, no device there, out of reach, a foreign plot, a bag
//! with no room for the give — is a `Refused` row alone, uncorrected
//! (`refuse_world_fact`), with Mirror's `ItemActionOutcome { accepted:
//! false, note }` beside it for an eat or a give (a device right-click has
//! no outcome, as in Mirror).
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::action_wire::ActionIntent;
use crate::item_actions::ItemNote;
use crate::strict_admission::StrictTeardown;
use crate::window_events::{StrictStageError, WindowEvent};

impl HostedServer {
    /// A Strict seat's typed `Eat` from hotbar `hotbar_slot`: Mirror's
    /// judge (`item_actions::judge_eat`) on the server copy's item, the food
    /// staged as a `Take` from the named slot, then Mirror's meal
    /// (`item_actions::eat`) and cooldown.
    pub(super) fn serve_strict_eat(&mut self, i: usize, action: ActionHeader, req: &protocol::ItemActionPacket) -> bool {
        let protocol::ItemAction::Eat { hotbar_slot, held_kind, held_id, held_full } = &req.action else { unreachable!("an Eat") };
        let slot = usize::from(*hotbar_slot);
        if slot >= 9 { self.refuse_typed_action(i, action); return false; }
        let sp = &self.server.players[i];
        let historical = sp.inventory.slot(slot).map(|s| s.item.clone());
        if !super::strict_attack::claim_parts_match(historical.as_ref(), *held_kind, *held_id, held_full) {
            self.refuse_typed_action(i, action); return false;
        }
        // Not food in the server copy's slot (the claim agrees with it): only
        // a modified client asks, a window rule — corrected.
        let Some(food) = historical.filter(|item| item.food_value().is_some()) else { self.refuse_typed_action(i, action); return false };
        // The world facts (Mirror's judge): not hungry, inside the server's
        // eating cooldown. A dead body never reaches here (superseded).
        let alive = sp.server_simulated && sp.is_present_and_alive();
        let value = match crate::item_actions::judge_eat(alive, Some(&food), &sp.combat, sp.eat_cooldown) {
            Ok(value) => value,
            Err(note) => return self.refuse_item_world_fact(i, action, req, note),
        };
        let events: Box<[WindowEvent]> = Box::new([WindowEvent::Take { slot, item: food.clone(), n: 1, how: "by eating" }]);
        if let Some(stop) = self.stage_strict_item_events(i, action, &events, "eat") { return stop; }
        let Some(included) = self.server.players[i].window_events.last_number().checked_add(1) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Eat window event numbers are exhausted"); return false;
        };
        let outcome = protocol::ItemActionOutcomePacket { seq: req.seq, accepted: true, consume_held: 1, note: ItemNote::None.to_wire(), window_event: included, wear_held: false, bite_after: 0 };
        let packet = protocol::serialize_packet(protocol::PacketType::ItemActionOutcome, &outcome);
        let Ok(prepared) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
            let detail = super::actions::reservation_detail(&self.server.players[i], 1, "eat");
            self.pause_typed_action(i, StrictTeardown::FellBehind, &detail); return false;
        };
        debug_assert_eq!(prepared.last_number(), included);
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            prepared.abandon(&mut self.server.players[i]);
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Eat result history is full"); return false;
        };
        // Every check, the event and the result row precede the meal.
        let now = self.server.tick_counter;
        let sp = &mut self.server.players[i];
        crate::item_actions::eat(&mut sp.combat, value, food.eat_poison_ticks());
        sp.eat_cooldown = crate::item_actions::EAT_COOLDOWN_TICKS;
        prepared.commit(sp, now);
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.send_to_joined_slot(i, &packet);
        self.strict_fold_holds(i, "eat")
    }

    /// A Strict seat's typed creative give (`ItemAction::CreativeGive`): a
    /// Creative world's private grant of `item` to the joiner's own window
    /// (Mirror's `serve_creative_give` rules), staged as a `Grant` on the
    /// effective window and numbered like any grant. No hand is named.
    pub(super) fn serve_strict_creative_give(&mut self, i: usize, action: ActionHeader, req: &protocol::ItemActionPacket) -> bool {
        let protocol::ItemAction::CreativeGive { item: wire } = &req.action else { unreachable!("a CreativeGive") };
        // Not a Creative world: the play mode can change under the client
        // (an operator's `/gamemode`), and the give predicted nothing — a
        // world fact, as Mirror's `NotHere`.
        if !self.server.play_mode.is_creative() {
            return self.refuse_item_world_fact(i, action, req, ItemNote::NotHere);
        }
        // An unreadable stack, or a Plan stack of more than one: only a
        // modified client sends it — a rule refusal, corrected.
        let Some(stack) = crate::inventory::stack_from_wire(wire, &self.server.registry, crate::inventory::PlanDecode::Marker) else {
            self.refuse_typed_action(i, action); return false;
        };
        let plan = matches!(stack.item, crate::item::Item::Plan(_));
        if plan && stack.count != 1 { self.refuse_typed_action(i, action); return false; }
        // No room in the bag (Mirror's check on the effective inventory): both
        // copies agree it is full and nothing was predicted — the client
        // toasts the note. Never corrected, so a held give on a full bag
        // never draws on the correction bound.
        let mut room = crate::window_events::effective_inventory(&self.server.players[i]);
        if room.add_item(stack.clone()).is_some() {
            return self.refuse_item_world_fact(i, action, req, ItemNote::InventoryFull);
        }
        let events: Box<[WindowEvent]> = Box::new([WindowEvent::Grant(stack.clone())]);
        if let Some(stop) = self.stage_strict_item_events(i, action, &events, "creative give") { return stop; }
        let Some(included) = self.server.players[i].window_events.last_number().checked_add(1) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Creative give window event numbers are exhausted"); return false;
        };
        // Only the caller's private marker for a Plan, as Mirror sends it.
        let grant = if plan {
            let pkt = protocol::InventoryGrantPacket { item_kind: protocol::item_kind::PLAN, item_id: 0, count: 1, full_item: wire.full_item, window_event: included };
            protocol::serialize_packet(protocol::PacketType::InventoryGrant, &pkt)
        } else {
            let Some(grant) = grant_packet(&stack, included) else { self.refuse_typed_action(i, action); return false };
            grant
        };
        let outcome = protocol::ItemActionOutcomePacket { seq: req.seq, accepted: true, consume_held: 0, note: ItemNote::None.to_wire(), window_event: 0, wear_held: false, bite_after: 0 };
        let packet = protocol::serialize_packet(protocol::PacketType::ItemActionOutcome, &outcome);
        let Ok(prepared) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
            let detail = super::actions::reservation_detail(&self.server.players[i], 1, "creative give");
            self.pause_typed_action(i, StrictTeardown::FellBehind, &detail); return false;
        };
        debug_assert_eq!(prepared.last_number(), included);
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            prepared.abandon(&mut self.server.players[i]);
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Creative give result history is full"); return false;
        };
        let now = self.server.tick_counter;
        let sp = &mut self.server.players[i];
        prepared.commit(sp, now);
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        // Mirror's order: the grant, then the outcome.
        self.send_to_joined_slot(i, &grant);
        self.send_to_joined_slot(i, &packet);
        self.strict_fold_holds(i, "creative give")
    }

    /// A Strict seat's typed right-click on the power device at `cell`
    /// (`ActionIntent::Device`): Mirror's rules (`handle_device_interact`:
    /// the play mode and plot, reach, a toggle-class device there), with the
    /// reach of the server copy's item in hotbar `hotbar_slot` (a Reach Claw
    /// counts), never the movement packet's claimed hand. The shared
    /// transition (`power::interact_device`) is the server's; its changes go
    /// out on the ordinary broadcast. No window event.
    pub(super) fn serve_strict_device(&mut self, i: usize, action: ActionHeader, cell: [i32; 3], hotbar_slot: u8) -> bool {
        let slot = usize::from(hotbar_slot);
        if slot >= 9 { self.refuse_typed_action(i, action); return false; }
        let intent = ActionIntent::Device { cell, hotbar_slot };
        let sp = &self.server.players[i];
        let pos = (cell[0], cell[1], cell[2]);
        let (kind, id) = sp.inventory.slot(slot).map_or(protocol::ItemRef::Empty.to_wire(), |s| crate::inventory::item_to_ref(&s.item).to_wire());
        let d = glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5) - sp.player.eye_pos();
        let toggle_class = self.server.world.power_device_at(pos).is_some_and(|dev| crate::power::is_toggle_class(dev.kind));
        if self.remote_may_touch(sp, cell[0], cell[2]).is_err()
            || !block_change_within_reach(d.length_squared(), kind, id, false)
            || !toggle_class
        {
            return self.refuse_world_fact(i, action, &intent);
        }
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result_at(i, action, ActionFate::Accepted, included, &[cell]) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Device result history is full"); return false;
        };
        let now = self.server.tick_counter;
        let Some(outcome) = crate::power::interact_device(&mut self.server.world, pos, now) else {
            // Checked toggle-class above, and `interact_device` has an arm
            // for every one (`electricity_sources` pins it).
            unreachable!("a toggle-class device always has an interaction")
        };
        if self.lends_host_world() {
            self.lent_edit_cells.extend(outcome.changes.iter().map(|bc| (bc.x, bc.y, bc.z)));
        }
        self.pending_block_changes.extend(outcome.changes);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        true
    }

    /// C3d run 25 — a Strict seat's typed `Sleep` in the bed at `bed`:
    /// Mirror's rule (`item_actions::serve_sleep`: a living body, a bed
    /// there, in reach, night, not slept tonight), the spawn set at the bed
    /// and the body healed as the server already does for every joiner. No
    /// item and no window event: the client claims nothing and predicts
    /// nothing (its spawn is set by the accepted outcome), so every refusal
    /// is a world fact — the outcome's note and a `Refused` row,
    /// uncorrected. A dead body never reaches here (superseded).
    pub(super) fn serve_strict_sleep(&mut self, i: usize, action: ActionHeader, req: &protocol::ItemActionPacket, bed: [i32; 3]) -> bool {
        let server = &mut self.server;
        // C2a verify L1 — show the calendar the clock first (idempotent).
        server.night_calendar.observe(server.world_time);
        let (world_time, tonight) = (server.world_time, server.night_calendar.tonight());
        let sp = &server.players[i];
        let judged = crate::item_actions::judge_sleep(
            crate::item_actions::can_act(sp),
            server.world.get_block(bed[0], bed[1], bed[2]),
            crate::item_actions::bed_in_reach(sp.player.eye_pos(), bed),
            crate::item_actions::is_night(&server.world, world_time),
            tonight,
            sp.slept_night,
        );
        if let Err(note) = judged { return self.refuse_item_world_fact(i, action, req, note); }
        let included = self.server.players[i].window_events.last_number();
        let outcome = protocol::ItemActionOutcomePacket { seq: req.seq, accepted: true, consume_held: 0, note: ItemNote::None.to_wire(), window_event: 0, wear_held: false, bite_after: 0 };
        let packet = protocol::serialize_packet(protocol::PacketType::ItemActionOutcome, &outcome);
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Sleep result history is full"); return false;
        };
        // Judged above; Mirror's own run sets the spawn and heals.
        let server = &mut self.server;
        let slept = crate::item_actions::serve_sleep(&mut server.players[i], &server.world, world_time, tonight, bed);
        debug_assert!(slept.is_ok(), "judged before the result row");
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.send_to_joined_slot(i, &packet);
        true
    }

    /// A world fact refuses joiner `i`'s eat or give: Mirror's
    /// `ItemActionOutcome { accepted: false, note }`, then the `Refused` row
    /// alone, uncorrected (the client predicted nothing).
    fn refuse_item_world_fact(&mut self, i: usize, action: ActionHeader, req: &protocol::ItemActionPacket, note: ItemNote) -> bool {
        self.refuse_item_action(i, req.seq, note);
        self.refuse_world_fact(i, action, &ActionIntent::Item(req.clone()))
    }

    /// Stage `events` on the effective window: `None` when they fit, else
    /// the queue's answer after the refusal or pause (`Some(false)`): a
    /// debt is the invariant breach, a raw window that can't be represented
    /// is refused then torn down, a payment the window can't make or a
    /// product it can't carry is corrected, the rest fail closed.
    fn stage_strict_item_events(&mut self, i: usize, action: ActionHeader, events: &[WindowEvent], path: &str) -> Option<bool> {
        let sp = &self.server.players[i];
        match crate::window_events::stage_strict_effects(sp, &self.server.registry, events) {
            Ok(()) => None,
            Err(StrictStageError::Debt) => Some(self.strict_debt_at_staging(i, path)),
            Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); Some(false) }
            Err(StrictStageError::Unrepresentable | StrictStageError::MissingPayment) => { self.refuse_typed_action(i, action); Some(false) }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, &format!("{path}: payment or pending fit is uncertain")); Some(false) }
        }
    }
}
