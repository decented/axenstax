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
//!
//! C3c-2 (protocol v81) — the rules are shared by every seat: the cast's water
//! ray ([`finds_water`]), the bite's wait ([`wait_ticks`]) and the catch
//! ([`roll_catch`]). A joiner's cast and reel are requests (`ItemAction::Cast`,
//! `Reel`): the SERVER casts from its own position for the player, draws the
//! wait and rolls the catch on its own seed, and grants the catch; the client
//! only shows the line and the bite (`ItemActionOutcomePacket::bite_after`).
//! A landed catch wears the rod by `Inventory::use_tool_at` on every seat, so
//! a rod at 0 breaks like every other tool; on a seat that owns its inventory
//! a catch that doesn't fit drops at the player ([`reel_in`]).

use glam::Vec3;

use crate::item::{ItemStack, MaterialId};

/// The cast's aim ray: this many steps of [`CAST_STEP`] blocks from the eye
/// along the look (12 blocks).
pub const CAST_STEPS: u32 = 24;
/// One step of the cast's aim ray, in blocks.
pub const CAST_STEP: f32 = 0.5;

/// C3c-2 — how early a joiner's reel may reach the server and still land the
/// catch: the server's bite tick less this. An honest client shows the bite
/// `bite_after` ticks after the cast's outcome ARRIVES, so its reel reaches
/// the server at least a round trip after the server's bite; the slack only
/// absorbs a client clock running ahead of a loaded server's (one second).
/// A hooked line has no closing window (single-player's never did): it waits
/// until it is reeled in.
pub const REEL_SLACK_TICKS: u64 = 20;

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

/// Is there water along the cast from `eye` along `dir` (unit), within
/// [`CAST_STEPS`] steps? `is_water` reads the caster's world: single-player's
/// own, or the server's for a joiner's `Cast`.
pub fn finds_water(eye: Vec3, dir: Vec3, is_water: impl Fn(i32, i32, i32) -> bool) -> bool {
    (1..=CAST_STEPS).any(|step| {
        let p = eye + dir * (step as f32 * CAST_STEP);
        is_water(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    })
}

/// C3c-2 — has a line cast to bite at server tick `bite_tick` hooked by
/// server tick `now` (with [`REEL_SLACK_TICKS`])?
pub fn hooked(now: u64, bite_tick: u64) -> bool {
    now.saturating_add(REEL_SLACK_TICKS) >= bite_tick
}

/// What reeling in a hooked line did on a seat that owns its inventory.
#[derive(Debug)]
pub struct Reeled {
    /// What was caught.
    pub catch: ItemStack,
    /// The part of the catch the inventory had no room for: the caller drops
    /// it at the player (C3c-2: it used to vanish).
    pub leftover: Option<ItemStack>,
    /// The rod's wear (`Inventory::use_hotbar_tool`; C3c-2: a rod at 0
    /// breaks, as every tool does — it used to stay at 0 for ever).
    pub wear: Option<crate::inventory::ToolUseInfo>,
}

/// Single-player's and a host's seat's reel of a hooked line with the rod in
/// hotbar slot `hot`: the catch rolled from `seed` is added to `inv`, then the
/// rod wears.
pub fn reel_in(inv: &mut crate::inventory::Inventory, hot: usize, seed: u64) -> Reeled {
    let catch = roll_catch(seed);
    let leftover = inv.add_item(catch.clone());
    let wear = inv.use_hotbar_tool(hot);
    Reeled { catch, leftover, wear }
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
    fn the_cast_finds_water_within_twelve_blocks_of_the_eye() {
        let eye = Vec3::new(0.5, 70.5, 0.5);
        let dir = Vec3::new(1.0, 0.0, 0.0);
        let pond = |at: i32| move |x: i32, y: i32, z: i32| x == at && y == 70 && z == 0;
        assert!(finds_water(eye, dir, pond(5)));
        assert!(finds_water(eye, dir, pond(12)), "the last step reaches x = 12.5");
        assert!(!finds_water(eye, dir, pond(13)));
        assert!(!finds_water(eye, -dir, pond(5)), "behind the caster");
    }

    #[test]
    fn a_line_hooks_at_its_bite_less_the_slack_and_stays_hooked() {
        assert!(!hooked(100, 100 + REEL_SLACK_TICKS + 1));
        assert!(hooked(100, 100 + REEL_SLACK_TICKS));
        assert!(hooked(1_000_000, 100), "no closing window");
    }

    /// C3c-2 — single-player's reel: the rod wears by `use_hotbar_tool`, so
    /// a rod at 0 breaks; a catch that doesn't fit comes back to be dropped.
    #[test]
    fn a_rod_at_zero_breaks_and_a_full_bags_catch_comes_back() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::inventory::Inventory;
        let rod = |durability| ItemStack::new_tool(Tool { durability, ..Tool::new(ToolType::FishingRod, ToolMaterial::Wood) });
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(rod(5)));
        let r = reel_in(&mut inv, 0, 1);
        assert!(r.leftover.is_none());
        assert!(!r.wear.as_ref().unwrap().just_broke);
        assert_eq!(inv.slot(0), Some(&rod(4)), "worn once");
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(rod(0)));
        let r = reel_in(&mut inv, 0, 1);
        assert!(r.wear.unwrap().just_broke, "a rod at 0 breaks");
        assert!(inv.slot(0).is_none(), "and leaves the hand");
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(rod(9)));
        for k in 1..36 {
            inv.set_slot(k, Some(ItemStack::new_block(crate::block::STONE, 64)));
        }
        let r = reel_in(&mut inv, 0, 1);
        assert_eq!(r.leftover.as_ref(), Some(&r.catch), "the whole catch comes back to be dropped");
    }

    #[test]
    fn double_fish_sometimes_drops_two() {
        let saw_two = (0..1000u64).any(|s| roll_catch(s).count == 2);
        assert!(saw_two, "a double-fish catch should occur across 1000 rolls");
    }
}
