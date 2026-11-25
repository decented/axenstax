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
