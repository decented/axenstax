//! Truthful pre-begin boundary for the later retained-source acceptance path.
use super::*;
use crate::test_integration::handover_carry_fixture as activity;

#[test]
#[ignore = "needs two real GameState GPU adapters and signed quest/menu setup"]
fn game_harness_d2c3_retained_source_preserves_active_own_quests_and_current_timers() {
    let _worlds = crate::save::WorldsRootGuard::new("active-retained-boundary");
    let dir = Tmp::new("active-retained-boundary");
    let mut source = activity::active_source("active-retained-source", [SOURCE_PERSONA; 32]);
    let mut peer = HeadlessGame::boot_into_world("active-retained-peer");
    activity::start_lent(&mut source);
    let client = source.state.hosted_server.as_mut().unwrap().attach_test_remote_with_binding(Some(BINDING));
    let (_seat, observation) = activity::attach(
        &mut source,
        &mut peer,
        client,
        [RECIPIENT_PERSONA; 32],
        key(SOURCE_PERSONA),
        key(SOURCE_RUNTIME),
    );
    let imports = dir.0.join("imports");
    peer.state.handover_fixture = Some(Fixture {
        signed_in: Some(key(RECIPIENT_PERSONA)),
        recipient: Some(Box::new(move || Ok(RecipientEnv { identity: identity(), imports: imports.clone() }))),
    });
    let seat = activity::active_recipient(&mut source, &mut peer, &observation);
    assert!(seat.2 == key(RECIPIENT_PERSONA), "actual recipient npub differs");
    activity::assert_boundary_ready(&source, &peer, seat);
    assert!(source.state.handover.is_none());
    let before = activity::boundary(&source, &peer, seat);
    let context = root();
    let prefix = fresh_prefix(&context);
    let result = source.state.begin_handover_with(
        seat,
        SourceEnv {
            runtime: keys(SOURCE_RUNTIME),
            root: context.clone(),
            scratch: dir.0.join("scratch"),
            bearer: bearer(),
            attestation: source_attestation(),
            continuation: None,
        },
        now(),
    );
    eprintln!("active-retained first begin result={result:?} slot={} generation={}", seat.0, seat.1);
    if result.is_err() {
        assert!(source.state.handover.is_none());
        assert!(
            activity::boundary(&source, &peer, seat) == before,
            "refused pre-retirement boundary changed source authority or live state"
        );
        assert_eq!(fresh_prefix(&context), prefix, "refused begin changed the fresh independent root prefix");
    }
    // FIRST OWN: activity was created BEFORE this actual begin, unlike the
    // original frozen_after callback. No retained/Imported proof is fabricated.
    assert!(result.is_ok(), "genuine active source must reach the retained-copy transfer boundary: {result:?}");
    let completed = activity::continuation::transfer(&mut source, &mut peer, seat, now);
    assert!(completed.cut_tick > 0);
    let original_dir = crate::save::world_dir(&source.state.world_name);
    let retired_data = world_data(&original_dir);
    assert!(crate::handover::live::authority::open_guard(&original_dir).is_err());
    assert!(source.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    let retained = completed.retained.clone();
    activity::continuation::open(&mut source, &retained);
    activity::continuation::restored(&mut source, &completed.source, completed.scope, key(SOURCE_PERSONA));
    actual_cow(&mut source);
    let saved = activity::native(&source);
    source.state.save_live_world().unwrap();
    assert!(source.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    activity::continuation::open(&mut source, &retained);
    activity::continuation::restored(&mut source, &saved, completed.scope, key(SOURCE_PERSONA));
    // Save an exact current baseline, then genuinely play and discard a
    // later credit. Discard must not install an old transfer snapshot.
    source.state.save_live_world().unwrap();
    let selected = std::fs::read(crate::save::world_dir(&retained).join("world.dat")).unwrap();
    let baseline = activity::native(&source);
    actual_cow(&mut source);
    assert_ne!(activity::native(&source).kills, baseline.kills);
    assert!(source.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    assert_eq!(std::fs::read(crate::save::world_dir(&retained).join("world.dat")).unwrap(), selected);
    activity::continuation::open(&mut source, &retained);
    activity::continuation::restored(&mut source, &baseline, completed.scope, key(SOURCE_PERSONA));
    assert!(source.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    let retained_dir = crate::save::world_dir(&retained);
    let portable = "active-retained-complete-folder";
    let portable_dir = crate::save::world_dir(portable);
    copy_complete(&retained_dir, &portable_dir);
    assert_eq!(folder_bytes(&portable_dir), folder_bytes(&retained_dir), "all proof/member/chunk files move together");
    let fresh_ledger = dir.0.join("fresh-install").join("ledger");
    let _missing_install = crate::handover::ledger::fixture_install_path(&fresh_ledger);
    assert!(!fresh_ledger.exists());
    // Wrong profile and an actual strict directory failure both preserve the
    // complete portable selected folder and cannot start gameplay.
    source.state.handover_fixture = Some(Fixture { signed_in: Some(key(0x55)), recipient: None });
    let protected = folder_bytes(&portable_dir);
    refuse(&mut source, portable);
    assert_eq!(folder_bytes(&portable_dir), protected);
    source.state.handover_fixture = Some(Fixture { signed_in: Some(key(SOURCE_PERSONA)), recipient: None });
    #[cfg(unix)]
    {
        let fault = crate::fsync::tests::fail_directory_on_match(&portable_dir, 1);
        refuse(&mut source, portable);
        assert!(fault.failures() > 0, "the actual native loader attempted the selected directory barrier");
        assert_eq!(folder_bytes(&portable_dir), protected);
        let failures = fault.failures();
        refuse(&mut source, portable);
        assert!(fault.failures() > failures, "persistent failure is retried at the actual boundary");
        assert_eq!(folder_bytes(&portable_dir), protected);
        drop(fault);
    }
    if let crate::GameMode::Menu(menu) = &mut source.state.mode {
        menu.notice = None;
    }
    let saved = crate::save::read_world_save(&std::fs::read(portable_dir.join("world.dat")).unwrap()).unwrap();
    let saved_personal = crate::joiner_save::JoinerSave::decode(&saved.local_personal.unwrap().0)
        .unwrap()
        .personal
        .unwrap()
        .record()
        .unwrap();
    activity::continuation::open(&mut source, portable);
    activity::continuation::restored(&mut source, &saved_personal, completed.scope, key(SOURCE_PERSONA));
    assert!(!fresh_ledger.exists(), "directory proof stands without inventing an installation ledger entry");
    activity::continuation::complete_dedicated_and_return(&mut source, [SOURCE_PERSONA; 32]);
    assert_eq!(world_data(&original_dir), retired_data, "retained play never rewrites original world data");
    assert!(crate::handover::live::authority::open_guard(&original_dir).is_err());
}

fn fresh_prefix(context: &RootContext) -> crate::handover::certificate::Prefix {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(crate::handover::live::continuation::lookup(
            context.clone(),
            bearer(),
            None,
            now(),
            &std::sync::atomic::AtomicBool::new(false),
        ))
        .unwrap()
        .prefix
}

fn actual_cow(game: &mut HeadlessGame) {
    let pos = game.state.players[0].player.pos + glam::Vec3::X;
    let victim = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Cow, pos);
    game.state.ecs.get::<&mut crate::combat::Health>(victim).unwrap().current = 1.0;
    let slot = &game.state.players[0];
    assert!(crate::combat::strike(
        &mut game.state.ecs,
        victim,
        &crate::combat::Swing {
            attacker: crate::combat::Attacker::Local(0),
            owner_key: "local-player-0",
            eye: slot.player.eye_pos(),
            look_dir: (pos - slot.player.eye_pos()).normalize(),
            crit: false,
            sprinting: false,
            sneaking: false,
            base_damage: slot.inventory.hotbar_attack_damage(slot.hotbar_slot),
        }
    ));
    let old = game.state.players[0].kill_counter.get(&crate::mob::MobType::Cow).copied().unwrap_or(0);
    game.ticks(1);
    assert!(!game.state.ecs.contains(victim));
    assert_eq!(game.state.players[0].kill_counter[&crate::mob::MobType::Cow], old + 1);
}
fn copy_complete(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink());
        if kind.is_dir() {
            copy_complete(&entry.path(), &target.join(entry.file_name()));
        } else {
            assert!(kind.is_file());
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}
fn folder_bytes(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                walk(base, &entry.path(), out);
            } else {
                out.insert(entry.path().strip_prefix(base).unwrap().to_owned(), std::fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}
fn world_data(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    folder_bytes(dir).into_iter().filter(|(path, _)| !path.starts_with("handover")).collect()
}
fn refuse(game: &mut HeadlessGame, folder: &str) {
    if let crate::GameMode::Menu(menu) = &mut game.state.mode {
        menu.notice = None;
    }
    game.state.pending_menu_action = Some((
        crate::menu::MenuAction::LoadWorld(folder.into(), crate::spawn_pref::SpawnPref::Default),
        web_time::Instant::now(),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        game.frames(1);
        if let crate::GameMode::Menu(menu) = &game.state.mode {
            if menu.notice.is_some() {
                break;
            }
        }
        assert!(!matches!(game.state.mode, crate::GameMode::Playing), "protected current folder entered play");
        assert!(std::time::Instant::now() < deadline, "actual protected native load did not finish refusing");
    }
}

/// The earlier baseline deliberately contains no private Plan. This independent
/// authored physical fixture proves all nested allocations across cut/abort,
/// retry and Prepared close, followed by checked own-record native opens.
#[test]
#[ignore = "needs two real GPU adapters, signed current ACKs and private Plan bodies"]
fn game_harness_d2c3_active_carry_keeps_private_plan_allocations_through_abort_and_prepared_close() {
    let _worlds = crate::save::WorldsRootGuard::new("active-private-custody");
    let dir = Tmp::new("active-private-custody");
    let mut source = activity::active_source("active-private-source", [SOURCE_PERSONA; 32]);
    let mut peer = HeadlessGame::boot_into_world("active-private-peer");
    activity::start_lent(&mut source);
    let client = source.state.hosted_server.as_mut().unwrap().attach_test_remote_with_binding(Some(BINDING));
    let (_, observation) = activity::attach(
        &mut source,
        &mut peer,
        client,
        [RECIPIENT_PERSONA; 32],
        key(SOURCE_PERSONA),
        key(SOURCE_RUNTIME),
    );
    let imports = dir.0.join("imports");
    peer.state.handover_fixture = Some(Fixture {
        signed_in: Some(key(RECIPIENT_PERSONA)),
        recipient: Some(Box::new(move || Ok(RecipientEnv { identity: identity(), imports: imports.clone() }))),
    });
    let seat = activity::active_recipient(&mut source, &mut peer, &observation);
    activity::assert_boundary_ready(&source, &peer, seat);
    // Authored bodies are seeded only AFTER the no-Plan activity baseline.
    let custody = activity::custody::seed_authored(&mut source, &mut peer, seat);
    let begin = |source: &mut HeadlessGame, scratch: &str| {
        source
            .state
            .begin_handover_with(
                seat,
                SourceEnv {
                    runtime: keys(SOURCE_RUNTIME),
                    root: root(),
                    scratch: dir.0.join(scratch),
                    bearer: bearer(),
                    attestation: source_attestation(),
                    continuation: None,
                },
                now(),
            )
            .unwrap();
    };
    begin(&mut source, "first-scratch");
    activity::continuation::abort_current_with_custody(&mut source, &mut peer, seat, &custody);
    custody.assert_unchanged(&source, &peer);
    begin(&mut source, "retry-scratch");
    let completed =
        activity::continuation::transfer_with_custody(&mut source, &mut peer, seat, now, Some(&custody), true);
    assert!(source.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    activity::continuation::open(&mut source, &completed.retained);
    activity::continuation::restored(&mut source, &completed.source, completed.scope, key(SOURCE_PERSONA));
    custody.assert_source_contents(&source);
    let imported = activity::continuation::promoted(&mut peer, completed.transfer);
    // Prepared close has already left the peer in the lobby; promotion/open
    // must reconstruct only its own canonical physical and common state.
    activity::continuation::open(&mut peer, &imported);
    activity::continuation::restored(&mut peer, &completed.recipient, completed.scope, key(RECIPIENT_PERSONA));
    custody.assert_recipient_contents(&peer);
}
