//! C3d run 23 — a Strict seat's typed entity interaction
//! (`ActionIntent::Interact`: feed, tame, shear, milk, Lead on and off, pet
//! commands, and `Mount` and `LeadToPost`, which ride the same packet),
//! modelled on the typed attack (`strict_attack.rs`).
//!
//! The item is the server copy's item in the NAMED hotbar slot at the
//! action's boundary, never the claim: a slot past the hotbar or a claim
//! that disagrees with that item is a window disagreement, a corrected
//! refusal (`refuse_typed_action`: one `Replace`, inside the bound). The
//! client predicts NOTHING for any interaction (`ActionIntent::
//! predicts_nothing`), so every world fact it could not know — the target
//! gone or out of reach on the server, `NotOnThisServer`, the interaction
//! cooldown, a product not ready, a pet not its own — is a `Refused` row
//! alone, uncorrected (`refuse_world_fact`), with the note on an
//! `InteractOutcome { accepted: false }` beside it, as Mirror answers.
//!
//! `mob_interact::run` (and `lead_to_post`, and the riding pass) decides
//! and applies in one call, so the order is: the claim; the pure world-fact
//! gate (target, `NotOnThisServer`, cooldown); then the accepted cost from
//! the pure per-kind table (`mob_interact::accepted_cost`, pinned against
//! `run` by a unit test) staged on the effective window as `Take` +
//! `Grant` (a payment the window can't cover, or shears a pending event wore
//! out or replaced, is a corrected refusal — no other kind is refused for
//! what a pending event did to the hand, run-23 review MEDIUM-1), the
//! events and the result row reserved; then the run, with the server
//! copy's item; a refusal there abandons the reservation exactly (zero
//! effect beyond the cooldown Mirror also starts); an accepted run whose
//! cost differs from the table is a fail-closed pause.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::action_wire::ActionIntent;
use crate::strict_admission::StrictTeardown;
use crate::window_events::{StrictStageError, WindowEvent};

impl HostedServer {
    pub(super) fn serve_strict_interact(&mut self, i: usize, action: ActionHeader, req: &protocol::EntityInteractPacket) -> bool {
        let slot = usize::from(req.hotbar_slot);
        if slot >= 9 {
            self.refuse_typed_action(i, action);
            return false;
        }
        // The historical held item: the server copy's, never the claim.
        let historical = self.server.players[i].inventory.slot(slot).map(|s| s.item.clone());
        if !super::strict_attack::claim_parts_match(historical.as_ref(), req.held_kind, req.held_id, &req.held_full) {
            self.refuse_typed_action(i, action);
            return false;
        }
        // The pure world-fact gate, Mirror's rules (`run_joiner_interaction`).
        if let Some(note) = self.interaction_world_fact(i, req) {
            return self.refuse_interaction_world_fact(i, action, req, note);
        }
        let (consume, give) = crate::mob_interact::accepted_cost(req.kind);
        let mut events = Vec::new();
        if consume > 0 {
            // A cost with nothing in hand can't be paid: the window disagrees.
            let Some(item) = historical.clone() else { self.refuse_typed_action(i, action); return false };
            events.push(WindowEvent::Take { slot, item, n: consume, how: "in an interaction" });
        }
        events.extend(give.iter().cloned().map(WindowEvent::Grant));
        let sp = &self.server.players[i];
        let mut effective = match crate::window_events::strict_effective_marker(sp, &self.server.registry) {
            Ok(effective) => effective,
            Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "interaction"),
            Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(StrictStageError::Unrepresentable) => { self.refuse_typed_action(i, action); return false; }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Interaction item lineage is uncertain"); return false; }
        };
        // C3d run 24 (run-23 review MEDIUM-1) — the one tool an interaction
        // uses without spending it, shears, must still be in the named slot
        // once the server's pending events apply (a last-point pair worn out
        // by a pending event can't shear). Its type is compared, not only its
        // tier (`ItemRef` names a tool by tier). No other kind uses what is in
        // hand without spending it: a Lead removal needs only "not a Lead"
        // and a mount only an empty or a full hand, so a pending event that
        // took the hand's last unit never refuses them.
        if matches!(req.kind, protocol::InteractKind::Shear) {
            let held_now = effective.view().inv.slot(slot).map(|s| s.item.clone());
            if !same_tool(held_now.as_ref(), historical.as_ref()) {
                self.refuse_typed_action(i, action);
                return false;
            }
        }
        let events = events.into_boxed_slice();
        if !events.is_empty() {
            match crate::window_events::stage_strict_effects(sp, &self.server.registry, &events) {
                Ok(()) => {}
                Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "interaction"),
                Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
                // The item isn't there to pay, or a product can't be carried:
                // deterministic on this state, corrected (nothing predicted).
                Err(StrictStageError::Unrepresentable | StrictStageError::MissingPayment) => { self.refuse_typed_action(i, action); return false; }
                Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Interaction payment or pending fit is uncertain"); return false; }
            }
        }
        let first = sp.window_events.last_number();
        let Some(included) = u32::try_from(events.len()).ok().and_then(|n| first.checked_add(n)) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Interaction window event numbers are exhausted"); return false;
        };
        // Every packet is serialised before anything is reserved.
        let take_event = if consume > 0 { first + 1 } else { 0 };
        let mut grants = Vec::with_capacity(give.len());
        for (k, stack) in give.iter().enumerate() {
            let number = first + 1 + u32::from(consume > 0) + k as u32;
            let Some(packet) = grant_packet(stack, number) else { self.refuse_typed_action(i, action); return false };
            grants.push(packet);
        }
        let wanted = events.len();
        let prepared = if events.is_empty() {
            None
        } else {
            let Ok(prepared) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
                let detail = super::actions::reservation_detail(&self.server.players[i], wanted, "interaction");
                self.pause_typed_action(i, StrictTeardown::FellBehind, &detail); return false;
            };
            debug_assert_eq!(prepared.last_number(), included);
            Some(prepared)
        };
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            if let Some(prepared) = prepared { prepared.abandon(&mut self.server.players[i]); }
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Interaction result history is full"); return false;
        };
        // Every check, the events' reservation and the result row precede
        // the run, which decides and applies in one call.
        let ran = self.run_joiner_interaction_with(i, req, historical);
        let done = match ran {
            Some(r) if r.done => r,
            ran => {
                // Refused by a world fact inside the run: nothing was applied
                // (the run's refusals change nothing), so the reservation goes
                // back exactly and the note reaches the client.
                drop(result);
                if let Some(prepared) = prepared { prepared.abandon(&mut self.server.players[i]); }
                let note = ran.map_or(crate::mob_interact::InteractNote::None, |r| r.note);
                return self.refuse_interaction_world_fact(i, action, req, note);
            }
        };
        if done.consume != consume || done.give != give {
            // The pinned table and the run disagree: the run already applied,
            // so nothing else commits (fail-closed until recovery).
            if let Some(prepared) = prepared { prepared.abandon(&mut self.server.players[i]); }
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Interaction cost differs from its table"); return false;
        }
        let note = done.note.to_wire();
        let outcome = protocol::InteractOutcomePacket { seq: req.seq, entity: req.entity, kind: Some(req.kind), accepted: true, consume_held: consume, note, window_event: take_event };
        let packet = protocol::serialize_packet(protocol::PacketType::InteractOutcome, &outcome);
        // Serialised after the run (the note is the run's); an outcome is a
        // few dozen bytes, far inside a journal page.
        debug_assert!(packet.len() <= crate::action_journal::MAX_PAGE_BYTES);
        let now = self.server.tick_counter;
        let sp = &mut self.server.players[i];
        if let Some(prepared) = prepared { prepared.commit(sp, now); }
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.send_to_joined_slot(i, &packet);
        for grant in grants { self.send_to_joined_slot(i, &grant); }
        self.strict_fold_holds(i, "interaction")
    }

    /// A world fact refuses joiner `i`'s interaction: its note on an
    /// `InteractOutcome { accepted: false }` (Mirror's answer), then the
    /// `Refused` row alone, uncorrected (the client predicted nothing).
    fn refuse_interaction_world_fact(&mut self, i: usize, action: ActionHeader, req: &protocol::EntityInteractPacket, note: crate::mob_interact::InteractNote) -> bool {
        self.send_outcome(i, req.seq, req.entity, Some(req.kind), false, 0, note.to_wire(), 0);
        self.refuse_world_fact(i, action, &ActionIntent::Interact(req.clone()))
    }

    /// The world facts Mirror's `run_joiner_interaction` refuses on before
    /// it runs anything, read without changing anything: the target (a mob
    /// in reach; a mount may be a parked cart; a post within block reach),
    /// a world that doesn't simulate the animal life the kind needs
    /// (`NotOnThisServer`), the interaction cooldown. `Some(note)` refuses.
    fn interaction_world_fact(&self, i: usize, req: &protocol::EntityInteractPacket) -> Option<crate::mob_interact::InteractNote> {
        use crate::mob_interact::InteractNote;
        let sp = self.server.players.get(i)?;
        match req.kind {
            protocol::InteractKind::LeadToPost { post } => {
                let centre = glam::Vec3::new(post[0] as f32 + 0.5, post[1] as f32 + 0.5, post[2] as f32 + 0.5);
                if !block_change_within_reach((centre - sp.player.eye_pos()).length_squared(), 0, 0, true) { return Some(InteractNote::None); }
                if !self.server.animal_life_simulated { return Some(InteractNote::NotOnThisServer); }
            }
            protocol::InteractKind::Mount => {
                if sp.riding.is_some() { return Some(InteractNote::None); }
                let mount = match self.joiner_target(i, req.entity) {
                    Some((e, kind)) if crate::mob::is_rideable(kind) => e,
                    Some(_) => return Some(InteractNote::None),
                    None => match self.joiner_cart(i, req.entity) { Some((e, _)) => e, None => return Some(InteractNote::None) },
                };
                // C3d run 24 (run-23 review LOW-3) — Mirror's order
                // (`run_joiner_mount`): a steed or cart someone else rides is
                // `MountTaken` (the toast) before the cooldown is read.
                if let Err(crate::riding::MountRefusal::Taken) = crate::riding::may_mount(&self.server.ecs, mount) {
                    return Some(InteractNote::MountTaken);
                }
            }
            kind => {
                if self.joiner_target(i, req.entity).is_none() { return Some(InteractNote::None); }
                if !self.server.animal_life_simulated && crate::mob_interact::needs_animal_life(kind) {
                    return Some(InteractNote::NotOnThisServer);
                }
            }
        }
        (sp.interact_cooldown > 0).then_some(InteractNote::None)
    }
}

/// The shears' check (run-23 review MEDIUM-1): the same tool type and
/// material in both, durability aside; anything else is not the same tool.
fn same_tool(now: Option<&crate::item::Item>, then: Option<&crate::item::Item>) -> bool {
    match (now, then) {
        (Some(crate::item::Item::Tool(a)), Some(crate::item::Item::Tool(b))) => a.tool_type == b.tool_type && a.material == b.material,
        _ => false,
    }
}
