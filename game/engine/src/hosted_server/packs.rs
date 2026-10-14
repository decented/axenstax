//! Sequential entity pack authority; controls never wait behind its own flight.
use super::*;
use crate::{action_identity::{ActionFate, ActionHeader}, container_window::ContainerTarget, pack_state::{PackFlight, PackSession}, pack_wire::{PackNote, PackRequest, PackResult, PackView, PackViewPush}, window_events::WindowEvent};
use super::pack_attempt::{Attempt, CargoEffect};

impl HostedServer {
    pub(super) fn pack_queue_waits(&self, i: usize) -> bool {
        self.server.players.get(i).and_then(|sp| sp.action_queue.as_ref()).and_then(|queue| queue.front())
            .and_then(|head| crate::action_wire::ActionPacket::decode(&head.payload).ok())
            .is_some_and(|packet| matches!(packet.intent, crate::action_wire::ActionIntent::Pack(_)))
    }

    pub(super) fn pack_target_policy(&self, i: usize, entity: hecs::Entity) -> Result<(), PackNote> {
        if self.server.play_mode.is_spectator() { return Err(PackNote::Denied); }
        let pos = self.server.ecs.get::<&crate::entity::Position>(entity).map_err(|_| PackNote::NoPack)?.0;
        self.remote_plot_allows(&self.server.players[i], pos.x.floor() as i32, pos.z.floor() as i32).map_err(|_| PackNote::Denied)
    }
    pub(super) fn live_pack_target(&self, i: usize, session: PackSession) -> Result<(), PackNote> {
        let sp = &self.server.players[i];
        if !sp.is_present_and_alive() || session.actor_respawns != sp.respawns { return Err(PackNote::Dead); }
        let ContainerTarget::Entity(id) = session.target else { return Err(PackNote::NoPack) };
        let ecs = &self.server.ecs;
        if ecs.get::<&crate::entity::ProtocolId>(session.entity).map(|key| key.0).ok() != Some(id) { return Err(PackNote::NoPack); }
        if ecs.get::<&crate::combat::Health>(session.entity).map_or(true, |health| health.is_dead()) { return Err(PackNote::NoPack); }
        let kind = ecs.get::<&crate::entity::MobKind>(session.entity).map_err(|_| PackNote::NoPack)?.0;
        if !crate::mob::can_carry_pack(kind) { return Err(PackNote::Species); }
        let data = ecs.get::<&crate::horse_ai::HorseData>(session.entity).map_err(|_| PackNote::NoPack)?;
        if !data.is_kept() { return Err(PackNote::NeedsKeeper); }
        let pos = ecs.get::<&crate::entity::Position>(session.entity).map_err(|_| PackNote::NoPack)?.0;
        let height = ecs.get::<&crate::entity::Hitbox>(session.entity).map_or(0.0, |hitbox| hitbox.height);
        if (pos + glam::Vec3::Y * (height * 0.5)).distance(sp.player.eye_pos()) > crate::combat::ATTACK_REACH + ATTACK_REACH_TOLERANCE { return Err(PackNote::OutOfReach); }
        self.pack_target_policy(i, session.entity)
    }
    fn current_pack_view(&self, i: usize) -> Option<PackView> {
        let session = self.server.players[i].pack_session?;
        self.live_pack_target(i, session).ok()?;
        let data = self.server.ecs.get::<&crate::horse_ai::HorseData>(session.entity).ok()?;
        let projected = crate::pack_state::project(data.pack.as_ref()?, &self.server.registry).ok()?;
        PackView::capture(session.target, session.incarnation, crate::pack_state::revision(&self.server.ecs, session.entity), &projected).ok()
    }

    /// Nothing has changed yet: reserve the canonical event and the immutable
    /// result row together, then recheck and commit the real cargo once.
    pub(super) fn serve_pack(&mut self, i: usize, action: ActionHeader, request: &PackRequest) -> bool {
        let sp = &mut self.server.players[i];
        if sp.pack_flight.is_some() || !self.edit_queues[i].is_empty() { return false; }
        if sp.window_events.accepts_boundary(action.events_applied) && !sp.window_events.predates_replace(action.events_applied) {
            crate::window_events::apply_through(sp, action.events_applied, self.server.tick_counter);
        }
        let attempted = self.prepare_pack_attempt(i, action, request);
        let (attempt, note, fate) = match attempted {
            Ok(attempt) => (attempt, PackNote::Done, ActionFate::Accepted),
            Err(note) => {
                let sp = &self.server.players[i];
                let Ok(window) = crate::window_events::pack_canonical_window(sp, &self.server.registry) else {
                    let _ = self.release_slot(i, Some("This world's pack inventory couldn't be read; saved player records are preserved."));
                    return false;
                };
                let view = self.current_pack_view(i);
                let session = view.as_ref().and(sp.pack_session).map(|mut session| { session.revision = view.as_ref().expect("view checked").revision; session });
                let fate = if request.actor_respawns != sp.respawns || sp.window_events.predates_replace(action.events_applied) || !sp.is_present_and_alive() { ActionFate::Superseded } else { ActionFate::Refused };
                (Attempt { window, view, session, effect: CargoEffect::None, spill: None }, note, fate)
            }
        };
        let Attempt { window, view, session, effect, spill } = attempt;
        let carried = match crate::pack_wire::PackWindow::capture(&window.window, &window.debt, &self.server.registry) {
            Ok(carried) => carried,
            Err(error) => { log::warn!("Pack window cannot be carried: {error}"); return false; }
        };
        let event = WindowEvent::PackCanonical { actor_respawns: request.actor_respawns, window: window.window, debt: window.debt };
        let sp = &mut self.server.players[i];
        let Ok(prepared_events) = crate::window_events::prepare_pack_events(sp, Box::new([event])) else { return false };
        let included_events = prepared_events.last_number();
        let result = PackResult { actor_respawns: request.actor_respawns, action, fate, note, window_event: included_events, window: carried, view };
        let carrier = result.encode().map(|bytes| {
            let mut tagged = Vec::with_capacity(bytes.len() + 1);
            tagged.push(protocol::PacketType::PackResult as u8);
            tagged.extend_from_slice(&bytes);
            tagged.into_boxed_slice()
        });
        let prepared_result = carrier.ok().and_then(|carrier| sp.action_results.as_mut().and_then(|results| results.prepare_with_carrier(crate::action_wire::ActionReceipt { action, fate, included_events }, Box::new([]), carrier).ok()));
        let Some(prepared_result) = prepared_result else { prepared_events.abandon(sp); return false };
        // Each mutation still checks the actual generational entity and cargo,
        // not the rendering view or a stored contents snapshot.
        if !self.commit_pack_cargo(i, request, effect) {
            prepared_events.abandon(&mut self.server.players[i]);
            return false;
        }
        let sp = &mut self.server.players[i];
        if request.actor_respawns == sp.respawns {
            sp.pack_session = session;
            if let Some(session) = session { sp.pack_next_incarnation = sp.pack_next_incarnation.max(session.incarnation); }
            crate::window_ops::close_container_view(sp);
        }
        prepared_events.commit(sp, self.server.tick_counter);
        sp.pack_flight = Some(PackFlight { action, event: included_events });
        if let Some(stack) = spill { crate::entity::spawn_thrown_item(&mut self.server.ecs, self.server.players[i].player.eye_pos(), glam::Vec3::ZERO, stack, i as u8); }
        self.complete_typed_result(i, action, fate, prepared_result);
        true
    }

    fn commit_pack_cargo(&mut self, i: usize, request: &PackRequest, effect: CargoEffect) -> bool {
        let (entity, before_revision, revision) = match &effect {
            CargoEffect::None => return true,
            CargoEffect::Equip { entity, before_revision, revision, .. } | CargoEffect::Patch { entity, before_revision, revision, .. } | CargoEffect::Remove { entity, before_revision, revision } => (*entity, *before_revision, *revision),
        };
        let sp = &self.server.players[i];
        if request.actor_respawns != sp.respawns || !sp.is_present_and_alive() || self.pack_target_policy(i, entity).is_err() { return false; }
        let old_revision = crate::pack_state::revision(&self.server.ecs, entity);
        if before_revision != old_revision || revision < old_revision || revision > old_revision.saturating_add(1) { return false; }
        let probe = PackSession { entity, target: request.target, incarnation: 0, revision: old_revision, actor_respawns: request.actor_respawns };
        if self.live_pack_target(i, probe).is_err() { return false; }
        let committed = {
            let Ok(mut data) = self.server.ecs.get::<&mut crate::horse_ai::HorseData>(entity) else { return false };
            if !data.is_kept() { return false; }
            match effect {
                CargoEffect::Equip { cargo, .. } if data.pack.is_none() => { data.pack = Some(cargo); true }
                CargoEffect::Patch { patch, .. } => data.pack.as_mut().is_some_and(|pack| patch.commit(pack)),
                CargoEffect::Remove { .. } if data.pack.as_ref().is_some_and(|pack| pack.slots.iter().all(Option::is_none)) => { data.pack = None; true }
                _ => false,
            }
        };
        if committed { let _ = self.server.ecs.insert_one(entity, crate::pack_state::PackRevision(revision)); }
        committed
    }

    pub(super) fn push_open_packs(&mut self) {
        for i in 0..self.server.players.len() {
            let Some(mut session) = self.server.players[i].pack_session else { continue };
            if !self.handshake_done.get(i).copied().unwrap_or(false) || self.disconnected.get(i).copied().unwrap_or(true) { continue; }
            let view = self.current_pack_view(i);
            if view.as_ref().is_some_and(|view| view.revision == session.revision) { continue; }
            let sp = &mut self.server.players[i];
            let Some(namespace) = sp.action_frontier.as_ref().map(|frontier| frontier.header(0, 0)) else { continue };
            let Ok(events) = crate::window_events::prepare_pack_events(sp, Box::new([WindowEvent::PackView])) else { continue };
            let packet = PackViewPush { actor_respawns: sp.respawns, session: namespace.session, epoch: namespace.epoch, life: namespace.life, window_event: events.last_number(), target: session.target, incarnation: session.incarnation, view };
            let bytes = protocol::serialize_packet(protocol::PacketType::PackView, &packet);
            if bytes.len() > crate::action_journal::MAX_PAGE_BYTES + 1 { events.abandon(sp); continue; }
            events.commit(sp, self.server.tick_counter);
            sp.pack_session = match packet.view.as_ref() {
                Some(view) => { session.revision = view.revision; Some(session) },
                None => None,
            };
            self.send_to_joined_slot(i, &bytes);
        }
    }
}
