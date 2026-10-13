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
            schema: 1,
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
        proof: Proof { core, manifest },
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
    assert_eq!(saved.encode().unwrap()[0], 3);
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
fn handover_personal_missing_wrong_manifest_and_second_transfer_are_checked() {
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
    let mut record = saved.personal.as_ref().unwrap().record().unwrap();
    record.context.transfer = [8; 16];
    record.context.generation += 1;
    preflight(&seed, &record.encode().unwrap(), record.context).unwrap();
    assert!(
        PersonalRecord::decode(&record.encode().unwrap(), &f.proof.binding(f.key).context).is_err()
    );
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
