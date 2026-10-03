//! Design §5 — player-avatar broadcast integration coverage.
//!
//! Two TestHost-driven tests that protect the full server-tick → broadcast
//! resolution path for animated remote avatars, in particular:
//!   - `broadcast_held_ref`'s server-simulated (remote) branch trusting the
//!     live client-sent held ref over the stale save-time inventory, and the
//!     local (position-trusted) branch resolving from the live inventory;
//!   - the SWINGING flag flowing from `last_acting` through a real tick.
//!
//! Note on the harness surface: `TestHost` wraps `GameServer` directly for
//! synchronous, deterministic ticking. The per-tick `Vec<PlayerState>` is
//! collected only inside `HostedServer::broadcast_state`, which is self-clocked
//! and not driveable from this synchronous harness. The per-player resolution
//! it performs is, however, fully encapsulated in the pure functions
//! `server::broadcast_held_ref` + `server::player_anim_fields` (see
//! hosted_server.rs:619/635). These tests therefore set the relevant
//! `ServerPlayer` fields on a live `TestHost`, run a *real* `GameServer::tick()`
//! (which runs server-player physics for remote players and leaves the
//! client-relayed held/acting fields intact), then compose the exact same
//! resolution `broadcast_state` uses to build a broadcast `PlayerState`.

use crate::protocol::{item_kind, player_flags, ItemRef, PlayerState};
use crate::test_harness::{TestConfig, TestHost};

/// Resolve a broadcast `PlayerState` for player `idx` from the live server
/// state. This calls the *same* function `HostedServer::broadcast_state` uses
/// (`server::collect_player_state`), so the test exercises the production
/// per-player collection logic rather than mirroring it.
fn broadcast_player_state(host: &TestHost, idx: usize) -> PlayerState {
    crate::server::collect_player_state(&host.server.players[idx], idx as u32)
}

/// Test 1 — a server-simulated player swinging a tool broadcasts the tool ref
/// and the SWINGING flag after a real tick.
#[test]
fn swinging_tool_broadcasts_held_ref_and_swing_flag() {
    let mut host = TestHost::start_with(TestConfig { num_players: 2, ..Default::default() });

    // Player 1 is a remote (server-simulated) client holding a tool (id 2)
    // and mining/placing this tick.
    {
        let sp = &mut host.server.players[1];
        sp.server_simulated = true;
        sp.held_kind = item_kind::TOOL;
        sp.held_id = 2;
        sp.last_acting = true;
    }

    // Drive a real server tick. tick() runs server-player physics for remote
    // players; it does not touch the client-relayed held_*/last_acting fields,
    // which the hosted_server input path owns — so they survive the tick.
    host.tick(1);

    let ps = broadcast_player_state(&host, 1);
    assert_eq!(ps.held_kind, item_kind::TOOL, "broadcast held_kind is TOOL");
    assert_eq!(ps.held_id, 2, "broadcast held_id is the client-sent tool id");
    assert_ne!(
        ps.flags & player_flags::SWINGING,
        0,
        "acting this tick sets the SWINGING flag in the broadcast"
    );
    // Sanity: the same resolution surfaces through the encapsulated helper.
    assert_eq!(
        crate::server::broadcast_held_ref(&host.server.players[1]),
        ItemRef::Tool(2).to_wire()
    );
}

/// Test 2 — a remote player's broadcast held item uses the live client-sent
/// ref, not the stale save-time inventory; a local (position-trusted) player
/// resolves from its live inventory instead.
#[test]
fn remote_uses_client_ref_local_uses_inventory() {
    use crate::item::ItemStack;

    let mut host = TestHost::start_with(TestConfig { num_players: 2, ..Default::default() });

    // Player 1: remote, EMPTY server-side inventory (so the inventory path
    // would yield Empty), but the client relayed a live tool ref (id 3).
    {
        let sp = &mut host.server.players[1];
        sp.server_simulated = true;
        sp.inventory = crate::inventory::Inventory::new(); // empty
        sp.hotbar_slot = 0;
        sp.held_kind = item_kind::TOOL;
        sp.held_id = 3;
    }

    host.tick(1);

    // Remote: broadcast trusts the client-sent live ref over stale inventory.
    let remote = broadcast_player_state(&host, 1);
    assert_eq!(remote.held_kind, item_kind::TOOL, "remote held_kind = client-sent TOOL");
    assert_eq!(remote.held_id, 3, "remote held_id = client-sent id 3, not stale-inventory Empty");
    assert_eq!(
        crate::server::broadcast_held_ref(&host.server.players[1]),
        ItemRef::Tool(3).to_wire(),
        "remote branch yields the client-relayed Tool(3) ref"
    );

    // Player 0: local (position-trusted) with a known block live in its
    // hotbar. The broadcast must resolve from the inventory, ignoring held_*.
    {
        let sp = &mut host.server.players[0];
        sp.server_simulated = false;
        sp.inventory.set_slot(0, Some(ItemStack::new_block(42, 5)));
        sp.hotbar_slot = 0;
        // A misleading client-sent ref that must be ignored for local players.
        sp.held_kind = item_kind::TOOL;
        sp.held_id = 99;
    }

    host.tick(1);

    let local = broadcast_player_state(&host, 0);
    assert_eq!(local.held_kind, item_kind::BLOCK, "local held_kind resolves from inventory (BLOCK)");
    assert_eq!(local.held_id, 42, "local held_id is the inventory block 42, not the client-sent 99");
    assert_eq!(
        crate::server::broadcast_held_ref(&host.server.players[0]),
        ItemRef::Block(42).to_wire(),
        "local branch resolves from the live hotbar slot"
    );
}
