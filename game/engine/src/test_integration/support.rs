//! pt1124 — torches and lanterns drop when their support goes, through the
//! full `GameServer::tick` (Spec 02 §1 "Support"): a break, a blast, a chain.

use glam::Vec3;

use crate::block::{self, AIR, DIRT, LANTERN, STONE, TORCH};
use crate::item::{Item, ItemStack};
use crate::test_harness::{TestConfig, TestHost};

fn host() -> TestHost {
    let mut host = TestHost::start_with(TestConfig::default());
    // A floor in every column near the cells under test (the support pass
    // treats a column the world holds nothing of as unknown, and holds).
    for x in -8..=8 {
        for z in -8..=8 {
            host.set_block(x, 2, z, STONE);
        }
    }
    host.teleport_player(0, Vec3::new(0.5, 3.0, 6.5));
    host.tick(8);
    host.server.pending_block_changes.clear();
    host
}

/// Items of `id` lying in the server's ECS.
fn dropped(host: &TestHost, id: block::BlockId) -> u32 {
    host.ecs()
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, ie)| ie.stack.item == Item::Block(id))
        .map(|(_, ie)| u32::from(ie.stack.count))
        .sum()
}

fn told_air(host: &TestHost, cell: (i32, i32, i32)) -> bool {
    host.server
        .pending_block_changes
        .iter()
        .any(|bc| (bc.x, bc.y, bc.z) == cell && bc.new_block == AIR)
}

#[test]
fn breaking_the_block_under_a_torch_drops_the_torch_and_its_light() {
    let mut host = host();
    host.set_block(0, 9, 0, STONE);
    host.set_block(0, 10, 0, TORCH);
    crate::lighting::update_for_block_change(&mut host.server.world, (0, 10, 0), AIR, TORCH, &host.server.registry);
    host.tick(8);
    assert_eq!(host.get_block(0, 10, 0), TORCH, "a torch on a block stays");
    assert!(host.world().block_light_at(0, 11, 0) > 0);

    host.mine_block(0, 9, 0);
    host.tick(8);
    assert_eq!(host.get_block(0, 10, 0), AIR, "the torch went with its block");
    assert_eq!(dropped(&host, TORCH), 1, "as a torch item");
    assert!(told_air(&host, (0, 10, 0)), "every client is told");
    assert_eq!(host.world().block_light_at(0, 11, 0), 0, "and its light went with it");
}

#[test]
fn a_hanging_chain_of_lanterns_falls_when_its_block_is_broken() {
    let mut host = host();
    let hanging = crate::lantern::placement_meta([0, -1, 0]);
    host.set_block(3, 12, 0, STONE);
    for y in [11, 10] {
        host.set_block(3, y, 0, LANTERN);
        host.server.world.set_meta((3, y, 0), hanging);
    }
    // A standing lantern on a block beside them.
    host.set_block(-3, 9, 0, DIRT);
    host.set_block(-3, 10, 0, LANTERN);
    host.tick(8);
    assert_eq!((host.get_block(3, 11, 0), host.get_block(3, 10, 0), host.get_block(-3, 10, 0)), (LANTERN, LANTERN, LANTERN));

    host.mine_block(3, 12, 0);
    host.mine_block(-3, 9, 0);
    host.tick(16);
    assert_eq!(host.get_block(3, 11, 0), AIR);
    assert_eq!(host.get_block(3, 10, 0), AIR, "the lantern under a lantern follows it down");
    assert_eq!(host.get_block(-3, 10, 0), AIR, "the standing lantern lost its floor");
    assert_eq!(dropped(&host, LANTERN), 3);
}

#[test]
fn a_blast_that_takes_a_torchs_wall_drops_the_torch_outside_the_blast() {
    let mut host = host();
    let centre = (0, 20, 0);
    let r = crate::explosion::BLAST_RADIUS as i32;
    // The wall is the blast's last cell along +x; the torch on it is one past.
    host.set_block(r, 20, 0, DIRT);
    host.set_block(r + 1, 20, 0, TORCH);
    host.tick(8);
    assert_eq!(host.get_block(r + 1, 20, 0), TORCH);

    let s = &mut host.server;
    let blast = crate::explosion::detonate_keg_core(&mut s.world, &mut s.ecs, &mut s.water, &mut s.lava, &s.registry, centre);
    assert!(blast.destroyed.iter().any(|&(p, _)| p == (r, 20, 0)), "the wall is blasted");
    assert_eq!(host.get_block(r + 1, 20, 0), TORCH, "the blast itself does not reach the torch");
    host.tick(8);
    assert_eq!(host.get_block(r + 1, 20, 0), AIR, "the torch lost its wall");
    assert_eq!(dropped(&host, TORCH), 1);
}

#[test]
fn a_floating_torch_in_a_loaded_world_stays_until_something_beside_it_changes() {
    let mut host = host();
    host.set_block(0, 10, 0, TORCH);
    host.server.world.discard_support_log();
    host.tick(8);
    assert_eq!(host.get_block(0, 10, 0), TORCH, "nothing changed next to it");
    host.set_block(5, 10, 5, STONE);
    host.tick(8);
    assert_eq!(host.get_block(0, 10, 0), TORCH, "a change elsewhere does not reach it");
    host.set_block(1, 10, 0, STONE);
    host.set_block(1, 10, 0, AIR);
    host.tick(8);
    assert_eq!(host.get_block(0, 10, 0), AIR, "a change beside it does");
    assert_eq!(dropped(&host, TORCH), 1);
    assert_eq!(host.server.registry.mine_drop(TORCH), ItemStack::new_block(TORCH, 1));
}

/// Lantern items lying in a server's ECS.
fn lantern_items(s: &crate::server::GameServer) -> u32 {
    s.ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, ie)| ie.stack.item == Item::Block(LANTERN))
        .map(|(_, ie)| u32::from(ie.stack.count))
        .sum()
}

/// pt1124 review 3 items 6 and 7 — a save between two passes of a lantern
/// chain's collapse neither strands it nor eats its items: the save keeps the
/// links still hanging (no settlement), the load marks their section for the
/// support rescan, and the ticks after it drop each remaining link once. Three
/// links, three lantern items in all (the one that fell before the save, the
/// two after the load; items on the ground are not saved).
#[test]
fn a_save_mid_collapse_keeps_the_hanging_links_and_they_fall_after_the_load() {
    let _g = crate::save::WorldsRootGuard::new("support-resume");
    let name = "support-resume".to_string();
    let mut s = crate::server::GameServer::new(1, name.clone(), 31);
    for x in -4..=4 {
        for z in -4..=4 {
            s.world.set_block(x, 150, z, STONE);
        }
    }
    let hanging = crate::lantern::placement_meta([0, -1, 0]);
    s.world.set_block(0, 200, 0, STONE);
    s.world.set_block(2, 198, 0, STONE); // a control block in the chain's chunk
    for y in 197..=199 {
        s.world.set_block(0, y, 0, LANTERN);
        s.world.set_meta((0, y, 0), hanging);
        s.world.set_placed(0, y, 0, true); // a player hung them
    }
    s.world.discard_support_log();
    s.world.set_block(0, 200, 0, AIR);
    let first = crate::support::tick_support(&mut s.world, &s.registry);
    crate::support::spawn_drops(&mut s.ecs, &first);
    assert_eq!(first.len(), 1, "one link a pass");
    assert!(s.world.support_pending(), "the rest waits on the runtime log");
    let before_save = lantern_items(&s);
    assert_eq!(before_save, 1);
    s.try_save().unwrap();
    assert_eq!(lantern_items(&s), before_save, "the save drops nothing (no settlement)");
    for y in 197..=198 {
        assert_eq!(s.world.get_block(0, y, 0), LANTERN, "still hanging at {y} when saved");
    }
    drop(s);

    let mut loaded = crate::server::GameServer::new(1, name, 31);
    loaded.initial_load().unwrap();
    assert_eq!(loaded.world.get_block(2, 198, 0), STONE, "the chain's chunk loaded");
    for y in 197..=198 {
        assert_eq!(loaded.world.get_block(0, y, 0), LANTERN, "the save kept the link at {y}");
    }
    assert_eq!(loaded.world.support_rechecks_len(), 2, "the load queued the two placed links for a check");
    for _ in 0..40 {
        loaded.tick();
    }
    for y in 197..=199 {
        assert_eq!(loaded.world.get_block(0, y, 0), AIR, "no link left floating at {y}");
    }
    let after_load = lantern_items(&loaded);
    assert_eq!(after_load, 2, "each remaining link fell once after the load");
    assert_eq!(before_save + after_load, 3, "three links, three lantern items in all");
    assert!(!loaded.world.support_pending());
    assert_eq!(loaded.world.get_block(2, 198, 0), STONE, "the control block stands");
}

/// A load checks only player-placed torches and lanterns, and a held one
/// stays: a placed floor torch whose floor is in the section below, a placed
/// wall torch whose wall is across a column border, and a world-gen torch (no
/// placed bit) hanging in mid-air, as some bundled village plans hang one, all
/// stand after a save, a reload and 40 ticks.
#[test]
fn a_load_drops_no_held_torch_and_no_world_gen_torch() {
    let _g = crate::save::WorldsRootGuard::new("support-held");
    let name = "support-held".to_string();
    let mut s = crate::server::GameServer::new(1, name.clone(), 31);
    for x in -4..=20 {
        for z in -4..=4 {
            s.world.set_block(x, 150, z, STONE);
        }
    }
    // A floor torch at y 176 (a section's bottom row) on a floor at y 175.
    s.world.set_block(0, 175, 0, STONE);
    s.world.set_block(0, 176, 0, TORCH);
    s.world.set_meta((0, 176, 0), crate::support::torch_meta([0, 1, 0]).unwrap());
    s.world.set_placed(0, 176, 0, true);
    // A wall torch at x 15 on a wall at x 16, the next column.
    s.world.set_block(16, 170, 0, STONE);
    s.world.set_block(15, 170, 0, TORCH);
    s.world.set_meta((15, 170, 0), crate::support::torch_meta([-1, 0, 0]).unwrap());
    s.world.set_placed(15, 170, 0, true);
    // A world-gen torch in mid-air, as a town hall's.
    s.world.set_block(3, 180, 0, TORCH);
    s.world.discard_support_log();
    s.try_save().unwrap();
    drop(s);

    let mut loaded = crate::server::GameServer::new(1, name, 31);
    loaded.initial_load().unwrap();
    assert_eq!(loaded.world.support_rechecks_len(), 2, "the two placed torches are queued, the world-gen one is not");
    for _ in 0..40 {
        loaded.tick();
    }
    assert_eq!(loaded.world.get_block(0, 176, 0), TORCH, "the floor torch over a section boundary holds");
    assert_eq!(loaded.world.get_block(15, 170, 0), TORCH, "the wall torch across a column border holds");
    assert_eq!(loaded.world.get_block(3, 180, 0), TORCH, "a world-gen torch is never judged by the load");
    assert_eq!(dropped_items(&loaded), 0);
}

/// Items of any kind lying in a server's ECS.
fn dropped_items(s: &crate::server::GameServer) -> usize {
    s.ecs.query::<&crate::entity::ItemEntity>().iter().count()
}

/// Only a player-placed torch or lantern in a section a load brings in is
/// queued: inserting a section (world-gen, a pushed chunk) queues nothing.
#[test]
fn only_loaded_placed_torches_and_lanterns_are_queued() {
    let mut w = crate::world::World::new();
    let mut torch = crate::chunk::Chunk::new();
    torch.set(1, 1, 1, TORCH);
    torch.set_placed(1, 1, 1, true);
    torch.set(2, 1, 1, LANTERN); // world-gen's: no placed bit
    let mut plain = crate::chunk::Chunk::new();
    plain.set(1, 1, 1, STONE);
    plain.set_placed(1, 1, 1, true);
    w.insert_chunk(0, 4, 0, torch);
    w.insert_chunk(1, 4, 0, plain);
    assert!(!w.support_pending(), "inserting (world-gen, a push) queues nothing");
    w.note_loaded_for_support((1, 4, 0));
    assert!(!w.support_pending(), "a loaded section with no torch or lantern queues nothing");
    w.note_loaded_for_support((0, 4, 0));
    assert_eq!(w.support_rechecks_len(), 1, "the placed torch is queued, the unplaced lantern is not");
    assert_eq!(w.take_support_rechecks(8), vec![(1, 65, 1)]);
}
