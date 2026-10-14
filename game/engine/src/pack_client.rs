//! Joined pack entry points and checked canonical window application.
use crate::{container_window::{ContainerTarget, SharedContainer}, item::{Item, ItemStack}, pack_wire::{PackOperation, PackResult, PackView, PackViewPush}, window::WindowMut};
#[cfg(test)]
mod tests;

fn remember_window(view: &WindowMut, uses: &crate::use_edits::SentUses) -> Result<(), String> {
    for stack in view.inv.slots_iter().flatten().chain(view.cursor.iter()).chain(view.grid.iter().flatten().flatten()) {
        if let Item::Plan(body) = &stack.item { crate::plan_store::remember_checked(body, 0)?; }
    }
    for body in uses.private_plans() { crate::plan_store::remember_checked(body, 0)?; }
    Ok(())
}

/// Validate and retain every private body before replacing one physical place.
/// Real bodies already held are moved, not cloned. SentUses/history is untouched.
pub(crate) fn apply_canonical_window(view: &mut WindowMut, packet: &crate::pack_wire::PackWindow, registry: &crate::block::BlockRegistry, uses: &crate::use_edits::SentUses, debt: &mut crate::container_window::CorrectionDebt, mode: crate::action_identity::InventoryMode) -> Result<(), String> {
    let mut decoded = packet.decode_window(registry)?;
    remember_window(view, uses)?;
    let refill = view.inv.auto_refill;
    let locks = view.inv.locked_indices();
    let held_keys: Vec<_> = view.inv.slots_iter().flatten().chain(view.cursor.iter()).chain(view.grid.iter().flatten().flatten()).filter_map(|stack| match &stack.item {
        Item::Plan(body) if body.marker.is_none() => Some((crate::plan::marker(body), body.develop_state.is_developed())),
        _ => None,
    }).collect();
    // Resolve only genuinely new local bodies, with a bounded additional
    // allocation. Existing Mirror bodies are moved without a new Strict cap.
    let mut additional = 0usize;
    fn resolve(stack: &mut Option<ItemStack>, held: &[([u8; 32], bool)], additional: &mut usize) {
        let Some(ItemStack { item: Item::Plan(stub), .. }) = stack else { return };
        let marker = crate::plan::marker(stub);
        let developed = stub.develop_state.is_developed();
        if held.contains(&(marker, developed)) { return; }
        if let Some(body) = crate::plan_store::resolve(marker)
            && body.develop_state.is_developed() == developed
            && let Ok(bytes) = crate::body_residency::allocated_body_bytes(&body)
            && bytes <= crate::action_capacity::MAX_PRIVATE_BODY_BYTES
            && additional.checked_add(bytes).is_some_and(|sum| sum <= crate::action_capacity::MAX_RESIDENT_BODY_BYTES)
        {
            *additional += bytes;
            *stub = body;
        }
    }
    for at in 0..36 {
        let mut stack = decoded.window.inv.take_slot(at);
        resolve(&mut stack, &held_keys, &mut additional);
        decoded.window.inv.set_slot(at, stack);
    }
    resolve(&mut decoded.window.cursor, &held_keys, &mut additional);
    for stack in decoded.window.grid.iter_mut().flatten() { resolve(stack, &held_keys, &mut additional); }
    decoded.window.inv.auto_refill = refill;
    if mode == crate::action_identity::InventoryMode::Mirror { decoded.window.inv.set_locked_from(&locks); }

    let mut old_inv = std::mem::replace(view.inv, decoded.window.inv);
    let old_cursor = std::mem::replace(view.cursor, decoded.window.cursor);
    let old_grid = std::mem::replace(view.grid, decoded.window.grid);
    *view.armour = decoded.window.armour;
    let mut bodies = Vec::with_capacity(held_keys.len());
    for stack in (0..36).filter_map(|at| old_inv.take_slot(at)).chain(old_cursor).chain(old_grid.into_iter().flatten().flatten()) {
        if let Item::Plan(body) = stack.item && body.marker.is_none() { bodies.push(((crate::plan::marker(&body), body.develop_state.is_developed()), body)); }
    }
    fn move_body(stack: &mut Option<ItemStack>, bodies: &mut Vec<(([u8; 32], bool), crate::plan::PlanData)>) {
        let Some(ItemStack { item: Item::Plan(stub), .. }) = stack else { return };
        if stub.marker.is_none() { return; }
        let marker = crate::plan::marker(stub);
        let developed = stub.develop_state.is_developed();
        if let Some(at) = bodies.iter().position(|(key, _)| *key == (marker, developed)) {
            *stub = bodies.swap_remove(at).1;
        }
    }
    for at in 0..36 {
        let mut stack = view.inv.take_slot(at);
        move_body(&mut stack, &mut bodies);
        view.inv.set_slot(at, stack);
    }
    move_body(view.cursor, &mut bodies);
    for stack in view.grid.iter_mut().flatten() { move_body(stack, &mut bodies); }
    // Unused old bodies were retained successfully before the marker fence.
    *debt = decoded.debt;
    Ok(())
}

impl crate::GameState {
    /// Independent of advertised Strict. It gates before prediction/mutation.
    pub(crate) fn pack_window_frozen(&self, pidx: usize) -> bool {
        pidx == 0 && self.remote_client.as_ref().is_some_and(|remote| remote.pack_request_pending())
    }

    pub(crate) fn request_pack_action(&mut self, pidx: usize, target: ContainerTarget, operation: PackOperation) -> bool {
        if pidx != 0 || self.players.is_empty() || self.players[pidx].is_dead() || self.pack_window_frozen(pidx) { return false; }
        let Some(remote) = self.remote_client.as_ref().filter(|remote| remote.is_connected()) else { return false };
        let actor_respawns = remote.applied_actor_respawns();
        let external = remote.pack_retained_usage();
        if self.window_inbox.add_bounded(Vec::new(), Vec::new(), Vec::new(), (external.0, external.1.saturating_add(self.chunk_intake.carried_metadata_bytes()))).is_err() { return false; }
        let p = &self.players[pidx];
        let held = p.inventory.slots_iter().flatten().chain(p.crafting_ui.cursor_item.iter()).chain(p.crafting_ui.grid.iter().flatten().flatten());
        for stack in held {
            if let Item::Plan(body) = &stack.item && let Err(error) = crate::plan_store::remember_checked(body, 0) {
                log::warn!("Pack request could not retain its private Plan: {error}");
                self.toast = Some(("A local Plan could not be saved; try again.".into(), web_time::Instant::now() + web_time::Duration::from_secs(3)));
                return false;
            }
        }
        for body in self.sent_uses.private_plans() {
            if let Err(error) = crate::plan_store::remember_checked(body, 0) { log::warn!("Pack request could not retain use history: {error}"); return false; }
        }
        let Some(raw) = crate::staged_window::MarkerWindow::try_capture_parts(&p.inventory, &p.armour_slots, &p.crafting_ui.cursor_item, &p.crafting_ui.grid, &self.registry) else { return false };
        let request = crate::pack_wire::PackRequest { actor_respawns, target, hotbar_slot: p.hotbar_slot.min(8) as u8, before: raw.digest_with(p.crafting_ui.station(), None), operation };
        let reserved = self.remote_client.as_mut().expect("connected remote was checked").reserve_pack_request(request);
        match reserved {
            Ok(action) => { self.send_request(crate::remote_client::Request::Action(action)); true }
            Err(error) => { log::debug!("Pack request could not be reserved: {error}"); false }
        }
    }

    fn install_pack_view(&mut self, view: Option<PackView>, actor_respawns: u32) -> Result<(), String> {
        let p = &mut self.players[0];
        let hidden = p.shared_container.as_ref().is_some_and(|mirror| view.as_ref().is_some_and(|view| mirror.target == view.target && mirror.pack_session.is_some_and(|session| session.incarnation == view.incarnation && session.close_requested)));
        match view {
            Some(view) => {
                let mut mirror = SharedContainer::from_pack(&view, actor_respawns, &self.registry)?;
                if let Some(session) = &mut mirror.pack_session { session.close_requested = hidden; }
                p.open_chest = None;
                p.open_dispenser = None;
                p.open_furnace = None;
                p.shared_container = Some(mirror);
                if !hidden { self.release_cursor(); }
            }
            None => {
                if p.shared_container.as_ref().is_some_and(|mirror| matches!(mirror.target, ContainerTarget::Entity(_))) { p.shared_container = None; }
            }
        }
        Ok(())
    }

    pub(crate) fn apply_pack_result(&mut self, packet: &PackResult) -> Result<(), String> {
        let Some(remote) = self.remote_client.as_ref() else { return Ok(()) };
        if packet.actor_respawns != remote.applied_actor_respawns() { return Ok(()); }
        if let Some(view) = &packet.view { view.decode_contents(&self.registry)?; }
        let mode = remote.inventory_mode;
        let p = &mut self.players[0];
        apply_canonical_window(&mut WindowMut { inv: &mut p.inventory, armour: &mut p.armour_slots, cursor: &mut p.crafting_ui.cursor_item, grid: &mut p.crafting_ui.grid, container: None }, &packet.window, &self.registry, &self.sent_uses, &mut self.window_inbox.debt, mode)?;
        p.crafting_ui.update_result();
        self.install_pack_view(packet.view.clone(), packet.actor_respawns)?;
        let message = packet.note.message();
        if !message.is_empty() { self.toast = Some((message.into(), web_time::Instant::now() + web_time::Duration::from_secs(2))); }
        Ok(())
    }

    pub(crate) fn apply_pack_view(&mut self, packet: &PackViewPush) -> Result<(), String> {
        let Some(remote) = self.remote_client.as_ref() else { return Ok(()) };
        if !remote.pack_view_namespace_matches(packet) { return Err("Pack view changed its action namespace".into()); }
        if packet.actor_respawns != remote.applied_actor_respawns() { return Ok(()); }
        let Some(current) = self.players[0].shared_container.as_ref() else { return Ok(()) };
        let Some(session) = current.pack_session else { return Ok(()) };
        if current.target != packet.target || session.incarnation != packet.incarnation { return Ok(()); }
        self.install_pack_view(packet.view.clone(), packet.actor_respawns)
    }

    /// Rendering never owns cargo. Mutate the real current ECS pack once
    /// per click, then advance the changed revision for shared viewers.
    pub(crate) fn apply_local_pack_clicks(&mut self, pidx: usize, entity: hecs::Entity, clicks: &[crate::container_window::ContainerClick]) {
        if self.joined() || self.players[pidx].is_dead() { return; }
        let ctx = self.container_ctx(pidx, false);
        for click in clicks {
            let Some(next_revision) = crate::pack_state::revision(&self.ecs, entity).checked_add(1) else { return };
            let changed = {
                let Ok(mut data) = self.ecs.get::<&mut crate::horse_ai::HorseData>(entity) else { return };
                let Some(pack) = data.pack.as_mut() else { return };
                let p = &mut self.players[pidx];
                let mut view = WindowMut { inv: &mut p.inventory, armour: &mut p.armour_slots, cursor: &mut p.crafting_ui.cursor_item, grid: &mut p.crafting_ui.grid, container: Some(crate::container_window::ContainerMut::Chest(pack)) };
                !crate::container_window::apply_container(&mut view, click, &ctx).touched.is_empty()
            };
            if changed { let _ = self.ecs.insert_one(entity, crate::pack_state::PackRevision(next_revision)); }
        }
    }

    pub(crate) fn draw_joined_pack(&mut self, pidx: usize, mut mirror: SharedContainer) {
        let Some(session) = mirror.pack_session else { return };
        if session.close_requested { self.players[pidx].shared_container = Some(mirror); return; }
        let crate::container_window::ContainerData::Chest(data) = &mirror.contents else { return };
        let result = crate::chest_ui::show_pack_dialog(&self.renderer.egui.ctx, "Pack", data, &self.players[pidx].inventory, egui::Id::new(("joined-pack", pidx, mirror.target, session.incarnation)), !self.pack_window_frozen(pidx));
        let ctx = self.container_ctx(pidx, true);
        for click in &result.clicks { self.apply_shared_container_click(pidx, &mut mirror, click, &ctx); }
        self.players[pidx].shared_container = Some(mirror);
        if result.remove_pack_requested {
            self.request_pack_action(pidx, self.players[pidx].shared_container.as_ref().expect("drawn mirror").target, PackOperation::Remove { incarnation: session.incarnation, revision: session.revision });
        } else if result.close_requested {
            self.request_pack_close(pidx);
            if pidx == 0 { self.capture_cursor(); }
        }
    }

    pub(crate) fn request_pack_close(&mut self, pidx: usize) {
        let Some(mirror) = self.players[pidx].shared_container.as_mut() else { return };
        let Some(session) = mirror.pack_session.as_mut() else { return };
        session.close_requested = true;
        let target = mirror.target;
        let incarnation = session.incarnation;
        self.request_pack_action(pidx, target, PackOperation::Close { incarnation });
    }

    pub(crate) fn tick_pack_mirror(&mut self, pidx: usize) -> bool {
        let Some(mirror) = self.players[pidx].shared_container.as_ref() else { return false };
        let Some(session) = mirror.pack_session else { return false };
        let current_life = self.remote_client.as_ref().map_or(u32::MAX, |remote| remote.applied_actor_respawns());
        if !self.joined() || session.actor_respawns != current_life || self.players[pidx].is_dead() {
            self.players[pidx].shared_container = None;
            return true;
        }
        let ContainerTarget::Entity(id) = mirror.target else { return false };
        let reachable = self.remote_mobs.position_of(id).is_some_and(|pos| self.players[pidx].player.eye_pos().distance(pos + glam::Vec3::Y * 0.75) <= crate::combat::ATTACK_REACH);
        if session.close_requested || !reachable { self.request_pack_close(pidx); }
        true
    }
}
