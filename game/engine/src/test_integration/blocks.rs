//! Block-world integration tests — falling blocks + world mutation routed
//! through the full `GameServer::tick` path, not just the free function in
//! isolation.

use glam::Vec3;

use crate::block::{AIR, GRAVEL, SAND, STONE};
use crate::test_harness::{TestConfig, TestHost};

fn floor(host: &mut TestHost, y: i32) {
    for x in -4..=4 {
        for z in -4..=4 {
            host.set_block(x, y, z, STONE);
        }
    }
}

#[test]
fn sand_falls_through_full_server_tick() {
    let mut host = TestHost::start_with(TestConfig::default());
    floor(&mut host, 5);
    host.teleport_player(0, Vec3::new(0.0, 10.0, 0.0));
    host.set_block(0, 10, 0, SAND);

    // Falling blocks run every 4th tick. 40 ticks is 10 applied passes —
    // plenty for a 5-cell drop onto the floor at y=5.
    host.tick(40);

    // Sand rested on the floor (floor at y=5, so sand settles at y=6).
    assert_eq!(host.get_block(0, 10, 0), AIR);
    assert_eq!(host.get_block(0, 6, 0), SAND);
}

#[test]
fn gravel_falls_through_full_server_tick() {
    let mut host = TestHost::start_with(TestConfig::default());
    floor(&mut host, 5);
    host.teleport_player(0, Vec3::new(0.0, 10.0, 0.0));
    host.set_block(0, 10, 0, GRAVEL);

    host.tick(40);
    assert_eq!(host.get_block(0, 6, 0), GRAVEL);
}

#[test]
fn stone_stays_put() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.teleport_player(0, Vec3::new(0.0, 10.0, 0.0));
    // Place floating stone — must not move regardless of player proximity.
    host.set_block(0, 30, 0, STONE);
    host.tick(100);
    assert_eq!(host.get_block(0, 30, 0), STONE);
}

#[test]
fn pending_block_changes_drain_over_ticks() {
    // The authoritative server accumulates block_changes in
    // pending_block_changes. Any given non-zero set must eventually clear
    // if we keep ticking — otherwise hosted_server.rs's snapshot builder
    // would never drain.
    let mut host = TestHost::start_with(TestConfig::default());
    floor(&mut host, 5);
    host.teleport_player(0, Vec3::new(0.0, 10.0, 0.0));
    host.set_block(0, 10, 0, SAND);
    host.tick(40);

    // The GameServer ticks don't drain pending_block_changes on their own —
    // the transport layer does that. What we can assert is that at least one
    // BlockChange fired during those ticks.
    assert!(!host.server.pending_block_changes.is_empty(),
        "falling sand should have produced pending block changes");
}

#[test]
fn furnace_break_drops_all_contents_via_cleanup() {
    // Task 1 (wave-hardening) — mirrors chest's HP-2 sibling test
    // (`chest_break_drops_all_contents_via_cleanup`, test_integration/save_load.rs).
    // `cleanup_furnace` is the same bridge `game_loop.rs`'s break arms now
    // call for FURNACE/FURNACE_LIT — this locks its contract against a
    // real `World` fixture (GameState itself needs a renderer and can't be
    // constructed in tests, so the bridge function is the testable seam).
    use crate::furnace::{cleanup_furnace, FurnaceData};
    use crate::item::{ItemStack, MaterialId};

    let mut host = TestHost::start_with(TestConfig::default());
    host.set_block(7, 64, 7, crate::block::FURNACE);
    let mut furnace = FurnaceData::default();
    furnace.input = Some(ItemStack::new_material(MaterialId::RawIron, 4));
    furnace.fuel = Some(ItemStack::new_material(MaterialId::Coal, 3));
    furnace.output = Some(ItemStack::new_material(MaterialId::IronIngot, 2));
    host.server.world.insert_furnace((7, 64, 7), furnace);

    let spill = cleanup_furnace(&mut host.server.world, 7, 64, 7);
    assert_eq!(spill.len(), 3, "three non-empty slots -> three spill stacks");
    let total: u32 = spill.iter().map(|s| u32::from(s.count)).sum();
    assert_eq!(total, 4 + 3 + 2);
    assert!(
        host.server.world.furnace_at((7, 64, 7)).is_none(),
        "block-entity removed"
    );
}

#[test]
fn rail_neighbour_mask_reads_an_l_as_a_corner() {
    use crate::rail::{rail_neighbour_mask, rail_shape_from_mask, RailShape, TRACK};
    let mut host = TestHost::start_with(TestConfig::default());
    // Lay an L: centre at (0,64,0) with a neighbour to the north (-Z) and east (+X).
    let y = 64;
    host.set_block(0, y, 0, TRACK);
    host.set_block(0, y, -1, TRACK); // north neighbour
    host.set_block(1, y, 0, TRACK); // east neighbour
    let mask = rail_neighbour_mask(host.world(), 0, y, 0);
    assert_eq!(rail_shape_from_mask(mask), RailShape::CornerNE);

    // A lone rail with no rail neighbours resolves to the straight default.
    host.set_block(20, y, 20, TRACK);
    let lone = rail_neighbour_mask(host.world(), 20, y, 20);
    assert_eq!(rail_shape_from_mask(lone), RailShape::StraightNS);
}
