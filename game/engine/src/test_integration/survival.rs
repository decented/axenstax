//! W2 survival basics, server side — a server-simulated (remote) player takes
//! fall damage and drowns through `GameServer::tick_player_physics`'s survival
//! pass, using the same `survival::tick_player_survival` driver the client runs
//! for local players. The pure rules are unit-tested in `survival::tests`.

use glam::Vec3;

use crate::block::{STONE, WATER};
use crate::player_intent::PlayerIntent;
use crate::survival::DamageCause;
use crate::test_harness::{TestConfig, TestHost};

/// Stone floor whose top face is y = 5.
fn stone_floor(host: &mut TestHost) {
    for x in -6..=6 {
        for z in -6..=6 {
            host.set_block(x, 4, z, STONE);
        }
    }
}

/// A server-simulated player at `pos`, not well fed (no passive regen to blur
/// the numbers) but not starving either.
fn remote_player_at(host: &mut TestHost, pos: Vec3) {
    host.teleport_player(0, pos);
    let sp = &mut host.server.players[0];
    sp.server_simulated = true;
    sp.combat.hunger = 17;
}

/// Tick `n` times, feeding an idle intent each tick (as a remote client would).
fn tick_with_idle_input(host: &mut TestHost, n: u32) {
    for _ in 0..n {
        host.server.players[0].queue_intent(PlayerIntent::default());
        host.tick(1);
    }
}

#[test]
fn remote_player_takes_fall_damage_server_side() {
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host);
    // 10 blocks above the floor top → ceil(10 − 3) = 7 HP.
    remote_player_at(&mut host, Vec3::new(0.5, 15.0, 0.5));

    tick_with_idle_input(&mut host, 60);

    let sp = &host.server.players[0];
    assert!(sp.player.on_ground, "must have landed, y={}", sp.player.pos.y);
    assert_eq!(sp.combat.health, 13.0);
    assert_eq!(sp.combat.last_damage, DamageCause::Fall);
}

#[test]
fn remote_player_short_drop_is_free() {
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host);
    remote_player_at(&mut host, Vec3::new(0.5, 8.0, 0.5)); // exactly 3 blocks

    tick_with_idle_input(&mut host, 40);

    assert_eq!(host.server.players[0].combat.health, 20.0);
}

#[test]
fn remote_player_landing_in_water_takes_no_fall_damage() {
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host);
    for x in -6..=6 {
        for z in -6..=6 {
            host.set_block(x, 5, z, WATER);
            host.set_block(x, 6, z, WATER);
        }
    }
    remote_player_at(&mut host, Vec3::new(0.5, 25.0, 0.5));

    tick_with_idle_input(&mut host, 80);

    assert_eq!(host.server.players[0].combat.health, 20.0);
}

#[test]
fn creative_remote_player_is_immune_to_falls() {
    let mut host = TestHost::start_with(TestConfig {
        play_mode: crate::play_mode::PlayMode::Creative,
        ..TestConfig::default()
    });
    stone_floor(&mut host);
    remote_player_at(&mut host, Vec3::new(0.5, 30.0, 0.5));

    tick_with_idle_input(&mut host, 80);

    assert_eq!(host.server.players[0].combat.health, 20.0);
}

#[test]
fn local_position_trusted_player_is_not_survival_simulated_server_side() {
    // Local players run their survival (fall damage, drowning) client-side
    // (game_loop). The server pass must skip them: with the head in water for
    // longer than the air supply (300 ticks + 20-tick grace, first hit on tick
    // 320) the server copy's health must still be 20. Deleting the
    // `server_simulated` guard makes this fail with health 18.
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host);
    for y in 5..=8 {
        host.set_block(0, y, 0, WATER);
    }
    host.teleport_player(0, Vec3::new(0.5, 5.0, 0.5));
    assert!(
        !host.server.players[0].server_simulated,
        "the host's own seat is position-trusted, not server-simulated"
    );

    host.tick(330);

    assert_eq!(host.server.players[0].combat.health, 20.0);
}

#[test]
fn remote_player_drowns_server_side_even_without_input() {
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host);
    for y in 5..=8 {
        host.set_block(0, y, 0, WATER);
    }
    // Standing on the floor, head (eye y ≈ 6.62) in water. No intents queued:
    // breath must still advance (its own pass, not gated on input).
    remote_player_at(&mut host, Vec3::new(0.5, 5.0, 0.5));

    host.tick(319);
    assert_eq!(host.server.players[0].combat.health, 20.0, "air lasts 15 s + 1 s grace");
    host.tick(1);
    let sp = &host.server.players[0];
    assert_eq!(sp.combat.health, 18.0, "first 2 HP drowning hit on tick 320");
    assert_eq!(sp.combat.last_damage, DamageCause::Drowning);
}

#[test]
fn a_server_copy_killed_by_a_fall_respawns() {
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host);
    // 40 blocks → 37 HP: lethal.
    remote_player_at(&mut host, Vec3::new(0.5, 45.0, 0.5));

    let mut died = false;
    for _ in 0..200 {
        tick_with_idle_input(&mut host, 1);
        if host.server.players[0].combat.dead {
            died = true;
            break;
        }
    }
    assert!(died, "a 40-block fall must kill");
    assert_eq!(host.server.players[0].combat.last_damage, DamageCause::Fall);

    // The 2 s respawn timer runs out → the server copy is alive again.
    tick_with_idle_input(&mut host, 45);
    let sp = &host.server.players[0];
    assert!(!sp.combat.dead);
    assert_eq!(sp.combat.health, sp.combat.max_health);
}
