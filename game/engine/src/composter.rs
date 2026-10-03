//! Composter (Spec 49 — Explosives) — the farming nitre-bed.
//!
//! Ages plant trimmings / food waste into **Compost**, and Compost further into
//! **Saltpetre** (the real nitre-bed chemistry: aged compost heaps were the
//! historical saltpetre source). Built on the generic [`WorkstationState`]
//! framework — input / output / progress, no fuel — like the spec's other
//! deferred workstations (Mill / Oven / Aging Rack). The farmer's route to
//! saltpetre; the miner's route is `NITRE_ORE`. Both are live in v1 (the owner's
//! explicit "mine AND farm" call).
//!
//! Two recipe legs share one mechanism:
//!   * organic matter (seeds / crops / saplings) → Compost  (`COMPOST_PERIOD_TICKS`)
//!   * Compost                                   → Saltpetre (`SALTPETRE_PERIOD_TICKS`)
//!
//! The Compost → Saltpetre leg is a deliberate two-step (collect the Compost,
//! re-insert it) — "aged further" is a separate beat, and Compost is a useful
//! farming output in its own right.
//!
//! Spec: `docs/foundations/2026-06-20-explosives-blasting-keg.md`.

use crate::item::{Item, ItemStack, MaterialId};
use crate::workstation::{WorkstationState, WorkstationTick};

/// Ticks to age organic matter into Compost (~30 s @ 20 TPS). Tunable.
pub const COMPOST_PERIOD_TICKS: u64 = 600;
/// Ticks to age Compost into Saltpetre (~60 s). Slower — the nitre bed. Tunable.
pub const SALTPETRE_PERIOD_TICKS: u64 = 1_200;
/// Output-slot cap (mirrors the furnace's 64).
pub const MAX_OUTPUT_STACK: u8 = 64;

/// Is this item plant trimmings / food waste that composts into Compost?
/// (Compost itself is the *next* leg's input, handled in [`composter_recipe`].)
pub fn is_organic_input(item: &Item) -> bool {
    matches!(
        item,
        Item::Material(
            MaterialId::WheatSeeds
                | MaterialId::CornSeeds
                | MaterialId::SugarBeetSeeds
                | MaterialId::BeetrootSeeds
                | MaterialId::CottonSeeds
                | MaterialId::HempSeeds
                | MaterialId::CornflowerSeeds
                | MaterialId::FieldPoppySeeds
                | MaterialId::ButtercupSeeds
                | MaterialId::Wheat
                | MaterialId::Berries
                | MaterialId::OakSapling
                | MaterialId::BirchSapling
                | MaterialId::SpruceSapling
                | MaterialId::JungleSapling
                | MaterialId::AcaciaSapling
                | MaterialId::DarkOakSapling
                | MaterialId::RubberSapling
        )
    )
}

/// The composter recipe for a given input: `(output stack, aging period)`.
/// Organic matter → Compost; Compost → Saltpetre. `None` for anything else.
pub fn composter_recipe(input: &ItemStack) -> Option<(ItemStack, u64)> {
    if is_organic_input(&input.item) {
        return Some((
            ItemStack::new_material(MaterialId::Compost, 1),
            COMPOST_PERIOD_TICKS,
        ));
    }
    if matches!(input.item, Item::Material(MaterialId::Compost)) {
        return Some((
            ItemStack::new_material(MaterialId::Saltpetre, 1),
            SALTPETRE_PERIOD_TICKS,
        ));
    }
    None
}

/// Can this stack be loaded into a Composter's input at all?
pub fn is_compostable(item: &ItemStack) -> bool {
    composter_recipe(item).is_some()
}

/// Try to load ONE unit of `held` into a Composter's input slot. Accepts only
/// compostable items; stacks onto a matching input (up to 64), fills an empty
/// slot, or refuses if the slot already holds a *different* item. Returns `true`
/// iff a unit was accepted — the caller then removes one from the player's hand.
/// Pure: operates on the state + the held stack, no world. (Compostable inputs
/// are always materials, so a non-material `held` is refused.)
pub fn try_load_input(state: &mut WorkstationState, held: &ItemStack) -> bool {
    if !is_compostable(held) {
        return false;
    }
    let Item::Material(m) = held.item else {
        return false;
    };
    match &mut state.input {
        None => {
            state.input = Some(ItemStack::new_material(m, 1));
            true
        }
        Some(existing) => {
            let same = matches!(&existing.item, Item::Material(e) if *e == m);
            if same && existing.count < 64 {
                existing.count += 1;
                true
            } else {
                false
            }
        }
    }
}

/// Advance one composter by a tick — the furnace-sweep pattern, fuel-free.
/// Resolves the recipe from the current input, advances progress, and on
/// completion decrements the input and adds the output. Pure (no world).
pub fn tick_one(state: &mut WorkstationState) -> WorkstationTick {
    // Resolve the recipe for the current input.
    let Some((output, period)) = state.input.as_ref().and_then(composter_recipe) else {
        // No input or non-compostable → idle. Drop any partial progress so a
        // half-aged-then-emptied bin doesn't fast-finish its next load.
        state.progress_ticks = 0;
        return WorkstationTick::Idle;
    };

    // Output slot must be empty or hold a matching, non-saturated stack.
    let output_has_room = match (&state.output, &output.item) {
        (None, _) => true,
        (Some(out), Item::Material(target)) => {
            matches!(&out.item, Item::Material(m) if m == target) && out.count < MAX_OUTPUT_STACK
        }
        _ => false,
    };
    if !output_has_room {
        state.progress_ticks = 0;
        return WorkstationTick::Idle;
    }

    if state.try_advance(period) {
        // Recipe complete: input − 1, output + 1.
        if let Some(input) = state.input.as_mut() {
            if input.count > 1 {
                input.count -= 1;
            } else {
                state.input = None;
            }
        }
        match &mut state.output {
            Some(out) => out.count += 1,
            None => state.output = Some(output),
        }
        WorkstationTick::Completed
    } else {
        WorkstationTick::Progressing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organic_matter_ages_into_compost() {
        let mut s = WorkstationState::new();
        s.input = Some(ItemStack::new_material(MaterialId::WheatSeeds, 1));
        for _ in 0..COMPOST_PERIOD_TICKS - 1 {
            assert_eq!(tick_one(&mut s), WorkstationTick::Progressing);
        }
        assert_eq!(tick_one(&mut s), WorkstationTick::Completed);
        assert!(s.input.is_none(), "input consumed");
        assert!(matches!(
            s.output.as_ref().map(|o| &o.item),
            Some(Item::Material(MaterialId::Compost))
        ));
    }

    #[test]
    fn compost_ages_into_saltpetre() {
        let mut s = WorkstationState::new();
        s.input = Some(ItemStack::new_material(MaterialId::Compost, 1));
        for _ in 0..SALTPETRE_PERIOD_TICKS - 1 {
            assert_eq!(tick_one(&mut s), WorkstationTick::Progressing);
        }
        assert_eq!(tick_one(&mut s), WorkstationTick::Completed);
        let out = s.output.expect("saltpetre produced");
        assert!(matches!(out.item, Item::Material(MaterialId::Saltpetre)));
        assert_eq!(out.count, 1);
    }

    #[test]
    fn non_compostable_input_is_idle() {
        let mut s = WorkstationState::new();
        s.input = Some(ItemStack::new_material(MaterialId::IronIngot, 1));
        assert_eq!(tick_one(&mut s), WorkstationTick::Idle);
        assert_eq!(s.progress_ticks, 0);
        assert!(s.output.is_none());
    }

    #[test]
    fn empty_composter_is_idle() {
        let mut s = WorkstationState::new();
        assert_eq!(tick_one(&mut s), WorkstationTick::Idle);
    }

    #[test]
    fn saltpetre_period_is_slower_than_compost() {
        // The nitre bed ages slower than the first compost pass.
        assert!(SALTPETRE_PERIOD_TICKS > COMPOST_PERIOD_TICKS);
    }

    #[test]
    fn is_compostable_accepts_organic_and_compost_only() {
        assert!(is_compostable(&ItemStack::new_material(MaterialId::WheatSeeds, 1)));
        assert!(is_compostable(&ItemStack::new_material(MaterialId::Compost, 1)));
        assert!(!is_compostable(&ItemStack::new_material(MaterialId::IronIngot, 1)));
        assert!(!is_compostable(&ItemStack::new_material(MaterialId::Saltpetre, 1)));
    }
}
