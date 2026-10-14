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
    assert_eq!(saved.encode().unwrap()[0], 4);
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
