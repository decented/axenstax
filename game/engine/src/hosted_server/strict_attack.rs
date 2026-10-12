//! C3d run 15 — a Strict seat's typed entity attack
//! (`ActionIntent::Attack`), modelled on the typed shot
//! (`actions.rs`, `serve_strict_shot`).
//!
//! The weapon is the server copy's item in the NAMED hotbar slot at the
//! action's boundary (raw R; Decision 8: the historical action slot supplies
//! heldness, an invalid slot refuses with no latest-slot fallback), never
//! the claim. A claim that disagrees with that item is refused exactly as a
//! shot's is (`refuse_typed_action`, a `Rule` refusal): zero external
//! effect, corrected by the S1 `Replace`, never torn down. The swing's own
//! rules (target alive and in reach ahead of the body, no friendly fire
//! unless sneaking, the swing schedule) are the Mirror swing's, shared
//! (`HostedServer::check_joiner_swing`); a swing they refuse is refused for
//! a WORLD FACT (run 16): zero effect (no damage, knockback, wear, schedule
//! move or event), its `Refused` receipt alone, no correction `Replace` and
//! no tick of the correction bound, since the client predicted nothing
//! (`HostedServer::refuse_world_fact`). A dead attacker never reaches here:
//! `process_typed_actions` supersedes a dead body's action first.
//!
//! A tool's wear is ONE numbered `WearWeapon` event, staged and reserved
//! (with the result row) BEFORE the damage lands; the server copy wears when
//! the client says it applied that event, as Mirror's does. A bare hand or
//! a non-tool item wears nothing (no event). The damage is the effective
//! surviving item's; a pending wear that broke or changed it (a last-point
//! sword swung twice inside one round trip) is a corrected refusal (run 16:
//! one `Replace`, never a teardown; the shot's rule too). Event/byte/result caps leave zero
//! external effect (a `FellBehind` teardown, exactly as a shot's). After the
//! commit, the one debt-zero predicate (`strict_fold_holds`).
//!
//! The answer is Mirror's `InteractOutcome` (accepted, the Wear's number):
//! the client wears its own copy through `joiner_actions::apply_outcome` as
//! before, and predicts nothing until then.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::item::{Item, ItemStack};
use crate::strict_admission::StrictTeardown;
use crate::window_events::StrictStageError;

impl HostedServer {
    pub(super) fn serve_strict_attack(&mut self, i: usize, action: ActionHeader, req: &protocol::EntityAttackPacket) -> bool {
        let slot = usize::from(req.hotbar_slot);
        if slot >= 9 {
            self.refuse_typed_action(i, action);
            return false;
        }
        // The historical held item: the server copy's, never the claim.
        let historical = self.server.players[i].inventory.slot(slot).map(|s| s.item.clone());
        if !claim_matches(historical.as_ref(), req) {
            self.refuse_typed_action(i, action);
            return false;
        }
        // C3d run 16 — the shared rule refuses only on world facts the
        // client could not know (reach and facing on the server's body, the
        // swing schedule, a target gone or dead, its own pet): a receipt
        // alone, no correction, since the client predicted nothing.
        let Some(target) = self.check_joiner_swing(i, req) else {
            return self.refuse_world_fact(i, action, &crate::action_wire::ActionIntent::Attack(req.clone()));
        };
        let sp = &self.server.players[i];
        let mut effective = match crate::window_events::strict_effective_marker(sp, &self.server.registry) {
            Ok(effective) => effective,
            Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "attack"),
            Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(StrictStageError::Unrepresentable) => { self.refuse_typed_action(i, action); return false; }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Attack weapon lineage is uncertain"); return false; }
        };
        let (base_damage, events) = match &historical {
            Some(Item::Tool(tool)) => {
                let view = effective.view();
                // C3d run 16 — a pending Wear the client hasn't applied yet
                // broke (or changed) the weapon it claims: a last-point sword
                // swung twice inside one round trip. The window disagrees,
                // deterministically on the server's own pending prefix, so it
                // is a corrected refusal (one `Replace`), never a teardown.
                let Some(ItemStack { item: Item::Tool(surviving), count: 1 }) = view.inv.slot(slot) else {
                    self.refuse_typed_action(i, action); return false;
                };
                if surviving.tool_type != tool.tool_type || surviving.material != tool.material {
                    self.refuse_typed_action(i, action); return false;
                }
                let damage = Item::Tool(*surviving).attack_damage();
                (damage, vec![crate::window_events::WindowEvent::WearWeapon { slot, tool: *tool }])
            }
            Some(item) => (item.attack_damage(), Vec::new()),
            None => (1.0, Vec::new()),
        };
        let events = events.into_boxed_slice();
        let wears = !events.is_empty();
        if wears {
            match crate::window_events::stage_strict_effects(sp, &self.server.registry, &events) {
                Ok(()) => {}
                Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "attack"),
                Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
                Err(StrictStageError::Unrepresentable) => { self.refuse_typed_action(i, action); return false; }
                // C3d run 18 (run-17 review LOW-4) — the ambiguous wear named apart.
                Err(StrictStageError::AmbiguousWear) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Attack wear is ambiguous (AmbiguousWear)"); return false; }
                Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Attack wear or pending fit is uncertain"); return false; }
            }
        }
        // The outcome names the Wear's number (0 when nothing wears, as
        // Mirror's does): known before anything is reserved.
        let Some(included) = sp.window_events.last_number().checked_add(u32::from(wears)) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Attack window event numbers are exhausted"); return false;
        };
        let window_event = if wears { included } else { 0 };
        let outcome = protocol::InteractOutcomePacket { seq: req.seq, entity: req.entity, kind: None, accepted: true, consume_held: 0, note: 0, window_event };
        let packet = protocol::serialize_packet(protocol::PacketType::InteractOutcome, &outcome);
        if packet.len() > crate::action_journal::MAX_PAGE_BYTES {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Attack outcome exceeds its reservation"); return false;
        }
        let prepared_events = if wears {
            let Ok(prepared) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
                let detail = super::actions::reservation_detail(&self.server.players[i], 1, "attack");
                self.pause_typed_action(i, StrictTeardown::FellBehind, &detail); return false;
            };
            debug_assert_eq!(prepared.last_number(), included);
            Some(prepared)
        } else {
            None
        };
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            // The Wear's reservation is released exactly: no capacity leaks.
            if let Some(prepared) = prepared_events { prepared.abandon(&mut self.server.players[i]); }
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Attack result history is full"); return false;
        };
        // Every check, the Wear's reservation and the result row precede the
        // damage, the knockback and the schedule move.
        self.strike_joiner_swing(i, target, req.sprint, req.sneak, base_damage);
        let now = self.server.tick_counter;
        let sp = &mut self.server.players[i];
        if let Some(prepared) = prepared_events { prepared.commit(sp, now); }
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.send_to_joined_slot(i, &packet);
        self.strict_fold_holds(i, "attack")
    }
}

/// Does the swing's claimed held item (`ItemRef` pair + full fidelity) name
/// exactly `held`, the server copy's item (`None` = an empty hand)? The
/// shot's claim rule, with the empty hand allowed.
pub(super) fn claim_matches(held: Option<&Item>, req: &protocol::EntityAttackPacket) -> bool {
    let (pair, full) = match held {
        Some(item) => (crate::inventory::item_to_ref(item).to_wire(), crate::inventory::item_to_wire_full(item)),
        None => (protocol::ItemRef::Empty.to_wire(), protocol::WireItem::None),
    };
    pair == (req.held_kind, req.held_id) && full == req.held_full
}
