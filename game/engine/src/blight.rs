//! Blight (living world Tier 2, S8; spec §8 and call T11).
//!
//! A big field of one crop is fragile. In a region (a chunk column) holding at
//! least [`MONO_MIN_CROPS`] crops of which [`MONO_SHARE_PCT`] percent or more are
//! one kind, each crop of that kind that would grow this pass turns into a
//! [`block::BLIGHTED_CROP`] one time in [`BLIGHT_ONE_IN`]. A Blighted Crop then
//! spreads to its same-kind horizontal neighbours one time in [`SPREAD_ONE_IN`]
//! per growth pass, whatever the region's mix is by then. A child's first wheat
//! patch (under 64 crops) or a mixed field never starts one.
//!
//! Everything here is pure and replay-stable: the dice are a position+tick hash
//! (the sapling pattern, `growth::advance_saplings`), no RNG state. The rules run
//! from `growth::tick_growth`, which both the dedicated/lending authority and the
//! standalone path reach through `sim_machines::step`, so they are written once.
//! Changes come back as ordinary [`BlockChange`]s (the kind rides `meta`), so
//! joiners see a blight exactly as they see a crop grow.
//!
//! A Blighted Crop is not "placed" and carries no author (the bit and the author
//! are cleared when a crop turns), so bloom reads it as bare ground
//! (`bloom::scan`). It never grows, drops nothing and bone meal does nothing to it
//! (it is not in `growth::is_crop`).

use crate::block::{self, BlockId};
use crate::play_mode::PlayMode;
use crate::protocol::BlockChange;
use crate::world::World;
use ahash::AHashMap;

/// A region needs at least this many crops to be a monoculture.
pub const MONO_MIN_CROPS: u32 = 64;
/// ...with the most common kind at least this share (percent) of them.
pub const MONO_SHARE_PCT: u32 = 90;
/// A monoculture crop that would grow blights one pass in this many.
pub const BLIGHT_ONE_IN: u64 = 40;
/// A neighbour of a Blighted Crop (same kind) blights one pass in this many.
pub const SPREAD_ONE_IN: u64 = 8;
/// At most this many Blighted Crops are spread from in one growth pass, so a
/// huge field cannot stall a tick. Past the cap the pass takes a window of the
/// (sorted) list that moves with the tick, so no part of a field starves.
pub const MAX_SPREAD_PER_PASS: usize = 512;

const SALT_START: u64 = 0xB11_6877_0000_0001;
const SALT_SPREAD: u64 = 0xB11_6877_0000_0100;

/// The crop kinds blight tells apart (one per `growth::is_crop` family). The
/// index is what a Blighted Crop keeps in `block_meta`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CropKind {
    Wheat = 0,
    Carrot = 1,
    Potato = 2,
    Corn = 3,
    Papyrus = 4,
    Cotton = 5,
    Hemp = 6,
    Cornflower = 7,
    FieldPoppy = 8,
    Buttercup = 9,
}

impl CropKind {
    pub const COUNT: usize = 10;
    const ALL: [CropKind; Self::COUNT] = [
        Self::Wheat, Self::Carrot, Self::Potato, Self::Corn, Self::Papyrus,
        Self::Cotton, Self::Hemp, Self::Cornflower, Self::FieldPoppy, Self::Buttercup,
    ];

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Option<CropKind> {
        Self::ALL.get(usize::from(i)).copied()
    }
}

/// The kind of a crop block, by id alone (any stage, including a mature wild-form
/// flower block, which [`crop_kind`] tells apart from a wild flower).
pub fn kind_of_block(b: BlockId) -> Option<CropKind> {
    use block::*;
    Some(match b {
        WHEAT_STAGE_0..=WHEAT_STAGE_3 => CropKind::Wheat,
        CARROT_STAGE_0..=CARROT_STAGE_3 => CropKind::Carrot,
        POTATO_STAGE_0..=POTATO_STAGE_3 => CropKind::Potato,
        CORN_STAGE_0..=CORN_STAGE_3 => CropKind::Corn,
        PAPYRUS_STAGE_0..=PAPYRUS_STAGE_3 => CropKind::Papyrus,
        COTTON_STAGE_0..=COTTON_STAGE_3 => CropKind::Cotton,
        HEMP_STAGE_0..=HEMP_STAGE_3 => CropKind::Hemp,
        CORNFLOWER | CORNFLOWER_STAGE_0..=CORNFLOWER_STAGE_2 => CropKind::Cornflower,
        FIELD_POPPY | FIELD_POPPY_STAGE_0..=FIELD_POPPY_STAGE_2 => CropKind::FieldPoppy,
        BUTTERCUP | BUTTERCUP_STAGE_0..=BUTTERCUP_STAGE_2 => CropKind::Buttercup,
        _ => return None,
    })
}

/// The kind of the FARMED crop `b` at `(x, y, z)`. A mature flower block is the
/// same id as the wild flower of a meadow; it is a crop only on tilled soil (the
/// rule `growth::crop_break` uses), so a wild meadow is never counted or blighted.
pub fn crop_kind(world: &World, x: i32, y: i32, z: i32, b: BlockId) -> Option<CropKind> {
    let kind = kind_of_block(b)?;
    let wild_form = matches!(b, block::CORNFLOWER | block::FIELD_POPPY | block::BUTTERCUP);
    if wild_form && world.get_block(x, y - 1, z) != block::TILLED_SOIL {
        return None;
    }
    Some(kind)
}

/// May blight run in this world this pass? Not in creative (a builder's field is
/// not a farm to lose), and not while a trial runs (`World::trial_running`, set by
/// the host client each tick: a farming trial must not turn flaky).
pub fn allowed(mode: PlayMode, world: &World) -> bool {
    !mode.is_creative() && !world.trial_running
}

/// Murmur-style avalanche of a cell, the tick and a salt (see
/// `growth::advance_saplings`: a plain xor keeps the tick's parity, and growth
/// ticks are always even).
fn roll(x: i32, y: i32, z: i32, tick: u64, salt: u64) -> u64 {
    let mut h = (x as u64 & 0xFFFF)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((z as u64 & 0xFFFF) << 20)
        .wrapping_add(y as u64)
        ^ tick.wrapping_mul(0xFF51_AFD7_ED55_8CCD)
        ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    h
}

/// Does the crop at this cell blight on this pass (given that it is in a
/// monoculture and would grow)?
pub fn blight_roll(x: i32, y: i32, z: i32, tick: u64) -> bool {
    roll(x, y, z, tick, SALT_START).is_multiple_of(BLIGHT_ONE_IN)
}

/// Does the Blighted Crop at this cell reach the neighbour in direction `dir`
/// (0..4) on this pass? Rolled per source and direction, so a cell with two
/// blighted neighbours is reached on either one's roll.
pub fn spread_roll(x: i32, y: i32, z: i32, tick: u64, dir: u8) -> bool {
    roll(x, y, z, tick, SALT_SPREAD + u64::from(dir)).is_multiple_of(SPREAD_ONE_IN)
}

/// The dominant crop kind of each monoculture region (chunk column).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Monocultures {
    dominant: AHashMap<(i32, i32), CropKind>,
}

impl Monocultures {
    fn region(x: i32, z: i32) -> (i32, i32) {
        let n = crate::chunk::CHUNK_SIZE as i32;
        (x.div_euclid(n), z.div_euclid(n))
    }

    /// The monoculture kind at column `(x, z)`, if its region is one.
    pub fn dominant_at(&self, x: i32, z: i32) -> Option<CropKind> {
        self.dominant.get(&Self::region(x, z)).copied()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.dominant.is_empty()
    }
}

/// Count `crops` (positions the growth pass already collected) by kind per
/// region and name the monocultures: at least [`MONO_MIN_CROPS`] crops, the most
/// common kind at least [`MONO_SHARE_PCT`] percent of them. Live crops only (a
/// Blighted Crop is not in `crops`), farmed ones only ([`crop_kind`]).
pub fn find_monocultures(world: &World, crops: &[(i32, i32, i32)]) -> Monocultures {
    let mut counts: AHashMap<(i32, i32), [u32; CropKind::COUNT]> = AHashMap::new();
    for &(x, y, z) in crops {
        let Some(kind) = crop_kind(world, x, y, z, world.get_block(x, y, z)) else { continue };
        counts.entry(Monocultures::region(x, z)).or_default()[usize::from(kind.index())] += 1;
    }
    let mut dominant = AHashMap::new();
    for (region, by_kind) in counts {
        let total: u32 = by_kind.iter().sum();
        if total < MONO_MIN_CROPS {
            continue;
        }
        let (best, &top) = by_kind.iter().enumerate().max_by_key(|&(i, &c)| (c, std::cmp::Reverse(i))).expect("ten kinds");
        // top / total >= pct / 100, in integers.
        if u64::from(top) * 100 >= u64::from(total) * u64::from(MONO_SHARE_PCT)
            && let Some(kind) = CropKind::from_index(best as u8)
        {
            dominant.insert(region, kind);
        }
    }
    Monocultures { dominant }
}

/// Turn the crop at `(x, y, z)` into a Blighted Crop of `kind`: the block, the
/// kind in `block_meta`, and no placed bit or author. The change to broadcast.
pub fn blight_cell(world: &mut World, x: i32, y: i32, z: i32, kind: CropKind) -> BlockChange {
    world.set_block(x, y, z, block::BLIGHTED_CROP);
    world.set_meta((x, y, z), kind.index());
    if world.is_placed(x, y, z) {
        world.set_placed(x, y, z, false);
    }
    BlockChange::with_meta(x, y, z, block::BLIGHTED_CROP, kind.index())
}

/// The kind a Blighted Crop at `cell` remembers (`None` for any other block, or a
/// meta byte that names no kind).
pub fn kind_remembered(world: &World, x: i32, y: i32, z: i32) -> Option<CropKind> {
    if world.get_block(x, y, z) != block::BLIGHTED_CROP {
        return None;
    }
    CropKind::from_index(world.meta_at(x, y, z))
}

const NEIGHBOURS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// Spread from the Blighted Crops in `blighted` (the pass's snapshot, so a crop
/// blighted this pass does not spread until the next one). Each same-kind farmed
/// crop beside one at the same height blights one time in [`SPREAD_ONE_IN`].
/// Bounded by [`MAX_SPREAD_PER_PASS`]. Returns the changes made.
pub fn spread(world: &mut World, tick: u64, blighted: Vec<(i32, i32, i32)>) -> Vec<BlockChange> {
    spread_with(world, tick, blighted, spread_roll)
}

/// [`spread`] with the dice passed in, so a test can make every roll land.
fn spread_with(
    world: &mut World,
    tick: u64,
    mut blighted: Vec<(i32, i32, i32)>,
    dice: impl Fn(i32, i32, i32, u64, u8) -> bool,
) -> Vec<BlockChange> {
    let mut changes = Vec::new();
    if blighted.is_empty() {
        return changes;
    }
    // The scan order follows chunk iteration; sort so the cap and the window
    // are the same on every machine for the same world.
    blighted.sort_unstable();
    blighted.dedup();
    let (start, take) = window(blighted.len(), tick);
    let n = blighted.len();
    for i in 0..take {
        let (x, y, z) = blighted[(start + i) % n];
        let Some(kind) = kind_remembered(world, x, y, z) else { continue };
        for (dir, (dx, dz)) in NEIGHBOURS.iter().enumerate() {
            let (nx, nz) = (x + dx, z + dz);
            let b = world.get_block(nx, y, nz);
            if crop_kind(world, nx, y, nz, b) != Some(kind) {
                continue;
            }
            if dice(x, y, z, tick, dir as u8) {
                changes.push(blight_cell(world, nx, y, nz, kind));
            }
        }
    }
    changes
}

/// Which of `n` sorted Blighted Crops a pass spreads from: `(start, take)`, the
/// indices `start..start + take` taken modulo `n`. Everything when `n` is within
/// the cap; otherwise a cap-sized window that moves on with the pass (one step
/// per 100 ticks, the growth cadence), so a big field is covered over a few
/// passes and no part of it starves.
fn window(n: usize, tick: u64) -> (usize, usize) {
    let take = n.min(MAX_SPREAD_PER_PASS);
    let start = if n > take { ((tick / 100) as usize).wrapping_mul(take) % n } else { 0 };
    (start, take)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::growth;

    /// A tick on the growth cadence (every 100 ticks; 200 for a dry crop).
    const PASS: u64 = 200;

    /// A sunlit field of `n` crops of `block_id` in one chunk column (chunk
    /// (0,0)), row by row, on tilled soil.
    fn field(world: &mut World, block_id: BlockId, n: i32, y: i32) -> Vec<(i32, i32, i32)> {
        let mut out = Vec::new();
        for i in 0..n {
            let (x, z) = (i % 16, i / 16);
            world.set_block(x, y - 1, z, block::TILLED_SOIL);
            world.set_block(x, y, z, block_id);
            world.set_sky_light_at(x, y, z, 15);
            out.push((x, y, z));
        }
        out
    }

    fn count(world: &World, b: BlockId, n: i32, y: i32) -> usize {
        (0..n).filter(|&i| world.get_block(i % 16, y, i / 16) == b).count()
    }

    /// Run `passes` growth passes (with blight on) and return how many Blighted
    /// Crops stand in the `n`-cell field afterwards. Crops regrow to wheat 0 each
    /// pass so the field stays "would advance" all the time.
    fn run_passes(world: &mut World, n: i32, y: i32, passes: u64, reset_to: BlockId) -> usize {
        for p in 1..=passes {
            let tick = PASS * p;
            for i in 0..n {
                let (x, z) = (i % 16, i / 16);
                let b = world.get_block(x, y, z);
                if b != block::BLIGHTED_CROP && growth::is_crop(b) {
                    // Back to stage 0 of its own kind, so it keeps "growing".
                    world.set_block(x, y, z, if b >= block::CARROT_STAGE_0 && b <= block::CARROT_STAGE_3 { block::CARROT_STAGE_0 } else { reset_to });
                }
            }
            growth::tick_growth(world, tick, true, 7, true);
        }
        count(world, block::BLIGHTED_CROP, n, y)
    }

    #[test]
    fn kinds_round_trip_through_the_meta_index() {
        for i in 0..CropKind::COUNT as u8 {
            let k = CropKind::from_index(i).unwrap();
            assert_eq!(k.index(), i);
        }
        assert_eq!(CropKind::from_index(CropKind::COUNT as u8), None);
    }

    #[test]
    fn every_crop_block_has_a_kind_and_nothing_else_does() {
        for id in 0..=block::BLIGHTED_CROP + 1 {
            assert_eq!(kind_of_block(id).is_some(), growth::is_crop(id), "block {id}");
        }
    }

    #[test]
    fn sixty_three_crops_of_one_kind_never_blight() {
        let mut w = World::new();
        let crops = field(&mut w, block::WHEAT_STAGE_0, 63, 70);
        assert!(find_monocultures(&w, &crops).is_empty(), "63 is below the floor");
        assert_eq!(run_passes(&mut w, 63, 70, 400, block::WHEAT_STAGE_0), 0);
    }

    #[test]
    fn sixty_four_crops_at_eighty_nine_percent_never_blight() {
        // 57 wheat + 7 carrots = 89.06% wheat.
        let mut w = World::new();
        let mut crops = field(&mut w, block::WHEAT_STAGE_0, 57, 70);
        for i in 57..64 {
            let (x, z) = (i % 16, i / 16);
            w.set_block(x, 69, z, block::TILLED_SOIL);
            w.set_block(x, 70, z, block::CARROT_STAGE_0);
            w.set_sky_light_at(x, 70, z, 15);
            crops.push((x, 70, z));
        }
        assert!(57 * 100 < 64 * MONO_SHARE_PCT, "the fixture is just under the share");
        assert!(find_monocultures(&w, &crops).is_empty());
        assert_eq!(run_passes(&mut w, 64, 70, 400, block::WHEAT_STAGE_0), 0);
    }

    #[test]
    fn ninety_percent_is_enough() {
        // 58 wheat + 6 carrots = 90.6%.
        let mut w = World::new();
        let mut crops = field(&mut w, block::WHEAT_STAGE_0, 58, 70);
        for i in 58..64 {
            let (x, z) = (i % 16, i / 16);
            w.set_block(x, 70, z, block::CARROT_STAGE_0);
            crops.push((x, 70, z));
        }
        assert_eq!(find_monocultures(&w, &crops).dominant_at(0, 0), Some(CropKind::Wheat));
    }

    #[test]
    fn sixty_four_of_one_kind_do_blight_over_enough_passes() {
        let mut w = World::new();
        let crops = field(&mut w, block::WHEAT_STAGE_0, 64, 70);
        assert_eq!(find_monocultures(&w, &crops).dominant_at(3, 3), Some(CropKind::Wheat));
        let blighted = run_passes(&mut w, 64, 70, 60, block::WHEAT_STAGE_0);
        assert!(blighted > 0, "1 in 40 over 60 passes of up to 64 crops");
        // Every blight remembers its kind and is plain ground to the owner of the cell.
        for i in 0..64 {
            let (x, z) = (i % 16, i / 16);
            if w.get_block(x, 70, z) == block::BLIGHTED_CROP {
                assert_eq!(kind_remembered(&w, x, 70, z), Some(CropKind::Wheat));
                assert!(!w.is_placed(x, 70, z));
            }
        }
    }

    #[test]
    fn a_mixed_field_does_not_blight() {
        let mut w = World::new();
        let kinds = [block::WHEAT_STAGE_0, block::CARROT_STAGE_0, block::POTATO_STAGE_0, block::CORN_STAGE_0];
        for i in 0..128 {
            let (x, z) = (i % 16, i / 16);
            w.set_block(x, 69, z, block::TILLED_SOIL);
            w.set_block(x, 70, z, kinds[(i % 4) as usize]);
            w.set_sky_light_at(x, 70, z, 15);
        }
        let crops = growth::collect_crop_positions(&w);
        assert_eq!(crops.len(), 128);
        assert!(find_monocultures(&w, &crops).is_empty());
        for p in 1..=200u64 {
            growth::tick_growth(&mut w, PASS * p, true, 7, true);
        }
        assert_eq!(growth::collect_crop_positions(&w).len(), 128, "nothing blighted");
    }

    #[test]
    fn a_wild_meadow_is_not_a_field() {
        // Mature cornflowers on grass are wild; only tilled soil makes one a crop.
        let mut w = World::new();
        for i in 0..100 {
            let (x, z) = (i % 16, i / 16);
            w.set_block(x, 69, z, block::GRASS);
            w.set_block(x, 70, z, block::CORNFLOWER);
        }
        let crops = growth::collect_crop_positions(&w);
        assert_eq!(crops.len(), 100);
        assert!(find_monocultures(&w, &crops).is_empty(), "wild flowers are not counted");
        // Farmed on tilled soil they are.
        for i in 0..100 {
            let (x, z) = (i % 16, i / 16);
            w.set_block(x, 69, z, block::TILLED_SOIL);
        }
        assert_eq!(find_monocultures(&w, &crops).dominant_at(0, 0), Some(CropKind::Cornflower));
    }

    #[test]
    fn the_same_world_and_tick_blight_the_same_cells() {
        let build = || {
            let mut w = World::new();
            field(&mut w, block::CARROT_STAGE_0, 100, 70);
            w
        };
        let (mut a, mut b) = (build(), build());
        let order_a = growth::collect_crop_positions(&a);
        let mut order_b = growth::collect_crop_positions(&b);
        order_b.reverse();
        let ma = find_monocultures(&a, &order_a);
        let mb = find_monocultures(&b, &order_b);
        assert_eq!(ma, mb, "the scan order does not matter");
        for p in 1..=30u64 {
            let mut ca = growth::advance_crops_blight(&mut a, PASS * p, true, order_a.clone(), Some(&ma));
            let mut cb = growth::advance_crops_blight(&mut b, PASS * p, true, order_b.clone(), Some(&mb));
            ca.sort_by_key(|c| (c.x, c.y, c.z));
            cb.sort_by_key(|c| (c.x, c.y, c.z));
            assert_eq!(ca, cb, "pass {p}");
        }
    }

    #[test]
    fn a_blight_change_names_the_block_and_its_kind() {
        let mut w = World::new();
        w.set_block(4, 70, 4, block::HEMP_STAGE_1);
        let c = blight_cell(&mut w, 4, 70, 4, CropKind::Hemp);
        assert_eq!((c.new_block, c.meta), (block::BLIGHTED_CROP, CropKind::Hemp.index()));
        assert_eq!(w.get_block(4, 70, 4), block::BLIGHTED_CROP);
        assert_eq!(w.meta_at(4, 70, 4), CropKind::Hemp.index());
    }

    #[test]
    fn a_planted_crop_that_blights_is_no_longer_placed() {
        let mut w = World::new();
        w.place_player_block(2, 70, 2, block::WHEAT_STAGE_1);
        assert!(w.is_placed(2, 70, 2));
        blight_cell(&mut w, 2, 70, 2, CropKind::Wheat);
        assert!(!w.is_placed(2, 70, 2), "bloom must see bare ground, not built ground");
    }

    /// A row of wheat with one blight in the middle and carrots either side of it.
    fn spread_world() -> World {
        let mut w = World::new();
        for x in 0..10 {
            w.set_block(x, 69, 0, block::TILLED_SOIL);
            w.set_block(x, 70, 0, block::WHEAT_STAGE_2);
        }
        w.set_block(5, 70, 1, block::CARROT_STAGE_2);
        w.set_block(5, 70, -1, block::CARROT_STAGE_2);
        blight_cell(&mut w, 5, 70, 0, CropKind::Wheat);
        w
    }

    #[test]
    fn spread_reaches_same_kind_neighbours_only() {
        let mut w = spread_world();
        let mut reached = Vec::new();
        for p in 1..=400u64 {
            for c in spread(&mut w, PASS * p, vec![(5, 70, 0)]) {
                assert_eq!((c.new_block, c.meta), (block::BLIGHTED_CROP, CropKind::Wheat.index()));
                reached.push((c.x, c.y, c.z));
            }
        }
        reached.sort_unstable();
        assert_eq!(reached, vec![(4, 70, 0), (6, 70, 0)], "the wheat beside it, once each, nothing else");
        assert_eq!(w.get_block(5, 70, 1), block::CARROT_STAGE_2, "another kind is never reached");
        assert_eq!(w.get_block(5, 70, -1), block::CARROT_STAGE_2);
    }

    #[test]
    fn spread_ignores_the_regions_mix_and_height() {
        // Two wheat among nothing else: not a monoculture, but a blight beside it spreads.
        let mut w = World::new();
        w.set_block(0, 70, 0, block::WHEAT_STAGE_3);
        w.set_block(1, 70, 0, block::WHEAT_STAGE_3);
        w.set_block(1, 71, 0, block::WHEAT_STAGE_3); // one up: not a neighbour at the same y
        blight_cell(&mut w, 0, 70, 0, CropKind::Wheat);
        for p in 1..=400u64 {
            spread(&mut w, PASS * p, vec![(0, 70, 0)]);
        }
        assert_eq!(w.get_block(1, 70, 0), block::BLIGHTED_CROP);
        assert_eq!(w.get_block(1, 71, 0), block::WHEAT_STAGE_3);
    }

    #[test]
    fn a_wild_flower_beside_a_blight_is_left_alone() {
        let mut w = World::new();
        w.set_block(1, 69, 0, block::GRASS);
        w.set_block(1, 70, 0, block::CORNFLOWER);
        blight_cell(&mut w, 0, 70, 0, CropKind::Cornflower);
        for p in 1..=400u64 {
            spread(&mut w, PASS * p, vec![(0, 70, 0)]);
        }
        assert_eq!(w.get_block(1, 70, 0), block::CORNFLOWER);
    }

    #[test]
    fn a_crop_blighted_this_pass_does_not_spread_until_the_next() {
        // Three in a row, the dice always landing: the snapshot holds one source.
        let mut w = World::new();
        for x in 0..3 {
            w.set_block(x, 70, 0, block::WHEAT_STAGE_3);
        }
        blight_cell(&mut w, 0, 70, 0, CropKind::Wheat);
        let ch = spread_with(&mut w, PASS, vec![(0, 70, 0)], |_, _, _, _, _| true);
        assert_eq!(ch.len(), 1);
        assert_eq!(w.get_block(2, 70, 0), block::WHEAT_STAGE_3, "two cells away needs a second pass");
    }

    #[test]
    fn spread_respects_the_per_pass_cap() {
        // More Blighted Crops than the cap, each with exactly one wheat beside it
        // (to its east) and the dice always landing: a pass blights exactly the cap.
        let n = MAX_SPREAD_PER_PASS as i32 + 300;
        let mut w = World::new();
        let mut sources = Vec::new();
        for i in 0..n {
            let x = i * 3; // blight at x, wheat at x + 1, a gap at x + 2
            w.set_block(x + 1, 70, 0, block::WHEAT_STAGE_1);
            blight_cell(&mut w, x, 70, 0, CropKind::Wheat);
            sources.push((x, 70, 0));
        }
        let ch = spread_with(&mut w, PASS, sources.clone(), |_, _, _, _, _| true);
        assert_eq!(ch.len(), MAX_SPREAD_PER_PASS, "the cap, not all {n}");
    }

    #[test]
    fn the_cap_window_moves_so_no_blight_starves() {
        let n = MAX_SPREAD_PER_PASS + 300;
        let mut seen = vec![false; n];
        for p in 1..=20u64 {
            let (start, take) = window(n, PASS * p);
            assert_eq!(take, MAX_SPREAD_PER_PASS);
            for i in 0..take {
                seen[(start + i) % n] = true;
            }
        }
        assert!(seen.iter().all(|&s| s), "every index is taken within a few passes");
        assert_eq!(window(10, PASS), (0, 10), "within the cap: everything");
    }

    #[test]
    fn blight_is_off_in_creative_and_in_a_trial() {
        let mut w = World::new();
        assert!(allowed(PlayMode::Survival, &w));
        assert!(!allowed(PlayMode::Creative, &w));
        w.trial_running = true;
        assert!(!allowed(PlayMode::Survival, &w));
    }

    /// 64 wheat in one column, every pass on the cadence, a generous number of
    /// passes: the standing field blights when allowed and not when it is not.
    fn blighted_after(passes: u64, allowed: bool) -> usize {
        let mut w = World::new();
        field(&mut w, block::WHEAT_STAGE_0, 64, 70);
        let mut total = 0;
        for p in 1..=passes {
            for i in 0..64 {
                let (x, z) = (i % 16, i / 16);
                let b = w.get_block(x, 70, z);
                if b != block::BLIGHTED_CROP && growth::is_crop(b) {
                    w.set_block(x, 70, z, block::WHEAT_STAGE_0);
                }
            }
            total += growth::tick_growth(&mut w, PASS * p, true, 7, allowed)
                .crop_changes
                .iter()
                .filter(|c| c.new_block == block::BLIGHTED_CROP)
                .count();
        }
        total
    }

    #[test]
    fn the_growth_pass_blights_only_when_the_flag_is_on() {
        assert!(blighted_after(80, true) > 0);
        assert_eq!(blighted_after(80, false), 0);
    }
}
