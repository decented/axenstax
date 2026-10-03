//! Layer B acceptance: spin up a TestHost, run 200 ticks, confirm the server
//! is still alive and state is plausible. Proof that the harness compiles and
//! the GameServer tick loop is driveable from a test.

use crate::test_harness::{TestConfig, TestHost};

#[test]
fn spins_up_and_ticks() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.tick(200);
    // World-time wraps at 24_000 and advances by 1 per tick (canonical
    // 20-min day @ 20 TPS), so after 200 ticks it must equal
    // (initial 6000 + 200) % 24_000.
    assert_eq!(host.server.world_time, 6200);
    assert_eq!(host.server.players.len(), 1);
}

#[test]
fn multi_player_config_honoured() {
    let mut host = TestHost::start_with(TestConfig { num_players: 3, ..Default::default() });
    host.tick(1);
    assert_eq!(host.server.players.len(), 3);
}
