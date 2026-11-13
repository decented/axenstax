use super::*;
use crate::handover::{
    codec::frame,
    manifest::{Agreement, Member},
    personal::{PersonalRecord, RecordContext},
};
use crate::item::ItemStack;

struct Fixture {
    _root: crate::save::WorldsRootGuard,
    world: std::path::PathBuf,
    proof: Proof,
    key: [u8; 32],
    seed: Vec<u8>,
}
fn fixture(tag: &str) -> Fixture {
    fixture_with(tag, None)
}
/// [`fixture`], with `offhand` in the frozen seat's off-hand.
fn fixture_with(tag: &str, offhand: Option<ItemStack>) -> Fixture {
    let root = crate::save::WorldsRootGuard::new(tag);
    let world = crate::save::world_dir("personal");
    fs::create_dir(&world).unwrap();
    let key = nostr::Keys::new(nostr::SecretKey::from_slice(&[22; 32]).unwrap())
        .public_key()
        .to_bytes();
    let source = nostr::Keys::new(nostr::SecretKey::from_slice(&[21; 32]).unwrap())
        .public_key()
        .to_bytes();
    let context = RecordContext {
        participant: key,
        lineage: [1; 16],
        generation: 9,
        transfer: [4; 16],
    };
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(4.5, 70.0, 9.5), 1.0);
    slot.inventory
        .set_slot(4, Some(ItemStack::new_block(crate::block::GLASS, 9)));
    slot.crafting_ui.cursor_item = Some(ItemStack::new_block(crate::block::DIRT, 3));
    slot.crafting_ui.grid[2][2] =
        Some(ItemStack::new_material(crate::item::MaterialId::Diamond, 2));
    slot.combat.health = 7.0;
    slot.combat.hunger = 9;
    slot.kill_counter.insert(crate::mob::MobType::Pig, 17);
    slot.reputation.per_village.insert((3, 4), -30);
    slot.bounties_claimed.insert(7, 2);
    slot.offhand = offhand;
    let seed = super::super::capture::local0_sidecar(&slot, [9; 16]).unwrap();
    let progress = super::super::capture::local0_progress(&slot, context).unwrap();
    fs::create_dir(world.join("joiners")).unwrap();
    fs::create_dir(world.join("participants")).unwrap();
    fs::write(path(&world, key), &seed).unwrap();
    let npub = crate::hosted_server::pubkey_to_npub(&key);
    let progress_path = format!("participants/{npub}.dat");
    fs::write(world.join(&progress_path), &progress).unwrap();
    let mut save = crate::save::minimal_world_save_for_tests(42);
    super::super::eligibility::neutralise(
        &mut save,
        &crate::hosted_server::pubkey_to_npub(&source),
    )
    .unwrap();
    let world_bytes = crate::save_format::encode_world_save(&save).unwrap();
    fs::write(world.join("world.dat"), &world_bytes).unwrap();
    let mut meta = crate::save::WorldMeta::new("personal");
    meta.owner_pubkey = Some(hex::encode(key));
    let meta_bytes = serde_json::to_vec(&meta).unwrap();
    fs::write(world.join("world_meta.json"), &meta_bytes).unwrap();
    let mut members = vec![
        Member {
            path: format!("joiners/{npub}.dat"),
            kind: MemberKind::Sidecar,
            length: seed.len() as u64,
            digest: crate::handover::hash(&seed),
            schema: 1,
        },
        Member {
            path: progress_path,
            kind: MemberKind::Progression,
            length: progress.len() as u64,
            digest: crate::handover::hash(&progress),
            schema: crate::handover::personal::SCHEMA,
        },
        Member {
            path: "world.dat".into(),
            kind: MemberKind::WorldData,
            length: world_bytes.len() as u64,
            digest: crate::handover::hash(&world_bytes),
            schema: crate::save_format::SAVE_FORMAT_VERSION,
        },
        Member {
            path: "world_meta.json".into(),
            kind: MemberKind::WorldMeta,
            length: meta_bytes.len() as u64,
            digest: crate::handover::hash(&meta_bytes),
            schema: 1,
        },
    ];
    members.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = Manifest {
        source,
        receiver: key,
        world: [2; 16],
        lineage: context.lineage,
        transfer: context.transfer,
        generation: context.generation,
        stream_length: 100,
        stream_hash: [3; 32],
        agreements: vec![Agreement {
            participant: key,
            proof: [7; 32],
        }],
        members,
    };
    let core = Core {
        context: [9; 32],
        root_runtime: source,
        root_persona: source,
        root_expiry: 1000,
        lineage: context.lineage,
        snapshot_world: manifest.world,
        manifest: manifest.digest().unwrap(),
        generation: context.generation,
        epoch: 1,
        predecessor: [8; 32],
        transfer: context.transfer,
        source_runtime: source,
        source_persona: source,
        successor_runtime: key,
        successor_persona: key,
        issued_at: 100,
        not_before: 100,
        expires_at: 900,
        remaining_hops: 3,
    };
    let store = MarkerStore::new(world.join("handover")).unwrap();
    let mut activated = Vec::new();
    frame(&mut activated, b"AxeNStax/handover/activated/v2");
    frame(&mut activated, &core.encode());
    activated.extend_from_slice(&[1; 32]);
    activated.extend_from_slice(&[2; 32]);
    activated.extend_from_slice(&[3; 16]);
    store
        .persist(MarkerKind::Activated, core.transfer, &activated)
        .unwrap();
    fs::write(
        world.join("handover/personal-manifest.dat"),
        manifest.encode().unwrap(),
    )
    .unwrap();
    Fixture {
        _root: root,
        world,
        proof: Proof { core, manifest, retained: false, retained_copy: None },
        key,
        seed,
    }
}

#[test]
fn handover_personal_bootstrap_selects_one_identity_and_never_replays_current_v3() {
    let f = fixture("personal-once");
    assert!(
        load_or_bootstrap(&f.world, f.proof.core.source_persona, &f.proof)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fs::read(path(&f.world, f.key)).unwrap(),
        f.seed,
        "unknown identity altered another player's seed"
    );
    let selected = proof(&f.world).unwrap().unwrap();
    let mut saved = load_or_bootstrap(&f.world, f.key, &selected)
        .unwrap()
        .unwrap();
    assert_eq!(saved.encode().unwrap()[0], 6);
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    install(&saved, &mut slot).unwrap();
    assert_eq!(slot.inventory.slot(4).unwrap().count, 9);
    assert_eq!(slot.crafting_ui.cursor_item.as_ref().unwrap().count, 3);
    assert_eq!(slot.crafting_ui.grid[2][2].as_ref().unwrap().count, 2);
    assert_eq!((slot.combat.health, slot.combat.hunger), (7.0, 9));
    assert_eq!(slot.kill_counter[&crate::mob::MobType::Pig], 17);
    assert_eq!(slot.reputation.per_village[&(3, 4)], -30);
    assert_eq!(slot.bounties_claimed[&7], 2);
    slot.inventory.set_slot(4, None);
    slot.kill_counter.insert(crate::mob::MobType::Pig, 23);
    saved =
        JoinerSave::decode(&crate::joiner_save::capture_local(&slot).unwrap().unwrap().0).unwrap();
    write(&f.world, f.key, &saved).unwrap();
    let current = load_or_bootstrap(&f.world, f.key, &selected)
        .unwrap()
        .unwrap();
    assert!(
        current.window().unwrap().inv.slot(4).is_none(),
        "empty mutable bag replayed a frozen bag"
    );
    assert_eq!(
        current.personal.unwrap().record().unwrap().kills,
        vec![(crate::mob::MobType::Pig, 23)]
    );
    fs::write(path(&f.world, f.key), b"\x03broken").unwrap();
    assert!(load_or_bootstrap(&f.world, f.key, &selected).is_err());
    assert_eq!(
        fs::read(path(&f.world, f.key)).unwrap(),
        b"\x03broken",
        "corruption was replaced by the seed"
    );
}

#[test]
fn handover_personal_missing_wrong_manifest_and_next_context_preflight_are_checked() {
    let f = fixture("personal-proof");
    let saved = load_or_bootstrap(&f.world, f.key, &f.proof)
        .unwrap()
        .unwrap();
    let seed = saved.encode_seed().unwrap();
    assert_eq!(seed[0], 2);
    let parsed = JoinerSave::decode(&seed).unwrap();
    assert!(
        parsed.personal.is_none(),
        "previous import binding leaked into next seed"
    );
    let record = saved.personal.as_ref().unwrap().record().unwrap();
    let crate::progression::GameplayContext::Imported(mut next_context) = record.context else {
        panic!("authenticated imported record changed its binding class");
    };
    next_context.transfer = [8; 16];
    next_context.generation += 1;
    let progress = PersonalRecord::from_gameplay(next_context, &record).unwrap().encode().unwrap();
    preflight(&seed, &progress, next_context).unwrap();
    assert!(PersonalRecord::decode(&progress, &f.proof.binding(f.key).context).is_err());
    fs::remove_file(f.world.join("handover/personal-manifest.dat")).unwrap();
    assert!(
        proof(&f.world).is_err(),
        "missing retained proof admitted a current personal record"
    );
    fs::write(
        f.world.join("handover/personal-manifest.dat"),
        f.proof.manifest.encode().unwrap(),
    )
    .unwrap();
    let mut changed = f.proof.manifest.clone();
    changed.generation += 1;
    fs::write(
        f.world.join("handover/personal-manifest.dat"),
        changed.encode().unwrap(),
    )
    .unwrap();
    assert!(
        proof(&f.world).is_err(),
        "foreign manifest was accepted under Activated"
    );
}

#[test]
fn handover_personal_atomic_failure_preserves_seed_or_complete_v3_and_retries_durability() {
    struct Fail(crate::handover::durable::WriteBoundary);
    impl crate::handover::durable::BoundaryObserver for Fail {
        fn reached(&mut self, at: crate::handover::durable::WriteBoundary) -> std::io::Result<()> {
            if at == self.0 {
                Err(std::io::Error::other("personal injected failure"))
            } else {
                Ok(())
            }
        }
    }
    use crate::handover::durable::WriteBoundary;
    let f = fixture("personal-atomic");
    let mut saved = JoinerSave::decode(&f.seed).unwrap();
    let bytes = fs::read(f.world.join(format!(
        "participants/{}.dat",
        crate::hosted_server::pubkey_to_npub(&f.key)
    )))
    .unwrap();
    let record = PersonalRecord::decode(&bytes, &f.proof.binding(f.key).context).unwrap();
    saved.personal = Some(BoundPersonal::new(f.proof.binding(f.key), &record).unwrap());
    assert!(
        write_observed(
            &f.world,
            f.key,
            &saved,
            &mut Fail(WriteBoundary::TempSynced)
        )
        .is_err()
    );
    assert_eq!(
        fs::read(path(&f.world, f.key)).unwrap(),
        f.seed,
        "failed pre-rename bootstrap consumed the seed"
    );
    assert!(write_observed(&f.world, f.key, &saved, &mut Fail(WriteBoundary::Renamed)).is_err());
    assert_eq!(
        JoinerSave::decode(&fs::read(path(&f.world, f.key)).unwrap())
            .unwrap()
            .encode()
            .unwrap(),
        saved.encode().unwrap()
    );
    let (reloaded, syncs) =
        crate::fsync::tests::count_syncs(|| load_or_bootstrap(&f.world, f.key, &f.proof));
    assert!(reloaded.unwrap().is_some());
    assert!(
        syncs >= 3,
        "visible v3 admitted gameplay without confirming rename durability"
    );
}

#[test]
fn handover_personal_copy_refusal_and_proofless_remote_restore_preserve_bound_records() {
    let f = fixture("personal-copy-proof");
    assert!(generic_copy_guard(&f.world).is_err());
    assert!(
        crate::native_world_io::export_world_native("personal").is_err(),
        "archive/folder/profile exporter dropped activation proof"
    );
    assert!(
        crate::save::fork_world("personal", "personal").is_err(),
        "Fork copied bound records while dropping proof"
    );
    assert!(!crate::save::world_dir("personal_fork").exists());
    let current = load_or_bootstrap(&f.world, f.key, &f.proof)
        .unwrap()
        .unwrap();
    let original = current.encode().unwrap();
    fs::remove_dir_all(f.world.join("handover")).unwrap();
    assert!(proof(&f.world).unwrap().is_none());
    assert!(
        has_bound_records(&f.world).unwrap(),
        "dedicated role's absent local field hid the canonical binding"
    );
    assert!(generic_copy_guard(&f.world).is_err());
    let mut server = crate::server::GameServer::new(1, "personal".into(), 42);
    server.players[0].verified_pubkey = Some(f.key);
    server.players[0].server_simulated = true;
    assert!(server.load_joiner(0).is_none());
    assert!(
        server.players[0].personal_read_failed,
        "missing proof donated an ordinary sidecar window"
    );
    assert!(server.save_joiner(0).is_err());
    assert_eq!(
        fs::read(path(&f.world, f.key)).unwrap(),
        original,
        "proofless admission/save overwrote the bound bag/maps"
    );
    fs::write(path(&f.world, f.key), b"\x03broken").unwrap();
    let mut server = crate::server::GameServer::new(1, "personal".into(), 42);
    server.players[0].verified_pubkey = Some(f.key);
    assert!(server.load_joiner(0).is_none());
    assert!(
        server.players[0].personal_read_failed,
        "corrupt current v3 became a fresh kit without proof"
    );
    assert_eq!(fs::read(path(&f.world, f.key)).unwrap(), b"\x03broken");
}

#[test]
fn handover_personal_generic_copy_keeps_ordinary_unbound_worlds_available() {
    let f = fixture("personal-copy-ordinary");
    fs::remove_dir_all(f.world.join("handover")).unwrap();
    fs::remove_dir_all(f.world.join("participants")).unwrap();
    let plain = JoinerSave::decode(&f.seed).unwrap().encode().unwrap();
    fs::write(path(&f.world, f.key), &plain).unwrap();
    assert!(!has_bound_records(&f.world).unwrap());
    generic_copy_guard(&f.world).unwrap();
    let archive = crate::native_world_io::export_world_native("personal").unwrap();
    crate::world_archive::unpack_world_for_import(&archive, &mut crate::world::World::new())
        .unwrap();
    assert!(crate::save::fork_world("personal", "personal").is_ok());
    assert_eq!(fs::read(path(&f.world, f.key)).unwrap(), plain);
}

#[path = "personal_role_tests.rs"]
mod role_tests;

#[test]
fn handover_personal_current_world_snapshot_corruption_never_defaults_to_a_seed() {
    let f = fixture("personal-world-tail");
    let saved = load_or_bootstrap(&f.world, f.key, &f.proof)
        .unwrap()
        .unwrap();
    let mut world =
        crate::save::read_world_save(&fs::read(f.world.join("world.dat")).unwrap()).unwrap();
    world.local_personal = Some(crate::personal_value::LocalPersonalSave(
        saved.encode().unwrap(),
    ));
    let bytes = crate::save_format::encode_world_save(&world).unwrap();
    let (payload, version) = crate::save_format::split_save_footer(&bytes);
    assert!(version.unwrap() >= 53);
    let mut torn = payload[..payload.len() - 1].to_vec();
    torn.extend_from_slice(&bytes[payload.len()..]);
    fs::write(f.world.join("world.dat"), &torn).unwrap();
    assert!(
        crate::save::read_world_save(&torn).is_err(),
        "current corrupt personal snapshot silently became an absent seed baseline"
    );
    assert_eq!(fs::read(f.world.join("world.dat")).unwrap(), torn);
    // Pin a genuinely readable ancient eight-field record. Relabelling it
    // as current must not turn a corrupt current prefix into a legacy reset.
    let mut ancient = bincode::serialize(&(
        42u32,
        0.0f32,
        70.0f32,
        0.0f32,
        20.0f32,
        0usize,
        vec![(crate::block::STONE, 9u8)],
        Vec::<u8>::new(),
    ))
    .unwrap();
    assert!(
        crate::save::read_world_save(&ancient).is_ok(),
        "known ancient world compatibility changed"
    );
    ancient.extend_from_slice(&bytes[payload.len()..]);
    assert!(
        crate::save::read_world_save(&ancient).is_err(),
        "current corrupt prefix took the ancient legacy reset path"
    );
}

/// A checkpoint from another world, which the bytes' own checks all accept, so a
/// refusal has to come from what is done with the folder.
fn restore_on_the_fixture(f: &Fixture) -> Result<crate::server::GameServer, crate::authority_checkpoint::RestoreError> {
    let mut donor = crate::server::GameServer::new(0, "donor".into(), 42);
    donor.world.generator = crate::biome::Generator::for_new_world();
    let bytes = donor.checkpoint().unwrap();
    crate::server::GameServer::restore_from_checkpoint(&donor.authority_config(), &bytes, &f.world)
}

fn folder_bytes(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        if entry.metadata().unwrap().is_dir() {
            out.extend(folder_bytes(&entry.path()));
        } else {
            out.insert(entry.path(), fs::read(entry.path()).unwrap());
        }
    }
    out
}

/// A role cut left to `initial_load` would write the owner's sidecar, save the world
/// and park the autosave. A folder that shows one pending, with every digest matching,
/// is refused before the load.
#[test]
fn a_restore_refuses_a_folder_the_role_cut_has_not_reached() {
    use crate::authority_checkpoint::RestoreError;
    let f = fixture("personal-restore-uncut");
    let (h, _, bytes) = authority_of_the_fixture(&f);
    // A local recovery snapshot, which a cut parks. No record the checkpoint binds.
    fs::create_dir(f.world.join("autosave")).unwrap();
    fs::copy(f.world.join("world.dat"), f.world.join("autosave/world.dat")).unwrap();
    let before = folder_bytes(&f.world);
    match restore_of(&h, &bytes, &f).err() {
        Some(RestoreError::Fault(why)) => assert!(why.contains("role cut"), "{why}"),
        other => panic!("an autosave: expected Fault, got {other:?}"),
    }
    assert_eq!(folder_bytes(&f.world), before, "the refused restore changed the folder");

    // A selected local record in the primary, which a cut clears.
    fs::remove_dir_all(f.world.join("autosave")).unwrap();
    let mut save = crate::save::read_world_save(&fs::read(f.world.join("world.dat")).unwrap()).unwrap();
    let record = load_or_bootstrap_unwritten(&f.world, f.key, &f.proof).unwrap().unwrap().encode().unwrap();
    save.local_personal = Some(crate::personal_value::LocalPersonalSave(record));
    fs::write(f.world.join("world.dat"), crate::save_format::encode_world_save(&save).unwrap()).unwrap();
    h.server.stored_records.note_world_dat("personal");
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("the authority checkpoints");
    let before = folder_bytes(&f.world);
    match restore_of(&h, &bytes, &f).err() {
        Some(RestoreError::Fault(why)) => assert!(why.contains("role cut"), "{why}"),
        other => panic!("a selected record: expected Fault, got {other:?}"),
    }
    assert_eq!(folder_bytes(&f.world), before, "the refused restore changed the folder");
}

/// A folder the cut has reached goes on to the manifest, and the load's call into
/// the cut only confirms what is there.
#[test]
fn a_restore_of_a_cut_folder_writes_nothing_and_reaches_the_manifest() {
    use crate::authority_checkpoint::RestoreError;
    let f = fixture("personal-restore-cut");
    let before = folder_bytes(&f.world);
    match restore_on_the_fixture(&f).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("manifest"), "{why}"),
        other => panic!("expected the manifest to refuse it, got {other:?}"),
    }
    assert_eq!(folder_bytes(&f.world), before, "the refused restore changed the folder");
}

/// A server that has run the fixture's handed-over world as its authority: the
/// participant holds a body, its sidecar still the frozen seed, as a seat that hasn't
/// come back since the import leaves it.
fn authority_of_the_fixture(f: &Fixture) -> (crate::test_harness::TestHost, usize, Vec<u8>) {
    let mut server = crate::server::GameServer::new(0, "personal".into(), 42);
    server.render_distance = 0;
    server.initial_load().unwrap();
    let mut h = crate::test_harness::TestHost::adopt(server);
    let slot = crate::test_integration::twin_run::add_joiner(&mut h, glam::Vec3::new(0.5, 90.0, 0.5));
    h.server.players[slot].verified_pubkey = Some(f.key);
    h.server.players[slot].awaiting_reattach = true;
    h.server.persist_authority().expect("the authority persists");
    let bytes = h.server.checkpoint().expect("the authority checkpoints");
    (h, slot, bytes)
}

fn restore_of(h: &crate::test_harness::TestHost, bytes: &[u8], f: &Fixture) -> Result<crate::server::GameServer, crate::authority_checkpoint::RestoreError> {
    crate::server::GameServer::restore_from_checkpoint(&h.server.authority_config(), bytes, &f.world)
}

/// `load_or_bootstrap` turns a frozen seed into the current record and writes it back.
/// A restore takes the same record and leaves the file: the join after it writes it.
#[test]
fn a_restore_bootstraps_a_seed_sidecar_without_writing_it() {
    let f = fixture("personal-restore-seed");
    let (h, slot, bytes) = authority_of_the_fixture(&f);
    let sidecar = path(&f.world, f.key);
    assert_eq!(fs::read(&sidecar).unwrap(), f.seed, "the fixture's sidecar is the frozen seed");
    let before = folder_bytes(&f.world);
    let mut back = restore_of(&h, &bytes, &f).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(folder_bytes(&f.world), before, "a restore wrote");
    assert!(back.players[slot].personal.is_some(), "the seat has its bound record, in memory");
    // A normal open keeps what it needs: the next ordinary read writes the record.
    back.load_joiner(slot).expect("the joiner reads");
    assert_ne!(fs::read(&sidecar).unwrap(), f.seed, "the join after it bootstraps the file");
}

/// Every `handover/` record and `participants/` file a restore reads is bound: a flipped
/// byte, absent, another key's valid bytes and an older value of its own key are each
/// `invalid-checkpoint` from a real restore, and an added record too.
#[test]
fn a_handed_over_worlds_records_are_bound() {
    use crate::authority_checkpoint::RestoreError;
    let f = fixture("personal-restore-bound");
    let npub = crate::hosted_server::pubkey_to_npub(&f.key);
    let activated = f.world.join(format!("handover/activated-{}.dat", hex::encode(f.proof.core.transfer)));
    let manifest = f.world.join("handover/personal-manifest.dat");
    let progress = f.world.join(format!("participants/{npub}.dat"));

    // Older values, from the same encoders with what an earlier state held.
    let older_marker = {
        let aside = f.world.join("older-marker");
        let store = MarkerStore::new(aside.clone()).unwrap();
        let mut payload = Vec::new();
        frame(&mut payload, b"AxeNStax/handover/activated/v2");
        frame(&mut payload, &f.proof.core.encode());
        payload.extend_from_slice(&[9; 80]);
        store.persist(MarkerKind::Activated, f.proof.core.transfer, &payload).unwrap();
        let bytes = fs::read(aside.join(format!("activated-{}.dat", hex::encode(f.proof.core.transfer)))).unwrap();
        fs::remove_dir_all(aside).unwrap();
        bytes
    };
    let older_manifest = {
        let mut earlier = f.proof.manifest.clone();
        earlier.stream_length -= 1;
        earlier.encode().unwrap()
    };
    let older_progress = {
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(4.5, 70.0, 9.5), 1.0);
        slot.kill_counter.insert(crate::mob::MobType::Pig, 3);
        let context = RecordContext { participant: f.key, lineage: [1; 16], generation: 9, transfer: [4; 16] };
        super::super::capture::local0_progress(&slot, context).unwrap()
    };

    let (h, _, bytes) = authority_of_the_fixture(&f);
    restore_of(&h, &bytes, &f).unwrap_or_else(|e| panic!("untouched, it restores: {e:?}"));
    let good = |file: &std::path::Path| fs::read(file).unwrap();
    let records = [
        ("activated", activated.clone(), good(&manifest), older_marker),
        ("manifest", manifest.clone(), good(&progress), older_manifest),
        ("participant", progress.clone(), good(&activated), older_progress),
    ];
    let mut wrong = Vec::new();
    for (name, file, swapped, older) in &records {
        let was = good(file);
        assert_ne!(&was, older, "{name}: the older value is the current one");
        let mut flipped = was.clone();
        flipped[2] ^= 1;
        for (case, there) in [("flipped", Some(&flipped)), ("absent", None), ("swapped", Some(swapped)), ("older", Some(older))] {
            match there {
                Some(b) => fs::write(file, b).unwrap(),
                None => fs::remove_file(file).unwrap(),
            }
            match restore_of(&h, &bytes, &f).err() {
                Some(RestoreError::InvalidCheckpoint(_)) => {}
                other => wrong.push(format!("{name}, {case}: {other:?}")),
            }
        }
        fs::write(file, &was).unwrap();
    }
    // A record no persist bound: `open_guard` scans the folder, so it changes what a
    // restore does, and it isn't a stale leftover it could ignore.
    let added = f.world.join(format!("handover/quarantined-{}.dat", hex::encode([6u8; 16])));
    fs::write(&added, b"added").unwrap();
    match restore_of(&h, &bytes, &f).err() {
        Some(RestoreError::InvalidCheckpoint(_)) => {}
        other => wrong.push(format!("added record: {other:?}")),
    }
    fs::remove_file(&added).unwrap();
    assert!(wrong.is_empty(), "handover records' mutations not refused: {wrong:#?}");
    restore_of(&h, &bytes, &f).unwrap_or_else(|e| panic!("all put back, it restores: {e:?}"));
}

/// A checkpoint can't bind what it can't read, and a restore would refuse what it left out.
#[test]
fn a_handover_record_that_cant_be_read_is_an_error_at_checkpoint() {
    use crate::authority_checkpoint::CheckpointError;
    let f = fixture("personal-unreadable");
    let (h, _, _) = authority_of_the_fixture(&f);
    let link = f.world.join("handover/ready-link.dat");
    std::os::unix::fs::symlink(f.world.join("world.dat"), &link).unwrap();
    h.server.stored_records.note_handover("personal");
    assert!(matches!(h.server.checkpoint(), Err(CheckpointError::Unrestorable(_))), "a symlink");
    assert!(h.server.write_bound_records().is_err(), "and no persist can bind it");
    fs::remove_file(&link).unwrap();
    fs::create_dir(&link).unwrap();
    h.server.stored_records.note_handover("personal");
    assert!(matches!(h.server.checkpoint(), Err(CheckpointError::Unrestorable(_))), "a folder");
    fs::remove_dir(&link).unwrap();
    h.server.stored_records.note_handover("personal");
    h.server.checkpoint().expect("put right, it checkpoints");
}

/// Off-hand review B2: a frozen seat's off-hand travels in its seed (not the
/// v2 layout, which has no place for it) and the recipient's bootstrap and
/// install put it back in the off-hand, once.
#[test]
fn a_seed_carries_the_off_hand_into_the_installed_seat() {
    let torch = ItemStack::new_block(crate::block::TORCH, 5);
    let f = fixture_with("personal-offhand", Some(torch.clone()));
    assert_ne!(f.seed[0], 2, "a seed holding an off-hand is not the v2 layout");
    assert_eq!(JoinerSave::decode(&f.seed).unwrap().window().unwrap().offhand, Some(torch.clone()));
    assert!(JoinerSave::decode(&f.seed).unwrap().personal.is_none(), "the seed carries no import binding");
    let saved = load_or_bootstrap(&f.world, f.key, &f.proof).unwrap().unwrap();
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    install(&saved, &mut slot).unwrap();
    assert_eq!(slot.offhand, Some(torch.clone()), "the off-hand arrives in the off-hand");
    assert!(
        slot.inventory.slots_iter().flatten().all(|s| s.item != torch.item),
        "and nowhere else"
    );
    // The bootstrapped current record (v6) keeps it too.
    let current = load_or_bootstrap(&f.world, f.key, &f.proof).unwrap().unwrap();
    assert_eq!(current.window().unwrap().offhand, Some(torch));
}

/// The load order of a bound local seat (`chunk_stream::initial_load`): field
/// 63 into the off-hand, then the `local_personal` record's install over it.
fn reload_bound(save: &crate::save::WorldSave, scope: [u8; 16], key: [u8; 32]) -> crate::player_slot::PlayerSlot {
    let bytes = crate::save_format::encode_world_save(save).unwrap();
    let back = crate::save::read_world_save(&bytes).unwrap();
    let mut loaded = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    loaded.offhand = crate::save::restore_offhand(back.player_offhands.as_ref(), 0);
    let snapshot = back.local_personal.as_ref().expect("a bound seat saves its record");
    let selected = crate::gameplay_attachment::selected_world_local(snapshot, Some(scope), Some(key), Some(0)).unwrap();
    install(&selected, &mut loaded).unwrap();
    loaded
}

fn bound_seat(scope: [u8; 16], key: [u8; 32]) -> crate::player_slot::PlayerSlot {
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(4.5, 70.0, 9.5), 1.0);
    // As `GameState::prepare_local_gameplay` binds a hosting seat.
    slot.personal_binding = Some(crate::progression::GameplayBinding::WorldLocal {
        scope,
        player: crate::progression::WorldPlayer::Npub(key),
    });
    slot
}

/// Off-hand review B1: a seat with a personal binding (any world that has
/// been hosted) saves its off-hand in its `local_personal` record, so the
/// install that runs over field 63 on reload keeps it.
#[test]
fn a_bound_local_seats_off_hand_survives_a_save_and_reload() {
    let _root = crate::save::WorldsRootGuard::new("bound-offhand");
    let (scope, key) = ([19; 16], [17; 32]);
    let torch = ItemStack::new_block(crate::block::TORCH, 5);
    let mut slot = bound_seat(scope, key);
    slot.offhand = Some(torch.clone());
    slot.inventory.set_slot(2, Some(ItemStack::new_block(crate::block::STONE, 3)));
    let world = crate::world::World::new();
    let save = crate::save::build_world_save(&world, std::slice::from_ref(&slot), 42, &[], &[], None, None).unwrap();
    let saved = JoinerSave::decode(&save.local_personal.as_ref().unwrap().0).unwrap();
    assert_eq!(saved.window().unwrap().offhand, Some(torch.clone()), "the record holds the off-hand");
    let loaded = reload_bound(&save, scope, key);
    assert_eq!(loaded.offhand, Some(torch.clone()), "the off-hand is back in the off-hand");
    let torches: u32 = loaded.inventory.slots_iter().flatten().chain(loaded.offhand.iter())
        .chain(loaded.crafting_ui.cursor_item.iter()).chain(loaded.crafting_ui.grid.iter().flatten().flatten())
        .filter(|s| s.item == torch.item).map(|s| u32::from(s.count)).sum();
    assert_eq!(torches, 5, "and nowhere else twice");
    assert_eq!(loaded.inventory.slot(2).map(|s| s.count), Some(3));
    // And the next save writes it again.
    let again = crate::save::build_world_save(&world, std::slice::from_ref(&loaded), 42, &[], &[], None, None).unwrap();
    assert_eq!(reload_bound(&again, scope, key).offhand, Some(torch));
}

/// Off-hand review M3: a developed Plan in a bound seat's off-hand is saved as
/// a marker, so its body must be remembered first, as the inventory's are.
#[test]
fn a_developed_plan_in_a_bound_seats_off_hand_survives_a_save_and_reload_with_its_body() {
    let _root = crate::save::WorldsRootGuard::new("bound-offhand-plan");
    let (scope, key) = ([21; 16], [23; 32]);
    let mut body = crate::plan::PlanData::debug_3x3_stone();
    body.name = "Off-hand plan that is no built-in".into();
    body.develop_state = crate::plan::DevelopState::Developed;
    let encoded = bincode::serialize(&body).unwrap();
    let marker = crate::plan::marker(&body);
    assert!(crate::plan_store::resolve(marker).is_none(), "not a built-in, not remembered yet");
    let mut slot = bound_seat(scope, key);
    slot.offhand = Some(ItemStack { item: crate::item::Item::Plan(body), count: 1 });
    assert_eq!(super::super::capture::local0_plan_items(&slot).len(), 1, "the off-hand's Plan is one of the seat's private bodies");
    super::super::capture::remember_local0_plans(&slot).unwrap();
    let world = crate::world::World::new();
    let save = crate::save::build_world_save(&world, std::slice::from_ref(&slot), 42, &[], &[], None, None).unwrap();
    let loaded = reload_bound(&save, scope, key);
    let Some(ItemStack { item: crate::item::Item::Plan(back), count: 1 }) = &loaded.offhand else {
        panic!("the Plan left the off-hand: {:?}", loaded.offhand.as_ref().map(|s| &s.item))
    };
    assert!(back.marker.is_none() && !back.cells.is_empty(), "the body came back, not a marker");
    assert_eq!(bincode::serialize(back).unwrap(), encoded);
}
