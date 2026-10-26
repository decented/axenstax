use super::*;
use crate::server::GameServer;

/// The worlds the phone runs played on (Keep logs), then a spread.
const PHONE_SEEDS: [u32; 5] = [397_521_144, 1_805_289_314, 2_189_147_539, 650_316_741, 2_041_798_994];
const SPREAD: usize = 55;

fn seeds() -> Vec<u32> {
    let mut out = PHONE_SEEDS.to_vec();
    let mut h: u64 = 0x5eed_5afe;
    for _ in 0..SPREAD {
        // splitmix64: a fixed, well-spread sequence.
        h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = h;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        out.push((z ^ (z >> 31)) as u32);
    }
    out
}

/// A server on a world made as `keep_world::open_world` makes one: a fresh
/// `WorldMeta` (newest generator, normal terrain) and the seed.
fn keep_like(seed: u32) -> GameServer {
    let mut s = GameServer::new(0, format!("spawn-safety-{seed}-{}", std::process::id()), seed);
    let meta = crate::save::WorldMeta::new("spawn-safety");
    s.world.apply_meta_rules(&meta);
    s.biome_gen = crate::biome::BiomeGenerator::new(seed, s.world.generator);
    s
}

/// Feet and head cells not solid (nor fluid), solid ground under the feet.
/// The world spawn search counts a plant (tall grass, a crop, a bush) as the
/// ground it stands on, so its feet can be one cell above a non-solid plant
/// on solid ground: the body settles one cell into the plant, its head in
/// the clear feet cell. That is accepted too (and is not inside terrain).
fn body_fits(s: &GameServer, p: Vec3) -> Result<(), String> {
    let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let at = |y| s.world.get_block(x, y, z);
    let (feet, head, below) = (at(y), at(y + 1), at(y - 1));
    let open = |b| !s.registry.is_solid(b) && !block::is_fluid(b);
    let ground = s.registry.is_solid(below)
        || (below != block::AIR && open(below) && s.registry.is_solid(at(y - 2)));
    if !open(feet) || !open(head) || !ground {
        return Err(format!("{p:?}: feet {feet}, head {head}, below {below}"));
    }
    Ok(())
}

/// [`body_fits`] with nothing to settle into: solid ground right under the
/// feet. What the raw search result is also checked against, for the record.
fn on_solid_ground(s: &GameServer, p: Vec3) -> bool {
    s.registry.is_solid(s.world.get_block(p.x.floor() as i32, p.y.floor() as i32 - 1, p.z.floor() as i32))
}

fn load_square(s: &mut GameServer, (cx, cz): (i32, i32), r: i32) {
    for dx in -r..=r {
        for dz in -r..=r {
            s.ensure_column_loaded(cx + dx, cz + dz);
        }
    }
}

/// Part 1's reproduction attempt, server side: for each seed, the spawn a
/// Keep hands a joiner (`GameServer::world_spawn`, as `join_spawn` calls
/// it), checked once the columns round it have generated as they do for a
/// joining player (the joiner's ring and the streamer's square). The raw
/// search result (`world_spawn_point`, before `fix_up`) is checked too and
/// any miss printed, so a run with `--nocapture` shows whether the search
/// alone ever put a body inside terrain.
#[test]
fn a_keep_joiners_spawn_is_clear_ground_across_seeds() {
    let started = std::time::Instant::now();
    let mut raw_misses = Vec::new();
    let mut misses = Vec::new();
    let mut over_a_plant = 0;
    for seed in seeds() {
        let mut s = keep_like(seed);
        // The Keep's initial load starts round the origin.
        load_square(&mut s, (0, 0), 1);
        let spawn = s.world_spawn();
        let raw = crate::chunk_stream::world_spawn_point(&s.world, &s.biome_gen);
        load_square(&mut s, crate::chunk_stream::column_of(spawn), 2);
        if let Err(why) = body_fits(&s, raw) {
            raw_misses.push(format!("seed {seed}: {why}"));
        }
        if !on_solid_ground(&s, spawn) {
            over_a_plant += 1;
        }
        if let Err(why) = body_fits(&s, spawn) {
            misses.push(format!("seed {seed}: {why}"));
        }
        assert_eq!(s.world_spawn(), spawn, "seed {seed}: the next joiner is handed the same spawn");
    }
    eprintln!(
        "spawn search alone: {} of {} seeds inside terrain {raw_misses:?}; {over_a_plant} stand one cell over a plant ({:.1} s)",
        raw_misses.len(),
        PHONE_SEEDS.len() + SPREAD,
        started.elapsed().as_secs_f32()
    );
    assert!(misses.is_empty(), "joiner spawns inside terrain: {misses:?}");
}

/// A returning joiner's saved spawn buried in the ground (or anywhere a
/// body can't stand) is moved to clear ground, loading its columns first.
#[test]
fn a_buried_or_far_saved_spawn_is_stood_on_clear_ground() {
    let mut s = keep_like(PHONE_SEEDS[0]);
    let spawn = s.world_spawn();
    let buried = spawn - Vec3::new(0.0, 6.0, 0.0);
    let fixed = s.safe_spawn_loading(buried, true);
    body_fits(&s, fixed).unwrap();
    assert!(fixed.y >= spawn.y - 1.0, "lifted out of the ground: {fixed:?}");

    // A saved spawn in columns nobody has loaded yet.
    let far = Vec3::new(300.5, 20.0, -200.5);
    assert!(!s.world.is_column_present_at(300, -201));
    let fixed = s.safe_spawn_loading(far, true);
    body_fits(&s, fixed).unwrap();

    // A spot already clear is kept exactly.
    assert_eq!(s.safe_spawn_loading(spawn, true), spawn);
    // A lending host loads nothing: an unloaded spot comes back unchanged.
    let elsewhere = Vec3::new(-900.5, 70.0, 900.5);
    assert_eq!(s.safe_spawn_loading(elsewhere, false), elsewhere);
}

/// A spot whose own column can't hold a body (a trunk from the ground up)
/// moves to the nearest clear column within the search radius.
#[test]
fn a_blocked_column_moves_the_spawn_to_a_clear_neighbour() {
    let mut s = keep_like(PHONE_SEEDS[3]);
    let spawn = s.world_spawn();
    let (x, y, z) = (spawn.x.floor() as i32, spawn.y as i32, spawn.z.floor() as i32);
    for dy in 0..6 {
        s.world.set_block(x, y + dy, z, block::OAK_LOG);
    }
    let fixed = fix_up(&s.world, &s.registry, spawn);
    body_fits(&s, fixed).unwrap();
    let (fx, fz) = (fixed.x.floor() as i32, fixed.z.floor() as i32);
    assert_ne!((fx, fz), (x, z), "off the trunk");
    assert!((fx - x).abs().max((fz - z).abs()) <= SEARCH_RADIUS);
    assert_eq!(fix_up(&s.world, &s.registry, spawn), fixed, "the same answer for every joiner");
}

/// The world spawn search's fallback (nothing clear within its spiral)
/// stands on the real loaded column's top, not the generator's estimate.
#[test]
fn the_spawn_fallback_stands_on_the_real_column_top() {
    let mut s = keep_like(PHONE_SEEDS[1]);
    load_square(&mut s, (0, 0), 1);
    let estimate = s.biome_gen.surface_y(0, 0).max(s.biome_gen.terrain_height(0, 0));
    // Rock well above where the clear-ground scan starts, round the origin.
    let roof = estimate + 40;
    for x in -7..=7 {
        for z in -7..=7 {
            for y in 1..=roof {
                s.world.set_block(x, y, z, block::STONE);
            }
        }
    }
    let p = crate::chunk_stream::snap_spawn_to_ground(&s.world, &s.biome_gen, Vec3::new(0.5, 70.0, 0.5));
    assert_eq!(p.y, (roof + 1) as f32, "above the rock, not inside it: {p:?}");
    assert_eq!(s.world.get_block(0, roof + 1, 0), block::AIR);
}

/// A respawn at a buried spawn point stands the body on clear ground.
#[test]
fn a_respawn_at_a_buried_spawn_point_stands_clear() {
    let mut s = keep_like(PHONE_SEEDS[2]);
    let spawn = s.world_spawn();
    let mut sp = crate::server::ServerPlayer::new(spawn - Vec3::new(0.0, 5.0, 0.0));
    sp.server_simulated = true;
    sp.combat.die(crate::survival::DamageCause::Generic);
    sp.dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
    s.players.push(sp);
    let at = s.respawn_player(0).expect("respawned");
    body_fits(&s, at).unwrap();
}

#[test]
fn a_camera_on_the_same_spawn_cell_is_inside_the_other_avatar() {
    let feet = Vec3::new(0.5, 65.0, 0.5);
    let eye = |dx: f32, dy: f32, dz: f32| feet + Vec3::new(dx, dy, dz);
    // Two joiners on one spawn: the eye 1.62 above the other's feet, which
    // the old 0.7-block feet check let through.
    assert!(camera_inside_avatar(feet, eye(0.0, 1.62, 0.0)));
    assert!(camera_inside_avatar(feet, eye(0.3, 1.62, -0.3)));
    assert!(camera_inside_avatar(feet, eye(0.2, 0.3, 0.0)), "the old rule still holds");
    assert!(!camera_inside_avatar(feet, eye(1.0, 1.62, 0.0)), "a step away it is drawn");
    assert!(!camera_inside_avatar(feet, eye(0.0, 3.0, 0.0)), "above the head it is drawn");
    assert!(!camera_inside_avatar(feet, eye(0.0, -1.5, 0.0)), "below the feet it is drawn");
}
