//! Spec 28b — per-species tree shape pure functions.
//!
//! Each `place_<species>` returns the set of `(dx, dy, dz, BlockId)`
//! offsets that the species' tree puts down relative to the trunk
//! base. Pure: deterministic per `(species, seed)` — same coordinates
//! always produce the same tree.
//!
//! Live placement (deciding *where* trees go in world-gen) needs the
//! 28a biome layer + Axolittle playtest. This module is the shape
//! library that the eventual live placer consumes.
//!
//! Each shape is testable for invariants:
//! - canopy radius scales with species
//! - trunk height in expected range
//! - block ids match the species
//! - generator deterministic per seed
//!
//! Spec: `docs/foundations/2026-05-20-wood-species-multi-tree.md`.

use crate::block::{self, BlockId, WoodSpecies};

/// One block of the tree shape: position relative to trunk base
/// `(dx=0, dy=0, dz=0)` and the block to place there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeBlock {
    pub dx: i32,
    pub dy: i32,
    pub dz: i32,
    pub id: BlockId,
}

/// Deterministic per-tree height: hash position + seed into the
/// species' trunk-height range.
fn height_for(species: WoodSpecies, x: i32, z: i32, seed: u32) -> i32 {
    let mut h = (x as u32).wrapping_mul(374761393)
        ^ (z as u32).wrapping_mul(668265263)
        ^ seed.wrapping_mul(2246822519);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    let (min, max) = match species {
        WoodSpecies::Oak => (4, 7),
        WoodSpecies::Birch => (6, 9),
        WoodSpecies::Spruce => (7, 13),
        WoodSpecies::Jungle => (8, 16),
        WoodSpecies::Acacia => (4, 6),
        WoodSpecies::DarkOak => (6, 9),
        WoodSpecies::Rubber => (10, 14),
    };
    min + (h as i32).rem_euclid(max - min + 1)
}

/// Top-level entry. Returns the species' tree shape rooted at the
/// trunk base offset `(0, 0, 0)`.
pub fn place_tree(species: WoodSpecies, x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    match species {
        WoodSpecies::Oak => place_oak(x, z, seed),
        WoodSpecies::Birch => place_birch(x, z, seed),
        WoodSpecies::Spruce => place_spruce(x, z, seed),
        WoodSpecies::Jungle => place_jungle(x, z, seed),
        WoodSpecies::Acacia => place_acacia(x, z, seed),
        WoodSpecies::DarkOak => place_dark_oak(x, z, seed),
        WoodSpecies::Rubber => place_rubber(x, z, seed),
    }
}

/// Oak — 4-7 tall trunk, roughly spherical canopy. The familiar default
/// shape; existing world-gen places this one.
fn place_oak(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let height = height_for(WoodSpecies::Oak, x, z, seed);
    let mut out = Vec::new();
    // Trunk
    for dy in 0..height {
        out.push(TreeBlock { dx: 0, dy, dz: 0, id: block::OAK_LOG });
    }
    // Spherical-ish canopy: top 3 layers + 2 inset.
    let top = height;
    // Bottom-of-canopy two layers (height-2 and height-1): 5×5 minus corners.
    for layer in (top - 2)..top {
        for dx in -2i32..=2 {
            for dz in -2i32..=2 {
                if dx == 0 && dz == 0 {
                    continue; // trunk position
                }
                let corner = (dx.abs() == 2 && dz.abs() == 2)
                    || (dx.abs() == 2 && dz.abs() == 1)
                    || (dx.abs() == 1 && dz.abs() == 2);
                if !corner {
                    out.push(TreeBlock { dx, dy: layer, dz, id: block::OAK_LEAVES });
                }
            }
        }
    }
    // Top two layers: 3×3 plus a single cap.
    for layer in top..(top + 2) {
        for dx in -1i32..=1 {
            for dz in -1i32..=1 {
                if dx == 0 && dz == 0 && layer == top {
                    continue; // trunk extends to here
                }
                if dx.abs() == 1 && dz.abs() == 1 && layer == top + 1 {
                    continue; // cap corners removed
                }
                out.push(TreeBlock { dx, dy: layer, dz, id: block::OAK_LEAVES });
            }
        }
    }
    // Trunk poking into the canopy.
    out.push(TreeBlock { dx: 0, dy: top, dz: 0, id: block::OAK_LOG });
    out
}

/// Birch — 6-9 tall trunk, narrower canopy than Oak. Birch grows
/// taller than oak in Minecraft; the canopy is tighter (radius 1
/// instead of radius 2).
fn place_birch(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let height = height_for(WoodSpecies::Birch, x, z, seed);
    let mut out = Vec::new();
    for dy in 0..height {
        out.push(TreeBlock { dx: 0, dy, dz: 0, id: block::BIRCH_LOG });
    }
    // Canopy: top 3 layers, radius 1.
    for layer in (height - 2)..(height + 2) {
        for dx in -1..=1 {
            for dz in -1..=1 {
                let cap = layer == height + 1 && (dx != 0 || dz != 0);
                if cap {
                    continue;
                }
                if dx == 0 && dz == 0 && layer < height {
                    continue; // trunk
                }
                out.push(TreeBlock { dx, dy: layer, dz, id: block::BIRCH_LEAVES });
            }
        }
    }
    out
}

/// Spruce — 7-13 tall trunk, conical layered canopy that shrinks with
/// height. The Taiga-defining shape.
fn place_spruce(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let height = height_for(WoodSpecies::Spruce, x, z, seed);
    let mut out = Vec::new();
    for dy in 0..height {
        out.push(TreeBlock { dx: 0, dy, dz: 0, id: block::SPRUCE_LOG });
    }
    // Cone: bottom is radius 2, narrows by 1 every 2 layers, finishes
    // at a point. Start the cone partway up the trunk (the bottom 1/3
    // of the trunk is bare to mirror Minecraft's spruce silhouette).
    let cone_start = height / 3;
    let layers_to_top = height + 2 - cone_start;
    for i in 0..layers_to_top {
        let layer = cone_start + i;
        let radius = (layers_to_top - i) / 2;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx == 0 && dz == 0 && layer < height {
                    continue;
                }
                // Round the corners: skip if dx² + dz² > radius² + 1.
                if dx * dx + dz * dz > radius * radius + 1 {
                    continue;
                }
                out.push(TreeBlock { dx, dy: layer, dz, id: block::SPRUCE_LEAVES });
            }
        }
    }
    out
}

/// Jungle — 8-16 tall, single-thick trunk, dense top canopy with
/// occasional leaves clinging to the trunk on the way up. (Spec calls
/// for 2×2 trunks at the top end; we keep 1×1 for alpha — the spec
/// doc flags 2×2 jungle giants as a polish wave.)
fn place_jungle(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let height = height_for(WoodSpecies::Jungle, x, z, seed);
    let mut out = Vec::new();
    for dy in 0..height {
        out.push(TreeBlock { dx: 0, dy, dz: 0, id: block::JUNGLE_LOG });
    }
    // Mid-trunk leaves: every 3rd block, attach a single leaf block
    // on a deterministic cardinal direction.
    let mut h = seed.wrapping_add(x as u32);
    for dy in 3..(height - 3) {
        if dy % 3 == 0 {
            h = h.wrapping_mul(1664525).wrapping_add(1013904223);
            let dir = h % 4;
            let (ldx, ldz) = match dir {
                0 => (1, 0),
                1 => (-1, 0),
                2 => (0, 1),
                _ => (0, -1),
            };
            out.push(TreeBlock { dx: ldx, dy, dz: ldz, id: block::JUNGLE_LEAVES });
        }
    }
    // Top canopy: 5×5 layer at trunk-top, 3×3 above. Full disc each
    // layer except trimmed corners.
    let top = height;
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            let corner = dx.abs() == 2 && dz.abs() == 2;
            if !corner && (dx != 0 || dz != 0) {
                out.push(TreeBlock { dx, dy: top, dz, id: block::JUNGLE_LEAVES });
            }
        }
    }
    for dx in -1..=1 {
        for dz in -1..=1 {
            out.push(TreeBlock { dx, dy: top + 1, dz, id: block::JUNGLE_LEAVES });
        }
    }
    out.push(TreeBlock { dx: 0, dy: top, dz: 0, id: block::JUNGLE_LOG });
    out
}

/// Acacia — 4-6 trunk, umbrella canopy: trunk goes up, then bends
/// off-centre, then a flat 5×5 canopy. The Savanna-defining shape.
fn place_acacia(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let height = height_for(WoodSpecies::Acacia, x, z, seed);
    let mut out = Vec::new();
    // Lower trunk straight up
    let split = height - 2;
    for dy in 0..split {
        out.push(TreeBlock { dx: 0, dy, dz: 0, id: block::ACACIA_LOG });
    }
    // Branch direction picked deterministically.
    let h = (x as u32 ^ seed).wrapping_mul(2654435761);
    let (bdx, bdz) = match h % 4 {
        0 => (1, 0),
        1 => (-1, 0),
        2 => (0, 1),
        _ => (0, -1),
    };
    // Upper trunk diagonals into the branch
    for step in 0..2 {
        out.push(TreeBlock {
            dx: bdx * step,
            dy: split + step,
            dz: bdz * step,
            id: block::ACACIA_LOG,
        });
    }
    // Flat 5×5 umbrella canopy centred at branch tip.
    let tip_x = bdx * 2;
    let tip_z = bdz * 2;
    let canopy_y = height;
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            let corner = dx.abs() == 2 && dz.abs() == 2;
            if !corner {
                out.push(TreeBlock {
                    dx: tip_x + dx,
                    dy: canopy_y,
                    dz: tip_z + dz,
                    id: block::ACACIA_LEAVES,
                });
            }
        }
    }
    out
}

/// Dark Oak — 6-9 tall, **2×2 trunk** (this is the visual signature),
/// canopy spreading ~3 blocks beyond the trunk on all sides.
fn place_dark_oak(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let height = height_for(WoodSpecies::DarkOak, x, z, seed);
    let mut out = Vec::new();
    // 2×2 trunk (occupies (0,0), (1,0), (0,1), (1,1)).
    for dy in 0..height {
        for tx in 0..=1 {
            for tz in 0..=1 {
                out.push(TreeBlock {
                    dx: tx, dy, dz: tz, id: block::DARK_OAK_LOG,
                });
            }
        }
    }
    // Dense low canopy: 5×5 at top minus 1 from trunk edges, then 7×7
    // at top+1 (the spread).
    let top = height;
    // Top + 1 — extra spread layer.
    for dx in -2..=3 {
        for dz in -2..=3 {
            let outer_corner = (dx == -2 || dx == 3) && (dz == -2 || dz == 3);
            if !outer_corner {
                out.push(TreeBlock { dx, dy: top + 1, dz, id: block::DARK_OAK_LEAVES });
            }
        }
    }
    // Top — 6×6 layer.
    for dx in -2..=3 {
        for dz in -2..=3 {
            let in_trunk = (0..=1).contains(&dx) && (0..=1).contains(&dz);
            if !in_trunk {
                out.push(TreeBlock { dx, dy: top, dz, id: block::DARK_OAK_LEAVES });
            }
        }
    }
    out
}

/// Rubber — Hevea-style: tall (10-14) thin trunk, sparse rounded
/// canopy in the upper 4 layers. Intentionally smaller canopy than
/// Jungle so Rubber trees stay distinct in the same biome.
fn place_rubber(x: i32, z: i32, seed: u32) -> Vec<TreeBlock> {
    let mut out: Vec<TreeBlock> = Vec::new();
    let height = height_for(WoodSpecies::Rubber, x, z, seed);
    // Trunk
    for dy in 0..height {
        out.push(TreeBlock {
            dx: 0,
            dy,
            dz: 0,
            id: block::RUBBER_LOG,
        });
    }
    // Canopy — radius-2 disc on top 3 layers (smaller hat on top).
    let canopy_start = height - 4;
    for dy in canopy_start..height {
        let r: i32 = if dy == height - 1 { 1 } else { 2 };
        for dx in -r..=r {
            for dz in -r..=r {
                if dx == 0 && dz == 0 {
                    continue; // trunk
                }
                if (dx * dx + dz * dz) > r * r {
                    continue; // round it
                }
                out.push(TreeBlock {
                    dx,
                    dy,
                    dz,
                    id: block::RUBBER_LEAVES,
                });
            }
        }
    }
    // Crown leaf on top of trunk.
    out.push(TreeBlock {
        dx: 0,
        dy: height,
        dz: 0,
        id: block::RUBBER_LEAVES,
    });
    let _ = x;
    let _ = z;
    let _ = seed;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every species' tree must have at least one log block and one
    /// leaves block.
    #[test]
    fn every_species_has_logs_and_leaves() {
        for s in block::ALL_WOOD_SPECIES {
            let tree = place_tree(*s, 0, 0, 42);
            let logs = tree
                .iter()
                .filter(|b| b.id == block::log_block_for(*s))
                .count();
            let leaves = tree
                .iter()
                .filter(|b| b.id == block::leaves_block_for(*s))
                .count();
            assert!(logs > 0, "{:?} has no logs", s);
            assert!(leaves > 0, "{:?} has no leaves", s);
        }
    }

    /// All blocks placed by a species' tree must use that species'
    /// block ids — no cross-contamination.
    #[test]
    fn species_uses_only_its_own_block_ids() {
        for s in block::ALL_WOOD_SPECIES {
            let own_log = block::log_block_for(*s);
            let own_leaves = block::leaves_block_for(*s);
            let tree = place_tree(*s, 100, 100, 42);
            for b in &tree {
                assert!(
                    b.id == own_log || b.id == own_leaves,
                    "{:?} tree placed foreign id {} (own log={}, own leaves={})",
                    s, b.id, own_log, own_leaves
                );
            }
        }
    }

    /// Trees are deterministic per `(species, x, z, seed)`.
    #[test]
    fn shape_is_deterministic_per_seed() {
        for s in block::ALL_WOOD_SPECIES {
            let a = place_tree(*s, 100, 200, 42);
            let b = place_tree(*s, 100, 200, 42);
            assert_eq!(a, b, "{:?} not deterministic", s);
        }
    }

    /// Different seeds at the same position should produce at least
    /// one variant — confirms seed actually flows through.
    #[test]
    fn shape_varies_with_seed_for_height() {
        // Vary by height: gather heights for 50 seeds; expect ≥2 distinct.
        for s in block::ALL_WOOD_SPECIES {
            let mut heights = std::collections::HashSet::new();
            for seed in 0..50u32 {
                let tree = place_tree(*s, 0, 0, seed);
                let max_y = tree.iter().map(|b| b.dy).max().unwrap_or(0);
                heights.insert(max_y);
            }
            assert!(heights.len() >= 2,
                "{:?} produced only 1 distinct height across 50 seeds", s);
        }
    }

    /// Spruce should be taller than Oak on average — the conical
    /// silhouette depends on Spruce reaching higher.
    #[test]
    fn spruce_is_generally_taller_than_oak() {
        let mut spruce_sum = 0;
        let mut oak_sum = 0;
        for seed in 0..50u32 {
            let s = place_tree(WoodSpecies::Spruce, 0, 0, seed);
            let o = place_tree(WoodSpecies::Oak, 0, 0, seed);
            spruce_sum += s.iter().map(|b| b.dy).max().unwrap_or(0);
            oak_sum += o.iter().map(|b| b.dy).max().unwrap_or(0);
        }
        assert!(spruce_sum > oak_sum, "spruce {} not taller than oak {}", spruce_sum, oak_sum);
    }

    /// Dark Oak has a 2×2 trunk — verify at least 4 log blocks at y=0.
    #[test]
    fn dark_oak_trunk_is_two_by_two() {
        let tree = place_tree(WoodSpecies::DarkOak, 0, 0, 42);
        let trunk_at_y0: Vec<_> = tree
            .iter()
            .filter(|b| b.dy == 0 && b.id == block::DARK_OAK_LOG)
            .collect();
        assert_eq!(trunk_at_y0.len(), 4, "expected 4 trunk blocks at y=0, got {}", trunk_at_y0.len());
    }

    /// Acacia canopy lands off-centre from the trunk — the umbrella
    /// motif. Verify at least one canopy block has |dx| ≥ 2 or |dz| ≥ 2.
    #[test]
    fn acacia_canopy_is_offset_from_trunk() {
        let tree = place_tree(WoodSpecies::Acacia, 0, 0, 42);
        let canopy_offset = tree.iter().any(|b| {
            b.id == block::ACACIA_LEAVES && (b.dx.abs() >= 2 || b.dz.abs() >= 2)
        });
        assert!(canopy_offset, "acacia canopy should extend ≥2 from trunk centre");
    }

    /// Jungle trees should be among the tallest — height ≥ 8 minimum.
    #[test]
    fn jungle_has_minimum_height_eight() {
        for seed in 0..50u32 {
            let tree = place_tree(WoodSpecies::Jungle, 0, 0, seed);
            let max_y = tree.iter().map(|b| b.dy).max().unwrap_or(0);
            assert!(max_y >= 8, "jungle height {max_y} below 8 at seed {seed}");
        }
    }

    /// Spruce canopy should narrow with height (conical). The radius
    /// at the lower canopy layer should exceed the radius near the top.
    #[test]
    fn spruce_canopy_narrows_with_height() {
        let tree = place_tree(WoodSpecies::Spruce, 0, 0, 42);
        let leaves: Vec<_> = tree
            .iter()
            .filter(|b| b.id == block::SPRUCE_LEAVES)
            .collect();
        // Find lower-canopy and upper-canopy max radius.
        let max_y = leaves.iter().map(|b| b.dy).max().unwrap_or(0);
        let min_y = leaves.iter().map(|b| b.dy).min().unwrap_or(max_y);
        let lower_radius = leaves
            .iter()
            .filter(|b| b.dy == min_y)
            .map(|b| b.dx.abs().max(b.dz.abs()))
            .max()
            .unwrap_or(0);
        let upper_radius = leaves
            .iter()
            .filter(|b| b.dy == max_y)
            .map(|b| b.dx.abs().max(b.dz.abs()))
            .max()
            .unwrap_or(0);
        assert!(
            lower_radius >= upper_radius,
            "spruce should narrow with height (lower r={}, upper r={})",
            lower_radius, upper_radius
        );
    }

    /// No tree should be absurdly large — sanity guard against shape
    /// bugs that explode the block count. 700 is the ceiling: tall
    /// Spruce + dense Dark Oak both legitimately exceed 400.
    #[test]
    fn no_tree_exceeds_700_blocks() {
        for s in block::ALL_WOOD_SPECIES {
            for seed in 0..10u32 {
                let tree = place_tree(*s, 0, 0, seed);
                assert!(
                    tree.len() < 700,
                    "{:?} tree exploded to {} blocks at seed {seed}",
                    s, tree.len()
                );
            }
        }
    }

    /// No tree should be empty — every species produces at least 5 blocks.
    #[test]
    fn every_tree_has_at_least_five_blocks() {
        for s in block::ALL_WOOD_SPECIES {
            for seed in 0..10u32 {
                let tree = place_tree(*s, 0, 0, seed);
                assert!(
                    tree.len() >= 5,
                    "{:?} tree too sparse ({}) at seed {seed}",
                    s, tree.len()
                );
            }
        }
    }
}
