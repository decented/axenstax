//! Exact original-giver identity and the appended, bounded world carrier.
use crate::{
    entity::{MobKind, Position, Velocity},
    mob::MobType,
    progression::GameplayId,
    villager::VillagerComponent,
};
use serde::{Deserialize, Serialize};
mod codec;
pub(crate) const NPC_LIMIT: usize = 512;
pub(crate) const WORLD_CARRIER_LIMIT: usize = 512 * 1024;
pub(crate) const GOSSIP_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QuestNpcId(pub GameplayId);

#[derive(Clone, Debug)]
pub struct SavedQuestNpc {
    pub(crate) id: GameplayId,
    pub(crate) kind: MobType,
    pub(crate) pos: [f32; 3],
    pub(crate) velocity: [f32; 3],
    pub(crate) health: [f32; 2],
    pub(crate) health_ticks: [u32; 2],
    pub(crate) component: VillagerComponent,
    pub(crate) authored: bool,
}
impl SavedQuestNpc {
    fn validate(&self) -> Result<(), String> {
        if self.id == [0; 16]
            || !matches!(self.kind, MobType::Villager | MobType::Peddler)
            || self.component.profession == crate::villager::Profession::Builder
            || self.component.commission.is_some()
            || self
                .component
                .gossip_line
                .as_ref()
                .is_some_and(|line| line.len() > GOSSIP_LIMIT)
            || !self
                .pos
                .iter()
                .chain(self.velocity.iter())
                .chain(self.health.iter())
                .all(|value| value.is_finite())
            || self.health[0] <= 0.0
            || self.health[1] <= 0.0
            || self.health[0] > self.health[1]
        {
            return Err("invalid saved ordinary quest giver".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct QuestWorldSave {
    pub(crate) scope: GameplayId,
    pub(crate) npcs: Vec<SavedQuestNpc>,
}
impl QuestWorldSave {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.scope == [0; 16]
            || self.npcs.len() > NPC_LIMIT
            || self.npcs.windows(2).any(|w| w[0].id >= w[1].id)
        {
            return Err("invalid quest world scope or giver identities".into());
        }
        for npc in &self.npcs {
            npc.validate()?;
        }
        Ok(())
    }
}
impl Serialize for QuestWorldSave {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        codec::encode(self)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for QuestWorldSave {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bounded;
        impl<'de> serde::de::Visitor<'de> for Bounded {
            type Value = QuestWorldSave;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("one bounded quest world")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let capacity = sequence.size_hint().unwrap_or(0);
                if capacity > WORLD_CARRIER_LIMIT {
                    return Err(serde::de::Error::custom(
                        "quest world exceeds its byte limit",
                    ));
                }
                let mut bytes = Vec::with_capacity(capacity);
                while let Some(byte) = sequence.next_element::<u8>()? {
                    if bytes.len() >= WORLD_CARRIER_LIMIT {
                        return Err(serde::de::Error::custom(
                            "quest world exceeds its byte limit",
                        ));
                    }
                    bytes.push(byte);
                }
                codec::decode(&bytes).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_seq(Bounded)
    }
}

pub(crate) fn random_id() -> Result<GameplayId, String> {
    let mut value = [0u8; 16];
    getrandom::fill(&mut value).map_err(|e| format!("Couldn't create a gameplay identity: {e}"))?;
    if value == [0; 16] {
        return Err("Couldn't create a non-zero gameplay identity".into());
    }
    Ok(value)
}
fn snapshot(
    ecs: &hecs::World,
    entity: hecs::Entity,
    id: GameplayId,
) -> Result<SavedQuestNpc, String> {
    if ecs.get::<&crate::satoshi::SatoshiMarker>(entity).is_ok() {
        return Err("The guide is outside ordinary quests".into());
    }
    let health = ecs
        .get::<&crate::combat::Health>(entity)
        .map_err(|_| "Quest giver has no health")?;
    let value = SavedQuestNpc {
        id,
        kind: ecs
            .get::<&MobKind>(entity)
            .map_err(|_| "Quest giver has no kind")?
            .0,
        pos: ecs
            .get::<&Position>(entity)
            .map_err(|_| "Quest giver has no position")?
            .0
            .to_array(),
        velocity: ecs
            .get::<&Velocity>(entity)
            .map_err(|_| "Quest giver has no velocity")?
            .0
            .to_array(),
        health: [health.current, health.max],
        health_ticks: [health.flash_timer, health.invincible_timer],
        component: (*ecs
            .get::<&VillagerComponent>(entity)
            .map_err(|_| "Quest giver has no ordinary component")?)
        .clone(),
        authored: ecs.get::<&crate::entity::Authored>(entity).is_ok(),
    };
    value.validate()?;
    Ok(value)
}
pub(crate) fn capture(
    scope: Option<GameplayId>,
    ecs: &hecs::World,
) -> Result<Option<QuestWorldSave>, String> {
    let Some(scope) = scope else {
        if ecs.query::<&QuestNpcId>().iter().next().is_some() {
            return Err("Quest givers have no durable world scope".into());
        }
        return Ok(None);
    };
    let mut npcs = Vec::new();
    for (entity, identity) in ecs.query::<&QuestNpcId>().iter() {
        if npcs.len() >= NPC_LIMIT {
            return Err("Too many engaged quest givers to save".into());
        }
        if ecs
            .get::<&crate::combat::Health>(entity)
            .is_ok_and(|health| health.is_dead())
        {
            continue;
        }
        npcs.push(snapshot(ecs, entity, identity.0)?);
    }
    npcs.sort_by_key(|npc| npc.id);
    let value = QuestWorldSave { scope, npcs };
    value.validate()?;
    Ok(Some(value))
}
pub(crate) fn restore(ecs: &mut hecs::World, saved: &QuestWorldSave) -> Result<(), String> {
    saved.validate()?;
    let mut present = ahash::AHashMap::new();
    for (entity, identity) in ecs.query::<&QuestNpcId>().iter() {
        if present.insert(identity.0, entity).is_some() {
            return Err("Duplicate live quest giver identity".into());
        }
    }
    for npc in &saved.npcs {
        if let Some(entity) = present.get(&npc.id) {
            if ecs
                .get::<&MobKind>(*entity)
                .map_or(true, |kind| kind.0 != npc.kind)
            {
                return Err("Live quest giver identity changed kind".into());
            }
            continue;
        }
        let entity = crate::entity::spawn_mob(ecs, npc.kind, glam::Vec3::from_array(npc.pos));
        ecs.insert(
            entity,
            (
                QuestNpcId(npc.id),
                npc.component.clone(),
                Velocity(glam::Vec3::from_array(npc.velocity)),
                crate::combat::Health {
                    current: npc.health[0],
                    max: npc.health[1],
                    flash_timer: npc.health_ticks[0],
                    invincible_timer: npc.health_ticks[1],
                },
            ),
        )
        .map_err(|e| e.to_string())?;
        if npc.authored {
            ecs.insert_one(entity, crate::entity::Authored)
                .map_err(|e| e.to_string())?;
        }
        present.insert(npc.id, entity);
    }
    Ok(())
}

/// A scoped world cannot be saved through a path which omitted its live ECS capture.
pub(crate) fn check_capture(
    scope: Option<GameplayId>,
    saved: Option<&QuestWorldSave>,
) -> Result<(), String> {
    match (scope, saved) {
        (None, None) => Ok(()),
        (Some(expected), Some(saved)) if expected == saved.scope => saved.validate(),
        _ => Err("Quest world capture does not match the live authority scope".into()),
    }
}

/// Prepare identity and its complete save carrier without changing the entity.
pub(crate) fn prepare_engagement(
    scope: GameplayId,
    ecs: &hecs::World,
    entity: hecs::Entity,
) -> Result<GameplayId, String> {
    if let Ok(existing) = ecs.get::<&QuestNpcId>(entity) {
        snapshot(ecs, entity, existing.0)?;
        return Ok(existing.0);
    }
    let id = random_id()?;
    let mut carrier = capture(Some(scope), ecs)?.ok_or("missing scoped quest carrier")?;
    if carrier.npcs.len() >= NPC_LIMIT || carrier.npcs.iter().any(|npc| npc.id == id) {
        return Err("Quest giver identities are full or collided".into());
    }
    carrier.npcs.push(snapshot(ecs, entity, id)?);
    carrier.npcs.sort_by_key(|npc| npc.id);
    codec::encode(&carrier)?;
    Ok(id)
}
pub(crate) fn commit_engagement(
    ecs: &mut hecs::World,
    entity: hecs::Entity,
    id: GameplayId,
) -> Result<(), String> {
    if let Ok(existing) = ecs.get::<&QuestNpcId>(entity) {
        if existing.0 != id {
            return Err("Quest giver identity changed before engagement".into());
        }
    } else {
        ecs.insert_one(entity, QuestNpcId(id))
            .map_err(|e| e.to_string())?;
    }
    // Once engaged, the original giver is no longer disposable scatter wildlife.
    let _ = ecs.remove_one::<crate::entity::Scattered>(entity);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn d2c3_giver_carrier_retains_original_identity_once_and_excludes_authored_duplicate() {
        let scope = [3; 16];
        let mut ecs = hecs::World::new();
        let npc = crate::entity::spawn_mob(
            &mut ecs,
            MobType::Villager,
            glam::Vec3::new(2.0, 80.0, -7.0),
        );
        ecs.insert(
            npc,
            (
                VillagerComponent {
                    profession: crate::villager::Profession::Farmer,
                    claimed_workstation: Some([1, 80, -8]),
                    gossip_line: Some("A fine harvest.".into()),
                    gossip_last_refresh_tick: 79,
                    commission: None,
                },
                crate::entity::Authored,
            ),
        )
        .unwrap();
        let id = prepare_engagement(scope, &ecs, npc).unwrap();
        assert!(
            ecs.get::<&QuestNpcId>(npc).is_err(),
            "preparation mutated the giver"
        );
        commit_engagement(&mut ecs, npc, id).unwrap();
        assert!(ecs.get::<&crate::entity::Scattered>(npc).is_err());
        assert_eq!(prepare_engagement(scope, &ecs, npc).unwrap(), id);
        assert!(
            crate::save::tamed_mobs_to_saved(&ecs).unwrap().is_empty(),
            "old Authored carrier duplicated the engaged NPC"
        );
        let saved = capture(Some(scope), &ecs).unwrap().unwrap();
        let bytes = bincode::serialize(&saved).unwrap();
        let saved: QuestWorldSave = bincode::deserialize(&bytes).unwrap();
        let mut resumed = hecs::World::new();
        restore(&mut resumed, &saved).unwrap();
        restore(&mut resumed, &saved).unwrap();
        assert_eq!(resumed.query::<&QuestNpcId>().iter().count(), 1);
        let identity = resumed.query::<&QuestNpcId>().iter().next().unwrap().1.0;
        assert_eq!(identity, id);
        assert_eq!(
            bincode::serialize(&capture(Some(scope), &resumed).unwrap().unwrap()).unwrap(),
            bytes
        );
        assert!(check_capture(None, Some(&saved)).is_err());
        assert!(check_capture(Some([4; 16]), Some(&saved)).is_err());
        assert!(check_capture(Some(scope), None).is_err());
    }
}
