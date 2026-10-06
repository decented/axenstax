//! Mob simulation integration tests — spawn cycle and AI target selection via
//! the GameServer the TestHost wraps.

use glam::Vec3;

use crate::block::STONE;
use crate::entity;
use crate::mob::MobType;
use crate::mob_ai::MobAi;
use crate::test_harness::{TestConfig, TestHost};

fn flat_floor(host: &mut TestHost, y: i32) {
    for x in -32..=32 {
        for z in -32..=32 {
            host.set_block(x, y, z, STONE);
        }
    }
}

#[test]
fn mob_cap_holds_across_many_tick_cycles() {
    let mut host = TestHost::start_with(TestConfig::default());
    flat_floor(&mut host, 10);
    host.teleport_player(0, Vec3::new(0.0, 12.0, 0.0));

    // Pre-populate at cap. GameServer::tick runs the spawner every 400 ticks
    // only when world_time%400==0; run a few thousand ticks to cross several
    // cycles and confirm the cap isn't exceeded.
    // All on the floor: a hostile in a column with no blocks is frozen and
    // does not count toward the cap (Phase B1 review), so the old row out to
    // x = 79 left 32 of them outside it.
    for i in 0..80 {
        entity::spawn_mob(&mut host.server.ecs, MobType::Brigand,
            Vec3::new((i % 40) as f32 - 20.0, 12.0, (i / 40) as f32 * 4.0));
    }
    assert_eq!(host.ecs().query::<&crate::entity::MobKind>().iter().count(), 80);

    host.tick(2000);

    // The cap counts hostiles in present columns (Phase B1 review): a brigand
    // that walks off this hand-built floor (most end up just past its z = -32
    // edge) into a column with no blocks is frozen there and no longer counts.
    let world = &host.server.world;
    let (mut counted, mut frozen) = (0, 0);
    for (_, (_, pos)) in host.ecs().query::<(&crate::entity::MobKind, &entity::Position)>().iter() {
        if world.is_column_present_at(pos.0.x.floor() as i32, pos.0.z.floor() as i32) {
            counted += 1;
        } else {
            frozen += 1;
        }
    }
    assert!(counted <= 80, "soft-cap breached: {counted} > 80 ({frozen} frozen off the floor)");
}

#[test]
fn mob_ai_targets_nearer_of_two_players() {
    let mut host = TestHost::start_with(TestConfig { num_players: 2, ..Default::default() });
    flat_floor(&mut host, 4);
    host.teleport_player(0, Vec3::new(0.0, 5.0, 0.0));
    host.teleport_player(1, Vec3::new(30.0, 5.0, 0.0));

    // Spawn a mob right next to player 0 — well inside DETECT_RANGE (16).
    entity::spawn_mob(&mut host.server.ecs, MobType::Brigand, Vec3::new(3.0, 5.0, 0.0));
    let mob_id = host
        .ecs()
        .query::<&crate::entity::MobKind>()
        .iter()
        .next()
        .map(|(id, _)| id)
        .unwrap();

    // Force an Idle{0} so the AI tick re-evaluates on the next call.
    {
        let mut ai = host.server.ecs.get::<&mut MobAi>(mob_id).unwrap();
        ai.state = crate::mob_ai::AiState::Idle { timer: 0 };
    }

    host.tick(1);

    let state = host.server.ecs.get::<&MobAi>(mob_id).unwrap();
    assert!(matches!(state.state, crate::mob_ai::AiState::Chase),
        "mob should be chasing the nearest player; got {:?}", state.state);
}

#[test]
fn peaceful_no_ticks_needed_for_empty_world() {
    // A GameServer with no pre-spawned mobs and no terrain stays empty even
    // after many ticks, because tick_mob_spawning requires a player position
    // and terrain support for spawns. Guards against accidental spawn bursts
    // from empty-world code paths.
    let mut host = TestHost::start_with(TestConfig::default());
    // Deliberately skip flat_floor + teleport — no terrain, no spawns.
    host.tick(2000);
    assert_eq!(host.ecs().query::<&crate::entity::MobKind>().iter().count(), 0);
}

// ── Blank-canvas: mobs_enabled=false ─────────────────────────────────────────

/// When `world.mobs_enabled = false`, `scatter_mobs_in_column` must produce
/// no spawns even when terrain + a player are present.
#[test]
fn scatter_mobs_produces_no_spawns_when_mobs_disabled() {
    use crate::biome::BiomeGenerator;
    use crate::entity::scatter_mobs_in_column;

    let mut ecs = hecs::World::new();
    let mut world = crate::world::World::new();
    world.mobs_enabled = false;

    // Build a minimal flat floor so surface-block checks pass.
    for x in -16..=16_i32 {
        for z in -16..=16_i32 {
            world.set_block(x, 10, z, STONE);
        }
    }

    let biome_gen = BiomeGenerator::new(42);

    // Call scatter for every column in a 4×4 patch — none should spawn.
    for cx in 0..4_i32 {
        for cz in 0..4_i32 {
            scatter_mobs_in_column(&mut ecs, cx, cz, &world, &biome_gen);
        }
    }

    assert_eq!(
        ecs.query::<&crate::entity::MobKind>().iter().count(),
        0,
        "mobs_enabled=false must produce zero spawns from scatter_mobs_in_column"
    );
}

/// Sanity: with `mobs_enabled = true`, scatter CAN spawn at least one mob
/// when real terrain is present (uses `do_initial_load` to generate proper
/// chunks that match biome_gen terrain heights).
#[test]
fn scatter_mobs_can_spawn_when_mobs_enabled() {
    use crate::biome::BiomeGenerator;
    use crate::entity::scatter_mobs_in_column;

    let mut ecs = hecs::World::new();
    let mut world = crate::world::World::new();
    world.mobs_enabled = true;

    // Use the real biome generator so terrain_height matches actual blocks.
    let biome_gen = BiomeGenerator::new(42);

    // Generate terrain for a 16×16 patch of columns so scatter has real
    // surface blocks to land on (not AIR from ungenerated chunks).
    for cx in -8..8_i32 {
        for cz in -8..8_i32 {
            world.generate_column(cx, cz, &biome_gen);
        }
    }

    for cx in -8..8_i32 {
        for cz in -8..8_i32 {
            scatter_mobs_in_column(&mut ecs, cx, cz, &world, &biome_gen);
        }
    }

    let count = ecs.query::<&crate::entity::MobKind>().iter().count();
    assert!(count > 0, "mobs_enabled=true with real terrain should spawn at least one mob; got 0");
}

/// `time_lock="day"` — the server spawning path must never produce night-mobs
/// even when `world_time` is at midnight (0) and terrain + player are present.
#[test]
fn mobs_off_at_night_when_time_locked_to_day() {
    let mut host = TestHost::start_with(TestConfig::default());
    flat_floor(&mut host, 10);
    host.teleport_player(0, Vec3::new(0.0, 12.0, 0.0));

    // Lock time to day.
    host.server.world.time_lock = "day".to_string();

    // Force world_time to midnight — night-mob spawning would normally fire here.
    host.server.world_time = 0;

    // Run enough ticks to cover several 400-tick spawn cycles.
    host.tick(2000);

    let count = host.ecs().query::<&crate::entity::MobKind>().iter().count();
    assert_eq!(count, 0, "time_lock=day should suppress night-mob spawning even at world_time=0; got {count}");
}

/// `time_lock="night"` — the server spawning path fires as night even when
/// `world_time` is at noon (12000).
#[test]
fn mobs_spawn_at_noon_when_time_locked_to_night() {
    let mut host = TestHost::start_with(TestConfig::default());
    flat_floor(&mut host, 10);
    host.teleport_player(0, Vec3::new(0.0, 12.0, 0.0));

    // Lock time to night.
    host.server.world.time_lock = "night".to_string();

    // Force world_time to noon — normally no night spawns.
    host.server.world_time = 12000;

    // Run enough ticks to cross several 400-tick spawn cycles.
    host.tick(2000);

    let count = host.ecs().query::<&crate::entity::MobKind>().iter().count();
    assert!(count > 0, "time_lock=night should enable night-mob spawning even at world_time=12000; got 0");
}
