//! Living world S4 (2026-11-08) — the Grey Quarry, driven end to end without a
//! window: the bundled trial's arena is built into a flat world exactly as the
//! fresh-world launch builds it (`arena_build::build_arena`), the grey start is
//! applied, the `Place` probe is registered, and a mixed restoration is carried
//! out and ticked at the trial's time scale until the scenario ends.
//!
//! What they pin (spec `docs/superpowers/specs/2026-11-08-living-world-tier1.md` §6):
//! - the quarry starts grey: level at the drained 0.05, target well under the
//!   "grey" line, stage "Grey and still";
//! - three tree kinds, three crops, water and animals bring it back to the
//!   trial's `at_least`, and the run ends by itself;
//! - the shortcuts do not: water alone, a single crop, animals alone leave it
//!   short of the goal, and nothing completes in the first minute;
//! - five guests at a Feast carry it too (the people route), and the end
//!   card's recovery names it.

use glam::Vec3;

use crate::biome::{BiomeGenerator, Generator};
use crate::bloom::{BloomField, Stage, TickInput};
use crate::block::{self, BlockRegistry};
use crate::mob::MobType;
use crate::place_trial;
use crate::scenario::{self, ScenarioState};
use crate::water::WaterSystem;
use crate::world::World;

/// Where the player's feet are on a flat arena world.
const ORIGIN: (i32, i32, i32) = (0, 80, 0);
/// The top surface (block y) of each terrace of the quarry: the dirt rows sit here.
const T0_TOP: i32 = 79;
const T1_TOP: i32 = 80;
const T2_TOP: i32 = 81;

struct Rig {
    world: World,
    bg: BiomeGenerator,
    ecs: hecs::World,
    registry: BlockRegistry,
    water: WaterSystem,
    bloom: BloomField,
    scn: ScenarioState,
    ticks: u64,
    /// How many times Satoshi's halfway line came due.
    halfway_lines: u32,
    /// True in a fresh arena, where a leaf may complete (`place_trial::step`).
    fresh_arena: bool,
}

impl Rig {
    fn new() -> Self {
        Self::with_scale(None)
    }

    /// The trial as launched; `scale` overrides its bloom time scale (to show
    /// what the tempo is worth).
    fn with_scale(scale: Option<f32>) -> Self {
        Self::build(scale, true)
    }

    /// The `/scenario grey-quarry` overlay in a player's own, living world: the
    /// same arena terrain, but no grey start, the world's own tempo, and no
    /// completing.
    fn overlay() -> Self {
        Self::build(Some(1.0), false)
    }

    fn build(scale: Option<f32>, fresh_arena: bool) -> Self {
        let def = scenario::named_builtin_def("grey-quarry").expect("the Grey Quarry is bundled");
        assert_eq!(def.world_type.as_deref(), Some("flat"));
        let arena = def.arena.clone().expect("it has an arena");
        let mut world = World::new();
        world.world_type = "flat".to_string();
        let bg = BiomeGenerator::new(1, Generator::V3);
        for cx in -2..=2 {
            for cz in -2..=1 {
                world.generate_column(cx, cz, &bg);
            }
        }
        let mut ecs = hecs::World::new();
        let mut water = WaterSystem::new();
        let mut lava = crate::lava::LavaSystem::new();
        if fresh_arena {
            crate::arena_build::build_arena(&mut world, &mut water, &mut lava, &mut ecs, ORIGIN, &arena);
        }
        let mut bloom = BloomField::new();
        let mut scn = ScenarioState::new(def.clone());
        bloom.set_time_scale(place_trial::effective_bloom_time_scale(
            Some(scale.unwrap_or(def.tuning.bloom_time_scale)),
            fresh_arena,
        ));
        place_trial::register_probes(&mut scn, &mut bloom, ORIGIN);
        if fresh_arena {
            place_trial::apply_drained(&mut bloom, &arena, ORIGIN);
        }
        Self { world, bg, ecs, registry: BlockRegistry::new(), water, bloom, scn, ticks: 0, halfway_lines: 0, fresh_arena }
    }

    fn tick(&mut self) {
        self.ticks += 1;
        crate::growth::tick_growth(&mut self.world, self.ticks, false, 7, false);
        self.water.tick_spread(&mut self.world);
        let me = [Vec3::new(0.5, 80.0, 0.5)];
        self.bloom.tick(&TickInput {
            world: &self.world,
            ecs: &self.ecs,
            biome_gen: &self.bg,
            registry: &self.registry,
            local_players: &me,
            remote_players: &[],
            creative: false,
            dt: 0.05,
            allow_grey_event: false,
            told_people: None,
            natural_known: true,
        });
        if place_trial::step(&mut self.scn, &self.bloom, self.fresh_arena) {
            self.halfway_lines += 1;
        }
    }

    fn run(&mut self, n: u64) {
        for _ in 0..n {
            self.tick();
        }
    }

    /// Tick until the probe has been scanned.
    fn settle(&mut self) {
        for _ in 0..600 {
            if self.bloom.probe(place_trial::probe_id(0)).is_some_and(|p| p.scanned) {
                return;
            }
            self.tick();
        }
        panic!("the probe was never scanned");
    }

    fn probe(&self) -> crate::bloom::ProbeState {
        self.bloom.probe(place_trial::probe_id(0)).expect("the probe is registered")
    }

    fn at(&self, dx: i32, y: i32, dz: i32) -> (i32, i32, i32) {
        (ORIGIN.0 + dx, y, ORIGIN.2 + dz)
    }

    /// Plant `id` the way a player does (placed, lit: a headless world has no
    /// light pass, and growth wants light 9).
    fn plant(&mut self, dx: i32, y: i32, dz: i32, id: block::BlockId) {
        let (x, y, z) = self.at(dx, y, dz);
        self.world.place_player_block(x, y, z, id);
        self.world.set_block_light_at(x, y, z, 15);
    }

    /// Hoe the dirt at `(dx, top, dz)` and sow `seed_stage_0` above it.
    fn sow(&mut self, dx: i32, top: i32, dz: i32, seed_stage_0: block::BlockId) {
        let (x, y, z) = self.at(dx, top, dz);
        self.world.place_player_block(x, y, z, block::TILLED_SOIL);
        self.plant(dx, top + 1, dz, seed_stage_0);
    }

    fn animal(&mut self, kind: MobType, dx: i32, dz: i32) {
        let p = Vec3::new((ORIGIN.0 + dx) as f32 + 0.5, 80.0, (ORIGIN.2 + dz) as f32 + 0.5);
        let e = crate::entity::spawn_mob(&mut self.ecs, kind, p);
        let _ = self.ecs.insert_one(e, crate::entity::Authored);
    }

    /// Break the cobblestone dam at the spring, as the player does.
    fn open_the_spring(&mut self) {
        for dx in [26, 27] {
            let (x, y, z) = self.at(dx, 82, 0);
            assert_eq!(self.world.get_block(x, y, z), block::COBBLESTONE, "the dam stands at {dx}");
            self.world.set_block(x, y, z, block::AIR);
            self.water.notify_block_removed(x, y, z, &self.world);
        }
    }

    fn plant_trees(&mut self) {
        // Two of each kind, on the dirt rows.
        for (dx, dz, top, sap) in [
            (9, -8, T0_TOP, block::SAPLING_OAK),
            (12, -8, T0_TOP, block::SAPLING_BIRCH),
            (9, 6, T0_TOP, block::SAPLING_SPRUCE),
            (12, 6, T0_TOP, block::SAPLING_OAK),
            (16, -5, T1_TOP, block::SAPLING_BIRCH),
            (17, 5, T1_TOP, block::SAPLING_SPRUCE),
        ] {
            self.plant(dx, top + 1, dz, sap);
        }
    }

    fn sow_crops(&mut self) {
        for (dx, dz, top, seed) in [
            (10, -8, T0_TOP, block::WHEAT_STAGE_0),
            (11, -8, T0_TOP, block::CARROT_STAGE_0),
            (10, 6, T0_TOP, block::POTATO_STAGE_0),
            (11, 6, T0_TOP, block::WHEAT_STAGE_0),
            (21, -4, T2_TOP, block::CARROT_STAGE_0),
            (22, -4, T2_TOP, block::POTATO_STAGE_0),
            (23, -4, T2_TOP, block::WHEAT_STAGE_0),
            (24, -4, T2_TOP, block::CARROT_STAGE_0),
            (21, 4, T2_TOP, block::POTATO_STAGE_0),
            (22, 4, T2_TOP, block::WHEAT_STAGE_0),
            (23, 4, T2_TOP, block::CARROT_STAGE_0),
            (24, 4, T2_TOP, block::POTATO_STAGE_0),
        ] {
            self.sow(dx, top, dz, seed);
        }
    }

    fn animals(&mut self) {
        self.animal(MobType::Sheep, 12, 0);
        self.animal(MobType::Sheep, 14, 1);
        self.animal(MobType::Cow, 16, -1);
        self.animal(MobType::Rabbit, 18, 2);
        self.animal(MobType::Rabbit, 20, -2);
    }
}


/// The goal the bundled trial asks for (the Rig reads the same JSON).
fn at_least() -> f32 {
    match scenario::named_builtin_def("grey-quarry").unwrap().objective {
        scenario::Objective::Place { at_least, .. } => at_least,
        other => panic!("the Grey Quarry is a Place trial, not {other:?}"),
    }
}

const MINUTE: u64 = 20 * 60;

#[test]
fn the_quarry_starts_grey_and_stays_grey_on_its_own() {
    let mut r = Rig::new();
    r.settle();
    let p = r.probe();
    assert!((p.level - 0.05).abs() < 0.011, "the drained start: {}", p.level);
    assert_eq!(p.stage, Stage::GreyAndStill);
    assert!(p.target < 0.2 && p.eco < 0.2, "the arena's own soil, stumps and spring are almost nothing: target {} eco {}", p.target, p.eco);
    assert!(p.people == 0.0);
    // Left alone for ten minutes it climbs to its target and no further.
    r.run(10 * MINUTE);
    let p = r.probe();
    assert!(p.level < 0.25, "it stays grey on its own: {}", p.level);
    assert_eq!(p.stage, Stage::GreyAndStill);
    assert!(!r.scn.is_ended());
    assert_eq!(r.scn.place_stage_phrase(), Some("Grey and still"));
}

#[test]
fn the_overlay_in_a_living_world_never_finishes_the_trial_by_itself() {
    // `/scenario grey-quarry` in the player's own world: no arena is built and
    // nothing is drained, so the probe starts at its target, and ordinary
    // living ground (a flat grass world here) already reads "Alive".
    let mut allowed = Rig::overlay();
    allowed.fresh_arena = true; // what the old code did: complete anywhere
    allowed.run(40 * MINUTE);
    assert!(allowed.scn.is_ended(), "living ground reads Alive on its own (level {})", allowed.probe().level);

    // As the overlay it still shows the stage words but never completes.
    let mut r = Rig::overlay();
    r.run(40 * MINUTE);
    assert!(!r.scn.is_ended(), "an overlay run never ends by itself");
    assert!(r.scn.place_recovery().is_none(), "and records no recovery");
    assert_eq!(r.scn.place_stage_phrase(), Some("Alive"), "the stage words still show");
}

/// Plant, sow and stock the quarry, open the spring, and tick until it ends.
/// Returns the ticks the restoration took from the moment the work was done.
fn restore_fully(r: &mut Rig) -> u64 {
    r.settle();
    r.open_the_spring();
    r.animals();
    r.plant_trees();
    r.sow_crops();
    let done_at = r.ticks;
    for _ in 0..(60 * MINUTE) {
        r.tick();
        if r.scn.is_ended() {
            return r.ticks - done_at;
        }
    }
    let p = r.probe();
    panic!("the mixed restoration never finished: level {} target {} eco {}", p.level, p.target, p.eco);
}

#[test]
fn three_tree_kinds_three_crops_water_and_animals_bring_it_back() {
    let mut r = Rig::new();
    let took = restore_fully(&mut r);
    // Trees roll to grow once per 200 ticks (one in thirty), so this is
    // seed-and-start-tick dependent but deterministic: it is neither seconds
    // nor hours. (Start the work three minutes later and the same trial took
    // twelve; playtest owns the real figure.)
    assert!(took > MINUTE, "{took} ticks is too quick for trees to grow");
    assert!(took < 30 * MINUTE, "{took} ticks is too slow");
    eprintln!("grey-quarry scripted restoration: {took} ticks after the work ({:.1} min), {} in all", took as f64 / MINUTE as f64, r.ticks);
    assert!(r.scn.is_ended());
    let p = r.probe();
    assert!(p.level >= at_least(), "ended at {}", p.level);
    assert_eq!(r.scn.place_stage_phrase(), Some("Alive"));
    assert_eq!(r.halfway_lines, 1, "Satoshi's halfway line came due exactly once, on the way");
    // The end card: mostly life.
    let rec = r.scn.place_recovery().expect("a finished Place trial has a recovery");
    assert_eq!(rec.ways(), vec!["life"]);
    assert_eq!(rec.sentence(), "You brought it back mostly through life.");
    assert!(rec.life > 0.9, "{rec:?}");
    assert!(rec.start.life < 0.2 && rec.end.life >= 0.5, "{rec:?}");
    // The probe left the field when the run ended.
    place_trial::remove_probes(&mut r.scn, &mut r.bloom);
    assert!(r.bloom.probe(place_trial::probe_id(0)).is_none());
}

#[test]
fn the_shortcuts_do_not_get_there() {
    // Water, animals and a field of crops, no trees: stuck well short of the goal.
    let mut r = Rig::new();
    r.settle();
    r.open_the_spring();
    r.animals();
    r.sow_crops();
    r.run(30 * MINUTE);
    let p = r.probe();
    assert!(p.level < at_least() - 0.05, "no trees: {} (target {})", p.level, p.target);
    assert!(!r.scn.is_ended());

    // Animals alone.
    let mut r = Rig::new();
    r.settle();
    r.animals();
    r.run(15 * MINUTE);
    assert!(r.probe().level < 0.3, "animals alone: {}", r.probe().level);
    assert!(!r.scn.is_ended());

    // The spring alone.
    let mut r = Rig::new();
    r.settle();
    r.open_the_spring();
    r.run(15 * MINUTE);
    assert!(r.probe().level < 0.3, "water alone: {}", r.probe().level);
    assert!(!r.scn.is_ended());

    // The work done, but not yet grown: nothing finishes in the first minute.
    let mut r = Rig::new();
    r.settle();
    r.open_the_spring();
    r.animals();
    r.plant_trees();
    r.sow_crops();
    r.run(MINUTE);
    assert!(!r.scn.is_ended());
}

#[test]
fn five_guests_at_a_feast_carry_it_too_and_the_card_says_people() {
    let mut r = Rig::new();
    r.settle();
    let (x, z) = (ORIGIN.0 + 10, ORIGIN.2);
    r.bloom.note_feast(x, z, 5);
    r.run(10 * MINUTE);
    assert!(r.scn.is_ended(), "five guests lift it past the goal: level {}", r.probe().level);
    let rec = r.scn.place_recovery().unwrap();
    assert_eq!(rec.ways(), vec!["people"]);
    assert_eq!(rec.sentence(), "You brought it back mostly through people together.");

    // Three guests are not enough.
    let mut r = Rig::new();
    r.settle();
    r.bloom.note_feast(x, z, 3);
    r.run(10 * MINUTE);
    assert!(!r.scn.is_ended(), "three guests fall short: level {}", r.probe().level);
}

#[test]
fn the_trials_tempo_is_what_makes_it_a_few_minutes() {
    // The same Feast at the world's own pace has hardly begun after four minutes.
    let mut slow = Rig::with_scale(Some(1.0));
    slow.settle();
    slow.bloom.note_feast(ORIGIN.0 + 10, ORIGIN.2, 6);
    slow.run(4 * MINUTE);
    assert!(!slow.scn.is_ended());
    assert!(slow.probe().level < 0.4, "at 1x: {}", slow.probe().level);
    // At the trial's 12x it has finished.
    let mut fast = Rig::new();
    fast.settle();
    fast.bloom.note_feast(ORIGIN.0 + 10, ORIGIN.2, 6);
    fast.run(4 * MINUTE);
    assert!(fast.scn.is_ended(), "at 12x: {}", fast.probe().level);
}

#[test]
fn the_arena_builds_the_terraces_fence_chest_and_meadow_the_trial_describes() {
    let r = Rig::new();
    let at = |dx, y, dz| r.world.get_block(ORIGIN.0 + dx, y, ORIGIN.2 + dz);
    // Terraces rise toward the spring: stone tops at 79, 80, 81, 82 (y).
    for (dx, top) in [(7, 79), (16, 80), (22, 81), (27, 82)] {
        assert_eq!(at(dx, top, -3), block::STONE, "terrace top at x+{dx}");
        assert_eq!(at(dx, top + 1, -3), block::AIR, "nothing above it at x+{dx}");
    }
    // The dry stream bed runs down the middle, below the terrace tops.
    assert_eq!(at(16, 79, 0), block::STONE);
    assert_eq!(at(16, 80, 0), block::AIR, "a trench, one below the terrace beside it");
    // The spring, held back by cobblestone.
    assert_eq!(at(28, 82, 0), block::WATER);
    assert!(r.water.is_source(ORIGIN.0 + 28, 82, ORIGIN.2));
    assert_eq!((at(27, 82, 0), at(26, 82, 0)), (block::COBBLESTONE, block::COBBLESTONE));
    // Dirt rows, log stumps.
    assert_eq!(at(10, 79, -8), block::DIRT);
    assert_eq!(at(7, 80, -11), block::OAK_LOG);
    // The chest at spawn and the fenced meadow outside the area.
    assert_eq!(at(1, 80, 1), block::CHEST);
    assert_eq!(at(-8, 80, 0), block::OAK_FENCE_GATE);
    assert_eq!(at(-15, 80, -6), block::OAK_FENCE_POST);
    // 2 sheep, 1 cow, 2 rabbits, all outside the quarry's x range.
    let mut census = std::collections::BTreeMap::new();
    for (_, (pos, kind)) in r.ecs.query::<(&crate::entity::Position, &crate::entity::MobKind)>().iter() {
        assert!(pos.0.x < ORIGIN.0 as f32, "the animals are in the meadow, not the quarry: {}", pos.0.x);
        *census.entry(format!("{:?}", kind.0)).or_insert(0) += 1;
    }
    assert_eq!(census.get("Sheep"), Some(&2));
    assert_eq!(census.get("Cow"), Some(&1));
    assert_eq!(census.get("Rabbit"), Some(&2));
    assert_eq!(census.len(), 3);
}

/// Living world S1 — "Try again" on the Grey Quarry in its fresh arena. The
/// first run is restored fully (the probe and the regions under the quarry
/// are high, the player has dug a terrace out); the retry the game performs
/// (`GameState::scenario_try_again`: fresh scenario state, a new probe, then
/// `place_trial::rewind_arena`) puts the terraces back and starts it grey again.
#[test]
fn try_again_relays_the_terraces_and_starts_the_quarry_grey_again() {
    let mut r = Rig::new();
    restore_fully(&mut r);
    assert!(r.scn.is_ended());
    let def = scenario::named_builtin_def("grey-quarry").unwrap();
    let arena = def.arena.clone().unwrap();
    let probe_cell = (ORIGIN.0 + 10, ORIGIN.2 - 3);
    assert!(r.bloom.level_at(probe_cell.0 as f32, probe_cell.1 as f32) > 0.5, "the region under the quarry came back to life");
    // The player dug a terrace cell out and flooded another with the spring.
    let dug = r.at(16, 79, -3);
    assert_eq!(r.world.get_block(dug.0, dug.1, dug.2), block::STONE);
    r.world.set_block(dug.0, dug.1, dug.2, block::AIR);

    let animals_before = r.ecs.query::<&crate::entity::MobKind>().iter().count();
    // What the game does on retry, step for step.
    let retry = |r: &mut Rig, rewind: bool| {
        place_trial::remove_probes(&mut r.scn, &mut r.bloom);
        r.scn = ScenarioState::new(def.clone());
        place_trial::register_probes(&mut r.scn, &mut r.bloom, ORIGIN);
        if rewind {
            let mut lava = crate::lava::LavaSystem::new();
            let touched = place_trial::rewind_arena(&mut r.world, &mut r.water, &mut lava, &mut r.bloom, &arena, ORIGIN);
            assert!(!touched.is_empty(), "the chunks to re-mesh are reported");
        }
        r.settle();
    };

    // Without the rewind (what a retry did before): the dug cell stays dug and
    // the world's bloom under the quarry stays high.
    let mut before = Rig::new();
    restore_fully(&mut before);
    let cell = before.at(16, 79, -3);
    before.world.set_block(cell.0, cell.1, cell.2, block::AIR);
    retry(&mut before, false);
    assert_eq!(before.world.get_block(cell.0, cell.1, cell.2), block::AIR, "not re-laid");
    assert!(before.bloom.level_at(probe_cell.0 as f32, probe_cell.1 as f32) > 0.5, "the region stays high");

    // With it: terraces back, grey again, a fresh run that is not already over.
    retry(&mut r, true);
    assert_eq!(r.world.get_block(dug.0, dug.1, dug.2), block::STONE, "the terrace is re-laid");
    let p = r.probe();
    assert!((p.level - 0.05).abs() < 0.011, "the grey start again: {}", p.level);
    assert_eq!(p.stage, Stage::GreyAndStill);
    assert!(
        r.bloom.level_at(probe_cell.0 as f32, probe_cell.1 as f32) < 0.1,
        "the regions under it are drained too: {}",
        r.bloom.level_at(probe_cell.0 as f32, probe_cell.1 as f32)
    );
    assert!(!r.scn.is_ended());
    assert_eq!(r.scn.place_stage_phrase(), Some("Grey and still"));
    // The arena's single blocks and mobs were not re-run: no duplicated animals.
    let animals = r.ecs.query::<&crate::entity::MobKind>().iter().count();
    assert_eq!(animals, animals_before, "no duplicated animals");
}

/// Living world S1/S2 review (SF4) — the retry re-lays the terraces as the
/// ARENA's terrain, not the player's: a cell the player built on is not left
/// placed (and so not left with an author), so the second run's bloom scan
/// reads exactly what the first run's did.
#[test]
fn try_again_leaves_no_player_placed_ground_so_the_second_scan_equals_the_first() {
    use crate::authorship::AuthorKey;
    let mut r = Rig::new();
    let def = scenario::named_builtin_def("grey-quarry").unwrap();
    let arena = def.arena.clone().unwrap();
    let scan = |w: &World| {
        let mut tally = crate::bloom::EcoTally::default();
        let reg = BlockRegistry::new();
        for cx in -2..=2 {
            for cz in -2..=1 {
                let chunks = crate::bloom::scan::RegionChunks::of(w, (cx, cz));
                crate::bloom::scan::scan_columns(&chunks, None, &reg, 0..=15, 0..=15, &mut tally);
            }
        }
        tally
    };
    let first = scan(&r.world);
    assert!(first.columns > 0);

    // The player digs terrace cells out and builds on them with their own blocks.
    let cells = [r.at(16, T1_TOP, -3), r.at(17, T1_TOP, -3), r.at(18, T1_TOP, -3)];
    for (x, y, z) in cells {
        assert_eq!(r.world.get_block(x, y, z), block::STONE, "a terrace cell");
        r.world.place_player_block_as(x, y, z, block::DIRT, Some(&AuthorKey::Local(0)));
    }
    assert!(!r.world.authors.is_empty());
    assert_ne!(scan(&r.world), first, "built ground reads differently from bare terrace");

    let mut lava = crate::lava::LavaSystem::new();
    place_trial::rewind_arena(&mut r.world, &mut r.water, &mut lava, &mut r.bloom, &arena, ORIGIN);
    for (x, y, z) in cells {
        assert_eq!(r.world.get_block(x, y, z), block::STONE, "re-laid");
        assert!(!r.world.is_placed(x, y, z), "and no longer placed");
    }
    assert!(r.world.authors.is_empty(), "no author is left on re-laid terrain");
    assert_eq!(scan(&r.world), first, "the second run's bloom inputs equal the first's");
}
