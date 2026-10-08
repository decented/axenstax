//! Drying Rack — block-entity state + seasoning-progress tick.
//!
//! Per Spec 29 / `docs/foundations/2026-05-19-log-seasoning.md`. The rack
//! takes green logs and matures them into seasoned logs over ~5 real
//! minutes per slot (6000 ticks @ 20 TPS), gated by an air-above check
//! (so racks must be placed outdoors or under an open window — not
//! buried in a basement).
//!
//! State lives in `World::drying_racks` — a per-position map mirroring
//! the Campfire pattern (Spec 17). When Spec 20 Furnace ships the
//! `BlockEntityData` enum framework, this folds in as a single new
//! variant; the struct shape + field name are picked to make that a
//! mechanical rename.

use serde::{Deserialize, Serialize};

use crate::block;
use crate::item::MaterialId;

/// Number of seasoning slots per rack. Eight parallel-seasoning slots
/// gives the player a reason to stockpile green logs (it pays off when
/// they all mature together) without making the rack feel infinite.
pub const RACK_SLOTS: usize = 8;

/// Ticks of seasoning required for a slot to be fully mature.
///
/// 6000 = 5 real minutes @ 20 TPS. Picked as the "set it and forget it"
/// timescale — long enough to be a meaningful workflow ("I'll load the
/// rack, go chop more trees, come back to fuel"), short enough that a
/// kid doesn't bounce off the loop. Phase 9 playtest tunes.
pub const SEASON_TICKS: u32 = 6000;

/// Species of log being seasoned. Alpha is Oak-only (every tree in
/// world-gen is OAK_LOG today). Future species (Birch, Spruce,
/// Satori-adjacent exotics) add variants here; per-species mature
/// outputs land in [`mature_output`]. Picked as a real enum (not a
/// boolean) so the species-variation work is a one-arm extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogSpecies {
    #[default]
    Oak,
}

/// One rack slot — either empty or holding a green log of some species
/// with its current seasoning progress (in ticks).
///
/// **This travels on the wire** (`protocol::BlockView::DryingRack`, v79) as
/// well as in the save: any change to its fields, or to [`LogSpecies`]
/// (a new species is a new serde index an older peer can't decode), changes
/// the protocol — bump `PROTOCOL_VERSION`
/// (`protocol::tests::campfire_and_rack_views_are_pinned_on_the_full_bytes`
/// pins the bytes and fails on any such change: re-pin it with the bump).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RackSlot {
    pub species: Option<LogSpecies>,
    pub seasoning_ticks: u32,
}

impl RackSlot {
    pub fn is_empty(&self) -> bool {
        self.species.is_none()
    }
    pub fn is_mature(&self) -> bool {
        self.species.is_some() && self.seasoning_ticks >= SEASON_TICKS
    }
    /// Seasoning progress as a 0.0..=1.0 fraction. Useful for HUD
    /// "47% ready" toasts. Empty slots return 0.0.
    pub fn progress(&self) -> f32 {
        if self.species.is_none() {
            return 0.0;
        }
        (self.seasoning_ticks as f32 / SEASON_TICKS as f32).clamp(0.0, 1.0)
    }
}

/// State stored per Drying Rack block in `World::drying_racks`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DryingRackData {
    pub slots: [RackSlot; RACK_SLOTS],
}

impl DryingRackData {
    /// First empty slot index, if any.
    pub fn empty_slot(&self) -> Option<usize> {
        self.slots.iter().position(RackSlot::is_empty)
    }

    /// First fully-mature slot index, if any. Empty-hand right-click
    /// withdraws this slot.
    pub fn first_mature_slot(&self) -> Option<usize> {
        self.slots.iter().position(RackSlot::is_mature)
    }

    /// The maximum progress fraction across all slots (0.0 if rack is
    /// empty). Used by the "Not ready yet — N%" toast so the player
    /// knows they're close vs. just-loaded.
    pub fn max_progress(&self) -> f32 {
        self.slots
            .iter()
            .map(RackSlot::progress)
            .fold(0.0_f32, f32::max)
    }

    /// Try to place a green log of `species`. Returns `true` if the rack
    /// had an empty slot and the log was placed; `false` if every slot
    /// was occupied.
    pub fn try_place_green(&mut self, species: LogSpecies) -> bool {
        if let Some(idx) = self.empty_slot() {
            self.slots[idx] = RackSlot { species: Some(species), seasoning_ticks: 0 };
            true
        } else {
            false
        }
    }

    /// Take the first mature slot's contents, returning its species and
    /// clearing the slot. Returns `None` if no slot is mature.
    pub fn take_mature(&mut self) -> Option<LogSpecies> {
        let idx = self.first_mature_slot()?;
        let species = self.slots[idx].species?;
        self.slots[idx] = RackSlot::default();
        Some(species)
    }

    /// Number of non-empty slots — how many green logs would spill if
    /// the rack were mined right now (see [`spill_on_mine`]).
    pub fn occupied_slots(&self) -> usize {
        self.slots.iter().filter(|s| !s.is_empty()).count()
    }
}

/// Whether a rack at `(x, y, z)` is currently operating. The block
/// directly above must be `AIR` for seasoning to advance — encourages
/// outdoor placement or open-window racks; a sheltered rack just stalls
/// (no damage, no loss, just no progress until the air is opened up).
pub fn is_operating(world: &crate::world::World, x: i32, y: i32, z: i32) -> bool {
    world.get_block(x, y + 1, z) == block::AIR
}

/// Outcome of advancing one rack by one tick. Captures slots that just
/// matured this tick so the caller can fire a "rack ready" toast or
/// audio cue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RackTickOutcome {
    pub newly_mature_slots: Vec<usize>,
}

/// Advance one rack's state by one tick. Pure data-mutation: the caller
/// supplies the `operating` flag (typically `is_operating(world, ...)`).
/// Decoupled from the world reference so the engine tick can call this
/// while still holding a `&mut World.drying_racks` cursor — Rust's
/// aliasing rules disallow the inline `&world` lookup inside the same
/// statement.
///
/// Seasoning is capped at [`SEASON_TICKS`] so a long-sitting full slot
/// doesn't keep climbing into u32::MAX territory.
pub fn tick_one(data: &mut DryingRackData, operating: bool) -> RackTickOutcome {
    let mut newly_mature = Vec::new();
    if !operating {
        return RackTickOutcome::default();
    }
    for (i, slot) in data.slots.iter_mut().enumerate() {
        if slot.species.is_none() {
            continue;
        }
        if slot.seasoning_ticks < SEASON_TICKS {
            slot.seasoning_ticks += 1;
            if slot.seasoning_ticks == SEASON_TICKS {
                newly_mature.push(i);
            }
        }
    }
    RackTickOutcome { newly_mature_slots: newly_mature }
}

/// Map a species to its withdrawal material — the seasoned-log material
/// that pops into the player's inventory when they take a mature slot.
/// Alpha: Oak → SeasonedLog. Future species earn their own seasoned
/// variants.
pub fn mature_output(species: LogSpecies) -> MaterialId {
    match species {
        LogSpecies::Oak => MaterialId::SeasonedLog,
    }
}

/// Map a green-log MATERIAL to its species. Currently every log is
/// Oak; this helper exists so the rack's right-click handler can route
/// the placement through a species-aware path without hardcoding Oak.
/// Returns `None` for non-green-log materials.
pub fn species_of_green(m: MaterialId) -> Option<LogSpecies> {
    match m {
        MaterialId::GreenLog => Some(LogSpecies::Oak),
        _ => None,
    }
}

/// Cleanup hook when a Drying Rack block is destroyed (mined, blown up,
/// /setblock-replaced, …). Removes the rack's state entry from the world
/// AND returns a list of ItemStacks to spill — one [`MaterialId::GreenLog`]
/// per occupied slot, regardless of seasoning progress. Mining a working
/// workstation loses the in-progress seasoning by design (matches the
/// Minecraft "you uprooted the rack, the wood went back to fresh" feel),
/// so the player can't trivially relocate a half-seasoned stack.
///
/// Idempotent — safe to call on cells that were never racks (returns
/// an empty Vec).
pub fn cleanup_drying_rack(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<crate::item::ItemStack> {
    let Some(data) = world.drying_racks.remove(&(x, y, z)) else {
        return Vec::new();
    };
    let count = data.occupied_slots();
    if count == 0 {
        return Vec::new();
    }
    // One ItemStack per occupied slot (max 8). Could combine into a
    // single stack of N, but separate stacks keeps the inventory-add
    // path's overflow handling simpler — each call to add_item routes
    // through the standard stacking logic.
    (0..count)
        .map(|_| crate::item::ItemStack::new_material(MaterialId::GreenLog, 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    fn rack_at(world: &mut World, x: i32, y: i32, z: i32) -> &mut DryingRackData {
        world.set_block(x, y, z, block::DRYING_RACK);
        // Air-above by default (block::AIR is the world default).
        world.drying_racks.entry((x, y, z)).or_default()
    }

    #[test]
    fn empty_slot_progress_is_zero() {
        let s = RackSlot::default();
        assert_eq!(s.progress(), 0.0);
        assert!(!s.is_mature());
        assert!(s.is_empty());
    }

    #[test]
    fn slot_progress_clamps_to_one() {
        let s = RackSlot {
            species: Some(LogSpecies::Oak),
            seasoning_ticks: SEASON_TICKS + 1000,
        };
        assert!((s.progress() - 1.0).abs() < 1e-6);
        assert!(s.is_mature());
    }

    #[test]
    fn try_place_green_uses_first_empty_slot() {
        let mut d = DryingRackData::default();
        assert!(d.try_place_green(LogSpecies::Oak));
        assert_eq!(d.slots[0].species, Some(LogSpecies::Oak));
        assert_eq!(d.slots[0].seasoning_ticks, 0);
        assert!(d.try_place_green(LogSpecies::Oak));
        assert_eq!(d.slots[1].species, Some(LogSpecies::Oak));
    }

    #[test]
    fn try_place_green_fails_when_rack_full() {
        let mut d = DryingRackData::default();
        for _ in 0..RACK_SLOTS {
            assert!(d.try_place_green(LogSpecies::Oak));
        }
        assert!(!d.try_place_green(LogSpecies::Oak), "9th place must fail");
    }

    #[test]
    fn take_mature_returns_first_mature_and_clears_slot() {
        let mut d = DryingRackData::default();
        d.slots[2] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS };
        let species = d.take_mature().expect("mature slot");
        assert_eq!(species, LogSpecies::Oak);
        assert!(d.slots[2].is_empty());
    }

    #[test]
    fn take_mature_skips_immature_slots() {
        let mut d = DryingRackData::default();
        d.slots[0] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS / 2 };
        d.slots[3] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS };
        let species = d.take_mature().expect("mature slot");
        assert_eq!(species, LogSpecies::Oak);
        // The immature slot at 0 was NOT touched.
        assert_eq!(d.slots[0].seasoning_ticks, SEASON_TICKS / 2);
        // Slot 3 is now empty.
        assert!(d.slots[3].is_empty());
    }

    #[test]
    fn take_mature_returns_none_when_nothing_ready() {
        let mut d = DryingRackData::default();
        d.slots[0] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS - 1 };
        assert!(d.take_mature().is_none());
    }

    #[test]
    fn occupied_slots_counts_non_empty() {
        let mut d = DryingRackData::default();
        assert_eq!(d.occupied_slots(), 0);
        d.try_place_green(LogSpecies::Oak);
        d.try_place_green(LogSpecies::Oak);
        d.try_place_green(LogSpecies::Oak);
        assert_eq!(d.occupied_slots(), 3);
    }

    #[test]
    fn is_operating_requires_air_above() {
        let mut world = World::new();
        let _ = rack_at(&mut world, 0, 70, 0);
        // World default — air above the rack at (0, 71, 0).
        assert!(is_operating(&world, 0, 70, 0));
        // Block above stalls it.
        world.set_block(0, 71, 0, block::STONE);
        assert!(!is_operating(&world, 0, 70, 0));
    }

    #[test]
    fn tick_advances_seasoning_when_operating() {
        let mut data = DryingRackData::default();
        data.try_place_green(LogSpecies::Oak);
        let out = tick_one(&mut data, true);
        assert!(out.newly_mature_slots.is_empty());
        assert_eq!(data.slots[0].seasoning_ticks, 1);
    }

    #[test]
    fn tick_does_not_advance_when_not_operating() {
        let mut data = DryingRackData::default();
        data.try_place_green(LogSpecies::Oak);
        let out = tick_one(&mut data, false);
        assert!(out.newly_mature_slots.is_empty());
        assert_eq!(data.slots[0].seasoning_ticks, 0, "non-operating rack must not advance");
    }

    #[test]
    fn tick_caps_at_season_threshold() {
        let mut data = DryingRackData::default();
        data.slots[0] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS };
        for _ in 0..1000 {
            tick_one(&mut data, true);
        }
        assert_eq!(data.slots[0].seasoning_ticks, SEASON_TICKS, "must not overflow past threshold");
    }

    #[test]
    fn tick_reports_newly_mature_slots() {
        let mut data = DryingRackData::default();
        // Slot 1 is one tick short of mature.
        data.slots[1] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS - 1 };
        let out = tick_one(&mut data, true);
        assert_eq!(out.newly_mature_slots, vec![1]);
        // Next tick must NOT re-report — it's already mature, no transition.
        let out = tick_one(&mut data, true);
        assert!(out.newly_mature_slots.is_empty());
    }


    #[test]
    fn mature_output_oak_returns_seasoned_log() {
        assert_eq!(mature_output(LogSpecies::Oak), MaterialId::SeasonedLog);
    }

    #[test]
    fn species_of_green_only_for_green_log() {
        assert_eq!(species_of_green(MaterialId::GreenLog), Some(LogSpecies::Oak));
        assert_eq!(species_of_green(MaterialId::SeasonedLog), None);
        assert_eq!(species_of_green(MaterialId::KilnDriedLog), None);
        assert_eq!(species_of_green(MaterialId::Stick), None);
    }

    #[test]
    fn cleanup_spills_one_green_per_filled_slot_and_removes_entry() {
        let mut world = World::new();
        let _ = rack_at(&mut world, 1, 70, 2);
        // Manually populate three slots — one fresh, one half-seasoned,
        // one mature. All spill back as green logs (seasoning lost).
        {
            let d = world.drying_racks.get_mut(&(1, 70, 2)).unwrap();
            d.try_place_green(LogSpecies::Oak);
            d.slots[1] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS / 2 };
            d.slots[2] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS };
        }
        let spilled = cleanup_drying_rack(&mut world, 1, 70, 2);
        assert_eq!(spilled.len(), 3);
        for stack in &spilled {
            match stack.item {
                crate::item::Item::Material(MaterialId::GreenLog) => {}
                ref other => panic!("expected GreenLog, got {:?}", other),
            }
            assert_eq!(stack.count, 1);
        }
        // Entry is gone.
        assert!(!world.drying_racks.contains_key(&(1, 70, 2)));
    }

    #[test]
    fn cleanup_is_idempotent_on_non_rack_cells() {
        let mut world = World::new();
        // No rack at this position.
        let spilled = cleanup_drying_rack(&mut world, 5, 70, 5);
        assert!(spilled.is_empty());
        // Second call also safe.
        let spilled = cleanup_drying_rack(&mut world, 5, 70, 5);
        assert!(spilled.is_empty());
    }

    #[test]
    fn cleanup_returns_empty_for_empty_rack() {
        let mut world = World::new();
        let _ = rack_at(&mut world, 0, 70, 0);
        let spilled = cleanup_drying_rack(&mut world, 0, 70, 0);
        assert!(spilled.is_empty());
        assert!(!world.drying_racks.contains_key(&(0, 70, 0)));
    }
}
