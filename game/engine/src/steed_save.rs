//! Shared saved-pet restoration. Ownership remains unchanged; eligibility
//! refuses positional carriers before transfer. Packs extends this exact API.

pub(crate) fn restore_pets(
    ecs: &mut hecs::World,
    pets: &[crate::save::SavedTamedPet],
    _local_bodies: bool,
) -> Result<(), String> {
    for pet in pets {
        let kind = pet.data.mob_type();
        let pos = glam::Vec3::new(pet.x, pet.y, pet.z);
        let id = crate::entity::spawn_mob(ecs, kind, pos);
        match &pet.data {
            crate::save::SavedTamedPetData::Wolf(d) => {
                // Belt-and-braces (Task 7): a save written before the
                // save-time fix can still have a persisted
                // AttackHostile/AttackRecentAttacker baked into its
                // bytes, carrying a stale hecs entity id — sanitize
                // on the way back in too.
                let mut d = d.clone();
                d.sanitize_attack_state_for_persistence();
                let _ = ecs.insert_one(id, d);
            }
            crate::save::SavedTamedPetData::Nostrich(d) => {
                let _ = ecs.insert_one(id, d.clone());
            }
            crate::save::SavedTamedPetData::Companion { data, .. } => {
                // Legacy save (pre-1C): no command state on the wire —
                // CompanionData restores with state = Follow (Default).
                let _ = ecs.insert_one(id, data.clone());
            }
            crate::save::SavedTamedPetData::Companion2 { data, state, .. } => {
                // 1C save: `state` is serde(skip) on CompanionData, so
                // re-attach the command state carried alongside it.
                let mut d = data.clone();
                d.state = *state;
                let _ = ecs.insert_one(id, d);
            }
            crate::save::SavedTamedPetData::Steed { data, .. } => {
                // Legacy save (pre-Task 11): no cargo pack on the wire —
                // spawn_mob attached a fresh wild HorseData; overwrite
                // with the kept state (owner slot + AI state), packless.
                let _ = ecs.insert_one(id, data.clone());
            }
            crate::save::SavedTamedPetData::Steed2 { data, pack, .. } => {
                // Task 11 save: `pack` is serde(skip) on HorseData, so
                // re-attach the cargo pack carried alongside it.
                let mut d = data.clone();
                d.pack = pack.clone();
                let _ = ecs.insert_one(id, d);
            }
            crate::save::SavedTamedPetData::Authored { .. } => {
                // #129 — re-tag as authored so it survives chunk unload
                // (no `Scattered`) and is re-collected on the next save.
                let _ = ecs.insert_one(id, crate::entity::Authored);
            }
        }
    }
    Ok(())
}
