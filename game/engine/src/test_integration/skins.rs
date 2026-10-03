//! Phase 3 Task 5 — per-player skin reference broadcast coverage.
//!
//! A skin *reference* (`u64` content hash, `CosmeticDescriptor::skin_key`) is
//! carried on the multiplayer wire so a future networked/Signet-verified path
//! can match + diff each player's look without shipping bytes every tick. This
//! protects the broadcast leg of that wiring: a `ServerPlayer.skin_key` flows
//! through `server::collect_player_state` onto every per-player `PlayerState`.
//!
//! Scope note: this asserts the KEY round-trips through the broadcast. The skin
//! BYTES delivery (a reliable blob packet) is out of scope / gated — there's no
//! native skin-upload source today, so native `skin_key`s are `0` (default) in
//! practice; the value proven here is the foundation + this headless test.
//!
//! Modelled on `avatars.rs`: set the relevant `ServerPlayer` fields on a live
//! `TestHost`, run a real `GameServer::tick()`, then call the *same* function
//! the broadcast path uses (`server::collect_player_state`) to build the
//! per-player `PlayerState` and inspect it.

use crate::protocol::PlayerState;
use crate::test_harness::{TestConfig, TestHost};

/// Resolve a broadcast `PlayerState` for player `idx` from the live server
/// state, via the same function `HostedServer::broadcast_state` calls.
fn broadcast_player_state(host: &TestHost, idx: usize) -> PlayerState {
    crate::server::collect_player_state(&host.server.players[idx], idx as u32)
}

/// Two players with DIFFERENT skin keys broadcast their matching distinct keys.
#[test]
fn distinct_skin_keys_round_trip_on_broadcast() {
    let mut host = TestHost::start_with(TestConfig { num_players: 2, ..Default::default() });

    // Give each server player a distinct, non-default skin reference. These
    // stand in for the FNV-1a hashes a real uploaded skin would produce; what
    // matters for the wire is that the per-player value is carried verbatim.
    host.server.players[0].skin_key = 0xA11CE;
    host.server.players[1].skin_key = 0xB0B;

    host.tick(1);

    let p0 = broadcast_player_state(&host, 0);
    let p1 = broadcast_player_state(&host, 1);

    assert_eq!(p0.skin_key, 0xA11CE, "player 0 broadcasts its own skin key");
    assert_eq!(p1.skin_key, 0xB0B, "player 1 broadcasts its own skin key");
    assert_ne!(
        p0.skin_key, p1.skin_key,
        "distinct per-player skin keys stay distinct on the broadcast"
    );
}

/// Two players with the SAME skin key broadcast the same key (a shared default
/// or shared custom skin must hash-match across the wire).
#[test]
fn shared_skin_key_broadcasts_identically() {
    let mut host = TestHost::start_with(TestConfig { num_players: 2, ..Default::default() });

    host.server.players[0].skin_key = 0xDECAF;
    host.server.players[1].skin_key = 0xDECAF;

    host.tick(1);

    let p0 = broadcast_player_state(&host, 0);
    let p1 = broadcast_player_state(&host, 1);

    assert_eq!(p0.skin_key, 0xDECAF);
    assert_eq!(p1.skin_key, 0xDECAF);
    assert_eq!(p0.skin_key, p1.skin_key, "same skin -> same broadcast key");
}

/// The default sentinel (`0`) is the resting value — a freshly-constructed
/// `ServerPlayer` broadcasts `skin_key: 0` (== `CosmeticDescriptor::default`).
#[test]
fn default_skin_key_is_zero_sentinel() {
    let mut host = TestHost::start_with(TestConfig { num_players: 1, ..Default::default() });
    host.tick(1);
    assert_eq!(
        broadcast_player_state(&host, 0).skin_key,
        0,
        "a default ServerPlayer broadcasts the 0 sentinel (default skin)"
    );
}
