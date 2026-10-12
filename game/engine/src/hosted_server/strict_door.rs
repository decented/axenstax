//! C3d run 18 — Decision 20 on a Strict seat: a door placement is ONE
//! compound `DoorPlace`, a door break derives its pair once, and a
//! door/gate/trapdoor click is a typed `ToggleBlock`. Every affected cell's
//! existing permission is preflighted before any effect; reach applies to
//! the clicked primary only; the whole action refuses if any companion is
//! forbidden. Refusals are corrected (the client predicted the door): the
//! affected cells sent back, then the `Refused` row and one `Replace`.
//!
//! C3d run 19 — a Mirror joiner's toggle is a typed `ToggleBlock` too, served
//! by the same arm under its own rule (`FlipEach`, uncorrected refusals), so
//! a door opens for everyone.
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::block::{AIR, OAK_DOOR};
use crate::strict_admission::{RefusalKind, StrictTeardown};
use crate::window_events::StrictStageError;

/// The edit-budget credits one door placement or door break takes (two
/// cells), required in full before it runs.
pub(super) const DOOR_EDIT_CREDITS: usize = 2;

impl HostedServer {
    /// The existing permission a joiner's edit of `cell` (holding `current`)
    /// needs, without reach: in the world, loaded, not bedrock, the world's
    /// play mode, a foreign plot, a foreign economy block.
    pub(super) fn cell_permitted(&self, i: usize, cell: [i32; 3]) -> Result<(), EditRefusal> {
        let sp = self.server.players.get(i).ok_or(EditRefusal::Malformed)?;
        let cs = crate::chunk::CHUNK_SIZE as i32;
        if cell[1] < 0 || cell[1] >= 6 * cs { return Err(EditRefusal::OutOfWorld); }
        if !self.server.loaded_columns.contains(&(cell[0].div_euclid(cs), cell[2].div_euclid(cs))) { return Err(EditRefusal::Unloaded); }
        let current = self.server.world.get_block(cell[0], cell[1], cell[2]);
        if current == crate::block::BEDROCK { return Err(EditRefusal::Protected); }
        if sp.server_simulated {
            self.remote_may_touch(sp, cell[0], cell[2])?;
            let npub = verified_npub(sp.verified_pubkey);
            if !economy_owner_allows(&self.server.world, (cell[0], cell[1], cell[2]), current, npub.as_deref()) {
                return Err(EditRefusal::EconomyOwner);
            }
        }
        Ok(())
    }

    /// C3d run 20 — what a toggle of `cell` needs: a toggle is a USE, not
    /// an edit, so its play-mode rule is the single-player toggle arm's
    /// (`strict_doors::toggle_mode_allows`: an Adventure player opens
    /// doors), and the server's other checks stay as an edit's, in an
    /// edit's order: in the world, loaded, not bedrock, then (with
    /// `reach_slot`, the clicked primary only) reach with the server copy's
    /// item in that hotbar slot, then the play mode, a foreign plot and a
    /// foreign economy block. Structurally derived companions pass `None`.
    fn toggle_cell_permitted(&self, i: usize, cell: [i32; 3], reach_slot: Option<usize>) -> Result<(), EditRefusal> {
        let sp = self.server.players.get(i).ok_or(EditRefusal::Malformed)?;
        let cs = crate::chunk::CHUNK_SIZE as i32;
        if cell[1] < 0 || cell[1] >= 6 * cs { return Err(EditRefusal::OutOfWorld); }
        if !self.server.loaded_columns.contains(&(cell[0].div_euclid(cs), cell[2].div_euclid(cs))) { return Err(EditRefusal::Unloaded); }
        let current = self.server.world.get_block(cell[0], cell[1], cell[2]);
        if current == crate::block::BEDROCK { return Err(EditRefusal::Protected); }
        if let Some(slot) = reach_slot {
            let (kind, id) = sp.inventory.slot(slot).map_or(protocol::ItemRef::Empty.to_wire(), |s| crate::inventory::item_to_ref(&s.item).to_wire());
            let d = glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5) - sp.player.eye_pos();
            if !block_change_within_reach(d.length_squared(), kind, id, false) { return Err(EditRefusal::Reach); }
        }
        if sp.server_simulated {
            if !crate::strict_doors::toggle_mode_allows(self.server.play_mode) { return Err(EditRefusal::PlayMode); }
            self.remote_plot_allows(sp, cell[0], cell[2])?;
            let npub = verified_npub(sp.verified_pubkey);
            if !economy_owner_allows(&self.server.world, (cell[0], cell[1], cell[2]), current, npub.as_deref()) {
                return Err(EditRefusal::EconomyOwner);
            }
        }
        Ok(())
    }

    /// The server's live matching half of the door at `cell`, if any (the
    /// pair a break derives, `clear_door_pair`'s rule).
    pub(super) fn live_door_pair(&self, cell: [i32; 3]) -> Option<[i32; 3]> {
        let world = &self.server.world;
        if world.get_block(cell[0], cell[1], cell[2]) != OAK_DOOR { return None; }
        let top = crate::block_shape::door_is_top(world.meta_at(cell[0], cell[1], cell[2]));
        let y = cell[1].checked_add(if top { -1 } else { 1 })?;
        (world.get_block(cell[0], y, cell[2]) == OAK_DOOR && crate::block_shape::door_is_top(world.meta_at(cell[0], y, cell[2])) != top)
            .then_some([cell[0], y, cell[2]])
    }

    /// Put each of `cells` back as the server holds it (block and meta).
    pub(super) fn send_back_cells(&mut self, cells: &[[i32; 3]], budget: &mut EditTickBudget) {
        for c in cells {
            let bc = protocol::BlockChange { x: c[0], y: c[1], z: c[2], new_block: AIR, meta: 0 };
            self.send_back_authoritative_block(&bc, budget);
        }
    }

    /// A refused door action: its cells sent back, then the corrected
    /// `Refused` row (`refuse_typed_action_at`). `false` stops the queue.
    fn refuse_door(&mut self, i: usize, action: ActionHeader, cells: &[[i32; 3]], budget: &mut EditTickBudget, why: &str, kind: RefusalKind) -> bool {
        log::debug!("Strict door action at {:?} refused: {why}", cells.first());
        self.send_back_cells(cells, budget);
        self.refuse_typed_action_at(i, action, cells, kind);
        false
    }

    /// The cells a superseded door action's client predicted (sent back by
    /// the `Superseded` path): a placement's two halves, a toggle's
    /// footprint as the server holds it.
    pub(super) fn door_intent_cells(&self, intent: &crate::action_wire::ActionIntent) -> Vec<[i32; 3]> {
        match intent {
            crate::action_wire::ActionIntent::DoorPlace { bottom, .. } => vec![*bottom, [bottom[0], bottom[1].saturating_add(1), bottom[2]]],
            crate::action_wire::ActionIntent::ToggleBlock { cell, expected_block, .. } => crate::strict_doors::toggle_sendback_cells(&self.server.world, *cell, *expected_block),
            _ => Vec::new(),
        }
    }

    /// Decision 20 — ONE compound door placement: the server copy's slot
    /// must place a door; both halves, the orientation, the headroom and
    /// every cell's permission (reach to the bottom only) are checked; one
    /// door is taken; both cells and one row commit together. The caller
    /// charged [`DOOR_EDIT_CREDITS`]. `false` stops this tick's queue.
    pub(super) fn serve_strict_door_place(&mut self, i: usize, action: ActionHeader, bottom: [i32; 3], facing: u8, hotbar_slot: u8, budget: &mut EditTickBudget) -> bool {
        let top = [bottom[0], bottom[1].saturating_add(1), bottom[2]];
        let cells = [bottom, top];
        let slot = usize::from(hotbar_slot);
        if slot >= 9 { return self.refuse_door(i, action, &cells, budget, "invalid action slot", RefusalKind::Rule); }
        let Some([(_, bottom_meta), (_, top_meta)]) = crate::strict_doors::door_halves(bottom, facing) else {
            return self.refuse_door(i, action, &cells, budget, "a door faces a horizontal direction", RefusalKind::Rule);
        };
        let held = self.server.players[i].inventory.slot(slot).map(|s| s.item.clone());
        if held.as_ref().and_then(crate::joiner_inventory::placeable_block) != Some(OAK_DOOR) {
            return self.refuse_door(i, action, &cells, budget, "the server's copy of the slot doesn't hold a door", RefusalKind::Rule);
        }
        let held_wire = held.as_ref().map_or((protocol::item_kind::EMPTY, 0), |item| crate::inventory::item_to_ref(item).to_wire());
        let lower = protocol::BlockChange { x: bottom[0], y: bottom[1], z: bottom[2], new_block: OAK_DOOR, meta: bottom_meta };
        let upper = protocol::BlockChange { x: top[0], y: top[1], z: top[2], new_block: OAK_DOOR, meta: top_meta };
        // Reach (the server copy's slot) and permission for the bottom;
        // permission without reach for the structurally derived top.
        if let Err(why) = self.validate_block_edit(i, &lower, held_wire, slot) {
            return self.refuse_door(i, action, &cells, budget, &format!("bottom: {why:?}"), RefusalKind::Rule);
        }
        if let Err(why) = self.cell_permitted(i, top) {
            return self.refuse_door(i, action, &cells, budget, &format!("top: {why:?}"), RefusalKind::Rule);
        }
        // The honest client sent this only on a replaceable bottom with AIR
        // above (its own checks), so anything else here is the world having
        // changed under it: stale-world (the intent carries no `seen`).
        let old = self.server.world.get_block(bottom[0], bottom[1], bottom[2]);
        let old_meta = self.server.world.meta_at(bottom[0], bottom[1], bottom[2]);
        if !crate::strict_edits::placement_replaceable(old) || self.server.world.get_block(top[0], top[1], top[2]) != AIR {
            return self.refuse_door(i, action, &cells, budget, "no room for the door", RefusalKind::StaleWorld);
        }
        let sp = &self.server.players[i];
        let place: Box<dyn FnOnce(&mut crate::window::WindowMut)> = Box::new(move |v: &mut crate::window::WindowMut| { crate::strict_edits::place_window(v.inv, slot); });
        let after = match crate::window_events::stage_strict_edit(sp, &self.server.registry, place, &[]) {
            Ok(after) => after,
            Err(StrictStageError::Debt) => return self.strict_debt_at_staging(i, "door"),
            Err(StrictStageError::RawUnrepresentable) => {
                self.send_back_cells(&cells, budget);
                self.refuse_raw_unrepresentable(i, action);
                return false;
            }
            Err(StrictStageError::Unrepresentable) => return self.refuse_door(i, action, &cells, budget, "a place can't be represented exactly", RefusalKind::Rule),
            Err(_) => { self.pause_typed_action(i, StrictTeardown::FellBehind, "Door source or pending fit is uncertain"); return false; }
        };
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result_at(i, action, ActionFate::Accepted, included, &cells) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Door result history is full"); return false;
        };
        // Effects: both cells, raw R (one door), the row; then the shared
        // world finish for each half.
        self.apply_edit_world(&lower, old, true, false);
        self.apply_edit_world(&upper, AIR, true, false);
        let (inv, armour, cursor, grid) = after.into_parts();
        let sp = &mut self.server.players[i];
        sp.inventory = inv; sp.armour = armour; sp.cursor = cursor; sp.craft_grid = grid;
        #[cfg(test)]
        super::actions::commit_fault::after_commit(sp);
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        self.finish_edit_world(&lower, old, old_meta, true, false);
        self.finish_edit_world(&upper, AIR, 0, true, true);
        self.strict_fold_holds(i, "door")
    }

    /// A typed toggle of `cell`'s door, gate or trapdoor, from either kind
    /// of seat: the shared footprint derived from the server's world, reach
    /// to the primary with the server copy's item in `hotbar_slot` (an
    /// edit's reach rule, so a Reach Claw counts; run 19), every other
    /// cell's permission preflighted. No window change. Run 20: a toggle is
    /// a use, not an edit — its play-mode rule is the single-player toggle
    /// arm's ([`Self::toggle_cell_permitted`]), so an Adventure joiner opens
    /// doors.
    ///
    /// Strict (Decision 20): the canonical open bit from the primary.
    /// Mirror (run 19): `FlipEach`, the rule its client predicted,
    /// byte-for-byte the old local click. A refusal in either mode sends
    /// the cells back and retires `Refused`, uncorrected (run-18 review M1:
    /// nothing in the window was predicted). `false` stops the queue.
    pub(super) fn serve_toggle(&mut self, i: usize, action: ActionHeader, cell: [i32; 3], expected: crate::block::BlockId, hotbar_slot: u8, budget: &mut EditTickBudget) -> bool {
        let strict = self.strict_slot(i);
        let block = self.server.world.get_block(cell[0], cell[1], cell[2]);
        let cells = crate::strict_doors::toggle_footprint(&self.server.world, cell);
        // Run 19 (run-18 review M1/L2): a toggle changes no window, so a
        // refusal in either mode is the cells sent back (the union of the
        // server's footprint and the client's possible one) and the
        // `Refused` row — no whole-window `Replace`, no tick of the
        // correction bound. Strict still tallies it.
        let back = crate::strict_doors::toggle_sendback_cells(&self.server.world, cell, expected);
        let refuse = |hs: &mut Self, why: &str, kind: RefusalKind, budget: &mut EditTickBudget| -> bool {
            log::debug!("Toggle at {cell:?} refused: {why}");
            if strict { hs.server.players[i].strict_refusals.note(kind); }
            hs.send_back_cells(&back, budget);
            hs.complete_typed_noop_at(i, action, ActionFate::Refused, &back)
        };
        // Run 15's rule: stale-world only when the block id differs.
        if block != expected { return refuse(self, "the cell no longer holds what the client toggled", RefusalKind::StaleWorld, budget); }
        if !crate::block_shape::is_toggleable(block) { return refuse(self, "not a toggleable block", RefusalKind::Rule, budget); }
        let slot = usize::from(hotbar_slot);
        if slot >= 9 { return refuse(self, "invalid action slot", RefusalKind::Rule, budget); }
        let rule = if strict { crate::strict_doors::OpenRule::Canonical } else { crate::strict_doors::OpenRule::FlipEach };
        let metas = crate::strict_doors::toggle_metas(&self.server.world, cell, rule);
        // Run 20: a use's permission, not an edit's (no edit play-mode gate).
        if let Err(why) = self.toggle_cell_permitted(i, cell, Some(slot)) {
            return refuse(self, &format!("primary: {why:?}"), RefusalKind::Rule, budget);
        }
        if let Err(why) = cells[1..].iter().try_for_each(|c| self.toggle_cell_permitted(i, *c, None)) {
            return refuse(self, &format!("{why:?}"), RefusalKind::Rule, budget);
        }
        let included = self.server.players[i].window_events.last_number();
        let Some(result) = self.prepare_typed_result_at(i, action, ActionFate::Accepted, included, &cells) else {
            self.pause_typed_action(i, StrictTeardown::FellBehind, "Toggle result history is full"); return false;
        };
        for (c, m) in metas {
            let pos = (c[0], c[1], c[2]);
            self.server.world.set_meta(pos, m);
            self.server.world.notify_neighbours(pos);
            let b = self.server.world.get_block(c[0], c[1], c[2]);
            self.pending_block_changes.push(protocol::BlockChange { x: c[0], y: c[1], z: c[2], new_block: b, meta: m });
            if self.lends_host_world() { self.lent_edit_cells.push(pos); }
        }
        self.complete_typed_result(i, action, ActionFate::Accepted, result);
        true
    }
}
