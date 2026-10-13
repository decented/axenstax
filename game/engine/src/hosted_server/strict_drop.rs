//! C3d run 25 — a Strict seat's typed Q-drop (`ItemAction::Drop` inside
//! `ActionIntent::Item`).
//!
//! The one PREDICTED item action: the client takes one of the held item
//! from its hand (`Inventory::take_one_from_hotbar`) when it reserves the
//! drop, as a typed placement takes its block (`strict_edits.rs`). So the
//! server pays it the same way: the item is the server copy's item in the
//! NAMED hotbar slot (a claim that disagrees with it, a slot past the
//! hotbar, an empty slot or a Plan — which a joined client keeps, with a
//! toast — is a window disagreement), the take is staged on the copy
//! (`window_events::stage_strict_edit`, the client's own
//! `take_one_from_hotbar`, no numbered event: the client already made it),
//! and only then is the item thrown from the server's body as a real
//! ground item. Every refusal is corrected (`refuse_typed_action`: one
//! `Replace` puts the item back): there is no uncorrected world fact for
//! a drop, because the client predicted its take.
//!
//! Mirror's bounded belief (`spawn_joiner_drop`: the shortfall charged to
//! the believed-units bound) is not reached from a Strict seat: the copy
//! pays, or the drop is refused. The pacing is Mirror's drop bucket, by
//! deferral, not refusal: a typed drop at the head of the queue waits
//! there until the bucket holds a token (`process_typed_actions`), so an
//! honest client paced by its own `drop_ready_tick` is never refused for
//! clock jitter.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::strict_admission::StrictTeardown;
use crate::window_events::StrictStageError;

impl HostedServer {
    /// A Strict seat's typed `Drop` from hotbar `hotbar_slot`. `false`
    /// stops this tick's queue.
    pub(super) fn serve_strict_drop(&mut self, i: usize, action: ActionHeader, req: &protocol::ItemActionPacket) -> bool {
        let protocol::ItemAction::Drop { hotbar_slot, held_kind, held_id, held_full } = &req.action else { unreachable!("a Drop") };
        let slot = usize::from(*hotbar_slot);
        if slot >= 9 { self.refuse_typed_action(i, action); return false; }
        let historical = self.server.players[i].inventory.slot(slot).map(|s| s.item.clone());
        if !super::strict_attack::claim_parts_match(historical.as_ref(), *held_kind, *held_id, held_full) {
            self.refuse_typed_action(i, action); return false;
        }
        // Nothing in the slot, or a Plan (a joined client keeps a Plan:
        // `JOINED_PLAN_DROP_TOAST`, nothing sent): only a modified client.
        if historical.as_ref().is_none_or(|item| matches!(item, crate::item::Item::Plan(_))) {
            self.refuse_typed_action(i, action); return false;
        }
        // The client's own take, staged on the copy (Decision 6/7: the
        // named slot's raw units, backed through any pending event).
        let sp = &self.server.players[i];
        let mut thrown = None;
        let staged = crate::window_events::stage_strict_edit(sp, &self.server.registry, |v: &mut crate::window::WindowMut| {
            thrown = v.inv.take_one_from_hotbar(slot);
        }, &[]);
        let after = match staged {
            Ok(after) => after,
            Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "drop"),
            Err(StrictStageError::RawUnrepresentable) => { self.refuse_raw_unrepresentable(i, action); return false; }
            Err(StrictStageError::Unrepresentable | StrictStageError::MissingPayment) => { self.refuse_typed_action(i, action); return false; }
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Drop source is uncertain"); return false; }
        };
        // The staged take is from the marker copy; the thrown stack is the
        // server copy's real item (a tool whole, with its wear).
        let Some(item) = historical.filter(|_| thrown.is_some()) else { self.refuse_typed_action(i, action); return false };
        let stack = crate::item::ItemStack { item, count: 1 };
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Drop result history is full"); return false;
        };
        // Every check and the result row precede the effects: raw R, the
        // drop bucket's token, the thrown item, the terminal row.
        let now = self.server.tick_counter;
        let (inv, armour, cursor, grid) = after.into_parts();
        let sp = &mut self.server.players[i];
        sp.inventory = inv; sp.armour = armour; sp.cursor = cursor; sp.craft_grid = grid;
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        let _ = sp.drop_bucket.take(now);
        sp.possession.drops = sp.possession.drops.saturating_add(1);
        let (pos, velocity) = crate::entity::q_drop_launch(sp.player.eye_pos(), crate::camera::forward_from(sp.yaw, sp.pitch));
        crate::entity::spawn_thrown_item(&mut self.server.ecs, pos, velocity, stack, i as u8);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.strict_fold_holds(i, "drop")
    }
}
