//! Spec 28d chunk 8 — Bee Hive block-entity.
//!
//! The Hive is the home + storage for bees. It holds two state
//! quantities:
//! - `bees_inside`: 0..=3 — how many bees are sheltered (post-flight
//!   or pre-departure). Live deposit/return wiring is playtest-gated;
//!   this module ships the data layer so the world-save format is
//!   locked and the right-click matrix works against fake state.
//! - `honey_level`: 0..=5 — how full the hive is. Bees increment this
//!   on return after pollinating; the player drains it via Bottle
//!   (1 → HoneyBottle) or Shears (1 → 3 × Honeycomb).
//!
//! Block-entity tagged onto `World::block_entities` via
//! `BlockEntityData::Hive`.

use serde::{Deserialize, Serialize};

use crate::item::{Item, ItemStack, MaterialId};

/// Maximum honey level. At 5 the hive is "full" — visual cue to drain it.
pub const MAX_HONEY_LEVEL: u8 = 5;

/// Maximum bees sheltered at once.
#[cfg_attr(not(test), allow(dead_code))]
pub const MAX_BEES_INSIDE: u8 = 3;

/// Honey accumulation cadence: every this many ticks, a hive with a bee
/// nearby gains one honey level (2026-07-04 — the live loop; the full
/// enter/leave-hive bee AI wiring can replace this without changing the
/// harvest side).
pub const HONEY_ACCUM_INTERVAL_TICKS: u64 = 1200;
/// A bee within this many blocks of a hive counts as working it.
pub const BEE_WORK_RADIUS: f32 = 8.0;

/// Is any bee within `radius` blocks of the hive cell? Pure.
pub fn bee_within(hive: (i32, i32, i32), bees: &[(f32, f32, f32)], radius: f32) -> bool {
    let (hx, hy, hz) = (
        hive.0 as f32 + 0.5,
        hive.1 as f32 + 0.5,
        hive.2 as f32 + 0.5,
    );
    let r2 = radius * radius;
    bees.iter().any(|&(bx, by, bz)| {
        let (dx, dy, dz) = (bx - hx, by - hy, bz - hz);
        dx * dx + dy * dy + dz * dz <= r2
    })
}

/// Honey accumulation sweep (2026-07-04; shared 2026-10-05, T1-3) — every
/// [`HONEY_ACCUM_INTERVAL_TICKS`], each hive with a bee working within
/// [`BEE_WORK_RADIUS`] gains one honey level. Gated on `tick` internally, so
/// callers run it on their block-machine cadence and it no-ops off-interval.
///
/// The ONE implementation: the client loop (single-player / LAN host) and the
/// dedicated server (`block_machines.rs`) both call this — never a copy.
pub fn accumulate_honey(world: &mut crate::world::World, ecs: &hecs::World, tick: u64) {
    if !tick.is_multiple_of(HONEY_ACCUM_INTERVAL_TICKS) {
        return;
    }
    let bees: Vec<(f32, f32, f32)> = ecs
        .query::<(&crate::entity::Position, &crate::entity::MobKind)>()
        .iter()
        .filter(|(_, (_, k))| k.0 == crate::mob::MobType::Bee)
        .map(|(_, (p, _))| (p.0.x, p.0.y, p.0.z))
        .collect();
    if bees.is_empty() {
        return;
    }
    let hive_positions: Vec<(i32, i32, i32)> = world.iter_hives().map(|(p, _)| p).collect();
    for hp in hive_positions {
        if bee_within(hp, &bees, BEE_WORK_RADIUS)
            && let Some(h) = world.hive_at_mut(hp)
        {
            h.deposit_honey();
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveData {
    pub bees_inside: u8,
    pub honey_level: u8,
}

impl HiveData {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_full(&self) -> bool {
        self.honey_level >= MAX_HONEY_LEVEL
    }

    pub fn is_empty(&self) -> bool {
        self.honey_level == 0
    }

    /// Bee returns from pollination — caller has already decided the
    /// trip succeeded (e.g., bee touched a mature crop). Increments
    /// honey level (clamped to MAX). Returns the new level for the
    /// caller to broadcast or render.
    pub fn deposit_honey(&mut self) -> u8 {
        if self.honey_level < MAX_HONEY_LEVEL {
            self.honey_level += 1;
        }
        self.honey_level
    }
}

/// Right-click outcome — what the player gets back from a hive
/// interaction. Pure: caller resolves inventory updates + tool durability.
///
/// Doesn't derive PartialEq because ItemStack doesn't. Tests use
/// `matches!` against the variant instead.
#[derive(Clone, Debug)]
pub enum HiveRightClickOutcome {
    /// Nothing changed (no tool, hive empty, etc.).
    Nothing,
    /// Hand the player an item; consume one input slot (Bucket → MilkBucket
    /// pattern); decrement honey level by 1. Caller handles inventory.
    Give { item: ItemStack },
    /// Hand the player an item; decrement tool durability by 1 (Shears
    /// path); decrement honey level by 1.
    GiveAndUseTool { item: ItemStack },
}

/// Resolve a right-click on a Hive. Hand-item determines the outcome
/// per the spec matrix:
/// - empty hand → Nothing (future: open status overlay)
/// - glass bottle / bucket → 1 HoneyBottle, requires honey_level >= 1
/// - shears → 3 Honeycomb, requires honey_level >= 1 + tool durability
///   handled by caller
/// - anything else → Nothing
///
/// Pure. Modifies `hive.honey_level` via `&mut`.
pub fn resolve_right_click(
    hive: &mut HiveData,
    held: Option<&Item>,
) -> HiveRightClickOutcome {
    let Some(item) = held else { return HiveRightClickOutcome::Nothing; };
    if hive.is_empty() {
        return HiveRightClickOutcome::Nothing;
    }
    match item {
        // Bucket → HoneyBottle (T1.5 pattern; future Glass Bottle
        // sits here too once the glass surface lands).
        Item::Material(MaterialId::Bucket) => {
            hive.honey_level = hive.honey_level.saturating_sub(1);
            HiveRightClickOutcome::Give {
                item: ItemStack::new_material(MaterialId::HoneyBottle, 1),
            }
        }
        // Shears → 3 Honeycomb. Caller decrements tool durability.
        Item::Tool(t) if matches!(t.tool_type, crate::crafting::ToolType::Shears) => {
            hive.honey_level = hive.honey_level.saturating_sub(1);
            HiveRightClickOutcome::GiveAndUseTool {
                item: ItemStack::new_material(MaterialId::Honeycomb, 3),
            }
        }
        _ => HiveRightClickOutcome::Nothing,
    }
}

/// Can `kind` shelter another bee right now?
#[cfg_attr(not(test), allow(dead_code))]
pub fn can_accept_bee(hive: &HiveData) -> bool {
    hive.bees_inside < MAX_BEES_INSIDE
}

/// A bee returns and shelters. Pure mutation.
#[cfg_attr(not(test), allow(dead_code))]
pub fn enter_hive(hive: &mut HiveData) -> bool {
    if can_accept_bee(hive) {
        hive.bees_inside += 1;
        true
    } else {
        false
    }
}

/// A bee leaves to pollinate. Pure mutation.
#[cfg_attr(not(test), allow(dead_code))]
pub fn leave_hive(hive: &mut HiveData) -> bool {
    if hive.bees_inside > 0 {
        hive.bees_inside -= 1;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    #[test]
    fn bee_within_radius_detects_only_near_bees() {
        let hive = (10, 20, 10);
        assert!(!bee_within(hive, &[], BEE_WORK_RADIUS), "no bees, no work");
        assert!(bee_within(hive, &[(12.0, 21.0, 9.0)], BEE_WORK_RADIUS));
        assert!(
            !bee_within(hive, &[(30.0, 20.0, 10.0)], BEE_WORK_RADIUS),
            "20 blocks away is out of range"
        );
    }

    #[test]
    fn new_hive_is_empty() {
        let h = HiveData::new();
        assert_eq!(h.bees_inside, 0);
        assert_eq!(h.honey_level, 0);
        assert!(h.is_empty());
        assert!(!h.is_full());
    }

    #[test]
    fn deposit_honey_increments_and_caps() {
        let mut h = HiveData::new();
        for expected in 1..=MAX_HONEY_LEVEL {
            assert_eq!(h.deposit_honey(), expected);
        }
        // Cap reached.
        assert_eq!(h.deposit_honey(), MAX_HONEY_LEVEL);
        assert!(h.is_full());
    }

    #[test]
    fn empty_hive_right_click_with_bucket_does_nothing() {
        let mut h = HiveData::new();
        let bucket = Item::Material(MaterialId::Bucket);
        let result = resolve_right_click(&mut h, Some(&bucket));
        assert!(matches!(result, HiveRightClickOutcome::Nothing));
        assert_eq!(h.honey_level, 0);
    }

    #[test]
    fn bucket_on_full_hive_gives_honey_bottle() {
        let mut h = HiveData { bees_inside: 0, honey_level: 3 };
        let bucket = Item::Material(MaterialId::Bucket);
        let result = resolve_right_click(&mut h, Some(&bucket));
        match result {
            HiveRightClickOutcome::Give { item, .. } => {
                assert!(matches!(item.item, Item::Material(MaterialId::HoneyBottle)));
            }
            other => panic!("expected Give, got {:?}", other),
        }
        assert_eq!(h.honey_level, 2);
    }

    #[test]
    fn shears_give_three_honeycomb_and_use_tool() {
        let mut h = HiveData { bees_inside: 0, honey_level: 4 };
        let shears = Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron));
        let result = resolve_right_click(&mut h, Some(&shears));
        match result {
            HiveRightClickOutcome::GiveAndUseTool { item } => {
                assert!(matches!(item.item, Item::Material(MaterialId::Honeycomb)));
                assert_eq!(item.count, 3);
            }
            other => panic!("expected GiveAndUseTool, got {:?}", other),
        }
        assert_eq!(h.honey_level, 3);
    }

    #[test]
    fn empty_hand_does_nothing() {
        let mut h = HiveData { bees_inside: 0, honey_level: 3 };
        let result = resolve_right_click(&mut h, None);
        assert!(matches!(result, HiveRightClickOutcome::Nothing));
        assert_eq!(h.honey_level, 3);
    }

    #[test]
    fn unrelated_tool_does_nothing() {
        let mut h = HiveData { bees_inside: 0, honey_level: 3 };
        let pickaxe = Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        let result = resolve_right_click(&mut h, Some(&pickaxe));
        assert!(matches!(result, HiveRightClickOutcome::Nothing));
        assert_eq!(h.honey_level, 3);
    }

    #[test]
    fn bees_enter_until_max() {
        let mut h = HiveData::new();
        for _ in 0..MAX_BEES_INSIDE {
            assert!(enter_hive(&mut h));
        }
        assert!(!enter_hive(&mut h), "should reject the 4th bee");
        assert_eq!(h.bees_inside, MAX_BEES_INSIDE);
    }

    #[test]
    fn bees_leave_only_if_inside() {
        let mut h = HiveData { bees_inside: 1, honey_level: 0 };
        assert!(leave_hive(&mut h));
        assert!(!leave_hive(&mut h), "empty hive shouldn't permit bee exit");
    }
}
