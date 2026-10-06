//! Spec 28d chunk 10 — pure-function helper bundle.
//!
//! A handful of cross-system pure helpers that have been duplicated
//! across modules or asked-for by multiple consumers. None of these
//! depend on the ECS or the renderer; all are testable in isolation.
//!
//! - `crop_can_plant_at` — mirrors `sapling::can_plant_sapling_at` for
//!   the farming crop family. Target = AIR; below = TILLED_SOIL.
//! - `achievable_recipes` — given an inventory snapshot, returns which
//!   craftable recipes the player can complete right now.
//! - `crit_multiplier` — combat crit roll given a 0..1 seed.
//! - `knockback_vector` — unit vector from attacker → target,
//!   horizontal only.
//! - `pick_passive_for_biome` — deterministic sample of a passive mob
//!   from `mob::biome_passive_spawn_weights` given a seed.

use crate::block;
#[cfg(test)]
use crate::block::BlockId;
use crate::biome::Biome;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::mob::{self, MobType};
use crate::world::World;

/// Spec 28d chunk 10 — can a crop seed (`Wheat/Carrot/Potato/Corn/
/// SugarBeet/Beetroot`) be planted at this position? Mirrors
/// `sapling::can_plant_sapling_at` for the farming crop family.
///
/// Rules:
/// - Target cell must be AIR.
/// - Block below must be TILLED_SOIL.
///
/// Tier-1.5 specialties (e.g. sugarcane wants water-adjacent sand)
/// have their own predicates and don't route through this helper.
#[cfg_attr(not(test), allow(dead_code))]
pub fn crop_can_plant_at(x: i32, y: i32, z: i32, world: &World) -> bool {
    if world.get_block(x, y, z) != block::AIR {
        return false;
    }
    world.get_block(x, y - 1, z) == block::TILLED_SOIL
}

/// Spec 28d chunk 10 — what recipes can the player complete from
/// their current inventory? Returns a list of (recipe_label, output)
/// tuples. Caller renders this however it likes.
///
/// Doesn't try to enumerate every possible recipe — covers the
/// staples Axolittle will hit first: Bread, Sticks, Planks, basic
/// tool tiers. The crafting registry is already shape-based; the
/// achievable-list is an additive surface.
#[cfg_attr(not(test), allow(dead_code))]
pub fn achievable_recipes(inv: &Inventory) -> Vec<(&'static str, ItemStack)> {
    let mut out = Vec::new();

    let count = |needle: &Item| -> u32 {
        (0..36)
            .filter_map(|i| inv.slot(i))
            .filter(|stack| matches_item(&stack.item, needle))
            .map(|stack| stack.count as u32)
            .sum()
    };

    // Bread — 3 wheat (horizontal).
    if count(&Item::Material(MaterialId::Wheat)) >= 3 {
        out.push((
            "Bread",
            ItemStack::new_material(MaterialId::Bread, 1),
        ));
    }

    // Stick — 2 planks (any species) → 4 sticks.
    let planks_count: u32 = [
        Item::Block(block::OAK_PLANKS),
        Item::Block(block::BIRCH_PLANKS),
        Item::Block(block::SPRUCE_PLANKS),
        Item::Block(block::JUNGLE_PLANKS),
        Item::Block(block::ACACIA_PLANKS),
        Item::Block(block::DARK_OAK_PLANKS),
    ]
    .iter()
    .map(count)
    .sum();
    if planks_count >= 2 {
        out.push((
            "Stick",
            ItemStack::new_material(MaterialId::Stick, 4),
        ));
    }

    // Planks — 1 log → 4 planks. Any species works (recipe is species-
    // neutral).
    let log_count = count(&Item::Material(MaterialId::SeasonedLog));
    if log_count >= 1 {
        out.push((
            "Oak Planks",
            ItemStack::new_block(block::OAK_PLANKS, 4),
        ));
    }

    // Arrow — stick + feather → 4 arrows.
    if count(&Item::Material(MaterialId::Stick)) >= 1
        && count(&Item::Material(MaterialId::Feather)) >= 1
    {
        out.push((
            "Arrow",
            ItemStack::new_material(MaterialId::Arrow, 4),
        ));
    }

    out
}

#[cfg_attr(not(test), allow(dead_code))]
fn matches_item(a: &Item, b: &Item) -> bool {
    use Item::*;
    match (a, b) {
        (Block(x), Block(y)) => x == y,
        (Material(x), Material(y)) => x == y,
        // Tools / Plans / Armour don't participate in this matcher
        // (per-instance items; treated as opaque).
        _ => false,
    }
}

/// Spec 28d chunk 10 — combat crit multiplier. Given a 0..=1 seed
/// (typically the high bits of an HMAC hash on the swing), returns
/// the damage multiplier:
///   0.00..0.10 → 2.0 (crit)
///   0.10..0.95 → 1.0 (normal)
///   0.95..=1.0 → 0.7 (graze)
///
/// The 0.7 graze adds a tiny risk pocket so players can't tell whether
/// a fixed-damage spec means "always X" or "X±a bit"; this is a
/// commonly-requested combat texture.
#[cfg_attr(not(test), allow(dead_code))]
pub fn crit_multiplier(seed_unit: f32) -> f32 {
    let s = seed_unit.clamp(0.0, 1.0);
    if s < 0.10 {
        2.0
    } else if s >= 0.95 {
        0.7
    } else {
        1.0
    }
}

/// Spec 28d chunk 10 — knockback vector. Returns a unit vector
/// pointing from attacker → target in the horizontal plane, scaled by
/// `strength`. Y is zeroed; the caller applies an explicit Y impulse
/// (typically positive so the target lifts).
#[cfg_attr(not(test), allow(dead_code))]
pub fn knockback_vector(
    attacker_pos: (f32, f32, f32),
    target_pos: (f32, f32, f32),
    strength: f32,
) -> (f32, f32, f32) {
    let dx = target_pos.0 - attacker_pos.0;
    let dz = target_pos.2 - attacker_pos.2;
    let dist_sq = dx * dx + dz * dz;
    if dist_sq < 1e-6 {
        return (0.0, 0.0, 0.0);
    }
    let inv = 1.0 / dist_sq.sqrt();
    (dx * inv * strength, 0.0, dz * inv * strength)
}

/// Spec 28d chunk 10 — pick a passive mob species for a biome, given
/// a seed. Walks `mob::biome_passive_spawn_weights` and samples
/// proportionally. Returns `None` if the biome has no passive roster
/// (e.g. pre-chunk-6 Ocean before Squid landed).
/// Map a player index to its skin texture-array layer (capped; players beyond
/// the cap share the last *player* layer rather than reading out of bounds).
/// The very last array layer (`SKIN_LAYERS - 1`) is reserved for the #17 "Your
/// look" 3D preview (`renderer::SKIN_PREVIEW_LAYER`), so players cap one below
/// it and the preview never clobbers a player's skin.
pub(crate) fn skin_layer_of(player_index: usize) -> u32 {
    (player_index as u32).min(crate::renderer::SKIN_LAYERS - 2)
}

/// Is the server's player slot `player_index` a REMOTE peer to the host client
/// — rather than one of its own `local_seats`? Server slots `0..local_seats`
/// are the host's own seats (slot 0 is the one that sends input; the others
/// follow their seats through `HostedServer::sync_local_slots` so joiners see
/// them); joiners come after. The host draws a remote peer from the broadcast
/// roster, but every local seat is drawn by the split-screen loop from its own
/// live body — so a seat that also came back through the roster is drawn twice
/// in each other seat's viewport, one tick apart.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn is_remote_player_index(player_index: u32, local_seats: usize) -> bool {
    player_index as usize >= local_seats
}

/// Should an on-sign-in Stash skin load, resolving late, be applied?
///
/// The load is kicked off at world entry and lands asynchronously. If the
/// player has already chosen a look this session (`user_acted`) — uploaded,
/// picked a preset, or reset — a late load must NOT revert that choice. Only
/// apply the loaded skin when the player hasn't acted yet. Extracted so the
/// guard is pinned headlessly (the drain itself is WASM-only, hence dead on
/// native — but the test below still exercises it on every target).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn should_apply_loaded_skin(user_acted: bool) -> bool {
    !user_acted
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn pick_passive_for_biome(biome: Biome, seed_unit: f32) -> Option<MobType> {
    let weights = mob::biome_passive_spawn_weights(biome);
    if weights.is_empty() {
        return None;
    }
    let total: u32 = weights.iter().map(|(_, w)| *w as u32).sum();
    if total == 0 {
        return None;
    }
    let pick = (seed_unit.clamp(0.0, 1.0) * total as f32) as u32;
    let mut acc = 0u32;
    for (kind, w) in &weights {
        acc += *w as u32;
        if pick < acc {
            return Some(*kind);
        }
    }
    weights.last().map(|(k, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    fn world_with(x: i32, y: i32, z: i32, id: BlockId) -> World {
        let mut w = World::new();
        w.set_block(x, y, z, id);
        w
    }

    // crop_can_plant_at ----------------------------------------------

    #[test]
    fn crop_plant_on_tilled_soil_succeeds() {
        let w = world_with(0, 60, 0, block::TILLED_SOIL);
        assert!(crop_can_plant_at(0, 61, 0, &w));
    }

    #[test]
    fn crop_plant_on_grass_fails() {
        let w = world_with(0, 60, 0, block::GRASS);
        assert!(!crop_can_plant_at(0, 61, 0, &w));
    }

    #[test]
    fn crop_plant_into_non_air_fails() {
        let mut w = World::new();
        w.set_block(0, 60, 0, block::TILLED_SOIL);
        w.set_block(0, 61, 0, block::STONE);
        assert!(!crop_can_plant_at(0, 61, 0, &w));
    }

    // achievable_recipes ---------------------------------------------

    #[test]
    fn empty_inventory_has_no_achievable_recipes() {
        let inv = Inventory::new();
        assert!(achievable_recipes(&inv).is_empty());
    }

    #[test]
    fn three_wheat_unlocks_bread() {
        let mut inv = Inventory::new();
        inv.add_item(ItemStack::new_material(MaterialId::Wheat, 3));
        let recipes = achievable_recipes(&inv);
        assert!(recipes.iter().any(|(label, _)| *label == "Bread"));
    }

    #[test]
    fn two_planks_unlock_sticks() {
        let mut inv = Inventory::new();
        inv.add_item(ItemStack::new_block(block::OAK_PLANKS, 2));
        let recipes = achievable_recipes(&inv);
        assert!(recipes.iter().any(|(label, _)| *label == "Stick"));
    }

    #[test]
    fn tools_in_inventory_dont_break_recipe_walk() {
        // Tools are opaque to matches_item; ensure they don't
        // contribute material counts.
        let mut inv = Inventory::new();
        inv.add_item(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron)));
        let recipes = achievable_recipes(&inv);
        assert!(recipes.is_empty());
    }

    // crit_multiplier ------------------------------------------------

    #[test]
    fn low_seed_yields_crit() {
        assert_eq!(crit_multiplier(0.05), 2.0);
    }

    #[test]
    fn high_seed_yields_graze() {
        assert_eq!(crit_multiplier(0.99), 0.7);
    }

    #[test]
    fn mid_seed_yields_normal() {
        assert_eq!(crit_multiplier(0.5), 1.0);
    }

    #[test]
    fn seed_clamped_to_unit() {
        assert_eq!(crit_multiplier(-1.0), 2.0);
        assert_eq!(crit_multiplier(2.0), 0.7);
    }

    // knockback_vector -----------------------------------------------

    #[test]
    fn knockback_points_away_from_attacker() {
        let v = knockback_vector((0.0, 64.0, 0.0), (3.0, 64.0, 0.0), 1.0);
        assert!(v.0 > 0.99);
        assert_eq!(v.1, 0.0);
        assert!(v.2.abs() < 0.01);
    }

    #[test]
    fn knockback_zero_when_overlapping() {
        let v = knockback_vector((1.0, 64.0, 1.0), (1.0, 64.0, 1.0), 1.0);
        assert_eq!(v, (0.0, 0.0, 0.0));
    }

    #[test]
    fn knockback_scales_with_strength() {
        let v = knockback_vector((0.0, 64.0, 0.0), (1.0, 64.0, 0.0), 5.0);
        assert!((v.0 - 5.0).abs() < 0.01);
    }

    // pick_passive_for_biome -----------------------------------------

    #[test]
    fn pick_passive_in_plains_returns_some() {
        let pick = pick_passive_for_biome(Biome::Plains, 0.5);
        assert!(pick.is_some());
    }

    #[test]
    fn pick_passive_in_jungle_returns_some() {
        let pick = pick_passive_for_biome(Biome::Jungle, 0.0);
        assert!(pick.is_some());
    }

    #[test]
    fn pick_passive_walks_the_distribution() {
        // Across a swept seed, we should see multiple species in
        // Plains (which has Cow / Sheep / Pig / Chicken / Horse / Bee
        // / Rabbit).
        let mut seen: ahash::AHashSet<MobType> = ahash::AHashSet::new();
        for i in 0..100 {
            if let Some(k) = pick_passive_for_biome(Biome::Plains, i as f32 / 100.0) {
                seen.insert(k);
            }
        }
        assert!(seen.len() >= 3, "expected ≥3 species across the seed sweep, got {}", seen.len());
    }

    #[test]
    fn skin_layer_caps_below_reserved_preview_layer() {
        assert_eq!(super::skin_layer_of(0), 0);
        assert_eq!(super::skin_layer_of(3), 3);
        // Players cap at the second-to-last layer; the last is reserved for the
        // #17 avatar preview, so a player's skin can never land on it.
        assert_eq!(super::skin_layer_of(999), crate::renderer::SKIN_LAYERS - 2);
        assert_ne!(super::skin_layer_of(999), crate::renderer::SKIN_LAYERS - 1);
    }

    #[test]
    fn loaded_skin_applies_only_when_user_has_not_acted() {
        // Player already chose a look this session → a late load must NOT revert it.
        assert!(!super::should_apply_loaded_skin(true));
        // Player hasn't touched their look → the restored skin should apply.
        assert!(super::should_apply_loaded_skin(false));
    }

    /// Final review fix 2 — a split-screen host's second seat is a LOCAL seat,
    /// not a remote peer: only indices past the host client's seats are.
    #[test]
    fn local_seats_are_never_remote_players() {
        // One seat: slot 0 is the host; a joiner is slot 1.
        assert!(!super::is_remote_player_index(0, 1));
        assert!(super::is_remote_player_index(1, 1));
        // Two seats: slot 1 is the second seat, the first joiner is slot 2.
        assert!(!super::is_remote_player_index(0, 2));
        assert!(!super::is_remote_player_index(1, 2), "seat 1 is drawn by the split-screen loop");
        assert!(super::is_remote_player_index(2, 2));
        assert!(super::is_remote_player_index(7, 2));
    }
}
