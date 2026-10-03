//! Server-side play-mode wiring (Spec 05 §8). The break/place GATES live in the
//! client `game_loop.rs` path (dual-sim debt), so they're build-validated +
//! playtest-confirmed rather than asserted here. This covers that `TestHost`
//! actually propagates the mode to the server and that the cached projection
//! stays consistent.
//!
//! NOTE: do NOT call `tick()` before asserting the mode — `GameServer::tick`
//! reloads the mode from disk meta, which would clobber an in-memory set.

use crate::play_mode::PlayMode;
use crate::test_harness::{TestConfig, TestHost};

#[test]
fn start_with_propagates_play_mode_to_server() {
    let host = TestHost::start_with(TestConfig {
        play_mode: PlayMode::Adventure,
        ..Default::default()
    });
    assert_eq!(host.server.play_mode, PlayMode::Adventure);
    assert!(!host.server.is_creative); // cached projection consistent
}

#[test]
fn set_play_mode_keeps_is_creative_cache_consistent() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.set_play_mode(PlayMode::Creative);
    assert_eq!(host.server.play_mode, PlayMode::Creative);
    assert!(host.server.is_creative);

    host.set_play_mode(PlayMode::Spectator);
    assert_eq!(host.server.play_mode, PlayMode::Spectator);
    assert!(!host.server.is_creative);
}
