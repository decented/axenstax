//! Real prepared native holders with explicitly authored numerical fault
//! fixtures. These exercise lost bound mutations, not earned quest activity.
use super::*;
use crate::progression::GameplayRecord;

fn current(game: &HeadlessGame) -> Vec<u8> {
    let slot = &game.state.players[0];
    let mut record = GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot).unwrap();
    // Real movement/health may advance during the surrounding native tick;
    // the failure contract here is the common maps/timers/revision/clock.
    record.local = None;
    record.encode().unwrap()
}
fn prepared(name: &str) -> HeadlessGame {
    let mut game = HeadlessGame::boot_into_world(name);
    clear_pad(&mut game, None);
    game.state.difficulty = "peaceful".into();
    game.state.prepare_local_gameplay().unwrap();
    assert!(game.state.players[0].gameplay_carry_complete());
    assert!(game.state.players[0].personal_binding.is_some());
    game
}
fn cow(game: &mut HeadlessGame) {
    let pos = game.state.players[0].player.pos + glam::Vec3::X;
    let entity = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Cow, pos);
    game.state.ecs.get::<&mut crate::combat::Health>(entity).unwrap().current = 1.0;
    let slot = &game.state.players[0];
    assert!(crate::combat::strike(
        &mut game.state.ecs,
        entity,
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
    assert!(game.state.ecs.get::<&crate::combat::Health>(entity).unwrap().is_dead());
    assert!(matches!(
        game.state.ecs.get::<&crate::combat::LastAttacker>(entity).unwrap().0,
        crate::combat::Attacker::Local(0)
    ));
    game.ticks(1);
    assert!(!game.state.ecs.contains(entity), "the actual native death sweep ran");
}
#[test]
#[ignore = "needs real native preparation and bound numerical failure handlers"]
fn game_harness_d2c3_native_lost_bound_credit_remains_incomplete_after_success_install_and_save() {
    let _root = crate::save::WorldsRootGuard::new("native-carry-incomplete-credit");
    let mut game = prepared("native-carry-incomplete-credit");
    let binding = game.state.players[0].personal_binding.clone();
    // An overflow fixture, never described as billions of actual kills.
    game.state.players[0].kill_counter.insert(crate::mob::MobType::Cow, u32::MAX);
    let before = current(&game);
    let clock = game.state.players[0].gameplay_clock;
    let pos = game.state.players[0].player.pos;
    assert!(
        game.state.credit_native_personal_kill(0, crate::mob::MobType::Cow, pos),
        "a failed bound operation still owns attribution and forbids legacy fallback"
    );
    assert_eq!(current(&game), before);
    assert_eq!(game.state.players[0].gameplay_clock, clock);
    assert!(!game.state.players[0].gameplay_carry_complete());
    cow(&mut game);
    assert_eq!(game.state.players[0].kill_counter[&crate::mob::MobType::Cow], u32::MAX);
    assert!(!game.state.players[0].gameplay_carry_complete());
    // Remove the numerical fault. A genuine later death can succeed, but
    // cannot establish the missing earlier credit on this SAME holder.
    game.state.players[0].kill_counter.remove(&crate::mob::MobType::Cow);
    cow(&mut game);
    assert_eq!(game.state.players[0].kill_counter[&crate::mob::MobType::Cow], 1);
    assert_eq!(game.state.players[0].personal_binding, binding);
    let record = GameplayRecord::from_slot(binding.as_ref().unwrap().context(), &game.state.players[0]).unwrap();
    record.install_progression(&mut game.state.players[0]);
    assert!(!game.state.players[0].gameplay_carry_complete());
    game.state.save_live_world().unwrap();
    assert!(!game.state.players[0].gameplay_carry_complete());
    assert!(
        crate::handover::live::eligibility::local_progress(&game.state.players[0], game.state.tick_counter).is_err()
    );
}
#[test]
#[ignore = "needs real native clocks and bound expiry/decay failure handlers"]
fn game_harness_d2c3_native_failed_expiry_and_day_preserve_clock_and_sticky_completeness() {
    let _root = crate::save::WorldsRootGuard::new("native-carry-incomplete-timers");
    let mut game = prepared("native-carry-incomplete-expiry");
    // A well-formed numerical revision-exhaustion fixture, not an accepted
    // gameplay quest. No live clock is assigned or fast-forwarded.
    game.state.players[0].quest_progress.declines.push(([9; 16], 1));
    game.state.players[0].gameplay_revision = u64::MAX;
    let before = current(&game);
    let clock = game.state.players[0].gameplay_clock;
    game.ticks(1);
    assert_eq!(current(&game), before);
    assert_eq!(game.state.players[0].gameplay_clock, clock);
    assert!(!game.state.players[0].gameplay_carry_complete());
    game.state.players[0].gameplay_revision = 0;
    game.ticks(1);
    assert!(game.state.players[0].quest_progress.declines.is_empty());
    assert!(!game.state.players[0].gameplay_carry_complete());
    drop(game);
    let mut game = prepared("native-carry-incomplete-decay");
    game.state.players[0].reputation.per_village.insert((2, 3), -25);
    game.state.players[0].gameplay_revision = u64::MAX;
    let before = current(&game);
    let clock = game.state.players[0].gameplay_clock;
    assert!(game.state.decay_native_personal(0, 1));
    assert_eq!(current(&game), before);
    assert_eq!(game.state.players[0].gameplay_clock, clock);
    assert!(!game.state.players[0].gameplay_carry_complete());
    game.state.players[0].gameplay_revision = 0;
    assert!(game.state.decay_native_personal(0, 1));
    assert_eq!(game.state.players[0].reputation.score((2, 3)), -24);
    assert!(!game.state.players[0].gameplay_carry_complete());
}
