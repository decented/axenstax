//! Wave 1A — breeding genetics (inheritance).
//!
//! A `Genetics` component on breedable animals carries heritable traits. A
//! baby's genes are a per-gene coin-flip between its two parents plus a small
//! bounded mutation, so selectively breeding (keep the biggest / highest-yield
//! parents) shifts a herd over generations. This is the depth layer on top of
//! the shipped breeding system: it drives the "this one is mine" attachment
//! beat, a breed-for-value economy loop (higher `yield_q` → richer husbandry
//! products → Vendor value), and the Mule cross-breed showcase (Wave 5).
//!
//! Pure + deterministic: `breed()` and `wild()` are seeded free functions,
//! fully unit-testable with no ECS. The game_loop attaches the component; the
//! husbandry system (Wave 1B) reads `yield_q`; the renderer reads `size`/`tint`.
//!
//! Eventual crate home: `genesis_sim`.

use serde::{Deserialize, Serialize};

/// Heritable trait bundle. Small + `Copy` so it rides the ECS + the save infra
/// cheaply.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Genetics {
    /// Render + hitbox scale multiplier. Clamped to [`SIZE_MIN`, `SIZE_MAX`].
    pub size: f32,
    /// Subtle coat tint (per-channel multiplier ×255). Cosmetic variation.
    pub tint: [u8; 3],
    /// Husbandry product quality/quantity, 0..=255 (baseline 128). Higher =
    /// richer milk / more wool / better drops.
    pub yield_q: u8,
    /// Movement-speed quality, 0..=255 (baseline 128). Higher = a touch faster
    /// (matters for horses/mules/donkeys).
    pub speed_q: u8,
}

pub const SIZE_MIN: f32 = 0.80;
pub const SIZE_MAX: f32 = 1.20;
/// Per-gene mutation magnitude (fraction of the trait's range).
const SIZE_MUT: f32 = 0.04;
const Q_MUT: i32 = 12;
const TINT_MUT: i32 = 10;

impl Genetics {
    /// The unremarkable middle-of-the-road animal — what an un-bred / legacy
    /// animal defaults to.
    pub fn baseline() -> Self {
        Self { size: 1.0, tint: [255, 255, 255], yield_q: 128, speed_q: 128 }
    }

    /// A wild-spawned animal: baseline with a little seeded variation so no two
    /// look/perform identically and there's raw stock worth breeding from.
    pub fn wild(seed: u32) -> Self {
        let g = |salt: u32, spread: f32| -> f32 {
            let h = hash(seed, salt);
            (h as f32 / u32::MAX as f32 - 0.5) * 2.0 * spread // [-spread, spread]
        };
        let size = (1.0 + g(1, 0.12)).clamp(SIZE_MIN, SIZE_MAX);
        let tint_ch = |salt: u32| (235 + (hash(seed, salt) % 21) as i32).clamp(0, 255) as u8; // 235..=255
        let q = |salt: u32| (128.0 + g(salt, 0.0) + (hash(seed, salt) % 61) as f32 - 30.0)
            .clamp(0.0, 255.0) as u8; // 98..=158-ish
        Self {
            size,
            tint: [tint_ch(2), tint_ch(3), tint_ch(4)],
            yield_q: q(5),
            speed_q: q(6),
        }
    }
}

impl Default for Genetics {
    fn default() -> Self {
        Self::baseline()
    }
}

/// Hash for deterministic, replay-stable genetics (splitmix-style).
fn hash(seed: u32, salt: u32) -> u32 {
    let mut h = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (salt as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h & 0xFFFF_FFFF) as u32
}

/// Breed two parents into an offspring: each gene is inherited from one parent
/// (seeded coin-flip) then nudged by a small bounded mutation. Pure +
/// deterministic on `(a, b, seed)`.
pub fn breed(a: &Genetics, b: &Genetics, seed: u32) -> Genetics {
    let pick_f = |salt: u32, va: f32, vb: f32| if hash(seed, salt) & 1 == 0 { va } else { vb };
    let pick_u = |salt: u32, va: u8, vb: u8| if hash(seed, salt) & 1 == 0 { va } else { vb };

    // size: inherit + mutate within [MIN, MAX].
    let size_mut = (hash(seed, 10) as f32 / u32::MAX as f32 - 0.5) * 2.0 * SIZE_MUT;
    let size = (pick_f(1, a.size, b.size) + size_mut).clamp(SIZE_MIN, SIZE_MAX);

    // quality genes: inherit + integer mutation, clamped 0..=255.
    let mut_q = |salt: u32, base: u8| -> u8 {
        let m = (hash(seed, salt) % (2 * Q_MUT as u32 + 1)) as i32 - Q_MUT;
        (base as i32 + m).clamp(0, 255) as u8
    };
    let yield_q = mut_q(11, pick_u(2, a.yield_q, b.yield_q));
    let speed_q = mut_q(12, pick_u(3, a.speed_q, b.speed_q));

    // tint: per-channel inherit + small mutation.
    let mut_t = |salt: u32, base: u8| -> u8 {
        let m = (hash(seed, salt) % (2 * TINT_MUT as u32 + 1)) as i32 - TINT_MUT;
        (base as i32 + m).clamp(0, 255) as u8
    };
    let tint = [
        mut_t(20, pick_u(4, a.tint[0], b.tint[0])),
        mut_t(21, pick_u(5, a.tint[1], b.tint[1])),
        mut_t(22, pick_u(6, a.tint[2], b.tint[2])),
    ];

    Genetics { size, tint, yield_q, speed_q }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_is_neutral() {
        let g = Genetics::baseline();
        assert_eq!(g.size, 1.0);
        assert_eq!(g.yield_q, 128);
    }

    #[test]
    fn breed_is_deterministic() {
        let a = Genetics::wild(1);
        let b = Genetics::wild(2);
        assert_eq!(breed(&a, &b, 42), breed(&a, &b, 42), "same parents+seed → same child");
    }

    #[test]
    fn size_stays_in_bounds_across_many_seeds() {
        let a = Genetics { size: SIZE_MAX, ..Genetics::baseline() };
        let b = Genetics { size: SIZE_MAX, ..Genetics::baseline() };
        for s in 0..500 {
            let c = breed(&a, &b, s);
            assert!((SIZE_MIN..=SIZE_MAX).contains(&c.size), "size {} out of bounds at seed {s}", c.size);
        }
    }

    #[test]
    fn child_genes_trace_to_a_parent_or_a_small_mutation() {
        // yield_q must come from one parent ± Q_MUT — never wander far.
        let a = Genetics { yield_q: 200, ..Genetics::baseline() };
        let b = Genetics { yield_q: 60, ..Genetics::baseline() };
        for s in 0..500 {
            let c = breed(&a, &b, s);
            let near_a = (c.yield_q as i32 - 200).abs() <= Q_MUT;
            let near_b = (c.yield_q as i32 - 60).abs() <= Q_MUT;
            assert!(near_a || near_b, "child yield_q {} not within mutation of either parent (seed {s})", c.yield_q);
        }
    }

    #[test]
    fn selective_breeding_lifts_a_trait_over_generations() {
        // Always keep breeding the higher-yield line with a strong partner →
        // the population mean should climb above the wild baseline.
        let mut champ = Genetics { yield_q: 130, ..Genetics::baseline() };
        let partner = Genetics { yield_q: 150, ..Genetics::baseline() };
        let mut best = champ.yield_q;
        for round in 0..40u32 {
            // breed several offspring, keep the best (selective pressure).
            for k in 0..6u32 {
                let child = breed(&champ, &partner, round * 100 + k);
                if child.yield_q > best {
                    best = child.yield_q;
                    champ = child;
                }
            }
        }
        assert!(best > 150, "selective breeding should push yield_q above 150, got {best}");
    }

    #[test]
    fn wild_varies_but_stays_sane() {
        let mut sizes = Vec::new();
        for s in 0..200 {
            let g = Genetics::wild(s);
            assert!((SIZE_MIN..=SIZE_MAX).contains(&g.size));
            sizes.push(g.size);
        }
        // not all identical
        assert!(sizes.iter().any(|&s| (s - sizes[0]).abs() > 1e-3), "wild genetics should vary");
    }
}
