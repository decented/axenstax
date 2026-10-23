//! Acceptance at the real native owner, command, menu and hosted authority cuts.
#[path = "quest_day_authority_fixture.rs"]
mod fixture;
use super::*;
use crate::hosted_server::HostWorld;
use crate::transport::ClientTransport as _;
use fixture::*;

#[test]
#[ignore = "needs a GPU world, signed Lent actor and observed real owner days"]
fn game_harness_d2c3_lent_reputation_uses_one_actual_native_day_for_each_current_holder() {
    let mut rig = DayRig::new("d2c3-day-lent-authority");
    rig.start_host(HostWorld::Lent);
    rig.connect();
    rig.kill_native();
    rig.kill_remote();
    rig.quiesce();
    assert_eq!(score(&rig.native()), -25);
    assert_eq!(score(&rig.remote()), -25);
    assert_eq!(actor_score(rig.actor.as_ref().unwrap()), -25);
    let native_context = rig.native().context;
    let remote_context = rig.remote().context;
    assert_ne!(native_context, remote_context);
    let owner = rig.game.state.reputation_day.owner();
    let own_server = &rig.game.state.hosted_server.as_ref().unwrap().server;
    assert_ne!(own_server.reputation_day.owner(), owner);
    assert_eq!(own_server.reputation_day.remainder(), 0, "Lent has no second natural clock");

    // Align two independently observed genuine boundaries. n default advances,
    // then n at speed7 and the rest at4 reach 24000 units at logical tick6000.
    // Both boundaries are reached through the normal complete fixed-tick path.
    let n = rig.game.state.tick_counter;
    assert_eq!(rig.units, n);
    assert!(n < 100);
    rig.command("/time speed 7");
    rig.ticks(n as u32);
    rig.command("/time speed 4");
    rig.to_units(23_996);
    assert_eq!(rig.game.state.tick_counter, 5999);
    assert_eq!(score(&rig.native()), -25);
    assert_eq!(score(&rig.remote()), -25);
    assert_eq!(rig.native().quests.penalty_left, 0);
    let first_offer = rig.step().expect("actual first Lent day offer");
    assert_eq!(first_offer.pulse.days, 1);
    assert_eq!(rig.units, 24_000);
    trace(&rig, "first-lent-day");
    assert_eq!(score(&rig.native()), -24, "OWN: native consumes its real day once");
    assert_eq!(score(&rig.remote()), -24, "OWN: signed remote consumes the SAME owner day");
    assert_eq!(actor_score(rig.actor.as_ref().unwrap()), -24);
    rig.quiesce();

    // The second event uses a bounded actual variable-speed tail. Damage only
    // the metadata at its genuine autosave boundary; restore the exact bytes.
    // At12000 ticks, an additional accumulated period can be aligned naturally.
    rig.command("/time speed 4");
    while rig.game.state.tick_counter < 11_999 {
        rig.step();
    }
    assert_eq!(rig.units, 47_996);
    let directory = crate::save::world_dir(&rig.game.state.world_name);
    let meta_path = directory.join("world_meta.json");
    let good_meta = std::fs::read(&meta_path).unwrap();
    std::fs::write(&meta_path, b"{ torn").unwrap();
    let before_native = rig.native();
    let before_remote = rig.remote();
    let offer = rig.step().expect("actual same-run offer exists at the natural advance");
    assert_eq!(rig.game.state.tick_counter, 12_000);
    assert_eq!(offer.pulse.days, 1);
    assert_eq!(offer.pulse.owner, owner);
    assert!(rig.game.state.save_live_world().is_err(), "the actual save writer refuses damaged metadata");
    assert_eq!(std::fs::read(&meta_path).unwrap(), b"{ torn");
    assert_eq!(rig.units, 48_000);
    trace(&rig, "faulted-second-lent-day");
    assert_eq!(score(&rig.native()), -23, "OWN: actual autosave refusal cannot lose the native day");
    assert_eq!(score(&rig.remote()), -23, "OWN: the surviving current offer reaches the signed holder once");
    assert_eq!(rig.native().revision, before_native.revision + 1);
    assert_eq!(rig.remote().revision, before_remote.revision + 1);
    std::fs::write(&meta_path, good_meta).unwrap();

    // This is the captured actual offer, through a real same-tick lend window.
    let applied = rig.actor.as_ref().unwrap().applied;
    rig.repeat_window(Some(offer));
    assert_eq!(score(&rig.native()), -23);
    assert_eq!(score(&rig.remote()), -23);
    assert_eq!(rig.actor.as_ref().unwrap().applied, applied);
    rig.quiesce();
    let revision = rig.remote().revision;
    rig.repeat_window(Some(offer)); // same observed old pulse, current actual tick
    assert_eq!(rig.remote().revision, revision);
    assert_eq!(score(&rig.remote()), -23);

    // A genuine authenticated handover link freezes the actual advance gates.
    rig.quiesce();
    let i = rig.actor.as_ref().unwrap().i();
    let host = rig.game.state.hosted_server.as_mut().unwrap();
    let (generation, key) = host.handover_seat(i).unwrap();
    let lease = host.handover_bind(i, generation, key, [103; 16]).unwrap();
    host.handover_freeze(Some(&directory)).unwrap();
    assert!(rig.game.state.handover_holds_play());
    let frozen = (
        rig.game.state.tick_counter,
        rig.game.state.world_time,
        rig.game.state.world.total_ticks,
        rig.game.state.reputation_day.remainder(),
        rig.native(),
        rig.remote(),
    );
    rig.game.hosted_ticks(3);
    assert_eq!(
        (
            rig.game.state.tick_counter,
            rig.game.state.world_time,
            rig.game.state.world.total_ticks,
            rig.game.state.reputation_day.remainder(),
            rig.native(),
            rig.remote()
        ),
        frozen
    );
    assert!(rig.game.state.reputation_day_offer.is_none());
    rig.game.state.hosted_server.as_mut().unwrap().handover_release();
    drop(lease);
    let units = rig.units;
    rig.step();
    assert_eq!(rig.units, units + 4, "resume has exactly its actual natural increment");
    rig.quiesce();

    // Real departure and replacement create a new run after the completed event.
    rig.actor
        .as_ref()
        .unwrap()
        .client
        .send_to_server(&crate::protocol::serialize_packet(crate::protocol::PacketType::Disconnect, &()));
    rig.actor = None;
    rig.step();
    let run = rig.game.state.hosted_server.as_ref().unwrap().server.run_id;
    rig.game.state.hosted_server = None;
    let retained = rig.game.state.reputation_day.remainder();
    rig.start_host(HostWorld::Lent);
    assert_ne!(rig.game.state.hosted_server.as_ref().unwrap().server.run_id, run);
    assert_eq!(rig.game.state.reputation_day.owner(), owner);
    assert_eq!(rig.game.state.reputation_day.remainder(), (retained + 4) % 24_000);
    rig.connect();
    assert_eq!(score(&rig.remote()), -23, "actual signed record restored by the new run");
    let revision = rig.remote().revision;
    rig.repeat_window(Some(offer));
    rig.repeat_window(None);
    assert_eq!(rig.remote().revision, revision, "OWN: new hosting cannot adopt a completed old run's event");
    assert_eq!(score(&rig.remote()), -23);
    assert_eq!(rig.native().context, native_context);
    assert_eq!(rig.remote().context, remote_context);
}

#[test]
#[ignore = "needs native command dispatch, real bed use and observed solar units"]
fn game_harness_d2c3_native_time_commands_and_sleep_do_not_invent_reputation_days() {
    let mut rig = DayRig::new("d2c3-day-native-commands");
    rig.kill_native();
    let initial = rig.native();
    let owner = rig.game.state.reputation_day.owner();
    rig.command("/time speed 4");
    rig.ticks(80);
    assert_eq!(score(&rig.native()), -25);
    for (line, phase) in [("/time set midnight", 0), ("/time set noon", 12_000), ("/time set night", 19_000)] {
        let before = rig.native();
        rig.command(line);
        assert_eq!(rig.game.state.world_time, phase);
        assert_eq!(rig.native(), before, "actual /time set only relocates phase");
    }

    let feet = clear_pad(&mut rig.game, None);
    let bed = [feet[0], feet[1], feet[2] - 2];
    rig.game.state.world.set_block(bed[0], bed[1], bed[2], crate::block::BED);
    aim_at(&mut rig.game, glam::Vec3::new(bed[0] as f32 + 0.5, bed[1] as f32 + 0.3, bed[2] as f32 + 0.5));
    let before = (rig.game.state.tick_counter, rig.game.state.world.total_ticks, rig.native());
    rig.game.state.input.cursor_captured = true;
    rig.game.state.input.right_held = true;
    rig.game.frames(1);
    rig.game.state.input.right_held = false;
    let delta = rig.game.state.tick_counter - before.0;
    rig.units += delta * 4;
    assert_eq!(rig.game.state.world.total_ticks - before.1, delta);
    assert_eq!(
        rig.game.state.players[0].spawn_pos,
        crate::item_actions::bed_spawn(bed),
        "prerequisite: the real bed-click arm accepted sleep"
    );
    assert_eq!(rig.game.state.world_time, 7000);
    assert_eq!(u64::from(rig.game.state.reputation_day.remainder()), rig.units % 24_000);
    assert_eq!(score(&rig.native()), -25, "OWN: night relocation is no elapsed day");
    assert_eq!(rig.native().revision, before.2.revision);

    for (lock, effective) in [("day", 12_000), ("night", 0)] {
        rig.game.state.world.time_lock = lock.into();
        let units = rig.units;
        rig.ticks(32);
        assert_eq!(rig.units, units + 128);
        assert_eq!(rig.game.state.world.effective_world_time(rig.game.state.world_time), effective);
        assert_eq!(score(&rig.native()), -25, "locked lighting leaves genuine raw advances intact");
    }
    rig.game.state.world.time_lock = "cycle".into();
    fast_to(&mut rig, 23_999);
    assert_eq!(score(&rig.native()), -25);
    let before = rig.native();
    rig.step();
    trace(&rig, "commands-first-day");
    assert_eq!(score(&rig.native()), -24, "OWN: only observed natural units complete the day");
    assert_eq!(rig.native().revision, before.revision + 1);
    rig.command("/time speed 4");
    rig.ticks(100);
    rig.command("/time speed 7");
    rig.ticks(73);
    fast_to(&mut rig, 47_999);
    assert_eq!(score(&rig.native()), -24);
    let before = rig.native();
    rig.step();
    assert_eq!(score(&rig.native()), -23, "OWN: variable speed contributes exactly its genuine units");
    assert_eq!(rig.native().revision, before.revision + 1);
    assert_eq!(rig.native().context, initial.context);
    assert_eq!(rig.game.state.reputation_day.owner(), owner);
    private_body_refusal_day(&mut rig);
}

#[test]
#[ignore = "needs actual save refusal, successful leave and queued native menu LoadWorld"]
fn game_harness_d2c3_native_real_menu_reopen_resets_only_the_partial_day() {
    let mut rig = DayRig::new("d2c3-day-native-menu");
    rig.start_host(HostWorld::Lent);
    rig.kill_native();
    fast_to(&mut rig, 23_999);
    let old_offer = rig.step().expect("actual old native owner's day offer");
    let old_clock = rig.observed_clock(Some(old_offer));
    assert_eq!(score(&rig.native()), -24);
    rig.game.state.hosted_server = None;
    while rig.native().quests.penalty_left > 0 {
        rig.step();
    }
    rig.kill_native();
    rig.command("/time speed 64");
    rig.ticks(50);
    let directory = crate::save::world_dir(&rig.game.state.world_name);
    let owner = rig.game.state.reputation_day.owner();
    let remainder = rig.game.state.reputation_day.remainder();
    assert!(remainder > 3000 && remainder < 23_000);
    let before = rig.native();
    assert!(before.quests.penalty_left > 0 && before.quests.penalty_left < 600);
    let meta_path = directory.join("world_meta.json");
    let good_meta = std::fs::read(&meta_path).unwrap();
    std::fs::write(&meta_path, b"{ torn").unwrap();
    assert!(!rig.game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    assert!(matches!(rig.game.state.mode, GameMode::Playing));
    assert_eq!(rig.game.state.reputation_day.owner(), owner);
    assert_eq!(
        rig.game.state.reputation_day.remainder(),
        remainder,
        "OWN: an actual failed leave retains the partial period"
    );
    assert_eq!(rig.native(), before);
    std::fs::write(&meta_path, good_meta).unwrap();
    rig.step();
    assert_eq!(rig.game.state.reputation_day.remainder(), remainder + 64);
    let saved_record = rig.native();
    let saved_total = rig.game.state.world.total_ticks;
    let name = rig.game.state.world_name.clone();
    assert!(rig.game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    assert!(matches!(rig.game.state.mode, GameMode::Menu(_)));
    let (_, selected) = super::quest_preparation::selected_record(&directory);
    assert_eq!(selected, saved_record, "actual save retained canonical score and honest timers");
    assert_eq!(crate::save::load_world_meta(&name).total_ticks, saved_total);
    rig.game.state.pending_menu_action = Some((
        crate::menu::MenuAction::LoadWorld(name, crate::spawn_pref::SpawnPref::default()),
        web_time::Instant::now(),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !matches!(rig.game.state.mode, GameMode::Playing) {
        rig.game.frames(1);
        assert!(std::time::Instant::now() < deadline, "genuine menu entry never reopened the saved world");
    }
    clear_pad(&mut rig.game, None);
    let s = &rig.game.state;
    assert_ne!(s.reputation_day.owner(), owner, "OWN: actual menu reset creates a fresh runtime owner");
    assert!(s.pending_menu_action.is_none());
    assert_eq!(s.world_time_step, 1);
    assert!(s.tick_counter < 100);
    assert_eq!(s.world.total_ticks, saved_total + s.tick_counter, "actual metadata ActiveTicks restored");
    assert_eq!(s.world_time, 7500 + s.tick_counter as u32);
    rig.units = u64::from(s.reputation_day.remainder());
    assert_eq!(rig.units, s.tick_counter, "OWN: saved phase and partial units were not reconstructed");
    assert_eq!(score(&rig.native()), score(&saved_record));
    assert_eq!(rig.native().context, saved_record.context);
    assert_eq!(
        rig.native().quests.penalty_left,
        saved_record.quests.penalty_left.saturating_sub(s.tick_counter as u32)
    );
    fast_to(&mut rig, 23_999);
    assert_eq!(score(&rig.native()), score(&saved_record), "OWN: even the old partial-period threshold gives no event");
    rig.step();
    assert_eq!(score(&rig.native()), score(&saved_record) + 1);
    rig.start_host(HostWorld::Lent);
    rig.connect();
    rig.kill_remote();
    let i = rig.actor.as_ref().unwrap().i();
    let before =
        rig.game.state.hosted_server.as_ref().unwrap().server.players[i].personal.as_ref().unwrap().record().unwrap();
    rig.repeat_clock(old_clock);
    let after =
        rig.game.state.hosted_server.as_ref().unwrap().server.players[i].personal.as_ref().unwrap().record().unwrap();
    assert_eq!(after, before, "OWN: a genuine old owner's offer cannot mutate the current canonical holder");
    trace(&rig, "actual-menu-reopen-day");
}

#[test]
#[ignore = "needs actual no-lend native/server fixed ticks through the owning server day"]
fn game_harness_d2c3_no_lend_reputation_counts_its_actual_owning_server_steps() {
    let mut rig = DayRig::new("d2c3-day-no-lend-authority");
    rig.start_host(HostWorld::Owned);
    rig.connect();
    rig.kill_remote();
    rig.quiesce();
    let context = rig.remote().context;
    let native_owner = rig.game.state.reputation_day.owner();
    let server_owner = rig.game.state.hosted_server.as_ref().unwrap().server.reputation_day.owner();
    assert_ne!(native_owner, server_owner);
    while rig.game.state.hosted_server.as_ref().unwrap().server.tick_counter < 5999 {
        rig.step();
    }
    assert_eq!(score(&rig.remote()), -25);
    rig.step();
    assert_eq!(score(&rig.remote()), -25, "OWN: actual no-lend6000 is only a quarter day");
    // This deliberately qualifies the D4 limitation: native acceleration does
    // not accelerate the separate server's actual owning+1 reputation clock.
    rig.command("/time speed 64");
    let native_units = rig.units;
    rig.ticks(375);
    assert_eq!(rig.units, native_units + 24_000);
    assert_eq!(score(&rig.remote()), -25, "OWN: native accelerated day is no owning-server day");
    while rig.game.state.hosted_server.as_ref().unwrap().server.tick_counter < 23_999 {
        rig.step();
    }
    assert_eq!(score(&rig.remote()), -25);
    let before = rig.remote();
    rig.step();
    trace(&rig, "actual-no-lend-owning-day");
    assert_eq!(rig.game.state.hosted_server.as_ref().unwrap().server.reputation_day.remainder(), 0);
    assert_eq!(score(&rig.remote()), -24, "OWN: 24000 actual owning+1 advances decay once");
    assert_eq!(rig.remote().revision, before.revision + 1);
    rig.quiesce();
    assert_eq!(actor_score(rig.actor.as_ref().unwrap()), -24);
    assert_eq!(rig.remote().context, context);
}
