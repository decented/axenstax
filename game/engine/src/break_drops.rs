//! C1 (2026-10-07) — what a survival block break yields: one set of rules for
//! every breaker (Spec 05 §4 "Drops", Spec 06 §2.2c).
//!
//! Single-player and a host's own players run them in the client's break arm
//! (`game_loop`), which takes the yield into the breaker's inventory
//! ([`take_yield`]). A joiner's break is the SERVER's to yield: it runs the
//! same [`break_yield`] when it accepts the break and delivers the stacks by
//! `InventoryGrant`, so a joined client takes nothing itself (it would double).
//!
//! - A harvested crop (`growth::crop_break`) yields its crop drops and leaves
//!   its replacement in the cell (tilled soil, a papyrus root).
//! - Lava, water, fire, smoke and air yield nothing ([`yields_drops`]), for
//!   every breaker.
//! - Anything else drops only when the tool's tier allows it
//!   (`crafting::can_harvest`): the mine drop
//!   (`BlockRegistry::mine_drop_with_seed`) plus any bonus stack.
//! - A Satori may drop alongside ([`satori_drop`]), rolled on the WORLD's
//!   Proof-of-Play secret ([`PopKeys`]) — never from a player-placed block.

use crate::block::{BlockId, BlockRegistry};
use crate::crafting::Tool;
use crate::item::{ItemStack, MaterialId};
use crate::world::World;

/// The roll seed for a break at `(x, y, z)` on tick `tick`: chance drops
/// (gravel → flint, salt counts) and crop yields are seeded by tick and cell,
/// so a replay of the same break rolls the same drop.
pub fn drop_seed(tick: u64, x: i32, y: i32, z: i32) -> u64 {
    tick ^ (x as u64).wrapping_mul(73856093)
        ^ (y as u64).wrapping_mul(19349663)
        ^ (z as u64).wrapping_mul(83492791)
}

/// The world's Proof-of-Play keys (Spec 06 §1.3): its own random secret
/// (`WorldMeta.pop_secret`), its seed and the epoch. Whoever holds the world
/// rolls with these — the single-player client, a host, or a dedicated server
/// — never a joiner whose edits reach the server, whose client has no
/// business knowing the secret (a web joiner's edits never arrive, so it rolls
/// on its own local world's).
#[derive(Clone, Copy)]
pub struct PopKeys<'a> {
    pub secret: &'a [u8; 32],
    pub world_seed: u64,
    pub epoch: u32,
}

/// Strike-time Satori roll for mining `block_id` at `(x, y, z)` on tick `now`
/// (Spec 06 §2.2c). All of:
///   1. a pure-deepslate-family block (variants share the canonical's mining
///      identity);
///   2. at or below `Y_DP - 21`;
///   3. a diamond-or-better pickaxe;
///   4. vein membership (the dual-hash vein algorithm, on the world's secret);
///   5. the exposure decay of the cell (`World::pop_exposure`: when mining
///      first exposed it; a cell never exposed by mining — a cave wall — is
///      fully decayed and drops nothing), checked against the hash's
///      exposure byte.
///
/// The caller decides the placed-block rule (a player-placed block never
/// rolls — Spec 06 §2.2): [`break_yield`] does. A Satori is a chance drop and
/// carries NO sats value anywhere (`economy::is_chance_drop`).
pub fn satori_drop(
    world: &World,
    block_id: BlockId,
    (x, y, z): (i32, i32, i32),
    tool: Option<&Tool>,
    keys: &PopKeys,
    now: u64,
) -> Option<ItemStack> {
    use crate::crafting::{tier_index, ToolMaterial, ToolType};
    use crate::proof_of_play::{
        block_is_vein_member, exposure_decay_multiplier, passes_exposure_check, proof_hash,
        EXPOSURE_DECAY_DURATION_TICKS,
    };
    if !crate::block::is_pure_deepslate_family(block_id) {
        return None;
    }
    if y > crate::biome::Y_DP - 21 {
        return None;
    }
    // Non-pickaxe tools index as 0, so they fail the diamond+ gate.
    let pickaxe_tier = tool
        .filter(|t| t.tool_type == ToolType::Pickaxe)
        .map(|t| tier_index(t.material))
        .unwrap_or(0);
    if pickaxe_tier < tier_index(ToolMaterial::Diamond) {
        return None;
    }
    if !block_is_vein_member(keys.secret, keys.world_seed, keys.epoch, x, y, z) {
        return None;
    }
    let multiplier = match world.pop_exposure.get(&(x, y, z)) {
        Some(&exposed_at) => {
            let age = now.saturating_sub(exposed_at).min(u32::MAX as u64) as u32;
            exposure_decay_multiplier(age, EXPOSURE_DECAY_DURATION_TICKS)
        }
        None => 0.0,
    };
    if multiplier <= 0.0 {
        return None;
    }
    let hash = proof_hash(keys.secret, keys.world_seed, keys.epoch, x, y, z);
    passes_exposure_check(&hash, multiplier).then(|| ItemStack::new_material(MaterialId::Satori, 1))
}

/// After a break at `(x, y, z)`: start the exposure clock (Spec 06 §2.2c.3) of
/// every face-neighbour that is pure deepslate and not already exposed. An
/// earlier exposure keeps its clock; cave-natural exposures never enter the
/// map. The map lives on the [`World`], so a host's breaks and its joiners'
/// (rolled by the server on the host's lent world) share one clock.
pub fn mark_exposed_neighbours(world: &mut World, x: i32, y: i32, z: i32, now: u64) {
    for (nx, ny, nz) in [
        (x + 1, y, z),
        (x - 1, y, z),
        (x, y + 1, z),
        (x, y - 1, z),
        (x, y, z + 1),
        (x, y, z - 1),
    ] {
        if crate::block::is_pure_deepslate_family(world.get_block(nx, ny, nz)) {
            world.pop_exposure.entry((nx, ny, nz)).or_insert(now);
        }
    }
}

/// What one survival break yields. Computed BEFORE the block leaves the
/// world: the Satori roll reads the broken block and its cell's exposure, a
/// crop reads what it stands on, and the placed flag is the cell's.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakYield {
    /// The block left in the cell: `AIR`, or a harvested crop's replacement.
    pub replacement: BlockId,
    /// Stacks for the breaker, in the order they are taken: a crop's drops,
    /// or the mine drop then its bonus. Never an empty stack.
    pub drops: Vec<ItemStack>,
    /// A Satori, taken after `drops`.
    pub gem: Option<ItemStack>,
    /// `drops` are a crop harvest (the break arm says so when the inventory
    /// can't hold them).
    pub crop: bool,
    /// The tool's tier allowed a drop (`crafting::can_harvest`) — also what
    /// gates Proof-of-Play work (`crafting::break_work`).
    pub harvestable: bool,
}

/// Does breaking `blk` yield anything at all? Not an empty cell, and not a
/// fluid (WATER, LAVA), fire or smoke: a survival break can target LAVA and
/// FIRE (`raycast::is_pickable` skips only AIR, WATER and smoke) and digs them
/// up, but they are not items — no LAVA, WATER or FIRE block comes out. One
/// rule for every breaker (FU2, C1 verify N2): single-player used to get a
/// placeable LAVA block from digging lava while a joiner, whose break the
/// server classifies, got nothing.
pub fn yields_drops(blk: BlockId) -> bool {
    blk != crate::block::AIR
        && !crate::block::is_fluid(blk)
        && !matches!(blk, crate::block::FIRE | crate::block::CAMPFIRE_SMOKE)
}

/// The yield of breaking `blk` at `cell` with `tool` on tick `tick`, read from
/// `world` as it stands before the break. See the module doc for the rules.
pub fn break_yield(
    world: &World,
    registry: &BlockRegistry,
    blk: BlockId,
    cell: (i32, i32, i32),
    tool: Option<&Tool>,
    tick: u64,
    keys: &PopKeys,
) -> BreakYield {
    let (x, y, z) = cell;
    let seed = drop_seed(tick, x, y, z);
    let harvestable = crate::crafting::can_harvest(blk, tool);
    // #16 — a wild flower on grass breaks to AIR; a farmed one on tilled soil
    // resets the soil.
    let on_tilled = world.get_block(x, y - 1, z) == crate::block::TILLED_SOIL;
    let crop = crate::growth::crop_break(blk, seed, on_tilled);
    // Spec 06 §2.2 — a player-placed block yields no hash-driven drop (no
    // place→break farming).
    let gem = if world.is_placed(x, y, z) {
        None
    } else {
        satori_drop(world, blk, cell, tool, keys, tick)
    };
    let is_crop = crop.is_some();
    let (replacement, mut drops) = match crop {
        _ if !yields_drops(blk) => (crate::block::AIR, Vec::new()),
        Some(result) => (result.replacement, result.drops),
        None if harvestable => {
            let mut drops = vec![registry.mine_drop_with_seed(blk, seed)];
            drops.extend(registry.bonus_mine_drop(blk, seed));
            (crate::block::AIR, drops)
        }
        None => (crate::block::AIR, Vec::new()),
    };
    drops.retain(|s| s.count > 0);
    BreakYield { replacement, drops, gem, crop: is_crop, harvestable }
}

/// Whether a joined client's block edits and `mined` tags reach its server:
/// only when joined and not the web build (L-web-edit). `GameState::
/// edits_reach_server` is this on the real values; the break arm skips its own
/// yield only then, because only then does the server yield the break.
pub fn edits_reach_server(joined: bool, web: bool) -> bool {
    joined && !web
}

/// What [`take_yield`] took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Taken {
    /// What didn't fit the inventory, to be spilled on the ground at the
    /// breaker's feet ([`spill_at_feet`]): the drops that overflowed and a
    /// Satori that found no slot.
    pub spilled: Vec<ItemStack>,
    /// Part of a crop harvest overflowed (the break arm's "inventory full"
    /// toast: players repeat-harvest, so it says so).
    pub crop_overflowed: bool,
    /// The Satori's material, when one was found (scenario + celebration);
    /// set whether or not it fit: a Satori that spilled is still found.
    pub gem: Option<MaterialId>,
}

/// The break arm's own grant of `y` to the breaker: the drops, then the
/// Satori, into `inv` — what single-player and a host's own players get. A
/// client whose edits reach the server (`joined`, i.e. [`edits_reach_server`])
/// takes nothing: the server yields the same break and grants it by
/// `InventoryGrant` (C1); taking it here too would double it. A web joiner's
/// edits never arrive, so it passes `false` and keeps its own drops.
/// What doesn't fit comes back in [`Taken::spilled`] for the caller to drop at
/// the breaker's feet ([`spill_at_feet`]) — the rule a joiner's grant has
/// (`remote_entities::apply_inventory_grant`): nothing is lost, and the
/// pickup pass takes it back when space frees up (FU2, 2026-10-07; it used to
/// be lost silently, a crop harvest excepted).
pub fn take_yield(inv: &mut crate::inventory::Inventory, y: &BreakYield, joined: bool) -> Taken {
    let mut taken = Taken::default();
    if joined {
        return taken;
    }
    for stack in &y.drops {
        if let Some(leftover) = inv.add_item(stack.clone()) {
            taken.crop_overflowed |= y.crop;
            taken.spilled.push(leftover);
        }
    }
    if let Some(gem) = &y.gem {
        if let Some(leftover) = inv.add_item(gem.clone()) {
            taken.spilled.push(leftover);
        }
        taken.gem = match gem.item {
            crate::item::Item::Material(m) => Some(m),
            _ => None,
        };
    }
    taken
}

/// Drop `stacks` ([`Taken::spilled`]) on the ground at the breaker's feet
/// (`pos`) as ordinary item entities, which the pickup pass takes back once
/// there is room. `seed` spreads them slightly
/// ([`drop_seed`] of the break).
pub fn spill_at_feet(ecs: &mut hecs::World, pos: glam::Vec3, stacks: &[ItemStack], seed: u64) {
    for (i, stack) in stacks.iter().enumerate() {
        if stack.count > 0 {
            crate::entity::spawn_item(ecs, pos, stack.clone(), (seed as u32).wrapping_add(i as u32 * 31));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::crafting::{ToolMaterial, ToolType};
    use crate::item::Item;

    const SECRET: [u8; 32] = [7u8; 32];

    fn keys(secret: &[u8; 32]) -> PopKeys<'_> {
        PopKeys { secret, world_seed: 12345, epoch: 0 }
    }

    fn pick(material: ToolMaterial) -> Tool {
        Tool::new(ToolType::Pickaxe, material)
    }

    /// A world with `blk` at `cell` (on stone).
    fn world_with(cell: (i32, i32, i32), blk: BlockId) -> World {
        let mut w = World::new();
        w.set_block(cell.0, cell.1 - 1, cell.2, block::STONE);
        w.set_block(cell.0, cell.1, cell.2, blk);
        w
    }

    #[test]
    fn stone_yields_cobblestone_to_a_wooden_pickaxe_and_nothing_to_a_fist() {
        let reg = BlockRegistry::new();
        let w = world_with((0, 40, 0), block::STONE);
        let y = break_yield(&w, &reg, block::STONE, (0, 40, 0), Some(&pick(ToolMaterial::Wood)), 5, &keys(&SECRET));
        assert_eq!(y.drops, vec![ItemStack::new_block(block::COBBLESTONE, 1)]);
        assert_eq!(y.replacement, block::AIR);
        assert!(y.harvestable && !y.crop && y.gem.is_none());
        let bare = break_yield(&w, &reg, block::STONE, (0, 40, 0), None, 5, &keys(&SECRET));
        assert!(bare.drops.is_empty() && !bare.harvestable, "below the tier: the block breaks, nothing drops");
    }

    #[test]
    fn a_mature_crop_on_tilled_soil_yields_its_harvest_and_leaves_the_soil() {
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(3, 40, 3, block::TILLED_SOIL);
        w.set_block(3, 41, 3, block::WHEAT_STAGE_3);
        let y = break_yield(&w, &reg, block::WHEAT_STAGE_3, (3, 41, 3), None, 99, &keys(&SECRET));
        assert!(y.crop);
        assert_eq!(y.replacement, block::TILLED_SOIL);
        let expected = crate::growth::crop_break(block::WHEAT_STAGE_3, drop_seed(99, 3, 41, 3), true).unwrap();
        assert_eq!(y.drops, expected.drops, "the crop rule, seeded by tick and cell");
    }

    #[test]
    fn gravel_rolls_on_the_shared_seed() {
        let reg = BlockRegistry::new();
        let w = world_with((1, 40, 1), block::GRAVEL);
        for tick in 0..50u64 {
            let y = break_yield(&w, &reg, block::GRAVEL, (1, 40, 1), None, tick, &keys(&SECRET));
            assert_eq!(y.drops, vec![reg.mine_drop_with_seed(block::GRAVEL, drop_seed(tick, 1, 40, 1))]);
        }
    }

    /// A vein origin (its own vein member) for `secret`, at Satori depth.
    fn vein_cell(secret: &[u8; 32]) -> (i32, i32, i32) {
        for x in 0..200 {
            for y in 0..=(crate::biome::Y_DP - 21) {
                for z in 0..200 {
                    if crate::proof_of_play::is_vein_origin(secret, 12345, 0, x, y, z) {
                        return (x, y, z);
                    }
                }
            }
        }
        panic!("no vein origin found");
    }

    #[test]
    fn a_freshly_exposed_vein_cell_drops_a_satori_on_the_worlds_secret_only() {
        let reg = BlockRegistry::new();
        let cell = vein_cell(&SECRET);
        let mut w = world_with(cell, block::PURE_DEEPSLATE);
        w.pop_exposure.insert(cell, 1_000);
        let diamond = pick(ToolMaterial::Diamond);
        let y = break_yield(&w, &reg, block::PURE_DEEPSLATE, cell, Some(&diamond), 1_000, &keys(&SECRET));
        assert_eq!(y.gem, Some(ItemStack::new_material(MaterialId::Satori, 1)));
        // Another secret with no vein here rolls nothing: the world's secret
        // decides.
        let other = (8u8..=255)
            .map(|b| [b; 32])
            .find(|s| !crate::proof_of_play::block_is_vein_member(s, 12345, 0, cell.0, cell.1, cell.2))
            .expect("some secret has no vein at the cell");
        let y = break_yield(&w, &reg, block::PURE_DEEPSLATE, cell, Some(&diamond), 1_000, &keys(&other));
        assert_eq!(y.gem, None);
        // An iron pickaxe, a cave wall (never exposed by mining) and a placed
        // block never roll.
        let iron = pick(ToolMaterial::Iron);
        assert_eq!(break_yield(&w, &reg, block::PURE_DEEPSLATE, cell, Some(&iron), 1_000, &keys(&SECRET)).gem, None);
        let cave = world_with(cell, block::PURE_DEEPSLATE);
        assert_eq!(break_yield(&cave, &reg, block::PURE_DEEPSLATE, cell, Some(&diamond), 1_000, &keys(&SECRET)).gem, None);
        w.place_player_block(cell.0, cell.1, cell.2, block::PURE_DEEPSLATE);
        assert_eq!(break_yield(&w, &reg, block::PURE_DEEPSLATE, cell, Some(&diamond), 1_000, &keys(&SECRET)).gem, None);
    }

    #[test]
    fn mining_exposes_pure_deepslate_neighbours_once() {
        let mut w = World::new();
        w.set_block(5, 5, 5, block::PURE_DEEPSLATE);
        w.set_block(5, 6, 6, block::PURE_DEEPSLATE);
        mark_exposed_neighbours(&mut w, 5, 6, 5, 10);
        mark_exposed_neighbours(&mut w, 5, 6, 5, 20);
        assert_eq!(w.pop_exposure.get(&(5, 5, 5)), Some(&10), "an earlier exposure keeps its clock");
        assert_eq!(w.pop_exposure.get(&(5, 6, 6)), Some(&10));
        assert_eq!(w.pop_exposure.len(), 2, "only pure deepslate is tracked");
    }

    #[test]
    fn a_web_joiner_keeps_its_own_drops_a_native_one_does_not() {
        // The break arm passes `edits_reach_server(joined, web)` as
        // `take_yield`'s `joined` argument (the arm itself needs a GPU).
        assert!(edits_reach_server(true, false), "native joiner: the server yields");
        assert!(!edits_reach_server(true, true), "web joiner: its edits never arrive");
        assert!(!edits_reach_server(false, false));
        assert!(!edits_reach_server(false, true));
        let y = BreakYield {
            replacement: block::AIR,
            drops: vec![ItemStack::new_block(block::COBBLESTONE, 1)],
            gem: None,
            crop: false,
            harvestable: true,
        };
        let mut web = crate::inventory::Inventory::new();
        let taken = take_yield(&mut web, &y, edits_reach_server(true, true));
        assert_eq!(taken, Taken::default());
        assert_eq!(web.slot(0).map(|s| s.item.clone()), Some(Item::Block(block::COBBLESTONE)));
        let mut native = crate::inventory::Inventory::new();
        take_yield(&mut native, &y, edits_reach_server(true, false));
        assert!(native.slots_iter().all(|s| s.is_none()));
    }

    #[test]
    fn a_joined_client_takes_nothing_from_its_own_break() {
        let y = BreakYield {
            replacement: block::AIR,
            drops: vec![ItemStack::new_block(block::COBBLESTONE, 1)],
            gem: Some(ItemStack::new_material(MaterialId::Satori, 1)),
            crop: false,
            harvestable: true,
        };
        let mut inv = crate::inventory::Inventory::new();
        assert_eq!(take_yield(&mut inv, &y, true), Taken::default());
        assert!(inv.slots_iter().all(|s| s.is_none()), "the server grants a joiner's yield");
        let taken = take_yield(&mut inv, &y, false);
        assert_eq!(taken.gem, Some(MaterialId::Satori));
        assert_eq!(inv.slot(0).map(|s| s.item.clone()), Some(Item::Block(block::COBBLESTONE)));
        assert_eq!(inv.slot(1).map(|s| s.item.clone()), Some(Item::Material(MaterialId::Satori)));
    }

    /// An inventory with every slot full of a different-enough full stack.
    fn full_inventory() -> crate::inventory::Inventory {
        let mut inv = crate::inventory::Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_block(block::GLASS, 64)));
        }
        inv
    }

    fn ground_items(ecs: &hecs::World) -> Vec<ItemStack> {
        ecs.query::<&crate::entity::ItemEntity>().iter().map(|(_, i)| i.stack.clone()).collect()
    }

    #[test]
    fn a_break_drop_that_does_not_fit_spills_at_the_feet_instead_of_being_lost() {
        // FU2 — single-player overflow spill (Spec 05 §2.5 "Drops").
        let y = BreakYield {
            replacement: block::AIR,
            drops: vec![ItemStack::new_block(block::COBBLESTONE, 3)],
            gem: Some(ItemStack::new_material(MaterialId::Satori, 1)),
            crop: false,
            harvestable: true,
        };
        let mut inv = full_inventory();
        let taken = take_yield(&mut inv, &y, false);
        assert_eq!(
            taken.spilled,
            vec![ItemStack::new_block(block::COBBLESTONE, 3), ItemStack::new_material(MaterialId::Satori, 1)],
            "the whole drop and the Satori, in order"
        );
        assert_eq!(taken.gem, Some(MaterialId::Satori), "a Satori that spilled is still found");
        assert!(!taken.crop_overflowed, "not a crop");
        let mut ecs = hecs::World::new();
        spill_at_feet(&mut ecs, glam::Vec3::new(4.5, 70.0, 9.5), &taken.spilled, drop_seed(5, 1, 2, 3));
        let on_ground = ground_items(&ecs);
        assert_eq!(on_ground.len(), 2);
        assert!(on_ground.contains(&ItemStack::new_block(block::COBBLESTONE, 3)));
        assert!(on_ground.contains(&ItemStack::new_material(MaterialId::Satori, 1)));
        for (pos, _) in ecs.query::<(&crate::entity::Position, &crate::entity::ItemEntity)>().iter().map(|(_, c)| c) {
            assert!((pos.0.x - 4.5).abs() < 0.5 && (pos.0.z - 9.5).abs() < 0.5, "at the player's feet");
        }
    }

    #[test]
    fn only_the_part_that_does_not_fit_spills_and_a_crop_says_so() {
        let mut inv = full_inventory();
        // One slot with room for 2 more cobblestone.
        inv.set_slot(7, Some(ItemStack::new_block(block::COBBLESTONE, 62)));
        let y = BreakYield {
            replacement: block::AIR,
            drops: vec![ItemStack::new_block(block::COBBLESTONE, 5)],
            gem: None,
            crop: true,
            harvestable: true,
        };
        let taken = take_yield(&mut inv, &y, false);
        assert_eq!(inv.slot(7).map(|s| s.count), Some(64), "topped up first");
        assert_eq!(taken.spilled, vec![ItemStack::new_block(block::COBBLESTONE, 3)], "only the remainder");
        assert!(taken.crop_overflowed, "a crop harvest says the inventory was full");
        // Nothing to spill when it all fits.
        let mut roomy = crate::inventory::Inventory::new();
        assert!(take_yield(&mut roomy, &y, false).spilled.is_empty());
        // And a joined client's break spills nothing: the server's grant spills at its end.
        assert_eq!(take_yield(&mut full_inventory(), &y, true), Taken::default());
    }

    #[test]
    fn lava_fire_water_and_air_yield_nothing_to_any_breaker() {
        // C1 verify N2: single-player used to get a placeable LAVA block.
        let biome = crate::block::BlockRegistry::new();
        for blk in [block::LAVA, block::FIRE, block::WATER, block::AIR, block::CAMPFIRE_SMOKE] {
            let w = world_with((5, 6, 5), blk);
            for tool in [None, Some(pick(ToolMaterial::Diamond))] {
                let y = break_yield(&w, &biome, blk, (5, 6, 5), tool.as_ref(), 10, &keys(&SECRET));
                assert!(y.drops.is_empty() && y.gem.is_none(), "{blk} yields nothing");
                assert_eq!(y.replacement, block::AIR);
            }
            assert!(!yields_drops(blk));
        }
        // Real blocks still do.
        let w = world_with((5, 6, 5), block::STONE);
        let y = break_yield(&w, &biome, block::STONE, (5, 6, 5), Some(&pick(ToolMaterial::Wood)), 10, &keys(&SECRET));
        assert!(!y.drops.is_empty() && yields_drops(block::STONE));
    }
}
