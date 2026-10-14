//! Real persisted primary/recovery selection and the dedicated role boundary.
use super::*;

#[test]
fn handover_personal_dedicated_load_selects_matching_snapshot_and_parks_future_autosave() {
    for recovery in [false, true] {
        let f = fixture(if recovery {
            "personal-role-auto"
        } else {
            "personal-role-primary"
        });
        let current = load_or_bootstrap(&f.world, f.key, &f.proof)
            .unwrap()
            .unwrap();
        let mut primary =
            crate::save::read_world_save(&fs::read(f.world.join("world.dat")).unwrap()).unwrap();
        primary.local_personal = Some(crate::personal_value::LocalPersonalSave(
            current.encode().unwrap(),
        ));
        let chest_at = (3, 1, 3);
        let mut chest = crate::chest::ChestData::default();
        chest.slots[0] = Some(ItemStack::new_block(crate::block::DIRT, 2));
        primary.chests.push(crate::save::SavedChest {
            x: 3,
            y: 1,
            z: 3,
            data: chest.clone(),
        });
        let source_npub = crate::hosted_server::pubkey_to_npub(&f.proof.core.source_persona);
        let mut wolf = crate::wolf::WolfData::untamed();
        wolf.ownership.owner_pubkey = source_npub.clone();
        primary.saved_mobs.push(crate::save::SavedTamedPet {
            x: 4.5,
            y: 2.0,
            z: 4.5,
            data: crate::save::SavedTamedPetData::Wolf(wolf),
        });
        let mut terrain = crate::world::World::new();
        terrain.set_block(3, 1, 3, crate::block::CHEST);
        let meta = crate::save::load_world_meta("personal");
        crate::save::write_world_folder("personal", &meta, &primary, &terrain).unwrap();
        let (bag_count, kills, chest_count) = if recovery {
            let selected = f.world.join("autosave");
            fs::create_dir(&selected).unwrap();
            fs::create_dir(selected.join("chunks")).unwrap();
            for entry in fs::read_dir(f.world.join("chunks")).unwrap() {
                let entry = entry.unwrap();
                fs::copy(
                    entry.path(),
                    selected.join("chunks").join(entry.file_name()),
                )
                .unwrap();
            }
            let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
            install(&current, &mut slot).unwrap();
            slot.inventory
                .set_slot(4, Some(ItemStack::new_block(crate::block::GLASS, 13)));
            slot.kill_counter.insert(crate::mob::MobType::Pig, 31);
            primary.local_personal = crate::joiner_save::capture_local(&slot).unwrap();
            primary.chests[0].data.slots[0] = Some(ItemStack::new_block(crate::block::DIRT, 3));
            write_world_snapshot(&f.world, &selected, &primary).unwrap();
            let file = fs::OpenOptions::new()
                .write(true)
                .open(selected.join("world.dat"))
                .unwrap();
            file.set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::now() + std::time::Duration::from_secs(86400),
            ))
            .unwrap();
            (13, 31, 3)
        } else {
            (9, 17, 2)
        };
        let mut server = crate::server::GameServer::new(0, "personal".into(), 42);
        server.render_distance = 0;
        server.initial_load().unwrap();
        assert!(
            server.selected_local_personal.is_none(),
            "dedicated startup exposed simulation before the local role cut"
        );
        assert!(
            !crate::save::has_autosave("personal"),
            "future-dated autosave can regain the old role"
        );
        let saved =
            crate::save::read_world_save(&fs::read(f.world.join("world.dat")).unwrap()).unwrap();
        assert!(saved.local_personal.is_none());
        assert_eq!(
            saved.chests[0].data.slots[0].as_ref().unwrap().count,
            chest_count,
            "role cut replaced the selected/current world with the older primary"
        );
        assert!(saved.saved_mobs.iter().any(|pet| matches!(&pet.data, crate::save::SavedTamedPetData::Wolf(wolf) if wolf.ownership.owner_pubkey == source_npub)), "role cut lost or donated the transferred source's pet");
        let selected = load_or_bootstrap(&f.world, f.key, &f.proof)
            .unwrap()
            .unwrap();
        assert_eq!(
            selected.window().unwrap().inv.slot(4).unwrap().count,
            bag_count,
            "remote role selected the wrong local bag"
        );
        assert_eq!(
            selected.personal.as_ref().unwrap().record().unwrap().kills,
            vec![(crate::mob::MobType::Pig, kills)]
        );
        if recovery {
            assert!(
                fs::read_dir(&f.world).unwrap().any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("autosave-kept-remote-")),
                "superseded autosave bytes were deleted"
            );
        }
        // A real remote holder changes and saves its own bag/maps. A later
        // native load selects that mutable authority, never the old local blob.
        server
            .players
            .push(crate::server::ServerPlayer::new(glam::Vec3::ZERO));
        server.players[0].server_simulated = true;
        server.players[0].verified_pubkey = Some(f.key);
        server.players[0].enter_world();
        let loaded = server.load_joiner(0).unwrap();
        let window = loaded.window().unwrap();
        let mut personal = loaded.personal.clone().unwrap();
        let mut record = personal.record().unwrap();
        record.kills = vec![(crate::mob::MobType::Pig, 41)];
        personal = BoundPersonal::new(personal.binding.clone(), &record).unwrap();
        loaded.restore_vitals(&mut server.players[0], server.run_id);
        server.players[0].inventory = window.inv;
        server.players[0].cursor = window.cursor;
        server.players[0].craft_grid = window.grid;
        server.players[0].armour = window.armour;
        server.players[0].personal = Some(personal);
        server.players[0]
            .inventory
            .set_slot(4, Some(ItemStack::new_block(crate::block::GLASS, 19)));
        server.save_joiner(0).unwrap();
        server.try_save().unwrap();
        let mut native_world = crate::world::World::new();
        let opened = crate::world_open::open_world(
            "personal",
            &mut native_world,
            crate::world_open::AutosavePolicy::Prefer,
        )
        .unwrap();
        let crate::world_open::OpenedWorld::Loaded { save, from, .. } = opened else {
            panic!("remote role lost the world")
        };
        assert!(
            !from.opened_from_autosave(),
            "old future-dated autosave regained authority"
        );
        assert!(save.local_personal.is_none());
        let current = load_or_bootstrap(&f.world, f.key, &f.proof)
            .unwrap()
            .unwrap();
        let mut native = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        install(&current, &mut native).unwrap();
        assert_eq!(native.inventory.slot(4).unwrap().count, 19);
        assert_eq!(native.kill_counter[&crate::mob::MobType::Pig], 41);
        assert_eq!(
            native_world.chest_at(chest_at).unwrap().slots[0]
                .as_ref()
                .unwrap()
                .count,
            chest_count
        );
    }
}
