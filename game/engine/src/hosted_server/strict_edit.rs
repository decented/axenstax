//! C3d run 6 — typed block edits in Strict: a plain placement and a mined
//! break, judged from the server's own copy of the joiner's window, never
//! from the hand or tool its client claims. The world half is the shared
//! `apply_edit_world` / `finish_edit_world` the Mirror path runs.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::strict_admission::StrictTeardown;
use crate::window_events::{StrictStageError, WindowEvent};

/// What a Strict edit is, read from raw R at the action's boundary.
#[derive(Debug)]
enum StrictEdit {
    /// A block-item placement; the server-held hotbar slot places exactly
    /// the edit's block.
    Place { slot: usize },
    /// A mined break with the server-held tool in the slot (`None`: the
    /// slot holds no tool, a bare hand).
    Break { slot: usize, tool: Option<crate::crafting::Tool> },
    /// Definitively invalid: a typed `Refused`, zero effect.
    Refuse(&'static str),
    /// A category this run doesn't type yet (creative free edit, a same-block
    /// Noop, a door, a non-yielding manual break, a use): paused, as every
    /// untyped mutation still is.
    Unsupported(&'static str),
}

/// Decision 18 — the classification, exhaustive for this class. `held` is the
/// server copy's item in the action's slot; the client's claimed hand and
/// `MinedBlock.tool` are never read.
fn judge(old: crate::block::BlockId, change: &protocol::BlockChange, mined: Option<&protocol::MinedBlock>, slot: usize, held: Option<&crate::item::Item>, creative: bool) -> StrictEdit {
    use crate::block::{AIR, OAK_DOOR, WATER};
    let new = change.new_block;
    if creative { return StrictEdit::Unsupported("Creative free edits await their typed class"); }
    if old == new { return StrictEdit::Unsupported("A same-block edit awaits the typed Noop class"); }
    // Decision 20: a door is one compound action (DoorPlace / its pair).
    if old == OAK_DOOR || new == OAK_DOOR { return StrictEdit::Unsupported("Doors are one compound action"); }
    if let Some(m) = mined {
        if (m.x, m.y, m.z) != (change.x, change.y, change.z) { return StrictEdit::Refuse("mined tag names another cell"); }
        if crate::break_drops::yields_drops(old) && (new == AIR || crate::growth::is_crop(old)) {
            let tool = match held { Some(crate::item::Item::Tool(t)) => Some(*t), _ => None };
            return StrictEdit::Break { slot, tool };
        }
        return StrictEdit::Unsupported("A non-yielding manual break awaits its typed class");
    }
    if (old == AIR || old == WATER) && new != AIR {
        // Heldness: the historical slot must place exactly this block.
        return if held.and_then(crate::joiner_inventory::placeable_block) == Some(new) {
            StrictEdit::Place { slot }
        } else {
            StrictEdit::Refuse("the server's copy of the slot doesn't hold the placed block")
        };
    }
    // Untagged emptying edits, untagged in-place changes: no mutation authority.
    StrictEdit::Refuse("an untagged edit is no placement or break")
}

impl HostedServer {
    /// Refuse a typed edit: the sender's predicted cell is put back by the
    /// existing block-correction path, then the typed `Refused` row and the
    /// correction that must follow it (`refuse_typed_action`).
    fn refuse_strict_edit(&mut self, i: usize, action: ActionHeader, change: &protocol::BlockChange, budget: &mut EditTickBudget, why: &str) -> bool {
        log::debug!("Strict edit at ({}, {}, {}) refused: {why}", change.x, change.y, change.z);
        self.send_back_refused_edit(change, budget);
        self.refuse_typed_action(i, action);
        false
    }

    /// One typed Strict edit at the queue head. Every check, the window
    /// staging, the event and result reservations and the grant packets
    /// precede the first effect; then the world, raw R, the numbered grants
    /// and the terminal row commit together. `false` stops this tick's queue.
    pub(super) fn serve_strict_edit(&mut self, i: usize, action: ActionHeader, change: &protocol::BlockChange, hand: protocol::EditHand, mined: Option<&protocol::MinedBlock>, budget: &mut EditTickBudget) -> bool {
        let slot = usize::from(hand.0);
        // Decision 8: an invalid action slot refuses; no latest-slot fallback.
        if slot >= 9 { return self.refuse_strict_edit(i, action, change, budget, "invalid action slot"); }
        let held = self.server.players[i].inventory.slot(slot).map(|s| s.item.clone());
        let held_wire = held.as_ref().map_or((protocol::item_kind::EMPTY, 0), |item| crate::inventory::item_to_ref(item).to_wire());
        if let Err(why) = self.validate_block_edit(i, change, held_wire) {
            return self.refuse_strict_edit(i, action, change, budget, &format!("{why:?}"));
        }
        let old = self.server.world.get_block(change.x, change.y, change.z);
        let old_meta = self.server.world.meta_at(change.x, change.y, change.z);
        let edit = judge(old, change, mined, slot, held.as_ref(), self.server.play_mode.is_creative());
        let (direct, grants, yielded): (Box<dyn FnOnce(&mut crate::window::WindowMut)>, Vec<crate::item::ItemStack>, bool) = match edit {
            StrictEdit::Refuse(why) => return self.refuse_strict_edit(i, action, change, budget, why),
            StrictEdit::Unsupported(why) => { self.pause_typed_action(i, StrictTeardown::Untyped, why); return false; }
            StrictEdit::Place { slot } => (Box::new(move |v: &mut crate::window::WindowMut| { v.inv.take_placeable_from_hotbar(slot); }), Vec::new(), false),
            StrictEdit::Break { slot, tool } => {
                // The yield's tier is the server-held tool's, never the claim.
                let Some(y) = self.joiner_break_yield(change, old, tool) else {
                    // Decision 19 Drift: the server's cell disagrees with the break.
                    return self.refuse_strict_edit(i, action, change, budget, "the server's cell doesn't break that way");
                };
                let grants: Vec<_> = y.drops.into_iter().chain(y.gem).filter(|s| s.count > 0).collect();
                let wear: Box<dyn FnOnce(&mut crate::window::WindowMut)> = if tool.is_some() {
                    Box::new(move |v: &mut crate::window::WindowMut| { v.inv.use_tool_at(slot); })
                } else { Box::new(|_: &mut crate::window::WindowMut| {}) };
                (wear, grants, true)
            }
        };
        let events: Box<[WindowEvent]> = grants.iter().cloned().map(WindowEvent::Grant).collect();
        let sp = &self.server.players[i];
        let after = match crate::window_events::stage_strict_edit(sp, &self.server.registry, direct, &events) {
            Ok(after) => after,
            Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "edit"),
            Err(StrictStageError::Unrepresentable) => return self.refuse_strict_edit(i, action, change, budget, "a place can't be represented exactly"),
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Edit source or pending fit is uncertain"); return false; }
        };
        let first = sp.window_events.last_number();
        let Some(included) = u32::try_from(events.len()).ok().and_then(|n| first.checked_add(n)) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Window event numbers are exhausted"); return false;
        };
        // Every grant packet is serialised before anything is reserved.
        let mut packets = Vec::with_capacity(grants.len());
        for (k, stack) in grants.iter().enumerate() {
            let Some(packet) = grant_packet(stack, first + 1 + k as u32) else {
                self.pause_typed_action(i, StrictTeardown::FellBehind, "Edit yield can't be carried"); return false;
            };
            packets.push(packet);
        }
        let Ok(prepared) = crate::window_events::prepare_strict_events(&mut self.server.players[i], events) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Whole edit event reservation is unavailable"); return false;
        };
        debug_assert_eq!(prepared.last_number(), included);
        let Some(result) = self.prepare_typed_result(i, action, ActionFate::Accepted, included) else {
            prepared.abandon(&mut self.server.players[i]);
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Edit result history is full"); return false;
        };
        // Effects: the world, raw R, the numbered yield, the terminal row.
        let now = self.server.tick_counter;
        self.apply_edit_world(change, old, true, yielded);
        if yielded { crate::break_drops::mark_exposed_neighbours(&mut self.server.world, change.x, change.y, change.z, now); }
        let (inv, armour, cursor, grid) = after.into_parts();
        let sp = &mut self.server.players[i];
        sp.inventory = inv; sp.armour = armour; sp.cursor = cursor; sp.craft_grid = grid;
        prepared.commit(sp, now);
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.finish_edit_world(change, old, old_meta, true, false);
        for packet in packets { self.send_to_joined_slot(i, &packet); }
        self.strict_fold_holds(i, "edit")
    }
}

