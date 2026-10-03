//! Workstation framework — generic shape for processing blocks.
//!
//! Extracts the shared pattern from Furnace + Drying Rack + Vendor +
//! T1.5 Mill/Oven/Aging Rack:
//! - Block-entity with named slots (input, fuel, output) holding
//!   `Option<ItemStack>`.
//! - Tick-driven progress with a configurable period.
//! - Optional fuel consumption.
//! - Sealed by `WorkstationKind` so each consumer knows what it is.
//!
//! Live workstations (Furnace, Drying Rack, Vendor Block) remain in
//! their own modules with species-specific recipe tables. The
//! framework lives alongside as the contract that future workstations
//! (Mill, Oven, Aging Rack) consume — and that existing consumers can
//! refactor onto when their next change lands.
//!
//! Spec: `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md`
//! §"Workstation framework".

use serde::{Deserialize, Serialize};

use crate::item::ItemStack;

/// Which kind of workstation this is. Drives which recipe table the
/// caller consults + which UI panel renders. No consumer has adopted the
/// `WorkstationKind` tag yet (composter.rs uses `WorkstationState` directly,
/// untagged) — tested here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum WorkstationKind {
    Furnace,
    DryingRack,
    Mill,
    Oven,
    AgingRack,
}

#[cfg_attr(not(test), allow(dead_code))]
impl WorkstationKind {
    /// Default per-recipe processing period (ticks @ 20 TPS) for this
    /// kind. Overrideable per-recipe later.
    pub fn default_period_ticks(&self) -> u64 {
        match self {
            // Spec 20: 200 ticks per Furnace smelt baseline.
            WorkstationKind::Furnace => 200,
            // Spec 29: 5 real-minutes per drying-rack slot = 6000 ticks
            // at 20 TPS (4× game-time-step gives ~5 in-game minutes).
            WorkstationKind::DryingRack => 6_000,
            // T1.5 — Mill grinds fast (100 ticks per output).
            WorkstationKind::Mill => 100,
            // T1.5 — Oven slower than Furnace (multi-ingredient takes
            // longer to assemble + bake).
            WorkstationKind::Oven => 300,
            // T1.5 — Aging Rack glacial (in-game-day timer; 6000 ticks
            // baseline matches DryingRack until per-recipe override
            // ships).
            WorkstationKind::AgingRack => 6_000,
        }
    }

    /// Does this kind require fuel input? Furnace + Oven yes; the
    /// passive racks no.
    pub fn requires_fuel(&self) -> bool {
        matches!(self, WorkstationKind::Furnace | WorkstationKind::Oven)
    }
}

/// Generic workstation state. Lives in the world's per-position
/// block-entity map. Each consumer (Furnace, Mill, etc.) wraps this
/// with its own recipe-table-specific logic.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct WorkstationState {
    pub input: Option<ItemStack>,
    pub fuel: Option<ItemStack>,
    pub output: Option<ItemStack>,
    /// Ticks accumulated toward the current recipe's completion.
    pub progress_ticks: u64,
}

impl WorkstationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Is the workstation idle (no input, no progress)?
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_idle(&self) -> bool {
        self.input.is_none() && self.progress_ticks == 0
    }

    /// Progress 0.0..=1.0 toward completion at the given period. Used
    /// by UI panels to render the progress bar.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn progress_fraction(&self, period_ticks: u64) -> f32 {
        if period_ticks == 0 {
            return 0.0;
        }
        (self.progress_ticks as f32 / period_ticks as f32).clamp(0.0, 1.0)
    }

    /// Pure progression: advance progress by 1 tick if the workstation
    /// is active. Returns true when the recipe completes this tick
    /// (the caller should swap input for output + reset progress).
    /// Pure — doesn't touch slots; caller drives the slot mutation.
    pub fn try_advance(&mut self, period_ticks: u64) -> bool {
        // No input → no progress.
        if self.input.is_none() {
            self.progress_ticks = 0;
            return false;
        }
        self.progress_ticks += 1;
        if self.progress_ticks >= period_ticks {
            self.progress_ticks = 0;
            true
        } else {
            false
        }
    }
}

/// Outcome of a single workstation tick. Caller consumes this to
/// drive slot mutations (input − 1, output + 1) without the framework
/// needing to know what the recipe is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkstationTick {
    /// Idle this tick — no fuel / no input / no progress.
    Idle,
    /// Made progress toward completion; no slot change.
    Progressing,
    /// Completed a recipe this tick. Caller decrements input + adds
    /// output stack.
    Completed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::COBBLESTONE;

    #[test]
    fn default_state_is_idle() {
        let s = WorkstationState::new();
        assert!(s.is_idle());
        assert_eq!(s.progress_ticks, 0);
        assert!(s.input.is_none());
    }

    #[test]
    fn progress_fraction_in_unit_interval() {
        let mut s = WorkstationState::new();
        s.input = Some(ItemStack::new_block(COBBLESTONE, 1));
        s.progress_ticks = 50;
        let f = s.progress_fraction(200);
        assert!((f - 0.25).abs() < 0.001);
        // Clamps above 1.0.
        s.progress_ticks = 300;
        assert_eq!(s.progress_fraction(200), 1.0);
        // Zero-period guard.
        assert_eq!(s.progress_fraction(0), 0.0);
    }

    #[test]
    fn try_advance_no_input_yields_no_progress() {
        let mut s = WorkstationState::new();
        assert!(!s.try_advance(200));
        assert_eq!(s.progress_ticks, 0);
    }

    #[test]
    fn try_advance_increments_with_input() {
        let mut s = WorkstationState::new();
        s.input = Some(ItemStack::new_block(COBBLESTONE, 1));
        assert!(!s.try_advance(200));
        assert_eq!(s.progress_ticks, 1);
        assert!(!s.try_advance(200));
        assert_eq!(s.progress_ticks, 2);
    }

    #[test]
    fn try_advance_completes_at_period() {
        let mut s = WorkstationState::new();
        s.input = Some(ItemStack::new_block(COBBLESTONE, 1));
        for _ in 0..199 {
            assert!(!s.try_advance(200));
        }
        // 200th call completes.
        assert!(s.try_advance(200));
        // Progress resets.
        assert_eq!(s.progress_ticks, 0);
    }

    #[test]
    fn default_period_furnace_is_two_hundred() {
        assert_eq!(WorkstationKind::Furnace.default_period_ticks(), 200);
    }

    #[test]
    fn fuel_requirement_is_kind_specific() {
        assert!(WorkstationKind::Furnace.requires_fuel());
        assert!(WorkstationKind::Oven.requires_fuel());
        assert!(!WorkstationKind::DryingRack.requires_fuel());
        assert!(!WorkstationKind::Mill.requires_fuel());
        assert!(!WorkstationKind::AgingRack.requires_fuel());
    }

    #[test]
    fn mill_is_fastest_kind() {
        // The grind is quick — wheat through a mill shouldn't take
        // ages. Mill period < Furnace period < DryingRack period.
        let mill = WorkstationKind::Mill.default_period_ticks();
        let furnace = WorkstationKind::Furnace.default_period_ticks();
        let rack = WorkstationKind::DryingRack.default_period_ticks();
        assert!(mill < furnace);
        assert!(furnace < rack);
    }

    #[test]
    fn is_idle_correctly_recognises_inactive_state() {
        let mut s = WorkstationState::new();
        assert!(s.is_idle());
        // With input → not idle.
        s.input = Some(ItemStack::new_block(COBBLESTONE, 1));
        assert!(!s.is_idle());
        // No input + accumulated progress → not idle (would be
        // catching up after item was extracted).
        s.input = None;
        s.progress_ticks = 5;
        assert!(!s.is_idle());
    }
}
