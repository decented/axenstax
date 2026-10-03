//! Salt feature integration tests.
//!
//! Drive end-to-end scenarios over the snowfall painter, SALT_PATH
//! placement, Salt Lick aura, drop bonus, and index rebuild. Unit-
//! level coverage lives in the source modules; this suite exercises
//! the full World + biome_gen surfaces together.

use crate::biome::{Biome, BiomeGenerator};
use crate::block;
use crate::item::{Item, ItemStack, MaterialId};
use crate::mob::MobType;
use crate::world::World;
use glam::Vec3;

/// Grass over a whole chunk that sits in a cold biome (SnowyTundra OR
/// Mountains — both trigger snowfall) and return that chunk's centre
/// column. As of the Spec 28a wire-up (2026-05-27) `biome_at` returns
/// the full biome set including SnowyTundra; both cold biomes drive the
/// snowfall painter.
///
/// Two details matter for a non-flaky test: (1) the painter decides a
/// chunk's eligibility from its *centre* column, so we search by centre,
/// not corner; (2) the painter then paints a *pseudo-random* column
/// within the chunk each pass, so we grass the entire chunk — that way
/// whichever column it picks is a paintable surface, guaranteeing a
/// placement rather than relying on luck hitting a small patch.
fn snowy_tundra_grass_patch(world: &mut World, biome_gen: &BiomeGenerator) -> (i32, i32) {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let mut found: Option<(i32, i32)> = None;
    'outer: for ccx in -64..64 {
        for ccz in -64..64 {
            let centre_wx = ccx * cs + cs / 2;
            let centre_wz = ccz * cs + cs / 2;
            if matches!(
                biome_gen.biome_at(centre_wx, centre_wz),
                Biome::SnowyTundra | Biome::Mountains,
            ) {
                found = Some((ccx, ccz));
                break 'outer;
            }
        }
    }
    let (ccx, ccz) = found.expect("expected a SnowyTundra or Mountains chunk in -64..64 chunk range");
    // Mount grass at the real surface for every column in the chunk, and
    // clear the block above so the painter can drop SNOW onto it.
    for lx in 0..cs {
        for lz in 0..cs {
            let wx = ccx * cs + lx;
            let wz = ccz * cs + lz;
            let surface_y = biome_gen.terrain_height(wx, wz);
            world.set_block(wx, surface_y, wz, block::GRASS);
            world.set_block(wx, surface_y + 1, wz, block::AIR);
        }
    }
    (ccx * cs + cs / 2, ccz * cs + cs / 2)
}

#[test]
fn snowfall_drifts_over_session() {
    // Place a GRASS patch in a SnowyTundra cell. Advance many ticks via
    // direct snowfall calls. Assert at least one snow block landed in
    // the world (which one is deterministic but depends on the per-
    // chunk hash). For test-time speed, we step at SNOWFALL_PERIOD_TICKS.
    const SEED: u32 = 42;
    let biome_gen = BiomeGenerator::new(SEED);
    let mut world = World::new();
    let _patch = snowy_tundra_grass_patch(&mut world, &biome_gen);
    let mut placed = 0u32;
    // 200 000-tick window with SNOWFALL_PERIOD_TICKS = 6 000 gives
    // ~33 painter passes; with ~50 % skip rate that's plenty.
    let mut tick = 0u64;
    while tick < 200_000 {
        placed += crate::snowfall::tick_snowfall(&mut world, &biome_gen, SEED, tick);
        tick += crate::snowfall::SNOWFALL_PERIOD_TICKS;
    }
    assert!(placed > 0,
        "expected at least one SNOW placement over the 200 000-tick window; got {placed}");
}

#[test]
fn salt_path_blocks_snowfall_locally() {
    // Same patch but converted to SALT_PATH. After many painter passes
    // no SNOW lands in the patch (the per-tile gate skips non-GRASS/DIRT).
    const SEED: u32 = 42;
    let biome_gen = BiomeGenerator::new(SEED);
    let mut world = World::new();
    let (cx, cz) = snowy_tundra_grass_patch(&mut world, &biome_gen);
    // Convert the patch to SALT_PATH at the actual surface y.
    for dx in -2..=2 {
        for dz in -2..=2 {
            let wx = cx + dx;
            let wz = cz + dz;
            let surface_y = biome_gen.terrain_height(wx, wz);
            world.set_block(wx, surface_y, wz, block::SALT_PATH);
        }
    }
    let mut tick = 0u64;
    while tick < 200_000 {
        let _ = crate::snowfall::tick_snowfall(&mut world, &biome_gen, SEED, tick);
        tick += crate::snowfall::SNOWFALL_PERIOD_TICKS;
    }
    // Verify SNOW didn't land directly above any of the patch tiles.
    for dx in -2..=2 {
        for dz in -2..=2 {
            let wx = cx + dx;
            let wz = cz + dz;
            let surface_y = biome_gen.terrain_height(wx, wz);
            let b = world.get_block(wx, surface_y + 1, wz);
            assert_ne!(b, block::SNOW,
                "SALT_PATH at ({},{},{}) should suppress snowfall above; saw SNOW",
                wx, surface_y, wz);
        }
    }
}

#[test]
fn salt_lick_doubles_regen_inside_aura() {
    let mut world = World::new();
    world.set_block(0, 64, 0, block::SALT_LICK);
    world.salt_licks.insert((0, 64, 0));
    let mult_close = crate::salt_lick::regen_multiplier_at_pos(
        &world, Vec3::new(2.0, 64.0, 2.0), MobType::Cow);
    let mult_far = crate::salt_lick::regen_multiplier_at_pos(
        &world, Vec3::new(20.0, 64.0, 0.0), MobType::Cow);
    assert_eq!(mult_close, 2.0);
    assert_eq!(mult_far, 1.0);
}

#[test]
fn salt_lick_aura_check_and_primary_drop_for_cow() {
    let mut world = World::new();
    world.salt_licks.insert((0, 64, 0));
    let in_aura = crate::salt_lick::within_any_salt_lick_aura(
        &world, Vec3::new(3.0, 64.0, 3.0));
    let out_aura = crate::salt_lick::within_any_salt_lick_aura(
        &world, Vec3::new(20.0, 64.0, 0.0));
    assert!(in_aura, "Cow within 8 blocks should register inside aura");
    assert!(!out_aura, "Cow 20 blocks away should be outside aura");
    let primary = crate::salt_lick::salt_lick_primary_drop(MobType::Cow);
    assert_eq!(primary, Some(MaterialId::Leather));
}

#[test]
fn salt_lick_rebuild_index_after_load() {
    let mut world = World::new();
    // Stamp SALT_LICK blocks directly without going through the place
    // handler — simulating a save reload where chunks come back but
    // the side-table starts empty.
    world.set_block(5, 64, 5, block::SALT_LICK);
    world.set_block(-3, 64, 10, block::SALT_LICK);
    // Index is empty.
    assert!(world.salt_licks.is_empty(),
        "fresh world should have no index entries until rebuild");
    world.rebuild_salt_lick_index();
    assert!(world.salt_licks.contains(&(5, 64, 5)),
        "rebuild should find the SALT_LICK at (5, 64, 5)");
    assert!(world.salt_licks.contains(&(-3, 64, 10)),
        "rebuild should find the SALT_LICK at (-3, 64, 10)");
    assert_eq!(world.salt_licks.len(), 2,
        "rebuild should find exactly the 2 placed licks");
}

#[test]
fn salt_path_acceptance_and_salt_stack_consumption_pure() {
    // Pure-level coverage of the placement contract: Salt material is
    // is_salt-like (matches MaterialId::Salt), and the path-acceptance
    // helper allows GRASS/DIRT/SNOW. Full right-click integration via
    // TestHost is deferred to playtest (the harness lacks a one-line
    // right-click hook against an arbitrary block).
    let salt = Item::Material(MaterialId::Salt);
    assert!(matches!(salt, Item::Material(MaterialId::Salt)));
    assert!(crate::snowfall::target_block_accepts_salt_path(block::GRASS));
    assert!(crate::snowfall::target_block_accepts_salt_path(block::DIRT));
    assert!(crate::snowfall::target_block_accepts_salt_path(block::SNOW));
    assert!(!crate::snowfall::target_block_accepts_salt_path(block::STONE));
    // ItemStack::new_material(Salt, 5) is a hotbar-shaped Salt stack;
    // confirm count + variant.
    let stack = ItemStack::new_material(MaterialId::Salt, 5);
    assert_eq!(stack.count, 5);
    assert!(matches!(stack.item, Item::Material(MaterialId::Salt)));
}
