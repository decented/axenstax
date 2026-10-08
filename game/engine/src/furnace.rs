//! Spec 20 — Furnace block-entity.
//!
//! Replaces the Wave 6 "raw_meat over coal in a vertical 2-slot crafting
//! grid" smelting hack with a real workstation. Three slots (input,
//! fuel, output), tick-driven smelt progress, fuel ladder mirroring
//! Campfire's pattern. Block-entity state lives in
//! `World::block_entities` under `BlockEntityData::Furnace`.
//!
//! Phase boundaries (see `docs/foundations/2026-05-18-furnace.md`):
//! - Phase 2 — `FurnaceData` struct + `BlockEntityData` enum wiring.
//! - Phase 3 (this commit) — `tick_one(&mut FurnaceData) -> Outcome`
//!   smelt logic + fuel ladder + recipe table.
//! - Phase 4 — `FURNACE` + `FURNACE_LIT` block ids + crafting recipe
//!   + right-click handler.
//! - Phase 5 — `furnace_ui.rs` egui overlay.
//! - Phase 6 — recipe migration off `crafting.rs::match_recipe`.

use crate::block::BlockId;
use crate::item::{ItemStack, MaterialId};

/// One smelt cycle in ticks. Mirrors `campfire::COOK_TICKS_PER_ITEM`
/// — 10 s at 20 Hz. Same per-item duration on alpha; the spec lets
/// us re-tune in playtest if the furnace should feel meaningfully
/// faster than the campfire.
pub const SMELT_TICKS_PER_ITEM: u32 = 200;

/// Spec 20 Phase 10 — Proof-of-Play sats trickle per completed smelt.
/// 1 sat per recipe-complete on Bitcoin-enabled servers when a
/// player is within `POP_TRICKLE_RADIUS_BLOCKS` of the furnace AND
/// their Charter sats flag is on. Lower per-event payout than a
/// pickaxe strike (passive vs active loop). Tunable in playtest;
/// actual Lightning settlement is BRIDGEd through `apply_sats_payout`.
pub const PROOF_OF_PLAY_TRICKLE_SATS: u64 = 1;

/// How close a player must be to a furnace at smelt-complete to
/// receive the PoP trickle. 16 blocks ≈ same-room distance; further-
/// away players see no trickle, keeping the mechanic tied to presence
/// + intent rather than background farming.
pub const POP_TRICKLE_RADIUS_BLOCKS: f32 = 16.0;

/// Stack-size cap for the output slot. Matches the existing item
/// `count: u8` representation in `inventory.rs` so we never overflow
/// when a recipe-complete bumps an in-progress stack.
pub const MAX_OUTPUT_STACK: u8 = 64;

/// Per-furnace state. Lives in `World::block_entities` keyed by the
/// furnace block's world-space position.
///
/// **Slots** are `Option<ItemStack>` so the egui UI can show empty
/// placeholders cleanly. The input + fuel slots are consumed by the
/// smelt tick; the output slot accumulates produced items until the
/// player Takes them (manual-take only on alpha — auto-eject hoppers
/// are a future polish per the spec's Design choice 5).
///
/// **Progress fields** mirror Campfire: `smelt_progress` is per-recipe
/// ticks; `smelt_total` is the recipe's required duration. The
/// `fuel_ticks_remaining` counter is how many ticks of burn-time the
/// current fuel still has — when it hits zero and the input slot is
/// non-empty, the tick consumes another fuel unit; if no fuel is
/// available the smelt halts.
///
/// `lit` mirrors the visual block-id: `true` while a smelt is actively
/// in progress (fuel + input both present). The renderer reads the
/// block-id directly (`FURNACE` vs `FURNACE_LIT`), so this field is
/// kept purely so the tick can detect lit→unlit transitions without
/// having to peek at the world.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct FurnaceData {
    pub input: Option<ItemStack>,
    pub fuel: Option<ItemStack>,
    pub output: Option<ItemStack>,
    pub smelt_progress: u32,
    pub smelt_total: u32,
    pub fuel_ticks_remaining: u32,
    pub lit: bool,
}

/// Outcome returned by [`tick_one`]. The caller (the per-tick
/// block-entity sweep in `game_loop`) uses this to (a) flip the
/// block-id between `FURNACE` and `FURNACE_LIT` when the lit state
/// changes, (b) play an audio cue on `recipe_completed`, and (c)
/// emit `BlockChange` packets when the lit state changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FurnaceTickOutcome {
    /// Some(material) iff this tick produced a finished item. The
    /// caller doesn't need to mutate state — the tick has already
    /// updated the output slot. This is purely an event signal.
    pub recipe_completed: Option<MaterialId>,
    /// Some(true) = furnace just lit. Some(false) = just unlit.
    /// None = no change. Caller flips the block-id accordingly.
    pub lit_changed: Option<bool>,
}

/// Pure recipe table. **Spec 29 (2026-05-21) — furnace is ore-only.**
/// The raw-meat → cooked-meat arms that v1 inherited from the grid-
/// smelt migration moved entirely to the Campfire (Spec 17) following
/// Axolittle's playtest call: the furnace is the smelter, the
/// campfire is the cook station. No overlap.
pub fn smelt_recipe_for_input(input: MaterialId) -> Option<MaterialId> {
    use MaterialId::*;
    match input {
        // Spec 5 §4.5 — `raw iron → iron ingot`. The grid hack
        // smelts via `IRON_ORE block + coal` to `IronIngot`; in the
        // furnace world the input material is `RawIron`. The grid
        // arm was removed by Spec 20 Phase 6.
        RawIron => Some(IronIngot),
        // Spec 28c — Copper / Tin smelting. Mirrors RawIron's pattern.
        // Bronze alloy is NOT a smelt recipe — it's a 2-input crafting
        // recipe (CopperIngot + TinIngot → BronzeIngot) handled in
        // `crafting.rs::match_recipe` if/when 28c Phase 4 ships the
        // grid recipe.
        Copper => Some(CopperIngot),
        Tin => Some(TinIngot),
        // Everything else — including raw meats — returns None. Raw
        // meats are cooked at the Campfire (`campfire::tick_one`),
        // not the furnace. Spec 18 baked roots / corn also route
        // through the campfire.
        _ => None,
    }
}

/// Fuel value (in ticks) for an item used as furnace fuel. Reuses
/// `campfire::fuel_value` so the same fuels work in both
/// workstations — coal, planks, logs, sticks, leaves. The values
/// are conservative for alpha; the spec calls out that the
/// fuel/smelts ratio is tunable in playtest. If a future fuel
/// (e.g. lava bucket) is furnace-only, add a wrapper here.
pub fn fuel_value(material: Option<MaterialId>, block: Option<BlockId>) -> Option<u32> {
    crate::campfire::fuel_value(material, block)
}

/// Advance one furnace's state by one tick. Pure function: takes the
/// data mutably, returns the event signal. The caller iterates the
/// block-entity map and applies block-flip side-effects (lit↔unlit
/// block-id swap + chunk-rebuild + BlockChange broadcast).
///
/// Behaviour summary:
/// - If `input` is empty OR the output slot would overflow, the
///   smelt is gated — `smelt_progress` resets to 0 and the furnace
///   reports `lit_changed = Some(false)` if it was previously lit.
/// - If `input` is present + output has room + `fuel_ticks_remaining`
///   > 0, the tick advances `smelt_progress` by 1 and burns one
///   > fuel tick. On `smelt_progress >= smelt_total`, the recipe
///   > completes: input decrements by 1, output gains 1 unit of the
///   > recipe's output material, progress resets.
/// - If `input` is present + output has room + `fuel_ticks_remaining`
///   == 0, the tick consumes one unit of the fuel slot (`fuel.count
///   -= 1`, slot cleared if it drops to 0) and refills
///   `fuel_ticks_remaining` from `fuel_value`. The smelt does not
///   advance on this tick — fuel-consume is its own beat. (Matches
///   campfire's no-progress-on-fuel-add cadence.)
/// - If no fuel is available + input present, the furnace stalls:
///   `lit_changed = Some(false)` if it was previously lit; progress
///   does NOT reset (so dropping in fresh fuel resumes the same
///   smelt mid-stream — a quality-of-life beat that doesn't change
///   the steady-state economy).
///
/// The 2-input-types-on-input-slot edge case (mixing raw beef and
/// raw iron in the same input slot) is impossible by construction
/// because the slot is a single ItemStack — players can only have
/// one input material in flight at a time.
pub fn tick_one(data: &mut FurnaceData) -> FurnaceTickOutcome {
    let mut outcome = FurnaceTickOutcome::default();
    let was_lit = data.lit;

    // Resolve the recipe — if the input slot doesn't map to any
    // known recipe, gate the smelt the same as an empty input.
    let recipe_output: Option<MaterialId> = data
        .input
        .as_ref()
        .and_then(|s| match s.item {
            crate::item::Item::Material(m) => smelt_recipe_for_input(m),
            _ => None,
        });

    let output_has_room = match (&data.output, recipe_output) {
        (None, Some(_)) => true,
        (Some(out), Some(target)) => {
            // Output stack must match the target material and not
            // be saturated. `count: u8` means MAX_OUTPUT_STACK=64
            // is well below overflow; the bounds check guards the
            // economy expectation that smelts can never sneak past
            // a "full" output slot.
            matches!(&out.item, crate::item::Item::Material(m) if *m == target)
                && out.count < MAX_OUTPUT_STACK
        }
        _ => false,
    };

    if recipe_output.is_none() || !output_has_room {
        // Smelt gated. Reset progress to 0 — partial-smelt timers
        // don't persist across input-removal. Lit drops to false
        // (no work being done). Fuel reserves are PRESERVED — a
        // partially-burnt fuel unit isn't refunded but it also
        // isn't wasted until the furnace next runs work.
        data.smelt_progress = 0;
        if data.lit {
            data.lit = false;
            outcome.lit_changed = Some(false);
        }
        return outcome;
    }

    // We have a recipe and output room. Need fuel.
    if data.fuel_ticks_remaining == 0 {
        // Try to consume one fuel unit.
        let fuel_ticks = data.fuel.as_ref().and_then(|s| {
            let mat = match &s.item {
                crate::item::Item::Material(m) => Some(*m),
                _ => None,
            };
            let blk = match &s.item {
                crate::item::Item::Block(b) => Some(*b),
                _ => None,
            };
            fuel_value(mat, blk)
        });
        if let Some(ticks) = fuel_ticks {
            data.fuel_ticks_remaining = ticks;
            // Decrement the fuel stack.
            if let Some(fuel) = data.fuel.as_mut() {
                if fuel.count > 1 {
                    fuel.count -= 1;
                } else {
                    data.fuel = None;
                }
            }
            // Fuel-consume tick — no smelt progress this tick.
            // Mark lit (caller flips block to FURNACE_LIT on first lit).
            if !data.lit {
                data.lit = true;
                outcome.lit_changed = Some(true);
            }
            return outcome;
        } else {
            // No fuel available. Stall — preserve smelt_progress but
            // mark unlit.
            if data.lit {
                data.lit = false;
                outcome.lit_changed = Some(false);
            }
            return outcome;
        }
    }

    // We have fuel + recipe + output room. Burn one tick of fuel
    // and advance the smelt.
    data.fuel_ticks_remaining -= 1;

    if !data.lit {
        data.lit = true;
        outcome.lit_changed = Some(true);
    }

    // Initialise smelt_total on first work tick.
    if data.smelt_total == 0 {
        data.smelt_total = SMELT_TICKS_PER_ITEM;
    }
    data.smelt_progress += 1;

    if data.smelt_progress >= data.smelt_total {
        // Recipe complete. Decrement input by 1; add 1 to output.
        let produced = recipe_output.expect("guarded above");
        if let Some(input) = data.input.as_mut() {
            if input.count > 1 {
                input.count -= 1;
            } else {
                data.input = None;
            }
        }
        match &mut data.output {
            Some(out) => out.count += 1,
            None => data.output = Some(ItemStack::new_material(produced, 1)),
        }
        data.smelt_progress = 0;
        data.smelt_total = 0;
        outcome.recipe_completed = Some(produced);
    }

    // Defence-in-depth: was_lit unused warning — squashes the
    // clippy `unused_variable` warning even though we only need
    // was_lit to gate the lit-transitions inside the branches.
    let _ = was_lit;

    outcome
}

/// Tick a FUEL-ONLY burner (Spec 48 — Steam Generator). Unlike [`tick_one`],
/// which only burns fuel to drive a smelt (and so needs an input + output room),
/// a burner consumes fuel **continuously** to do its own work (generate power):
///   * while `fuel_ticks_remaining > 0` it stays lit and counts down one tick;
///   * at 0 it consumes one unit from the `fuel` slot and relights, or — if the
///     slot is empty — goes dark.
///     The smelt fields (`input`/`output`/`smelt_*`) are ignored. Returns
///     `Some(new_lit)` only when the lit state flips (so the power tick flips the
///     block id + re-evaluates the network just once), else `None`.
pub fn tick_burner(data: &mut FurnaceData) -> Option<bool> {
    let was_lit = data.lit;
    if data.fuel_ticks_remaining > 0 {
        data.fuel_ticks_remaining -= 1;
        data.lit = true;
    } else {
        // Burn-time exhausted — try to consume a fresh fuel unit.
        let ticks = data.fuel.as_ref().and_then(|s| {
            let mat = if let crate::item::Item::Material(m) = s.item { Some(m) } else { None };
            let blk = if let crate::item::Item::Block(b) = s.item { Some(b) } else { None };
            fuel_value(mat, blk)
        });
        if let Some(t) = ticks {
            // This tick counts as the first of the new unit's burn.
            data.fuel_ticks_remaining = t.saturating_sub(1);
            if let Some(fuel) = data.fuel.as_mut() {
                if fuel.count > 1 {
                    fuel.count -= 1;
                } else {
                    data.fuel = None;
                }
            }
            data.lit = true;
        } else {
            data.lit = false;
        }
    }
    if data.lit != was_lit { Some(data.lit) } else { None }
}

// ── Spec 29 — slot-click free function ───────────────────────────────

/// Which slot of a furnace UI was clicked. C3b-1 — wire data (inside
/// `container_window::ContainerClick::Furnace`), APPEND ONLY: Input = 0,
/// Fuel = 1, Output = 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SlotKind {
    Input,
    Fuel,
    Output,
}

/// How much to move on a click: 1 (left-click) or the whole hotbar
/// / slot stack (shift-click). Matches Minecraft's left-click vs
/// shift-click idiom for furnace slots. C3b-1 — wire data, APPEND ONLY:
/// Single = 0, Stack = 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClickMode {
    Single,
    Stack,
}

/// Outcome of a slot click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotClickResult {
    /// Moved `count` items between the player's inventory and the
    /// furnace slot. Direction is implied by `SlotKind` + whether
    /// the player was holding an item.
    Moved { count: u8 },
    /// Click had no visible effect — wrong item type, slot saturated,
    /// inventory full, etc.
    NoOp,
}

/// Apply a click on a furnace UI slot. Mutates `furnace` and
/// `inventory` in place; pure with respect to the world / ECS so
/// it's testable without UI infrastructure.
///
/// Semantics:
/// - **Input slot, held item**: place 1 (Single) or whole hotbar
///   stack (Stack) if the held item is a smeltable material AND
///   either the slot is empty OR holds the same material.
/// - **Fuel slot, held item**: place 1 / whole stack if the held
///   item has a non-`None` `fuel_value` AND stacks compatibly.
/// - **Output slot, held item**: held item ignored; take from
///   output (insert is forbidden — matches Minecraft).
/// - **Any slot, empty hand**: take 1 / whole stack from the slot
///   into the player's inventory (if there's room).
pub fn apply_slot_click(
    furnace: &mut FurnaceData,
    inventory: &mut crate::inventory::Inventory,
    hotbar_slot: usize,
    slot_kind: SlotKind,
    click_mode: ClickMode,
) -> SlotClickResult {
    let held = inventory.hotbar_slot(hotbar_slot).cloned();
    match slot_kind {
        SlotKind::Output => take_from_furnace_slot(&mut furnace.output, inventory, click_mode),
        SlotKind::Input => match held {
            None => take_from_furnace_slot(&mut furnace.input, inventory, click_mode),
            Some(held_stack) => place_into_input(furnace, inventory, hotbar_slot, &held_stack, click_mode),
        },
        SlotKind::Fuel => match held {
            None => take_from_furnace_slot(&mut furnace.fuel, inventory, click_mode),
            Some(held_stack) => place_into_fuel(furnace, inventory, hotbar_slot, &held_stack, click_mode),
        },
    }
}

fn take_from_furnace_slot(
    slot: &mut Option<ItemStack>,
    inventory: &mut crate::inventory::Inventory,
    mode: ClickMode,
) -> SlotClickResult {
    let stack = match slot.as_mut() {
        Some(s) => s,
        None => return SlotClickResult::NoOp,
    };
    let want = match mode {
        ClickMode::Single => 1u8,
        ClickMode::Stack => stack.count,
    };
    let take_count = want.min(stack.count);
    // Try to move take_count units to the inventory one at a time so
    // that partial fills still register as a Moved with the right count.
    let mut moved = 0u8;
    for _ in 0..take_count {
        let one = ItemStack { item: stack.item.clone(), count: 1 };
        if inventory.add_item(one).is_none() {
            moved += 1;
        } else {
            break;
        }
    }
    if moved == 0 {
        return SlotClickResult::NoOp;
    }
    if stack.count > moved {
        stack.count -= moved;
    } else {
        *slot = None;
    }
    SlotClickResult::Moved { count: moved }
}

fn place_into_input(
    furnace: &mut FurnaceData,
    inventory: &mut crate::inventory::Inventory,
    hotbar_slot: usize,
    held: &ItemStack,
    mode: ClickMode,
) -> SlotClickResult {
    // Only materials can smelt — blocks held in hand are rejected.
    let mat = match &held.item {
        crate::item::Item::Material(m) => *m,
        _ => return SlotClickResult::NoOp,
    };
    if smelt_recipe_for_input(mat).is_none() {
        return SlotClickResult::NoOp;
    }
    // If the input slot is occupied with a different material, refuse.
    if let Some(existing) = &furnace.input {
        match &existing.item {
            crate::item::Item::Material(em) if *em == mat => {}
            _ => return SlotClickResult::NoOp,
        }
    }
    let want = match mode {
        ClickMode::Single => 1u8,
        ClickMode::Stack => held.count,
    };
    // Cap by the room left in the input slot. saturating_add at 255 silently
    // ate any overflow we'd already consumed from the inventory (item loss /
    // cap violation — engine audit 2026-06-04, F). Only consume what fits.
    let existing = furnace.input.as_ref().map(|s| s.count).unwrap_or(0);
    let room = crate::item::Item::Material(mat).max_stack().saturating_sub(existing);
    let want = want.min(room);
    if want == 0 {
        return SlotClickResult::NoOp; // input slot already at max_stack
    }
    let mut moved = 0u8;
    for _ in 0..want {
        if inventory.consume_one_material(hotbar_slot, mat) {
            moved += 1;
        } else {
            break;
        }
    }
    if moved == 0 {
        return SlotClickResult::NoOp;
    }
    match &mut furnace.input {
        Some(s) => s.count = s.count.saturating_add(moved),
        None => furnace.input = Some(ItemStack::new_material(mat, moved)),
    }
    SlotClickResult::Moved { count: moved }
}

fn place_into_fuel(
    furnace: &mut FurnaceData,
    inventory: &mut crate::inventory::Inventory,
    hotbar_slot: usize,
    held: &ItemStack,
    mode: ClickMode,
) -> SlotClickResult {
    // Fuel can be either a material (Coal, Stick) or a block
    // (OAK_PLANKS, OAK_LOG, OAK_LEAVES). Other item kinds reject.
    let (mat_opt, blk_opt): (Option<MaterialId>, Option<BlockId>) = match &held.item {
        crate::item::Item::Material(m) => (Some(*m), None),
        crate::item::Item::Block(b) => (None, Some(*b)),
        _ => return SlotClickResult::NoOp,
    };
    if fuel_value(mat_opt, blk_opt).is_none() {
        return SlotClickResult::NoOp;
    }
    // Stack-compatibility check against existing fuel.
    if let Some(existing) = &furnace.fuel {
        let same = match (&existing.item, &held.item) {
            (crate::item::Item::Material(em), crate::item::Item::Material(m)) => em == m,
            (crate::item::Item::Block(eb), crate::item::Item::Block(b)) => eb == b,
            _ => false,
        };
        if !same {
            return SlotClickResult::NoOp;
        }
    }
    let want = match mode {
        ClickMode::Single => 1u8,
        ClickMode::Stack => held.count,
    };
    // Cap by room left in the fuel slot (same loss/cap bug as the input slot).
    let existing = furnace.fuel.as_ref().map(|s| s.count).unwrap_or(0);
    let room = held.item.max_stack().saturating_sub(existing);
    let want = want.min(room);
    if want == 0 {
        return SlotClickResult::NoOp; // fuel slot already at max_stack
    }
    let mut moved = 0u8;
    for _ in 0..want {
        let took = if let Some(m) = mat_opt {
            inventory.consume_one_material(hotbar_slot, m)
        } else if blk_opt.is_some() {
            inventory.take_block_from_hotbar(hotbar_slot).is_some()
        } else {
            false
        };
        if took {
            moved += 1;
        } else {
            break;
        }
    }
    if moved == 0 {
        return SlotClickResult::NoOp;
    }
    match &mut furnace.fuel {
        Some(s) => s.count = s.count.saturating_add(moved),
        None => {
            let item = match (mat_opt, blk_opt) {
                (Some(m), _) => crate::item::Item::Material(m),
                (_, Some(b)) => crate::item::Item::Block(b),
                _ => unreachable!("fuel_value would have rejected"),
            };
            furnace.fuel = Some(ItemStack { item, count: moved });
        }
    }
    SlotClickResult::Moved { count: moved }
}

/// Spec 29 — legacy-meat detector for the save-load eject hook.
/// Returns true iff `stack` is one of the four raw meats that v1
/// furnaces could smelt. Used by [`eject_legacy_food_from_furnace`]
/// to identify input-slot contents that no longer match a valid
/// recipe and need to be returned to the player as a loose drop.
pub fn is_legacy_food_input(stack: &ItemStack) -> bool {
    matches!(
        stack.item,
        crate::item::Item::Material(MaterialId::RawBeef)
            | crate::item::Item::Material(MaterialId::RawChicken)
            | crate::item::Item::Material(MaterialId::RawMutton)
            | crate::item::Item::Material(MaterialId::RawPorkchop)
    )
}

/// Spec 29 — one-shot save-load hook called for each furnace after
/// it deserialises in `save::load_world`. If the input slot holds
/// one of the four raw meats that v1 could smelt, clear the slot
/// and return the ejected stack so the caller can spawn it as a
/// loose item entity near the furnace block. Returns `None` if the
/// input slot is empty or holds a valid ore-tier smelt input —
/// in that case the furnace is left untouched.
///
/// Fuel and output slots are **never** touched: they may legitimately
/// hold a v1 player's coal reserves or freshly-smelted ingots, and
/// resetting them would burn the player's progress.
pub fn eject_legacy_food_from_furnace(data: &mut FurnaceData) -> Option<ItemStack> {
    let should_eject = data
        .input
        .as_ref()
        .map(is_legacy_food_input)
        .unwrap_or(false);
    if should_eject {
        let ejected = data.input.take();
        // Reset smelt progress too — any partial work on the meat is
        // moot once the input vanishes. tick_one would have done this
        // on the next tick anyway, but resetting here keeps the load
        // state coherent for inspection.
        data.smelt_progress = 0;
        data.smelt_total = 0;
        data.lit = false;
        ejected
    } else {
        None
    }
}

/// Spec 29 — walk every furnace in `world`, eject legacy raw-meat
/// inputs into `world.pending_legacy_meat_drops` for the caller
/// (GameState or GameServer first-tick path) to spawn as ItemEntities
/// near the furnace block. Idempotent — running twice on the same
/// world is a no-op the second time because the inputs are already
/// clear. Safe to call after `save::load_world` returns.
pub fn eject_legacy_food_into_world_pending(world: &mut crate::world::World) {
    // Two passes to satisfy the borrow checker: first collect
    // positions of furnaces holding legacy meat, then mutate each.
    let positions: Vec<(i32, i32, i32)> = world
        .iter_furnaces()
        .filter(|(_, f)| f.input.as_ref().map(is_legacy_food_input).unwrap_or(false))
        .map(|(pos, _)| pos)
        .collect();
    let mut ejections: Vec<((i32, i32, i32), ItemStack)> = Vec::new();
    for pos in positions {
        if let Some(f) = world.furnace_at_mut(pos)
            && let Some(stack) = eject_legacy_food_from_furnace(f) {
                ejections.push((pos, stack));
            }
    }
    world.pending_legacy_meat_drops.extend(ejections);
}

/// Spec 20 Phase 4 — cleanup hook when a furnace block is being
/// destroyed (mined, blown up, /setblock-replaced). Removes the
/// block-entity entry. Returns the spillover ItemStacks the caller
/// should drop into the world (input + fuel + output) so the player
/// gets their stuff back. Idempotent — safe to call on cells that
/// were never furnaces.
///
/// Called at every furnace-mining site in `game_loop.rs`, mirroring
/// `chest::cleanup_chest` (its exact structural sibling) exactly. Task 1
/// (wave-hardening) wired this in: previously it had zero call sites, so
/// mining a furnace silently destroyed its contents AND left the
/// `FurnaceData` orphaned in `world.block_entities` — placing a new
/// furnace at the same cell would resurrect the old contents (the
/// reported "ghost iron" bug), because the right-click-open path
/// (`game_loop.rs`, `furnace_at(pos).is_none()`) only initialises fresh
/// state when nothing is already there.
pub fn cleanup_furnace(world: &mut crate::world::World, x: i32, y: i32, z: i32) -> Vec<ItemStack> {
    let mut spill = Vec::new();
    if let Some(f) = world.furnace_at((x, y, z)) {
        // `count > 0` mirrors `chest::cleanup_chest`'s spill guard — never
        // spill a zero-count stack as a phantom item.
        for s in [&f.input, &f.fuel, &f.output].into_iter().flatten() {
            if s.count > 0 {
                spill.push(s.clone());
            }
        }
    }
    world.remove_block_entity((x, y, z));
    spill
}

/// What one [`tick_all`] sweep did, for the caller's side effects.
#[derive(Debug, Default)]
pub struct FurnaceSweep {
    /// FURNACE ↔ FURNACE_LIT flips, already applied to the world and built with
    /// the world's meta byte — queue them for broadcast as-is.
    pub changes: Vec<crate::protocol::BlockChange>,
    /// Furnaces that finished a smelt this tick (audio / smoke / the
    /// Proof-of-Play trickle are the client's — they need `PlayerSlot`).
    pub completed: Vec<(i32, i32, i32)>,
}

/// Advance every furnace in the world one smelt tick (Spec 20 Phase 4) and flip
/// its block between FURNACE and FURNACE_LIT when the lit state changes. Shared
/// by the client loop and the dedicated server (T1-3, 2026-10-05) — the ONE
/// sweep, so a hosted furnace and a single-player furnace smelt identically.
///
/// The flip only fires when the world's block actually disagrees with the
/// desired state AND is still a furnace (defence against a `/setblock` that
/// swapped the block out from under the entity — trust the next tick to
/// converge).
pub fn tick_all(world: &mut crate::world::World) -> FurnaceSweep {
    let mut sweep = FurnaceSweep::default();
    let positions: Vec<(i32, i32, i32)> = world.iter_furnaces().map(|(p, _)| p).collect();
    for pos in positions {
        let current_block = world.get_block(pos.0, pos.1, pos.2);
        let outcome = match world.furnace_at_mut(pos) {
            Some(data) => tick_one(data),
            None => continue,
        };
        if let Some(lit) = outcome.lit_changed {
            let want_block = if lit { crate::block::FURNACE_LIT } else { crate::block::FURNACE };
            if current_block != want_block
                && (current_block == crate::block::FURNACE
                    || current_block == crate::block::FURNACE_LIT)
            {
                world.set_block(pos.0, pos.1, pos.2, want_block);
                sweep.changes.push(crate::game_loop::broadcast_change(
                    world, pos.0, pos.1, pos.2, want_block,
                ));
            }
        }
        if outcome.recipe_completed.is_some() {
            sweep.completed.push(pos);
        }
    }
    sweep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, ItemStack, MaterialId};

    fn furnace_with(input: Option<MaterialId>, fuel: Option<(MaterialId, u8)>) -> FurnaceData {
        let mut f = FurnaceData::default();
        if let Some(m) = input {
            f.input = Some(ItemStack::new_material(m, 1));
        }
        if let Some((m, n)) = fuel {
            f.fuel = Some(ItemStack::new_material(m, n));
        }
        f
    }

    #[test]
    fn tick_burner_burns_fuel_continuously_without_input() {
        // Spec 48 — a Steam Generator burns fuel with NO smelting input (unlike
        // a furnace). Empty → dark; fuelled → lights, counts down, consumes one
        // unit at a time; empties → dark again.
        let mut d = FurnaceData::default();
        assert_eq!(tick_burner(&mut d), None, "no fuel: no change");
        assert!(!d.lit);

        // Two coal. First tick lights it and consumes one unit.
        d.fuel = Some(ItemStack::new_material(MaterialId::Coal, 2));
        assert_eq!(tick_burner(&mut d), Some(true), "lights on first fuel tick");
        assert!(d.lit);
        assert!(d.fuel_ticks_remaining > 0, "burn timer charged");
        assert_eq!(d.fuel.as_ref().unwrap().count, 1, "one coal consumed");

        // It keeps burning the current unit without consuming more or re-flipping.
        for _ in 0..5 {
            assert_eq!(tick_burner(&mut d), None, "still burning, no flip");
            assert!(d.lit);
        }
        assert_eq!(d.fuel.as_ref().unwrap().count, 1, "no extra coal burned mid-unit");

        // Drain the timer with the slot empty → it goes dark on the next tick.
        d.fuel = None;
        d.fuel_ticks_remaining = 0;
        assert_eq!(tick_burner(&mut d), Some(false), "dark when out of fuel");
        assert!(!d.lit);
    }

    #[test]
    fn furnace_input_caps_at_max_and_does_not_over_consume() {
        // Input slot already holds 60 RawIron (room = 4 up to max_stack 64).
        // The player holds 10; only 4 should move, the other 6 stay in hand.
        // The old code consumed all 10 and saturating_add'd past the cap
        // (item loss + cap violation — engine audit F).
        let mut furnace = FurnaceData::default();
        furnace.input = Some(ItemStack::new_material(MaterialId::RawIron, 60));
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::RawIron, 10)));
        let held = ItemStack::new_material(MaterialId::RawIron, 10);
        let result = place_into_input(&mut furnace, &mut inv, 0, &held, ClickMode::Stack);
        assert!(matches!(result, SlotClickResult::Moved { count: 4 }), "only 4 fit");
        assert_eq!(furnace.input.as_ref().unwrap().count, 64, "input capped at max_stack");
        assert_eq!(inv.slot(0).map(|s| s.count), Some(6), "the other 6 are not consumed");
    }

    #[test]
    fn default_furnace_is_unlit_and_empty() {
        let f = FurnaceData::default();
        assert!(f.input.is_none());
        assert!(f.fuel.is_none());
        assert!(f.output.is_none());
        assert_eq!(f.smelt_progress, 0);
        assert_eq!(f.smelt_total, 0);
        assert_eq!(f.fuel_ticks_remaining, 0);
        assert!(!f.lit);
    }

    // ── Spec 20 Phase 10 — Proof-of-Play trickle constants ────────

    #[test]
    fn pop_trickle_sats_is_one() {
        // Alpha pace — 1 sat per smelt-complete. Tunable in playtest;
        // locked here so an accidental bump to 10/100 needs explicit
        // test maintenance (an unaudited 100× jump would change the
        // sats-flow ergonomics of the whole economy).
        assert_eq!(PROOF_OF_PLAY_TRICKLE_SATS, 1);
    }

    #[test]
    fn pop_trickle_radius_is_room_scale() {
        // 16 blocks ≈ same-room distance. Specifically NOT bigger
        // than a chunk (16 blocks = chunk width); chunk-scale was a
        // deliberate choice so the trickle is presence-gated and
        // can't be farmed across loaded-but-distant chunks.
        assert_eq!(POP_TRICKLE_RADIUS_BLOCKS, 16.0);
    }

    #[test]
    fn smelt_recipe_lookup_known_inputs() {
        // Spec 29 — furnace is ore-only post-2026-05-21. The four
        // raw-meat → cooked-meat arms moved to the campfire (Spec 17).
        assert_eq!(smelt_recipe_for_input(MaterialId::RawIron), Some(MaterialId::IronIngot));
        // Spec 28c additions.
        assert_eq!(smelt_recipe_for_input(MaterialId::Copper), Some(MaterialId::CopperIngot));
        assert_eq!(smelt_recipe_for_input(MaterialId::Tin), Some(MaterialId::TinIngot));
    }

    #[test]
    fn smelt_recipe_unknown_input_returns_none() {
        assert_eq!(smelt_recipe_for_input(MaterialId::Stick), None);
        assert_eq!(smelt_recipe_for_input(MaterialId::Wheat), None);
        assert_eq!(smelt_recipe_for_input(MaterialId::Bread), None);
        // CookedBeef is already cooked — no further smelt.
        assert_eq!(smelt_recipe_for_input(MaterialId::CookedBeef), None);
    }

    #[test]
    fn smelt_recipe_rejects_raw_meats_post_spec29() {
        // Spec 29 — Axolittle's playtest call 2026-05-21: cooking
        // food belongs at the Campfire (Spec 17), not in the furnace.
        // These four arms were live up to and including v1 (Spec 20);
        // Spec 29 removes them. The four meats now return None;
        // putting a raw chicken in the furnace input slot does nothing.
        assert_eq!(smelt_recipe_for_input(MaterialId::RawBeef), None);
        assert_eq!(smelt_recipe_for_input(MaterialId::RawChicken), None);
        assert_eq!(smelt_recipe_for_input(MaterialId::RawMutton), None);
        assert_eq!(smelt_recipe_for_input(MaterialId::RawPorkchop), None);
    }

    #[test]
    fn empty_furnace_tick_is_noop_unlit() {
        let mut f = FurnaceData::default();
        let outcome = tick_one(&mut f);
        assert_eq!(outcome.recipe_completed, None);
        assert_eq!(outcome.lit_changed, None);
        assert!(!f.lit);
        assert_eq!(f.smelt_progress, 0);
    }

    #[test]
    fn tick_consumes_one_fuel_unit_and_lights() {
        // Spec 29 — these tick-mechanics tests use RawIron (canonical
        // ore smelt) since RawBeef no longer matches a recipe.
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 2)));
        let outcome = tick_one(&mut f);
        // First tick consumes one fuel unit, doesn't progress smelt.
        assert_eq!(outcome.lit_changed, Some(true));
        assert!(f.lit);
        assert_eq!(f.fuel_ticks_remaining, 4800); // Coal fuel value
        assert_eq!(f.smelt_progress, 0);
        assert_eq!(f.fuel.as_ref().unwrap().count, 1);
    }

    #[test]
    fn coal_burns_down_and_completes_smelt() {
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 1)));
        // Tick 1: consume fuel, light, no progress.
        tick_one(&mut f);
        // Now burn through SMELT_TICKS_PER_ITEM ticks to complete one smelt.
        let mut completed = None;
        for _ in 0..SMELT_TICKS_PER_ITEM {
            let o = tick_one(&mut f);
            if o.recipe_completed.is_some() {
                completed = o.recipe_completed;
            }
        }
        assert_eq!(completed, Some(MaterialId::IronIngot));
        // Output stack should have 1 IronIngot.
        let out = f.output.as_ref().unwrap();
        assert_eq!(out.count, 1);
        match &out.item {
            Item::Material(m) => assert_eq!(*m, MaterialId::IronIngot),
            _ => panic!("output should be Material"),
        }
        // Input slot consumed.
        assert!(f.input.is_none());
    }

    #[test]
    fn smelt_halts_when_input_removed_mid_smelt() {
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 1)));
        tick_one(&mut f); // fuel consume tick
        // Advance partway.
        for _ in 0..50 {
            tick_one(&mut f);
        }
        assert!(f.smelt_progress > 0);
        assert!(f.lit);
        // Remove the input.
        f.input = None;
        let outcome = tick_one(&mut f);
        // Lit flag drops; progress resets.
        assert_eq!(outcome.lit_changed, Some(false));
        assert!(!f.lit);
        assert_eq!(f.smelt_progress, 0);
        // Fuel is preserved (partially burnt).
        assert!(f.fuel_ticks_remaining > 0);
    }

    #[test]
    fn smelt_halts_when_no_fuel_available() {
        // Input but no fuel — first tick should NOT light.
        let mut f = furnace_with(Some(MaterialId::RawIron), None);
        let outcome = tick_one(&mut f);
        assert_eq!(outcome.lit_changed, None);
        assert!(!f.lit);
        assert_eq!(f.smelt_progress, 0);
    }

    #[test]
    fn output_slot_blocks_wrong_material() {
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 1)));
        // Pre-fill output with CopperIngot — doesn't match RawIron's
        // IronIngot output, so smelt should be gated.
        f.output = Some(ItemStack::new_material(MaterialId::CopperIngot, 1));
        let outcome = tick_one(&mut f);
        assert_eq!(outcome.lit_changed, None);
        assert!(!f.lit);
        assert_eq!(f.smelt_progress, 0);
        // Fuel is untouched — no work happened.
        assert_eq!(f.fuel_ticks_remaining, 0);
    }

    #[test]
    fn output_slot_blocks_when_saturated() {
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 1)));
        // Pre-fill output with MAX_OUTPUT_STACK IronIngot — full.
        f.output = Some(ItemStack::new_material(MaterialId::IronIngot, MAX_OUTPUT_STACK));
        let outcome = tick_one(&mut f);
        assert_eq!(outcome.lit_changed, None);
        assert!(!f.lit);
    }

    #[test]
    fn smelt_progress_preserved_when_fuel_runs_dry_midway() {
        // Post-merge review #10: when fuel is exhausted mid-smelt,
        // `smelt_progress` should NOT reset. Dropping more fuel in
        // should resume the smelt from where it stalled.
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Stick, 1)));
        // Tick 1: fuel-consume (sets fuel_ticks_remaining = 40).
        tick_one(&mut f);
        assert_eq!(f.fuel_ticks_remaining, 40);
        // Burn 40 ticks — fuel exhausted, smelt_progress at 40.
        for _ in 0..40 {
            tick_one(&mut f);
        }
        assert_eq!(f.fuel_ticks_remaining, 0);
        let progress_at_stall = f.smelt_progress;
        assert!(progress_at_stall > 0, "smelt should have advanced");
        // Several stall ticks with no fuel — progress holds, lit drops.
        for _ in 0..30 {
            tick_one(&mut f);
        }
        assert_eq!(f.smelt_progress, progress_at_stall);
        assert!(!f.lit);
    }

    #[test]
    fn ticks_accumulate_into_stacked_output() {
        // Confirm a second smelt into an existing output stack
        // increments count, doesn't create a parallel stack.
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 1)));
        // Bump input to 2.
        f.input.as_mut().unwrap().count = 2;
        // Burn through two smelts. Total = 1 fuel-consume + 2 * SMELT_TICKS_PER_ITEM ticks.
        for _ in 0..(1 + 2 * SMELT_TICKS_PER_ITEM) {
            tick_one(&mut f);
        }
        let out = f.output.as_ref().unwrap();
        assert_eq!(out.count, 2);
        // Input fully consumed.
        assert!(f.input.is_none());
    }

    // ── Spec 29 — slot-click free function ────────────────────────────

    fn make_inv_with(material: MaterialId, count: u8, hotbar_slot: usize) -> crate::inventory::Inventory {
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(hotbar_slot, Some(ItemStack::new_material(material, count)));
        inv
    }

    #[test]
    fn slot_click_input_with_smeltable_places_one() {
        let mut f = FurnaceData::default();
        let mut inv = make_inv_with(MaterialId::RawIron, 5, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::Moved { count: 1 }));
        assert_eq!(f.input.as_ref().unwrap().count, 1);
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 4);
    }

    #[test]
    fn slot_click_input_stack_mode_places_all() {
        let mut f = FurnaceData::default();
        let mut inv = make_inv_with(MaterialId::RawIron, 5, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Stack);
        assert!(matches!(result, SlotClickResult::Moved { count: 5 }));
        assert_eq!(f.input.as_ref().unwrap().count, 5);
        assert!(inv.hotbar_slot(0).is_none());
    }

    #[test]
    fn slot_click_input_rejects_non_smeltable() {
        let mut f = FurnaceData::default();
        let mut inv = make_inv_with(MaterialId::Stick, 5, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::NoOp));
        assert!(f.input.is_none());
        // Inventory unchanged.
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 5);
    }

    #[test]
    fn slot_click_input_rejects_raw_meat_post_spec29() {
        // Locks the Spec 29 promise: putting RawBeef in the input slot
        // via a slot click is a no-op. Cooking moves to the campfire.
        let mut f = FurnaceData::default();
        let mut inv = make_inv_with(MaterialId::RawBeef, 5, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::NoOp));
        assert!(f.input.is_none());
    }

    #[test]
    fn slot_click_input_refuses_mismatched_material() {
        // Slot already has RawIron — click while holding Copper should
        // refuse (no swap, no overwrite — alpha-safe).
        let mut f = FurnaceData::default();
        f.input = Some(ItemStack::new_material(MaterialId::RawIron, 2));
        let mut inv = make_inv_with(MaterialId::Copper, 3, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::NoOp));
        assert_eq!(f.input.as_ref().unwrap().count, 2);
        // Material in the slot didn't change.
        match &f.input.as_ref().unwrap().item {
            crate::item::Item::Material(m) => assert_eq!(*m, MaterialId::RawIron),
            _ => panic!("expected RawIron"),
        }
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 3);
    }

    #[test]
    fn slot_click_fuel_with_fuel_item_places() {
        let mut f = FurnaceData::default();
        let mut inv = make_inv_with(MaterialId::Coal, 5, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Fuel, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::Moved { count: 1 }));
        assert_eq!(f.fuel.as_ref().unwrap().count, 1);
    }

    #[test]
    fn slot_click_fuel_rejects_non_fuel() {
        let mut f = FurnaceData::default();
        let mut inv = make_inv_with(MaterialId::RawIron, 5, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Fuel, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::NoOp));
        assert!(f.fuel.is_none());
    }

    #[test]
    fn slot_click_input_empty_hand_takes_to_inventory() {
        let mut f = FurnaceData::default();
        f.input = Some(ItemStack::new_material(MaterialId::RawIron, 3));
        let mut inv = crate::inventory::Inventory::new();
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::Moved { count: 1 }));
        assert_eq!(f.input.as_ref().unwrap().count, 2);
        // Inventory gained 1 RawIron.
        let inv_has = inv.slots_iter().flatten()
            .any(|s| matches!(s.item, crate::item::Item::Material(MaterialId::RawIron)));
        assert!(inv_has, "inventory should now hold 1 RawIron");
    }

    #[test]
    fn slot_click_input_empty_hand_stack_mode_takes_all() {
        let mut f = FurnaceData::default();
        f.input = Some(ItemStack::new_material(MaterialId::RawIron, 4));
        let mut inv = crate::inventory::Inventory::new();
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Stack);
        assert!(matches!(result, SlotClickResult::Moved { count: 4 }));
        assert!(f.input.is_none());
    }

    #[test]
    fn slot_click_output_empty_hand_takes_to_inventory() {
        let mut f = FurnaceData::default();
        f.output = Some(ItemStack::new_material(MaterialId::IronIngot, 2));
        let mut inv = crate::inventory::Inventory::new();
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Output, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::Moved { count: 1 }));
        assert_eq!(f.output.as_ref().unwrap().count, 1);
    }

    #[test]
    fn slot_click_output_with_held_item_still_takes() {
        // Spec 29 — Output is read-only on insert; clicking Output
        // while holding an item ignores the held item and just takes
        // from output (matches Minecraft).
        let mut f = FurnaceData::default();
        f.output = Some(ItemStack::new_material(MaterialId::IronIngot, 1));
        let mut inv = make_inv_with(MaterialId::Coal, 3, 0);
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Output, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::Moved { count: 1 }));
        assert!(f.output.is_none()); // 1 taken; output empty
        // The held coal is unchanged.
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 3);
    }

    #[test]
    fn slot_click_empty_slot_empty_hand_is_noop() {
        let mut f = FurnaceData::default();
        let mut inv = crate::inventory::Inventory::new();
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Input, ClickMode::Single);
        assert!(matches!(result, SlotClickResult::NoOp));
    }

    // ── Spec 29 — legacy raw-meat eject hook ─────────────────────────

    #[test]
    fn is_legacy_food_input_flags_only_raw_meats() {
        // The four raw meats that v1 could smelt are the eject set.
        for m in [
            MaterialId::RawBeef,
            MaterialId::RawChicken,
            MaterialId::RawMutton,
            MaterialId::RawPorkchop,
        ] {
            assert!(is_legacy_food_input(&ItemStack::new_material(m, 1)),
                "{:?} should be flagged as legacy food", m);
        }
        // Ores and other smeltables are NOT legacy food.
        for m in [
            MaterialId::RawIron,
            MaterialId::Copper,
            MaterialId::Tin,
            MaterialId::Stick,
            MaterialId::Wheat,
        ] {
            assert!(!is_legacy_food_input(&ItemStack::new_material(m, 1)),
                "{:?} should NOT be flagged as legacy food", m);
        }
    }

    #[test]
    fn eject_legacy_food_clears_meat_and_returns_stack() {
        let mut f = furnace_with(Some(MaterialId::RawBeef), None);
        f.input.as_mut().unwrap().count = 5;
        let ejected = eject_legacy_food_from_furnace(&mut f);
        // Stack is returned with full count for entity-drop spawning.
        let stack = ejected.expect("RawBeef input should be ejected");
        assert_eq!(stack.count, 5);
        match stack.item {
            Item::Material(m) => assert_eq!(m, MaterialId::RawBeef),
            _ => panic!("ejected item should be Material(RawBeef)"),
        }
        // Input slot is now empty.
        assert!(f.input.is_none());
    }

    #[test]
    fn eject_legacy_food_leaves_ore_intact() {
        let mut f = furnace_with(Some(MaterialId::RawIron), None);
        let ejected = eject_legacy_food_from_furnace(&mut f);
        assert!(ejected.is_none(), "RawIron must not be ejected — it's a valid smelt input");
        assert!(f.input.is_some(), "RawIron input slot must be preserved");
    }

    #[test]
    fn eject_legacy_food_handles_empty_furnace() {
        let mut f = FurnaceData::default();
        let ejected = eject_legacy_food_from_furnace(&mut f);
        assert!(ejected.is_none());
        assert!(f.input.is_none());
    }

    #[test]
    fn eject_into_world_pending_walks_all_furnaces() {
        // World-level helper: walks every furnace, ejects legacy meat
        // into world.pending_legacy_meat_drops for the caller (GameState
        // or GameServer) to spawn as ItemEntities on its next tick.
        use crate::world::World;
        let mut world = World::new();
        let mut beef = FurnaceData::default();
        beef.input = Some(ItemStack::new_material(MaterialId::RawBeef, 3));
        let mut iron = FurnaceData::default();
        iron.input = Some(ItemStack::new_material(MaterialId::RawIron, 2));
        world.insert_furnace((1, 64, 1), beef);
        world.insert_furnace((10, 64, 10), iron);

        eject_legacy_food_into_world_pending(&mut world);

        // One ejection — only the beef furnace.
        assert_eq!(world.pending_legacy_meat_drops.len(), 1);
        let (pos, stack) = &world.pending_legacy_meat_drops[0];
        assert_eq!(*pos, (1, 64, 1));
        assert_eq!(stack.count, 3);
        match &stack.item {
            Item::Material(m) => assert_eq!(*m, MaterialId::RawBeef),
            _ => panic!("ejected item should be RawBeef material"),
        }
        // Beef furnace input cleared; iron furnace untouched.
        assert!(world.furnace_at((1, 64, 1)).unwrap().input.is_none());
        assert!(world.furnace_at((10, 64, 10)).unwrap().input.is_some());
    }

    #[test]
    fn eject_legacy_food_does_not_touch_fuel_or_output() {
        // Defence-in-depth: the hook MUST only touch the input slot.
        // Fuel + output may legitimately hold any item from v1 saves;
        // resetting them would lose the player's coal / smelted ingots.
        let mut f = furnace_with(Some(MaterialId::RawBeef), Some((MaterialId::Coal, 4)));
        f.output = Some(ItemStack::new_material(MaterialId::IronIngot, 3));
        let _ = eject_legacy_food_from_furnace(&mut f);
        // Fuel and output untouched.
        assert!(f.fuel.is_some());
        assert_eq!(f.fuel.as_ref().unwrap().count, 4);
        assert!(f.output.is_some());
        assert_eq!(f.output.as_ref().unwrap().count, 3);
    }

    #[test]
    fn fuel_value_matches_campfire() {
        // Sanity: furnace and campfire share fuel_value so a coal in
        // a campfire and a coal in a furnace have the same burn-time.
        assert_eq!(
            fuel_value(Some(MaterialId::Coal), None),
            crate::campfire::fuel_value(Some(MaterialId::Coal), None),
        );
        assert_eq!(
            fuel_value(None, Some(crate::block::OAK_PLANKS)),
            crate::campfire::fuel_value(None, Some(crate::block::OAK_PLANKS)),
        );
    }

    // ── Task 1 (wave-hardening) — cleanup_furnace wiring ─────────────

    #[test]
    fn cleanup_furnace_empties_returns_contents_and_is_idempotent() {
        // Mirrors chest.rs's `cleanup_chest_empties_returns_contents_and_is_idempotent`
        // (its exact structural sibling) — locks the previously-untested
        // `cleanup_furnace` contract now that game_loop.rs calls it.
        use crate::world::World;
        let mut world = World::new();
        let mut f = FurnaceData::default();
        f.input = Some(ItemStack::new_material(MaterialId::RawIron, 4));
        f.fuel = Some(ItemStack::new_material(MaterialId::Coal, 2));
        f.output = Some(ItemStack::new_material(MaterialId::IronIngot, 3));
        world.insert_furnace((10, 70, 10), f);

        let spill = cleanup_furnace(&mut world, 10, 70, 10);
        assert_eq!(spill.len(), 3, "three non-empty slots -> three spill stacks");
        assert!(world.furnace_at((10, 70, 10)).is_none(), "block-entity removed");

        let spill2 = cleanup_furnace(&mut world, 10, 70, 10);
        assert!(spill2.is_empty(), "second cleanup is a no-op");
    }

    #[test]
    fn cleanup_furnace_on_a_never_furnace_cell_is_a_noop() {
        use crate::world::World;
        let mut world = World::new();
        let spill = cleanup_furnace(&mut world, 4, 64, 4);
        assert!(spill.is_empty());
    }

    #[test]
    fn cleanup_furnace_skips_zero_count_stacks() {
        // Wave-hardening backlog (2026-07-11) — mirror `cleanup_chest`'s
        // `count > 0` spill guard. A zero-count stack must not spill as a
        // phantom item entity.
        use crate::world::World;
        let mut world = World::new();
        let mut f = FurnaceData::default();
        f.input = Some(ItemStack::new_material(MaterialId::RawIron, 0));
        f.fuel = Some(ItemStack::new_material(MaterialId::Coal, 2));
        world.insert_furnace((10, 70, 10), f);

        let spill = cleanup_furnace(&mut world, 10, 70, 10);
        assert_eq!(spill.len(), 1, "the zero-count input must not spill");
    }

    #[test]
    fn ledger_report_output_take_clears_and_never_repopulates() {
        // Regression for the "ghost iron" ledger report sequence: smelt 4
        // RawIron to a 4-count IronIngot output, take the whole stack via
        // the slot-click path into an inventory, then confirm further
        // ticks never repopulate the output once input is exhausted.
        let mut f = furnace_with(Some(MaterialId::RawIron), Some((MaterialId::Coal, 1)));
        f.input.as_mut().unwrap().count = 4;
        // One fuel-consume tick + 4 full smelts (matches
        // `ticks_accumulate_into_stacked_output`'s arithmetic, scaled to 4).
        for _ in 0..(1 + 4 * SMELT_TICKS_PER_ITEM) {
            tick_one(&mut f);
        }
        assert_eq!(f.output.as_ref().unwrap().count, 4);
        assert!(f.input.is_none(), "all 4 RawIron consumed");

        let mut inv = crate::inventory::Inventory::new();
        let result = apply_slot_click(&mut f, &mut inv, 0, SlotKind::Output, ClickMode::Stack);
        assert!(matches!(result, SlotClickResult::Moved { count: 4 }));
        assert!(f.output.is_none(), "output slot cleared after taking the whole stack");

        let iron_count: u32 = inv
            .slots_iter()
            .flatten()
            .filter(|s| matches!(s.item, Item::Material(MaterialId::IronIngot)))
            .map(|s| u32::from(s.count))
            .sum();
        assert_eq!(iron_count, 4, "inventory should hold exactly the 4 smelted ingots");

        // Further ticks must not repopulate output — input is empty so the
        // smelt is gated (tick_one's `recipe_output.is_none()` branch).
        for _ in 0..50 {
            tick_one(&mut f);
        }
        assert!(f.output.is_none(), "output must not resurrect once emptied");
    }
}
