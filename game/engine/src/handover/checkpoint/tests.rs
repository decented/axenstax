//! Real file/codecs and fault boundaries; these are utility proofs, not consent.
use super::super::{
    certificate::Core,
    context::RootContext,
    manifest::{Agreement, MemberKind},
    phase::Authority,
};
use super::*;
use crate::{
    chunk::Chunk,
    save::{WorldMeta, WorldSave},
    world::World,
};
use nostr::{JsonUtil, Keys, SecretKey, ToBech32};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir();
        Self(files::owned_dir(&root, "handover-io-test").unwrap())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Fixture {
    source: Keys,
    recipient: Keys,
    root: RootContext,
    core: Core,
}
impl Fixture {
    fn new() -> Self {
        let source = Keys::new(SecretKey::from_slice(&[11; 32]).unwrap());
        let recipient = Keys::new(SecretKey::from_slice(&[22; 32]).unwrap());
        let root = RootContext::new(
            source.public_key().to_bytes(),
            &source.public_key().to_bech32().unwrap(),
            900,
            &["wss://relay.example".into()],
            100,
        )
        .unwrap();
        let core = Core {
            context: root.digest(),
            root_runtime: root.runtime(),
            root_persona: root.persona(),
            root_expiry: 900,
            lineage: [1; 16],
            snapshot_world: [2; 16],
            manifest: [3; 32],
            generation: 1,
            epoch: 1,
            predecessor: [0; 32],
            transfer: [4; 16],
            source_runtime: root.runtime(),
            source_persona: root.persona(),
            successor_runtime: recipient.public_key().to_bytes(),
            successor_persona: recipient.public_key().to_bytes(),
            issued_at: 100,
            not_before: 100,
            expires_at: 800,
            remaining_hops: 3,
        };
        Self {
            source,
            recipient,
            root,
            core,
        }
    }
    fn binding(&self, mut agreements: Vec<Agreement>) -> PackBinding {
        agreements.sort_by_key(|a| a.participant);
        PackBinding {
            source: self.core.source_persona,
            receiver: self.core.successor_persona,
            world: self.core.snapshot_world,
            lineage: self.core.lineage,
            transfer: self.core.transfer,
            generation: self.core.generation,
            agreements,
        }
    }
    fn admitted(&self, manifest: &super::super::manifest::Manifest) -> AdmittedManifest {
        let mut core = self.core.clone();
        core.manifest = manifest.digest().unwrap();
        let challenge = [55; 16];
        let offer = pollster::block_on(sign_offer(&self.source, &core, challenge))
            .unwrap()
            .as_json();
        AdmittedManifest::verify(
            &manifest.encode().unwrap(),
            offer.as_bytes(),
            OfferContext {
                core,
                root: &self.root,
                authority: &Authority::root(&self.root, [1; 16]).unwrap(),
                challenge,
                recipient_runtime: self.recipient.public_key().to_bytes(),
                recipient_persona: self.recipient.public_key().to_bytes(),
                now: 100,
            },
        )
        .unwrap()
    }
}
fn feed(import: &mut Import, archive: &Path) {
    let mut file = fs::File::open(archive).unwrap();
    let mut buf = [0; super::super::stream::CHUNK_LIMIT];
    let mut offset = 0;
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        import
            .accept(
                &super::super::stream::Chunk {
                    transfer: [4; 16],
                    offset,
                    bytes: buf[..n].to_vec(),
                },
                0,
            )
            .unwrap();
        offset += n as u64;
    }
}
fn simple_pack(f: &Fixture, parent: &Path) -> PackedCheckpoint {
    let meta = transfer_metadata(&WorldMeta::new("source")).unwrap();
    pack(
        CheckpointSource {
            meta: &meta,
            save: &neutral_save(42),
            world: &World::new(),
            disk_chunks: None,
            exhibit_dir: None,
            participants: &[],
        },
        f.binding(vec![]),
        parent,
    )
    .unwrap()
}
fn neutral_save(seed: u32) -> WorldSave {
    WorldSave {
        seed,
        player_x: 0.0,
        player_y: 0.0,
        player_z: 0.0,
        player_health: 20.0,
        hotbar_slot: 0,
        inventory: Vec::new(),
        players: Vec::new(),
        campfires: Vec::new(),
        furnaces: Vec::new(),
        vendors: Vec::new(),
        drying_racks: Vec::new(),
        hives: Vec::new(),
        chests: Vec::new(),
        tip_jars: Vec::new(),
        plots: Vec::new(),
        market_hubs: Vec::new(),
        auctions: Vec::new(),
        latent_prints: Vec::new(),
        construction_anchors: Vec::new(),
        architect_plaques: Vec::new(),
        village_anchors: Vec::new(),
        populated_villages: Vec::new(),
        village_bells: Vec::new(),
        village_treasuries: Vec::new(),
        active_raids: Vec::new(),
        raid_scheduler: crate::raid::RaidScheduler::new(),
        raid_kills: Vec::new(),
        brigand_hideouts: Vec::new(),
        bounties: Vec::new(),
        bounty_next_id: 0,
        bounty_last_refresh_tick: 0,
        face_overlays: Vec::new(),
        face_blueprints: Vec::new(),
        face_blueprint_blanks: Vec::new(),
        workshop: Default::default(),
        carts: Vec::new(),
        graves: Vec::new(),
        waypoints: Vec::new(),
        block_meta: Vec::new(),
        power_devices: Vec::new(),
        signs: Vec::new(),
        item_frames: Vec::new(),
        locked_slots: Vec::new(),
        hostile_acts: Vec::new(),
        rigs: Vec::new(),
        exhibits: Vec::new(),
        composters: Vec::new(),
        saved_mobs: Vec::new(),
        satoshi: Default::default(),
        dispensers: Vec::new(),
        rig_clips: Vec::new(),
        local_personal: None,
    }
}

#[test]
fn handover_checkpoint_real_world_roundtrip_preserves_members_and_source() {
    let t = Temp::new();
    let f = Fixture::new();
    let disk = t.0.join("chunks");
    fs::create_dir(&disk).unwrap();
    let images = t.0.join("images");
    fs::create_dir(&images).unwrap();
    fs::write(images.join("art.png"), b"reference image bytes").unwrap();
    let mut old = Chunk::new();
    old.set(1, 1, 1, crate::block::STONE);
    for x in [0, 3, 4] {
        fs::write(disk.join(format!("{x}_0_0.chunk")), old.as_bytes()).unwrap();
    }
    let mut world = World::new();
    world.set_block(1, 1, 1, crate::block::GLASS);
    world.set_block(33, 1, 1, crate::block::GLASS);
    assert!(world.evict_column(2, 0));
    let mut spill = Chunk::new();
    spill.set(2, 2, 2, crate::block::STONE);
    world.insert_chunk(2, 0, 0, spill);
    world.insert_chunk(4, 0, 0, Chunk::new());
    let mut save = neutral_save(42);
    let mut pet = crate::wolf::WolfData::untamed();
    pet.ownership.owner_pubkey = hex::encode(f.core.source_persona);
    save.saved_mobs.push(crate::save::SavedTamedPet {
        x: 5.0,
        y: 6.0,
        z: 7.0,
        data: crate::save::SavedTamedPetData::Wolf(pet),
    });
    let cart = crate::cart::CartData {
        cell: (8, 1, 2),
        progress: 0.45,
        speed: 0.07,
        ..Default::default()
    };
    save.carts.push(crate::save::SavedCart { data: cart });
    save.campfires.push(crate::save::SavedCampfire {
        x: 1,
        y: 2,
        z: 3,
        data: crate::campfire::CampfireData {
            fuel_ticks: 40,
            ..Default::default()
        },
    });
    let plan = crate::plan::PlanData::debug_3x3_stone();
    let marker = crate::plan::marker(&plan);
    save.face_blueprints.push(crate::save::SavedFaceBlueprint {
        x: 2,
        y: 3,
        z: 4,
        face: 1,
        plan,
    });
    save.exhibits.push(crate::exhibit::Exhibit {
        x: 3,
        y: 4,
        z: 5,
        presentation: crate::exhibit::Presentation::Wall,
        image_ref: "art.png".into(),
        width: 1.0,
        height: 1.0,
        yaw: 0.0,
        label: "Work".into(),
        link: None,
        sku: None,
        price: None,
    });
    let original_save = crate::save_format::encode_world_save(&save).unwrap();
    let mut original = WorldMeta::new("History");
    original.genesis_block_found = true;
    original.total_work = 57;
    original.cloud_save = true;
    let original_json = serde_json::to_vec(&original).unwrap();
    let meta = transfer_metadata(&original).unwrap();
    let mut a = crate::server::ServerPlayer::new(glam::Vec3::new(1.0, 2.0, 3.0));
    a.combat.health = 13.0;
    let mut b = crate::server::ServerPlayer::new(glam::Vec3::new(4.0, 5.0, 6.0));
    b.combat.health = 9.0;
    let row_a = crate::joiner_save::JoinerSave::capture(&a, [7; 16]).encode_seed().unwrap();
    let row_b = crate::joiner_save::JoinerSave::capture(&b, [7; 16]).encode_seed().unwrap();
    let npub_a = f.source.public_key().to_bech32().unwrap();
    let npub_b = f.recipient.public_key().to_bech32().unwrap();
    let participants = [
        ParticipantMember {
            npub: &npub_a,
            kind: MemberKind::Sidecar,
            schema: 1,
            bytes: &row_a,
        },
        ParticipantMember {
            npub: &npub_b,
            kind: MemberKind::Sidecar,
            schema: 1,
            bytes: &row_b,
        },
    ];
    let agreements = vec![
        Agreement {
            participant: f.core.source_persona,
            proof: [1; 32],
        },
        Agreement {
            participant: f.core.successor_persona,
            proof: [2; 32],
        },
    ];
    let packed = pack(
        CheckpointSource {
            meta: &meta,
            save: &save,
            world: &world,
            disk_chunks: Some(&disk),
            exhibit_dir: Some(&images),
            participants: &participants,
        },
        f.binding(agreements),
        &t.0,
    )
    .unwrap();
    assert!(
        !packed
            .manifest()
            .members
            .iter()
            .any(|m| m.path == "chunks/4_0_0.chunk"),
        "current all-air deletes stale disk content"
    );
    let mut import = Import::new(f.admitted(packed.manifest()), &t.0, 0).unwrap();
    feed(&mut import, packed.archive());
    let receipt = import.complete(0, &mut import::NoFault).unwrap();
    let raw = receipt.received_world();
    let working = receipt.working_world();
    let current = Chunk::from_bytes(&fs::read(raw.join("chunks/0_0_0.chunk")).unwrap()).unwrap();
    assert_eq!(current.get(1, 1, 1), crate::block::GLASS);
    let evicted = Chunk::from_bytes(&fs::read(raw.join("chunks/2_0_0.chunk")).unwrap()).unwrap();
    assert_eq!(evicted.get(1, 1, 1), crate::block::GLASS);
    assert_eq!(evicted.get(2, 2, 2), crate::block::AIR);
    let disk_only = Chunk::from_bytes(&fs::read(raw.join("chunks/3_0_0.chunk")).unwrap()).unwrap();
    assert_eq!(disk_only.get(1, 1, 1), crate::block::STONE);
    let back = bounded_decode::world(&fs::read(raw.join("world.dat")).unwrap()).unwrap();
    assert_eq!(back.carts[0].data.progress, 0.45);
    assert_eq!(back.campfires[0].data.fuel_ticks, 40);
    assert_eq!(back.saved_mobs, save.saved_mobs);
    assert_eq!(crate::plan::marker(&back.face_blueprints[0].plan), marker);
    assert_eq!(
        fs::read(raw.join("exhibits/art.png")).unwrap(),
        b"reference image bytes"
    );
    assert_eq!(
        fs::read(raw.join(format!("joiners/{npub_a}.dat"))).unwrap(),
        row_a
    );
    assert_eq!(
        fs::read(raw.join(format!("joiners/{npub_b}.dat"))).unwrap(),
        row_b
    );
    let received: WorldMeta =
        serde_json::from_slice(&fs::read(working.join("world_meta.json")).unwrap()).unwrap();
    assert!(received.pop_secret.is_some());
    assert_ne!(received.pop_secret, original.pop_secret);
    assert!(!received.cloud_save);
    assert!(received.published_to.is_none());
    assert!(received.genesis_block_found);
    assert_eq!(received.total_work, 57);
    assert_eq!(serde_json::to_vec(&original).unwrap(), original_json);
    assert_eq!(
        crate::save_format::encode_world_save(&save).unwrap(),
        original_save
    );
    assert_eq!(receipt.manifest_hash(), packed.manifest().digest().unwrap());
    assert_eq!(receipt.transform_version(), 1);
    assert!(
        !receipt.directory().join("world.dat").exists(),
        "inactive wrapper is not an ordinary world"
    );
    drop(import);
    assert!(
        receipt.directory().exists(),
        "late cleanup must not delete an admitted copy"
    );
    let mut resumed = Import::resume(f.admitted(packed.manifest()), &t.0).unwrap();
    assert_eq!(
        resumed
            .complete(0, &mut import::NoFault)
            .unwrap()
            .manifest_hash(),
        receipt.manifest_hash()
    );
}
#[test]
fn handover_checkpoint_personal_rows_and_operator_metadata_refused() {
    let t = Temp::new();
    let f = Fixture::new();
    let mut save = neutral_save(42);
    let clean = transfer_metadata(&WorldMeta::new("safe")).unwrap();
    save.player_y = 64.0;
    assert!(
        pack(
            CheckpointSource {
                meta: &clean,
                save: &save,
                world: &World::new(),
                disk_chunks: None,
                exhibit_dir: None,
                participants: &[]
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
    save.player_y = 0.0;
    save.locked_slots.push(1);
    assert!(validate_neutral(&save).is_err());
    save.locked_slots.clear();
    save.inventory.push(crate::save::SavedSlot::Empty);
    assert!(validate_neutral(&save).is_err());
    save.inventory.clear();
    assert!(validate_neutral(&save).is_ok());
    // Seat-numbered ownership the session never mapped refuses at the packer.
    let seat_owned: [fn(&mut WorldSave); 4] = [
        |s| s.plots.push(crate::plot::PlotData::from_marker(crate::plot::PlotOwner::LocalPlayer(0), 0, 64, 0)),
        |s| s.raid_kills.push(((0, 0), 0, 1)),
        |s| {
            let mut d = crate::wolf::WolfData::untamed();
            d.ownership.owner_pubkey = "local-player-0".into();
            s.saved_mobs.push(crate::save::SavedTamedPet {
                x: 0.0,
                y: 64.0,
                z: 0.0,
                data: crate::save::SavedTamedPetData::Wolf(d),
            });
        },
        |s| s.waypoints.push(crate::waypoint::Waypoint {
            id: 0,
            name: "pin".into(),
            pos: [0; 3],
            colour: crate::waypoint::MANUAL_COLOUR,
            kind: crate::waypoint::WaypointKind::Manual,
        }),
    ];
    for (i, case) in seat_owned.iter().enumerate() {
        let mut s = neutral_save(42);
        case(&mut s);
        assert!(validate_neutral(&s).is_err(), "seat-owned case {i} must refuse");
    }
    let dirty = WorldMeta::new("secret");
    assert!(validate_metadata(&dirty).is_err());
    let mut published = clean.clone();
    published.cloud_save = true;
    assert!(validate_metadata(&published).is_err());
}
#[test]
fn handover_checkpoint_strict_footer_payload_and_hostile_sequence_refused() {
    let bytes = crate::save_format::encode_world_save(&neutral_save(42)).unwrap();
    assert!(bounded_decode::world(&bytes).is_ok());
    let payload = &bytes[..bytes.len() - crate::save_format::SAVE_FOOTER_LEN];
    assert!(bounded_decode::world(payload).is_err());
    let mut truncated = payload[..payload.len() - 1].to_vec();
    truncated.extend_from_slice(&crate::save_format::footer_bytes(
        crate::save_format::SAVE_FORMAT_VERSION,
    ));
    assert!(bounded_decode::world(&truncated).is_err());
    let mut trailing = payload.to_vec();
    trailing.push(0);
    trailing.extend_from_slice(&crate::save_format::footer_bytes(
        crate::save_format::SAVE_FORMAT_VERSION,
    ));
    assert!(bounded_decode::world(&trailing).is_err());
    let mut newer = payload.to_vec();
    newer.extend_from_slice(&crate::save_format::footer_bytes(
        crate::save_format::SAVE_FORMAT_VERSION + 1,
    ));
    assert!(bounded_decode::world(&newer).is_err());
    // First Vec length follows seed, four floats and the64-bit hotbar index.
    let mut hostile = bytes.clone();
    hostile[28..36].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(bounded_decode::world(&hostile).is_err());
    let mut bad = bytes;
    bad[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(bounded_decode::world(&bad).is_err());
}
#[test]
fn handover_checkpoint_signed_offer_rejects_challenge_identity_and_context_substitution() {
    let t = Temp::new();
    let f = Fixture::new();
    let packed = simple_pack(&f, &t.0);
    let mut core = f.core.clone();
    core.manifest = packed.manifest().digest().unwrap();
    let challenge = [55; 16];
    let offer = pollster::block_on(sign_offer(&f.source, &core, challenge))
        .unwrap()
        .as_json();
    let manifest = packed.manifest().encode().unwrap();
    let authority = Authority::root(&f.root, [1; 16]).unwrap();
    for changed in 0..5 {
        let mut altered = core.clone();
        let mut ch = challenge;
        let mut recipient = f.core.successor_runtime;
        let mut offer = offer.clone();
        let mut now = 100;
        match changed {
            0 => ch = [56; 16],
            1 => altered.generation += 1,
            2 => recipient = f.core.source_runtime,
            3 => offer = offer.replace("checkpoint-offer", "forged"),
            _ => now = 801,
        };
        // Signed content is Base64; forcase3 corrupt signature explicitly.
        if changed == 3 {
            let mut event: serde_json::Value = serde_json::from_str(&offer).unwrap();
            event["sig"] = serde_json::Value::String("00".repeat(64));
            offer = event.to_string();
        }
        assert!(
            AdmittedManifest::verify(
                &manifest,
                offer.as_bytes(),
                OfferContext {
                    core: altered,
                    root: &f.root,
                    authority: &authority,
                    challenge: ch,
                    recipient_runtime: recipient,
                    recipient_persona: f.core.successor_persona,
                    now
                }
            )
            .is_err()
        );
    }
    let wrong = Authority::root(&f.root, [9; 16]).unwrap();
    assert!(
        AdmittedManifest::verify(
            &manifest,
            offer.as_bytes(),
            OfferContext {
                core,
                root: &f.root,
                authority: &wrong,
                challenge,
                recipient_runtime: f.core.successor_runtime,
                recipient_persona: f.core.successor_persona,
                now: 100
            }
        )
        .is_err()
    );
}
#[test]
fn handover_checkpoint_allocation_and_stream_caps_precede_growth() {
    let data = bincode::serialize(&vec![vec![7u8; 64]; 2]).unwrap();
    assert!(bounded_decode::binary::<Vec<Vec<u8>>>(&data, 128).is_err());
    assert_eq!(
        bounded_decode::binary::<Vec<Vec<u8>>>(&data, 65536).unwrap(),
        vec![vec![7u8; 64]; 2]
    );
    let hostile = u64::MAX.to_le_bytes();
    assert!(bounded_decode::binary::<Vec<()>>(&hostile, 128).is_err());
    let mut count = files::Count {
        inner: Vec::new(),
        bytes: 0,
        limit: 3,
    };
    use std::io::Write;
    assert!(count.write_all(&[1, 2, 3, 4]).is_err());
    assert_eq!(
        count.inner.capacity(),
        0,
        "cap rejects before output allocation"
    );
    let mut counted = files::Count {
        inner: Vec::new(),
        bytes: 0,
        limit: 1024,
    };
    let bomb = vec![0u8; 65536];
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&bomb).unwrap();
    let compressed = gz.finish().unwrap();
    let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
    assert!(std::io::copy(&mut decoder, &mut counted).is_err());
    assert!(counted.inner.len() <= 1024);
}
struct Fault {
    boundary: ImportBoundary,
    suffix: Option<&'static str>,
    fired: bool,
}
impl ImportObserver for Fault {
    fn reached(&mut self, b: ImportBoundary, path: &Path) -> std::io::Result<()> {
        if !self.fired
            && b == self.boundary
            && self
                .suffix
                .is_none_or(|s| path.to_string_lossy().ends_with(s))
        {
            self.fired = true;
            return Err(std::io::Error::other("injected actual I/O boundary"));
        }
        Ok(())
    }
}
#[test]
fn handover_checkpoint_real_io_faults_cleanup_and_postrename_restart_retry() {
    for boundary in [
        ImportBoundary::Write,
        ImportBoundary::FileSync,
        ImportBoundary::DirectorySync,
        ImportBoundary::BeforeRename,
        ImportBoundary::Renamed,
        ImportBoundary::ParentSynced,
    ] {
        let t = Temp::new();
        let f = Fixture::new();
        let packed = simple_pack(&f, &t.0);
        let mut importing = Import::new(f.admitted(packed.manifest()), &t.0, 0).unwrap();
        feed(&mut importing, packed.archive());
        let mut fault = Fault {
            boundary,
            suffix: None,
            fired: false,
        };
        assert!(importing.complete(0, &mut fault).is_err());
        assert!(fault.fired);
        if [ImportBoundary::Renamed, ImportBoundary::ParentSynced].contains(&boundary) {
            // Real process-restart semantics: old object is dropped, renamed
            // owned evidence survives and new authenticated admission resumes.
            drop(importing);
            let mut restart = Import::resume(f.admitted(packed.manifest()), &t.0).unwrap();
            let receipt = restart.complete(0, &mut import::NoFault).unwrap();
            assert!(receipt.received_world().join("world.dat").is_file());
            drop(restart);
            assert!(receipt.directory().exists());
        } else {
            drop(importing);
            assert!(!fs::read_dir(&t.0).unwrap().any(|e| {
                e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".handover-receive-")
            }));
        }
    }
}
#[test]
fn handover_checkpoint_collisions_never_replace_file_or_incomplete_world() {
    for directory in [false, true] {
        let t = Temp::new();
        let f = Fixture::new();
        let packed = simple_pack(&f, &t.0);
        let mut importer = Import::new(f.admitted(packed.manifest()), &t.0, 0).unwrap();
        feed(&mut importer, packed.archive());
        let destination = t.0.join(format!(
            "handover-{}-{}",
            hex::encode([2u8; 16]),
            hex::encode([4u8; 16])
        ));
        if directory {
            fs::create_dir(&destination).unwrap();
            fs::write(destination.join("Workshop"), b"retained").unwrap();
        } else {
            fs::write(&destination, b"retained").unwrap();
        }
        assert!(importer.complete(0, &mut import::NoFault).is_err());
        if directory {
            assert_eq!(fs::read(destination.join("Workshop")).unwrap(), b"retained");
        } else {
            assert_eq!(fs::read(&destination).unwrap(), b"retained");
        }
        assert!(Import::new(f.admitted(packed.manifest()), &t.0, 0).is_err());
    }
}
#[test]
fn handover_checkpoint_raw_receipt_corruption_fails_closed_on_restart() {
    let t = Temp::new();
    let f = Fixture::new();
    let packed = simple_pack(&f, &t.0);
    let mut importer = Import::new(f.admitted(packed.manifest()), &t.0, 0).unwrap();
    feed(&mut importer, packed.archive());
    let receipt = importer.complete(0, &mut import::NoFault).unwrap();
    let raw = receipt.received_world().join("world.dat");
    fs::write(raw, b"corrupted").unwrap();
    assert!(Import::resume(f.admitted(packed.manifest()), &t.0).is_err());
    drop(importer);
    assert!(receipt.directory().exists());
}
struct TestMember {
    path: String,
    bytes: Vec<u8>,
    kind: tar::EntryType,
}
fn altered_archive(
    p: &PackedCheckpoint,
    alter: impl FnOnce(&mut Vec<TestMember>),
    update_hashes: bool,
) -> (PathBuf, super::super::manifest::Manifest) {
    let gz = flate2::read::GzDecoder::new(fs::File::open(p.archive()).unwrap());
    let mut tar = tar::Archive::new(gz);
    let mut rows: Vec<_> = tar
        .entries()
        .unwrap()
        .map(|entry| {
            let mut e = entry.unwrap();
            let path = e.path().unwrap().to_string_lossy().to_string();
            let mut bytes = Vec::new();
            e.read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() < 65536);
            TestMember {
                path,
                bytes,
                kind: tar::EntryType::Regular,
            }
        })
        .collect();
    alter(&mut rows);
    let file = p.archive().parent().unwrap().join("altered.tar.gz");
    let mut gz = flate2::write::GzEncoder::new(
        fs::File::create(&file).unwrap(),
        flate2::Compression::default(),
    );
    {
        let mut tar = tar::Builder::new(&mut gz);
        for row in &rows {
            let mut h = tar::Header::new_ustar();
            h.set_size(row.bytes.len() as u64);
            h.set_mode(0o600);
            h.set_entry_type(row.kind);
            // Raw header construction intentionally allows malformed traversal names.
            assert!(row.path.len() <= 100);
            h.as_mut_bytes()[..row.path.len()].copy_from_slice(row.path.as_bytes());
            h.set_cksum();
            tar.append(&h, row.bytes.as_slice()).unwrap();
        }
        tar.finish().unwrap();
    }
    gz.finish().unwrap();
    let mut manifest = p.manifest().clone();
    let (length, hash) = files::digest(
        fs::File::open(&file).unwrap(),
        super::super::manifest::COMPRESSED_LIMIT,
    )
    .unwrap();
    manifest.stream_length = length;
    manifest.stream_hash = hash;
    if update_hashes {
        for member in &mut manifest.members {
            if let Some(row) = rows.iter().find(|r| r.path == member.path) {
                member.length = row.bytes.len() as u64;
                member.digest = super::super::hash(&row.bytes);
            }
        }
    }
    (file, manifest)
}
#[test]
fn handover_checkpoint_member_hash_footer_missing_extra_duplicate_and_types_refused() {
    for case in 0..11 {
        let t = Temp::new();
        let f = Fixture::new();
        let p = simple_pack(&f, &t.0);
        let (file, manifest) = altered_archive(
            &p,
            |rows| match case {
                0 => rows[0].bytes[0] ^= 1,
                1 => {
                    rows.pop();
                }
                2 => rows.push(TestMember {
                    path: "unexpected".into(),
                    bytes: vec![1],
                    kind: tar::EntryType::Regular,
                }),
                3 => rows.insert(
                    1,
                    TestMember {
                        path: rows[0].path.clone(),
                        bytes: rows[0].bytes.clone(),
                        kind: tar::EntryType::Regular,
                    },
                ),
                4 => rows[0].path = "../world.dat".into(),
                5 => rows[0].kind = tar::EntryType::Symlink,
                6 => rows[0].kind = tar::EntryType::Link,
                7 => rows[0].kind = tar::EntryType::Char,
                8 => rows[0].kind = tar::EntryType::GNULongName,
                9 => {
                    let at = rows.iter().position(|r| r.path == "world.dat").unwrap();
                    let len = rows[at].bytes.len();
                    rows[at].bytes[len - 1] ^= 1;
                }
                _ => rows.reverse(),
            },
            case == 9,
        );
        let mut importing = Import::new(f.admitted(&manifest), &t.0, 0).unwrap();
        feed(&mut importing, &file);
        assert!(
            importing.complete(0, &mut import::NoFault).is_err(),
            "malformed case{case}"
        );
    }
}
#[test]
fn handover_checkpoint_actual_compressed_and_logical_caps_and_bomb_tail() {
    let t = Temp::new();
    let f = Fixture::new();
    let meta = transfer_metadata(&WorldMeta::new("small")).unwrap();
    let save = neutral_save(42);
    let world = World::new();
    for limits in [
        Limits {
            compressed: 8,
            logical: 512 * 1024 * 1024,
            members: 65536,
        },
        Limits {
            compressed: 128 * 1024 * 1024,
            logical: 8,
            members: 65536,
        },
        Limits {
            compressed: 128 * 1024 * 1024,
            logical: 512 * 1024 * 1024,
            members: 1,
        },
    ] {
        assert!(
            pack_limited(
                CheckpointSource {
                    meta: &meta,
                    save: &save,
                    world: &world,
                    disk_chunks: None,
                    exhibit_dir: None,
                    participants: &[]
                },
                f.binding(vec![]),
                &t.0,
                limits
            )
            .is_err()
        );
    }
    let packed = simple_pack(&f, &t.0);
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(fs::File::open(packed.archive()).unwrap())
        .read_to_end(&mut decoded)
        .unwrap();
    assert!(decoded.len() < 65536);
    // A source-signed stream whose valid tar ends before a highly compressed
    // zero suffix still hits the same bounded decompressor in real import.
    decoded.extend_from_slice(&[0u8; 65536]);
    let path = t.0.join("bomb.gz");
    let mut gz = flate2::write::GzEncoder::new(
        fs::File::create(&path).unwrap(),
        flate2::Compression::default(),
    );
    use std::io::Write;
    gz.write_all(&decoded).unwrap();
    gz.finish().unwrap();
    let mut manifest = packed.manifest().clone();
    let (n, h) = files::digest(fs::File::open(&path).unwrap(), 128 * 1024 * 1024).unwrap();
    manifest.stream_length = n;
    manifest.stream_hash = h;
    let logical = manifest.members.iter().map(|m| m.length).sum();
    let limits = Limits {
        compressed: 128 * 1024 * 1024,
        logical,
        members: 65536,
    };
    let mut importer = Import::new_limited(f.admitted(&manifest), &t.0, 0, limits).unwrap();
    feed(&mut importer, &path);
    assert!(importer.complete(0, &mut import::NoFault).is_err());
}
#[test]
fn handover_checkpoint_required_image_sidecar_codec_and_unsafe_disk_entry_refused() {
    let t = Temp::new();
    let f = Fixture::new();
    let meta = transfer_metadata(&WorldMeta::new("safe")).unwrap();
    let mut save = neutral_save(42);
    let world = World::new();
    save.exhibits.push(crate::exhibit::Exhibit {
        x: 0,
        y: 0,
        z: 0,
        presentation: crate::exhibit::Presentation::Wall,
        image_ref: "required.png".into(),
        width: 1.0,
        height: 1.0,
        yaw: 0.0,
        label: String::new(),
        link: None,
        sku: None,
        price: None,
    });
    assert!(
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: None,
                exhibit_dir: None,
                participants: &[]
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
    let images = t.0.join("images");
    fs::create_dir(&images).unwrap();
    assert!(
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: None,
                exhibit_dir: Some(&images),
                participants: &[]
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
    fs::File::create(images.join("required.png"))
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: None,
                exhibit_dir: Some(&images),
                participants: &[]
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
    save.exhibits.clear();
    let npub = f.source.public_key().to_bech32().unwrap();
    let bad = [ParticipantMember {
        npub: &npub,
        kind: MemberKind::Sidecar,
        schema: 1,
        bytes: b"not current JoinerSave",
    }];
    assert!(
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: None,
                exhibit_dir: None,
                participants: &bad
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
    let bad_path = [ParticipantMember {
        npub: "../private",
        kind: MemberKind::Progression,
        schema: 1,
        bytes: b"opaque",
    }];
    assert!(
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: None,
                exhibit_dir: None,
                participants: &bad_path
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
    let chunks = t.0.join("chunks");
    fs::create_dir(&chunks).unwrap();
    fs::write(chunks.join("01_0_0.chunk"), vec![0; 8192]).unwrap();
    assert!(
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: Some(&chunks),
                exhibit_dir: None,
                participants: &[]
            },
            f.binding(vec![]),
            &t.0
        )
        .is_err()
    );
}
#[test]
fn handover_checkpoint_required_exhibit_and_sidecar_write_failures_abort_import() {
    for suffix in ["received/exhibits/art.png", "received/joiners"] {
        let t = Temp::new();
        let f = Fixture::new();
        let images = t.0.join("images");
        fs::create_dir(&images).unwrap();
        fs::write(images.join("art.png"), b"required bytes").unwrap();
        let mut save = neutral_save(42);
        save.exhibits.push(crate::exhibit::Exhibit {
            x: 0,
            y: 0,
            z: 0,
            presentation: crate::exhibit::Presentation::Wall,
            image_ref: "art.png".into(),
            width: 1.0,
            height: 1.0,
            yaw: 0.0,
            label: String::new(),
            link: None,
            sku: None,
            price: None,
        });
        let row = crate::joiner_save::JoinerSave::capture(
            &crate::server::ServerPlayer::new(glam::Vec3::ZERO),
            [7; 16],
        ).encode_seed().unwrap();
        let npub = f.source.public_key().to_bech32().unwrap();
        let participants = [ParticipantMember {
            npub: &npub,
            kind: MemberKind::Sidecar,
            schema: 1,
            bytes: &row,
        }];
        let meta = transfer_metadata(&WorldMeta::new("parts")).unwrap();
        let packed = pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &World::new(),
                disk_chunks: None,
                exhibit_dir: Some(&images),
                participants: &participants,
            },
            f.binding(vec![Agreement {
                participant: f.core.source_persona,
                proof: [1; 32],
            }]),
            &t.0,
        )
        .unwrap();
        let mut importing = Import::new(f.admitted(packed.manifest()), &t.0, 0).unwrap();
        feed(&mut importing, packed.archive());
        struct MemberFault<'a>(&'a str, bool);
        impl ImportObserver for MemberFault<'_> {
            fn reached(&mut self, b: ImportBoundary, p: &Path) -> std::io::Result<()> {
                if b == ImportBoundary::Write && p.to_string_lossy().contains(self.0) {
                    self.1 = true;
                    return Err(std::io::Error::other("required member write"));
                }
                Ok(())
            }
        }
        let mut fault = MemberFault(suffix, false);
        assert!(importing.complete(0, &mut fault).is_err());
        assert!(fault.1);
        drop(importing);
        assert!(!fs::read_dir(&t.0).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("handover-")
        }));
    }
}
#[cfg(unix)]
#[test]
fn handover_checkpoint_symlink_source_and_atomic_directory_collision_refused() {
    let t = Temp::new();
    let real = t.0.join("real");
    fs::write(&real, b"private").unwrap();
    let link = t.0.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(files::open(&link).is_err());
    let source = t.0.join("source");
    let destination = t.0.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("retained"), b"original").unwrap();
    assert!(files::rename_new(&source, &destination).is_err());
    assert!(source.exists());
    assert_eq!(fs::read(destination.join("retained")).unwrap(), b"original");
    // Ordinary rename replaces an EMPTY existing directory; no-replace must
    // preserve the target inode and all source state in this exact race seam.
    use std::os::unix::fs::MetadataExt;
    let empty = t.0.join("empty-existing");
    fs::create_dir(&empty).unwrap();
    let target_inode = fs::metadata(&empty).unwrap().ino();
    let source_inode = fs::metadata(&source).unwrap().ino();
    fs::write(source.join("source-state"), b"untouched").unwrap();
    assert!(files::rename_new(&source, &empty).is_err());
    assert_eq!(fs::metadata(&empty).unwrap().ino(), target_inode);
    assert_eq!(fs::metadata(&source).unwrap().ino(), source_inode);
    assert_eq!(fs::read(source.join("source-state")).unwrap(), b"untouched");
    assert_eq!(fs::read(destination.join("retained")).unwrap(), b"original");
}
#[test]
fn handover_checkpoint_signed_archive_missing_required_image_refused_by_importer() {
    let t = Temp::new();
    let f = Fixture::new();
    let packed = simple_pack(&f, &t.0);
    let (file, manifest) = altered_archive(
        &packed,
        |rows| {
            let at = rows.iter().position(|r| r.path == "world.dat").unwrap();
            let mut save = neutral_save(42);
            save.exhibits.push(crate::exhibit::Exhibit {
                x: 0,
                y: 1,
                z: 2,
                presentation: crate::exhibit::Presentation::Wall,
                image_ref: "missing.png".into(),
                width: 1.0,
                height: 1.0,
                yaw: 0.0,
                label: String::new(),
                link: None,
                sku: None,
                price: None,
            });
            rows[at].bytes = crate::save_format::encode_world_save(&save).unwrap();
        },
        true,
    );
    // Genuine new signed offer, correct member hashes and compressed hash:
    // this specifically exercises cross-member validation, not hash rejection.
    let mut importer = Import::new(f.admitted(&manifest), &t.0, 0).unwrap();
    feed(&mut importer, &file);
    assert!(matches!(
        importer.complete(0, &mut import::NoFault),
        Err(Error::Invalid("required exhibit member absent"))
    ));
}
#[test]
fn handover_checkpoint_offline_evidence_reverifies_offer_and_fails_closed_on_tamper() {
    let t = Temp::new();
    let f = Fixture::new();
    let packed = simple_pack(&f, &t.0);
    let mut importer = Import::new(f.admitted(packed.manifest()), &t.0, 0).unwrap();
    feed(&mut importer, packed.archive());
    let receipt = importer.complete(0, &mut import::NoFault).unwrap();
    drop(importer);
    let expected = || OfflineRecovery {
        expected_context: f.root.digest(),
        recipient_runtime: f.core.successor_runtime,
        recipient_persona: f.core.successor_persona,
        source_authority: None,
    };
    // No live source/fresh offer or current-time assertion: recovery is ONLY
    // historical file evidence and local fsync, never current lifecycle power.
    let recovered = RecoveredCheckpoint::recover(&t.0, [2; 16], [4; 16], expected()).unwrap();
    assert_eq!(recovered.manifest_hash(), receipt.manifest_hash());
    let wrong = OfflineRecovery {
        expected_context: [99; 32],
        ..expected()
    };
    assert!(RecoveredCheckpoint::recover(&t.0, [2; 16], [4; 16], wrong).is_err());
    let path = receipt.directory().join("admission.dat");
    let original = fs::read(&path).unwrap();
    let mut corrupted = original.clone();
    corrupted[10] ^= 1;
    fs::write(&path, &corrupted).unwrap();
    assert!(RecoveredCheckpoint::recover(&t.0, [2; 16], [4; 16], expected()).is_err());
    // Recompute the accidental checksum around an invalid Schnorr signature:
    // true signed-offer verification must still reject it.
    let mut forged = original;
    let at = forged.windows(7).position(|w| w == b"\"sig\":\"").unwrap() + 7;
    forged[at] = if forged[at] == b'0' { b'1' } else { b'0' };
    let end = forged.len() - 32;
    let checksum = super::super::hash(&forged[..end]);
    forged[end..].copy_from_slice(&checksum);
    fs::write(&path, &forged).unwrap();
    assert!(RecoveredCheckpoint::recover(&t.0, [2; 16], [4; 16], expected()).is_err());
    assert!(
        receipt.directory().exists(),
        "failed recovery never deletes retained data"
    );
}
#[test]
fn handover_checkpoint_participant_record_is_strictly_bound_to_manifest_context() {
    use super::super::personal::{PersonalRecord, RecordContext};
    let t = Temp::new();
    let f = Fixture::new();
    let npub = f.recipient.public_key().to_bech32().unwrap();
    let pk = f.recipient.public_key().to_bytes();
    let context = |transfer, generation| RecordContext {
        participant: pk,
        lineage: f.core.lineage,
        generation,
        transfer,
    };
    let rep = [((1, 2), -5i16)];
    let record = |c| {
        PersonalRecord::from_maps(c, rep.iter().map(|(k, v)| (k, v)), [], [], None)
            .unwrap()
            .encode()
            .unwrap()
    };
    let meta = transfer_metadata(&WorldMeta::new("source")).unwrap();
    let save = neutral_save(42);
    let world = World::new();
    let agreement = vec![Agreement {
        participant: pk,
        proof: [9; 32],
    }];
    let pack_with = |bytes: &[u8]| {
        let rows = [ParticipantMember {
            npub: &npub,
            kind: MemberKind::Progression,
            schema: 1,
            bytes,
        }];
        pack(
            CheckpointSource {
                meta: &meta,
                save: &save,
                world: &world,
                disk_chunks: None,
                exhibit_dir: None,
                participants: &rows,
            },
            f.binding(agreement.clone()),
            &t.0,
        )
    };
    // A record bound to another transfer is refused before it is packed.
    assert_eq!(
        pack_with(&record(context([5; 16], f.core.generation))).err(),
        Some(super::super::Error::Invalid("participant context"))
    );
    let good = record(context(f.core.transfer, f.core.generation));
    let p = pack_with(&good).unwrap();
    let mut importing = Import::new(f.admitted(p.manifest()), &t.0, 0).unwrap();
    feed(&mut importing, p.archive());
    let done = importing.complete(0, &mut import::NoFault).unwrap();
    let path = format!("participants/{npub}.dat");
    assert_eq!(fs::read(done.received_world().join(&path)).unwrap(), good);
    // The importer decodes against the MANIFEST context on its own: a
    // genuinely hashed and signed member bound to another generation fails.
    let t2 = Temp::new();
    let p2 = pack_with(&good).unwrap();
    let foreign = record(context(f.core.transfer, f.core.generation + 1));
    let (file, manifest) = altered_archive(
        &p2,
        |rows| {
            let at = rows.iter().position(|r| r.path == path).unwrap();
            rows[at].bytes = foreign.clone();
        },
        true,
    );
    let mut importing = Import::new(f.admitted(&manifest), &t2.0, 0).unwrap();
    feed(&mut importing, &file);
    assert_eq!(
        importing.complete(0, &mut import::NoFault).err(),
        Some(super::super::Error::Invalid("participant context"))
    );
}

#[cfg(test)]
mod sync_pool_tests {
    use super::super::sync_pool::SyncPool;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    #[cfg(unix)]
    #[test]
    fn handover_import_sync_pool_propagates_a_real_flush_failure() {
        let mut pool = SyncPool::new().unwrap();
        let file = crate::fsync::open_written(&mut std::fs::OpenOptions::new(), std::path::Path::new("/dev/null")).unwrap();
        pool.submit(file).unwrap();
        assert!(pool.finish().is_err(), "a real fsync failure was hidden");
    }

    #[test]
    fn handover_import_sync_pool_drains_every_file_after_a_worker_error() {
        let temp = super::Temp::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counted = attempts.clone();
        let mut pool = SyncPool::start(move |file| {
            counted.fetch_add(1, Ordering::SeqCst);
            if file.metadata()?.len() == 1 {
                Err(std::io::Error::other("injected worker sync error"))
            } else {
                crate::fsync::sync_written(file)
            }
        }).unwrap();
        for i in 0..37 {
            let file = super::files::new_written(&temp.0.join(format!("member-{i}"))).unwrap();
            file.set_len(if i == 0 { 1 } else { 2 }).unwrap();
            pool.submit(file).unwrap();
        }
        assert!(pool.finish().is_err(), "a worker sync error was lost");
        assert_eq!(attempts.load(Ordering::SeqCst), 37, "finish returned before every worker joined");
    }

    #[test]
    fn handover_import_sync_pool_joins_before_scratch_cleanup() {
        // An extraction/observer error exits before finish: Drop must still
        // drain and join, so scratch cleanup cannot race an open writer.
        let temp = super::Temp::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counted = attempts.clone();
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let blocked = gate.clone();
        let mut pool = SyncPool::start(move |file| {
            let (lock, wake) = &*blocked;
            let _released = wake.wait_while(lock.lock().unwrap(), |released| !*released).unwrap();
            counted.fetch_add(1, Ordering::SeqCst);
            crate::fsync::sync_written(file)
        }).unwrap();
        for i in 0..8 {
            pool.submit(super::files::new_written(&temp.0.join(format!("cancel-{i}"))).unwrap()).unwrap();
        }
        let (begun, waiting) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let cleanup = std::thread::spawn(move || {
            begun.send(()).unwrap();
            drop(pool);
            done.send(()).unwrap();
        });
        waiting.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let early = finished.recv_timeout(std::time::Duration::from_millis(250)).is_ok();
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        if !early {
            finished.recv_timeout(std::time::Duration::from_secs(5)).expect("cleanup never joined its workers");
        }
        cleanup.join().unwrap();
        assert!(!early, "scratch cleanup returned while its sync workers were still blocked");
        assert_eq!(attempts.load(Ordering::SeqCst), 8, "scratch cleanup raced a sync worker");
    }
}


#[test]
fn handover_checkpoint_refuses_mutable_or_legacy_sidecars_before_ready() {
    let saved = crate::joiner_save::JoinerSave::capture(&crate::server::ServerPlayer::new(glam::Vec3::ZERO), [7; 16]);
    let seed = saved.encode_seed().unwrap();
    assert_eq!(seed[0], 2);
    super::bounded_decode::sidecar(&seed).unwrap();
    let mutable = saved.encode().unwrap();
    assert_eq!(mutable[0], 3);
    assert!(super::bounded_decode::sidecar(&mutable).is_err(), "Ready accepted an unbound live v3 that cannot bootstrap");
    let mut legacy = seed; legacy[0] = 1;
    assert!(super::bounded_decode::sidecar(&legacy).is_err(), "private legacy checkpoint bypassed the immutable seed contract");
}
