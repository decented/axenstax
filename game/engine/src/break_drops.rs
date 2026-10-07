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
/// — never a joiner, whose client has no business knowing the secret.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Taken {
    /// Part of a crop harvest didn't fit (the break arm's "harvest lost").
    pub crop_lost: bool,
    /// The Satori's material, when one was taken (scenario + celebration).
    pub gem: Option<MaterialId>,
}

/// The break arm's own grant of `y` to the breaker: the drops, then the
/// Satori, into `inv` — what single-player and a host's own players get. A
/// client whose edits reach the server (`joined`, i.e. [`edits_reach_server`])
/// takes nothing: the server yields the same break and grants it by
/// `InventoryGrant` (C1); taking it here too would double it. A web joiner's
/// edits never arrive, so it passes `false` and keeps its own drops.
/// What doesn't fit is lost, as it always was for the break arm (only a crop
/// harvest says so).
pub fn take_yield(inv: &mut crate::inventory::Inventory, y: &BreakYield, joined: bool) -> Taken {
    let mut taken = Taken::default();
    if joined {
        return taken;
    }
    for stack in &y.drops {
        if inv.add_item(stack.clone()).is_some() && y.crop {
            taken.crop_lost = true;
        }
    }
    if let Some(gem) = &y.gem {
        inv.add_item(gem.clone());
        taken.gem = match gem.item {
            crate::item::Item::Material(m) => Some(m),
            _ => None,
        };
    }
    taken
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
}
