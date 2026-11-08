//! Building a Challenge's tailored arena into a world (`ScenarioDef::arena`):
//! the box fills, the single blocks and the mobs, with no renderer, so the
//! same code runs in the game (`GameState::apply_arena_setup`, which then
//! re-meshes what changed) and in tests.
//!
//! Order: `fills` first, then `blocks` (so single blocks sit on top of a
//! fill), then `mobs`. Offsets are from `origin`, the spawn cell.

use crate::scenario::ArenaSetup;
use crate::water::WaterSystem;
use crate::world::World;

/// The biggest box one fill may cover. A bundled fill is a few thousand cells;
/// this stops a hostile community def from stalling the first tick.
pub const MAX_FILL_CELLS: u64 = 100_000;

/// Resolve a `/give`-style block name to a placeable block.
fn arena_block(name: &str) -> Option<crate::block::BlockId> {
    let Ok(stack) = crate::commands::builtins::give::resolve_item(name, 1) else {
        log::warn!("arena: unknown block '{name}'");
        return None;
    };
    let id = stack.item.as_block();
    if id.is_none() {
        log::warn!("arena: '{name}' is not a placeable block");
    }
    id
}

/// Write one arena cell, with the side tables some blocks need.
fn put_cell(
    world: &mut World,
    water: &mut WaterSystem,
    lava: &mut crate::lava::LavaSystem,
    (x, y, z): (i32, i32, i32),
    block_id: crate::block::BlockId,
) {
    world.set_block(x, y, z, block_id);
    // A placed liquid must be a SOURCE so it can fill a bucket / host a
    // fishing spot — mirror the bucket-empty path's source registration.
    if block_id == crate::block::WATER {
        water.add_source(x, y, z);
    } else if block_id == crate::block::LAVA {
        lava.add_source(x, y, z);
    } else if block_id == crate::block::VENDOR_BLOCK {
        // An arena vendor belongs to a friendly NPC trader (a FOREIGN owner), so
        // the solo player is a BUYER — the owner view has no buy button, so a
        // player-owned vendor could never fire the sale. Pre-stock a simple
        // BARTER (bring 1 stone → get 1 bread); barter sidesteps the sats/Charter
        // gate, so the "Sell at a Vendor" Trial completes solo when the player
        // trades.
        world.insert_vendor(
            (x, y, z),
            crate::vendor::VendorData {
                owner: Some(crate::vendor::VendorOwner::Npub("arena-trader".to_string())),
                mode: Some(crate::vendor::VendorMode::Barter),
                slot: Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 1)),
                barter_request: Some(crate::item::ItemStack::new_block(crate::block::STONE, 1)),
                price_sats: 0,
                stock: 9,
                escrow_sats: 0,
                last_txn_tick: 0,
                lot_size: 1,
            },
        );
    }
}

/// Lay only `arena`'s box fills round `origin` (the terrain: terraces, soil
/// rows), not its single blocks or mobs. Returns one cell per chunk touched.
/// This is what a Place trial's "Try again" re-lays (`place_trial::rewind_arena`):
/// the blocks hold chests and a spring, the mobs would duplicate the animals.
pub(crate) fn build_fills(
    world: &mut World,
    water: &mut WaterSystem,
    lava: &mut crate::lava::LavaSystem,
    origin: (i32, i32, i32),
    arena: &ArenaSetup,
) -> Vec<(i32, i32, i32)> {
    let mut touched = std::collections::HashMap::new();
    lay_fills(world, water, lava, origin, arena, &mut touched);
    touched.into_values().collect()
}

type Touched = std::collections::HashMap<(i32, i32, i32), (i32, i32, i32)>;

fn lay_fills(
    world: &mut World,
    water: &mut WaterSystem,
    lava: &mut crate::lava::LavaSystem,
    (ox, oy, oz): (i32, i32, i32),
    arena: &ArenaSetup,
    touched: &mut Touched,
) {
    let mut note = |cell: (i32, i32, i32)| {
        touched.entry(World::block_to_chunk(cell.0, cell.1, cell.2)).or_insert(cell);
    };
    for f in &arena.fills {
        let Some(block_id) = arena_block(&f.block) else { continue };
        let lo = [f.from[0].min(f.to[0]), f.from[1].min(f.to[1]), f.from[2].min(f.to[2])];
        let hi = [f.from[0].max(f.to[0]), f.from[1].max(f.to[1]), f.from[2].max(f.to[2])];
        let cells: u64 = (0..3).map(|i| (hi[i] - lo[i]) as u64 + 1).product();
        if cells > MAX_FILL_CELLS {
            log::warn!("arena: fill of {cells} cells is over the {MAX_FILL_CELLS} cap; skipped");
            continue;
        }
        for dy in lo[1]..=hi[1] {
            for dz in lo[2]..=hi[2] {
                for dx in lo[0]..=hi[0] {
                    let cell = (ox + dx, oy + dy, oz + dz);
                    put_cell(world, water, lava, cell, block_id);
                    note(cell);
                }
            }
        }
    }
}

/// Build `arena` round `origin`. Returns one cell per chunk the blocks
/// touched, for the caller to re-mesh.
pub(crate) fn build_arena(
    world: &mut World,
    water: &mut WaterSystem,
    lava: &mut crate::lava::LavaSystem,
    ecs: &mut hecs::World,
    origin: (i32, i32, i32),
    arena: &ArenaSetup,
) -> Vec<(i32, i32, i32)> {
    let (ox, oy, oz) = origin;
    let mut touched = Touched::new();
    lay_fills(world, water, lava, origin, arena, &mut touched);
    let mut note = |cell: (i32, i32, i32)| {
        touched.entry(World::block_to_chunk(cell.0, cell.1, cell.2)).or_insert(cell);
    };
    for b in &arena.blocks {
        let Some(block_id) = arena_block(&b.block) else { continue };
        let cell = (ox + b.at[0], oy + b.at[1], oz + b.at[2]);
        put_cell(world, water, lava, cell, block_id);
        note(cell);
    }
    for m in &arena.mobs {
        let Some(kind) = crate::mob::MobType::from_name(&m.mob) else {
            log::warn!("arena: unknown mob '{}'", m.mob);
            continue;
        };
        for i in 0..m.count.max(1) {
            // Fan copies out along +X so they don't stack in one cell.
            let pos = glam::Vec3::new(
                (ox + m.at[0]) as f32 + 0.5 + i as f32 * 0.8,
                (oy + m.at[1]) as f32 + 0.5,
                (oz + m.at[2]) as f32 + 0.5,
            );
            let e = crate::entity::spawn_mob(ecs, kind, pos);
            // Authored = a deliberately-placed wild mob; it persists across
            // chunk unload (unlike scattered wildlife) so the arena survives.
            let _ = ecs.insert_one(e, crate::entity::Authored);
        }
    }
    touched.into_values().collect()
}
