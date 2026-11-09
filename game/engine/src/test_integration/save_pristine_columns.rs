//! Spec 02 §7.5.1 — a world-folder save writes the columns that differ from
//! generation and nothing else (review 2026-11-09, B1). Explored but untouched
//! land is regenerated from the world's frozen generator at the next load, so
//! the save, the next world open and every autosave stop growing with the area
//! the player has merely walked past.

use std::collections::BTreeSet;
use std::path::Path;

use glam::Vec3;

use crate::biome::{BiomeGenerator, Generator};
use crate::block::{BlockRegistry, AIR, GLASS, TORCH};
use crate::chunk_stream::{load_column_blocks, load_queue_order, mark_held_columns_loaded, ColumnSims};
use crate::fire::FireSystem;
use crate::lava::LavaSystem;
use crate::player_slot::PlayerSlot;
use crate::save::{self, WorldMeta, WorldsRootGuard};
use crate::water::WaterSystem;
use crate::world::{World, MAX_CHUNK_Y, WORLD_HEIGHT};
use crate::world_open::{is_new_world, open_world, AutosavePolicy, OpenedWorld};

const SEED: u32 = 4242;
/// A cell deep in column (1, 1), and one in column (-2, -2).
const EDIT: (i32, i32, i32) = (21, 5, 21);
const OTHER_EDIT: (i32, i32, i32) = (-30, 5, -30);

fn bg() -> BiomeGenerator {
    BiomeGenerator::new(SEED, Generator::V2)
}

fn columns() -> impl Iterator<Item = (i32, i32)> {
    (-2..=2).flat_map(|cx| (-2..=2).map(move |cz| (cx, cz)))
}

/// The 5×5 columns round the origin generated, as a walk leaves them: nothing
/// in them has been changed.
fn explored() -> World {
    let bg = bg();
    let mut w = World::new();
    for (cx, cz) in columns() {
        w.generate_column(cx, cz, &bg);
    }
    w
}

fn edit(w: &mut World, (x, y, z): (i32, i32, i32)) {
    assert_ne!(w.get_block(x, y, z), GLASS);
    w.set_block(x, y, z, GLASS);
}

fn players() -> Vec<PlayerSlot> {
    vec![PlayerSlot::new(0, Vec3::new(0.5, 90.0, 0.5), 1.0)]
}

/// A meta naming the generator the test generates with, so a server that
/// opens the folder generates the same terrain.
fn meta(name: &str) {
    let mut meta = WorldMeta::new(name);
    meta.seed = SEED;
    meta.worldgen_version = Generator::V2.version();
    save::save_world_meta(name, &meta).unwrap();
}

fn save(name: &str, w: &World) {
    save::save_world(name, w, &players(), SEED, &[], &[], None, None).expect("save");
}

fn autosave(name: &str, w: &World) {
    save::autosave_world(name, w, &players(), SEED, &[], &[], None, None).expect("autosave");
}

/// The chunk files in `dir`, by key.
fn files(dir: &Path) -> BTreeSet<(i32, i32, i32)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return BTreeSet::new() };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".chunk")?;
            let p: Vec<i32> = stem.split('_').map(|s| s.parse().unwrap()).collect();
            Some((p[0], p[1], p[2]))
        })
        .collect()
}

/// The non-empty sections of column `(cx, cz)`.
fn sections(w: &World, (cx, cz): (i32, i32)) -> BTreeSet<(i32, i32, i32)> {
    (0..=MAX_CHUNK_Y)
        .filter(|&cy| w.persistable_chunk(cx, cy, cz).is_some_and(|c| !c.is_empty()))
        .map(|cy| (cx, cy, cz))
        .collect()
}

/// Every section's bytes in column `(cx, cz)` (`None` for an absent or empty one).
fn column_bytes(w: &World, (cx, cz): (i32, i32)) -> Vec<Option<Vec<u8>>> {
    (0..=MAX_CHUNK_Y)
        .map(|cy| w.get_chunk(cx, cy, cz).filter(|c| !c.is_empty()).map(|c| c.as_bytes()))
        .collect()
}

fn open(name: &str) -> World {
    let mut back = World::new();
    match open_world(name, &mut back, AutosavePolicy::Prefer).expect("opens") {
        OpenedWorld::Loaded { .. } => {}
        OpenedWorld::New => panic!("a saved world must open as saved, not as new"),
    }
    back
}

fn loaded_columns(w: &World) -> BTreeSet<(i32, i32)> {
    w.chunk_positions().map(|(cx, _, cz)| (cx, cz)).collect()
}

#[test]
fn an_explored_world_with_no_edit_saves_no_chunk_file() {
    let _g = WorldsRootGuard::new("pristine_none");
    let w = explored();
    let dir = save::world_dir("w");
    // The world's first save (staged), a later one, and an autosave.
    save("w", &w);
    assert!(dir.join("world.dat").is_file());
    assert_eq!(files(&dir.join("chunks")), BTreeSet::new(), "first save");
    save("w", &w);
    assert_eq!(files(&dir.join("chunks")), BTreeSet::new(), "a later save");
    autosave("w", &w);
    assert_eq!(files(&dir.join("autosave/chunks")), BTreeSet::new(), "an autosave");
    // A saved world with no chunk file is still a saved world.
    assert!(!is_new_world("w").unwrap());
    let back = open("w");
    assert_eq!(back.chunk_positions().count(), 0, "nothing to load: all of it regenerates");
}

#[test]
fn one_edit_saves_its_whole_column_and_nothing_else() {
    let _g = WorldsRootGuard::new("pristine_one_edit");
    let mut w = explored();
    edit(&mut w, EDIT);
    let first = sections(&w, (1, 1));
    assert!(first.len() > 1, "the column holds more sections than the edited one");
    let dir = save::world_dir("w");
    save("w", &w);
    assert_eq!(files(&dir.join("chunks")), first, "first save: every non-empty section of the edited column");
    // A later save and an autosave add the next edited column, whole, and only it.
    edit(&mut w, OTHER_EDIT);
    let both: BTreeSet<_> = first.union(&sections(&w, (-2, -2))).copied().collect();
    save("w", &w);
    assert_eq!(files(&dir.join("chunks")), both, "a later save");
    autosave("w", &w);
    assert_eq!(files(&dir.join("autosave/chunks")), both, "an autosave");
}

#[test]
fn a_reload_keeps_the_edit_and_regenerates_untouched_columns_block_for_block() {
    let _g = WorldsRootGuard::new("pristine_round_trip");
    let bg = bg();
    let mut w = explored();
    edit(&mut w, EDIT);
    // Mine section cy 1 of the edited column (y 16..32) out entirely: the
    // column is written whole, so the load generates none of it and the
    // mined-out section stays air (Spec 02 §8.4, the hole rule).
    for x in 16..32 {
        for y in 16..32 {
            for z in 16..32 {
                w.set_block(x, y, z, AIR);
            }
        }
    }
    let before: Vec<_> = columns().map(|col| (col, column_bytes(&w, col))).collect();
    save("w", &w);

    autosave("w", &w);
    for name in ["last save", "autosave"] {
        let mut back = World::new();
        if name == "autosave" {
            save::load_autosave("w", &mut back).expect("the autosave loads");
        } else {
            save::load_world("w", &mut back).expect("the last save loads");
        }
        assert_eq!(loaded_columns(&back), BTreeSet::from([(1, 1)]), "{name}: only the edited column is read in");
        // What `step_load` does to every column the load queue names.
        for (cx, cz) in columns() {
            load_column_blocks(&mut back, cx, cz, &bg, false);
        }
        assert_eq!(back.get_block(EDIT.0, EDIT.1, EDIT.2), GLASS, "{name}: the edit is kept");
        assert!(back.get_chunk(1, 1, 1).is_none_or(|c| c.is_empty()), "{name}: the mined-out section stays air");
        for (col, bytes) in &before {
            assert_eq!(&column_bytes(&back, *col), bytes, "{name}: column {col:?} differs from before the save");
            if *col != (1, 1) {
                let mut fresh = World::new();
                fresh.generate_column(col.0, col.1, &bg);
                assert!(
                    crate::chunk_verdict::column_matches(&back, &fresh, *col),
                    "{name}: untouched column {col:?} is generation's, side data included"
                );
            }
        }
    }
}

#[test]
fn the_live_pack_and_an_export_import_carry_the_edited_column_only_and_round_trip() {
    let _g = WorldsRootGuard::new("pristine_archive");
    let bg = bg();
    let mut w = explored();
    edit(&mut w, EDIT);
    let want = sections(&w, (1, 1));

    // The web build saves by packing the live world.
    let packed = crate::world_archive::pack_world(
        &WorldMeta::new("w"),
        &save::minimal_world_save_for_tests(SEED),
        &w,
        &[],
    )
    .unwrap();
    let mut unpacked = World::new();
    crate::world_archive::unpack_world(&packed, &mut unpacked).unwrap();
    let held: BTreeSet<_> = unpacked.chunk_positions().collect();
    assert_eq!(held, want, "the live pack carries the edited column, whole, and nothing else");

    // A native export of the saved folder, imported as a new world.
    meta("w");
    save("w", &w);
    let bytes = crate::native_world_io::export_world_native("w").unwrap();
    let copy = crate::native_world_io::import_world_native(&bytes).unwrap();
    assert_eq!(files(&save::world_dir(&copy).join("chunks")), want);
    let mut back = open(&copy);
    for (cx, cz) in columns() {
        load_column_blocks(&mut back, cx, cz, &bg, false);
    }
    assert_eq!(back.get_block(EDIT.0, EDIT.1, EDIT.2), GLASS);
    for col in columns() {
        assert_eq!(column_bytes(&back, col), column_bytes(&w, col), "column {col:?} after export and import");
    }
}

/// What a world entry does with a folder after `save::load_world` (or with
/// the web build's unpacked record): `begin_load` marks the columns read in
/// loaded and queues them with the load circle round the player's column
/// `at`, nearest first; `step_load` takes each in turn
/// (`ColumnSims::load_queued`: restore or generate, then light it at once).
fn run_load_queue(back: &mut World, at: (i32, i32), round: bool, bg: &BiomeGenerator) -> ahash::AHashSet<(i32, i32)> {
    let mut fx = Fixture::new(bg);
    mark_held_columns_loaded(back, &mut fx.loaded);
    let queue = load_queue_order(&fx.loaded, at, 2, round);
    let mut sims = fx.sims(back);
    for (cx, cz) in queue {
        sims.load_queued(cx, cz);
    }
    fx.loaded
}

/// Review 2026-11-09 (BLOCKER) — the load queue runs nearest first and lights
/// each column as it goes. A torch at the east edge of an edited column A
/// lights into its untouched neighbour E before E's turn, which conjures
/// empty light-only sections there. They must not count as "E was read from
/// disk": E is generated all the same, floor included. Before the fix, a flat
/// world or the Workshop lost E's floor for the session (and for good once
/// the player built there), and a biome world E's bedrock until a stream pass
/// healed it.
#[test]
fn a_lit_edit_at_a_column_edge_never_stops_the_untouched_neighbour_generating() {
    for kind in ["flat", "workshop", "biome"] {
        let _g = WorldsRootGuard::new(&format!("pristine_lit_edge_{kind}"));
        let bg = bg();
        let mut m = WorldMeta::new("w");
        m.seed = SEED;
        m.worldgen_version = Generator::V2.version();
        match kind {
            "flat" => {
                m.world_type = "flat".into();
                m.ground = "grass".into();
            }
            "workshop" => m.is_workshop = true,
            _ => {}
        }
        save::save_world_meta("w", &m).unwrap();
        let mut w = World::new();
        w.apply_meta_rules(&m);
        // The walk: W = (-1, 0), A = (0, 0), E = (1, 0).
        for cx in -1..=1 {
            w.generate_column(cx, 0, &bg);
        }
        // A torch on A's east edge, standing on A's ground (or on whatever
        // tops it): its light crosses into E at x 16.
        let top = (0..WORLD_HEIGHT).rev().find(|&y| w.get_block(15, y, 8) != AIR).expect("A has ground");
        w.place_player_block(15, top + 1, 8, TORCH);
        save("w", &w);

        // Reload with the player standing in W, beyond A: W, then A (read from
        // disk and lit, the torch with it), then E.
        let mut back = w.generation_twin();
        save::load_world("w", &mut back).expect("the save loads");
        assert_eq!(loaded_columns(&back), BTreeSet::from([(0, 0)]), "{kind}: only A is read in");
        run_load_queue(&mut back, (-1, 0), false, &bg);

        assert_eq!(back.get_block(15, top + 1, 8), TORCH, "{kind}: the edit is kept");
        let floor = if kind == "biome" { (24, 0, 8) } else { (24, crate::workshop::WORKSHOP_FLOOR_Y, 8) };
        assert_ne!(back.get_block(floor.0, floor.1, floor.2), AIR, "{kind}: E has its floor");
        let mut fresh = w.generation_twin();
        fresh.generate_column(1, 0, &bg);
        assert!(
            crate::chunk_verdict::column_matches(&back, &fresh, (1, 0)),
            "{kind}: the untouched neighbour E is generation's"
        );
    }
}

/// Review 2026-11-09 — the web build's resume (the poll branch unpacks the
/// IndexedDB record, then `begin_load` runs its preloaded branch) marks every
/// unpacked column loaded, as the folder load does, so a saved column outside
/// the first load circle is queued and taken as it is. Left unmarked, the
/// streamer later streamed it in with `regen_present`, refilling its mined-out
/// section. (The poll branch itself is web-only; this drives the same World
/// steps natively.)
#[test]
fn a_web_resume_queues_a_far_saved_column_and_never_generates_over_it() {
    let bg = bg();
    let mut w = explored();
    // A base well outside the load circle: a glass block, and section cy 1
    // (y 16..32) mined out to air.
    let far = (12, 0);
    w.generate_column(far.0, far.1, &bg);
    let (bx, bz) = (far.0 * 16, far.1 * 16);
    for x in bx..bx + 16 {
        for y in 16..32 {
            for z in bz..bz + 16 {
                w.set_block(x, y, z, AIR);
            }
        }
    }
    w.set_block(bx + 5, 5, bz + 5, GLASS);
    let packed = crate::world_archive::pack_world(&WorldMeta::new("w"), &save::minimal_world_save_for_tests(SEED), &w, &[])
        .unwrap();
    let mined = |world: &World| world.get_chunk(far.0, 1, far.1).is_none_or(|c| c.is_empty());

    // What the old resume did: nothing marked, so the column waits for the
    // streamer, which generates over it.
    let mut old = World::new();
    crate::world_archive::unpack_world_to_play(&packed, &mut old).unwrap();
    let mut fx = Fixture::new(&bg);
    fx.sims(&mut old).stream_in(far.0, far.1);
    assert!(!mined(&old), "the old path regenerated the mined-out section (the loss this fixes)");

    // The resume now: the column is queued and taken as it is.
    let mut back = World::new();
    crate::world_archive::unpack_world_to_play(&packed, &mut back).unwrap();
    let loaded = run_load_queue(&mut back, (0, 0), true, &bg);
    assert!(loaded.contains(&far), "the far saved column is loaded with the world");
    assert!(mined(&back), "its mined-out section stays air");
    assert_eq!(back.get_block(bx + 5, 5, bz + 5), GLASS);
    // Walking away and back evicts and restores it, never generating it.
    let mut fx = Fixture::new(&bg);
    fx.sims(&mut back).stream_out(far.0, far.1);
    fx.sims(&mut back).stream_in(far.0, far.1);
    assert!(mined(&back), "still air after a stream out and back");
    assert_eq!(back.get_block(bx + 5, 5, bz + 5), GLASS);
}

/// The sims a [`ColumnSims`] borrows besides the world.
struct Fixture<'b> {
    bg: &'b BiomeGenerator,
    registry: BlockRegistry,
    loaded: ahash::AHashSet<(i32, i32)>,
    water: WaterSystem,
    lava: LavaSystem,
    fire: FireSystem,
    ecs: hecs::World,
}

impl<'b> Fixture<'b> {
    fn new(bg: &'b BiomeGenerator) -> Self {
        Self {
            bg,
            registry: BlockRegistry::new(),
            loaded: ahash::AHashSet::new(),
            water: WaterSystem::new(),
            lava: LavaSystem::new(),
            fire: FireSystem::new(),
            ecs: hecs::World::new(),
        }
    }

    fn sims<'a>(&'a mut self, world: &'a mut World) -> ColumnSims<'a> {
        ColumnSims {
            world,
            loaded: &mut self.loaded,
            registry: &self.registry,
            biome_gen: self.bg,
            water: &mut self.water,
            lava: &mut self.lava,
            fire: &mut self.fire,
            ecs: &mut self.ecs,
            tick: 0,
        }
    }
}

#[test]
fn a_dedicated_server_opening_the_folder_generates_the_untouched_columns() {
    let _g = WorldsRootGuard::new("pristine_server");
    let mut w = explored();
    edit(&mut w, EDIT);
    meta("w");
    save("w", &w);
    let mut server = crate::server::GameServer::new(0, "w".into(), SEED);
    server.render_distance = 2;
    server.initial_load().expect("the server opens it");
    assert_eq!(server.world.get_block(EDIT.0, EDIT.1, EDIT.2), GLASS, "the edit is kept");
    for (col, bytes) in columns().map(|col| (col, column_bytes(&w, col))) {
        assert_eq!(column_bytes(&server.world, col), bytes, "column {col:?} on the server");
    }
}
