//! Campfire — block-entity state + fuel-burn + cook-progress tick.
//!
//! Per Spec 17 / `docs/foundations/2026-05-18-campfire.md`. Phase 3 owns
//! the data shape + persistence; Phase 4 fills in the tick body; Phase 6
//! adds the friction/flint ignition paths.
//!
//! State lives in `World::block_entities` — a per-position map. Each
//! campfire block (lit OR unlit, ids 43/44) has a corresponding
//! [`CampfireData`] entry that tracks fuel + cooking slots. When a
//! campfire is mined, the caller is responsible for removing its
//! block-entity entry.

use serde::{Deserialize, Serialize};

use crate::block::{self, BlockId};
use crate::item::MaterialId;

/// Number of cooking slots on a single campfire — one per top-face quadrant.
pub const CAMPFIRE_SLOTS: usize = 4;

/// Ticks of cooking time per item to reach the cooked variant. 200 ticks
/// = 10 s @ 20 TPS — kid-patience-friendly per the spec.
pub const COOK_TICKS_PER_ITEM: u32 = 200;

/// A single cooking slot on a campfire — either empty or holding a raw
/// item with its current cook-progress (in ticks).
///
/// **This travels on the wire** (`protocol::BlockView::Campfire`, v79) as well
/// as in the save: any change to its fields, or to how `MaterialId`
/// serialises, changes the protocol — bump `PROTOCOL_VERSION`
/// (`protocol::tests::campfire_and_rack_views_are_pinned_on_the_full_bytes`
/// pins the bytes and fails on any such change: re-pin it with the bump).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CookSlot {
    pub item: Option<MaterialId>,
    pub progress_ticks: u32,
}

/// State stored per-campfire in `World::block_entities`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CampfireData {
    /// Ticks of fuel remaining. 0 = unlit (the block must be
    /// `CAMPFIRE_UNLIT`). Decrements by 1 per engine tick while
    /// the block is `CAMPFIRE`. When 0 and the block is `CAMPFIRE`,
    /// the tick transitions it to `CAMPFIRE_UNLIT`. When > 0 and the
    /// block is `CAMPFIRE_UNLIT`, the tick transitions to `CAMPFIRE`
    /// (lit).
    pub fuel_ticks: u32,
    /// Cooking slots — up to CAMPFIRE_SLOTS in flight at once.
    pub slots: [CookSlot; CAMPFIRE_SLOTS],
    /// Ticks of smoke remaining. Bumped when leaves are added as fuel
    /// (Wave 28). Decrements every engine tick. While > 0 AND the
    /// campfire is lit, the tick renders a CAMPFIRE_SMOKE pillar above.
    /// Defaulted via `#[serde(default)]` for backward-compat with
    /// pre-Wave-28 saves.
    #[serde(default)]
    pub smoke_ticks: u32,
    /// Spec 30 — ticks of **smoulder** remaining after fuel hits 0.
    /// While `> 0`, the campfire is visually unlit (block-id stays
    /// CAMPFIRE_UNLIT) but accepts fuel without re-ignition: dropping
    /// a fuel item in flips it straight back to lit. After this
    /// counter hits 0 (default `SMOULDER_TICKS` = 600 = 30 s @ 20 TPS),
    /// the campfire is fully cold and requires friction or flint-
    /// and-steel to relight. Defaulted via `#[serde(default)]`.
    #[serde(default)]
    pub smoulder_ticks: u32,
    /// Spec 22 Phase 7 — raid-warning red-tint flag. While `true`, the
    /// smoke pillar above this campfire renders in red (atmospheric
    /// "raid incoming" signal). Set by the raid scheduler when this
    /// campfire belongs to a warned village; cleared when the raid
    /// resolves. Default `false` for backward-compat with pre-Spec-22
    /// saves. Lives on the existing block-entity (per open question 1
    /// in the spec: a flag, not a new block-id slot).
    #[serde(default)]
    pub raid_warning_active: bool,
}

/// Default smoulder duration after fuel runs out — 30 in-game seconds
/// at 20 TPS. Tunable post-playtest per Spec 30 Open Question 1.
pub const SMOULDER_TICKS: u32 = 600;

impl CampfireData {
    pub fn is_lit(&self) -> bool {
        self.fuel_ticks > 0
    }

    /// Spec 30 — true iff fuel is exhausted but the campfire is still
    /// in its smoulder window. Used by the hover UI label + by the
    /// right-click handler to decide whether to instant-relight vs
    /// require friction/flint-and-steel.
    pub fn is_smouldering(&self) -> bool {
        self.fuel_ticks == 0 && self.smoulder_ticks > 0
    }

    /// Spec 30 — true iff fully cold (needs friction or flint-and-
    /// steel to relight). The "cold" predicate covers any state that
    /// isn't lit and isn't smouldering. No production caller — tested
    /// directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_cold(&self) -> bool {
        self.fuel_ticks == 0 && self.smoulder_ticks == 0
    }

    /// First empty cooking slot, if any.
    pub fn empty_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.item.is_none())
    }

    /// First slot containing a cooked item — used by the right-click
    /// pickup path to claim cooked meat back into the player's inventory.
    /// Returns the slot index and the cooked-material id.
    pub fn first_cooked_slot(&self) -> Option<(usize, MaterialId)> {
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(raw) = slot.item
                && slot.progress_ticks >= COOK_TICKS_PER_ITEM
                    && let Some(cooked) = cooked_variant(raw) {
                        return Some((i, cooked));
                    }
        }
        None
    }

    /// Add fuel ticks. Saturating add — overflow at u32::MAX is fine
    /// (it would take ~6 years of in-game time to overflow, far past
    /// any session).
    pub fn add_fuel(&mut self, ticks: u32) {
        self.fuel_ticks = self.fuel_ticks.saturating_add(ticks);
    }

    /// Place a raw item in the first empty slot. Returns true on success;
    /// false if every slot is occupied.
    pub fn try_place_raw(&mut self, raw: MaterialId) -> bool {
        if let Some(idx) = self.empty_slot() {
            self.slots[idx] = CookSlot { item: Some(raw), progress_ticks: 0 };
            true
        } else {
            false
        }
    }
}

/// True iff striking this campfire (stick friction OR flint-and-steel)
/// should actually light it — i.e. there is fuel to burn. Lighting an
/// unfueled campfire just extinguishes on the next fuel-burn tick (see
/// [`tick_one`]), so the ignition handlers gate on this and guide the
/// player to add fuel first instead of wasting the attempt on a dead
/// flash. `None` (no block-entity yet — a freshly placed, never-fuelled
/// campfire) and a smouldering-but-fuel-less campfire both return false.
pub fn can_ignite(cf: Option<&CampfireData>) -> bool {
    cf.map(|c| c.fuel_ticks > 0).unwrap_or(false)
}

/// Burn-time per fuel item, in 20-TPS ticks. Returns `None` if the
/// material/block isn't a recognised fuel. Fuel ladder (Wave 17 + 29):
/// leaves 1 s / sticks 2 s / green log 30 s / planks 12 s / placed-oak-log
/// block 60 s / seasoned log 60 s / coal 240 s / kiln-dried log 120 s.
pub fn fuel_value(material: Option<MaterialId>, block: Option<BlockId>) -> Option<u32> {
    if let Some(b) = block {
        return match b {
            block::OAK_LEAVES => Some(20),    // 1 s — fastest kindling
            block::OAK_PLANKS => Some(12 * 20),   // 12 s — light fuel
            // Back-compat: old saves carry oak-log as a block-item in
            // inventory (pre-Wave-29). New mines drop GreenLog material.
            // Both burn the same 60 s for old-save players.
            block::OAK_LOG => Some(60 * 20),      // 60 s — standard fuel
            _ => None,
        };
    }
    if let Some(m) = material {
        return match m {
            MaterialId::Stick => Some(2 * 20),    // 2 s — kindling+
            MaterialId::Coal => Some(240 * 20),   // 240 s — premium fuel
            // Wave 29 — log seasoning ladder. Green wood burns short +
            // smokes (see is_smoky_fuel); seasoned burns clean; kiln-dried
            // burns premium. The fuel-value table is the single source of
            // truth — Spec 20 Furnace will read it the same way.
            MaterialId::GreenLog => Some(30 * 20),       // 30 s — fresh-cut, half a seasoned burn
            MaterialId::SeasonedLog => Some(60 * 20),    // 60 s — same as old oak-log block-item
            MaterialId::KilnDriedLog => Some(120 * 20),  // 120 s — kiln output (future production)
            _ => None,
        };
    }
    None
}

/// Whether burning this fuel produces a smoke pillar (Wave 28 mechanic).
/// Green wood + leaves are the two smoky fuels; everything else burns
/// clean. Called by the right-click-fuel-add handler to decide whether
/// to bump the campfire's `smoke_ticks` counter.
pub fn is_smoky_fuel(material: Option<MaterialId>, block: Option<BlockId>) -> bool {
    matches!(block, Some(block::OAK_LEAVES))
        || matches!(material, Some(MaterialId::GreenLog))
}

/// Smoke ticks added when a single GreenLog is burned as fuel. 1200 ticks
/// = 60 s — 2× the log's 30 s burn so the smoke beacon outlives the flame,
/// matching the precedent of leaves (1 s burn / 3 s smoke).
pub const SMOKE_TICKS_PER_GREEN_LOG: u32 = 1200;

/// Map a raw input material to its cooked / baked variant. Returns
/// `None` for non-cookable inputs. Covers both meats (Wave 27) and
/// vegetables (Wave 28: Potato / Carrot / Corn → baked).
pub fn cooked_variant(raw: MaterialId) -> Option<MaterialId> {
    match raw {
        MaterialId::RawBeef => Some(MaterialId::CookedBeef),
        MaterialId::RawPorkchop => Some(MaterialId::CookedPorkchop),
        MaterialId::RawChicken => Some(MaterialId::CookedChicken),
        MaterialId::RawMutton => Some(MaterialId::CookedMutton),
        // Wave 28 — vegetables that bake on the campfire.
        MaterialId::Potato => Some(MaterialId::BakedPotato),
        MaterialId::Carrot => Some(MaterialId::BakedCarrot),
        MaterialId::Corn => Some(MaterialId::BakedCorn),
        // P6 — cook the fishing catch.
        MaterialId::RawFish => Some(MaterialId::CookedFish),
        _ => None,
    }
}

/// Whether a material is a cookable raw food (used by the right-click
/// "place meat" branch).
pub fn is_raw_cookable(m: MaterialId) -> bool {
    cooked_variant(m).is_some()
}

/// Whether the smoke pillar should be present after this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmokeState {
    /// Caller should ensure a CAMPFIRE_SMOKE pillar exists above the
    /// campfire (place blocks in air cells from y+1 upward).
    On,
    /// Caller should clear any CAMPFIRE_SMOKE blocks above the campfire.
    Off,
    /// No change required from the previous tick.
    NoChange,
}

/// Outcome of advancing one campfire by one tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampfireTickOutcome {
    /// New block id at this position after the tick. `None` means no
    /// change; `Some(id)` means caller should `world.set_block` and
    /// broadcast the block change.
    pub block_change: Option<BlockId>,
    /// Smoke pillar lifecycle decision. See [`SmokeState`].
    pub smoke_state: SmokeState,
}

/// How many engine ticks of smoke each OAK_LEAVES fuel-add grants. Leaves
/// burn for ~1 s (20 ticks) but smoke lingers ~3 s (60 ticks) so the
/// signal is actually readable.
pub const SMOKE_TICKS_PER_LEAF: u32 = 60;

/// Height of the smoke pillar in blocks above the campfire.
pub const SMOKE_PILLAR_HEIGHT: i32 = 6;

/// Place a CAMPFIRE_SMOKE pillar of [`SMOKE_PILLAR_HEIGHT`] blocks above
/// `(x, y, z)`. Only writes into cells that are currently AIR — never
/// overwrites a player-placed block. Returns the list of positions that
/// were actually written (for chunk-rebuild / broadcast).
pub fn place_smoke_pillar(world: &mut crate::world::World, x: i32, y: i32, z: i32) -> Vec<(i32, i32, i32)> {
    let mut placed = Vec::new();
    for dy in 1..=SMOKE_PILLAR_HEIGHT {
        let py = y + dy;
        if world.get_block(x, py, z) == block::AIR {
            world.set_block(x, py, z, block::CAMPFIRE_SMOKE);
            placed.push((x, py, z));
        } else {
            // Hit a non-air block (e.g. a ceiling) — pillar stops here.
            break;
        }
    }
    placed
}

/// Clear any CAMPFIRE_SMOKE blocks in the column above `(x, y, z)` up to
/// [`SMOKE_PILLAR_HEIGHT`]. Returns positions cleared (for chunk rebuild).
/// Stops at the first non-smoke block — won't punch through player-placed
/// blocks above a campfire whose smoke pillar terminated against them.
pub fn clear_smoke_pillar(world: &mut crate::world::World, x: i32, y: i32, z: i32) -> Vec<(i32, i32, i32)> {
    let mut cleared = Vec::new();
    for dy in 1..=SMOKE_PILLAR_HEIGHT {
        let py = y + dy;
        if world.get_block(x, py, z) == block::CAMPFIRE_SMOKE {
            world.set_block(x, py, z, block::AIR);
            cleared.push((x, py, z));
        } else {
            break;
        }
    }
    cleared
}

/// Cleanup hook for the campfire at `(x, y, z)` when its block is being
/// destroyed (mined, blown up, /setblock-replaced, etc). Removes the
/// block-entity entry, spills whatever was in its cook slots (the raw
/// item if still cooking, the cooked item if progress had already
/// finished — matching what the right-click pickup path would have
/// handed the player), AND clears any smoke pillar above. Idempotent —
/// safe to call on cells that were never campfires.
///
/// Mirrors `furnace::cleanup_furnace`'s spill contract — before this
/// fix, breaking a lit campfire silently destroyed anything still
/// cooking in it (wiki audit 2026-07-09, finding #2: the furnace got
/// this fix, the campfire never did).
pub fn cleanup_campfire(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    z: i32,
) -> (Vec<crate::item::ItemStack>, Vec<(i32, i32, i32)>) {
    let spill = cleanup_campfire_keep_smoke(world, x, y, z);
    let cleared = clear_smoke_pillar(world, x, y, z);
    (spill, cleared)
}

/// [`cleanup_campfire`] without the smoke-pillar clear: the entry removed and
/// the cook slots spilled, the pillar cells left as they are (FU4b, FU3 verify
/// L5). A joined client whose edits reach the server (`edits_reach_server()`)
/// breaks a campfire this way: the server's `on_block_edit` clears the pillar
/// and broadcasts the cells, so the joiner's own clear was a guess that a
/// refused break (reach, plot) left standing on its screen — a restored fire
/// with no smoke. The server's block changes clear it instead.
pub fn cleanup_campfire_keep_smoke(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<crate::item::ItemStack> {
    let mut spill = Vec::new();
    if let Some(cf) = world.campfire_at((x, y, z)) {
        for slot in &cf.slots {
            if let Some(raw) = slot.item {
                let material = if slot.progress_ticks >= COOK_TICKS_PER_ITEM {
                    cooked_variant(raw).unwrap_or(raw)
                } else {
                    raw
                };
                spill.push(crate::item::ItemStack::new_material(material, 1));
            }
        }
    }
    world.block_entities.remove(&(x, y, z));
    spill
}

/// The smoke pillar a campfire gets the moment it is lit: placed when the
/// fire is smoky (`smoke_ticks > 0` — leaves or green logs went in before it
/// was lit), since [`tick_one`]'s `On` signal never fires the unlit → lit
/// transition (fuel-add doesn't auto-ignite). Returns the cells placed. The
/// client's flint-and-steel and friction arms run it on their own world; the
/// server runs it (via [`on_block_edit`]) on a joiner's lighting.
pub fn smoke_on_light(world: &mut crate::world::World, x: i32, y: i32, z: i32) -> Vec<(i32, i32, i32)> {
    if world.campfire_at((x, y, z)).is_some_and(|cf| cf.smoke_ticks > 0) {
        place_smoke_pillar(world, x, y, z)
    } else {
        Vec::new()
    }
}

/// What a campfire block edit does around it ([`on_block_edit`]).
#[derive(Debug, Default, PartialEq)]
pub struct CampfireEdit {
    /// What was cooking on a broken fire ([`cleanup_campfire`]), to spill.
    pub spill: Vec<crate::item::ItemStack>,
    /// The smoke cells it wrote, each with its new block (`CAMPFIRE_SMOKE`
    /// placed, `AIR` cleared), to broadcast.
    pub smoke: Vec<((i32, i32, i32), BlockId)>,
}

fn is_campfire(b: BlockId) -> bool {
    b == block::CAMPFIRE || b == block::CAMPFIRE_UNLIT
}

/// FU3 (FU1 verify N3) — the campfire rule for an edit `old → new` at `cell`
/// (already applied, or about to be: it touches only the block entity and the
/// cells above): a broken fire is cleaned up ([`cleanup_campfire`]: its smoke
/// cleared, what was cooking returned to spill), a lit one raises its pillar
/// if smoky ([`smoke_on_light`]), and one going out clears its pillar (as
/// [`tick_one`]'s `Off`). Anything else: nothing.
///
/// The server runs it on every accepted JOINER edit (`HostedServer`), so a
/// joiner sends one edit for a campfire action, not the up to six pillar
/// cells behind it — which used to overflow the server's per-tick edit
/// budget and leave the refused cells as smoke floating in the shared world.
/// A host's own edits already carry their smoke (they are made in the world
/// the server ticks, or sent as edits by a `--no-lend` host's client).
pub fn on_block_edit(
    world: &mut crate::world::World,
    (x, y, z): (i32, i32, i32),
    old: BlockId,
    new: BlockId,
) -> CampfireEdit {
    if is_campfire(old) && !is_campfire(new) {
        let (spill, cleared) = cleanup_campfire(world, x, y, z);
        return CampfireEdit { spill, smoke: cleared.into_iter().map(|c| (c, block::AIR)).collect() };
    }
    let smoke = if old == block::CAMPFIRE_UNLIT && new == block::CAMPFIRE {
        smoke_on_light(world, x, y, z).into_iter().map(|c| (c, block::CAMPFIRE_SMOKE)).collect()
    } else if old == block::CAMPFIRE && new == block::CAMPFIRE_UNLIT {
        clear_smoke_pillar(world, x, y, z).into_iter().map(|c| (c, block::AIR)).collect()
    } else {
        Vec::new()
    };
    CampfireEdit { spill: Vec::new(), smoke }
}

/// Block-radius around a lit campfire where mobs feel "too hot" to
/// approach further. Scales with current fuel level — more fuel → bigger
/// roaring fire → wider safe zone. The log-of-fuel mapping gives a
/// gentle curve: small kindling fires barely warn, a coal-fueled blaze
/// clears a real ring of ground.
///
/// 200 fuel (1 plank, just lit) → 2 blocks
/// 1200 fuel (1 log) → ~5 blocks
/// 4800 fuel (1 coal) → 6 blocks (clamped)
pub fn heat_radius_blocks(fuel_ticks: u32) -> f32 {
    if fuel_ticks == 0 {
        return 0.0;
    }
    let logf = (fuel_ticks as f32).log2();
    (logf - 5.0).clamp(2.0, 6.0)
}

/// Find the nearest LIT campfire to `mob_pos` within `range_blocks`.
/// Returns the campfire's block-position and its current heat-radius.
/// Used by mob AI to decide whether to investigate a fire. O(N) over
/// `world.block_entities` — fine while N is small (handfuls of campfires
/// per loaded world); a spatial index lands when this becomes a hot path.
pub fn nearest_lit_campfire(
    world: &crate::world::World,
    mob_pos: glam::Vec3,
    range_blocks: f32,
) -> Option<((i32, i32, i32), f32)> {
    let range_sq = range_blocks * range_blocks;
    let mut best: Option<((i32, i32, i32), f32)> = None;
    let mut best_dist_sq = range_sq;
    for ((x, y, z), data) in world.iter_campfires() {
        if !data.is_lit() {
            continue;
        }
        // Skip orphan block-entities whose block has been destroyed —
        // defence in depth alongside cleanup_campfire.
        if world.get_block(x, y, z) != block::CAMPFIRE {
            continue;
        }
        let dx = x as f32 + 0.5 - mob_pos.x;
        let dy = y as f32 + 0.5 - mob_pos.y;
        let dz = z as f32 + 0.5 - mob_pos.z;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 < best_dist_sq {
            best_dist_sq = d2;
            best = Some(((x, y, z), heat_radius_blocks(data.fuel_ticks)));
        }
    }
    best
}

/// Advance one campfire's state by one tick. Pure function: takes the
/// data mutably, returns the block-change decision. Callers iterate
/// `world.block_entities` and apply the returned changes via
/// `world.set_block` + the broadcast path.
///
/// Behaviour per Spec 17 Phases 4 + 5:
/// - If the block is `CAMPFIRE` (lit) and `fuel_ticks > 0`: decrement
///   fuel by 1; advance each non-empty slot's cook_progress by 1
///   (capped at COOK_TICKS_PER_ITEM).
/// - If the block is `CAMPFIRE` (lit) and `fuel_ticks == 0`: extinguish
///   to `CAMPFIRE_UNLIT`. The block stays unlit until the player
///   friction-ignites or uses flint+steel.
/// - If the block is `CAMPFIRE_UNLIT`: fuel reserves DO NOT burn down
///   and cooking does NOT progress. **Adding fuel does not auto-ignite**
///   — players must friction-ignite (stick) or use flint+steel to
///   start the fire. (Pre-2026-05-19 the unlit branch auto-lit on
///   fuel-add, which made friction unreachable + sticks were
///   silently consumed as 2-s fuel.)
pub fn tick_one(data: &mut CampfireData, current_block: BlockId) -> CampfireTickOutcome {
    let mut block_change = None;
    // Track smoke before/after to decide On/Off/NoChange. Smoke is
    // visible ONLY when the campfire is actually lit (lit + smoke_ticks > 0).
    let smoke_was_visible = current_block == block::CAMPFIRE && data.smoke_ticks > 0;

    if current_block == block::CAMPFIRE {
        if data.fuel_ticks > 0 {
            data.fuel_ticks -= 1;
            // Advance cooking on every non-empty slot. Cap at the cook
            // limit so a cooked meat doesn't keep accumulating progress.
            for slot in data.slots.iter_mut() {
                if slot.item.is_some() && slot.progress_ticks < COOK_TICKS_PER_ITEM {
                    slot.progress_ticks += 1;
                }
            }
            // Spec 30 — if THIS tick exhausted the fuel, start the
            // smoulder window so the next-tick extinguish keeps a
            // grace period during which fuel-add relights instantly.
            if data.fuel_ticks == 0 {
                data.smoulder_ticks = SMOULDER_TICKS;
            }
        } else {
            // Lit campfire with no fuel left — extinguish.
            block_change = Some(block::CAMPFIRE_UNLIT);
        }
    } else if current_block == block::CAMPFIRE_UNLIT {
        // Spec 30 — decay the smoulder window. After it hits 0 the
        // campfire is fully cold (needs friction or flint-and-steel).
        if data.smoulder_ticks > 0 {
            data.smoulder_ticks -= 1;
        }
    }

    // Smoke decays regardless of fuel state — burning leaves leaves
    // (sic) a transient signal that doesn't depend on continued burning.
    if data.smoke_ticks > 0 {
        data.smoke_ticks -= 1;
    }

    // Compute the new visibility from POST-tick state. The block we'll
    // be at after this tick is `block_change.unwrap_or(current_block)`.
    let post_block = block_change.unwrap_or(current_block);
    let smoke_will_be_visible = post_block == block::CAMPFIRE && data.smoke_ticks > 0;
    let smoke_state = match (smoke_was_visible, smoke_will_be_visible) {
        (false, true) => SmokeState::On,
        (true, false) => SmokeState::Off,
        _ => SmokeState::NoChange,
    };

    CampfireTickOutcome { block_change, smoke_state }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuel_value_for_known_items() {
        assert_eq!(fuel_value(None, Some(block::OAK_LEAVES)), Some(20));
        assert_eq!(fuel_value(Some(MaterialId::Stick), None), Some(40));
        assert_eq!(fuel_value(None, Some(block::OAK_PLANKS)), Some(240));
        assert_eq!(fuel_value(None, Some(block::OAK_LOG)), Some(1200));
        assert_eq!(fuel_value(Some(MaterialId::Coal), None), Some(4800));
    }

    #[test]
    fn fuel_value_for_three_log_materials() {
        // Wave 29 — log seasoning ladder. Green burns half a seasoned;
        // seasoned matches the legacy block-item value (60 s); kiln-dried
        // burns 2× seasoned.
        assert_eq!(fuel_value(Some(MaterialId::GreenLog), None), Some(30 * 20));
        assert_eq!(fuel_value(Some(MaterialId::SeasonedLog), None), Some(60 * 20));
        assert_eq!(fuel_value(Some(MaterialId::KilnDriedLog), None), Some(120 * 20));
        // Back-compat: old saves' OAK_LOG block-items still burn the same.
        assert_eq!(fuel_value(None, Some(block::OAK_LOG)), Some(60 * 20));
    }

    #[test]
    fn is_smoky_fuel_green_log_and_leaves_only() {
        // Smoke pillar (Wave 28) fires for leaves + green logs only.
        // Seasoned + kiln-dried + sticks + planks + coal all burn clean.
        assert!(is_smoky_fuel(Some(MaterialId::GreenLog), None));
        assert!(is_smoky_fuel(None, Some(block::OAK_LEAVES)));
        assert!(!is_smoky_fuel(Some(MaterialId::SeasonedLog), None));
        assert!(!is_smoky_fuel(Some(MaterialId::KilnDriedLog), None));
        assert!(!is_smoky_fuel(Some(MaterialId::Stick), None));
        assert!(!is_smoky_fuel(Some(MaterialId::Coal), None));
        assert!(!is_smoky_fuel(None, Some(block::OAK_LOG)));
        assert!(!is_smoky_fuel(None, Some(block::OAK_PLANKS)));
    }

    #[test]
    fn smoke_ticks_per_green_log_constant_outlives_burn() {
        // Green logs burn 30 s = 600 ticks; the constant is 1200 (60 s)
        // so smoke outlives the flame — matches the leaves precedent.
        assert!(SMOKE_TICKS_PER_GREEN_LOG > fuel_value(Some(MaterialId::GreenLog), None).unwrap());
    }

    #[test]
    fn fuel_value_for_meat_returns_none() {
        // Can't burn cooked steak as fuel — no infinite loops in the
        // cooking economy.
        assert_eq!(fuel_value(Some(MaterialId::CookedBeef), None), None);
        assert_eq!(fuel_value(Some(MaterialId::RawChicken), None), None);
        assert_eq!(fuel_value(Some(MaterialId::Wheat), None), None);
        assert_eq!(fuel_value(Some(MaterialId::Bread), None), None);
    }

    #[test]
    fn fuel_value_for_unknown_returns_none() {
        assert_eq!(fuel_value(None, Some(block::STONE)), None);
        // Block branch wins when block is Some; a non-fuel block → None
        // even if a fuel material is also passed.
        assert_eq!(fuel_value(Some(MaterialId::Stick), Some(block::STONE)), None);
        assert_eq!(fuel_value(None, None), None);
    }

    #[test]
    fn cooked_variant_maps_each_raw_meat() {
        assert_eq!(cooked_variant(MaterialId::RawBeef), Some(MaterialId::CookedBeef));
        assert_eq!(cooked_variant(MaterialId::RawPorkchop), Some(MaterialId::CookedPorkchop));
        assert_eq!(cooked_variant(MaterialId::RawChicken), Some(MaterialId::CookedChicken));
        assert_eq!(cooked_variant(MaterialId::RawMutton), Some(MaterialId::CookedMutton));
    }

    #[test]
    fn cooked_variant_returns_none_for_non_meat() {
        assert_eq!(cooked_variant(MaterialId::Wheat), None);
        assert_eq!(cooked_variant(MaterialId::Coal), None);
        assert_eq!(cooked_variant(MaterialId::CookedBeef), None);
    }

    #[test]
    fn is_lit_only_when_fuel_remains() {
        let mut c = CampfireData::default();
        assert!(!c.is_lit());
        c.fuel_ticks = 1;
        assert!(c.is_lit());
        c.fuel_ticks = 0;
        assert!(!c.is_lit());
    }

    #[test]
    fn can_ignite_requires_actual_fuel() {
        // Guided arc: a campfire only lights if there's fuel to burn —
        // lighting an empty one just snuffs out the next tick. Ignition
        // handlers gate on this and tell the player to add fuel first.
        assert!(!can_ignite(None), "no block-entity yet (fresh placement) → cannot ignite");
        let mut c = CampfireData::default();
        assert!(!can_ignite(Some(&c)), "cold (no fuel) → cannot ignite");
        c.smoulder_ticks = 100; // smouldering, but smoulder is fuel-less
        assert!(!can_ignite(Some(&c)), "smouldering w/o fuel → cannot ignite (would snuff)");
        c.smoulder_ticks = 0;
        c.fuel_ticks = 1;
        assert!(can_ignite(Some(&c)), "has fuel → can ignite");
    }

    #[test]
    fn try_place_raw_fills_slots_in_order() {
        let mut c = CampfireData::default();
        assert!(c.try_place_raw(MaterialId::RawBeef));
        assert!(c.try_place_raw(MaterialId::RawChicken));
        assert!(c.try_place_raw(MaterialId::RawMutton));
        assert!(c.try_place_raw(MaterialId::RawPorkchop));
        assert!(!c.try_place_raw(MaterialId::RawBeef), "5th place must fail — slots full");
        assert_eq!(c.slots[0].item, Some(MaterialId::RawBeef));
        assert_eq!(c.slots[3].item, Some(MaterialId::RawPorkchop));
    }

    #[test]
    fn first_cooked_slot_finds_mature_meat() {
        let mut c = CampfireData::default();
        c.slots[1] = CookSlot {
            item: Some(MaterialId::RawBeef),
            progress_ticks: COOK_TICKS_PER_ITEM,
        };
        // slot 0 is empty, slot 1 is cooked
        let (idx, cooked) = c.first_cooked_slot().expect("should find cooked");
        assert_eq!(idx, 1);
        assert_eq!(cooked, MaterialId::CookedBeef);
    }

    #[test]
    fn first_cooked_slot_ignores_in_progress() {
        let mut c = CampfireData::default();
        c.slots[0] = CookSlot {
            item: Some(MaterialId::RawBeef),
            progress_ticks: COOK_TICKS_PER_ITEM - 1, // one tick short
        };
        assert!(c.first_cooked_slot().is_none());
    }

    #[test]
    fn add_fuel_saturates_at_max() {
        let mut c = CampfireData::default();
        c.fuel_ticks = u32::MAX - 5;
        c.add_fuel(100);
        assert_eq!(c.fuel_ticks, u32::MAX);
    }

    // --- Spec 30 — smoulder state ---

    #[test]
    fn fuel_exhaustion_starts_smoulder_window() {
        // A lit campfire with the last tick of fuel kicks the smoulder
        // window when the tick consumes that final unit.
        let mut c = CampfireData::default();
        c.fuel_ticks = 1;
        let out = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.fuel_ticks, 0);
        assert_eq!(c.smoulder_ticks, SMOULDER_TICKS);
        assert!(c.is_smouldering());
        // Block didn't flip yet — next tick extinguishes.
        assert_eq!(out.block_change, None);
    }

    #[test]
    fn smoulder_decays_per_tick_when_unlit() {
        let mut c = CampfireData::default();
        c.smoulder_ticks = 10;
        tick_one(&mut c, block::CAMPFIRE_UNLIT);
        assert_eq!(c.smoulder_ticks, 9);
        assert!(c.is_smouldering());
    }

    #[test]
    fn smoulder_expires_to_cold_after_window() {
        let mut c = CampfireData::default();
        c.smoulder_ticks = 3;
        for _ in 0..3 {
            tick_one(&mut c, block::CAMPFIRE_UNLIT);
        }
        assert_eq!(c.smoulder_ticks, 0);
        assert!(c.is_cold());
        assert!(!c.is_smouldering());
    }

    #[test]
    fn is_cold_iff_no_fuel_no_smoulder() {
        let mut c = CampfireData::default();
        // Fresh default — no fuel, no smoulder → cold.
        assert!(c.is_cold());
        // Smouldering → not cold.
        c.smoulder_ticks = 100;
        assert!(!c.is_cold());
        // Lit → not cold.
        c.smoulder_ticks = 0;
        c.fuel_ticks = 50;
        assert!(!c.is_cold());
    }

    // --- tick_one ---

    #[test]
    fn tick_decrements_fuel_when_lit() {
        let mut c = CampfireData::default();
        c.fuel_ticks = 10;
        let out = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.fuel_ticks, 9);
        assert!(out.block_change.is_none(), "no block change while lit + fueled");
    }

    #[test]
    fn tick_advances_cooking_when_lit() {
        let mut c = CampfireData::default();
        c.fuel_ticks = 100;
        c.slots[0] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 0 };
        tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.slots[0].progress_ticks, 1);
    }

    #[test]
    fn tick_caps_cooking_at_cook_limit() {
        // Once a slot is mature, additional ticks must NOT push its
        // progress past COOK_TICKS_PER_ITEM (a sentinel value that
        // first_cooked_slot keys off).
        let mut c = CampfireData::default();
        c.fuel_ticks = 100;
        c.slots[0] = CookSlot {
            item: Some(MaterialId::RawChicken),
            progress_ticks: COOK_TICKS_PER_ITEM,
        };
        tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.slots[0].progress_ticks, COOK_TICKS_PER_ITEM);
    }

    #[test]
    fn tick_does_not_advance_cooking_when_unfueled() {
        // Block-state stays CAMPFIRE_UNLIT; slots don't progress.
        let mut c = CampfireData::default();
        c.slots[0] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 50 };
        let out = tick_one(&mut c, block::CAMPFIRE_UNLIT);
        assert_eq!(c.slots[0].progress_ticks, 50);
        assert!(out.block_change.is_none());
    }

    #[test]
    fn tick_does_not_auto_ignite_unlit_with_fuel_reserves() {
        // 2026-05-19 fix: fuel-add no longer auto-ignites. An unlit
        // campfire with reserves sits dormant until friction or
        // flint+steel lights it. Fuel reserves do NOT decay while unlit.
        let mut c = CampfireData::default();
        c.fuel_ticks = 1200;
        let out = tick_one(&mut c, block::CAMPFIRE_UNLIT);
        assert!(out.block_change.is_none(), "unlit campfire must stay unlit on tick");
        assert_eq!(c.fuel_ticks, 1200, "fuel reserves must not burn down while unlit");
    }

    #[test]
    fn tick_transitions_lit_to_unlit_when_fuel_runs_out() {
        let mut c = CampfireData::default();
        c.fuel_ticks = 0;
        let out = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(out.block_change, Some(block::CAMPFIRE_UNLIT));
    }

    #[test]
    fn tick_extinguishes_on_last_tick_of_fuel() {
        // Fuel ticks down 1 → 0. The NEXT tick is what fires the
        // extinguish transition (because the current tick still has
        // fuel > 0 to spend).
        let mut c = CampfireData::default();
        c.fuel_ticks = 1;
        let out1 = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.fuel_ticks, 0);
        assert!(out1.block_change.is_none(), "still lit on this tick");
        let out2 = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(out2.block_change, Some(block::CAMPFIRE_UNLIT));
    }

    // --- Spec 18 — Wave 28 extensions ---

    #[test]
    fn cooked_variant_covers_baked_vegetables() {
        // Wave 28 — Potato / Carrot / Corn now bake on the campfire.
        assert_eq!(cooked_variant(MaterialId::Potato), Some(MaterialId::BakedPotato));
        assert_eq!(cooked_variant(MaterialId::Carrot), Some(MaterialId::BakedCarrot));
        assert_eq!(cooked_variant(MaterialId::Corn), Some(MaterialId::BakedCorn));
        // is_raw_cookable picks them up via cooked_variant.
        assert!(is_raw_cookable(MaterialId::Potato));
        assert!(is_raw_cookable(MaterialId::Carrot));
        assert!(is_raw_cookable(MaterialId::Corn));
    }

    #[test]
    fn tick_smoke_state_off_when_smoke_decays_to_zero_while_lit() {
        // A lit campfire with smoke_ticks==1 ticks down to 0; the
        // post-tick visibility flips from true to false → SmokeState::Off.
        // (This used to be covered by `tick_smoke_state_off_when_smoke_runs_out`
        // below; this test pins the same behaviour at the 1-tick boundary.)
        let mut c = CampfireData::default();
        c.fuel_ticks = 100;
        c.smoke_ticks = 1;
        let out = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.smoke_ticks, 0);
        assert_eq!(out.smoke_state, SmokeState::Off);
    }

    #[test]
    fn tick_smoke_state_off_when_extinguishing() {
        let mut c = CampfireData::default();
        c.fuel_ticks = 0;
        c.smoke_ticks = 30;
        // Currently CAMPFIRE + smoke_ticks > 0 = was-visible. Post-tick
        // we're CAMPFIRE_UNLIT (transition) so will-not-be-visible.
        let out = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(out.block_change, Some(block::CAMPFIRE_UNLIT));
        assert_eq!(out.smoke_state, SmokeState::Off);
    }

    #[test]
    fn tick_smoke_state_off_when_smoke_runs_out() {
        let mut c = CampfireData::default();
        c.fuel_ticks = 100;
        c.smoke_ticks = 1;
        // Was visible (lit + smoke > 0). Post-tick smoke drops to 0 →
        // will-not-be-visible even though still lit.
        let out = tick_one(&mut c, block::CAMPFIRE);
        assert_eq!(c.smoke_ticks, 0);
        assert_eq!(out.smoke_state, SmokeState::Off);
    }

    #[test]
    fn heat_radius_scales_with_fuel() {
        // Spec curve: more fuel → larger heat radius, clamped to [2, 6].
        assert_eq!(heat_radius_blocks(0), 0.0);
        // Tiny fuel still gets the minimum.
        assert_eq!(heat_radius_blocks(20), 2.0);
        // 1 plank (240 ticks) → log2(240) ≈ 7.9 → 7.9 - 5 ≈ 2.9
        let r_plank = heat_radius_blocks(240);
        assert!(r_plank > 2.0 && r_plank < 3.5, "got {r_plank}");
        // 1 log (1200 ticks) → log2(1200) ≈ 10.2 → ≈ 5.2 blocks
        let r_log = heat_radius_blocks(1200);
        assert!(r_log > 4.5 && r_log < 6.0, "got {r_log}");
        // 1 coal (4800 ticks) → log2(4800) ≈ 12.2 → 7.2 → clamped to 6
        assert_eq!(heat_radius_blocks(4800), 6.0);
        assert_eq!(heat_radius_blocks(u32::MAX), 6.0);
    }

    #[test]
    fn nearest_lit_campfire_finds_in_range() {
        use crate::world::World;
        let mut world = World::new();
        world.set_block(5, 70, 5, block::CAMPFIRE);
        let mut cf = CampfireData::default();
        cf.fuel_ticks = 1200;
        world.insert_campfire((5, 70, 5), cf);
        let mob_pos = glam::Vec3::new(4.0, 70.0, 5.0);
        let result = nearest_lit_campfire(&world, mob_pos, 12.0);
        assert!(result.is_some());
        let ((x, _y, z), heat) = result.unwrap();
        assert_eq!((x, z), (5, 5));
        assert!(heat > 4.5);
    }

    #[test]
    fn nearest_lit_campfire_skips_unlit() {
        use crate::world::World;
        let mut world = World::new();
        world.set_block(5, 70, 5, block::CAMPFIRE_UNLIT);
        let cf = CampfireData::default(); // fuel_ticks = 0
        world.insert_campfire((5, 70, 5), cf);
        let mob_pos = glam::Vec3::new(4.0, 70.0, 5.0);
        assert!(nearest_lit_campfire(&world, mob_pos, 12.0).is_none());
    }

    #[test]
    fn nearest_lit_campfire_skips_orphan_blocks() {
        // Defence-in-depth: if a block_entities entry exists but the
        // actual block is no longer a campfire (cleanup miss elsewhere),
        // mobs shouldn't be attracted to ghost campfires.
        use crate::world::World;
        let mut world = World::new();
        world.set_block(5, 70, 5, block::AIR); // not a campfire
        let mut cf = CampfireData::default();
        cf.fuel_ticks = 1200;
        world.insert_campfire((5, 70, 5), cf);
        let mob_pos = glam::Vec3::new(4.0, 70.0, 5.0);
        assert!(nearest_lit_campfire(&world, mob_pos, 12.0).is_none());
    }

    #[test]
    fn cleanup_campfire_spills_cook_slot_contents_and_is_idempotent() {
        // Mirrors furnace.rs's `cleanup_furnace_empties_returns_contents_and_is_idempotent`
        // (its exact structural sibling) — breaking a lit campfire used to
        // silently destroy whatever was cooking (wiki audit 2026-07-09,
        // finding #2: same bug class as the furnace ghost-iron fix, never
        // applied here). A slot mid-cook spills its raw item back; a slot
        // that's finished cooking spills the cooked item, matching what
        // the right-click pickup path would have handed the player.
        use crate::item::ItemStack;
        use crate::world::World;
        let mut world = World::new();
        let mut cf = CampfireData::default();
        cf.slots[0] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 50 }; // still cooking
        cf.slots[1] = CookSlot { item: Some(MaterialId::Potato), progress_ticks: COOK_TICKS_PER_ITEM }; // done
        world.insert_campfire((10, 70, 10), cf);

        let (spill, _cleared_smoke) = cleanup_campfire(&mut world, 10, 70, 10);
        assert_eq!(spill.len(), 2, "two occupied slots -> two spill stacks");
        assert!(spill.contains(&ItemStack::new_material(MaterialId::RawBeef, 1)), "in-progress slot spills its raw item");
        assert!(spill.contains(&ItemStack::new_material(MaterialId::BakedPotato, 1)), "finished slot spills the cooked item");
        assert!(world.campfire_at((10, 70, 10)).is_none(), "block-entity removed");

        let (spill2, _) = cleanup_campfire(&mut world, 10, 70, 10);
        assert!(spill2.is_empty(), "second cleanup is a no-op");
    }

    /// FU4b (FU3 verify L5) — a joiner whose edits reach the server breaks a
    /// fire with the keep-smoke variant: the same entry removal and spill, the
    /// pillar cells left for the server's own clear to arrive.
    #[test]
    fn cleanup_campfire_keep_smoke_spills_like_the_full_cleanup_but_leaves_the_pillar() {
        use crate::item::ItemStack;
        use crate::world::World;
        let build = || {
            let mut world = World::new();
            let mut cf = CampfireData::default();
            cf.slots[0] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 50 };
            world.insert_campfire((10, 70, 10), cf);
            world.set_block(10, 70, 10, block::CAMPFIRE);
            for dy in 1..=3 {
                world.set_block(10, 70 + dy, 10, block::CAMPFIRE_SMOKE);
            }
            world
        };
        let mut world = build();
        let spill = cleanup_campfire_keep_smoke(&mut world, 10, 70, 10);
        assert_eq!(spill, vec![ItemStack::new_material(MaterialId::RawBeef, 1)]);
        assert!(world.campfire_at((10, 70, 10)).is_none(), "entry removed");
        for dy in 1..=3 {
            assert_eq!(world.get_block(10, 70 + dy, 10), block::CAMPFIRE_SMOKE, "the pillar is the server's to clear");
        }
        let mut world = build();
        let (full_spill, cleared) = cleanup_campfire(&mut world, 10, 70, 10);
        assert_eq!(full_spill.len(), 1);
        assert_eq!(cleared.len(), 3, "the full cleanup still clears the pillar");
        assert_eq!(world.get_block(10, 71, 10), block::AIR);
    }

    /// FU3 — the rule the server runs on a joiner's campfire edit: lighting a
    /// smoky fire raises its pillar (a clean one raises none), putting it out
    /// clears it, and breaking it clears the pillar and spills what cooked.
    #[test]
    fn on_block_edit_raises_clears_and_cleans_up_the_smoke_pillar() {
        use crate::world::World;
        let at = (10, 70, 10);
        let mut world = World::new();
        let mut cf = CampfireData { fuel_ticks: 1_000, smoke_ticks: 600, ..Default::default() };
        cf.slots[0] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 0 };
        world.insert_campfire(at, cf);
        world.set_block(at.0, at.1, at.2, block::CAMPFIRE);
        let pillar: Vec<(i32, i32, i32)> = (1..=SMOKE_PILLAR_HEIGHT).map(|dy| (at.0, at.1 + dy, at.2)).collect();
        let smoke_cells = |w: &World| pillar.iter().filter(|c| w.get_block(c.0, c.1, c.2) == block::CAMPFIRE_SMOKE).count();

        let lit = on_block_edit(&mut world, at, block::CAMPFIRE_UNLIT, block::CAMPFIRE);
        assert_eq!(lit.smoke, pillar.iter().map(|&c| (c, block::CAMPFIRE_SMOKE)).collect::<Vec<_>>());
        assert_eq!(smoke_cells(&world), 6, "a smoky fire lit: the pillar rises");

        let out = on_block_edit(&mut world, at, block::CAMPFIRE, block::CAMPFIRE_UNLIT);
        assert_eq!(out.smoke.len(), 6);
        assert_eq!(smoke_cells(&world), 0, "put out: the pillar clears");

        let _ = on_block_edit(&mut world, at, block::CAMPFIRE_UNLIT, block::CAMPFIRE);
        let broken = on_block_edit(&mut world, at, block::CAMPFIRE, block::AIR);
        assert_eq!(broken.smoke.iter().filter(|(_, b)| *b == block::AIR).count(), 6);
        assert_eq!(smoke_cells(&world), 0, "broken: no smoke left");
        assert_eq!(broken.spill.len(), 1, "and what was cooking spills");
        assert!(world.campfire_at(at).is_none(), "its block entity is gone");

        world.insert_campfire(at, CampfireData { fuel_ticks: 1_000, ..Default::default() });
        assert_eq!(on_block_edit(&mut world, at, block::CAMPFIRE_UNLIT, block::CAMPFIRE), CampfireEdit::default(), "a clean fire: no smoke");
        assert_eq!(on_block_edit(&mut world, at, block::STONE, block::AIR), CampfireEdit::default(), "not a campfire: nothing");
    }
}
