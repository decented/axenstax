//! Player-physics integration tests — `GameServer::tick_player_physics`
//! runs only for `server_simulated = true` players. This covers the speed
//! cap (Task 1d anti-cheat seed) and the position-trusted local branch.

use glam::Vec3;

use crate::block::STONE;
use crate::player_intent::PlayerIntent;
use crate::test_harness::{TestConfig, TestHost};

fn stone_floor(host: &mut TestHost, y: i32) {
    for x in -20..=20 {
        for z in -20..=20 {
            host.set_block(x, y, z, STONE);
        }
    }
}

#[test]
fn local_player_physics_skipped() {
    // Default slots are local (server_simulated=false). Without a pending
    // intent the server physics branch must not advance the player.
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host, 4);
    let start = Vec3::new(0.0, 5.0, 0.0);
    host.teleport_player(0, start);

    host.tick(20);

    // Local players skip tick_player_physics entirely. Falling under gravity
    // is a separate concern (local clients run their own physics). Position
    // must be exactly unchanged here.
    assert_eq!(host.player_pos(0), start);
}

#[test]
fn server_simulated_player_honours_speed_cap() {
    // Flip a slot to server_simulated and feed it an intent that would
    // blow through the horizontal speed cap (1.089 × 1.5 b/tick). The cap
    // must clamp the resulting displacement.
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host, 4);
    host.teleport_player(0, Vec3::new(0.0, 5.0, 0.0));
    host.server.players[0].server_simulated = true;

    // Construct an intent that asks for max-forward with sprint.
    let intent = PlayerIntent {
        move_forward: 1.0,
        move_right: 0.0,
        sprint: true,
        ..PlayerIntent::default()
    };
    host.server.players[0].queue_intent(intent);

    let before = host.player_pos(0);
    host.tick(1);
    let after = host.player_pos(0);

    // Horizontal displacement must be bounded by the cap.
    let dx = after.x - before.x;
    let dz = after.z - before.z;
    let horizontal = (dx * dx + dz * dz).sqrt();
    const MAX_HORIZONTAL_PER_TICK: f32 = 1.089 * 1.5;
    assert!(horizontal <= MAX_HORIZONTAL_PER_TICK + 1e-3,
        "speed cap breached: {horizontal} > {MAX_HORIZONTAL_PER_TICK}");
}

#[test]
fn pending_intent_consumed_after_tick() {
    let mut host = TestHost::start_with(TestConfig::default());
    stone_floor(&mut host, 4);
    host.teleport_player(0, Vec3::new(0.0, 5.0, 0.0));
    host.server.players[0].server_simulated = true;
    host.server.players[0].queue_intent(PlayerIntent::default());

    host.tick(1);
    assert!(host.server.players[0].pending_intent.is_none(),
        "tick_player_physics must consume pending_intent");
}
