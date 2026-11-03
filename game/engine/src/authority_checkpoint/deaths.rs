//! Deaths waiting to be swept. A mob at no health stays in the ECS until the next
//! death pass has room for it (`sim_death`): 32 a pass, and only when the kill
//! credit it owes has somewhere to go. Mobs aren't otherwise in a checkpoint, but
//! what that pass will do is state (loot, kill credit, a hideout's or raid's count,
//! a Pet Bed rescue, a pack's cargo), so each corpse is written with what the pass
//! reads off it, and comes back as a corpse for the restored world's next pass.
//!
//! The count has no limit of its own: the checkpoint's cap bounds it.
//!
//! A pet's row in `world.dat` was taken when it last lived. It would come back
//! alive beside its own corpse, so the nearest living mob of its kind and owner
//! goes: pets have no stable id to match on.

use glam::Vec3;

use super::codec::{Malformed, Reader, Writer};
use crate::combat::{Attacker, Health, LastAttacker};
use crate::entity::{MobKind, Position};
use crate::server::GameServer;

/// Most of these belong to the corpse's own entity, as `sim_death::sweep` finds them.
pub(super) struct Corpse {
    kind: crate::mob::MobType,
    pos: Vec3,
    attacker: Option<Attacker>,
    rescue_eligible: Option<bool>,
    cull_tick: Option<u64>,
    home: Option<[i32; 3]>,
    raid: Option<crate::raid::RaidId>,
    /// A tamed or authored mob, as a save writes it: its owner and its cargo.
    row: Option<crate::save::SavedTamedPet>,
}

impl Corpse {
    pub(super) fn is_placed(&self) -> bool {
        self.pos.is_finite()
    }
}

fn write_attacker(w: &mut Writer, attacker: Attacker) {
    match attacker {
        Attacker::Local(slot) => {
            w.u8(0);
            w.u32(slot as u32);
        }
        Attacker::Remote { slot, generation } => {
            w.u8(1);
            w.u32(slot as u32);
            w.u32(generation);
        }
        Attacker::Departed => w.u8(2),
    }
}

fn read_attacker(r: &mut Reader<'_>) -> Result<Attacker, Malformed> {
    Ok(match r.u8()? {
        0 => Attacker::Local(r.u32()? as usize),
        1 => Attacker::Remote { slot: r.u32()? as usize, generation: r.u32()? },
        2 => Attacker::Departed,
        _ => return Err(Malformed("an unknown attacker")),
    })
}

/// One corpse's bytes, read from its entity.
fn encode(s: &GameServer, entity: hecs::Entity, health: &Health) -> Result<Option<Vec<u8>>, String> {
    let ecs = &s.ecs;
    // Anything else with health is removed by the pass and produces nothing.
    let (Ok(kind), Ok(pos)) = (ecs.get::<&MobKind>(entity), ecs.get::<&Position>(entity)) else {
        return Ok(None);
    };
    let mut w = Writer::new();
    // `MobType` holds no float and no container: a fixed encoding.
    w.blob(&bincode::serialize(&kind.0).map_err(|e| e.to_string())?);
    w.vec3(pos.0);
    w.opt(ecs.get::<&LastAttacker>(entity).ok().map(|a| a.0), write_attacker);
    w.opt(health.rescue_eligible, |w, eligible| w.bool(eligible));
    w.opt(ecs.get::<&crate::tameable::DeliberateCull>(entity).ok().map(|m| m.last_hit_tick), |w, tick| w.u64(tick));
    w.opt(ecs.get::<&crate::brigand::HomeHideout>(entity).ok().map(|h| h.anchor), |w, [x, y, z]| w.pos((x, y, z)));
    w.opt(ecs.get::<&crate::raid::RaidMember>(entity).ok().map(|m| m.raid_id), |w, raid| w.u32(raid));
    let row = crate::save::pet_rows(ecs, Some(entity))?.into_iter().next();
    let row = row.map(|row| bincode::serialize(&row).map_err(|e| e.to_string())).transpose()?;
    w.opt(row.as_deref(), |w, row| w.blob(row));
    w.finish().map(Some).map_err(|bytes| format!("a corpse of {bytes} bytes"))
}

fn decode(r: &mut Reader<'_>) -> Result<Corpse, Malformed> {
    let kind = bincode::deserialize(r.blob()?).map_err(|_| Malformed("an unknown mob kind"))?;
    Ok(Corpse {
        kind,
        pos: r.vec3()?,
        attacker: r.opt(read_attacker)?,
        rescue_eligible: r.opt(|r| r.bool())?,
        cull_tick: r.opt(|r| r.u64())?,
        home: r.opt(|r| {
            let (x, y, z) = r.pos()?;
            Ok([x, y, z])
        })?,
        raid: r.opt(|r| r.u32())?,
        row: r.opt(|r| bincode::deserialize(r.blob()?).map_err(|_| Malformed("a saved pet that doesn't decode")))?,
    })
}

/// Every corpse, sorted by its bytes: the ECS's order is no state.
pub(super) fn write(w: &mut Writer, s: &GameServer) -> Result<(), String> {
    let mut corpses = Vec::new();
    for (entity, health) in s.ecs.query::<&Health>().iter() {
        if health.is_dead()
            && let Some(bytes) = encode(s, entity, health)?
        {
            corpses.push(bytes);
        }
    }
    corpses.sort_unstable();
    w.count(corpses.len());
    for corpse in &corpses {
        w.blob(corpse);
    }
    Ok(())
}

pub(super) fn read(r: &mut Reader<'_>) -> Result<Vec<Corpse>, Malformed> {
    let n = r.count(4)?;
    let mut last: Option<&[u8]> = None;
    let mut corpses = Vec::new();
    for _ in 0..n {
        let bytes = r.blob()?;
        // Not strictly: two cows can die alike in one place.
        if last.is_some_and(|last| last > bytes) {
            return Err(Malformed("corpses that aren't in their one sorted order"));
        }
        last = Some(bytes);
        let mut one = Reader::new(bytes);
        corpses.push(decode(&mut one)?);
        if !one.is_empty() {
            return Err(Malformed("bytes left over after a corpse"));
        }
    }
    Ok(corpses)
}

/// Each corpse back in the ECS, for the next death pass.
pub(super) fn apply(corpses: &[Corpse], s: &mut GameServer) -> Result<(), String> {
    for c in corpses {
        let ecs = &mut s.ecs;
        let entity = match &c.row {
            Some(row) => crate::steed_save::spawn_pet(ecs, row, false)?,
            None => crate::entity::spawn_mob(ecs, c.kind, c.pos),
        };
        {
            let mut health = ecs.get::<&mut Health>(entity).map_err(|e| e.to_string())?;
            health.current = 0.0;
            health.rescue_eligible = c.rescue_eligible;
        }
        if let Some(attacker) = c.attacker {
            let _ = ecs.insert_one(entity, LastAttacker(attacker));
        }
        if let Some(last_hit_tick) = c.cull_tick {
            let _ = ecs.insert_one(entity, crate::tameable::DeliberateCull { last_hit_tick });
        }
        if let Some(anchor) = c.home {
            let _ = ecs.insert_one(entity, crate::brigand::HomeHideout { anchor });
        }
        if let Some(raid_id) = c.raid {
            let _ = ecs.insert_one(entity, crate::raid::RaidMember { raid_id });
        }
        if c.row.is_some()
            && let Some(living) = living_copy(ecs, entity, c)
        {
            let _ = ecs.despawn(living);
        }
    }
    Ok(())
}

/// The nearest living mob the corpse's saved row would have brought back: of its
/// kind, and of its owner (authored, for a mob with none).
fn living_copy(ecs: &hecs::World, corpse: hecs::Entity, c: &Corpse) -> Option<hecs::Entity> {
    let owner = crate::tameable::pet_owner_of(ecs, corpse);
    let authored = ecs.get::<&crate::entity::Authored>(corpse).is_ok();
    let mut nearest: Option<(f32, hecs::Entity)> = None;
    for (entity, (kind, pos, health)) in ecs.query::<(&MobKind, &Position, &Health)>().iter() {
        if kind.0 != c.kind || health.is_dead() || crate::tameable::pet_owner_of(ecs, entity) != owner {
            continue;
        }
        if owner.is_none() && (!authored || ecs.get::<&crate::entity::Authored>(entity).is_err()) {
            continue;
        }
        let distance = (pos.0 - c.pos).length_squared();
        if nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, entity));
        }
    }
    nearest.map(|(_, entity)| entity)
}
