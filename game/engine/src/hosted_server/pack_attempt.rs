//! Prepare one backed attempt; scratch cargo never becomes a second ledger.
use super::*;
use crate::{container_window::{ContainerData, ContainerTarget}, pack_state::{CargoCommit, PackSession}, pack_wire::{PackNote, PackOperation, PackRequest, PackView}, staged_window::StagedClick, window_events::PackWindowStage};

pub(super) enum CargoEffect {
    None,
    Equip { entity: hecs::Entity, before_revision: u64, revision: u64, cargo: crate::chest::ChestData },
    Patch { entity: hecs::Entity, before_revision: u64, revision: u64, patch: Box<CargoCommit> },
    Remove { entity: hecs::Entity, before_revision: u64, revision: u64 },
}
pub(super) struct Attempt {
    pub window: PackWindowStage,
    pub view: Option<PackView>,
    pub session: Option<PackSession>,
    pub effect: CargoEffect,
    pub spill: Option<crate::item::ItemStack>,
}

impl HostedServer {
    pub(super) fn prepare_pack_attempt(&self, i: usize, action: crate::action_identity::ActionHeader, request: &PackRequest) -> Result<Attempt, PackNote> {
        let sp = &self.server.players[i];
        let raw = crate::window_events::pack_raw_marker(sp, &self.server.registry).map_err(|_| PackNote::Updated)?;
        if !sp.is_present_and_alive() || request.actor_respawns != sp.respawns || sp.window_events.predates_replace(action.events_applied) { return Err(PackNote::Dead); }
        if !sp.window_events.accepts_boundary(action.events_applied) { return Err(PackNote::Updated); }
        if request.hotbar_slot >= 9 || raw.digest_with(sp.station, None) != request.before { return Err(PackNote::Updated); }
        if matches!(request.operation, PackOperation::Close { .. }) {
            let PackOperation::Close { incarnation } = request.operation else { unreachable!() };
            if sp.pack_session.is_some_and(|session| session.target != request.target || session.incarnation != incarnation) { return Err(PackNote::Updated); }
            return Ok(Attempt { window: crate::window_events::pack_canonical_window(sp, &self.server.registry).map_err(|_| PackNote::Updated)?, view: None, session: None, effect: CargoEffect::None, spill: None });
        }
        let ContainerTarget::Entity(id) = request.target else { return Err(PackNote::NoPack) };
        let initial = matches!(request.operation, PackOperation::Equip | PackOperation::Open);
        let entity = if initial {
            self.joiner_target(i, id).map(|(entity, _)| entity).ok_or(PackNote::OutOfReach)?
        } else {
            let session = sp.pack_session.filter(|session| session.target == request.target && session.actor_respawns == sp.respawns).ok_or(PackNote::NoPack)?;
            self.live_pack_target(i, session)?;
            session.entity
        };
        self.pack_target_policy(i, entity)?;
        let kind = self.server.ecs.get::<&crate::entity::MobKind>(entity).map_err(|_| PackNote::NoPack)?.0;
        if !crate::mob::can_carry_pack(kind) { return Err(PackNote::Species); }
        let data = self.server.ecs.get::<&crate::horse_ai::HorseData>(entity).map_err(|_| PackNote::NoPack)?;
        if !data.is_kept() { return Err(PackNote::NeedsKeeper); }
        if data.pack.as_ref().is_some_and(|pack| pack.tier != crate::chest::ChestTier::Wood || pack.slots.len() != 27) { return Err(PackNote::Updated); }
        let current_revision = crate::pack_state::revision(&self.server.ecs, entity);
        let mut staged = StagedClick { after: raw.clone(), result: crate::window::ClickResult::Done, sources: std::array::from_fn(|_| None) };
        let (effect, session, view, spill) = match &request.operation {
            PackOperation::Equip => {
                if data.pack.is_some() { return Err(PackNote::Updated); }
                let slot = usize::from(request.hotbar_slot);
                let before = raw.slots();
                if !matches!(&before[slot], Some(stack) if stack.item_kind == protocol::item_kind::BLOCK && stack.item_id == block::CHEST && stack.count != 0 && stack.full_item == protocol::WireItem::None) { return Err(PackNote::Updated); }
                let mut source = before[slot].clone().expect("checked named Chest");
                source.count = 1;
                staged.sources[slot] = Some(source);
                if staged.after.view().inv.take_block_from_hotbar(slot) != Some(block::CHEST) { return Err(PackNote::Updated); }
                let revision = current_revision.checked_add(1).ok_or(PackNote::Waiting)?;
                (CargoEffect::Equip { entity, before_revision: current_revision, revision, cargo: crate::chest::ChestData::new() }, None, None, None)
            }
            PackOperation::Open => {
                let pack = data.pack.as_ref().ok_or(PackNote::NoPack)?;
                let incarnation = sp.pack_next_incarnation.checked_add(1).ok_or(PackNote::Waiting)?;
                let session = PackSession { entity, target: request.target, incarnation, revision: current_revision, actor_respawns: sp.respawns };
                let projected = crate::pack_state::project(pack, &self.server.registry).map_err(|_| PackNote::Updated)?;
                let view = PackView::capture(request.target, incarnation, current_revision, &projected).map_err(|_| PackNote::Updated)?;
                (CargoEffect::None, Some(session), Some(view), None)
            }
            PackOperation::Click { incarnation, revision, click } => {
                let mut session = sp.pack_session.filter(|session| session.entity == entity && session.incarnation == *incarnation).ok_or(PackNote::Updated)?;
                if *revision != current_revision || session.revision != *revision { return Err(PackNote::Updated); }
                let pack = data.pack.as_ref().ok_or(PackNote::NoPack)?;
                let mut candidate = ContainerData::Chest(crate::pack_state::project(pack, &self.server.registry).map_err(|_| PackNote::Updated)?);
                let ctx = crate::window::ClickCtx::new(self.server.play_mode.is_creative(), crate::window::Station::Player, sp.player.eye_pos(), |_| block::AIR).with_shared(true);
                staged = crate::staged_window::stage_container(&raw, &mut candidate, click, &ctx);
                if staged.result != crate::window::ClickResult::Done { return Err(PackNote::Updated); }
                let ContainerData::Chest(candidate) = candidate else { unreachable!() };
                let patch = CargoCommit::prepare(pack, candidate).map_err(|_| PackNote::Updated)?;
                let revision = if patch.changed() { current_revision.checked_add(1).ok_or(PackNote::Waiting)? } else { current_revision };
                session.revision = revision;
                let view = patch.view(request.target, session.incarnation, revision).map_err(|_| PackNote::Updated)?;
                (CargoEffect::Patch { entity, before_revision: current_revision, revision, patch: Box::new(patch) }, Some(session), Some(view), None)
            }
            PackOperation::Remove { incarnation, revision } => {
                let session = sp.pack_session.filter(|session| session.entity == entity && session.incarnation == *incarnation).ok_or(PackNote::Updated)?;
                if *revision != current_revision || session.revision != *revision { return Err(PackNote::Updated); }
                let pack = data.pack.as_ref().ok_or(PackNote::NoPack)?;
                if pack.slots.iter().any(Option::is_some) { return Err(PackNote::NotEmpty); }
                let revision = current_revision.checked_add(1).ok_or(PackNote::Waiting)?;
                let spill = staged.after.view().inv.add_item(crate::item::ItemStack::new_block(block::CHEST, 1));
                (CargoEffect::Remove { entity, before_revision: current_revision, revision }, None, None, spill)
            }
            PackOperation::Close { .. } => unreachable!(),
        };
        let window = crate::window_events::admit_pack_staged(sp, &self.server.registry, &raw, &staged).map_err(|_| PackNote::Updated)?;
        Ok(Attempt { window, view, session, effect, spill })
    }
}
