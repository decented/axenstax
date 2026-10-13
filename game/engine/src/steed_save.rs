//! Steed3's marker-safe cargo and the shared saved-pet restoration path.
use crate::{chest::{ChestData, CHEST_SLOTS}, horse_ai::HorseData, save::{SavedTamedPet, SavedTamedPetData}, saved_item_slot::SavedItemSlot};

pub type SavedPack = [SavedItemSlot; CHEST_SLOTS];

pub(crate) fn capture_pack(pack: &ChestData) -> Result<SavedPack, String> {
    if pack.slots.len() != CHEST_SLOTS || pack.tier != crate::chest::ChestTier::Wood {
        return Err("A steed pack must contain exactly 27 wooden slots".into());
    }
    let mut slots = Vec::with_capacity(CHEST_SLOTS);
    for stack in &pack.slots {
        let saved = SavedItemSlot::capture_exact(stack.as_ref())?;
        if let Some(crate::item::ItemStack { item: crate::item::Item::Plan(body), .. }) = stack {
            // Save callers propagate this before committing the new marker
            // carrier. A failed private body store never destroys the body.
            crate::plan_store::remember_checked(body, 0)?;
        }
        slots.push(saved);
    }
    slots.try_into().map_err(|_| "A steed pack changed its slot count during capture".into())
}

fn restore_pack(saved: &SavedPack, local_bodies: bool) -> Result<ChestData, String> {
    let mut pack = ChestData::new();
    for (destination, source) in pack.slots.iter_mut().zip(saved) {
        let marker = source.restore()?;
        if !matches!(source, SavedItemSlot::Plan { .. }) && SavedItemSlot::capture(marker.as_ref()) != *source {
            return Err("A saved cargo slot could not be restored exactly".into());
        }
        *destination = if local_bodies { source.restore_local()? } else { marker };
    }
    Ok(pack)
}

fn restored_steed(data: &HorseData, keeper: &crate::horse_ai::SteedKeeper, pack: &Option<SavedPack>, local_bodies: bool) -> Result<HorseData, String> {
    if let crate::horse_ai::SteedKeeper::Npub(npub) = keeper
        && (!npub.starts_with("npub1") || npub.len() != 63 || !npub.is_ascii())
    {
        return Err("A saved steed keeper is not an npub".into());
    }
    let mut restored = data.clone();
    restored.keeper = Some(keeper.clone());
    restored.kept_by = match keeper { crate::horse_ai::SteedKeeper::LegacyLocal(slot) => Some(*slot), _ => None };
    restored.pack = pack.as_ref().map(|pack| restore_pack(pack, local_bodies)).transpose()?;
    Ok(restored)
}

/// Client/native reload, WASM resume and dedicated startup restore the
/// same persisted pets. Validate new cargo before spawning anything.
pub(crate) fn restore_pets(ecs: &mut hecs::World, pets: &[SavedTamedPet], local_bodies: bool) -> Result<(), String> {
    for pet in pets {
        if let SavedTamedPetData::Steed3 { data, keeper, pack, .. } = &pet.data {
            restored_steed(data, keeper, pack, false)?;
        }
    }
    for pet in pets {
        let id = crate::entity::spawn_mob(ecs, pet.data.mob_type(), glam::Vec3::new(pet.x, pet.y, pet.z));
        match &pet.data {
            SavedTamedPetData::Wolf(data) => {
                let mut data = data.clone();
                data.sanitize_attack_state_for_persistence();
                let _ = ecs.insert_one(id, data);
            },
            SavedTamedPetData::Nostrich(data) => { let _ = ecs.insert_one(id, data.clone()); },
            SavedTamedPetData::Companion { data, .. } => { let _ = ecs.insert_one(id, data.clone()); },
            SavedTamedPetData::Companion2 { data, state, .. } => {
                let mut data = data.clone();
                data.state = *state;
                let _ = ecs.insert_one(id, data);
            },
            SavedTamedPetData::Steed { data, .. } => { let _ = ecs.insert_one(id, data.clone()); },
            SavedTamedPetData::Steed2 { data, pack, .. } => {
                let mut data = data.clone();
                data.pack = pack.clone();
                let _ = ecs.insert_one(id, data);
            },
            SavedTamedPetData::Steed3 { data, keeper, pack, .. } => {
                let data = restored_steed(data, keeper, pack, local_bodies)?;
                let _ = ecs.insert_one(id, data);
            },
            SavedTamedPetData::Authored { .. } => { let _ = ecs.insert_one(id, crate::entity::Authored); },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d2c2_saved_keeper_and_pack_restore_for_server_and_local_reads() {
        let _guard = crate::save::WorldsRootGuard::new("d2c2-pet-restore");
        let mut ecs = hecs::World::new();
        let id = crate::entity::spawn_mob(&mut ecs, crate::mob::MobType::Donkey, glam::Vec3::new(2.0, 80.0, 3.0));
        let npub = crate::hosted_server::verified_npub(Some([7; 32])).unwrap();
        let mut data = ecs.get::<&mut HorseData>(id).unwrap();
        data.keeper = Some(crate::horse_ai::SteedKeeper::Npub(npub.clone()));
        let mut pack = ChestData::new();
        pack.slots[4] = Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 7));
        let plan = crate::plan::PlanData::debug_3x3_stone();
        let marker = crate::plan::marker(&plan);
        pack.slots[8] = Some(crate::item::ItemStack { item: crate::item::Item::Plan(plan), count: 1 });
        data.pack = Some(pack);
        drop(data);
        let saved = crate::save::tamed_mobs_to_saved(&ecs).unwrap();
        assert!(matches!(saved[0].data, SavedTamedPetData::Steed3 { .. }));
        for local in [false, true] {
            let mut restored = hecs::World::new();
            restore_pets(&mut restored, &saved, local).unwrap();
            let mut query = restored.query::<&HorseData>();
            let (_, data) = query.iter().next().unwrap();
            assert_eq!(data.keeper_identity(), Some(crate::horse_ai::SteedKeeper::Npub(npub.clone())));
            let pack = data.pack.as_ref().unwrap();
            assert_eq!(pack.slots[4].as_ref().unwrap().count, 7);
            assert!(pack.slots[0].is_none(), "slot positions survive");
            let crate::item::Item::Plan(plan) = &pack.slots[8].as_ref().unwrap().item else { panic!("Plan") };
            assert_eq!(crate::plan::marker(plan), marker);
            assert_eq!(!plan.cells.is_empty(), local, "server restores only the marker, local restores its private body");
        }
    }

    #[test]
    fn d2c2_plan_retention_failure_refuses_the_cargo_save_before_marker_capture() {
        let _guard = crate::save::WorldsRootGuard::new("d2c2-cargo-retain-failure");
        let mut body = crate::plan::PlanData::debug_3x3_stone();
        body.name = "x".repeat(crate::action_capacity::MAX_PRIVATE_BODY_BYTES + 1);
        let marker = crate::plan::marker(&body);
        let mut pack = ChestData::new();
        pack.slots[0] = Some(crate::item::ItemStack { item: crate::item::Item::Plan(body), count: 1 });
        assert!(capture_pack(&pack).is_err(), "a failed checked body retention cannot become a successful marker save");
        let crate::item::Item::Plan(held) = &pack.slots[0].as_ref().unwrap().item else { panic!("held Plan") };
        assert!(!held.cells.is_empty(), "the caller retains the actual private body");
        assert_eq!(crate::plan::marker(held), marker);
    }

    #[test]
    fn d2c2_dedicated_restart_restores_keeper_cargo_and_only_one_authored_steed() {
        let _guard = crate::save::WorldsRootGuard::new("d2c2-dedicated-steed");
        let name = "kept-cargo".to_string();
        let mut server = crate::server::GameServer::new(1, name.clone(), 31);
        let id = crate::entity::spawn_mob(&mut server.ecs, crate::mob::MobType::Mule, glam::Vec3::new(4.0, 80.0, 4.0));
        server.ecs.insert_one(id, crate::entity::Authored).unwrap();
        let npub = crate::hosted_server::verified_npub(Some([7; 32])).unwrap();
        let mut data = server.ecs.get::<&mut HorseData>(id).unwrap();
        data.keeper = Some(crate::horse_ai::SteedKeeper::Npub(npub.clone()));
        let mut pack = ChestData::new();
        pack.slots[3] = Some(crate::item::ItemStack::new_block(crate::block::STONE, 9));
        data.pack = Some(pack);
        drop(data);
        assert_eq!(crate::save::tamed_mobs_to_saved(&server.ecs).unwrap().len(), 1, "a kept authored steed is captured only once");
        server.try_save().unwrap();
        drop(server);
        let mut restored = crate::server::GameServer::new(1, name, 31);
        restored.initial_load().unwrap();
        let mut query = restored.ecs.query::<&HorseData>();
        let steeds: Vec<_> = query.iter().filter(|(_, data)| data.is_kept()).collect();
        assert_eq!(steeds.len(), 1, "dedicated restart restores the saved steed without an Authored duplicate");
        assert_eq!(steeds[0].1.keeper_identity(), Some(crate::horse_ai::SteedKeeper::Npub(npub)));
        assert_eq!(steeds[0].1.pack.as_ref().unwrap().slots[3].as_ref().unwrap().count, 9);
    }
}
