//! T1-3 (2026-10-05) — the dedicated server ticks the block machines.
//!
//! Every test drives the real `GameServer::tick` through [`TestHost`]. With
//! `simulates_block_machines` ON (what `HostedServer::start_inner` sets for a
//! 0-local-player dedicated server) a furnace smelts, a crop grows, a piston
//! extends and a hopper moves an item — and each change that touches a block
//! rides `pending_block_changes`, the queue `HostedServer` drains into every
//! joiner's `StateUpdate`. With the flag OFF (a LAN host, whose CLIENT ticks
//! these) the server leaves them alone: no double tick.
//!
//! The flag's wiring itself (0 local players ⇔ on) is pinned in
//! `hosted_server.rs`'s `block_machine_flag_tests`.

use crate::block;
use crate::item::{Item, ItemStack, MaterialId};
use crate::meta::Facing;
use crate::protocol::BlockChange;
use crate::test_harness::{TestConfig, TestHost};

/// Build height for every rig — comfortably inside the world's 0..95.
const Y: i32 = 64;

fn host(machines: bool) -> TestHost {
    let mut h = TestHost::start_with(TestConfig::default());
    h.server.simulates_block_machines = machines;
    h
}

/// Did the server queue a broadcast of `block` at `cell`? `TestHost` never
/// drains `pending_block_changes`, so this sees every change since start.
fn queued(h: &TestHost, cell: (i32, i32, i32), block: block::BlockId) -> bool {
    h.server
        .pending_block_changes
        .iter()
        .any(|bc: &BlockChange| (bc.x, bc.y, bc.z) == cell && bc.new_block == block)
}

// ── Furnace ────────────────────────────────────────────────────────────────

/// One furnace pass runs every 4 server ticks (the client's 4-tick machine
/// cadence), so a full smelt needs 4 × SMELT_TICKS_PER_ITEM ticks plus slack
/// for the fuel-catch beat.
fn ticks_for_one_smelt() -> u32 {
    4 * (crate::furnace::SMELT_TICKS_PER_ITEM + 8)
}

fn load_copper_furnace(h: &mut TestHost) -> (i32, i32, i32) {
    let f = (6, Y, 6);
    h.load_furnace(
        f.0,
        f.1,
        f.2,
        ItemStack::new_material(MaterialId::Copper, 1),
        ItemStack::new_material(MaterialId::GreenLog, 1),
    );
    f
}

#[test]
fn a_machine_ticking_server_smelts_and_broadcasts_the_lit_furnace() {
    let mut h = host(true);
    let f = load_copper_furnace(&mut h);
    h.tick(ticks_for_one_smelt());
    let out = h
        .server
        .world
        .furnace_at(f)
        .and_then(|d| d.output.clone())
        .expect("the server smelted the copper");
    assert!(matches!(out.item, Item::Material(MaterialId::CopperIngot)));
    assert_eq!(out.count, 1);
    assert!(
        queued(&h, f, block::FURNACE_LIT),
        "the lit flip rides pending_block_changes so joiners see the furnace burn"
    );
}

#[test]
fn a_host_backed_server_does_not_smelt() {
    // Flag off = a LAN host: its client owns the furnace sweep (and the mirror
    // copies the result in). The server must not smelt a second time.
    let mut h = host(false);
    let f = load_copper_furnace(&mut h);
    h.tick(ticks_for_one_smelt());
    let data = h.server.world.furnace_at(f).expect("furnace entity still there");
    assert!(data.output.is_none(), "no server-side smelt without the flag");
    assert_eq!(h.get_block(f.0, f.1, f.2), block::FURNACE, "never lit");
}

// ── Crops ──────────────────────────────────────────────────────────────────

/// A stage-0 wheat on tilled soil in full sky light. Growth is deterministic:
/// it fires on `tick_counter` multiples of the per-stage interval (200) when
/// effective light ≥ 9 — no RNG.
fn plant_wheat(h: &mut TestHost) -> (i32, i32, i32) {
    let c = (0, 70, 0);
    h.set_block(c.0, c.1 - 1, c.2, block::TILLED_SOIL);
    h.set_block(c.0, c.1, c.2, block::WHEAT_STAGE_0);
    h.server.world.set_sky_light_at(c.0, c.1, c.2, 15);
    c
}

#[test]
fn a_machine_ticking_server_grows_a_crop_and_broadcasts_it() {
    let mut h = host(true);
    let c = plant_wheat(&mut h);
    h.tick(crate::growth::CROP_GROWTH_TICKS_PER_STAGE as u32);
    let now = h.get_block(c.0, c.1, c.2);
    assert_ne!(now, block::WHEAT_STAGE_0, "the wheat advanced on the growth cadence");
    assert!(queued(&h, c, now), "the new stage rides pending_block_changes");
}

#[test]
fn a_host_backed_server_does_not_grow_crops() {
    let mut h = host(false);
    let c = plant_wheat(&mut h);
    h.tick(2 * crate::growth::CROP_GROWTH_TICKS_PER_STAGE as u32);
    assert_eq!(h.get_block(c.0, c.1, c.2), block::WHEAT_STAGE_0);
}

// ── Pistons ────────────────────────────────────────────────────────────────

/// `electricity.rs`'s rig: a piston at the origin facing East, a lever on its
/// west face, a stone block in front to shove.
fn piston_rig(h: &mut TestHost) {
    h.set_block(0, Y, 0, block::PISTON);
    h.set_meta_facing(0, Y, 0, Facing::East);
    h.set_block(1, Y, 0, block::STONE);
    h.place_power_block(-1, Y, 0, block::LEVER, Facing::East);
}

#[test]
fn a_machine_ticking_server_extends_a_powered_piston() {
    let mut h = host(true);
    piston_rig(&mut h);
    h.toggle_lever(-1, Y, 0);
    // Tick 1's power_tick energises the run; the tick-4 machine pass reads it.
    h.tick(4);
    assert_eq!(h.get_block(1, Y, 0), block::PISTON_HEAD, "the arm comes out");
    assert_eq!(h.get_block(2, Y, 0), block::STONE, "the stone is shoved one cell along");
    assert!(queued(&h, (1, Y, 0), block::PISTON_HEAD), "joiners see the arm");
    assert!(queued(&h, (2, Y, 0), block::STONE), "…and the shoved block");
}

#[test]
fn a_host_backed_server_does_not_fire_pistons() {
    let mut h = host(false);
    piston_rig(&mut h);
    h.toggle_lever(-1, Y, 0);
    h.tick(8);
    assert_eq!(h.get_block(1, Y, 0), block::STONE, "the host client owns the push");
}

// ── Hoppers ────────────────────────────────────────────────────────────────

fn hopper_rig(h: &mut TestHost) {
    h.set_block(0, Y + 1, 0, block::CHEST);
    h.insert_chest_with(0, Y + 1, 0, &[ItemStack::new_material(MaterialId::Copper, 3)]);
    h.set_block(0, Y, 0, block::HOPPER);
    h.set_block(0, Y - 1, 0, block::CHEST);
    h.insert_chest_with(0, Y - 1, 0, &[]);
}

fn copper_in(h: &TestHost, x: i32, y: i32, z: i32) -> u32 {
    h.chest_at(x, y, z)
        .expect("chest")
        .slots
        .iter()
        .flatten()
        .filter(|s| matches!(s.item, Item::Material(MaterialId::Copper)))
        .map(|s| s.count as u32)
        .sum()
}

#[test]
fn a_machine_ticking_server_runs_hoppers_on_their_cadence() {
    let mut h = host(true);
    hopper_rig(&mut h);
    h.tick(crate::hopper::HOPPER_INTERVAL_TICKS as u32);
    assert_eq!(copper_in(&h, 0, Y - 1, 0), 1, "one item per hopper interval");
    assert_eq!(copper_in(&h, 0, Y + 1, 0), 2);
}

#[test]
fn a_host_backed_server_does_not_run_hoppers() {
    let mut h = host(false);
    hopper_rig(&mut h);
    h.tick(4 * crate::hopper::HOPPER_INTERVAL_TICKS as u32);
    assert_eq!(copper_in(&h, 0, Y - 1, 0), 0);
    assert_eq!(copper_in(&h, 0, Y + 1, 0), 3);
}

// ── Leaf decay ─────────────────────────────────────────────────────────────

#[test]
fn server_leaf_decay_is_applied_and_broadcast() {
    // A leaf with no log within reach, queued the way a broken log queues it.
    // Before T1-3 the server computed this decay and threw the result away:
    // the leaf vanished from the server's world but no joiner was told.
    let mut h = host(true);
    let leaf = (10, Y, 10);
    h.set_block(leaf.0, leaf.1, leaf.2, block::OAK_LEAVES);
    h.server
        .leaf_decay
        .on_log_broken(leaf.0, leaf.1 - 1, leaf.2, &h.server.world);
    // Support check + a 27..=108-pass random delay, one pass per 4 ticks.
    h.tick(4 * 120);
    assert_eq!(h.get_block(leaf.0, leaf.1, leaf.2), block::AIR, "the leaf decayed");
    assert!(queued(&h, leaf, block::AIR), "the decay rides pending_block_changes");
}
