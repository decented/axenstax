//! Historical Pivot Sub-Foundation 2 (HP-2) — Bear AI.
//!
//! Bears are Forest/Taiga territorial omnivores. State machine:
//!
//!  - `Wander` — default ambient movement.
//!  - `SmellFood(target)` — a food source (mature crop or food-stocked
//!    Chest) within 12 blocks; walks toward it.
//!  - `EatCrop(pos)` — at a mature crop, 40 ticks to chew; reverts the
//!    block to TilledSoil and gains 60-second satiety.
//!  - `RaidChest(pos)` — at a chest containing food, 60 ticks to chew;
//!    removes ONE food item from the chest and gains 90-second satiety.
//!  - `Aggro(attacker)` — hit by player; 30-second revenge window.
//!
//! Scans throttled to once-per-second per Bear (20-tick cadence).
//!
//! Pure-function shape: state transitions take snapshots so the AI
//! tests don't need a full ECS. The live wire-up (per-tick scheduler,
//! mutation application) lands in the mob_ai module's per-species
//! dispatcher.
//!
//! Spec: `docs/foundations/2026-05-22-historical-pivot-wild-animals.md`.

use serde::{Deserialize, Serialize};

/// 12-block cube scan radius for food sources.
pub const FOOD_SCAN_RADIUS: i32 = 12;

/// 20-tick (= 1 s @ 20 TPS) cadence between food scans, per Bear.
pub const FOOD_SCAN_PERIOD_TICKS: u32 = 20;

/// Crop-chew duration in ticks. 40 ticks = 2 s.
pub const EAT_CROP_TICKS: u32 = 40;

/// Chest-raid duration in ticks. 60 ticks = 3 s.
pub const RAID_CHEST_TICKS: u32 = 60;

/// Satiety after eating a crop, in ticks. 1200 ticks = 60 s.
pub const SATIETY_AFTER_CROP_TICKS: u32 = 1200;

/// Satiety after raiding a chest, in ticks. 1800 ticks = 90 s.
pub const SATIETY_AFTER_RAID_TICKS: u32 = 1800;

/// Aggro window after being hit. 30 s = 600 ticks.
pub const AGGRO_DURATION_TICKS: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BearAiState {
    Wander,
    SmellFood { target: (i32, i32, i32), kind: FoodKind },
    EatCrop { pos: (i32, i32, i32), ticks_remaining: u32 },
    RaidChest { pos: (i32, i32, i32), ticks_remaining: u32 },
    /// Hit by a player — chase `attacker_pidx` (the player-slot index that
    /// landed the hit) for `ticks_remaining` more ticks. Not a raw ECS
    /// entity id: players aren't ECS entities in this engine, and a plain
    /// slot index carries no stale-reference risk across a tick even
    /// though (per `entity.rs`) wild Bears/Hyenas aren't save-persisted at
    /// all, so there's no reload hazard either.
    Aggro { ticks_remaining: u32, attacker_pidx: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoodKind {
    Crop,
    Chest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BearData {
    pub state: BearAiState,
    /// Decrements per tick. While > 0 the Bear stays in `Wander` only.
    pub satiety_ticks_remaining: u32,
    /// Tick of the most recent food scan. Throttle the 12-block scan
    /// to once-per-second per Bear to keep cost predictable.
    pub last_scan_tick: u64,
}

impl Default for BearData {
    fn default() -> Self {
        Self {
            state: BearAiState::Wander,
            satiety_ticks_remaining: 0,
            last_scan_tick: 0,
        }
    }
}

impl BearData {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when satiety hasn't expired yet — Bear ignores food scans.
    pub fn is_sated(&self) -> bool {
        self.satiety_ticks_remaining > 0
    }

    /// Tick the satiety counter down by 1. Saturating — never wraps.
    pub fn tick_satiety(&mut self) {
        self.satiety_ticks_remaining = self.satiety_ticks_remaining.saturating_sub(1);
    }

    /// True if this Bear is currently in an aggro window. `dispatch_bears`
    /// (species_ai.rs) matches `BearAiState::Aggro { .. }` directly rather
    /// than calling this (it needs `attacker_pidx` out of the match), so
    /// this accessor stays test-only for now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_aggro(&self) -> bool {
        matches!(self.state, BearAiState::Aggro { .. })
    }
}

/// Pure block predicate — is this BlockId a mature/harvest-ready crop
/// stage that a Bear would eat?
///
/// Mirrors the spec's "mature crop" list: Wheat/Carrot/Potato/Corn at
/// stage 3, SugarBeet/Beetroot at stage 3, Pumpkin/PumpkinStem mature,
/// Berry Bush at stage 3. Implementer enumerates ripe-stage block
/// constants per the spec; this is the canonical list.
pub fn is_mature_crop(id: crate::block::BlockId) -> bool {
    use crate::block::*;
    matches!(
        id,
        WHEAT_STAGE_3
            | CARROT_STAGE_3
            | POTATO_STAGE_3
            | CORN_STAGE_3
            | SUGAR_BEET_STAGE_3
            | BEETROOT_STAGE_3
            | PUMPKIN
            | PUMPKIN_STEM_4
            | BERRY_BUSH_3
    )
}

/// Snapshot a Bear takes when scanning its surroundings for food. Pure
/// inputs → pure outputs so the scan logic is testable without a full
/// ECS.
///
/// `bear_pos` is the Bear's foot position (rounded to block coords).
/// `world` provides chunk reads + chest lookups.
///
/// Returns the nearest qualifying food target (by Chebyshev distance to
/// the Bear), or None if nothing in the 12-block cube qualifies.
pub fn find_nearest_food(
    world: &crate::world::World,
    bear_pos: (i32, i32, i32),
) -> Option<((i32, i32, i32), FoodKind)> {
    let (bx, by, bz) = bear_pos;
    let r = FOOD_SCAN_RADIUS;
    let mut best: Option<((i32, i32, i32), FoodKind, i32)> = None;
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                let x = bx + dx;
                let y = by + dy;
                let z = bz + dz;
                let id = world.get_block(x, y, z);
                let kind = if is_mature_crop(id) {
                    Some(FoodKind::Crop)
                } else if id == crate::block::CHEST {
                    world.chest_at((x, y, z))
                        .filter(|c| c.has_food())
                        .map(|_| FoodKind::Chest)
                } else {
                    None
                };
                if let Some(k) = kind {
                    let dist = dx.abs().max(dy.abs()).max(dz.abs());
                    if best.is_none_or(|(_, _, d)| dist < d) {
                        best = Some(((x, y, z), k, dist));
                    }
                }
            }
        }
    }
    best.map(|(p, k, _)| (p, k))
}

/// Damage application — applied to a Bear by a player attack. Transitions
/// to Aggro (targeting the hitting player's slot) and resets the 30 s
/// timer. Pure on `BearData`. Wired from `combat::player_attack` (melee)
/// and `entity::tick_projectiles` (arrows) via
/// `combat::notify_hit_bear_or_hyena`.
pub fn on_hit_by_player(bear: &mut BearData, attacker_pidx: usize) {
    bear.state = BearAiState::Aggro { ticks_remaining: AGGRO_DURATION_TICKS, attacker_pidx };
}

/// Tick the Bear's state machine forward by one tick. Pure on
/// `(BearData, &World)` — caller applies the returned `BearTickEffect`
/// to the world after the call (the AI doesn't take a `&mut World` so
/// borrow tangles in the per-tick dispatch loop stay manageable).
///
/// The world snapshot drives state transitions; the returned effect is
/// the mutation to apply. `current_tick` lets the scanner throttle.
pub fn tick(
    bear: &mut BearData,
    world: &crate::world::World,
    bear_pos: (i32, i32, i32),
    current_tick: u64,
) -> BearTickEffect {
    bear.tick_satiety();

    if let BearAiState::Aggro { ticks_remaining, .. } = &mut bear.state {
        if *ticks_remaining > 0 {
            *ticks_remaining -= 1;
        }
        if *ticks_remaining == 0 {
            bear.state = BearAiState::Wander;
        }
        return BearTickEffect::None;
    }

    if bear.is_sated() {
        bear.state = BearAiState::Wander;
        return BearTickEffect::None;
    }

    match bear.state {
        BearAiState::EatCrop { pos, mut ticks_remaining } => {
            // If the crop has already gone (mined, eaten by another) drop
            // back to Wander immediately.
            if !is_mature_crop(world.get_block(pos.0, pos.1, pos.2)) {
                bear.state = BearAiState::Wander;
                return BearTickEffect::None;
            }
            if ticks_remaining > 0 {
                ticks_remaining -= 1;
                bear.state = BearAiState::EatCrop { pos, ticks_remaining };
                return BearTickEffect::None;
            }
            // Chew complete — revert to TilledSoil + gain satiety.
            bear.satiety_ticks_remaining = SATIETY_AFTER_CROP_TICKS;
            bear.state = BearAiState::Wander;
            BearTickEffect::ConsumeCrop { pos }
        }
        BearAiState::RaidChest { pos, mut ticks_remaining } => {
            // Chest gone or emptied → bail.
            let chest_present = world.chest_at(pos).is_some_and(|c| c.has_food());
            if !chest_present {
                bear.state = BearAiState::Wander;
                return BearTickEffect::None;
            }
            if ticks_remaining > 0 {
                ticks_remaining -= 1;
                bear.state = BearAiState::RaidChest { pos, ticks_remaining };
                return BearTickEffect::None;
            }
            bear.satiety_ticks_remaining = SATIETY_AFTER_RAID_TICKS;
            bear.state = BearAiState::Wander;
            BearTickEffect::RaidChestOnce { pos }
        }
        BearAiState::SmellFood { target, kind } => {
            // Arrived? Adjacent (chebyshev ≤1) counts.
            let d = (target.0 - bear_pos.0).abs().max((target.1 - bear_pos.1).abs()).max((target.2 - bear_pos.2).abs());
            if d <= 1 {
                match kind {
                    FoodKind::Crop if is_mature_crop(world.get_block(target.0, target.1, target.2)) => {
                        bear.state = BearAiState::EatCrop { pos: target, ticks_remaining: EAT_CROP_TICKS };
                    }
                    FoodKind::Chest if world.chest_at(target).is_some_and(|c| c.has_food()) => {
                        bear.state = BearAiState::RaidChest { pos: target, ticks_remaining: RAID_CHEST_TICKS };
                    }
                    _ => {
                        // Target gone — re-scan.
                        bear.state = BearAiState::Wander;
                    }
                }
            }
            BearTickEffect::None
        }
        BearAiState::Wander => {
            // Throttle scans to once per second per Bear.
            if current_tick - bear.last_scan_tick < FOOD_SCAN_PERIOD_TICKS as u64 {
                return BearTickEffect::None;
            }
            bear.last_scan_tick = current_tick;
            if let Some((target, kind)) = find_nearest_food(world, bear_pos) {
                bear.state = BearAiState::SmellFood { target, kind };
            }
            BearTickEffect::None
        }
        BearAiState::Aggro { .. } => BearTickEffect::None,
    }
}

/// World-mutation request returned from `tick`. The caller resolves
/// these against the live `&mut World`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BearTickEffect {
    None,
    /// Replace the crop at `pos` with TilledSoil.
    ConsumeCrop { pos: (i32, i32, i32) },
    /// Remove one food item from the chest at `pos`.
    RaidChestOnce { pos: (i32, i32, i32) },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::chest::ChestData;
    use crate::item::{ItemStack, MaterialId};
    use crate::world::World;

    fn world_with_crop_at(pos: (i32, i32, i32), id: block::BlockId) -> World {
        let mut w = World::new();
        w.set_block(pos.0, pos.1, pos.2, id);
        w
    }

    #[test]
    fn defaults_are_wander_unsated_no_aggro() {
        let b = BearData::new();
        assert!(matches!(b.state, BearAiState::Wander));
        assert!(!b.is_sated());
        assert!(!b.is_aggro());
        assert_eq!(b.last_scan_tick, 0);
    }

    #[test]
    fn is_mature_crop_covers_known_ripe_stages() {
        assert!(is_mature_crop(block::WHEAT_STAGE_3));
        assert!(is_mature_crop(block::CARROT_STAGE_3));
        assert!(is_mature_crop(block::POTATO_STAGE_3));
        assert!(is_mature_crop(block::CORN_STAGE_3));
        assert!(is_mature_crop(block::SUGAR_BEET_STAGE_3));
        assert!(is_mature_crop(block::BEETROOT_STAGE_3));
        assert!(is_mature_crop(block::PUMPKIN));
        assert!(is_mature_crop(block::PUMPKIN_STEM_4));
        assert!(is_mature_crop(block::BERRY_BUSH_3));

        assert!(!is_mature_crop(block::WHEAT_STAGE_0));
        assert!(!is_mature_crop(block::CARROT_STAGE_1));
        assert!(!is_mature_crop(block::TILLED_SOIL));
        assert!(!is_mature_crop(block::STONE));
    }

    #[test]
    fn find_nearest_food_locates_mature_crop_within_range() {
        let w = world_with_crop_at((10, 64, 0), block::WHEAT_STAGE_3);
        let found = find_nearest_food(&w, (0, 64, 0));
        assert_eq!(found, Some(((10, 64, 0), FoodKind::Crop)));
    }

    #[test]
    fn find_nearest_food_ignores_immature_crop() {
        let w = world_with_crop_at((5, 64, 0), block::WHEAT_STAGE_2);
        assert_eq!(find_nearest_food(&w, (0, 64, 0)), None);
    }

    #[test]
    fn find_nearest_food_ignores_crop_beyond_radius() {
        let w = world_with_crop_at((20, 64, 0), block::WHEAT_STAGE_3);
        assert_eq!(find_nearest_food(&w, (0, 64, 0)), None);
    }

    #[test]
    fn find_nearest_food_prefers_closer_target() {
        let mut w = World::new();
        w.set_block(11, 64, 0, block::WHEAT_STAGE_3);
        w.set_block(3, 64, 0, block::CARROT_STAGE_3);
        let found = find_nearest_food(&w, (0, 64, 0)).unwrap();
        assert_eq!(found.0, (3, 64, 0));
    }

    #[test]
    fn find_nearest_food_picks_chest_with_food() {
        let mut w = World::new();
        w.set_block(4, 64, 0, block::CHEST);
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 1));
        w.insert_chest((4, 64, 0), c);
        let found = find_nearest_food(&w, (0, 64, 0));
        assert_eq!(found, Some(((4, 64, 0), FoodKind::Chest)));
    }

    #[test]
    fn find_nearest_food_ignores_empty_chest() {
        let mut w = World::new();
        w.set_block(4, 64, 0, block::CHEST);
        w.insert_chest((4, 64, 0), ChestData::new());
        assert_eq!(find_nearest_food(&w, (0, 64, 0)), None);
    }

    #[test]
    fn find_nearest_food_ignores_chest_with_only_non_food() {
        let mut w = World::new();
        w.set_block(4, 64, 0, block::CHEST);
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::IronIngot, 1));
        w.insert_chest((4, 64, 0), c);
        assert_eq!(find_nearest_food(&w, (0, 64, 0)), None);
    }

    #[test]
    fn satiety_blocks_food_smell() {
        let w = world_with_crop_at((2, 64, 0), block::WHEAT_STAGE_3);
        let mut b = BearData::new();
        b.satiety_ticks_remaining = 100;
        // tick at t=100 forces a scan (last_scan_tick = 0 → delta ≥ 20).
        let _ = tick(&mut b, &w, (0, 64, 0), 100);
        // Should stay in Wander — satiety blocks scan.
        assert!(matches!(b.state, BearAiState::Wander));
    }

    #[test]
    fn tick_unsated_transitions_to_smell_food_when_crop_nearby() {
        let w = world_with_crop_at((2, 64, 0), block::WHEAT_STAGE_3);
        let mut b = BearData::new();
        let _ = tick(&mut b, &w, (0, 64, 0), 20);
        assert!(matches!(b.state, BearAiState::SmellFood { target: (2, 64, 0), kind: FoodKind::Crop }));
    }

    #[test]
    fn smell_food_transitions_to_eat_when_adjacent_to_crop() {
        let w = world_with_crop_at((1, 64, 0), block::WHEAT_STAGE_3);
        let mut b = BearData::new();
        b.state = BearAiState::SmellFood { target: (1, 64, 0), kind: FoodKind::Crop };
        // Bear at (0,64,0) → chebyshev distance 1 → arrived.
        let eff = tick(&mut b, &w, (0, 64, 0), 1);
        assert_eq!(eff, BearTickEffect::None);
        assert!(matches!(b.state, BearAiState::EatCrop { pos: (1, 64, 0), .. }));
    }

    #[test]
    fn eat_crop_completes_after_40_ticks_and_emits_consume() {
        let w = world_with_crop_at((0, 64, 0), block::WHEAT_STAGE_3);
        let mut b = BearData::new();
        b.state = BearAiState::EatCrop { pos: (0, 64, 0), ticks_remaining: 1 };
        let _ = tick(&mut b, &w, (0, 64, 0), 1);
        // ticks_remaining went 1→0 this tick → still chewing.
        assert!(matches!(b.state, BearAiState::EatCrop { ticks_remaining: 0, .. }));
        // Next tick: ticks_remaining is 0 already → consume fires.
        let eff = tick(&mut b, &w, (0, 64, 0), 2);
        assert_eq!(eff, BearTickEffect::ConsumeCrop { pos: (0, 64, 0) });
        assert!(matches!(b.state, BearAiState::Wander));
        // tick_satiety runs at the top of tick (decrements 0→0), then
        // consume sets satiety to the full 1200 — no post-decrement.
        assert_eq!(b.satiety_ticks_remaining, SATIETY_AFTER_CROP_TICKS);
    }

    #[test]
    fn raid_chest_emits_one_food_pull_and_sets_long_satiety() {
        let mut w = World::new();
        w.set_block(0, 64, 0, block::CHEST);
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 3));
        w.insert_chest((0, 64, 0), c);
        let mut b = BearData::new();
        b.state = BearAiState::RaidChest { pos: (0, 64, 0), ticks_remaining: 0 };
        let eff = tick(&mut b, &w, (0, 64, 0), 1);
        assert_eq!(eff, BearTickEffect::RaidChestOnce { pos: (0, 64, 0) });
        assert!(matches!(b.state, BearAiState::Wander));
        assert!(b.satiety_ticks_remaining >= SATIETY_AFTER_RAID_TICKS - 1);
    }

    #[test]
    fn on_hit_by_player_sets_aggro_window() {
        let mut b = BearData::new();
        on_hit_by_player(&mut b, 2);
        assert!(b.is_aggro());
        match b.state {
            BearAiState::Aggro { ticks_remaining, attacker_pidx } => {
                assert_eq!(ticks_remaining, AGGRO_DURATION_TICKS);
                assert_eq!(attacker_pidx, 2, "should target the player slot that landed the hit");
            }
            _ => panic!("expected Aggro state"),
        }
    }

    #[test]
    fn aggro_decays_and_returns_to_wander() {
        let w = World::new();
        let mut b = BearData::new();
        b.state = BearAiState::Aggro { ticks_remaining: 2, attacker_pidx: 0 };
        let _ = tick(&mut b, &w, (0, 64, 0), 1);
        assert!(matches!(b.state, BearAiState::Aggro { ticks_remaining: 1, .. }));
        // Next tick: ticks_remaining 1→0 and within the same tick the
        // 0-check fires → state flips to Wander in one step.
        let _ = tick(&mut b, &w, (0, 64, 0), 2);
        assert!(matches!(b.state, BearAiState::Wander));
    }

    #[test]
    fn aggro_preserves_attacker_pidx_across_ticks() {
        // The attacker identity must survive the countdown unchanged —
        // the movement layer (species_ai::dispatch_bears) reads it every
        // tick to know who to chase.
        let w = World::new();
        let mut b = BearData::new();
        on_hit_by_player(&mut b, 3);
        let _ = tick(&mut b, &w, (0, 64, 0), 1);
        match b.state {
            BearAiState::Aggro { attacker_pidx, .. } => assert_eq!(attacker_pidx, 3),
            _ => panic!("expected Aggro state to persist"),
        }
    }

    #[test]
    fn bear_raid_chest_takes_one_food_leaves_chest_intact_and_iron_untouched() {
        // Place a chest with [Bread, IronIngot] adjacent to the Bear.
        // Drive the AI through SmellFood → RaidChest → consume cycle.
        // Verify: bread gone, iron untouched, chest block still present,
        // Bear sated.
        let mut world = World::new();
        world.set_block(1, 64, 0, crate::block::CHEST);
        let mut c = ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 1));
        c.slots[1] = Some(ItemStack::new_material(MaterialId::IronIngot, 1));
        world.insert_chest((1, 64, 0), c);

        let mut bear = BearData::new();
        // Tick at t=20 → scan fires → SmellFood targets the chest.
        let _ = tick(&mut bear, &world, (0, 64, 0), 20);
        assert!(matches!(bear.state, BearAiState::SmellFood { kind: FoodKind::Chest, .. }));

        // Tick at t=21 → bear is adjacent (chebyshev=1) → enter RaidChest.
        let _ = tick(&mut bear, &world, (0, 64, 0), 21);
        assert!(matches!(bear.state, BearAiState::RaidChest { .. }));

        // Drive the timer to 0 and the consume tick.
        for t in 22..=82 {
            let eff = tick(&mut bear, &world, (0, 64, 0), t);
            if let BearTickEffect::RaidChestOnce { pos } = eff {
                assert_eq!(pos, (1, 64, 0));
                // Apply the world mutation the caller would normally do.
                let chest = world.chest_at_mut((1, 64, 0)).unwrap();
                let taken = chest.take_one_food().expect("food present");
                assert!(matches!(taken.item, crate::item::Item::Material(MaterialId::Bread)));
                break;
            }
        }
        // Post-raid: bread gone, iron remains, chest block + entity intact, Bear sated.
        let chest_after = world.chest_at((1, 64, 0)).expect("chest still present");
        assert!(chest_after.slots[0].is_none(), "bread eaten");
        assert!(chest_after.slots[1].as_ref().is_some_and(|s| matches!(s.item,
            crate::item::Item::Material(MaterialId::IronIngot))), "iron untouched");
        assert_eq!(world.get_block(1, 64, 0), crate::block::CHEST, "chest block intact");
        assert!(bear.is_sated());
    }

    #[test]
    fn wall_between_bear_and_crop_does_not_block_scan_but_documents_pathing() {
        // HP-2 spec says "Bear pathfinding treats blocks marked solid
        // as impassable." The pure scan IS line-of-sight-agnostic by
        // design — it surfaces what's in range; the live pathfinder is
        // the layer that respects walls. Document the contract so a
        // future change to find_nearest_food is intentional.
        let mut world = World::new();
        world.set_block(5, 64, 0, crate::block::WHEAT_STAGE_3);
        world.set_block(2, 64, 0, crate::block::COBBLESTONE); // wall
        let found = find_nearest_food(&world, (0, 64, 0));
        assert!(found.is_some(), "scan is line-of-sight-agnostic by design");
    }

    #[test]
    fn scan_throttle_skips_scan_within_one_second() {
        let w = world_with_crop_at((2, 64, 0), block::WHEAT_STAGE_3);
        let mut b = BearData::new();
        b.last_scan_tick = 100;
        // Next tick at 101 → delta 1 < 20, scan skipped.
        let _ = tick(&mut b, &w, (0, 64, 0), 101);
        assert!(matches!(b.state, BearAiState::Wander));
        // At 120 (delta 20) → scan fires.
        let _ = tick(&mut b, &w, (0, 64, 0), 120);
        assert!(matches!(b.state, BearAiState::SmellFood { .. }));
    }
}
