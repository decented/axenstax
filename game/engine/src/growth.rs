//! Crop growth — per-tick advancement of planted crop blocks.
//!
//! Pure free functions matching the `falling_blocks::tick_falling_blocks`
//! and `spawning::tick_mob_spawning` patterns so server + single-player
//! paths share one implementation and the multiplayer broadcast layer
//! re-uses the returned [`BlockChange`] vec.
//!
//! Per Spec 16 / `2026-05-14-farming-system.md` Phase 6:
//! - Base interval: 200 ticks per stage (10 s at 20 TPS).
//! - Water within 4 blocks horizontally → halved interval (100 ticks).
//! - Light gate: dropped for Tier 1 per spec ("if light propagation isn't
//!   viable yet, drop the gate"). Re-add when the engine's block-light
//!   propagation is mature.
//! - Mature stage (3) never advances further.

use crate::block::{
    self, BlockId,
    WHEAT_STAGE_0, WHEAT_STAGE_1, WHEAT_STAGE_2, WHEAT_STAGE_3,
    CARROT_STAGE_0, CARROT_STAGE_1, CARROT_STAGE_2, CARROT_STAGE_3,
    POTATO_STAGE_0, POTATO_STAGE_1, POTATO_STAGE_2, POTATO_STAGE_3,
    CORN_STAGE_0, CORN_STAGE_1, CORN_STAGE_2, CORN_STAGE_3,
    PAPYRUS_STAGE_0, PAPYRUS_STAGE_1, PAPYRUS_STAGE_2, PAPYRUS_STAGE_3,
    COTTON_STAGE_0, COTTON_STAGE_1, COTTON_STAGE_2, COTTON_STAGE_3,
    HEMP_STAGE_0, HEMP_STAGE_1, HEMP_STAGE_2, HEMP_STAGE_3,
    CORNFLOWER, FIELD_POPPY, BUTTERCUP,
    CORNFLOWER_STAGE_0, CORNFLOWER_STAGE_1, CORNFLOWER_STAGE_2,
    FIELD_POPPY_STAGE_0, FIELD_POPPY_STAGE_1, FIELD_POPPY_STAGE_2,
    BUTTERCUP_STAGE_0, BUTTERCUP_STAGE_1, BUTTERCUP_STAGE_2,
};
use crate::protocol::BlockChange;
use crate::world::World;

/// Base interval (in 20-TPS ticks) between crop-stage advancements.
/// 200 ticks = 10 s = a single stage. Sprout → mature = 600 ticks ≈ 30 s.
pub const CROP_GROWTH_TICKS_PER_STAGE: u64 = 200;

/// Water-adjacency speed multiplier. A crop with water within
/// [`WATER_SEARCH_RADIUS`] blocks horizontally grows in
/// `CROP_GROWTH_TICKS_PER_STAGE / WATER_GROWTH_DIVISOR` ticks per stage.
pub const WATER_GROWTH_DIVISOR: u64 = 2;

/// Radius (in blocks) of the horizontal water search around a crop.
pub const WATER_SEARCH_RADIUS: i32 = 4;

/// If this block id is a crop, return the next stage's id (or `None` if
/// already mature). Centralises the 12-crop ladder.
pub fn next_stage(block: BlockId) -> Option<BlockId> {
    match block {
        WHEAT_STAGE_0 => Some(WHEAT_STAGE_1),
        WHEAT_STAGE_1 => Some(WHEAT_STAGE_2),
        WHEAT_STAGE_2 => Some(WHEAT_STAGE_3),
        CARROT_STAGE_0 => Some(CARROT_STAGE_1),
        CARROT_STAGE_1 => Some(CARROT_STAGE_2),
        CARROT_STAGE_2 => Some(CARROT_STAGE_3),
        POTATO_STAGE_0 => Some(POTATO_STAGE_1),
        POTATO_STAGE_1 => Some(POTATO_STAGE_2),
        POTATO_STAGE_2 => Some(POTATO_STAGE_3),
        CORN_STAGE_0 => Some(CORN_STAGE_1),
        CORN_STAGE_1 => Some(CORN_STAGE_2),
        CORN_STAGE_2 => Some(CORN_STAGE_3),
        // Spec 23 — Papyrus Reed. Same 4-stage ladder; stage 3 is
        // mature + harvestable. Water-adjacency boost from the crop
        // tick fires automatically (papyrus is water-adjacent at
        // placement time per `papyrus::is_valid_planting_base`).
        PAPYRUS_STAGE_0 => Some(PAPYRUS_STAGE_1),
        PAPYRUS_STAGE_1 => Some(PAPYRUS_STAGE_2),
        PAPYRUS_STAGE_2 => Some(PAPYRUS_STAGE_3),
        // Spec 36 Phase 2 — farmable fibre crops.
        COTTON_STAGE_0 => Some(COTTON_STAGE_1),
        COTTON_STAGE_1 => Some(COTTON_STAGE_2),
        COTTON_STAGE_2 => Some(COTTON_STAGE_3),
        HEMP_STAGE_0 => Some(HEMP_STAGE_1),
        HEMP_STAGE_1 => Some(HEMP_STAGE_2),
        HEMP_STAGE_2 => Some(HEMP_STAGE_3),
        // Spec 35 farmable-flower follow-on — the existing wild-flower
        // block (CORNFLOWER / FIELD_POPPY / BUTTERCUP at ids 131-133)
        // is the mature stage. Stages 0..2 are new; stage 3 is the
        // wild-form id so worldgen + crafting recipes that already
        // reference the mature block keep working.
        CORNFLOWER_STAGE_0 => Some(CORNFLOWER_STAGE_1),
        CORNFLOWER_STAGE_1 => Some(CORNFLOWER_STAGE_2),
        CORNFLOWER_STAGE_2 => Some(CORNFLOWER),
        FIELD_POPPY_STAGE_0 => Some(FIELD_POPPY_STAGE_1),
        FIELD_POPPY_STAGE_1 => Some(FIELD_POPPY_STAGE_2),
        FIELD_POPPY_STAGE_2 => Some(FIELD_POPPY),
        BUTTERCUP_STAGE_0 => Some(BUTTERCUP_STAGE_1),
        BUTTERCUP_STAGE_1 => Some(BUTTERCUP_STAGE_2),
        BUTTERCUP_STAGE_2 => Some(BUTTERCUP),
        _ => None,
    }
}

/// Whether a block id is any (immature or mature) crop block.
pub fn is_crop(block: BlockId) -> bool {
    matches!(
        block,
        WHEAT_STAGE_0 | WHEAT_STAGE_1 | WHEAT_STAGE_2 | WHEAT_STAGE_3
            | CARROT_STAGE_0 | CARROT_STAGE_1 | CARROT_STAGE_2 | CARROT_STAGE_3
            | POTATO_STAGE_0 | POTATO_STAGE_1 | POTATO_STAGE_2 | POTATO_STAGE_3
            | CORN_STAGE_0 | CORN_STAGE_1 | CORN_STAGE_2 | CORN_STAGE_3
            | PAPYRUS_STAGE_0 | PAPYRUS_STAGE_1 | PAPYRUS_STAGE_2 | PAPYRUS_STAGE_3
            | COTTON_STAGE_0 | COTTON_STAGE_1 | COTTON_STAGE_2 | COTTON_STAGE_3
            | HEMP_STAGE_0 | HEMP_STAGE_1 | HEMP_STAGE_2 | HEMP_STAGE_3
            // Spec 35 farmable-flower follow-on — stages 0..2 + the
            // wild-form mature block all count as crops so they tick
            // forward + drop via `crop_break`.
            | CORNFLOWER_STAGE_0 | CORNFLOWER_STAGE_1 | CORNFLOWER_STAGE_2 | CORNFLOWER
            | FIELD_POPPY_STAGE_0 | FIELD_POPPY_STAGE_1 | FIELD_POPPY_STAGE_2 | FIELD_POPPY
            | BUTTERCUP_STAGE_0 | BUTTERCUP_STAGE_1 | BUTTERCUP_STAGE_2 | BUTTERCUP
    )
}

/// Bonemeal advance (Minecraft parity). Bonemeal is the strong, recognisable
/// crop accelerator: a single application bumps the crop **1–2 stages** (seeded
/// jitter), versus the Spec-37 Fertiliser's flat +1. Returns the new block id,
/// or `None` if `block` isn't a crop or is already mature (so callers know not
/// to consume the bonemeal — no waste on a finished crop).
///
/// Pure + deterministic given `seed` (typically `tick ^ position-hash`) so the
/// outcome is testable and replay-stable.
pub fn bonemeal_advance(block: BlockId, seed: u64) -> Option<BlockId> {
    // First stage always applies (this is what makes a young crop jump).
    let mut next = next_stage(block)?;
    // ~40% of the time, a second stage lands too — the "bonemeal feels strong"
    // moment, while still leaving headroom so it isn't a guaranteed instant-grow.
    if seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) % 5 < 2
        && let Some(n) = next_stage(next) {
            next = n;
        }
    Some(next)
}

/// Scan the WATER_SEARCH_RADIUS-wide horizontal disk around `(x, y, z)`
/// for `block::WATER`. Returns true if any tile within radius (Chebyshev
/// distance) is water. Matches Minecraft's "any water within 4 blocks"
/// crop-growth boost.
fn water_within_radius(world: &World, x: i32, y: i32, z: i32) -> bool {
    for dz in -WATER_SEARCH_RADIUS..=WATER_SEARCH_RADIUS {
        for dx in -WATER_SEARCH_RADIUS..=WATER_SEARCH_RADIUS {
            if dx == 0 && dz == 0 {
                continue;
            }
            if world.get_block(x + dx, y, z + dz) == block::WATER {
                return true;
            }
            // Also check the row directly below — irrigation channels
            // often sit one block below the cropped tile.
            if world.get_block(x + dx, y - 1, z + dz) == block::WATER {
                return true;
            }
        }
    }
    false
}

/// Try to advance every crop block in `crops` according to the current
/// tick + per-position water-adjacency multiplier. Returns the list of
/// block changes that actually fired this call.
///
/// `crops` is the caller-provided iterator over candidate crop positions
/// — typically derived from the loaded chunks at the cost of an iteration
/// over chunk blocks. The function itself is pure: deterministic given
/// the world state + tick counter.
///
/// `tick` is the engine-wide monotonic tick counter. The per-stage
/// timing rule fires on every multiple of [`CROP_GROWTH_TICKS_PER_STAGE`]
/// (or its water-accelerated counterpart). This means **all crops on
/// the same growth-rate tick advance together** — a deliberate
/// simplification that avoids per-block growth-timer storage in the
/// chunk format.
/// Whether [`advance_crops`] would advance any crop this tick. Callers gate the
/// expensive full-world `collect_crop_positions` scan on this so they don't scan
/// ~39 of every 40 ticks for nothing (engine audit 2026-06-04, B). The growth
/// cadence fires on the per-stage interval (dry crops) and the faster watered
/// interval; off-cadence ticks do no work.
pub fn crops_should_advance(tick: u64) -> bool {
    if tick == 0 {
        return false;
    }
    tick.is_multiple_of(CROP_GROWTH_TICKS_PER_STAGE)
        || tick.is_multiple_of(CROP_GROWTH_TICKS_PER_STAGE / WATER_GROWTH_DIVISOR)
}

/// Plain growth, no blight: the tests' entry point (the pass itself runs
/// [`advance_crops_blight`]).
#[cfg(test)]
pub fn advance_crops<I>(
    world: &mut World,
    tick: u64,
    raining: bool,
    crops: I,
) -> Vec<BlockChange>
where
    I: IntoIterator<Item = (i32, i32, i32)>,
{
    advance_crops_blight(world, tick, raining, crops, None)
}

/// [`advance_crops`] with blight (living world Tier 2 S8): a crop that would
/// advance and stands in a monoculture of its own kind (`blight`, from
/// [`crate::blight::find_monocultures`]) rolls [`crate::blight::blight_roll`] and,
/// on a hit, becomes a Blighted Crop of its kind instead of advancing. `None`
/// (creative, a trial, a caller that does not want it) is plain growth.
pub fn advance_crops_blight<I>(
    world: &mut World,
    tick: u64,
    raining: bool,
    crops: I,
    blight: Option<&crate::blight::Monocultures>,
) -> Vec<BlockChange>
where
    I: IntoIterator<Item = (i32, i32, i32)>,
{
    let mut changes = Vec::new();
    if !crops_should_advance(tick) {
        return changes;
    }
    // Per-crop rate gate: watered crops advance on the faster cadence, dry crops
    // on the per-stage cadence. (tick > 0 already guaranteed above.)
    let base_fire = tick.is_multiple_of(CROP_GROWTH_TICKS_PER_STAGE);
    let fast_fire = tick.is_multiple_of(CROP_GROWTH_TICKS_PER_STAGE / WATER_GROWTH_DIVISOR);

    for (x, y, z) in crops {
        let blk = world.get_block(x, y, z);
        let Some(next) = next_stage(blk) else { continue };
        // Spec 30 — crops require effective_light ≥ 9 to advance.
        // Closes the loop on Spec 5 §3.10's "light gate dropped for
        // Tier 1" callout that Spec 30 lifts. Sky-lit crops grow
        // naturally during the day; underground/under-roof crops need
        // a torch nearby. Effective-light uses max(block, sky-4) so
        // overnight crops sky-light is exactly 11 → still ≥ 9 → growth
        // continues through the night (good kid-game ergonomics).
        if world.effective_light_at(x, y, z) < 9 {
            continue;
        }
        // P8 — rain waters every crop (the light gate above still applies, so
        // dark indoor crops don't grow just because it's raining outside).
        let has_water = raining || water_within_radius(world, x, y, z);
        let advance = if has_water { fast_fire } else { base_fire };
        if advance {
            if let Some(m) = blight
                && let Some(kind) = crate::blight::kind_of_block(blk)
                && m.dominant_at(x, z) == Some(kind)
                && crate::blight::blight_roll(x, y, z, tick)
            {
                changes.push(crate::blight::blight_cell(world, x, y, z, kind));
                continue;
            }
            world.set_block(x, y, z, next);
            changes.push(BlockChange { x, y, z, new_block: next, meta: 0 });
        }
    }
    changes
}

/// What a broken crop block leaves behind + drops. Used by the
/// survival break path (`game_loop.rs`).
///
/// Per Spec 16 / Phase 7:
/// - Mature wheat (`WHEAT_STAGE_3`): 1 wheat + 1-3 seeds; reset to
///   tilled soil so the player can re-plant immediately.
/// - Mature carrot (`CARROT_STAGE_3`): 1-4 carrots (no separate seed —
///   the carrot doubles as both); reset to tilled soil.
/// - Mature potato (`POTATO_STAGE_3`): 1-4 potatoes; reset to tilled soil.
/// - Immature crop (stages 0..=2): no drops; block becomes air (no
///   tilled-soil revert — player must till again, mirroring Minecraft
///   "wasted-crop" feel).
/// - Anything else: returns `None` (caller falls through to the
///   standard break path).
///
/// `rng_seed` is mixed into the drop-count roll so the same crop at the
/// same position with the same world-secret produces a deterministic
/// drop on every replay. Use the player's current tick + position as a
/// per-call seed.
pub struct CropBreakResult {
    pub replacement: BlockId,
    pub drops: Vec<crate::item::ItemStack>,
}

/// `on_tilled` — whether the block beneath the crop is TILLED_SOIL. Wild flowers
/// growing on grass and farmed flowers planted on tilled soil share the same
/// mature block id, so the caller derives this from the world (the block at
/// `y-1`). Field crops (wheat/carrot/…) always reset to tilled soil regardless;
/// only the wild-or-farmed flowers branch on it (#16).
pub fn crop_break(blk: BlockId, rng_seed: u64, on_tilled: bool) -> Option<CropBreakResult> {
    use crate::item::{ItemStack, MaterialId};
    // Drop-count roll. Range is inclusive on both ends.
    fn roll(seed: u64, min: u8, max: u8) -> u8 {
        let span = (max - min + 1) as u64;
        min + ((seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)) % span) as u8
    }

    match blk {
        WHEAT_STAGE_3 => Some(CropBreakResult {
            replacement: block::TILLED_SOIL,
            drops: vec![
                ItemStack::new_material(MaterialId::Wheat, 1),
                ItemStack::new_material(MaterialId::WheatSeeds, roll(rng_seed, 1, 3)),
            ],
        }),
        CARROT_STAGE_3 => Some(CropBreakResult {
            replacement: block::TILLED_SOIL,
            drops: vec![ItemStack::new_material(MaterialId::Carrot, roll(rng_seed, 1, 4))],
        }),
        POTATO_STAGE_3 => Some(CropBreakResult {
            replacement: block::TILLED_SOIL,
            drops: vec![ItemStack::new_material(MaterialId::Potato, roll(rng_seed, 1, 4))],
        }),
        // Corn (Wave 28) — mature drops 1 ear + 1-3 seeds (wheat-style
        // seed-on-harvest so the player can replant without grinding for
        // seeds the same way they would for wheat).
        CORN_STAGE_3 => Some(CropBreakResult {
            replacement: block::TILLED_SOIL,
            drops: vec![
                ItemStack::new_material(MaterialId::Corn, 1),
                ItemStack::new_material(MaterialId::CornSeeds, roll(rng_seed, 1, 3)),
            ],
        }),
        // Spec 23 — Papyrus Reed. Sugarcane-style auto-regrow: the root
        // stays planted, the top resets to stage 0. Drops 1-2 reeds.
        // Distinct from the field crops (which break to TILLED_SOIL +
        // require explicit replanting) — papyrus is "wild" so the
        // riverbank-harvest loop feels loose. See spec §"Why
        // sugarcane-style regrowth".
        PAPYRUS_STAGE_3 => Some(CropBreakResult {
            replacement: block::PAPYRUS_STAGE_0,
            drops: vec![ItemStack::new_material(MaterialId::PapyrusReed, roll(rng_seed, 1, 2))],
        }),
        // Spec 36 Phase 2 — mature fibre crops drop fibre + 1-2 seeds
        // (wheat-style self-sustaining), reset to tilled soil.
        COTTON_STAGE_3 => Some(CropBreakResult {
            replacement: block::TILLED_SOIL,
            drops: vec![
                ItemStack::new_material(MaterialId::Cotton, 1),
                ItemStack::new_material(MaterialId::CottonSeeds, roll(rng_seed, 1, 2)),
            ],
        }),
        HEMP_STAGE_3 => Some(CropBreakResult {
            replacement: block::TILLED_SOIL,
            drops: vec![
                ItemStack::new_material(MaterialId::HempFibre, 1),
                ItemStack::new_material(MaterialId::HempSeeds, roll(rng_seed, 1, 2)),
            ],
        }),
        // Spec 35 farmable-flower follow-on (2026-05-28) — a mature flower
        // drops the flower-block itself + 1-2 matching seeds. Wild flowers and
        // farmed flowers share the same mature block id, so we branch on
        // `on_tilled` (the block beneath): farmed (on tilled soil) resets to
        // TILLED_SOIL for immediate replant; a wild flower on grass leaves AIR
        // so the grass underneath stays a clean meadow instead of a stray
        // tilled-soil scar (#16). `place_vegetation` refuses to place over
        // tilled soil, so the farmed reset never gets re-flowered by world-gen.
        CORNFLOWER => Some(CropBreakResult {
            replacement: if on_tilled { block::TILLED_SOIL } else { block::AIR },
            drops: vec![
                ItemStack::new_block(block::CORNFLOWER, 1),
                ItemStack::new_material(MaterialId::CornflowerSeeds, roll(rng_seed, 1, 2)),
            ],
        }),
        FIELD_POPPY => Some(CropBreakResult {
            replacement: if on_tilled { block::TILLED_SOIL } else { block::AIR },
            drops: vec![
                ItemStack::new_block(block::FIELD_POPPY, 1),
                ItemStack::new_material(MaterialId::FieldPoppySeeds, roll(rng_seed, 1, 2)),
            ],
        }),
        BUTTERCUP => Some(CropBreakResult {
            replacement: if on_tilled { block::TILLED_SOIL } else { block::AIR },
            drops: vec![
                ItemStack::new_block(block::BUTTERCUP, 1),
                ItemStack::new_material(MaterialId::ButtercupSeeds, roll(rng_seed, 1, 2)),
            ],
        }),
        // Immature crops drop nothing — wasted growth.
        WHEAT_STAGE_0 | WHEAT_STAGE_1 | WHEAT_STAGE_2
        | CARROT_STAGE_0 | CARROT_STAGE_1 | CARROT_STAGE_2
        | POTATO_STAGE_0 | POTATO_STAGE_1 | POTATO_STAGE_2
        | CORN_STAGE_0 | CORN_STAGE_1 | CORN_STAGE_2
        | COTTON_STAGE_0 | COTTON_STAGE_1 | COTTON_STAGE_2
        | HEMP_STAGE_0 | HEMP_STAGE_1 | HEMP_STAGE_2
        // Spec 35 farmable-flower — half-grown flowers drop nothing.
        | CORNFLOWER_STAGE_0 | CORNFLOWER_STAGE_1 | CORNFLOWER_STAGE_2
        | FIELD_POPPY_STAGE_0 | FIELD_POPPY_STAGE_1 | FIELD_POPPY_STAGE_2
        | BUTTERCUP_STAGE_0 | BUTTERCUP_STAGE_1 | BUTTERCUP_STAGE_2
        // Immature papyrus also drops nothing; clear-cutting a
        // half-grown reed costs the root — no auto-regrow penalty.
        | PAPYRUS_STAGE_0 | PAPYRUS_STAGE_1 | PAPYRUS_STAGE_2 => Some(CropBreakResult {
            replacement: block::AIR,
            drops: Vec::new(),
        }),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: how many chunks the crop scan and the sapling scan walked cell
    /// by cell on this thread (a chunk the counters rule out is not walked).
    static CROP_CHUNKS_WALKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SAPLING_CHUNKS_WALKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Collect every crop position in the currently-loaded chunks. Helper for
/// the engine's per-tick growth pass.
#[cfg(test)]
pub fn collect_crop_positions(world: &World) -> Vec<(i32, i32, i32)> {
    scan_crops(world).0
}

/// One scan of the loaded chunks for the farming pass: every crop position, and
/// every Blighted Crop (Tier 2 S8). One walk, not two: the scan is O(loaded
/// volume).
fn scan_crops(world: &World) -> (Vec<(i32, i32, i32)>, Vec<(i32, i32, i32)>) {
    let mut crops = Vec::new();
    let mut blighted = Vec::new();
    // Only the chunks whose maintained counters say they hold a crop or a
    // blighted crop: the rest are not read cell by cell.
    for ((cx, cy, cz), chunk) in world.chunks_with_crops() {
        #[cfg(test)]
        CROP_CHUNKS_WALKED.with(|c| c.set(c.get() + 1));
        for ly in 0..crate::chunk::CHUNK_SIZE {
            for lz in 0..crate::chunk::CHUNK_SIZE {
                for lx in 0..crate::chunk::CHUNK_SIZE {
                    let blk = chunk.get(lx, ly, lz);
                    let is_blight = blk == block::BLIGHTED_CROP;
                    if is_blight || is_crop(blk) {
                        let wx = cx * crate::chunk::CHUNK_SIZE as i32 + lx as i32;
                        let wy = cy * crate::chunk::CHUNK_SIZE as i32 + ly as i32;
                        let wz = cz * crate::chunk::CHUNK_SIZE as i32 + lz as i32;
                        if is_blight {
                            blighted.push((wx, wy, wz));
                        } else {
                            crops.push((wx, wy, wz));
                        }
                    }
                }
            }
        }
    }
    (crops, blighted)
}

/// One-in-N chance per growth scan for a planted sapling to become a tree
/// (~10 s per scan on the base cadence → ~5 min average).
const SAPLING_GROW_ONE_IN: u64 = 30;

/// Saplings roll to grow only on the base cadence (every 200 ticks), not on the
/// faster watered-crop passes: [`advance_saplings`] is inert on any other tick,
/// so [`tick_growth`] does not even collect them then.
fn saplings_should_advance(tick: u64) -> bool {
    tick != 0 && tick.is_multiple_of(CROP_GROWTH_TICKS_PER_STAGE)
}

/// Scan loaded chunks for planted sapling blocks (2026-07-04).
pub fn collect_sapling_positions(world: &World) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for ((cx, cy, cz), chunk) in world.chunks_with_saplings() {
        #[cfg(test)]
        SAPLING_CHUNKS_WALKED.with(|c| c.set(c.get() + 1));
        for ly in 0..crate::chunk::CHUNK_SIZE {
            for lz in 0..crate::chunk::CHUNK_SIZE {
                for lx in 0..crate::chunk::CHUNK_SIZE {
                    if crate::block::is_sapling_block(chunk.get(lx, ly, lz)) {
                        out.push((
                            cx * crate::chunk::CHUNK_SIZE as i32 + lx as i32,
                            cy * crate::chunk::CHUNK_SIZE as i32 + ly as i32,
                            cz * crate::chunk::CHUNK_SIZE as i32 + lz as i32,
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Advance planted saplings (2026-07-04): on the growth cadence, each sapling
/// rolls a deterministic position+tick hash; a hit replaces it with its
/// species' REAL worldgen tree (`tree_shapes::place_tree` — same shapes, same
/// canopy rules). Light ≥ 9 gate matches crops. Tree blocks are only written
/// into AIR (and the sapling's own cell), so a house next to the sapling
/// never gets overwritten. Returns every mutated cell for re-mesh/broadcast.
pub fn advance_saplings(
    world: &mut World,
    tick: u64,
    seed: u32,
    saplings: Vec<(i32, i32, i32)>,
) -> Vec<(i32, i32, i32)> {
    let mut dirty = Vec::new();
    if !saplings_should_advance(tick) {
        return dirty;
    }
    for (x, y, z) in saplings {
        let blk = world.get_block(x, y, z);
        let Some(species) = crate::block::species_for_sapling(blk) else { continue };
        if world.effective_light_at(x, y, z) < 9 {
            continue;
        }
        // Full avalanche mix — a plain xor of position and tick keeps the
        // tick's parity (cadence ticks are always even) and can make the
        // modulo unreachable. Murmur-style finalizer avoids that trap.
        let mut h = (x as u64 & 0xFFFF)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((z as u64 & 0xFFFF) << 20)
            .wrapping_add(y as u64)
            ^ tick.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 33;
        h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        h ^= h >> 33;
        if !h.is_multiple_of(SAPLING_GROW_ONE_IN) {
            continue;
        }
        // Grow: the sapling sits at surface+1, so place_tree's blocks
        // (surface + 1 + dy) land exactly on the sapling column.
        world.set_block(x, y, z, crate::block::AIR);
        dirty.push((x, y, z));
        for tb in crate::tree_shapes::place_tree(species, x, z, seed) {
            let (bx, by, bz) = (x + tb.dx, y + tb.dy, z + tb.dz);
            // Never into a column that is not present (Phase B1 review): a
            // canopy block there conjured a chunk its generation then skipped.
            if world.is_column_present_at(bx, bz) && world.get_block(bx, by, bz) == crate::block::AIR {
                world.set_block(bx, by, bz, tb.id);
                dirty.push((bx, by, bz));
            }
        }
    }
    dirty
}

/// What one [`tick_growth`] pass changed.
#[derive(Debug, Default)]
pub struct GrowthTick {
    /// Crops that advanced a stage (already applied to the world).
    pub crop_changes: Vec<BlockChange>,
    /// Every cell a growing sapling touched (the sapling cell + its new tree).
    /// Raw cells, not `BlockChange`s: the caller reads the settled block (and
    /// its meta) off the world when it broadcasts.
    pub grown_cells: Vec<(i32, i32, i32)>,
}

/// The farming growth pass (Spec 16 crops + 2026-07-04 saplings), shared by the
/// client loop and the dedicated server (T1-3, 2026-10-05) so neither forks the
/// cadence or the scan. Hoists the [`crops_should_advance`] gate BEFORE the
/// scans: growth only fires on ~1 tick in 40 (engine audit 2026-06-04, B). The
/// scans read cell by cell only the chunks whose maintained counters (`Chunk`:
/// crops, blighted crops, saplings) say they hold something, and the sapling
/// scan runs only on the 200-tick passes that advance saplings. Deterministic: crops advance on the tick cadence + light gate,
/// saplings on a position+tick hash — no RNG state to share.
///
/// `blight` (living world Tier 2 S8, `blight::allowed`: not creative, not in a
/// trial) turns on the monoculture roll and the spread. Blight is judged on the
/// pass's own scan, so a crop blighted this pass spreads from the next one.
pub fn tick_growth(world: &mut World, tick: u64, raining: bool, seed: u32, blight: bool) -> GrowthTick {
    if !crops_should_advance(tick) {
        return GrowthTick::default();
    }
    let (crop_positions, blighted) = scan_crops(world);
    let monocultures = blight.then(|| crate::blight::find_monocultures(world, &crop_positions));
    let mut crop_changes = advance_crops_blight(world, tick, raining, crop_positions, monocultures.as_ref());
    if blight {
        crop_changes.extend(crate::blight::spread(world, tick, blighted));
    }
    // `advance_saplings` acts only on the base cadence (every 200 ticks), so the
    // 100-tick watered passes skip the sapling scan.
    let saplings = if saplings_should_advance(tick) { collect_sapling_positions(world) } else { Vec::new() };
    let grown_cells = advance_saplings(world, tick, seed, saplings);
    GrowthTick { crop_changes, grown_cells }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walked() -> (usize, usize) {
        (CROP_CHUNKS_WALKED.with(|c| c.get()), SAPLING_CHUNKS_WALKED.with(|c| c.get()))
    }

    fn reset_walked() {
        CROP_CHUNKS_WALKED.with(|c| c.set(0));
        SAPLING_CHUNKS_WALKED.with(|c| c.set(0));
    }

    /// Lots of ground, and the farming blocks in a few chunks only:
    /// crops in two, a blighted crop in one, saplings in two (one shared with a
    /// crop), over negative and positive coordinates.
    fn scattered_farm_world() -> World {
        let mut w = World::new();
        // 40 chunks of plain stone that hold nothing the scans look for.
        for cx in -5..5 {
            for cz in -2..2 {
                w.set_block(cx * 16 + 3, 64, cz * 16 + 3, block::STONE);
            }
        }
        let mut lit = |w: &mut World, x: i32, y: i32, z: i32, id: BlockId| {
            w.set_block(x, y, z, id);
            w.set_sky_light_at(x, y, z, 15);
        };
        // Crops: chunk (0,4,0) and chunk (-1,4,-1).
        for i in 0..9 {
            lit(&mut w, i, 64, 0, WHEAT_STAGE_0);
            lit(&mut w, i, 64, 2, CARROT_STAGE_1);
        }
        lit(&mut w, -3, 64, -4, POTATO_STAGE_2);
        lit(&mut w, -4, 65, -5, PAPYRUS_STAGE_0);
        // A blighted crop in its own chunk.
        lit(&mut w, 40, 64, 40, block::BLIGHTED_CROP);
        // Saplings: one in a chunk with a crop, two elsewhere.
        lit(&mut w, 5, 64, 5, block::SAPLING_OAK);
        lit(&mut w, -33, 70, 20, block::SAPLING_BIRCH);
        lit(&mut w, -34, 70, 21, block::SAPLING_SPRUCE);
        w
    }

    #[test]
    fn the_farming_scans_walk_only_chunks_that_hold_what_they_look_for() {
        let w = scattered_farm_world();
        assert!(w.iter_chunks().count() >= 40, "the world has many chunks to skip");
        reset_walked();
        let (crops, blighted) = scan_crops(&w);
        let (crop_chunks, _) = walked();
        assert_eq!(crops.len(), 9 + 9 + 2, "every crop is still found");
        assert_eq!(blighted, vec![(40, 64, 40)]);
        // (0,4,0) holds wheat and carrots; (-1,4,-1) the potato and the reed;
        // (2,4,2) the blighted crop.
        assert_eq!(crop_chunks, 3, "only chunks with a crop or a blighted crop are walked");
        reset_walked();
        let saplings = collect_sapling_positions(&w);
        let (_, sapling_chunks) = walked();
        assert_eq!(saplings.len(), 3);
        assert_eq!(sapling_chunks, 2, "only chunks with a sapling are walked");
    }

    #[test]
    fn tick_growth_collects_saplings_only_on_the_passes_that_advance_them() {
        let mut w = scattered_farm_world();
        reset_walked();
        // Tick 100 is a watered-crop pass: crops advance, saplings do not.
        let _ = tick_growth(&mut w, CROP_GROWTH_TICKS_PER_STAGE / WATER_GROWTH_DIVISOR, false, 7, false);
        let (crop_chunks, sapling_chunks) = walked();
        assert!(crop_chunks > 0, "the crop scan ran");
        assert_eq!(sapling_chunks, 0, "no sapling scan on a pass that cannot grow one");
        reset_walked();
        let _ = tick_growth(&mut w, CROP_GROWTH_TICKS_PER_STAGE, false, 7, false);
        assert_eq!(walked().1, 2, "the 200-tick pass scans the sapling chunks");
    }

    /// The pass as it was before the counters: full walks of every chunk.
    fn full_walk_growth(world: &mut World, tick: u64, raining: bool, seed: u32, blight: bool) -> (Vec<BlockChange>, Vec<(i32, i32, i32)>) {
        if !crops_should_advance(tick) {
            return (Vec::new(), Vec::new());
        }
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let (mut crops, mut blighted, mut saplings) = (Vec::new(), Vec::new(), Vec::new());
        let mut keys: Vec<_> = world.iter_chunks().map(|(k, _)| k).collect();
        keys.sort_unstable();
        for (cx, cy, cz) in keys {
            for ly in 0..cs {
                for lz in 0..cs {
                    for lx in 0..cs {
                        let (x, y, z) = (cx * cs + lx, cy * cs + ly, cz * cs + lz);
                        let b = world.get_block(x, y, z);
                        if b == block::BLIGHTED_CROP {
                            blighted.push((x, y, z));
                        } else if is_crop(b) {
                            crops.push((x, y, z));
                        }
                        if crate::block::is_sapling_block(b) {
                            saplings.push((x, y, z));
                        }
                    }
                }
            }
        }
        let monocultures = blight.then(|| crate::blight::find_monocultures(world, &crops));
        let mut changes = advance_crops_blight(world, tick, raining, crops, monocultures.as_ref());
        if blight {
            changes.extend(crate::blight::spread(world, tick, blighted));
        }
        let grown = advance_saplings(world, tick, seed, saplings);
        (changes, grown)
    }

    fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
        v.sort();
        v
    }

    #[test]
    fn a_scattered_farm_grows_exactly_as_the_full_walk_grew_it() {
        let mut a = scattered_farm_world();
        let mut b = scattered_farm_world();
        // A 12 x 8 wheat field in one region, so the blight roll has a monoculture to find.
        for x in 100..112 {
            for z in 100..108 {
                for w in [&mut a, &mut b] {
                    w.set_block(x, 64, z, WHEAT_STAGE_0);
                    w.set_sky_light_at(x, 64, z, 15);
                }
            }
        }
        let mut grew = 0usize;
        for pass in 1..=80u64 {
            let tick = pass * 100;
            let new = tick_growth(&mut a, tick, pass % 7 == 0, 11, true);
            let (old_changes, old_grown) = full_walk_growth(&mut b, tick, pass % 7 == 0, 11, true);
            let key = |c: &BlockChange| (c.x, c.y, c.z, c.new_block, c.meta);
            assert_eq!(
                sorted(new.crop_changes.iter().map(key).collect()),
                sorted(old_changes.iter().map(key).collect()),
                "tick {tick}: crop changes differ"
            );
            assert_eq!(sorted(new.grown_cells), sorted(old_grown), "tick {tick}: grown cells differ");
            grew += new.crop_changes.len();
        }
        assert!(grew > 20, "the farm actually grew ({grew} changes)");
        for x in -40..120 {
            for z in -40..120 {
                for y in 62..100 {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z), "world differs at ({x},{y},{z})");
                }
            }
        }
    }

    #[test]
    fn tick_growth_is_inert_off_cadence_and_grows_on_it() {
        let mut world = World::new();
        world.set_block(0, 70, 0, WHEAT_STAGE_0);
        world.set_sky_light_at(0, 70, 0, 15);
        let off = tick_growth(&mut world, CROP_GROWTH_TICKS_PER_STAGE - 1, false, 7, false);
        assert!(off.crop_changes.is_empty() && off.grown_cells.is_empty());
        assert_eq!(world.get_block(0, 70, 0), WHEAT_STAGE_0);
        let on = tick_growth(&mut world, CROP_GROWTH_TICKS_PER_STAGE, false, 7, false);
        assert_eq!(on.crop_changes.len(), 1, "the lit wheat advances on the base cadence");
        assert_eq!(world.get_block(0, 70, 0), WHEAT_STAGE_1);
    }

    #[test]
    fn sapling_grows_into_its_species_tree() {
        let mut w = World::new();
        for x in -8..=8 {
            for z in -8..=8 {
                w.set_block(x, 20, z, crate::block::GRASS);
            }
        }
        w.set_block(0, 21, 0, crate::block::SAPLING_BIRCH);
        // Daylight: no sky-light computed in a hand-built world, so torch it.
        // effective_light_at reads the light grid; hand-set block light high.
        w.set_block_light_at(0, 21, 0, 15);
        // Sweep ticks on the cadence until the deterministic roll lands.
        let saplings = vec![(0, 21, 0)];
        let mut grown = Vec::new();
        for k in 1..400u64 {
            let t = k * CROP_GROWTH_TICKS_PER_STAGE;
            grown = advance_saplings(&mut w, t, 7, saplings.clone());
            if !grown.is_empty() {
                break;
            }
        }
        assert!(!grown.is_empty(), "the roll must land within 400 scans");
        assert_ne!(
            w.get_block(0, 21, 0),
            crate::block::SAPLING_BIRCH,
            "sapling consumed"
        );
        // A birch trunk grew on the sapling column.
        assert_eq!(w.get_block(0, 21, 0), crate::block::BIRCH_LOG, "birch trunk base");
    }

    /// Phase B1 review — a sapling on a loaded column's edge grows its tree
    /// only inside present columns: a canopy block written into a column that
    /// was never loaded conjured a non-empty chunk there, which that column's
    /// own generation then skipped (a slice of missing terrain).
    #[test]
    fn a_tree_never_grows_into_a_column_that_is_not_loaded() {
        let mut w = World::new();
        for x in 0..16 {
            for z in 0..16 {
                w.set_block(x, 20, z, crate::block::GRASS);
            }
        }
        let at = (15, 21, 8); // east edge of column (0, 0); (1, 0) never loaded
        w.set_block(at.0, at.1, at.2, crate::block::SAPLING_OAK);
        w.set_block_light_at(at.0, at.1, at.2, 15);
        let mut grown = Vec::new();
        for k in 1..400u64 {
            grown = advance_saplings(&mut w, k * CROP_GROWTH_TICKS_PER_STAGE, 7, vec![at]);
            if !grown.is_empty() {
                break;
            }
        }
        assert!(!grown.is_empty(), "the roll must land within 400 scans");
        assert_eq!(w.get_block(at.0, at.1, at.2), crate::block::OAK_LOG, "the trunk grew");
        assert!(
            grown.iter().all(|&(x, _, _)| x < 16),
            "every grown cell is inside the present column: {grown:?}"
        );
        assert!(
            !(0..=crate::world::MAX_CHUNK_Y).any(|cy| w.has_chunk(1, cy, 0)),
            "nothing was written into the never-loaded column"
        );
    }

    #[test]
    fn dark_sapling_never_grows() {
        let mut w = World::new();
        w.set_block(0, 20, 0, crate::block::GRASS);
        w.set_block(0, 21, 0, crate::block::SAPLING_OAK);
        // No light set → effective light 0 (underground vibes).
        for k in 1..200u64 {
            let t = k * CROP_GROWTH_TICKS_PER_STAGE;
            let grown = advance_saplings(&mut w, t, 7, vec![(0, 21, 0)]);
            assert!(grown.is_empty(), "no growth in the dark (light gate)");
        }
        assert_eq!(w.get_block(0, 21, 0), crate::block::SAPLING_OAK);
    }

    #[test]
    fn crops_should_advance_fires_only_on_the_growth_cadence() {
        // The hoisted scan-gate must fire exactly when advance_crops would do
        // work; an off-cadence skip would silently drop growth (engine audit B).
        assert!(!crops_should_advance(0), "tick 0 never grows");
        assert!(crops_should_advance(CROP_GROWTH_TICKS_PER_STAGE), "per-stage cadence (200)");
        assert!(crops_should_advance(CROP_GROWTH_TICKS_PER_STAGE / WATER_GROWTH_DIVISOR), "watered cadence (100)");
        assert!(crops_should_advance(2 * CROP_GROWTH_TICKS_PER_STAGE), "later multiple");
        assert!(!crops_should_advance(50), "off-cadence");
        assert!(!crops_should_advance(101), "off-cadence");
        assert!(!crops_should_advance(199), "off-cadence");
    }

    #[test]
    fn next_stage_progresses_wheat_carrot_potato() {
        // Spec 35 farmable-flower — stage 2 graduates to the wild-form
        // mature block (id 131-133), not a new "stage 3".
        assert_eq!(next_stage(CORNFLOWER_STAGE_0), Some(CORNFLOWER_STAGE_1));
        assert_eq!(next_stage(CORNFLOWER_STAGE_2), Some(CORNFLOWER));
        assert_eq!(next_stage(CORNFLOWER), None,
            "wild-form mature flower is the terminal stage");
        assert_eq!(next_stage(FIELD_POPPY_STAGE_2), Some(FIELD_POPPY));
        assert_eq!(next_stage(BUTTERCUP_STAGE_2), Some(BUTTERCUP));
        assert_eq!(next_stage(WHEAT_STAGE_0), Some(WHEAT_STAGE_1));
        assert_eq!(next_stage(WHEAT_STAGE_1), Some(WHEAT_STAGE_2));
        assert_eq!(next_stage(WHEAT_STAGE_2), Some(WHEAT_STAGE_3));
        assert_eq!(next_stage(WHEAT_STAGE_3), None);
        assert_eq!(next_stage(CARROT_STAGE_0), Some(CARROT_STAGE_1));
        assert_eq!(next_stage(CARROT_STAGE_3), None);
        assert_eq!(next_stage(POTATO_STAGE_2), Some(POTATO_STAGE_3));
        assert_eq!(next_stage(POTATO_STAGE_3), None);
    }

    #[test]
    fn next_stage_progresses_papyrus() {
        // Spec 23: papyrus rides the same 4-stage ladder.
        assert_eq!(next_stage(PAPYRUS_STAGE_0), Some(PAPYRUS_STAGE_1));
        assert_eq!(next_stage(PAPYRUS_STAGE_1), Some(PAPYRUS_STAGE_2));
        assert_eq!(next_stage(PAPYRUS_STAGE_2), Some(PAPYRUS_STAGE_3));
        assert_eq!(next_stage(PAPYRUS_STAGE_3), None);
    }

    #[test]
    fn is_crop_recognises_papyrus_stages() {
        for id in PAPYRUS_STAGE_0..=PAPYRUS_STAGE_3 {
            assert!(is_crop(id), "papyrus stage {id} should register as a crop");
        }
    }

    #[test]
    fn papyrus_advances_when_water_adjacent() {
        // Plant a stage-0 reed next to water; advance_crops at the
        // fast-fire tick (100) advances it to stage 1.
        let mut world = World::new();
        world.set_block(0, 70, 0, PAPYRUS_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        world.set_block(2, 70, 0, block::WATER);
        let changes = advance_crops(&mut world, 100, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 1);
        assert_eq!(world.get_block(0, 70, 0), PAPYRUS_STAGE_1);
    }

    #[test]
    fn papyrus_advances_at_base_interval_when_no_water_nearby() {
        // If the world somehow ends up with a papyrus on a dry tile
        // (water removed post-placement), growth falls back to the
        // 200-tick base interval — papyrus doesn't have a special-case
        // dry-out mechanic for v1. Documents the chosen behaviour.
        let mut world = World::new();
        world.set_block(0, 70, 0, PAPYRUS_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        // No water nearby.
        let changes = advance_crops(&mut world, 100, false, [(0, 70, 0)]);
        assert!(changes.is_empty(), "tick 100 is fast_fire but not base — dry papyrus stays");
        let changes = advance_crops(&mut world, 200, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 1, "tick 200 is base_fire — dry papyrus advances");
        assert_eq!(world.get_block(0, 70, 0), PAPYRUS_STAGE_1);
    }

    #[test]
    fn next_stage_returns_none_for_non_crop() {
        assert_eq!(next_stage(block::AIR), None);
        assert_eq!(next_stage(block::DIRT), None);
        assert_eq!(next_stage(block::TILLED_SOIL), None);
        assert_eq!(next_stage(block::STONE), None);
    }

    #[test]
    fn bonemeal_advances_young_crop_at_least_one_stage() {
        // Across many seeds, bonemeal on a stage-0 crop always lands on
        // stage 1 or 2 (never stays at 0, never overshoots past mature).
        let mut saw_one = false;
        let mut saw_two = false;
        for seed in 0..200u64 {
            let r = bonemeal_advance(WHEAT_STAGE_0, seed).expect("crop advances");
            assert!(r == WHEAT_STAGE_1 || r == WHEAT_STAGE_2, "seed {seed} -> {r}");
            saw_one |= r == WHEAT_STAGE_1;
            saw_two |= r == WHEAT_STAGE_2;
        }
        assert!(saw_one && saw_two, "bonemeal should sometimes +1 and sometimes +2");
    }

    #[test]
    fn bonemeal_never_overshoots_mature() {
        // One stage off mature: at most reaches mature, never past it.
        for seed in 0..50u64 {
            let r = bonemeal_advance(WHEAT_STAGE_2, seed).expect("advances");
            assert_eq!(r, WHEAT_STAGE_3, "from STAGE_2 the only next is mature");
        }
    }

    #[test]
    fn bonemeal_is_noop_on_mature_or_non_crop() {
        // Mature crop + non-crop both return None so the caller doesn't
        // waste a bonemeal on something it can't grow.
        assert_eq!(bonemeal_advance(WHEAT_STAGE_3, 7), None);
        assert_eq!(bonemeal_advance(block::STONE, 7), None);
        assert_eq!(bonemeal_advance(block::TILLED_SOIL, 7), None);
    }

    #[test]
    fn is_crop_recognises_all_12_stages_and_nothing_else() {
        for id in WHEAT_STAGE_0..=POTATO_STAGE_3 {
            assert!(is_crop(id), "id {id} should be a crop");
        }
        assert!(!is_crop(block::DIRT));
        assert!(!is_crop(block::TILLED_SOIL));
        assert!(!is_crop(block::WATER));
        // Spec 23: papyrus tiles are crops too (so the chunk-iteration
        // path that calls is_crop picks them up for the growth tick).
        assert!(!is_crop(block::AIR));
        assert!(!is_crop(block::SAND));
    }

    #[test]
    fn dry_crop_advances_at_base_interval() {
        // No water nearby — advance fires only on multiples of
        // CROP_GROWTH_TICKS_PER_STAGE (200).
        let mut world = World::new();
        world.set_block(0, 70, 0, WHEAT_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        let changes = advance_crops(&mut world, 200, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 1);
        assert_eq!(world.get_block(0, 70, 0), WHEAT_STAGE_1);
    }

    #[test]
    fn dry_crop_does_not_advance_off_interval() {
        let mut world = World::new();
        world.set_block(0, 70, 0, WHEAT_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        let changes = advance_crops(&mut world, 199, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 0);
        assert_eq!(world.get_block(0, 70, 0), WHEAT_STAGE_0);
    }

    #[test]
    fn water_adjacent_crop_advances_faster() {
        // Water within radius 4 horizontally. fast_fire interval = 100.
        let mut world = World::new();
        world.set_block(0, 70, 0, CARROT_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        world.set_block(2, 70, 0, block::WATER);
        let changes = advance_crops(&mut world, 100, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 1);
        assert_eq!(world.get_block(0, 70, 0), CARROT_STAGE_1);
    }

    #[test]
    fn rain_advances_a_dry_crop_on_the_fast_interval() {
        // P8 — rain waters every crop, so a dry crop with no water nearby
        // advances on the fast (100-tick) cadence when raining=true.
        let mut world = World::new();
        world.set_block(0, 70, 0, CARROT_STAGE_0);
        world.set_sky_light_at(0, 70, 0, 15);
        // raining=true, dry tile, tick 100 (fast cadence).
        let changes = advance_crops(&mut world, 100, true, [(0, 70, 0)]);
        assert_eq!(changes.len(), 1, "rain should water the dry crop");
        assert_eq!(world.get_block(0, 70, 0), CARROT_STAGE_1);
    }

    #[test]
    fn dry_crop_does_not_advance_on_fast_interval() {
        // Tick 100 is fast_fire but not base_fire. A dry crop must NOT
        // advance on this tick — only on multiples of 200.
        let mut world = World::new();
        world.set_block(0, 70, 0, POTATO_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        let changes = advance_crops(&mut world, 100, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 0);
        assert_eq!(world.get_block(0, 70, 0), POTATO_STAGE_0);
    }

    #[test]
    fn mature_crop_never_advances() {
        let mut world = World::new();
        world.set_block(0, 70, 0, WHEAT_STAGE_3); world.set_sky_light_at(0, 70, 0, 15);
        world.set_block(2, 70, 0, block::WATER);
        let changes = advance_crops(&mut world, 200, false, [(0, 70, 0)]);
        assert!(changes.is_empty());
        assert_eq!(world.get_block(0, 70, 0), WHEAT_STAGE_3);
    }

    #[test]
    fn tick_zero_does_not_fire() {
        // Guard against the modulo-zero false positive at startup.
        let mut world = World::new();
        world.set_block(0, 70, 0, WHEAT_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        let changes = advance_crops(&mut world, 0, false, [(0, 70, 0)]);
        assert!(changes.is_empty());
    }

    #[test]
    fn water_below_also_counts() {
        // Irrigation channels often sit one block below the crop;
        // water at (dx, y-1, dz) is also valid.
        let mut world = World::new();
        world.set_block(0, 70, 0, CARROT_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        world.set_block(1, 69, 0, block::WATER);
        let changes = advance_crops(&mut world, 100, false, [(0, 70, 0)]);
        assert_eq!(changes.len(), 1);
        assert_eq!(world.get_block(0, 70, 0), CARROT_STAGE_1);
    }

    #[test]
    fn mature_wheat_breaks_to_tilled_soil_with_wheat_and_seeds() {
        let result = crop_break(WHEAT_STAGE_3, 1234, true).expect("mature wheat must break");
        assert_eq!(result.replacement, block::TILLED_SOIL);
        assert_eq!(result.drops.len(), 2);
        match &result.drops[0].item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::Wheat));
                assert_eq!(result.drops[0].count, 1);
            }
            _ => panic!("expected wheat material drop"),
        }
        match &result.drops[1].item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::WheatSeeds));
                let n = result.drops[1].count;
                assert!((1..=3).contains(&n), "seed count {n} out of [1,3]");
            }
            _ => panic!("expected seed material drop"),
        }
    }

    #[test]
    fn mature_carrot_drops_carrots_only() {
        let result = crop_break(CARROT_STAGE_3, 42, true).expect("mature carrot must break");
        assert_eq!(result.replacement, block::TILLED_SOIL);
        assert_eq!(result.drops.len(), 1);
        match &result.drops[0].item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::Carrot));
                let n = result.drops[0].count;
                assert!((1..=4).contains(&n), "carrot count {n} out of [1,4]");
            }
            _ => panic!("expected carrot drop"),
        }
    }

    #[test]
    fn mature_potato_drops_potatoes_only() {
        let result = crop_break(POTATO_STAGE_3, 7, true).expect("mature potato must break");
        assert_eq!(result.replacement, block::TILLED_SOIL);
        assert_eq!(result.drops.len(), 1);
        let n = result.drops[0].count;
        assert!((1..=4).contains(&n));
    }

    #[test]
    fn immature_crops_break_to_air_with_no_drops() {
        for stage in [
            WHEAT_STAGE_0, WHEAT_STAGE_1, WHEAT_STAGE_2,
            CARROT_STAGE_0, CARROT_STAGE_1, CARROT_STAGE_2,
            POTATO_STAGE_0, POTATO_STAGE_1, POTATO_STAGE_2,
        ] {
            let result = crop_break(stage, 0, true).expect("immature crop must break");
            assert_eq!(result.replacement, block::AIR);
            assert!(result.drops.is_empty(), "stage {stage} should drop nothing");
        }
    }

    #[test]
    fn mature_papyrus_drops_one_or_two_reeds_and_replants_in_place() {
        // Spec 23: sugarcane-style auto-regrow. The mature block
        // resets to PAPYRUS_STAGE_0 (root keeps growing) and drops
        // 1-2 reed materials.
        let result = crop_break(PAPYRUS_STAGE_3, 99, true).expect("mature papyrus must break");
        assert_eq!(result.replacement, PAPYRUS_STAGE_0);
        assert_eq!(result.drops.len(), 1);
        match &result.drops[0].item {
            crate::item::Item::Material(crate::item::MaterialId::PapyrusReed) => {}
            other => panic!("expected PapyrusReed drop, got {:?}", other),
        }
        let n = result.drops[0].count;
        assert!((1..=2).contains(&n), "reed count {n} out of [1, 2]");
    }

    #[test]
    fn papyrus_drop_count_is_deterministic_for_same_seed() {
        let a = crop_break(PAPYRUS_STAGE_3, 555, true).unwrap();
        let b = crop_break(PAPYRUS_STAGE_3, 555, true).unwrap();
        assert_eq!(a.drops[0].count, b.drops[0].count);
    }

    #[test]
    fn immature_papyrus_breaks_to_air_with_no_drops() {
        // Same penalty as the other field crops — clear-cutting an
        // immature reed costs the root.
        for stage in [PAPYRUS_STAGE_0, PAPYRUS_STAGE_1, PAPYRUS_STAGE_2] {
            let result = crop_break(stage, 0, true).expect("immature papyrus must break");
            assert_eq!(result.replacement, block::AIR);
            assert!(result.drops.is_empty(), "papyrus stage {stage} should drop nothing");
        }
    }

    #[test]
    fn crop_break_returns_none_for_non_crops() {
        for blk in [block::AIR, block::DIRT, block::TILLED_SOIL, block::STONE] {
            assert!(crop_break(blk, 0, true).is_none(), "non-crop {blk} should be None");
        }
    }

    #[test]
    fn crop_break_drop_count_is_deterministic_for_same_seed() {
        // Re-running with the same seed must produce the same drop count.
        let a = crop_break(CARROT_STAGE_3, 555, true).unwrap();
        let b = crop_break(CARROT_STAGE_3, 555, true).unwrap();
        assert_eq!(a.drops[0].count, b.drops[0].count);
    }

    #[test]
    fn all_three_crops_grow_independently() {
        let mut world = World::new();
        world.set_block(0, 70, 0, WHEAT_STAGE_0); world.set_sky_light_at(0, 70, 0, 15);
        world.set_block(1, 70, 0, CARROT_STAGE_0); world.set_sky_light_at(1, 70, 0, 15);
        world.set_block(2, 70, 0, POTATO_STAGE_0); world.set_sky_light_at(2, 70, 0, 15);
        let changes = advance_crops(
            &mut world,
            200,
            false,
            [(0, 70, 0), (1, 70, 0), (2, 70, 0)],
        );
        assert_eq!(changes.len(), 3);
        assert_eq!(world.get_block(0, 70, 0), WHEAT_STAGE_1);
        assert_eq!(world.get_block(1, 70, 0), CARROT_STAGE_1);
        assert_eq!(world.get_block(2, 70, 0), POTATO_STAGE_1);
    }

    #[test]
    fn fibre_crops_grow_and_harvest_to_fibre_plus_seeds() {
        // Spec 36 Phase 2 — cotton/hemp chain + mature drop.
        assert_eq!(next_stage(COTTON_STAGE_0), Some(COTTON_STAGE_1));
        assert_eq!(next_stage(COTTON_STAGE_2), Some(COTTON_STAGE_3));
        assert_eq!(next_stage(COTTON_STAGE_3), None);
        assert!(is_crop(HEMP_STAGE_2));
        let r = crop_break(COTTON_STAGE_3, 7, true).expect("mature cotton breaks");
        assert_eq!(r.replacement, block::TILLED_SOIL);
        assert!(r.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::Cotton))));
        assert!(r.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::CottonSeeds))));
        let h = crop_break(HEMP_STAGE_3, 9, true).expect("mature hemp breaks");
        assert!(h.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::HempFibre))));
    }

    // ─── Spec 35 farmable-flower follow-on (2026-05-28) ──────────────

    #[test]
    fn flower_crops_grow_chain_through_to_mature_wild_block() {
        // Stage 2 graduates to the existing wild-form block (131/132/133),
        // not a new "stage 3" — keeps worldgen + crafting recipes that
        // reference the mature flower untouched.
        for (s0, s1, s2, mature) in [
            (CORNFLOWER_STAGE_0, CORNFLOWER_STAGE_1, CORNFLOWER_STAGE_2, CORNFLOWER),
            (FIELD_POPPY_STAGE_0, FIELD_POPPY_STAGE_1, FIELD_POPPY_STAGE_2, FIELD_POPPY),
            (BUTTERCUP_STAGE_0, BUTTERCUP_STAGE_1, BUTTERCUP_STAGE_2, BUTTERCUP),
        ] {
            assert_eq!(next_stage(s0), Some(s1));
            assert_eq!(next_stage(s1), Some(s2));
            assert_eq!(next_stage(s2), Some(mature));
            assert_eq!(next_stage(mature), None);
            assert!(is_crop(s0));
            assert!(is_crop(mature),
                "wild-form mature flower must register as a crop so it drops via crop_break");
        }
    }

    #[test]
    fn mature_flower_drops_block_and_seeds() {
        // Mature cornflower → 1 CORNFLOWER block + 1-2 CornflowerSeeds +
        // resets to TILLED_SOIL. Same shape as wheat.
        let r = crop_break(CORNFLOWER, 7, true).expect("mature cornflower breaks");
        assert_eq!(r.replacement, block::TILLED_SOIL);
        assert!(r.drops.iter().any(|s| matches!(&s.item,
            crate::item::Item::Block(b) if *b == CORNFLOWER)));
        assert!(r.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::CornflowerSeeds))));
        let p = crop_break(FIELD_POPPY, 11, true).expect("mature field poppy breaks");
        assert!(p.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::FieldPoppySeeds))));
        let b = crop_break(BUTTERCUP, 13, true).expect("mature buttercup breaks");
        assert!(b.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::ButtercupSeeds))));
    }

    #[test]
    fn wild_flower_breaks_to_air_farmed_resets_to_soil() {
        // #16 — wild + farmed flowers share a block id, so the substrate
        // (`on_tilled`) decides the replacement. A wild flower mined out of a
        // grass meadow must leave AIR (clean grass), not a stray tilled-soil
        // scar; a farmed flower on tilled soil resets to soil for replanting.
        for f in [CORNFLOWER, FIELD_POPPY, BUTTERCUP] {
            let wild = crop_break(f, 7, false).expect("flower breaks");
            assert_eq!(
                wild.replacement, block::AIR,
                "wild flower {f} on grass must break to AIR"
            );
            let farmed = crop_break(f, 7, true).expect("flower breaks");
            assert_eq!(
                farmed.replacement, block::TILLED_SOIL,
                "farmed flower {f} on tilled soil resets to soil"
            );
            // Same drops either way — only the replacement block differs.
            assert_eq!(wild.drops.len(), farmed.drops.len());
        }
    }

    #[test]
    fn immature_flower_stages_drop_nothing() {
        // Clear-cutting a half-grown crop costs the player the
        // investment — every immature stage drops nothing + reverts
        // to AIR (NOT tilled soil, since the player didn't fully
        // grow it).
        for s in [
            CORNFLOWER_STAGE_0, CORNFLOWER_STAGE_1, CORNFLOWER_STAGE_2,
            FIELD_POPPY_STAGE_0, FIELD_POPPY_STAGE_1, FIELD_POPPY_STAGE_2,
            BUTTERCUP_STAGE_0, BUTTERCUP_STAGE_1, BUTTERCUP_STAGE_2,
        ] {
            let r = crop_break(s, 1, true).expect("immature flower breaks");
            assert!(r.drops.is_empty(), "stage {s} dropped {:?}", r.drops);
            assert_eq!(r.replacement, block::AIR);
        }
    }

    #[test]
    fn flower_seeds_are_in_a_distinct_band_per_flower() {
        // A spot-check that the three seed kinds are distinguishable
        // — Cornflower seeds must not equal Field Poppy seeds via the
        // MaterialId comparison, even though `crop_break` returns the
        // same shape for all three.
        let c = crop_break(CORNFLOWER, 1, true).unwrap();
        let p = crop_break(FIELD_POPPY, 1, true).unwrap();
        let any_cornflower_seed = c.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::CornflowerSeeds)));
        let any_poppy_seed = p.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::FieldPoppySeeds)));
        assert!(any_cornflower_seed && any_poppy_seed);
        // Neither contains the other's seed.
        assert!(!c.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::FieldPoppySeeds))));
        assert!(!p.drops.iter().any(|s| matches!(s.item,
            crate::item::Item::Material(crate::item::MaterialId::CornflowerSeeds))));
    }
}
