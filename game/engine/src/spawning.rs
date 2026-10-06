//! Mob spawning.
//!
//! Pure free function — no `self`, no renderer. Both `GameServer::tick()` and
//! the single-player client path call this with their own `ecs` / `world`.
//! The long-term target is server-only (single-player routes through
//! HostedServer); until that lands, both callers invoke the same logic on
//! their respective ECSes.

use glam::Vec3;

use crate::block;
use crate::camera;
use crate::entity::{self, MobKind};
use crate::hyena_ai;
use crate::mob::MobType;
use crate::world::World;

/// Run the 400-tick mob spawn cycle.
///
/// Gated upstream on `world_time % 400 == 0`; this function runs the full cycle
/// unconditionally when called. `player_positions` is the set of player foot
/// positions the spawner scans around (24-64 blocks out). Soft-capped at 80
/// hostiles in present columns. First reclaims night spawns stranded in a
/// column that is not present (Phase B1 review).
pub fn tick_mob_spawning(
    ecs: &mut hecs::World,
    world: &World,
    world_time: u32,
    player_positions: &[Vec3],
) {
    reclaim_stranded_night_spawns(ecs, world);
    // No mobs in the Workshop — it's a creative authoring room, and mobs make
    // the space unusable (owner report 2026-06-18). The guard is on
    // `is_workshop`, not a saved meta flag, so existing Workshop saves are fixed
    // too. `mobs_enabled` is now honoured here as well: this function previously
    // ignored it (only blank-canvas's day-lock kept night hostiles away), so a
    // mobs-off *cycling* world would still have spawned at night.
    if world.is_workshop || !world.mobs_enabled {
        return;
    }
    let (_, brightness) = camera::compute_sun(world_time);
    let is_night = brightness < 0.3;

    if is_night && !player_positions.is_empty() {
        // The 80 cap is HOSTILE-only (see entity.rs scatter cap docs). Counting
        // all mobs let dense passive wildlife (cows, sheep, etc.) fill the cap
        // and silently suppress every night hostile spawn ("no monsters at
        // night" once you have a farm). Count only hostiles against the cap.
        // Only hostiles in a present column count (Phase B1 review): one
        // frozen in a column that is not loaded (a hideout brigand, a raid
        // mob) must not hold the players' own area's spawns hostage.
        let hostile_count: usize = ecs
            .query::<(&MobKind, &entity::Position)>()
            .iter()
            .filter(|(_, (k, p))| {
                crate::mob::mob_def(k.0).category == crate::mob::MobCategory::Hostile
                    && world.is_column_present_at(p.0.x.floor() as i32, p.0.z.floor() as i32)
            })
            .count();
        if hostile_count < 80 {
            for (p_idx, p_pos) in player_positions.iter().enumerate() {
                let px = p_pos.x.floor() as i32;
                let pz = p_pos.z.floor() as i32;
                for attempt in 0..3u32 {
                    // Deterministic per-(world_time, player, attempt) hash.
                    let seed = world_time
                        .wrapping_mul(374761393)
                        .wrapping_add(attempt.wrapping_mul(668265263))
                        .wrapping_add((p_idx as u32).wrapping_mul(2246822519));
                    let h = (seed ^ (seed >> 13)).wrapping_mul(1274126177);
                    let h = h ^ (h >> 16);
                    let angle = (h % 360) as f32 * std::f32::consts::PI / 180.0;
                    let dist = 24.0 + (h >> 10) as f32 % 40.0;
                    let sx = px + (angle.cos() * dist) as i32;
                    let sz = pz + (angle.sin() * dist) as i32;

                    let mut sy = 90;
                    while sy > 0 && world.get_block(sx, sy, sz) == block::AIR {
                        sy -= 1;
                    }
                    let surface = world.get_block(sx, sy, sz);
                    // Spec 30 — gate on **block-light** at the spawn
                    // cell. We're already inside the `is_night` branch
                    // so sky-light is already "night" by sun-angle;
                    // the stored sky-light value (15 outdoors) doesn't
                    // day-modulate, so using `effective_light_at` here
                    // wrongly returns 11 for any open-sky surface +
                    // blocked every night spawn (the 2026-05-21
                    // playtest bug). Just check block-light: torches
                    // keep mobs away, no torches = night spawn.
                    let spawn_block_light = world.block_light_at(sx, sy + 1, sz);
                    if surface != block::AIR
                        && surface != block::WATER
                        && spawn_block_light == 0
                    {
                        // HP-6 (2026-05-23) — historical-pivot cutover.
                        // The fantasy roster was excised entirely (open-
                        // source IP cleanup); brigand-family mobs are the
                        // live night spawn pool.
                        // Berserker stays exclusive to Brigand Hideouts
                        // (the boss tier players have to hunt out, not
                        // stumble into in the dark).
                        //
                        // Distribution per spawn seed:
                        //   - Brigand ~62.5 % (slots 0/1/2/3/5)
                        //   - Marauder ~12.5 % (slot 4)
                        //   - Bear if Forest/Taiga, else Brigand (slot 6)
                        //   - Hyena if Savanna, else Brigand (slot 7)
                        //
                        // Biome inference is the same cheap surface-block
                        // heuristic introduced in HP-2; refining to
                        // BiomeGenerator is out of scope for the cutover.
                        let tag = (h >> 24) % 8;
                        let is_savanna_like = surface == block::SAND
                            && world.get_block(sx, sy + 1, sz) == block::AIR;
                        let is_taiga_like = surface == block::SNOW;
                        let is_forest_like = surface == block::GRASS;
                        let kind = match tag {
                            4 => MobType::Marauder,
                            6 if is_forest_like || is_taiga_like => MobType::Bear,
                            7 if is_savanna_like => MobType::Hyena,
                            _ => MobType::Brigand,
                        };
                        let spawn_pos = Vec3::new(sx as f32 + 0.5, sy as f32 + 1.0, sz as f32 + 0.5);
                        if matches!(kind, MobType::Hyena) {
                            // HP-2 pack-spawn. 2-4 hyenas in a tight cluster.
                            let pack = hyena_ai::pack_size_for_seed(h);
                            for (dx, dz) in hyena_ai::pack_offsets(pack) {
                                let p = Vec3::new(
                                    spawn_pos.x + dx as f32,
                                    spawn_pos.y,
                                    spawn_pos.z + dz as f32,
                                );
                                spawn_night_mob(ecs, MobType::Hyena, p);
                            }
                        } else {
                            spawn_night_mob(ecs, kind, spawn_pos);
                        }
                    }
                }
            }
        }
    }
}

/// Phase B1 review — despawn every night spawn standing in a column that is not
/// present. It is frozen there (`entity::tick_entities`) and never saved; one
/// that walked out of the loaded area into a column that was never loaded would
/// otherwise wait there for ever, since no stream-out comes for that column.
fn reclaim_stranded_night_spawns(ecs: &mut hecs::World, world: &World) {
    let stranded: Vec<hecs::Entity> = ecs
        .query::<(&entity::Position, &entity::NightSpawn)>()
        .iter()
        .filter(|(_, (p, _))| !world.is_column_present_at(p.0.x.floor() as i32, p.0.z.floor() as i32))
        .map(|(id, _)| id)
        .collect();
    for id in stranded {
        let _ = ecs.despawn(id);
    }
}

/// Spawn a night hostile tagged [`entity::NightSpawn`], so its column's unload
/// reclaims it (it is never saved).
fn spawn_night_mob(ecs: &mut hecs::World, kind: MobType, pos: Vec3) {
    let id = entity::spawn_mob(ecs, kind, pos);
    let _ = ecs.insert_one(id, entity::NightSpawn);
}

// Hotbar icon rendering moved to hud_ui.rs (egui-based).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{STONE, GRASS};

    // Derived from `camera::compute_sun`: elevation = sin((t/24000)·τ − π/2).
    // world_time=0 → elevation=−1 (darkest night, brightness≈0.05).
    // world_time=12000 → elevation=+1 (noon, brightness=1.0).
    // The doc comment on compute_sun is wrong about the phase offset; the math above is authoritative.
    /// Midnight — `is_night` branch in tick_mob_spawning fires.
    const NIGHT: u32 = 0;
    /// Noon — `is_night` is false, so no spawns occur.
    const DAY: u32 = 12000;

    /// Build a flat stone floor at y=5 covering a big enough patch for the
    /// spawner's 24–64 radius scan around (0,0,0).
    fn fixture_floor(y: i32) -> World {
        let mut world = World::new();
        for x in -80..=80 {
            for z in -80..=80 {
                world.set_block(x, y, z, STONE);
                world.set_block(x, y + 1, z, GRASS);
            }
        }
        world
    }

    #[test]
    fn empty_player_positions_no_spawn_at_night() {
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        tick_mob_spawning(&mut ecs, &world, NIGHT, &[]);
        assert_eq!(ecs.query::<&MobKind>().iter().count(), 0);
    }

    #[test]
    fn day_time_does_not_spawn_mobs() {
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        tick_mob_spawning(&mut ecs, &world, DAY, &[Vec3::new(0.0, 12.0, 0.0)]);
        assert_eq!(ecs.query::<&MobKind>().iter().count(), 0);
    }

    #[test]
    fn deterministic_spawning_at_night() {
        let world = fixture_floor(10);
        let players = vec![Vec3::new(0.0, 12.0, 0.0)];

        let mut ecs_a = hecs::World::new();
        let mut ecs_b = hecs::World::new();
        tick_mob_spawning(&mut ecs_a, &world, NIGHT, &players);
        tick_mob_spawning(&mut ecs_b, &world, NIGHT, &players);

        let count_a = ecs_a.query::<&MobKind>().iter().count();
        let count_b = ecs_b.query::<&MobKind>().iter().count();
        assert_eq!(count_a, count_b, "same inputs must yield same spawn count");
    }

    #[test]
    fn mob_cap_prevents_runaway_spawns() {
        // Pre-populate at-or-over the soft cap (80). The spawner must not add more.
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        for i in 0..80 {
            entity::spawn_mob(&mut ecs, MobType::Brigand,
                Vec3::new(i as f32, 12.0, 0.0));
        }
        let before = ecs.query::<&MobKind>().iter().count();
        assert_eq!(before, 80);

        // Run the spawner at night many times — cap must hold.
        for t in 0..20 {
            tick_mob_spawning(&mut ecs, &world, NIGHT + t, &[Vec3::new(0.0, 12.0, 0.0)]);
        }
        let after = ecs.query::<&MobKind>().iter().count();
        assert_eq!(after, 80, "soft-cap must prevent overshoot");
    }

    /// Phase B1 review (LOW) — hostiles frozen in a column that is not loaded
    /// (here: 80 of them far outside the fixture) no longer fill the cap, so
    /// the players' own area still gets its night spawns; and every night
    /// spawn is tagged `NightSpawn`, so the column unload can reclaim it.
    #[test]
    fn hostiles_in_unloaded_columns_do_not_fill_the_cap_and_night_spawns_are_tagged() {
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        for i in 0..80 {
            entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(1000.0 + i as f32, 12.0, 0.0));
        }
        for t in 0..20 {
            tick_mob_spawning(&mut ecs, &world, NIGHT + t, &[Vec3::new(0.0, 12.0, 0.0)]);
        }
        let spawned = ecs.query::<&MobKind>().iter().count() - 80;
        assert!(spawned > 0, "the far frozen hostiles must not suppress local spawns");
        let tagged = ecs.query::<(&MobKind, &entity::NightSpawn)>().iter().count();
        assert_eq!(tagged, spawned, "every night spawn carries NightSpawn");
    }

    /// A night spawn that walked out of the loaded area into a column that
    /// was never loaded is frozen there, and no stream-out will ever reclaim
    /// it: the spawn cycle does (by day too). An untagged hostile stays.
    #[test]
    fn the_spawn_cycle_reclaims_night_spawns_stranded_in_unloaded_columns() {
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        let stranded = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(500.0, 12.0, 0.0));
        ecs.insert_one(stranded, entity::NightSpawn).unwrap();
        let on_floor = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(5.0, 12.0, 0.0));
        ecs.insert_one(on_floor, entity::NightSpawn).unwrap();
        let guard = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(500.0, 12.0, 0.0));
        tick_mob_spawning(&mut ecs, &world, DAY, &[Vec3::new(0.0, 12.0, 0.0)]);
        assert!(!ecs.contains(stranded), "the stranded night spawn is reclaimed");
        assert!(ecs.contains(on_floor), "one in a loaded column stays");
        assert!(ecs.contains(guard), "an untagged hostile stays, frozen");
    }

    #[test]
    fn workshop_world_never_spawns_mobs() {
        // 2026-06-18 — the Workshop is a creative authoring room; mobs make the
        // space unusable, so the spawner must refuse there regardless of time of
        // day. Prove the fixture WOULD spawn at night, then that is_workshop
        // suppresses it entirely (so existing Workshop saves are fixed too — the
        // guard is on is_workshop, not a saved meta flag).
        let players = vec![Vec3::new(0.0, 12.0, 0.0)];

        let normal = fixture_floor(10);
        let mut ecs_normal = hecs::World::new();
        tick_mob_spawning(&mut ecs_normal, &normal, NIGHT, &players);
        assert!(
            ecs_normal.query::<&MobKind>().iter().count() > 0,
            "fixture sanity: a normal world spawns mobs at night"
        );

        let mut workshop = fixture_floor(10);
        workshop.is_workshop = true;
        let mut ecs_ws = hecs::World::new();
        tick_mob_spawning(&mut ecs_ws, &workshop, NIGHT, &players);
        assert_eq!(
            ecs_ws.query::<&MobKind>().iter().count(),
            0,
            "the Workshop must never spawn mobs"
        );
    }

    #[test]
    fn mobs_disabled_world_does_not_spawn_at_night() {
        // tick_mob_spawning previously ignored mobs_enabled entirely — only
        // blank-canvas's day-lock kept night hostiles away. The flag is now
        // honoured here too, so a mobs-off cycling world stays mob-free.
        let mut world = fixture_floor(10);
        world.mobs_enabled = false;
        let mut ecs = hecs::World::new();
        tick_mob_spawning(&mut ecs, &world, NIGHT, &[Vec3::new(0.0, 12.0, 0.0)]);
        assert_eq!(ecs.query::<&MobKind>().iter().count(), 0);
    }

    #[test]
    fn night_spawn_yields_brigands_after_hp6_cutover() {
        // HP-6 (2026-05-23) cutover — the night spawn pool produces
        // brigand-family mobs. Across 200 ticks we expect at least one
        // Brigand and at least one Marauder.
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        for t in 0..200 {
            tick_mob_spawning(&mut ecs, &world, NIGHT + t, &[Vec3::new(0.0, 12.0, 0.0)]);
        }
        let mut brigands = 0;
        let mut marauders = 0;
        for (_, kind) in ecs.query::<&MobKind>().iter() {
            match kind.0 {
                MobType::Brigand => brigands += 1,
                MobType::Marauder => marauders += 1,
                _ => {}
            }
        }
        assert!(brigands > 0, "expected at least one Brigand across 200 spawn ticks");
        assert!(marauders > 0, "expected at least one Marauder across 200 spawn ticks");
    }

    #[test]
    fn night_spawn_only_produces_kept_roster_after_hp6() {
        // Post-excision guarantee: the night spawn pool only ever emits
        // the kept hostile roster (Brigand / Marauder / Bear / Hyena).
        // Since the fantasy MobType variants no longer exist, the check
        // is positive: every spawned mob must be one of the kept kinds.
        // Tests both grass-surface (forest-like) and sand-surface
        // (savanna-like) branches across 500 ticks.
        let world = fixture_floor(10);
        let mut ecs = hecs::World::new();
        for t in 0..500 {
            tick_mob_spawning(&mut ecs, &world, NIGHT + t, &[Vec3::new(0.0, 12.0, 0.0)]);
        }
        for (_, kind) in ecs.query::<&MobKind>().iter() {
            assert!(matches!(
                kind.0,
                MobType::Brigand | MobType::Marauder | MobType::Bear | MobType::Hyena
            ), "night spawn produced unexpected mob {:?} post-cutover", kind.0);
        }
    }
}
