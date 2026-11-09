//! Tier 2 S6: a tended Commons Shelf (`s`) and a looted one (x0.8) in bloom.
//!
//! Every shelf here is a block and a meta byte and nothing else (no chest
//! entity), because that is all a joined client has: the term is client-computed
//! from what every client can see.

use super::test_world::*;
use super::*;
use crate::biome::Generator;
use crate::block;
use crate::shelf::{FILL_MAX, LOOTED_BIT};

const FOREST: RegionKey = (0, 0);
const QUARRY_Y: i32 = 40;
/// Shelves sit just under the quarry floor, so they change no column's top and
/// the life term is the same with or without them: only `s` moves.
const UNDER: i32 = QUARRY_Y - 3;
/// ...and a section further down.
const DEEP: i32 = QUARRY_Y - 20;

fn quarry() -> Harness {
    let mut h = Harness::with_generator(2, Generator::V3);
    h.generate(FOREST);
    h.strip(FOREST, QUARRY_Y);
    h
}

/// Shelves on a stripped forest region, each `(x, y, z, meta)`; the region's
/// `(p, target)` after a fresh scan.
fn terms_with(shelves: &[(i32, i32, i32, u8)]) -> (f32, f32) {
    let mut h = quarry();
    for &(x, y, z, meta) in shelves {
        h.world.set_block(x, y, z, block::COMMONS_SHELF);
        h.world.set_meta((x, y, z), meta);
        assert!(h.world.chest_at((x, y, z)).is_none(), "no entity: blocks and meta alone");
    }
    h.rescan(FOREST);
    h.run(2);
    let (_, p, _, target) = h.field.region_terms(FOREST).unwrap();
    (p, target)
}

#[test]
fn a_full_shelf_lifts_p_to_a_half_and_a_half_full_one_to_a_quarter() {
    // A solo player, no Feast, no Commons, no authors: P is exactly s.
    assert_eq!(terms_with(&[]).0, 0.0);
    let (full, _) = terms_with(&[(3, UNDER, 3, FILL_MAX)]);
    assert!((full - 0.5).abs() < 1e-5, "{full}");
    let (half, _) = terms_with(&[(3, UNDER, 3, 64)]);
    assert!((half - 0.5 * 64.0 / 127.0).abs() < 1e-5, "{half}");
    let (empty, _) = terms_with(&[(3, UNDER, 3, 0)]);
    assert_eq!(empty, 0.0, "an empty shelf adds nothing");
}

#[test]
fn the_fullest_shelf_in_the_region_wins_and_shelves_do_not_add_up() {
    let one = terms_with(&[(3, UNDER, 3, 90)]).0;
    let three = terms_with(&[(3, UNDER, 3, 30), (5, UNDER, 3, 90), (7, UNDER, 3, 60)]).0;
    assert!((one - three).abs() < 1e-6, "{one} vs {three}");
    // Shelves at different heights count too; one in the next region does not.
    let high = terms_with(&[(3, DEEP, 3, 90)]).0;
    assert!((one - high).abs() < 1e-6);
    let elsewhere = terms_with(&[(20, UNDER, 3, FILL_MAX)]).0;
    assert_eq!(elsewhere, 0.0, "region (1, 0) is another place");
}

#[test]
fn a_looted_shelf_scales_the_target_by_four_fifths_and_adds_no_term_of_its_own() {
    let meta = 5;
    let (p_plain, plain) = terms_with(&[(3, UNDER, 3, meta)]);
    let (p_looted, looted) = terms_with(&[(3, UNDER, 3, meta | LOOTED_BIT)]);
    assert_eq!(p_plain, p_looted, "looted changes the target, not P");
    assert!((looted - 0.8 * plain).abs() < 1e-5, "{looted} vs 0.8 x {plain}");
    // A restocked shelf in the same region next to a looted one still dims it:
    // any looted shelf counts.
    let (_, both) = terms_with(&[(3, UNDER, 3, meta | LOOTED_BIT), (5, UNDER, 3, 100)]);
    let (_, only_full) = terms_with(&[(5, UNDER, 3, 100)]);
    assert!((both - 0.8 * only_full).abs() < 1e-5, "{both} vs 0.8 x {only_full}");
}

#[test]
fn creative_stays_vivid_whatever_the_shelves_say() {
    let mut h = quarry();
    h.creative = true;
    h.world.set_block(3, UNDER, 3, block::COMMONS_SHELF);
    h.world.set_meta((3, UNDER, 3), LOOTED_BIT);
    h.rescan(FOREST);
    h.run(2);
    let (_, _, _, target) = h.field.region_terms(FOREST).unwrap();
    assert!(target >= crate::bloom::level::CREATIVE_FLOOR - 1e-6, "{target}");
}

#[test]
fn a_shelf_lifts_a_grey_place_by_at_most_s_through_the_seven_tenths_cap() {
    let (_, bare) = terms_with(&[]);
    let (_, tended) = terms_with(&[(3, UNDER, 3, FILL_MAX)]);
    assert!(tended > bare);
    // s = 0.5, so the target gains 0.7 x 0.5 of the gap to 1.
    let want = 1.0 - (1.0 - bare) * (1.0 - 0.7 * 0.5);
    assert!((tended - want).abs() < 1e-4, "{tended} vs {want}");
}
