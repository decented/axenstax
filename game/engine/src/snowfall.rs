//! Salt feature — snowfall painter + path-acceptance helpers.
//!
//! Two halves:
//! 1. **Path placement** — `target_block_accepts_salt_path(BlockId) -> bool`
//!    decides what player-aimed blocks can be converted to SALT_PATH.
//! 2. **Snowfall painter** — `tick_snowfall(...)` drives slow SNOW
//!    drift onto GRASS/DIRT tops in `SnowyTundra`-biome chunks,
//!    naturally skipping SALT_PATH (which is not GRASS/DIRT).
//!
//! Spec: `docs/foundations/2026-05-23-salt.md`.

use crate::block::{self, BlockId};

/// True when the player's right-clicked target block accepts a salt
/// path placement. GRASS + DIRT are converted in place; SNOW is melted
/// into the path beneath (the SNOW block at `pos` becomes SALT_PATH).
pub fn target_block_accepts_salt_path(target: BlockId) -> bool {
    matches!(target, block::GRASS | block::DIRT | block::SNOW)
}

/// Ticks between snowfall painter passes. 6 000 ticks ≈ 5 in-game
/// minutes at 20 TPS (default world_time_step = 1).
pub const SNOWFALL_PERIOD_TICKS: u64 = 6_000;

/// Pure: deterministic per-chunk column pick for the snowfall pass.
/// Returns the (wx, wz) the painter should attempt this round, or
/// `None` on a roll-skip. `chunk_key` is `(cx, cz)`; `pass_id` is
/// typically `current_tick / SNOWFALL_PERIOD_TICKS`.
pub fn snowfall_attempt_for_chunk(
    seed: u32,
    chunk_key: (i32, i32),
    pass_id: u64,
) -> Option<(i32, i32)> {
    let (cx, cz) = chunk_key;
    let mut h = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (cx as u64).wrapping_mul(0xBB67_AE85_84CA_A73B);
    h ^= (cz as u64).wrapping_mul(0x3C6E_F372_FE94_F82B);
    h ^= pass_id.wrapping_mul(0xA54F_F53A_5F1D_36F1);
    h = h.rotate_left(13).wrapping_mul(0x510E_527F_ADE6_82D1);
    // 50 % skip rate — keeps the drift slow.
    if h.is_multiple_of(2) {
        return None;
    }
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let lx = ((h >> 8) as i32).rem_euclid(cs);
    let lz = ((h >> 24) as i32).rem_euclid(cs);
    Some((cx * cs + lx, cz * cs + lz))
}

/// Painter: walks every loaded chunk; if the chunk's centre biome is
/// `SnowyTundra`, picks a column via `snowfall_attempt_for_chunk` and
/// places a `SNOW` block above the surface if and only if the surface
/// is `GRASS` or `DIRT` (i.e. not already SNOW, not SALT_PATH).
///
/// Called every tick from the server + single-player game-loop, but
/// no-ops on ticks where `current_tick % SNOWFALL_PERIOD_TICKS != 0`
/// so it's cheap to spam.
///
/// Returns the number of SNOW blocks placed this call (0 on skipped
/// ticks; usually 0-N where N is the number of loaded SnowyTundra
/// chunks).
pub fn tick_snowfall(
    world: &mut crate::world::World,
    biome_gen: &crate::biome::BiomeGenerator,
    seed: u32,
    current_tick: u64,
) -> u32 {
    if !current_tick.is_multiple_of(SNOWFALL_PERIOD_TICKS) {
        return 0;
    }
    let pass_id = current_tick / SNOWFALL_PERIOD_TICKS;
    let cs = crate::chunk::CHUNK_SIZE as i32;
    // Snapshot loaded chunk keys so we don't hold a borrow on
    // the chunk map while mutating via set_block.
    let chunk_keys: Vec<(i32, i32, i32)> = world.iter_chunks().map(|(k, _)| k).collect();
    let mut visited: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut placed = 0u32;
    for (cx, _cy, cz) in chunk_keys {
        if !visited.insert((cx, cz)) {
            continue;
        }
        let centre_wx = cx * cs + cs / 2;
        let centre_wz = cz * cs + cs / 2;
        // Salt feature — snowfall fires in cold biomes. SnowyTundra
        // future-proofs against Spec 28a's classifier landing; Mountains
        // is the currently-live cold biome and gives the "snowy peaks"
        // feel without waiting for the classifier wire-up.
        if !matches!(
            biome_gen.biome_at(centre_wx, centre_wz),
            crate::biome::Biome::SnowyTundra | crate::biome::Biome::Mountains,
        ) {
            continue;
        }
        let Some((wx, wz)) = snowfall_attempt_for_chunk(seed, (cx, cz), pass_id) else {
            continue;
        };
        let surface_y = biome_gen.terrain_height(wx, wz);
        let surface_block = world.get_block(wx, surface_y, wz);
        if !matches!(surface_block, block::GRASS | block::DIRT) {
            continue;
        }
        // Accumulate (2026-07-04): AIR → first thin layer; an existing
        // SNOW_LAYER deepens by one (AUX = layers-1); the 8th layer converts
        // to the full SNOW block. Anything else blocks accumulation.
        let above = world.get_block(wx, surface_y + 1, wz);
        match above {
            block::AIR => {
                world.set_block(wx, surface_y + 1, wz, block::SNOW_LAYER);
                world.set_meta((wx, surface_y + 1, wz), 0);
                placed += 1;
            }
            block::SNOW_LAYER => {
                let aux = crate::meta::aux(world.meta_at(wx, surface_y + 1, wz));
                if aux >= 7 {
                    world.set_block(wx, surface_y + 1, wz, block::SNOW);
                    world.set_meta((wx, surface_y + 1, wz), 0);
                } else {
                    world.set_meta(
                        (wx, surface_y + 1, wz),
                        crate::meta::with_aux(0, aux + 1),
                    );
                }
                placed += 1;
            }
            _ => continue,
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_path_accepts_grass_dirt_snow() {
        assert!(target_block_accepts_salt_path(block::GRASS));
        assert!(target_block_accepts_salt_path(block::DIRT));
        assert!(target_block_accepts_salt_path(block::SNOW));
    }

    #[test]
    fn salt_path_rejects_stone_water_air() {
        assert!(!target_block_accepts_salt_path(block::STONE));
        assert!(!target_block_accepts_salt_path(block::WATER));
        assert!(!target_block_accepts_salt_path(block::AIR));
    }

    #[test]
    fn salt_path_rejects_salt_path_itself() {
        // Don't re-convert what's already a path — the right-click
        // handler should no-op, not spend another Salt.
        assert!(!target_block_accepts_salt_path(block::SALT_PATH));
    }

    #[test]
    fn snowfall_attempt_is_deterministic() {
        let a = snowfall_attempt_for_chunk(42, (3, 5), 100);
        let b = snowfall_attempt_for_chunk(42, (3, 5), 100);
        assert_eq!(a, b);
    }

    #[test]
    fn snowfall_attempt_mixes_skip_and_place() {
        // ~50 % skip rate; across 100 different (cx, cz) pairs at
        // least 10 should skip and 10 should produce a column.
        let mut some = 0;
        let mut none = 0;
        for cx in 0..10 {
            for cz in 0..10 {
                if snowfall_attempt_for_chunk(7, (cx, cz), 0).is_some() {
                    some += 1;
                } else {
                    none += 1;
                }
            }
        }
        assert!(some > 10 && none > 10,
            "expected mix of skip + place results; got some={some} none={none}");
    }

    #[test]
    fn snowfall_attempt_picks_within_chunk_bounds() {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        for cx in -5..5 {
            for cz in -5..5 {
                if let Some((wx, wz)) = snowfall_attempt_for_chunk(123, (cx, cz), 1) {
                    assert!(wx >= cx * cs && wx < (cx + 1) * cs,
                        "wx={wx} not in cx*cs..(cx+1)*cs for cx={cx}");
                    assert!(wz >= cz * cs && wz < (cz + 1) * cs,
                        "wz={wz} not in cz*cs..(cz+1)*cs for cz={cz}");
                }
            }
        }
    }
}
