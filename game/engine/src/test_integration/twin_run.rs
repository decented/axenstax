//! Keep slice 1 — twin-run determinism (Keep handoff §7). Two `TestHost`
//! servers in one process, built from the same seed
//! (`TestHost::start_seeded`) and fed the same scripted events at the same
//! logical ticks, must reach the same `GameServer::sim_digest` at ticks 1, 200,
//! 1,200 and 2,000; a different seed must give a different digest.
//!
//! What the fixture holds: more than `fire::EVENT_BUDGET` fire cells due at
//! once, loaded furnaces, hopper chains that cross a chunk boundary in nine
//! columns, lit campfires, flowing water, scattered and placed mobs, and two
//! server-simulated joiners whose scripted teleports make the dedicated column
//! streamer scatter new columns in and stream old ones out (despawning their
//! wildlife). Machines tick inside `GameServer::tick`
//! (`simulates_block_machines`, the dedicated server's setting).
//!
//! What it can actually catch at a checkpoint: order at the fire budget, the
//! hopper transfers, the column scatter and the stream-out. Furnaces and
//! campfires are present but their order can't show here: the dedicated path
//! reserves no delivery batch, so no furnace is ever deferred, and campfire
//! order only reaches the emitted block changes, which the digest leaves out.
//! `sim_machine_tests.rs` covers both. Not in the fixture at all: power devices,
//! carts, crops, bees, salt licks and the other open sites in Spec 01 §4.1.4.
//!
//! Both twins share the process statics (`CHUNK_SYNC`, `data_dir::ROOT`,
//! one worlds root); each opens its own world folder. The twins share one
//! architecture, so floats are not in question here.

use glam::Vec3;

use crate::block;
use crate::item::{ItemStack, MaterialId};
use crate::mob::MobType;
use crate::player_intent::PlayerIntent;
use crate::save::WorldsRootGuard;
use crate::test_harness::{TestConfig, TestHost};

/// Initial-load radius (columns). Small, so two debug-build twins load fast.
pub(crate) const LOAD_RADIUS: i32 = 3;
/// The machine platform's height: above every v2 column (the old 96 roof).
pub(crate) const BASE_Y: i32 = 100;
/// One hopper chain per `(x, z)`, each in its own chunk column. Without the
/// sort in `hopper::collect_hopper_transfers`, each column is one more chance
/// for the twins to visit its two hopper chunks in a different order: a chain
/// misses with odds of one in two, all nine with one in 512.
const CHAINS: [(i32, i32); 9] = [
    (-4, -12),
    (4, -12),
    (20, -12),
    (-4, 10),
    (4, 10),
    (20, 10),
    (-4, 20),
    (4, 20),
    (20, 20),
];
/// A chain's top and bottom chests, and how much the top one starts with: at
/// one item every 8 ticks, it is still draining at tick 2,000, so a difference
/// in how far items have travelled is still there at every checkpoint.
const CHAIN_TOP_Y: i32 = 114;
const CHAIN_BOTTOM_Y: i32 = 108;
const CHAIN_STACKS: u8 = 6;
const CHAIN_STACK_SIZE: u8 = 64;
/// The campfires' x (at `y = BASE_Y + 1`, `z = 14`).
const CAMPFIRES_X: [i32; 3] = [-6, -3, 0];
/// The furnaces' x and z (at `y = BASE_Y + 1`).
const FURNACES_X: [i32; 3] = [16, 18, 20];
const FURNACES_Z: [i32; 2] = [0, 2];

/// Hex of a digest, for messages.
fn hex32(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// One twin, ready to tick: seeded world, dedicated-style machine ticking,
/// a column streamer at `sim_distance`, the machine fixture built, and
/// `joiners` server-simulated joiners on the platform.
pub(crate) fn twin(seed: u32, name: &str, joiners: usize, sim_distance: i32) -> TestHost {
    twin_loaded(seed, name, joiners, sim_distance, LOAD_RADIUS)
}

/// [`twin`] with the initial-load radius chosen (the bench loads its whole
/// sim distance up front).
pub(crate) fn twin_loaded(seed: u32, name: &str, joiners: usize, sim_distance: i32, load_radius: i32) -> TestHost {
    let mut h = TestHost::start_seeded(
        TestConfig {
            seed,
            world_name: name.to_string(),
            do_initial_load: true,
            ..TestConfig::default()
        },
        load_radius,
    );
    h.server.simulates_block_machines = true;
    h.server.column_streamer = Some(crate::server_stream::ColumnStreamer::default());
    h.server.set_sim_distance(sim_distance);
    build_fixture(&mut h);
    for j in 0..joiners {
        add_joiner(&mut h, Vec3::new(4.5 + 4.0 * j as f32, (BASE_Y + 1) as f32, 20.5));
    }
    h
}

/// A server-simulated joiner (what a remote connection becomes once its join
/// completes), standing at `pos`. Returns its slot.
pub(crate) fn add_joiner(h: &mut TestHost, pos: Vec3) -> usize {
    let mut sp = crate::server::ServerPlayer::new(pos);
    sp.server_simulated = true;
    sp.connected = true;
    sp.enter_world();
    h.server.players.push(sp);
    h.server.players.len() - 1
}

/// Every joiner's input for `tick`: walk, sprint and hop, turning every two
/// seconds, each on its own heading. Deterministic in `tick` and the slot.
pub(crate) fn drive_joiners(h: &mut TestHost, tick: u64) {
    for (slot, sp) in h.server.players.iter_mut().enumerate() {
        if !sp.server_simulated {
            continue;
        }
        sp.yaw = ((tick / 40) as f32) * 1.3 + slot as f32;
        sp.queue_intent(PlayerIntent {
            move_forward: 1.0,
            sprint: true,
            jump_held: tick % 3 == 0,
            ..PlayerIntent::default()
        });
    }
}

/// The machine fixture, all on a stone platform above the terrain that spans
/// four columns, so every system's cells sit in several chunks:
/// - a plank field with 240 fires lit at once (more than `EVENT_BUDGET` = 64
///   are due on each 4-tick pass, so the budget's pick is what's tested);
/// - six loaded furnaces;
/// - nine hopper chains, one per column of [`CHAINS`],
///   chest/hopper/chest/hopper/chest/hopper/chest from y 114 down to 108: the
///   hopper at 113 is in chunk cy 7, those at 111 and 109 in cy 6, and the
///   chest at 112 is shared, so the order the hopper chunks are visited decides
///   how far an item moves in one pass;
/// - three lit campfires with a smoke timer, which burn out and smoulder in-run;
/// - three water sources, one pouring off the platform's edge;
/// - a dozen placed animals and brigands next to the joiners.
pub(crate) fn build_fixture(h: &mut TestHost) {
    for x in -8..24 {
        for z in -8..24 {
            h.set_block(x, BASE_Y, z, block::STONE);
        }
    }
    // Fire field.
    let mut lit = 0;
    for x in -6..14 {
        for z in -6..8 {
            h.set_block(x, BASE_Y + 1, z, block::OAK_PLANKS);
            if lit < 240 {
                let now = h.server.tick_counter;
                if h.server.fire.ignite(&mut h.server.world, x, BASE_Y + 2, z, now) {
                    lit += 1;
                }
            }
        }
    }
    assert!(lit > 64 * 2, "fire fixture lit {lit} cells");
    // Furnaces.
    for (i, x) in FURNACES_X.into_iter().enumerate() {
        for z in FURNACES_Z {
            h.load_furnace(
                x,
                BASE_Y + 1,
                z,
                ItemStack::new_material(MaterialId::Copper, 8 + i as u8),
                ItemStack::new_material(MaterialId::Coal, 4),
            );
        }
    }
    // Hopper chains.
    for (x, z) in CHAINS {
        for y in [CHAIN_TOP_Y, 112, 110, CHAIN_BOTTOM_Y] {
            h.set_block(x, y, z, block::CHEST);
            let stacks: Vec<ItemStack> = if y == CHAIN_TOP_Y {
                (0..CHAIN_STACKS)
                    .map(|i| match i % 3 {
                        0 => ItemStack::new_block(block::COBBLESTONE, CHAIN_STACK_SIZE),
                        1 => ItemStack::new_material(MaterialId::Coal, CHAIN_STACK_SIZE),
                        _ => ItemStack::new_material(MaterialId::Copper, CHAIN_STACK_SIZE),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            h.insert_chest_with(x, y, z, &stacks);
        }
        for y in [113, 111, 109] {
            h.set_block(x, y, z, block::HOPPER);
        }
    }
    // Campfires. A fire's timers run down once per 4-tick machine pass, so the
    // longest of these (400 passes) is out by tick 1,600.
    for x in CAMPFIRES_X {
        let pos = (x, BASE_Y + 1, 14);
        h.set_block(pos.0, pos.1, pos.2, block::CAMPFIRE);
        h.server.world.insert_campfire(
            pos,
            crate::campfire::CampfireData {
                fuel_ticks: 100 + 50 * (x.unsigned_abs()),
                smoke_ticks: 200,
                ..Default::default()
            },
        );
    }
    // Water.
    h.place_water_source(8, BASE_Y + 1, 18);
    h.place_water_source(12, BASE_Y + 1, 18);
    h.place_water_source(23, BASE_Y + 1, 23);
    // Mobs.
    let kinds = [MobType::Cow, MobType::Pig, MobType::Sheep, MobType::Brigand];
    for i in 0..12 {
        let pos = Vec3::new(2.5 + (i % 6) as f32 * 3.0, (BASE_Y + 1) as f32, 17.5 + (i / 6) as f32 * 3.0);
        crate::entity::spawn_mob(&mut h.server.ecs, kinds[i % kinds.len()], pos);
    }
}

/// The scripted events, applied to a twin just before tick `tick` runs.
/// Player edits, a fire lit late, a source drained, and two teleports that
/// make the streamer load far columns (scatter) and later drop them
/// (stream-out despawn).
pub(crate) fn script(h: &mut TestHost, tick: u64) {
    drive_joiners(h, tick);
    let joiner = h.server.players.iter().position(|sp| sp.server_simulated);
    match tick {
        60 => h.place_block(4, BASE_Y + 1, 20, block::OAK_PLANKS),
        120 => h.set_block(2, BASE_Y + 1, 2, block::AIR),
        300 => {
            if let Some(j) = joiner {
                h.teleport_player(j, Vec3::new(200.5, 120.0, 40.5));
            }
        }
        500 => {
            let now = h.server.tick_counter;
            for x in 0..6 {
                let _ = h.server.fire.ignite(&mut h.server.world, x, BASE_Y + 2, 12, now);
            }
        }
        900 => h.remove_water_source(12, BASE_Y + 1, 18),
        1000 => {
            if let Some(j) = joiner {
                h.teleport_player(j, Vec3::new(-150.5, 120.0, -60.5));
            }
        }
        1500 => {
            if let Some(j) = joiner {
                h.teleport_player(j, Vec3::new(6.5, (BASE_Y + 1) as f32, 20.5));
            }
        }
        _ => {}
    }
}

/// Advance a twin to `target` ticks, applying [`script`] before each tick.
pub(crate) fn run_to(h: &mut TestHost, target: u64) {
    while h.server.tick_counter < target {
        let next = h.server.tick_counter + 1;
        script(h, next);
        h.server.tick();
    }
}

/// How many items a chest holds, all stacks together.
fn chest_items(h: &TestHost, pos: (i32, i32, i32)) -> u32 {
    let chest = h.server.world.chest_at(pos).expect("a chain chest");
    chest.slots.iter().flatten().map(|s| u32::from(s.count)).sum()
}

/// Every chain, as `(items in its top chest, items in its bottom chest)`.
fn chain_levels(h: &TestHost) -> Vec<(u32, u32)> {
    CHAINS
        .iter()
        .map(|&(x, z)| (chest_items(h, (x, CHAIN_TOP_Y, z)), chest_items(h, (x, CHAIN_BOTTOM_Y, z))))
        .collect()
}

/// The sections that differ between two twins, by name.
fn differing(a: &TestHost, b: &TestHost) -> Vec<&'static str> {
    let (sa, sb) = (a.server.sim_digest_sections(), b.server.sim_digest_sections());
    sa.iter().filter(|(name, d)| sb.get(*name) != Some(*d)).map(|(name, _)| *name).collect()
}

#[test]
fn twin_run_same_seed_same_digest() {
    let _root = WorldsRootGuard::new("keep-s1-twin-run");
    let mut a = twin(4242, "twin-a", 2, 2);
    let mut b = twin(4242, "twin-b", 2, 2);
    assert_eq!(
        a.server.sim_digest(),
        b.server.sim_digest(),
        "twins differ before the first tick: {:?}",
        differing(&a, &b)
    );
    let mut peak_fire_due = 0;
    let mut levels = chain_levels(&a);
    for at in [1u64, 200, 1200, 2000] {
        while a.server.tick_counter < at {
            let next = a.server.tick_counter + 1;
            let due = a.server.fire.digest_view().iter().filter(|(_, event, _)| *event <= next).count();
            peak_fire_due = peak_fire_due.max(due);
            run_to(&mut a, next);
        }
        run_to(&mut b, at);
        if at == 1200 {
            // The streamer really did follow the teleported joiner: the far
            // column it stands in was streamed in (scattered), and the one
            // it left at tick 1000 was streamed out (wildlife despawned).
            assert!(a.server.loaded_columns.contains(&(-10, -4)), "joiner's far column loaded");
            assert!(!a.server.loaded_columns.contains(&(12, 2)), "column left behind streamed out");
        }
        let (da, db) = (a.server.sim_digest(), b.server.sim_digest());
        eprintln!("twin_run tick {at}: a {} b {}", hex32(&da), hex32(&db));
        assert_eq!(
            da,
            db,
            "twin digests differ at tick {at}; sections: {:?}",
            differing(&a, &b)
        );
        // Every chain is still moving at this checkpoint, so a difference in
        // how far its items have got is there to be seen.
        if at >= 200 {
            let now = chain_levels(&a);
            for (i, (was, is)) in levels.iter().zip(&now).enumerate() {
                assert!(is.0 > 0 && is.0 < was.0, "chain {i}'s top chest is draining at tick {at}: {was:?} -> {is:?}");
                assert!(is.1 > was.1, "chain {i}'s bottom chest is filling at tick {at}: {was:?} -> {is:?}");
            }
            levels = now;
        }
    }
    // The fixture really did run the systems it is there for.
    assert!(a.server.ecs.len() > 12, "mobs present: {}", a.server.ecs.len());
    assert!(
        peak_fire_due > crate::fire::EVENT_BUDGET,
        "{peak_fire_due} fire cells due at once is within the budget, so its pick never mattered"
    );
    for x in FURNACES_X {
        for z in FURNACES_Z {
            let furnace = a.server.world.furnace_at((x, BASE_Y + 1, z)).expect("a fixture furnace");
            assert!(furnace.output.is_some(), "furnace at ({x}, {z}) smelted something");
        }
    }
    for x in CAMPFIRES_X {
        assert_eq!(a.get_block(x, BASE_Y + 1, 14), block::CAMPFIRE_UNLIT, "campfire at x {x} burnt out");
    }
}

#[test]
fn twin_run_different_seed_different_digest() {
    let _root = WorldsRootGuard::new("keep-s1-twin-seed");
    let mut a = twin(4242, "seed-a", 0, 2);
    let mut b = twin(4243, "seed-b", 0, 2);
    run_to(&mut a, 1);
    run_to(&mut b, 1);
    assert_ne!(a.server.sim_digest(), b.server.sim_digest(), "another seed, another world");
}

/// Copy a world folder (the twins must not share one).
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("create world copy");
    for entry in std::fs::read_dir(from).expect("read world") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy world file");
        }
    }
}

/// A saved world opened twice: `initial_load`'s saved-world path relights,
/// re-registers and scatters every column, and that scatter allocates entity
/// ids. The fresh-world path the other tests take walks its region in a fixed
/// loop; this one used to walk a hash set.
#[test]
fn twin_run_saved_world_loads_the_same() {
    let _root = WorldsRootGuard::new("keep-s1-twin-load");
    let mut src = twin(4242, "load-src", 0, 2);
    run_to(&mut src, 40);
    src.server.try_save().expect("source world saves");
    for name in ["load-a", "load-b"] {
        copy_dir(&crate::save::world_dir("load-src"), &crate::save::world_dir(name));
    }
    let open = |name: &str| {
        TestHost::start_seeded(
            TestConfig {
                seed: 4242,
                world_name: name.to_string(),
                do_initial_load: true,
                ..TestConfig::default()
            },
            LOAD_RADIUS,
        )
    };
    let (mut a, mut b) = (open("load-a"), open("load-b"));
    assert_ne!(a.server.ecs.len(), 0, "the load scattered wildlife");
    assert_eq!(differing(&a, &b), Vec::<&str>::new(), "twins differ straight after loading");
    a.server.simulates_block_machines = true;
    b.server.simulates_block_machines = true;
    a.tick(200);
    b.tick(200);
    assert_eq!(differing(&a, &b), Vec::<&str>::new(), "twins differ 200 ticks after loading");
}

/// Diagnostic, not part of the gate: ticks two twins in lock-step and reports
/// the first tick and sections where they part. Run it by name with
/// `--ignored` when the twin test goes red.
#[test]
#[ignore]
fn twin_run_first_divergence() {
    let _root = WorldsRootGuard::new("keep-s1-twin-diverge");
    let mut a = twin(4242, "diverge-a", 2, 2);
    let mut b = twin(4242, "diverge-b", 2, 2);
    for t in 1..=2000u64 {
        run_to(&mut a, t);
        run_to(&mut b, t);
        let d = differing(&a, &b);
        if !d.is_empty() {
            panic!("first divergence at tick {t}: {d:?}");
        }
    }
}
