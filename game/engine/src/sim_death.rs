//! One bounded authority death pass, shared with standalone/offline play.
//! Rescue precedes despawn; all ownership and cargo follow exact entity IDs.
//! Callers reserve local and remote delivery before entering this pass. A
//! full queue defers the whole pass, leaving deaths pending in the ECS.

use crate::{
    combat::{KillCredit, MobDeath},
    mob::MobType,
    server::DeathLocation,
    world::World,
};
use glam::Vec3;

/// Smaller than the 1024-event window ledger: a mass death is spread across
/// ticks, including the costly Pet Bed scans, without discarding any victim.
pub const MAX_DEATHS_PER_PHASE: usize = 32;
/// Fixed-data notices own no strings, items or cargo. Charge Vec capacity,
/// rather than serialised length, against this retained local-effect budget.
pub const MAX_LOCAL_BYTES: usize = 16 * 1024;
pub const MAX_REMOTE_EVENTS: usize = 256;
pub const MAX_REMOTE_BYTES: usize = 16 * 1024;
/// Shared transient preflight reason; retry classification uses explicit
/// runtime state, never comparison of user-facing error strings.
pub const DEATHS_PENDING: &str = "Deaths are still being resolved.";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeathNotice {
    Rescued {
        entity: hecs::Entity,
        kind: MobType,
        pos: Vec3,
    },
    Killed {
        death: MobDeath,
        location: DeathLocation,
        tamed: bool,
        credit: KillCredit,
    },
}

/// Capacity is reserved before any victim is healed, removed or credited.
/// Only this module can append notices, at most one for each selected entity.
#[derive(Clone, Debug, PartialEq)]
pub struct DeathBatch(Vec<DeathNotice>);
impl DeathBatch {
    pub fn reserve() -> Option<Self> {
        let mut notices = Vec::new();
        notices.try_reserve_exact(MAX_DEATHS_PER_PHASE).ok()?;
        let batch = Self(notices);
        (batch.owned_bytes() <= MAX_LOCAL_BYTES).then_some(batch)
    }
    pub fn owned_bytes(&self) -> usize {
        self.0.capacity() * std::mem::size_of::<DeathNotice>()
    }
    pub fn notices(&self) -> &[DeathNotice] {
        &self.0
    }
    pub fn retain_local(&mut self) {
        self.0.retain(|notice| {
            matches!(
                notice,
                DeathNotice::Rescued { .. }
                    | DeathNotice::Killed {
                        credit: KillCredit::Local(_),
                        ..
                    }
            )
        });
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub struct DeathContext<'a> {
    pub tick: u64,
    pub world_time: u32,
    /// Slot indexed, with absent/remote slots marked dead. Never compact.
    pub locals: &'a [(Vec3, bool)],
    pub joiners: &'a [crate::combat::JoinerBody],
}

/// Freeze rescue eligibility for every corpse before any reservation or
/// batch selection. This per-body metadata cannot age while a queue is full.
pub fn observe_pending(ecs: &hecs::World, tick: u64) {
    for (entity, health) in ecs.query::<&mut crate::combat::Health>().iter() {
        if health.is_dead() {
            if health.rescue_eligible.is_none() {
                health.rescue_eligible = Some(crate::tameable::eligible_for_pet_bed_rescue(
                    ecs, entity, tick,
                ));
            }
        } else {
            health.rescue_eligible = None;
        }
    }
}

pub fn has_pending(ecs: &hecs::World) -> bool {
    ecs.query::<&crate::combat::Health>()
        .iter()
        .any(|(_, h)| h.is_dead())
}

pub fn sweep(
    world: &mut World,
    ecs: &mut hecs::World,
    ctx: DeathContext<'_>,
    batch: &mut DeathBatch,
) {
    assert!(batch.is_empty(), "a death reservation is consumed once");
    observe_pending(ecs, ctx.tick);
    // Stack-owned selection: unselected entities stay dead for a later pass.
    let mut selected = [None; MAX_DEATHS_PER_PHASE];
    for (dest, (entity, _)) in selected.iter_mut().zip(
        ecs.query::<&crate::combat::Health>()
            .iter()
            .filter(|(_, h)| h.is_dead()),
    ) {
        *dest = Some(entity);
    }
    // Rescue first, so neither drop snapshots nor bookkeeping include it.
    for entity in selected.iter().flatten().copied() {
        if !ecs
            .get::<&crate::combat::Health>(entity)
            .unwrap()
            .rescue_eligible
            .unwrap_or(false)
        {
            continue;
        }
        let (kind, pos) = {
            let (Ok(kind), Ok(pos)) = (
                ecs.get::<&crate::entity::MobKind>(entity),
                ecs.get::<&crate::entity::Position>(entity),
            ) else {
                continue;
            };
            (kind.0, pos.0)
        };
        let Some(bed) = crate::pet_bed::find_pet_bed_near(world, (pos.x, pos.y, pos.z)) else {
            continue;
        };
        let pos = Vec3::new(bed.0 as f32 + 0.5, bed.1 as f32 + 1.0, bed.2 as f32 + 0.5);
        ecs.get::<&mut crate::entity::Position>(entity).unwrap().0 = pos;
        {
            let mut health = ecs.get::<&mut crate::combat::Health>(entity).unwrap();
            health.current = crate::mob::mob_def(kind).health as f32;
            health.rescue_eligible = None;
        }
        batch.0.push(DeathNotice::Rescued { entity, kind, pos });
    }
    let mut snapshots =
        crate::death_drops::snapshot_entities(ecs, selected.iter().flatten().copied());
    for entity in selected.into_iter().flatten() {
        let tamed = crate::tameable::pet_owner_of(ecs, entity).is_some();
        let home = ecs
            .get::<&crate::brigand::HomeHideout>(entity)
            .ok()
            .map(|h| h.anchor);
        let raid = ecs
            .get::<&crate::raid::RaidMember>(entity)
            .ok()
            .map(|r| r.raid_id);
        let Some(death) = crate::combat::despawn_one_dead(ecs, entity) else {
            continue;
        };
        let location = DeathLocation::from_world(world, death.pos);
        let credit =
            crate::combat::attribute_kill(death.attacker, death.pos, ctx.locals, ctx.joiners);
        crate::death_drops::spawn_drops_for_death(
            ecs,
            world,
            &mut snapshots,
            death,
            ctx.world_time,
        );
        if crate::brigand::Tier::from_mob_type(death.kind).is_some()
            && let Some(home) = home
        {
            crate::brigand_hideout_gen::on_brigand_killed(world, home);
        }
        if let Some(raid) = raid {
            record_raid_death(world, raid, death, ctx.locals);
        }
        batch.0.push(DeathNotice::Killed {
            death,
            location,
            tamed,
            credit,
        });
    }
}

/// Keep the existing defender policy: nearest living local inside the
/// village radius, except that a joiner's (including departed) hit never
/// awards a local contribution. No joiner monetary settlement is added.
fn record_raid_death(
    world: &mut World,
    id: crate::raid::RaidId,
    death: MobDeath,
    locals: &[(Vec3, bool)],
) {
    let Some(raid) = crate::raid::find_raid_mut(world, id) else {
        return;
    };
    let a = raid.village_anchor;
    let anchor = Vec3::new(a[0] as f32 + 0.5, a[1] as f32 + 0.5, a[2] as f32 + 0.5);
    let mut nearest = None;
    for (slot, &(pos, dead)) in locals.iter().enumerate() {
        if dead
            || death.attacker.is_some_and(|a| a.is_joiner())
            || (pos - anchor).length() > crate::raid::DEFENDER_RADIUS_BLOCKS
        {
            continue;
        }
        let distance = (pos - death.pos).length();
        if nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((slot, distance));
        }
    }
    raid.mobs_alive = raid.mobs_alive.saturating_sub(1);
    if let Some((slot, _)) = nearest {
        crate::raid::record_kill(&mut raid.contribution_table, slot);
        if raid.mobs_alive == 0 {
            raid.killing_blow = Some(slot);
        }
    }
}

#[cfg(test)]
mod tests;
