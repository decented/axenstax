//! Fishing (P6 gap-closure).
//!
//! Cast a Fishing Rod at water → wait a randomised time → a bite → right-click
//! again to reel in a catch. The catch is mostly Raw Fish (cook it on a
//! campfire), occasionally a double, rarely a bit of junk treasure.
//!
//! State is the ephemeral per-player [`FishingLine`] (not persisted — a cast in
//! progress doesn't survive a save, same as other transient player state). The
//! wait timer + loot roll are pure functions here so they're unit-testable; the
//! `game_loop` owns the rod right-click, the per-tick bite, and the reel-in.

use crate::item::{ItemStack, MaterialId};

/// Shortest wait before a bite (5 s @ 20 TPS).
pub const MIN_WAIT_TICKS: u64 = 100;
/// Longest wait before a bite (15 s @ 20 TPS).
pub const MAX_WAIT_TICKS: u64 = 300;

/// An in-progress cast. `hooked` flips true once `catch_at_tick` is reached;
/// the player then right-clicks again to land the catch.
#[derive(Clone, Copy, Debug)]
pub struct FishingLine {
    pub catch_at_tick: u64,
    pub hooked: bool,
}

/// Randomised wait (ticks) until a bite. Seeded so it's deterministic +
/// replay-stable (typically `tick ^ player-something`).
pub fn wait_ticks(seed: u64) -> u64 {
    let span = MAX_WAIT_TICKS - MIN_WAIT_TICKS + 1;
    MIN_WAIT_TICKS + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) % span
}

/// Roll a catch. ~70% one Raw Fish, ~20% two, ~7% a Bone (driftwood/junk
/// stand-in), ~3% Leather (the classic "old boot"). Pure + seeded.
pub fn roll_catch(seed: u64) -> ItemStack {
    let r = seed.wrapping_mul(0x2545_F491_4F6C_DD1D) % 100;
    if r < 70 {
        ItemStack::new_material(MaterialId::RawFish, 1)
    } else if r < 90 {
        ItemStack::new_material(MaterialId::RawFish, 2)
    } else if r < 97 {
        ItemStack::new_material(MaterialId::Bone, 1)
    } else {
        ItemStack::new_material(MaterialId::Leather, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_is_always_in_range() {
        for seed in 0..1000u64 {
            let w = wait_ticks(seed);
            assert!((MIN_WAIT_TICKS..=MAX_WAIT_TICKS).contains(&w), "seed {seed} -> {w}");
        }
    }

    #[test]
    fn catch_is_mostly_fish_but_varied() {
        let mut fish = 0;
        let mut junk = 0;
        for seed in 0..1000u64 {
            let c = roll_catch(seed);
            match c.item {
                crate::item::Item::Material(MaterialId::RawFish) => fish += 1,
                crate::item::Item::Material(MaterialId::Bone)
                | crate::item::Item::Material(MaterialId::Leather) => junk += 1,
                other => panic!("unexpected catch {other:?}"),
            }
        }
        // Fish dominate; junk shows up but is the minority.
        assert!(fish > junk, "fish ({fish}) should dominate junk ({junk})");
        assert!(junk > 0, "some junk should appear across 1000 rolls");
    }

    #[test]
    fn double_fish_sometimes_drops_two() {
        let saw_two = (0..1000u64).any(|s| roll_catch(s).count == 2);
        assert!(saw_two, "a double-fish catch should occur across 1000 rolls");
    }
}
