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
    let at = s.respawn_player(0, true).expect("respawned");
    body_fits(&s, at).unwrap();
}

/// A world whose origin is open sea, and a player whose saved respawn point is
/// the default over that origin (what a new world used to save): a respawn
/// with no bed must land on dry ground, not drop into the sea (or stay at the
/// default, in the air over columns nobody has loaded).
#[test]
fn a_respawn_with_no_bed_on_a_sea_origin_world_lands_on_dry_ground() {
    let seed = crate::spawn_search::sea_origin_seed();
    let mut s = keep_like(seed);
    let _ = s.world_spawn();
    let mut sp = crate::server::ServerPlayer::new(Vec3::new(0.5, 80.0, 0.5));
    sp.server_simulated = true;
    sp.combat.die(crate::survival::DamageCause::Generic);
    sp.dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
    s.players.push(sp);
    let at = s.respawn_player(0, true).expect("respawned");
    body_fits(&s, at).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
    let ground = s.world.get_block(at.x.floor() as i32, at.y.floor() as i32 - 1, at.z.floor() as i32);
    assert!(!block::is_fluid(ground), "seed {seed}: standing on a liquid at {at:?}");
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

/// A saved spawn is any finite float: at the `i32` limits the block maths
/// saturates (a debug build would panic on overflow), and the answer is the
/// input unchanged (nothing is loaded there) or a sane spot.
#[test]
fn a_saved_spawn_at_the_coordinate_limits_does_not_overflow() {
    let mut s = keep_like(PHONE_SEEDS[0]);
    load_square(&mut s, (0, 0), 1);
    let limit = i32::MAX as f32;
    let extremes = [f32::MAX, f32::MIN, limit, -limit, 3.0e9, -3.0e9, 1.0e30];
    for &a in &extremes {
        for &b in &extremes {
            for pos in [Vec3::new(a, b, 0.5), Vec3::new(0.5, b, a), Vec3::new(a, 70.0, b), Vec3::new(a, a, a)] {
                let fixed = fix_up(&s.world, &s.registry, pos);
                assert!(fixed.is_finite(), "{pos:?} -> {fixed:?}");
                assert_eq!(fixed, pos, "nothing is loaded out there: unchanged");
                assert_eq!(s.safe_spawn_loading(pos, false), pos);
            }
        }
    }
    // The block helpers themselves, at the integer limits.
    for (x, y, z) in [(i32::MAX, i32::MAX, i32::MAX), (i32::MIN, i32::MIN, i32::MIN), (i32::MAX, i32::MIN, 0)] {
        assert!(!stands_clear(&s.world, &s.registry, x, y, z));
    }
    // Non-finite stays as it was.
    let nan = Vec3::new(f32::NAN, 70.0, 0.5);
    assert!(fix_up(&s.world, &s.registry, nan).x.is_nan());
}

// ---- Keep slice 2a-f: `can_stand_exactly`, the exact-spot check a returning
// joiner's saved position is judged by. ----

use crate::meta::{Facing, with_aux, with_facing};

/// A stone floor at y = 63 (x, z in -3..=3): its top is y = 64.
fn flat_floor() -> (World, BlockRegistry) {
    let mut w = World::new();
    for x in -3..=3 {
        for z in -3..=3 {
            w.set_block(x, 63, z, block::STONE);
        }
    }
    (w, BlockRegistry::new())
}

fn stands(w: &World, r: &BlockRegistry, x: f32, y: f32, z: f32) -> bool {
    can_stand_exactly(w, r, Vec3::new(x, y, z))
}

#[test]
fn a_body_stands_on_ordinary_ground() {
    let (w, r) = flat_floor();
    assert!(stands(&w, &r, 0.5, 64.0, 0.5));
    assert!(stands(&w, &r, 0.2, 64.0, -0.3), "anywhere over the cell");
}

/// The cases the whole-cell check turned away: each is a place physics leaves
/// a body, and `stands_clear` (feet cell non-solid, block below) says no.
#[test]
fn a_body_stands_on_a_partial_block_at_its_height() {
    let (mut w, r) = flat_floor();
    w.set_block(0, 64, 0, block::STONE_SLAB);
    w.set_meta((0, 64, 0), with_facing(0, Facing::Down));
    assert!(stands(&w, &r, 0.5, 64.5, 0.5), "a bottom slab");
    assert!(!stands_clear(&w, &r, 0, 64, 0), "(the whole-cell check says no)");
    assert!(!stands(&w, &r, 0.5, 64.0, 0.5), "but not inside it");
    assert!(!stands(&w, &r, 0.5, 65.0, 0.5), "nor floating a half block above it");

    w.set_block(0, 64, 0, block::SNOW_LAYER);
    w.set_meta((0, 64, 0), with_aux(0, 0));
    assert!(stands(&w, &r, 0.5, 64.125, 0.5), "a snow layer");
    assert!(!stands_clear(&w, &r, 0, 64, 0));

    w.set_block(0, 64, 0, block::OAK_TRAPDOOR);
    w.set_meta((0, 64, 0), with_facing(0, Facing::North));
    assert!(stands(&w, &r, 0.5, 64.1875, 0.5), "a bottom trapdoor");
    assert!(!stands_clear(&w, &r, 0, 64, 0));
}

#[test]
fn a_body_stands_on_the_low_step_of_a_stair_but_not_through_the_high_one() {
    let (mut w, r) = flat_floor();
    w.set_block(0, 64, 0, block::STONE_STAIRS);
    w.set_meta((0, 64, 0), with_facing(0, Facing::North)); // the tall half is z in 0..0.5
    assert!(stands(&w, &r, 0.5, 64.5, 0.8), "on the low half");
    assert!(!stands_clear(&w, &r, 0, 64, 0));
    assert!(!stands(&w, &r, 0.5, 64.5, 0.4), "its box overlaps the tall half");
    assert!(stands(&w, &r, 0.5, 65.0, 0.4), "on top of the tall half");
}

#[test]
fn a_body_within_a_ledges_reach_still_stands() {
    let mut w = World::new();
    for x in -3..=0 {
        for z in -3..=3 {
            w.set_block(x, 63, z, block::STONE);
        }
    }
    let r = BlockRegistry::new();
    assert!(stands(&w, &r, 1.2, 64.0, 0.5), "centre over air, 0.3 box still over the edge block");
    assert!(!stands_clear(&w, &r, 1, 64, 0), "(the cell under its centre is air)");
    assert!(!stands(&w, &r, 1.4, 64.0, 0.5), "past the edge");
}

#[test]
fn nowhere_unsafe_is_a_place_to_stand() {
    let (mut w, r) = flat_floor();
    assert!(!stands(&w, &r, 0.5, 63.5, 0.5), "inside the floor");
    assert!(!stands(&w, &r, 0.5, 65.0, 0.5), "a block up: over a drop");
    assert!(!stands(&w, &r, 0.5, 70.0, 0.5), "high in the air");
    w.set_block(0, 65, 0, block::STONE);
    assert!(!stands(&w, &r, 0.5, 64.0, 0.5), "head in a ceiling");
    w.set_block(0, 65, 0, block::AIR);
    for hazard in [block::LAVA, block::WATER, block::FIRE] {
        w.set_block(0, 64, 0, hazard);
        assert!(!stands(&w, &r, 0.5, 64.0, 0.5), "feet in {hazard}");
        w.set_block(0, 64, 0, block::AIR);
        w.set_block(0, 65, 0, hazard);
        assert!(!stands(&w, &r, 0.5, 64.0, 0.5), "head in {hazard}");
        w.set_block(0, 65, 0, block::AIR);
    }
    assert!(stands(&w, &r, 0.5, 64.0, 0.5), "the same spot, clear again");
}

#[test]
fn an_unloaded_column_or_a_position_that_isnt_a_place_fails_closed() {
    let (w, r) = flat_floor();
    assert!(!stands(&w, &r, 500.5, 64.0, 500.5), "no column there reads as air, so nothing is ground");
    assert!(!can_stand_exactly(&World::new(), &r, Vec3::new(0.5, 64.0, 0.5)));
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(!stands(&w, &r, bad, 64.0, 0.5));
        assert!(!stands(&w, &r, 0.5, bad, 0.5));
        assert!(!stands(&w, &r, 0.5, 64.0, bad));
    }
    // Finite but past anything a cell index holds: no overflow, no panic.
    for big in [1e30, -1e30, f32::MAX, f32::MIN] {
        assert!(!stands(&w, &r, big, big, big));
        assert!(!stands(&w, &r, 0.5, big, 0.5));
    }
}

// ---- The one respawn rule (`safe_respawn_point`) and the saved spawn point ----

/// A sea-origin seed's server, with its spawn columns loaded as a joiner's are.
fn sea_server() -> (GameServer, u32, Vec3) {
    let seed = crate::spawn_search::sea_origin_seed();
    let mut s = keep_like(seed);
    let world_spawn = s.world_spawn();
    (s, seed, world_spawn)
}

/// The rule on an owning server: it may load the columns it reads.
fn rule(s: &mut GameServer, spawn: Vec3) -> Vec3 {
    safe_respawn_point(&mut ServerRespawn { server: s, may_load: true }, spawn)
}

fn assert_dry_standing(s: &GameServer, at: Vec3, what: &str) {
    assert!(is_dry_standing(&s.world, &s.registry, at), "{what}: {at:?} is not dry standing ground");
    body_fits(s, at).unwrap_or_else(|why| panic!("{what}: {why}"));
}

/// The shared function, over the sea: a respawn point in a sea column (the old
/// default over a sea origin) stands on the world spawn's dry land, loading
/// what it reads, whatever the height asked for.
#[test]
fn the_respawn_rule_leaves_a_sea_column_for_the_world_spawn() {
    let (mut s, seed, world_spawn) = sea_server();
    for y in [80.0, 64.0, 63.0, 30.0, 200.0] {
        let at = rule(&mut s, Vec3::new(0.5, y, 0.5));
        assert_dry_standing(&s, at, &format!("seed {seed} y {y}"));
        assert_eq!(at, world_spawn, "seed {seed} y {y}: the world spawn, the point a new world starts on");
    }
}

/// Land is not touched: a dry spot stays where it is, and the rule only
/// stands a body, never moves a saved spawn (it returns a spot, not a spawn).
#[test]
fn the_respawn_rule_keeps_a_spawn_that_already_stands_on_dry_ground() {
    let (mut s, _, world_spawn) = sea_server();
    assert_eq!(rule(&mut s, world_spawn), world_spawn);
    // A bed under a roof (the spawn cell is under a ceiling, on a bed) stays
    // under the roof: the rule never lifts it onto the roof.
    let (x, y, z) = (world_spawn.x.floor() as i32, world_spawn.y.floor() as i32, world_spawn.z.floor() as i32);
    s.world.set_block(x, y - 1, z, block::BED);
    for dx in -1..=1 {
        for dz in -1..=1 {
            s.world.set_block(x + dx, y + 3, z + dz, block::STONE);
        }
    }
    assert_eq!(rule(&mut s, world_spawn), world_spawn, "the bed's spot, under its roof");
    // The bed gone: the body stands where the floor now is, still under the roof.
    s.world.set_block(x, y - 1, z, block::AIR);
    s.world.set_block(x, y - 2, z, block::STONE);
    let at = rule(&mut s, world_spawn);
    assert!(is_dry_standing(&s.world, &s.registry, at), "{at:?}");
    assert!(at.y < (y + 3) as f32, "not on the roof: {at:?}");
}

/// A spawn nobody has loaded the columns of: an owning server loads them (it
/// restores, else generates) and then decides; a lent world's server may not
/// generate, so it leaves the spot as it was.
#[test]
fn the_respawn_rule_loads_what_it_reads_unless_the_world_is_lent() {
    let (mut s, seed, _) = sea_server();
    let far = Vec3::new(1200.5, 90.0, -900.5);
    assert!(!s.world.is_column_present_at(1200, -901));
    let at = rule(&mut s, far);
    assert!(s.world.is_column_present_at(1200, -901), "the owning server loaded the spawn's column");
    assert_dry_standing(&s, at, &format!("seed {seed} far spawn"));

    // A lent world's server may not generate into the host's world: it reads
    // what the host holds (the world spawn's ring) and generates nothing.
    let (mut lent, _, world_spawn) = sea_server();
    let at = safe_respawn_point(&mut ServerRespawn { server: &mut lent, may_load: false }, far);
    assert!(!lent.world.is_column_present_at(1200, -901), "a lent world's server generated nothing");
    assert_eq!(at, world_spawn, "the unloaded spawn falls back to the world spawn it holds");
}

/// What cannot be read comes back unchanged and never panics.
#[test]
fn the_respawn_rule_takes_a_spawn_that_is_not_a_number_as_it_comes() {
    let (mut s, _, _) = sea_server();
    let before = s.world.chunk_positions().count();
    for pos in [Vec3::new(f32::NAN, 70.0, 0.5), Vec3::new(f32::INFINITY, 70.0, 0.5), Vec3::new(f32::MAX, f32::MAX, f32::MIN), Vec3::new(3.0e9, 70.0, 0.5)] {
        let at = rule(&mut s, pos);
        assert!(at.x.is_nan() == pos.x.is_nan() && (at.x.is_nan() || at == pos), "{pos:?} -> {at:?}");
    }
    assert_eq!(s.world.chunk_positions().count(), before, "and generates nothing out there");
}

/// The server's respawn and the single-player respawn are the one function:
/// the server's answer is exactly `safe_respawn_point`'s, and neither
/// `server.rs` nor `game_loop.rs` carries a rule of its own any more.
#[test]
fn the_server_respawn_and_the_single_player_respawn_share_one_function() {
    let (mut s, seed, _) = sea_server();
    let default = Vec3::new(0.5, 80.0, 0.5);
    let mut sp = crate::server::ServerPlayer::new(default);
    sp.server_simulated = true;
    sp.combat.die(crate::survival::DamageCause::Generic);
    sp.dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
    s.players.push(sp);
    let expected = rule(&mut s, default);
    let at = s.respawn_player(0, true).expect("respawned");
    assert_eq!(at, expected, "seed {seed}: one rule");
    assert_dry_standing(&s, at, "server respawn");
    assert_eq!(s.players[0].spawn_pos, default, "the saved spawn point itself is left alone");

    let server_src = include_str!("../server.rs");
    assert!(!server_src.contains("fn standing_spot"), "no second copy of the rule in server.rs");
    assert!(server_src.contains("spawn_safety::safe_respawn_point("), "the server calls the shared rule");
    let game_src = include_str!("../game_loop.rs");
    assert!(game_src.contains("spawn_safety::safe_respawn_point(self"), "the single-player respawn calls it too");
}

/// A living player's Respawn is no free teleport, and asking loads nothing.
#[test]
fn a_respawn_asked_by_a_living_player_does_nothing() {
    let (mut s, _, _) = sea_server();
    s.players.push(crate::server::ServerPlayer::new(Vec3::new(1200.5, 90.0, -900.5)));
    assert_eq!(s.respawn_player(0, true), None);
    assert!(!s.world.is_column_present_at(1200, -901), "and loads no column");
}

/// A new world's start is its respawn point, on a seed whose origin is sea:
/// the same position, on solid, non-liquid ground with two air cells above it.
#[test]
fn a_new_world_on_a_sea_origin_seed_starts_and_respawns_on_the_same_dry_ground() {
    let seed = crate::spawn_search::sea_origin_seed();
    let bg = crate::biome::BiomeGenerator::new(seed, crate::biome::Generator::V3);
    let mut world = World::new();
    for (cx, cz) in crate::chunk_stream::spawn_pregen_columns(&world, &bg) {
        world.generate_column(cx, cz, &bg);
    }
    let registry = BlockRegistry::new();
    let mut slot = crate::player_slot::PlayerSlot::new(0, Vec3::new(0.5, 80.0, 0.5), 1.0);
    assert_eq!(slot.spawn_pos.to_array(), LEGACY_DEFAULT_SPAWN, "control: a fresh slot holds the legacy default");
    let spawn = crate::chunk_stream::world_spawn_point(&world, &bg);
    slot.start_at(spawn);
    assert_eq!(slot.spawn_pos, slot.player.pos, "seed {seed}: the respawn point is the start");
    assert_ne!(slot.spawn_pos.to_array(), LEGACY_DEFAULT_SPAWN);
    assert!(is_dry_standing(&world, &registry, slot.spawn_pos), "seed {seed}: {spawn:?}");
    let (x, y, z) = (spawn.x.floor() as i32, spawn.y.floor() as i32, spawn.z.floor() as i32);
    let ground = world.get_block(x, y - 1, z);
    // Solid ground, or a plant standing on it (the spawn search counts a plant as
    // the ground it grows from, and the feet are then one cell above it).
    assert!(registry.is_solid(ground) || registry.is_solid(world.get_block(x, y - 2, z)), "seed {seed}");
    assert!(!block::is_fluid(ground), "seed {seed}: standing on a liquid");
    assert_eq!((world.get_block(x, y, z), world.get_block(x, y + 1, z)), (block::AIR, block::AIR));
}

/// The saved spawn point: the legacy default is "unset", anything else is the
/// player's and is never touched.
#[test]
fn a_saved_spawn_that_is_the_legacy_default_is_repaired_and_any_other_is_untouched() {
    let world_spawn = Vec3::new(120.5, 71.0, -44.5);
    let last = Vec3::new(10.0, 64.0, 10.0);
    let repaired = |saved: Option<[f32; 3]>, bed_here: bool| restored_spawn(saved, last, bed_here, || world_spawn);

    assert_eq!(repaired(Some(LEGACY_DEFAULT_SPAWN), false), world_spawn, "the exact default is unset");
    // A spawn a player set: a bed's cell, `/spawnpoint`, near the default but not it.
    for set in [
        [10.5, 70.0, -3.25],
        [0.5, 81.0, 0.5],
        [0.5, 80.0, 0.6],
        [0.5000001, 80.0, 0.5],
        [-0.5, 80.0, 0.5],
        [0.5, 79.99, 0.5],
        [4.0e5, 64.0, -4.0e5],
    ] {
        assert_eq!(repaired(Some(set), false), Vec3::from_array(set), "{set:?} is a chosen spawn");
    }
    // A bed at (0, 79, 0) makes the default's exact floats: that one is a bed.
    assert_eq!(repaired(Some(LEGACY_DEFAULT_SPAWN), true), Vec3::from_array(LEGACY_DEFAULT_SPAWN));
    // A legacy save with no spawn field falls back to where the player was.
    assert_eq!(repaired(None, false), last);
    // The world spawn is only asked for when it is needed.
    let _ = restored_spawn(Some([10.5, 70.0, -3.25]), last, false, || panic!("not needed"));
    let _ = restored_spawn(None, last, false, || panic!("not needed"));
}

/// Through the real save file: the world saved with the legacy default comes
/// back repaired (the world spawn), one saved with a bed's or `/spawnpoint`'s
/// spawn comes back as it was, and the repaired value saves as it is (no
/// format change: the field is the same `Option<[f32; 3]>`).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_world_saved_with_the_legacy_spawn_loads_repaired_and_a_chosen_one_loads_untouched() {
    use crate::player_slot::PlayerSlot;
    let name = format!("__test_spawn_repair_{}__", std::process::id());
    let _ = crate::save::delete_world(&name);
    let world = World::new();
    let mut slot0 = PlayerSlot::new(0, Vec3::new(30.5, 66.0, 12.5), 1.0);
    let mut chosen = PlayerSlot::new(1, Vec3::new(31.5, 66.0, 12.5), 1.0);
    chosen.spawn_pos = Vec3::new(-40.5, 71.0, 88.25);
    slot0.spawn_pos = Vec3::from_array(LEGACY_DEFAULT_SPAWN);
    crate::save::save_world(&name, &world, &[slot0, chosen], 42, &[], &[], None, None).expect("save_world");

    let mut loaded = World::new();
    let (save, _) = crate::save::load_world(&name, &mut loaded).expect("load_world");
    let world_spawn = Vec3::new(120.5, 71.0, -44.5);
    let spawns: Vec<Vec3> = save
        .players
        .iter()
        .map(|p| restored_spawn(p.spawn_pos, Vec3::new(p.x, p.y, p.z), false, || world_spawn))
        .collect();
    assert_eq!(spawns[0], world_spawn, "the legacy default loads as the world spawn");
    assert_eq!(spawns[1], Vec3::new(-40.5, 71.0, 88.25), "a chosen spawn loads untouched");
    let _ = crate::save::delete_world(&name);
}

/// The load's own repair, on a real saved world: the legacy default becomes the
/// world spawn, the chosen spawn is left, and a saved column holding an edit is
/// not generated over to find the spawn (`chunk_stream::repair_legacy_spawn_slots`,
/// what `begin_load` runs for every local slot).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_load_repair_uses_the_saved_columns_and_fixes_only_the_legacy_default() {
    use crate::player_slot::PlayerSlot;
    let seed = crate::spawn_search::sea_origin_seed();
    let bg = crate::biome::BiomeGenerator::new(seed, crate::biome::Generator::V3);
    let name = format!("__test_spawn_load_repair_{}__", std::process::id());
    let _ = crate::save::delete_world(&name);
    let mut world = World::new();
    for (cx, cz) in crate::chunk_stream::spawn_pregen_columns(&world, &bg) {
        world.generate_column(cx, cz, &bg);
    }
    let expected = crate::chunk_stream::world_spawn_point(&world, &bg);
    // An edit deep under the spawn: the column is saved whole, as an edited one is.
    let marker = (expected.x.floor() as i32, 10, expected.z.floor() as i32);
    world.set_block(marker.0, marker.1, marker.2, block::GLASS);
    let mut legacy = PlayerSlot::new(0, expected, 1.0);
    legacy.spawn_pos = Vec3::from_array(LEGACY_DEFAULT_SPAWN);
    let mut chosen = PlayerSlot::new(1, expected, 1.0);
    chosen.spawn_pos = Vec3::new(-40.5, 71.0, 88.25);
    crate::save::save_world(&name, &world, &[legacy, chosen], seed, &[], &[], None, None).expect("save_world");

    let mut loaded = World::new();
    let (save, _) = crate::save::load_world(&name, &mut loaded).expect("load_world");
    let mut slots: Vec<PlayerSlot> = save
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut slot = PlayerSlot::new(i, Vec3::new(p.x, p.y, p.z), 1.0);
            slot.spawn_pos = p.spawn_pos.map_or(slot.player.pos, Vec3::from_array);
            slot
        })
        .collect();
    assert_eq!(slots[0].spawn_pos.to_array(), LEGACY_DEFAULT_SPAWN, "control: the file holds the legacy default");
    let mut columns = ahash::AHashSet::new();
    crate::chunk_stream::mark_held_columns_loaded(&loaded, &mut columns);
    let mut intake = crate::chunk_intake::ChunkIntake::default();
    crate::chunk_stream::repair_legacy_spawn_slots(&mut loaded, &bg, &BlockRegistry::new(), &mut columns, &mut intake, &mut slots);
    assert_eq!(slots[0].spawn_pos, expected, "seed {seed}: the legacy default loads as the world spawn");
    assert_eq!(slots[1].spawn_pos, Vec3::new(-40.5, 71.0, 88.25), "a chosen spawn is untouched");
    assert_eq!(loaded.get_block(marker.0, marker.1, marker.2), block::GLASS, "the saved column was not generated over");
    let _ = crate::save::delete_world(&name);
}

// ---- A bed in a small room (S2), and a spawn that is sea all round (S9) ----

/// A world made of just what a test puts in it, and no generator reach.
struct PlainWorld {
    world: World,
    registry: BlockRegistry,
    biome_gen: crate::biome::BiomeGenerator,
}

impl PlainWorld {
    fn new(world: World) -> Self {
        Self {
            world,
            registry: BlockRegistry::new(),
            biome_gen: crate::biome::BiomeGenerator::new(1, crate::biome::Generator::V2),
        }
    }
}

impl RespawnWorld for PlainWorld {
    fn respawn_parts(&self) -> (&World, &BlockRegistry, &crate::biome::BiomeGenerator) {
        (&self.world, &self.registry, &self.biome_gen)
    }
    fn ensure_respawn_column(&mut self, _cx: i32, _cz: i32) -> bool {
        false
    }
}

/// Satoshi's own hut (the shared `hut_cells`: floor, two wall rows, roof) with
/// its floor at `(ox, oy, oz)`, on stone.
fn hut_world(ox: i32, oy: i32, oz: i32) -> World {
    let mut w = World::new();
    for z in oz - 3..oz + 8 {
        for x in ox - 3..ox + 8 {
            for y in oy - 4..oy {
                w.set_block(x, y, z, block::STONE);
            }
        }
    }
    for c in crate::satoshi::hut_cells() {
        w.set_block(ox + c.rx as i32, oy + c.ry as i32, oz + c.rz as i32, c.block_id);
    }
    w
}

/// A bed inside the hut `build_house` makes: the respawn stays inside the hut,
/// on its floor, beside the bed. Before, the body went on the roof (the cell
/// above the bed has the ceiling for a head, so the vertical nudge climbed
/// past it).
#[test]
fn a_bed_in_satoshis_hut_respawns_beside_the_bed_on_the_floor() {
    let (ox, oy, oz) = (10, 64, 20);
    for rz in 1..=3 {
        for rx in 1..=3 {
            let mut ctx = PlainWorld::new(hut_world(ox, oy, oz));
            let bed = [ox + rx, oy + 1, oz + rz];
            ctx.world.set_block(bed[0], bed[1], bed[2], block::BED);
            let spawn = crate::item_actions::bed_spawn(bed);
            let at = safe_respawn_point(&mut ctx, spawn);
            let what = format!("bed at hut cell ({rx},{rz}): respawn {at:?}");
            // Inside the hut's footprint (its doorway is part of it).
            assert!(at.x >= ox as f32 && at.x < (ox + 5) as f32, "{what}: inside the hut");
            assert!(at.z >= oz as f32 && at.z < (oz + 5) as f32, "{what}: inside the hut");
            assert_eq!(at.y, (oy + 1) as f32, "{what}: on the floor, not the roof");
            assert!(
                (at.x.floor() as i32, at.z.floor() as i32) != (bed[0], bed[2]),
                "{what}: beside the bed, not in it"
            );
            assert!(is_dry_standing(&ctx.world, &ctx.registry, at), "{what}");
        }
    }
}

/// A spawn that is already clear is never moved sideways.
#[test]
fn a_clear_spawn_is_not_moved_beside_anything() {
    let mut ctx = PlainWorld::new(hut_world(10, 64, 20));
    let spawn = Vec3::new(12.5, 65.0, 22.5);
    assert_eq!(safe_respawn_point(&mut ctx, spawn), spawn, "standing on the hut floor already");
}

/// An island of dry land in a lake: the nearest dry standing spot within the
/// reach is found, and only within it.
#[test]
fn the_dry_land_search_finds_land_within_its_reach_only() {
    let registry = BlockRegistry::new();
    let mut lake = World::new();
    for z in -40..=40 {
        for x in -40..=40 {
            for y in 58..=62 {
                lake.set_block(x, y, z, block::STONE);
            }
            lake.set_block(x, 63, z, block::WATER);
        }
    }
    let near = Vec3::new(0.5, 64.0, 0.5);
    assert_eq!(dry_land_near(&lake, &registry, near), None, "all water: nothing dry");
    // Land 25 blocks off: found, standing on it.
    lake.set_block(25, 63, 0, block::STONE);
    assert_eq!(dry_land_near(&lake, &registry, near), Some(Vec3::new(25.5, 64.0, 0.5)));
    // Land only 40 blocks off: out of reach.
    lake.set_block(25, 63, 0, block::WATER);
    lake.set_block(40, 63, 0, block::STONE);
    assert_eq!(dry_land_near(&lake, &registry, near), None, "past the search radius");
}

/// A world whose own spawn is sea, with land a few blocks beyond: the respawn
/// lands on that land instead of falling back to `here`, over the water.
#[test]
fn a_world_spawn_that_is_sea_respawns_on_the_nearest_loaded_land() {
    let (mut s, seed, world_spawn) = sea_server();
    let (x, y, z) = (world_spawn.x.floor() as i32, world_spawn.y.floor() as i32, world_spawn.z.floor() as i32);
    // Drown the spawn and everything round it to 8 blocks, ground and all, so
    // the spawn search's own +-6 reach finds nothing dry either.
    for dz in -8..=8 {
        for dx in -8..=8 {
            for dy in -40..=34 {
                s.world.set_block(x + dx, y + dy, z + dz, block::WATER);
            }
        }
    }
    // A spawn in a sea column far off falls to the world spawn, which is wet now.
    let at = rule(&mut s, Vec3::new(0.5, 80.0, 0.5));
    assert!(is_dry_standing(&s.world, &s.registry, at), "seed {seed}: {at:?} is dry standing ground");
    let (dx, dz) = ((at.x - world_spawn.x).abs(), (at.z - world_spawn.z).abs());
    assert!(dx.max(dz) <= DRY_SEARCH_RADIUS as f32 + 1.0, "seed {seed}: {at:?} within reach of the spawn {world_spawn:?}");
    assert!(dx.max(dz) > 8.0, "seed {seed}: and outside the flooded patch: {at:?}, spawn {world_spawn:?}");
}

/// A lending host's streamer anchors a dead joiner's spawn column and the
/// world spawn's, so the fallback's ring is loaded when a lent respawn reads it.
#[test]
fn a_lending_host_anchors_the_world_spawn_ring_for_a_dead_joiner() {
    use crate::chunk_stream::respawn_anchor_columns;
    assert!(respawn_anchor_columns(vec![], (3, 4)).is_empty(), "nobody dead: nothing extra to keep");
    assert_eq!(respawn_anchor_columns(vec![(9, 9)], (3, 4)), vec![(9, 9), (3, 4)]);
    assert_eq!(respawn_anchor_columns(vec![(3, 4)], (3, 4)), vec![(3, 4)], "not twice");
}
