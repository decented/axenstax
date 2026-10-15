//! Native daily-decay baseline through real preparation, death and natural ticks.
use super::*;
use crate::progression::{GameplayBinding, GameplayRecord};

fn record(game: &HeadlessGame) -> GameplayRecord {
    let slot = &game.state.players[0];
    GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot).unwrap()
}

fn score(record: &GameplayRecord, village: crate::reputation::VillageId) -> i16 {
    record
        .reputation
        .iter()
        .find(|(id, _)| *id == village)
        .map(|(_, value)| *value)
        .unwrap_or(0)
}

/// Observe the real tick entry without changing clocks, counters or authority.
fn natural_tick(game: &mut HeadlessGame, binding: &GameplayBinding, wraps: &mut u32) {
    assert!(matches!(game.state.mode, GameMode::Playing));
    assert!(!game.state.players[0].is_dead(), "the native seat is live");
    assert!(!game.state.joined());
    assert!(game.state.hosted_server.is_none());
    assert!(game.state.players[0].personal_binding.as_ref() == Some(binding));
    assert!(game.state.local_personal.as_ref() == Some(binding));
    assert_eq!(
        game.state.world_time_step, 1,
        "retain the real default speed"
    );
    let phase = game.state.world_time;
    let tick = game.state.tick_counter;
    let total = game.state.world.total_ticks;
    game.ticks(1);
    assert_eq!(game.state.tick_counter, tick + 1, "one real logical tick");
    assert_eq!(
        game.state.world.total_ticks,
        total + 1,
        "one active world tick"
    );
    assert_eq!(game.state.world_time, (phase + 1) % 24_000);
    if game.state.world_time < phase {
        *wraps += 1;
    }
    assert!(
        !game.state.players[0].is_dead(),
        "the same seat remains live"
    );
    assert!(game.state.players[0].personal_binding.as_ref() == Some(binding));
}

#[test]
#[ignore = "needs a native GPU world, real preparation and observed natural reputation days"]
fn game_harness_d2c3_native_reputation_does_not_decay_before_a_real_default_world_day() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-default-day");
    let name = "d2c3-native-default-day";
    let mut game = HeadlessGame::boot_into_world(name);
    clear_pad(&mut game, None);
    assert!(game.state.players[0].personal_binding.is_none());
    assert!(game.state.local_personal.is_none());
    assert!(game.state.world.quest_scope.is_none());
    assert!(!game.state.joined());
    assert!(game.state.hosted_server.is_none());
    assert!(matches!(game.state.mode, GameMode::Playing));
    assert!(!game.state.players[0].is_dead());

    let start_tick = game.state.tick_counter;
    let start_phase = game.state.world_time;
    let start_total = game.state.world.total_ticks;
    let default_step = game.state.world_time_step;
    assert_eq!(default_step, 1);
    assert!(start_tick < 100, "fresh native world has barely started");
    assert_eq!(start_phase, 7500 + start_tick as u32);
    assert!(
        start_phase + 6000 < 24_000,
        "the entire baseline stays far from rollover"
    );
    eprintln!(
        "native-default-day start_tick={start_tick} start_raw_phase={start_phase} default_step={default_step} start_total_ticks={start_total}"
    );

    // Let the actual durable preparation establish this fresh world's holder.
    // No identity, personal record or gameplay clock is installed by the test.
    game.state.prepare_local_gameplay().unwrap();
    let binding = game.state.players[0].personal_binding.clone().unwrap();
    assert!(matches!(binding, GameplayBinding::WorldLocal { .. }));
    assert!(game.state.local_personal.as_ref() == Some(&binding));
    assert!(game.state.gameplay_preparation.is_none());
    let (selected, initial) =
        super::quest_preparation::selected_record(&crate::save::world_dir(name));
    assert!(initial.context == binding.context());
    assert_eq!(
        selected.quest_world.as_ref().unwrap().scope,
        game.state.world.quest_scope.unwrap()
    );
    assert_eq!(game.state.tick_counter, start_tick);
    assert_eq!(game.state.world_time, start_phase);
    assert_eq!(game.state.world.total_ticks, start_total);

    let village = (14, 15);
    let pos = game.state.players[0].player.pos + glam::Vec3::X;
    game.state
        .world
        .village_anchors
        .insert(village, pos.to_array().map(|value| value.floor() as i32));
    assert_eq!(score(&initial, village), 0);
    let before_kills = game.state.players[0]
        .kill_counter
        .get(&crate::mob::MobType::Villager)
        .copied()
        .unwrap_or(0);
    let victim = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Villager, pos);
    // A valid one-HP fixture still needs real damage and real corpse attribution.
    game.state
        .ecs
        .get::<&mut crate::combat::Health>(victim)
        .unwrap()
        .current = 1.0;
    let slot = &game.state.players[0];
    let eye = slot.player.eye_pos();
    let damage = slot.inventory.hotbar_attack_damage(slot.hotbar_slot);
    assert!(damage >= 1.0);
    assert!((pos - eye).length() < crate::combat::ATTACK_REACH);
    let owner = crate::tameable::local_owner_key(0);
    assert!(crate::combat::strike(
        &mut game.state.ecs,
        victim,
        &crate::combat::Swing {
            attacker: crate::combat::Attacker::Local(0),
            owner_key: &owner,
            eye,
            look_dir: (pos - eye).normalize(),
            crit: false,
            sprinting: false,
            sneaking: false,
            base_damage: damage,
        },
    ));
    assert!(
        game.state
            .ecs
            .get::<&crate::combat::Health>(victim)
            .unwrap()
            .is_dead()
    );
    assert!(matches!(
        game.state
            .ecs
            .get::<&crate::combat::LastAttacker>(victim)
            .unwrap()
            .0,
        crate::combat::Attacker::Local(0)
    ));
    let mut wraps = 0;
    natural_tick(&mut game, &binding, &mut wraps);
    assert!(
        !game.state.ecs.contains(victim),
        "the actual native death sweep consumed the corpse"
    );
    let penalised = record(&game);
    assert_eq!(
        game.state.players[0].kill_counter[&crate::mob::MobType::Villager],
        before_kills + 1
    );
    let penalised_score = score(&penalised, village);
    assert_eq!(
        penalised_score, -25,
        "actual death established nonzero negative reputation"
    );
    assert_eq!(penalised.quests.penalty_left, 600);
    assert_eq!(
        game.state.players[0].reputation.score(village),
        penalised_score,
        "the common record projects the same native holder"
    );

    // Reach the actual global 6000 boundary only through normal gameplay ticks.
    while game.state.tick_counter < 5999 {
        natural_tick(&mut game, &binding, &mut wraps);
    }
    let before = record(&game);
    assert_eq!(
        score(&before, village),
        penalised_score,
        "the subject is still nonzero before the boundary"
    );
    assert_eq!(
        before.quests.penalty_left, 0,
        "actual gameplay expired the separate penalty cooldown"
    );
    eprintln!(
        "native-default-day before tick={} raw_phase={} natural_tick_delta={} wraps={wraps} total_ticks={} score={} revision={} penalty={} live={} bound={}",
        game.state.tick_counter,
        game.state.world_time,
        game.state.tick_counter - start_tick,
        game.state.world.total_ticks,
        score(&before, village),
        before.revision,
        before.quests.penalty_left,
        !game.state.players[0].is_dead(),
        game.state.players[0].personal_binding.as_ref() == Some(&binding),
    );
    natural_tick(&mut game, &binding, &mut wraps);
    let current = record(&game);
    let elapsed = game.state.tick_counter - start_tick;
    let natural_phase_delta =
        u64::from(wraps) * 24_000 + u64::from(game.state.world_time) - u64::from(start_phase);
    eprintln!(
        "native-default-day boundary start_raw_phase={start_phase} current_raw_phase={} default_step={default_step} start_tick={start_tick} current_tick={} natural_tick_delta={elapsed} natural_phase_delta={natural_phase_delta} wraps={wraps} start_total_ticks={start_total} total_ticks={} penalised_score={penalised_score} before_score={} current_score={} native_score={} penalised_revision={} before_revision={} current_revision={} penalised_penalty={} before_penalty={} current_penalty={} live={} bound={}",
        game.state.world_time,
        game.state.tick_counter,
        game.state.world.total_ticks,
        score(&before, village),
        score(&current, village),
        game.state.players[0].reputation.score(village),
        penalised.revision,
        before.revision,
        current.revision,
        penalised.quests.penalty_left,
        before.quests.penalty_left,
        current.quests.penalty_left,
        !game.state.players[0].is_dead(),
        game.state.players[0].personal_binding.as_ref() == Some(&binding),
    );
    assert_eq!(game.state.tick_counter, 6000);
    assert_eq!(natural_phase_delta, elapsed);
    assert_eq!(game.state.world.total_ticks - start_total, elapsed);
    assert_eq!(wraps, 0, "no natural day boundary occurred");
    assert!(
        natural_phase_delta < 24_000,
        "a full default day has not elapsed"
    );
    assert!(current.context == penalised.context);

    // FIRST own daily-cadence assertion: a 6000-tick quarter-day cannot decay.
    assert_eq!(
        score(&current, village),
        penalised_score,
        "native daily reputation must wait for an observed full 24000-tick day at the real default speed"
    );
    assert_eq!(
        game.state.players[0].reputation.score(village),
        penalised_score
    );
    assert_eq!(current.revision, before.revision);
    while game.state.tick_counter < 23_999 {
        natural_tick(&mut game, &binding, &mut wraps);
        assert_eq!(score(&record(&game), village), penalised_score);
    }
    assert_eq!(game.state.reputation_day.remainder(), 23_999);
    let before_day = record(&game);
    natural_tick(&mut game, &binding, &mut wraps);
    let day = record(&game);
    assert_eq!(game.state.tick_counter, 24_000);
    assert_eq!(game.state.reputation_day.remainder(), 0);
    assert_eq!(score(&day, village), penalised_score + 1);
    assert_eq!(day.revision, before_day.revision + 1);
    assert_eq!(day.context, penalised.context);
    assert_eq!(day.kills, penalised.kills);
    natural_tick(&mut game, &binding, &mut wraps);
    assert_eq!(record(&game).revision, day.revision);
    assert_eq!(score(&record(&game), village), penalised_score + 1);
}
