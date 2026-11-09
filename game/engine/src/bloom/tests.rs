//! Calibration and behaviour tests for the bloom field. The §1.2 bands are
//! pinned here: the formula's constants move to meet these tests.

use glam::{IVec3, Vec3};

use super::test_world::*;
use super::*;
use crate::biome::Generator;
use crate::block;
use crate::mob::MobType;

/// Untouched forest (seed 2, v3): the centre region.
const FOREST: RegionKey = (0, 0);
const FOREST_SEED: u32 = 2;
/// Untouched exposed stone, no green at all (mountains, seed 16).
const MOUNTAINS: RegionKey = (-5, -6);
const MOUNTAINS_SEED: u32 = 16;
/// Untouched desert (seed 15).
const DESERT: RegionKey = (-8, -8);
const DESERT_SEED: u32 = 15;

const QUARRY_Y: i32 = 40;

fn forest() -> Harness {
    let mut h = Harness::with_generator(FOREST_SEED, Generator::V3);
    h.generate(FOREST);
    h
}

fn quarry() -> Harness {
    let mut h = forest();
    h.strip(FOREST, QUARRY_Y);
    h
}

fn plant_kinds(h: &mut Harness, start: usize, count: usize, y: i32, id: u16) {
    let cols: Vec<_> = first_columns(start + count)[start..].to_vec();
    h.plant(FOREST, cols, y, id);
}

// ── Calibration of the life term (§1.2) ─────────────────────────────────

#[test]
fn untouched_forest_is_high_and_a_bare_natural_place_is_not_grey() {
    let mut h = forest();
    assert!(h.eco(FOREST) >= 0.8, "forest");
    for (seed, key, what) in [(MOUNTAINS_SEED, MOUNTAINS, "mountains"), (DESERT_SEED, DESERT, "desert")] {
        let mut m = Harness::with_generator(seed, Generator::V3);
        m.at(key);
        m.generate(key);
        let e = m.eco(key);
        assert!((0.45..0.7).contains(&e), "untouched {what} has no green and no wound: {e}");
    }
}

#[test]
fn natural_terrain_stays_natural_when_the_region_is_touched_elsewhere() {
    // The natural-top path: a single player block makes the region "touched",
    // and the column is regenerated to tell wound from rock. Natural stone,
    // cliffs and ravines must read the same as before.
    for (seed, key) in [(MOUNTAINS_SEED, MOUNTAINS), (DESERT_SEED, DESERT), (FOREST_SEED, FOREST), (13, (-4, 8))] {
        let mut h = Harness::with_generator(seed, Generator::V3);
        h.at(key);
        h.generate(key);
        let fresh = h.eco(key);
        h.world.place_player_block(key.0 * 16 + 1, 2, key.1 * 16 + 1, block::STONE);
        let touched = h.eco(key);
        assert!((fresh - touched).abs() < 0.02, "seed {seed} {key:?}: fresh {fresh} touched {touched}");
        assert!(touched >= 0.45);
    }
}

#[test]
fn a_first_mine_a_chopped_tree_and_a_small_house_stay_in_the_normal_band() {
    let mut h = forest();
    let (bx, bz) = (FOREST.0 * 16, FOREST.1 * 16);
    // First mine: a 3 x 3 shaft, ten deep, from the surface.
    for dz in 5..8 {
        for dx in 5..8 {
            let top = (0..256).rev().find(|&y| h.world.get_block(bx + dx, y, bz + dz) != block::AIR).unwrap();
            for y in (top - 9)..=top {
                h.world.set_block(bx + dx, y, bz + dz, block::AIR);
            }
        }
    }
    assert!(h.eco(FOREST) >= 0.6, "first mine");
    // One chopped tree: its logs and leaves within 3 blocks of the first log.
    let log = (0..16)
        .flat_map(|dz| (0..16).map(move |dx| (dx, dz)))
        .find_map(|(dx, dz)| {
            (0..256).rev().map(|y| (bx + dx, y, bz + dz)).find(|&(x, y, z)| block::is_any_log_block(h.world.get_block(x, y, z)))
        })
        .expect("a forest has a tree");
    for x in (log.0 - 3)..=(log.0 + 3) {
        for z in (log.2 - 3)..=(log.2 + 3) {
            for y in (log.1 - 2)..(log.1 + 12) {
                let b = h.world.get_block(x, y, z);
                if block::is_any_log_block(b) || block::is_any_leaves(b) {
                    h.world.set_block(x, y, z, block::AIR);
                }
            }
        }
    }
    assert!(h.eco(FOREST) >= 0.6, "one chopped tree");
    // A small house: 5 x 5 cobblestone, four high, placed by the player.
    let ground = (0..256).rev().find(|&y| h.world.get_block(bx + 1, y, bz + 1) != block::AIR).unwrap() + 1;
    for dz in 0..5 {
        for dx in 0..5 {
            for dy in 0..4 {
                let wall = dx == 0 || dz == 0 || dx == 4 || dz == 4 || dy == 3;
                if wall {
                    h.world.place_player_block(bx + 1 + dx, ground + dy, bz + 1 + dz, block::COBBLESTONE);
                }
            }
        }
    }
    assert!(h.eco(FOREST) >= 0.6, "a small house");
}

#[test]
fn a_stripped_quarry_is_grey() {
    let mut h = quarry();
    let e = h.eco(FOREST);
    assert!(e <= 0.1, "{e}");
}

#[test]
fn one_sapling_kind_forty_times_does_not_make_a_quarry_alive() {
    let mut h = quarry();
    plant_kinds(&mut h, 0, 40, QUARRY_Y + 1, block::SAPLING_OAK);
    let e = h.eco(FOREST);
    assert!(e < 0.35, "{e}");
    assert!(e > 0.1, "but it is not nothing: {e}");
}

/// A quarry with three tree kinds, three crops, water and two animal species.
fn rich_quarry() -> Harness {
    let mut h = quarry();
    let y = QUARRY_Y + 1;
    plant_kinds(&mut h, 0, 8, y, block::SAPLING_OAK);
    plant_kinds(&mut h, 8, 8, y, block::SAPLING_BIRCH);
    plant_kinds(&mut h, 16, 8, y, block::SAPLING_SPRUCE);
    plant_kinds(&mut h, 24, 8, y, block::WHEAT_STAGE_1);
    plant_kinds(&mut h, 32, 8, y, block::CARROT_STAGE_1);
    plant_kinds(&mut h, 40, 8, y, block::POTATO_STAGE_1);
    plant_kinds(&mut h, 48, 16, y, block::WATER);
    h.animal(FOREST, MobType::Cow);
    h.animal(FOREST, MobType::Sheep);
    h
}

#[test]
fn a_quarry_with_variety_comes_back() {
    let mut h = rich_quarry();
    let e = h.eco(FOREST);
    assert!(e >= 0.6, "{e}");
}

#[test]
fn people_and_villagers_are_not_wildlife() {
    let mut h = quarry();
    plant_kinds(&mut h, 0, 64, QUARRY_Y + 1, block::SAPLING_OAK);
    let alone = h.eco(FOREST);
    h.animal(FOREST, MobType::Villager);
    h.animal(FOREST, MobType::Peddler);
    h.animal(FOREST, MobType::Brigand);
    assert_eq!(h.eco(FOREST), alone, "people and hostiles add no life");
    h.animal(FOREST, MobType::Cow);
    assert!(h.eco(FOREST) > alone);
}

#[test]
fn a_monoculture_lowers_the_term() {
    let crops = |ids: &[u16]| {
        let mut h = quarry();
        let y = QUARRY_Y + 1;
        for (i, &id) in ids.iter().enumerate() {
            plant_kinds(&mut h, i * 32, 32, y, id);
        }
        h.eco(FOREST)
    };
    let mono = crops(&[block::WHEAT_STAGE_1, block::WHEAT_STAGE_2]);
    let mixed = crops(&[block::WHEAT_STAGE_1, block::CARROT_STAGE_1]);
    assert!(mono < mixed * 0.8, "mono {mono} mixed {mixed}");
}

#[test]
fn creative_is_vivid_even_in_a_quarry() {
    let mut h = quarry();
    h.creative = true;
    h.rescan(FOREST);
    let (eco, _, level, target) = h.field.region_terms(FOREST).unwrap();
    assert!(eco <= 0.1);
    assert!(target >= 0.6 && level >= 0.6, "level {level} target {target}");
}

#[test]
fn a_probe_reads_exactly_its_box() {
    let mut h = quarry();
    // Plant the north half of the region only.
    let cols: Vec<_> = (0..16).flat_map(|z| (0..8).map(move |x| (x, z))).collect();
    h.plant(FOREST, cols, QUARRY_Y + 1, block::SAPLING_OAK);
    let alive = h.probe_eco(IVec3::new(0, 0, 0), IVec3::new(7, 100, 15));
    let bare = h.probe_eco(IVec3::new(8, 0, 0), IVec3::new(15, 100, 15));
    assert!(alive > 0.4 && bare == 0.0, "alive {alive} bare {bare}");
}

// ── People together (§1.3) ──────────────────────────────────────────────

#[test]
fn two_players_together_raise_the_people_term_and_one_alone_does_not() {
    let mut solo = forest();
    solo.rescan(FOREST);
    solo.run(600);
    assert_eq!(solo.field.region_terms(FOREST).unwrap().1, 0.0);

    let mut pair = forest();
    pair.remote = vec![Vec3::new(12.0, 90.0, 12.0)];
    pair.rescan(FOREST);
    pair.run(600);
    let p = pair.field.region_terms(FOREST).unwrap().1;
    assert!(p > 0.05, "{p}");
    // And it is a lift, never a penalty: the target does not fall.
    let (eco, _, _, target) = pair.field.region_terms(FOREST).unwrap();
    assert!(target >= eco);
}

#[test]
fn a_feast_and_a_commons_lift_a_grey_place_to_at_most_seven_tenths() {
    let mut h = quarry();
    h.rescan(FOREST);
    h.field.note_feast(8, 8, 12);
    h.field.set_commons(&[CommonsArea::from_marker(8, 8)]);
    h.run(2);
    let (_, p, _, target) = h.field.region_terms(FOREST).unwrap();
    assert!(p > 0.6, "{p}");
    assert!(target > 0.4 && target <= 0.7 + 1e-4, "{target}");
    // Take the Commons away: the Feast surge alone remains, and fades.
    h.field.set_commons(&[]);
    h.run(2);
    assert!(h.field.region_terms(FOREST).unwrap().1 < p);
}

// ── Built together (Tier 2 S2, `b`) ─────────────────────────────────────

fn build_as(h: &mut Harness, who: &[&str]) {
    for (i, name) in who.iter().enumerate() {
        let key = crate::authorship::AuthorKey::Npub((*name).to_string());
        h.world.place_player_block_as(FOREST.0 * 16 + i as i32, QUARRY_Y + 1, 3, block::COBBLESTONE, Some(&key));
    }
}

#[test]
fn built_together_lifts_p_by_the_number_of_distinct_authors_and_one_author_does_not() {
    let p_of = |who: &[&str]| {
        let mut h = quarry();
        build_as(&mut h, who);
        h.rescan(FOREST);
        h.run(2);
        h.field.region_terms(FOREST).unwrap().1
    };
    // A solo player, no Feast, no Commons: P is exactly b.
    assert_eq!(p_of(&[]), 0.0, "no authors recorded");
    assert_eq!(p_of(&["a", "a", "a"]), 0.0, "one builder, however many blocks");
    for (who, n) in [(&["a", "b"][..], 2), (&["a", "b", "c"], 3), (&["a", "b", "c", "d", "e"], 5)] {
        let want = crate::authorship::built_together(n);
        let got = p_of(who);
        assert!((got - want).abs() < 1e-5, "{n} authors: {got} vs {want}");
    }
    let many: Vec<String> = (0..12).map(|i| format!("k{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!((p_of(&many) - crate::authorship::built_together(8)).abs() < 1e-5, "capped at eight");
}

#[test]
fn built_together_raises_the_target_of_a_grey_place_and_only_a_scan_moves_it() {
    let mut h = quarry();
    // Twenty-four neighbours are loaded too, so a sweep of the regions takes the
    // field's real cadence (four scans a tick, oldest first) rather than a tick.
    for rz in -2..=2 {
        for rx in -2..=2 {
            if (rx, rz) != FOREST {
                h.generate((rx, rz));
            }
        }
    }
    build_as(&mut h, &["a"]);
    h.rescan(FOREST);
    h.run(2);
    let (_, p0, _, target0) = h.field.region_terms(FOREST).unwrap();
    assert_eq!(p0, 0.0);
    // A second builder arrives. Bloom reads the table when it scans the region,
    // so a tick later the region has not been looked at again...
    build_as(&mut h, &["a", "b"]);
    h.run(1);
    assert_eq!(h.field.region_terms(FOREST).unwrap().1, 0.0, "not recomputed every tick");
    // ...and a full sweep of the tracked regions later it has.
    h.run(400);
    let (_, p1, _, target1) = h.field.region_terms(FOREST).unwrap();
    assert!(p1 > 0.39 && p1 < 0.40, "{p1}");
    assert!(target1 > target0, "{target1} vs {target0}");
}

#[test]
fn a_client_with_no_authorship_table_has_b_zero() {
    // A joined client's world copy has an empty table (the authority keeps it).
    let mut h = quarry();
    h.world.place_player_block(FOREST.0 * 16, QUARRY_Y + 1, 3, block::COBBLESTONE);
    h.world.place_player_block(FOREST.0 * 16 + 1, QUARRY_Y + 1, 3, block::COBBLESTONE);
    h.rescan(FOREST);
    h.run(2);
    assert_eq!(h.field.region_terms(FOREST).unwrap().1, 0.0);
}

// ── Lived in (Tier 2 S3, `u`) and Worn Paths ─────────────────────────────

/// Sixteen placed blocks in the region, so it is a place and not a field walked through.
fn build_sixteen(h: &mut Harness) {
    for i in 0..16 {
        h.world.place_player_block(FOREST.0 * 16 + i, QUARRY_Y + 1, 3, block::COBBLESTONE);
    }
}

/// `n` lived days of presence in the forest region, then the world on lived day `now`.
fn live_in(h: &mut Harness, n: u32, nights: u32) {
    for day in 0..n {
        h.world.use_record.note_present_at(100 + day, FOREST.0 * 16 + 8, 8);
    }
    for night in 0..nights {
        h.world.use_record.note_slept_at(night, FOREST.0 * 16 + 8, 8);
    }
}

#[test]
fn lived_in_lifts_p_only_where_sixteen_blocks_were_placed_and_follows_the_curve() {
    let p_of = |placed: bool, days: u32, nights: u32| {
        let mut h = quarry();
        if placed {
            build_sixteen(&mut h);
        } else {
            for i in 0..15 {
                h.world.place_player_block(FOREST.0 * 16 + i, QUARRY_Y + 1, 3, block::COBBLESTONE);
            }
        }
        live_in(&mut h, days, nights);
        h.rescan(FOREST);
        h.run(2);
        h.field.region_terms(FOREST).unwrap().1
    };
    assert_eq!(p_of(false, 10, 0), 0.0, "fifteen placed blocks is a field walked through");
    assert_eq!(p_of(true, 0, 0), 0.0, "nobody has lived there");
    // 10 lived days: 0.6 (1 - e^-1). Two nights count as four days.
    let want = 0.6 * (1.0 - (-1.0f32).exp());
    assert!((p_of(true, 10, 0) - want).abs() < 1e-5, "{}", p_of(true, 10, 0));
    assert!((p_of(true, 6, 2) - want).abs() < 1e-5, "{}", p_of(true, 6, 2));
    assert!(p_of(true, 10, 0) < 0.6 + 1e-6);
}

#[test]
fn lived_in_fades_with_lived_days_of_disuse_and_not_with_an_idle_calendar() {
    let mut h = quarry();
    build_sixteen(&mut h);
    live_in(&mut h, 10, 0);
    h.rescan(FOREST);
    h.run(2);
    let fresh = h.field.region_terms(FOREST).unwrap().1;
    assert!(fresh > 0.3);
    // Elsewhere in the world players live for 21 more lived days: this place is half faded.
    for day in 0..21 {
        h.world.use_record.note_present_at(500 + day, 10_000, 10_000);
    }
    h.rescan(FOREST);
    h.run(2);
    let half = h.field.region_terms(FOREST).unwrap().1;
    // Last used on lived day 10; the world is on lived day 31: 21 days of disuse.
    assert!((half - fresh / 2.0).abs() < 1e-5, "{half} vs {}", fresh / 2.0);
    // An idle server's calendar running on changes nothing: no player, no lived day.
    h.rescan(FOREST);
    h.run(400);
    assert!((h.field.region_terms(FOREST).unwrap().1 - half).abs() < 1e-5);
}

#[test]
fn a_client_with_no_use_record_has_u_zero() {
    let mut h = quarry();
    build_sixteen(&mut h);
    h.rescan(FOREST);
    h.run(2);
    assert_eq!(h.field.region_terms(FOREST).unwrap().1, 0.0);
}

#[test]
fn a_field_of_worn_paths_does_not_read_as_bare() {
    // The same quarry floor: bare stone reads as a wound...
    let mut h = quarry();
    assert_eq!(h.eco(FOREST), 0.0);
    // ...and trodden into roads it is not (a road is honour, not damage).
    for dz in 0..16 {
        for dx in 0..16 {
            h.world.set_block(FOREST.0 * 16 + dx, QUARRY_Y, FOREST.1 * 16 + dz, block::WORN_PATH);
        }
    }
    let roads = h.eco(FOREST);
    assert!(roads > 0.3, "{roads}");
    // And a few roads through a living lawn leave the lawn's average where it was.
    let mut lawn = forest();
    let before = lawn.eco(FOREST);
    for dx in 0..16 {
        let (x, z) = (FOREST.0 * 16 + dx, FOREST.1 * 16 + 5);
        let top = (0..=255).rev().find(|&y| lawn.world.get_block(x, y, z) != block::AIR).unwrap();
        if lawn.world.get_block(x, top, z) == block::GRASS {
            lawn.world.set_block(x, top, z, block::WORN_PATH);
        }
    }
    let after = lawn.eco(FOREST);
    assert!(after >= before - 0.05, "a road through the grass is not damage: {before} -> {after}");
}

#[test]
fn feast_markers_leave_a_lasting_floor() {
    let mut h = quarry();
    h.rescan(FOREST);
    h.field.set_feast_markers(&[(8, 8, 9), (9, 9, 9), (3, 3, 9)]);
    h.run(2);
    let p = h.field.region_terms(FOREST).unwrap().1;
    assert!((p - 0.3).abs() < 1e-3, "three big markers cap at 0.3: {p}");
}

// ── Level, tempo, probes, API (§1.4 to §1.7) ────────────────────────────

/// Two adjacent regions: forest (0,0) started grey, quarry (1,0) started full.
fn rise_and_fall(scale: f32) -> (f32, f32) {
    let mut h = forest();
    h.generate((1, 0));
    h.strip((1, 0), QUARRY_Y);
    h.field.set_time_scale(scale);
    h.field.set_initial_level(IVec3::new(0, 0, 0), IVec3::new(15, 0, 15), 0.0);
    h.field.set_initial_level(IVec3::new(16, 0, 0), IVec3::new(31, 0, 15), 1.0);
    h.settle((0, 0));
    h.settle((1, 0));
    let before = (h.field.level_at(8.0, 8.0), h.field.level_at(24.0, 8.0));
    assert!(before.0 < 0.01 && before.1 > 0.99, "authored starts: {before:?}");
    h.run(400);
    (h.field.level_at(8.0, 8.0) - before.0, before.1 - h.field.level_at(24.0, 8.0))
}

#[test]
fn falling_is_slower_than_rising_and_time_scale_speeds_both() {
    let (rise, fall) = rise_and_fall(1.0);
    assert!(rise > 0.03 && fall > 0.0, "rise {rise} fall {fall}");
    assert!(rise > fall * 4.0, "rise {rise} fall {fall}");
    let (rise12, fall12) = rise_and_fall(12.0);
    assert!(rise12 > rise * 5.0 && fall12 > fall * 5.0, "x12: {rise12} {fall12}");
}

#[test]
fn a_new_region_starts_at_its_target_and_unknown_places_read_neutral() {
    let mut h = quarry();
    assert_eq!(h.field.level_at(8.0, 8.0), 0.6, "nothing scanned yet");
    h.rescan(FOREST);
    let (_, _, level, target) = h.field.region_terms(FOREST).unwrap();
    assert_eq!(level, target);
    assert_eq!(h.field.level_at(8.0, 8.0), level);
    let (grid, ox, oz) = h.field.grid_around(8.0, 8.0);
    assert_eq!((ox, oz), (-128.0, -128.0));
    assert_eq!(grid[8 * 16 + 8], level, "the camera's own cell");
    assert_eq!(grid[0], 0.6, "unloaded neighbours are neutral");
}

#[test]
fn regions_are_tracked_round_local_players_only() {
    let mut h = forest();
    h.run(1);
    assert_eq!(h.field.tracked_regions(), 17 * 17);
    h.local = vec![Vec3::new(1000.0, 90.0, 1000.0)];
    h.run(1);
    assert_eq!(h.field.tracked_regions(), 17 * 17);
    assert_eq!(h.field.level_at(8.0, 8.0), 0.6, "the old region was dropped");
    h.remote = vec![Vec3::new(8.0, 90.0, 8.0)];
    h.run(1);
    assert_eq!(h.field.tracked_regions(), 17 * 17, "remote players do not extend tracking");
}

#[test]
fn probe_reports_stage_in_words_and_holds() {
    let mut h = rich_quarry();
    h.field.add_probe(1, IVec3::new(0, 0, 0), IVec3::new(15, 100, 15));
    h.field.set_probe_goal(1, 0.55, 5.0);
    assert!(!h.field.probe(1).unwrap().scanned);
    h.run(30);
    let s = h.field.probe(1).unwrap();
    assert!(s.scanned && s.level >= 0.55 && s.eco >= 0.6);
    assert_eq!(s.stage, Stage::WakingUp, "level reached but not held long enough");
    h.run(120);
    assert_eq!(h.field.probe(1).unwrap().stage, Stage::Alive);
    h.field.remove_probe(1);
    assert!(h.field.probe(1).is_none());

    let mut grey = quarry();
    grey.field.add_probe(2, IVec3::new(0, 0, 0), IVec3::new(15, 100, 15));
    grey.run(30);
    assert_eq!(grey.field.probe(2).unwrap().stage, Stage::GreyAndStill);
}

#[test]
fn an_authored_grey_start_sets_the_probe_and_the_region() {
    let mut h = rich_quarry();
    h.field.add_probe(1, IVec3::new(0, 0, 0), IVec3::new(15, 100, 15));
    h.field.set_initial_level(IVec3::new(0, 0, 0), IVec3::new(15, 100, 15), 0.05);
    h.run(30);
    assert!(h.field.probe(1).unwrap().level < 0.15);
    assert!(h.field.level_at(8.0, 8.0) < 0.15);
    assert!(h.field.probe(1).unwrap().target >= 0.6, "and it is climbing back");
}

#[test]
fn world_summary_averages_the_scanned_regions() {
    let mut h = forest();
    assert_eq!(h.field.world_summary(), WorldSummary::default());
    h.rescan(FOREST);
    let s = h.field.world_summary();
    assert_eq!(s.regions, 1);
    assert!(s.mean_eco >= 0.8 && s.mean_level >= 0.8 && s.mean_people == 0.0);
}

#[test]
fn place_went_grey_fires_once_and_only_after_the_grace_period() {
    let mut h = quarry();
    let mut fired = Vec::new();
    for tick in 1..=1500u32 {
        if h.tick().place_went_grey {
            fired.push(tick);
        }
    }
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert!(fired[0] as f32 * 0.05 >= 60.0, "fired at tick {}", fired[0]);
}

#[test]
fn place_went_grey_stays_quiet_in_a_trial_arena_and_in_a_healthy_world() {
    let mut arena = quarry();
    arena.allow_grey = false;
    arena.run(1500);
    assert!(!arena.tick().place_went_grey);
    // Once allowed again it still fires: the arena never used the one shot up.
    arena.allow_grey = true;
    assert!(arena.tick().place_went_grey);

    let mut healthy = forest();
    for _ in 0..1500 {
        assert!(!healthy.tick().place_went_grey);
    }
}

#[test]
fn a_creative_quarry_never_goes_grey() {
    let mut h = quarry();
    h.creative = true;
    for _ in 0..1500 {
        assert!(!h.tick().place_went_grey);
    }
}

#[test]
fn reset_forgets_the_world() {
    let mut h = forest();
    h.rescan(FOREST);
    h.field.add_probe(1, IVec3::ZERO, IVec3::new(15, 100, 15));
    h.field.reset();
    assert_eq!(h.field.tracked_regions(), 0);
    assert!(h.field.probe(1).is_none());
    assert_eq!(h.field.world_summary().regions, 0);
}
