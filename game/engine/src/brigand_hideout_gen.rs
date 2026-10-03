//! Historical Pivot Sub-Foundation 3 (HP-3) — Brigand Hideout worldgen.
//!
//! Mirrors `village_gen.rs`'s grid-based deterministic structure
//! placement, sparser (1 per 64×64 chunks). Each hideout is a wooden
//! palisade ring around a central stolen-goods Chest + unlit campfire,
//! gated to temperate biomes and kept ≥ 128 blocks from any village
//! anchor.
//!
//! Layout components (all deterministic per `(world_seed, gx, gz)`):
//!   - Oak-log palisade, radius 6, height 3, with one south-facing gap
//!   - Brigand Hideout Banner at the gate (`BlockId::BRIGAND_HIDEOUT_BANNER`)
//!   - `BlockId::CHEST` at the anchor, pre-seeded with `seed_hideout_loot`
//!   - Cobblestone hearth + unlit campfire two blocks east of the anchor
//!   - 4 corner TORCH blocks on the palisade
//!   - 3-4 small huts inside the palisade (3×3 oak-plank floors + log walls)
//!
//! See spec `docs/foundations/2026-05-23-historical-pivot-brigand-hideouts.md`.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::biome::{Biome, BiomeGenerator};
use crate::block;
use crate::chunk::CHUNK_SIZE;
use crate::item::{ItemStack, MaterialId};
use crate::world::World;

/// Grid cell size in chunks — 1 hideout per 64×64 cell = 1024 blocks.
/// Sparser than villages (`VILLAGE_GRID = 32`) per spec.
pub const HIDEOUT_GRID: i32 = 64;

/// Reach (in chunks) — how far an anchor's footprint extends. Palisade
/// radius is 6 blocks + huts inside = ~12 block reach, easily within
/// one chunk = `VILLAGE_CHUNK_REACH = 2` is more than enough.
pub const HIDEOUT_CHUNK_REACH: i32 = 1;

/// Minimum distance from any village anchor (Manhattan / Chebyshev
/// distance). Hideouts and villages don't share territory.
pub const MIN_DISTANCE_FROM_VILLAGE_BLOCKS: i32 = 128;

/// Palisade radius — distance from anchor to the wall ring.
pub const PALISADE_RADIUS: i32 = 6;

/// Palisade height in blocks above the surface.
pub const PALISADE_HEIGHT: i32 = 3;

/// Population target on a normal hideout (4 brigands + 1 marauder).
pub const POPULATION_TARGET_NORMAL: u32 = 4;

/// Population target on a rare "boss" hideout — adds 1 Berserker.
pub const POPULATION_TARGET_RARE: u32 = 5;

/// Ticks between replenish passes when population is below target.
/// 24 000 ticks = 1 in-game day @ 20 TPS.
pub const REPLENISH_COOLDOWN_TICKS: u64 = 24_000;

/// HP-3 v2 (2026-05-23) — how long a fully-cleared hideout sits dormant
/// before the stockpile chest refills and brigands repopulate from
/// scratch. 48 000 ticks ≈ 2 in-game days at 20 TPS, giving the player
/// a meaningful cadence to come back and clear the same hideout again.
/// Shorter than the spec's prose-only "v2" mention so playtest can
/// actually exercise the loop in one session.
pub const STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS: u64 = 48_000;

/// HP-3 v2 (2026-05-23) — distance from a hideout anchor within which
/// villages count as "neighbouring" for the brigand-bleed feedback
/// loop. 256 blocks is roughly two village-grid cells away — close
/// enough that the player can see the link, far enough that lone
/// hideouts don't tax every village in the world.
pub const BRIGAND_STEAL_RADIUS_BLOCKS: f32 = 256.0;

/// HP-3 v2 — sats taken from a neighbouring village treasury each
/// time a hideout refills its stockpile. 50 sats is a small bite
/// (much less than a single Vendor Block trade) so the bleed reads
/// as ambient cost rather than catastrophic loss. Capped at the
/// village's current treasury so we never overdraw.
pub const BRIGAND_STEAL_AMOUNT_SATS: u64 = 50;

/// Per-hideout state. Layout is regenerated from `(world_seed, gx, gz)`
/// on column streams; what persists across save/load is the spawn-loop
/// state (current population + cooldown).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HideoutData {
    pub anchor_world: [i32; 3],
    pub gx: i32,
    pub gz: i32,
    /// Live brigand population. Incremented at spawn, decremented at
    /// kill-attribution (combat path looks up the mob's `HomeHideout`
    /// component to find the right hideout to decrement).
    pub population: u32,
    /// Target population. 4 on a normal hideout; 5 on a rare one.
    pub population_target: u32,
    /// `true` if this hideout's roster includes a Berserker.
    pub has_berserker: bool,
    /// Last tick the replenish pass topped this hideout up.
    pub last_replenish_tick: u64,
    /// HP-3 v2 (2026-05-23) — tick when the hideout last entered its
    /// "dormant" state (population = 0 AND stockpile chest empty). When
    /// `current_tick - dormant_since >= STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS`
    /// the full reset fires: chest refills, brigands respawn next tick.
    /// `#[serde(default)]` so HP-3 v1 saves load with None.
    #[serde(default)]
    pub dormant_since: Option<u64>,
    /// HP-3 v2 — world seed at the moment this hideout was first
    /// placed. Needed so the stockpile refill on dormant-reset can
    /// re-derive the deterministic loot through `seed_hideout_loot`.
    /// `#[serde(default)]` so HP-3 v1 saves load with 0; refill on
    /// those will use seed 0 (a stable fallback — different from the
    /// original loot but still deterministic).
    #[serde(default)]
    pub world_seed: u32,
}

impl HideoutData {
    /// True when the population is below target and the replenish
    /// cooldown has expired — caller may spawn missing mobs.
    pub fn needs_replenish(&self, current_tick: u64) -> bool {
        self.population < self.population_target
            && current_tick.saturating_sub(self.last_replenish_tick) >= REPLENISH_COOLDOWN_TICKS
    }

    /// HP-3 v2 — true if the stockpile reset cooldown has expired
    /// (player cleared the hideout + looted the chest + waited long
    /// enough). Caller refills the chest and resets population to 0
    /// so the next replenish tick respawns the roster.
    pub fn stockpile_should_reset(&self, current_tick: u64) -> bool {
        match self.dormant_since {
            Some(since) => {
                current_tick.saturating_sub(since) >= STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS
            }
            None => false,
        }
    }
}

/// Deterministic 32-bit hash of `(world_seed, gx, gz)` — different
/// constants from `village_gen::anchor_hash` so adjacent village +
/// hideout cells don't share patterns.
fn hideout_hash(world_seed: u32, gx: i32, gz: i32) -> u32 {
    let mut h = world_seed.wrapping_mul(0x7F4A7C15);
    h ^= (gx as u32).wrapping_mul(0xBB67AE85);
    h = h.rotate_left(11).wrapping_mul(0x3C6EF372);
    h ^= (gz as u32).wrapping_mul(0xA54FF53A);
    h = h.rotate_left(7).wrapping_mul(0x510E527F);
    h ^ (h >> 16)
}

/// Should the grid cell host a hideout? ~60 % of cells do; the rest are
/// empty stretches so a player can walk for a while between encounters.
pub fn cell_has_hideout(world_seed: u32, gx: i32, gz: i32) -> bool {
    hideout_hash(world_seed, gx, gz) % 5 < 3
}

/// Is the rare-boss roll true for this cell? ~20 % of hideouts.
pub fn cell_has_berserker(world_seed: u32, gx: i32, gz: i32) -> bool {
    (hideout_hash(world_seed, gx, gz) >> 8).is_multiple_of(5)
}

/// Anchor world position for this cell.
fn anchor_world_xz(world_seed: u32, gx: i32, gz: i32) -> (i32, i32) {
    let h = hideout_hash(world_seed, gx, gz);
    let cell_origin_x = gx * HIDEOUT_GRID * CHUNK_SIZE as i32;
    let cell_origin_z = gz * HIDEOUT_GRID * CHUNK_SIZE as i32;
    let cell_blocks = HIDEOUT_GRID * CHUNK_SIZE as i32;
    // Keep at least PALISADE_RADIUS + 4 blocks from each cell edge so
    // the structure doesn't clip the grid boundary.
    let margin = PALISADE_RADIUS + 4;
    let span = cell_blocks - 2 * margin;
    let off_x = (h % span as u32) as i32 + margin;
    let off_z = ((h >> 12) % span as u32) as i32 + margin;
    (cell_origin_x + off_x, cell_origin_z + off_z)
}

/// Hut placement inside a hideout. 3-4 huts, each a 3×3 oak-plank floor
/// with a 1-block log wall on one side.
#[derive(Clone, Debug)]
pub struct HutSpec {
    /// NW-corner world position of the 3×3 floor.
    pub origin: [i32; 3],
}

/// Layout for a single hideout cell. Pure data; the apply-pass writes
/// the actual blocks into the world.
#[derive(Clone, Debug)]
pub struct HideoutLayout {
    pub anchor_world: [i32; 3],
    pub gx: i32,
    pub gz: i32,
    pub huts: Vec<HutSpec>,
    pub has_berserker: bool,
}

/// Compute the layout for a cell, or None if the cell has no hideout,
/// the biome is unsupported, or the anchor is too close to a village.
///
/// `village_anchors` is a snapshot of `World::village_anchors` taken
/// before the call. Pure — no world reads beyond `biome_gen`.
pub fn layout_for_hideout_cell(
    world_seed: u32,
    gx: i32,
    gz: i32,
    biome_gen: &BiomeGenerator,
    village_anchors: &AHashMap<(i32, i32), [i32; 3]>,
) -> Option<HideoutLayout> {
    if !cell_has_hideout(world_seed, gx, gz) {
        return None;
    }
    let (ax, az) = anchor_world_xz(world_seed, gx, gz);

    // Biome gate — temperate biomes only.
    let biome = biome_gen.biome_at(ax, az);
    if !matches!(
        biome,
        Biome::Plains | Biome::Forest | Biome::Savanna | Biome::Taiga
    ) {
        return None;
    }

    // Sea-level gate — don't place a hideout in a flooded pit. Reuse
    // the village rejection rule.
    let anchor_surface = biome_gen.terrain_height(ax, az);
    if anchor_surface < crate::biome::SEA_LEVEL + 2 {
        return None;
    }

    // Village-distance gate — reject if any village anchor sits within
    // MIN_DISTANCE_FROM_VILLAGE_BLOCKS (Chebyshev) of the hideout anchor.
    for &[vx, _vy, vz] in village_anchors.values() {
        let dx = (vx - ax).abs();
        let dz = (vz - az).abs();
        if dx.max(dz) < MIN_DISTANCE_FROM_VILLAGE_BLOCKS {
            return None;
        }
    }

    let h = hideout_hash(world_seed, gx, gz);

    // Hut count: 3-4.
    let n_huts = 3 + (h.rotate_left(5) % 2) as usize;
    let mut huts: Vec<HutSpec> = Vec::with_capacity(n_huts);
    for i in 0..n_huts {
        let h_i = h.wrapping_mul(2654435761).wrapping_add((i as u32).wrapping_mul(40503));
        let angle_base = (i as f32) / (n_huts as f32) * std::f32::consts::TAU + 0.5;
        let angle_jitter = (h_i % 40) as f32 / 200.0;
        let angle = angle_base + angle_jitter;
        // Huts on a tight inner ring (radius 3-4) so they sit inside
        // the palisade.
        let radius = 3.5 + (h_i.rotate_left(5) % 2) as f32;
        let dx = (radius * angle.cos()).round() as i32;
        let dz = (radius * angle.sin()).round() as i32;
        let hx = ax + dx;
        let hz = az + dz;
        let h_surface = biome_gen.terrain_height(hx, hz);
        huts.push(HutSpec { origin: [hx, h_surface, hz] });
    }

    Some(HideoutLayout {
        anchor_world: [ax, anchor_surface, az],
        gx,
        gz,
        huts,
        has_berserker: cell_has_berserker(world_seed, gx, gz),
    })
}

/// Place a hideout's blocks for any cell whose footprint reaches into
/// this column. Mirrors `village_gen::place_villages_for_column`.
///
/// On a new placement, also seeds `World::brigand_hideouts` with the
/// initial HideoutData entry + loots the stockpile chest. Idempotent —
/// repeat calls on a re-streamed column overwrite blocks with the same
/// deterministic layout and don't touch HideoutData on a re-visit.
pub fn place_hideouts_for_column(
    world: &mut World,
    cx: i32,
    cz: i32,
    biome_gen: &BiomeGenerator,
    world_seed: u32,
) {
    let cs = CHUNK_SIZE as i32;
    let col_min_x = cx * cs;
    let col_min_z = cz * cs;
    let col_max_x = col_min_x + cs - 1;
    let col_max_z = col_min_z + cs - 1;

    let gx_centre = cx.div_euclid(HIDEOUT_GRID);
    let gz_centre = cz.div_euclid(HIDEOUT_GRID);

    // Snapshot village anchors once — the layout call needs read-only
    // access and we're about to mutate `world` for block placement.
    let village_snapshot: AHashMap<(i32, i32), [i32; 3]> =
        world.village_anchors.iter().map(|(&k, &v)| (k, v)).collect();

    for dgz in -1..=1 {
        for dgx in -1..=1 {
            let gx = gx_centre + dgx;
            let gz = gz_centre + dgz;
            let Some(layout) = layout_for_hideout_cell(world_seed, gx, gz, biome_gen, &village_snapshot) else {
                continue;
            };
            let [ax, _ay, az] = layout.anchor_world;
            let reach = (HIDEOUT_CHUNK_REACH + 1) * cs;
            if (ax + reach) < col_min_x || (ax - reach) > col_max_x
                || (az + reach) < col_min_z || (az - reach) > col_max_z
            {
                continue;
            }
            apply_layout_to_column(
                world,
                &layout,
                col_min_x, col_min_z, col_max_x, col_max_z,
                world_seed,
            );
        }
    }
}

/// Apply a hideout layout's blocks to the slice of the world inside
/// the given column bounds. Idempotent on the block side; on the side-
/// table side, only inserts a fresh HideoutData if the cell isn't
/// already tracked.
fn apply_layout_to_column(
    world: &mut World,
    layout: &HideoutLayout,
    col_min_x: i32,
    col_min_z: i32,
    col_max_x: i32,
    col_max_z: i32,
    world_seed: u32,
) {
    let bounds = ColumnBounds {
        min_x: col_min_x, min_z: col_min_z, max_x: col_max_x, max_z: col_max_z,
    };
    let [ax, ay, az] = layout.anchor_world;

    // Palisade — log ring at radius PALISADE_RADIUS, height PALISADE_HEIGHT.
    // Leave a 2-wide gap on the south side (+z direction) for the gate.
    let r = PALISADE_RADIUS;
    for dx in -r..=r {
        for dz in -r..=r {
            // Only stamp the ring perimeter (chebyshev = r).
            if dx.abs().max(dz.abs()) != r {
                continue;
            }
            // Gate: 2-block gap centred on +z.
            if dz == r && (dx == 0 || dx == 1) {
                continue;
            }
            for dy in 1..=PALISADE_HEIGHT {
                try_set(world, &bounds, ax + dx, ay + dy, az + dz, block::OAK_LOG);
            }
        }
    }

    // Cobblestone hearth + unlit campfire 2 blocks east of the anchor.
    let hx = ax + 2;
    let hy = ay;
    let hz = az;
    try_set(world, &bounds, hx, hy, hz, block::COBBLESTONE);
    try_set(world, &bounds, hx, hy + 1, hz, block::CAMPFIRE_UNLIT);

    // Stockpile chest at the anchor (with a cobblestone base under it).
    try_set(world, &bounds, ax, ay, az, block::COBBLESTONE);
    try_set(world, &bounds, ax, ay + 1, az, block::CHEST);
    // Seed the chest's contents if this column actually owns the
    // anchor cell (so the insertion happens once, idempotently).
    if bounds.contains(ax, az) && world.chest_at((ax, ay + 1, az)).is_none() {
        let mut chest = crate::chest::ChestData::new();
        for (i, stack) in seed_hideout_loot(world_seed, layout.gx, layout.gz).into_iter().enumerate() {
            if i < chest.slots.len() {
                chest.slots[i] = Some(stack);
            }
        }
        world.insert_chest((ax, ay + 1, az), chest);
    }

    // 4 corner torches on top of the palisade.
    for (dx, dz) in [(-r, -r), (r, -r), (-r, r), (r, r)] {
        try_set(world, &bounds, ax + dx, ay + PALISADE_HEIGHT + 1, az + dz, block::TORCH);
    }

    // Brigand Hideout Banner — at the south gate, one block out from
    // the palisade ring.
    let banner_x = ax;
    let banner_y = ay + 1;
    let banner_z = az + r + 1;
    try_set(world, &bounds, banner_x, banner_y, banner_z, block::BRIGAND_HIDEOUT_BANNER);

    // Huts — small 3×3 oak-plank floor + one log wall + a hay-bale "bed".
    for hut in &layout.huts {
        let [hx, hy, hz] = hut.origin;
        for ddz in 0..3 {
            for ddx in 0..3 {
                try_set(world, &bounds, hx + ddx, hy, hz + ddz, block::OAK_PLANKS);
            }
        }
        // One wall on the south side (back toward the gate) at height 1.
        for ddx in 0..3 {
            try_set(world, &bounds, hx + ddx, hy + 1, hz, block::OAK_LOG);
        }
        // Hay-bale "bed" in the middle.
        try_set(world, &bounds, hx + 1, hy + 1, hz + 1, block::HAY_BALE);
    }

    // Seed the side-table entry once. We don't overwrite existing
    // entries — preserves population state across re-streams.
    if !world.brigand_hideouts.contains_key(&(layout.gx, layout.gz)) {
        let target = if layout.has_berserker {
            POPULATION_TARGET_RARE
        } else {
            POPULATION_TARGET_NORMAL
        };
        world.brigand_hideouts.insert(
            (layout.gx, layout.gz),
            HideoutData {
                anchor_world: layout.anchor_world,
                gx: layout.gx,
                gz: layout.gz,
                population: 0,
                population_target: target,
                has_berserker: layout.has_berserker,
                last_replenish_tick: 0,
                dormant_since: None,
                world_seed,
            },
        );
    }
}

/// Pure: deterministic stockpile loot for a hideout. Seeded by
/// `(world_seed, gx, gz)` so the same hideout always has the same
/// initial loot. Returns 4-6 ItemStacks; never the chieftain trophy
/// (that's the Berserker kill drop, not stockpile).
pub fn seed_hideout_loot(world_seed: u32, gx: i32, gz: i32) -> Vec<ItemStack> {
    let h = hideout_hash(world_seed.wrapping_add(0xDEAD_BEEF), gx, gz);
    let mut drops: Vec<ItemStack> = Vec::new();
    // Wheat 2-4
    let wheat = 2 + (h % 3) as u8;
    drops.push(ItemStack::new_material(MaterialId::Wheat, wheat));
    // Bread 1-2
    let bread = 1 + ((h >> 4) % 2) as u8;
    drops.push(ItemStack::new_material(MaterialId::Bread, bread));
    // Iron 1-2
    let iron = 1 + ((h >> 8) % 2) as u8;
    drops.push(ItemStack::new_material(MaterialId::IronIngot, iron));
    // Wool 1-2
    let wool = 1 + ((h >> 12) % 2) as u8;
    drops.push(ItemStack::new_material(MaterialId::Wool, wool));
    drops
}

/// Replenish loop. Once per call (caller throttles), walks every
/// loaded hideout and tops its population up to target. The first
/// new mob in a fresh hideout is always the Marauder (the captain);
/// fills with Brigands; if `has_berserker` and population still under
/// target, the last slot is a Berserker.
///
/// Mobs spawned here carry `BrigandTier` + `HomeHideout` components
/// so the override pass + kill-attribution helper can find them.
///
/// Returns the number of mobs spawned this call.
pub fn tick_hideout_spawning(
    world: &mut World,
    ecs: &mut hecs::World,
    current_tick: u64,
) -> u32 {
    use crate::brigand::{BrigandTier, HomeHideout, Tier};
    use crate::chunk::CHUNK_SIZE;
    use crate::entity::spawn_mob;
    use crate::mob::MobType;
    use glam::Vec3;

    let mut spawned_total = 0u32;
    // Snapshot the keys so we don't borrow `world.brigand_hideouts`
    // mutably during iteration over its own contents.
    let keys: Vec<(i32, i32)> = world.brigand_hideouts.keys().copied().collect();
    for key in keys {
        let data = world.brigand_hideouts.get(&key).cloned();
        let Some(mut data) = data else { continue };

        // HP-3 v2 — stockpile replenishment state machine. Anchor's
        // chest sits one block above the anchor block (`apply_layout_to_column`
        // sets a cobblestone base at ay then CHEST at ay+1).
        let [ax, ay, az] = data.anchor_world;
        let chest_pos = (ax, ay + 1, az);
        let chest_empty = world
            .chest_at(chest_pos)
            .map(|c| c.occupied() == 0)
            .unwrap_or(true);

        // Dormant entry: cleared + looted. Marks the moment we noticed.
        if data.population == 0 && chest_empty && data.dormant_since.is_none() {
            data.dormant_since = Some(current_tick);
        }
        // Dormant exit: refill chest + reset population to trigger
        // the regular replenish loop on the next tick.
        if data.stockpile_should_reset(current_tick) {
            let mut chest = crate::chest::ChestData::new();
            for (i, stack) in seed_hideout_loot(data.world_seed, data.gx, data.gz).into_iter().enumerate() {
                if i < chest.slots.len() {
                    chest.slots[i] = Some(stack);
                }
            }
            world.insert_chest(chest_pos, chest);
            // HP-3 v2 — bleed: the refill came from raiding a nearby
            // village's treasury. Find the nearest village within
            // BRIGAND_STEAL_RADIUS_BLOCKS and deduct the bite.
            let bled = bleed_nearest_village_treasury(
                world,
                data.anchor_world,
                BRIGAND_STEAL_AMOUNT_SATS,
            );
            if bled > 0 {
                log::debug!(
                    "hideout at {:?} bled {bled} sats from a neighbouring village",
                    data.anchor_world,
                );
            }
            data.dormant_since = None;
            data.last_replenish_tick = 0; // force respawn on next tick
            // Persist the state change immediately so the rest of this
            // tick (and the next call) see the fresh state.
            world.brigand_hideouts.insert(key, data.clone());
        }
        // Alive brigands cancel any dormant tracking — the hideout is
        // contested again and can't sit on the reset cooldown.
        if data.population > 0 && data.dormant_since.is_some() {
            data.dormant_since = None;
            world.brigand_hideouts.insert(key, data.clone());
        }

        if !data.needs_replenish(current_tick) {
            // Persist any dormant_since change we made above.
            if world.brigand_hideouts.get(&key).map(|d| d.dormant_since) != Some(data.dormant_since) {
                world.brigand_hideouts.insert(key, data.clone());
            }
            continue;
        }
        // Anchor chunk must be loaded — don't spawn into the void.
        let cs = CHUNK_SIZE as i32;
        let cx = ax.div_euclid(cs);
        let cy = ay.div_euclid(cs);
        let cz = az.div_euclid(cs);
        if !world.has_chunk(cx, cy, cz) {
            continue;
        }
        let deficit = data.population_target.saturating_sub(data.population);
        // Determine the kind of each new mob, in order:
        // - First in a fresh hideout: Marauder.
        // - Fill with Brigands.
        // - If has_berserker and we have at least one slot left at the
        //   final population, mark that final slot Berserker.
        for slot in 0..deficit {
            let population_after = data.population + 1;
            let kind = if data.population == 0 {
                MobType::Marauder
            } else if data.has_berserker && population_after == data.population_target {
                MobType::Berserker
            } else {
                MobType::Brigand
            };
            // Spawn 1 block above the anchor floor so the entity stands
            // on the cobble base rather than clipping into it.
            let spawn_pos = Vec3::new(
                ax as f32 + 0.5 + slot as f32 * 0.5,
                ay as f32 + 2.0,
                az as f32 + 0.5,
            );
            let id = spawn_mob(ecs, kind, spawn_pos);
            let tier = Tier::from_mob_type(kind).expect("brigand-family kind");
            let _ = ecs.insert(id, (
                BrigandTier { tier },
                HomeHideout { anchor: data.anchor_world },
            ));
            data.population += 1;
            spawned_total += 1;
        }
        data.last_replenish_tick = current_tick;
        world.brigand_hideouts.insert(key, data);
    }
    spawned_total
}

/// HP-3 v2 — deduct up to `amount` sats from the nearest village
/// treasury within `BRIGAND_STEAL_RADIUS_BLOCKS` of `hideout_anchor`.
/// Returns the actual amount deducted (0 if no neighbouring village,
/// or if all neighbouring treasuries were empty).
///
/// Pure on `&mut World` — separates village lookup from mutation so
/// the bleed step is independently testable.
pub fn bleed_nearest_village_treasury(
    world: &mut World,
    hideout_anchor: [i32; 3],
    amount: u64,
) -> u64 {
    // Snapshot the anchor map so we don't borrow it while mutating
    // village_treasuries below.
    let candidates: Vec<((i32, i32), [i32; 3])> = world
        .village_anchors
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    let mut best: Option<((i32, i32), f32)> = None;
    for (key, anchor) in candidates {
        let dx = (anchor[0] - hideout_anchor[0]) as f32;
        let dz = (anchor[2] - hideout_anchor[2]) as f32;
        let dist = (dx * dx + dz * dz).sqrt();
        if dist > BRIGAND_STEAL_RADIUS_BLOCKS {
            continue;
        }
        if best.map(|(_, bd)| dist < bd).unwrap_or(true) {
            best = Some((key, dist));
        }
    }
    let Some((vkey, _)) = best else { return 0 };
    let entry = world.village_treasuries.entry(vkey).or_insert(0);
    let bled = amount.min(*entry);
    *entry -= bled;
    bled
}

/// Decrement the population of the hideout owning the given anchor.
/// Idempotent if no matching hideout exists (logs a debug + returns).
/// Caller (combat / kill-attribution path) invokes this when a
/// brigand-family mob dies; the mob's `HomeHideout` component carries
/// the anchor.
pub fn on_brigand_killed(world: &mut World, home_anchor: [i32; 3]) {
    // Find the hideout whose anchor matches; usually only one will.
    for data in world.brigand_hideouts.values_mut() {
        if data.anchor_world == home_anchor {
            data.population = data.population.saturating_sub(1);
            return;
        }
    }
}

#[derive(Clone, Copy)]
struct ColumnBounds {
    min_x: i32,
    min_z: i32,
    max_x: i32,
    max_z: i32,
}

impl ColumnBounds {
    fn contains(&self, x: i32, z: i32) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }
}

fn try_set(world: &mut World, bounds: &ColumnBounds, x: i32, y: i32, z: i32, b: crate::block::BlockId) {
    if bounds.contains(x, z) {
        world.set_block(x, y, z, b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::BiomeGenerator;
    use crate::item::{Item, MaterialId};

    fn bg(seed: u32) -> BiomeGenerator {
        BiomeGenerator::new(seed)
    }

    #[test]
    fn hideout_hash_is_deterministic() {
        let a = hideout_hash(123, 5, 5);
        let b = hideout_hash(123, 5, 5);
        assert_eq!(a, b);
    }

    #[test]
    fn cell_has_hideout_distribution_is_roughly_60_percent() {
        let mut yes = 0;
        let total = 10_000;
        for i in 0..total {
            if cell_has_hideout(7, i, i * 3) {
                yes += 1;
            }
        }
        // 60 % ± 5 % is generous.
        assert!(yes > total * 55 / 100 && yes < total * 65 / 100,
            "expected ~60% hideout cells, got {yes}/{total}");
    }

    #[test]
    fn cell_has_berserker_is_minority() {
        let mut yes = 0;
        let total = 10_000;
        for i in 0..total {
            if cell_has_berserker(7, i, i * 3) {
                yes += 1;
            }
        }
        // 20 % ± 5 %.
        assert!(yes > total * 15 / 100 && yes < total * 25 / 100,
            "expected ~20% rare hideouts, got {yes}/{total}");
    }

    #[test]
    fn seed_hideout_loot_yields_4_stacks_in_expected_ranges() {
        for seed in 0u32..50 {
            let loot = seed_hideout_loot(seed, 0, 0);
            assert_eq!(loot.len(), 4, "hideout loot must have 4 stacks");
            let wheat = loot.iter().find(|s| matches!(s.item,
                Item::Material(MaterialId::Wheat))).unwrap();
            assert!(wheat.count >= 2 && wheat.count <= 4, "wheat={}", wheat.count);
            let bread = loot.iter().find(|s| matches!(s.item,
                Item::Material(MaterialId::Bread))).unwrap();
            assert!(bread.count >= 1 && bread.count <= 2, "bread={}", bread.count);
            let iron = loot.iter().find(|s| matches!(s.item,
                Item::Material(MaterialId::IronIngot))).unwrap();
            assert!(iron.count >= 1 && iron.count <= 2, "iron={}", iron.count);
            let wool = loot.iter().find(|s| matches!(s.item,
                Item::Material(MaterialId::Wool))).unwrap();
            assert!(wool.count >= 1 && wool.count <= 2, "wool={}", wool.count);
            // Trophy is NOT stockpile loot — only Berserker kill drop.
            assert!(loot.iter().all(|s| !matches!(s.item,
                Item::Material(MaterialId::BrigandChieftainTrophy))));
        }
    }

    #[test]
    fn seed_hideout_loot_deterministic_per_cell() {
        let a = seed_hideout_loot(42, 5, 5);
        let b = seed_hideout_loot(42, 5, 5);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.count, y.count);
        }
    }

    #[test]
    fn layout_rejects_near_village() {
        let bg = bg(123);
        // Force a cell to have a hideout we can find.
        let mut found: Option<(i32, i32)> = None;
        for gx in 0..50 {
            for gz in 0..50 {
                if let Some(layout) = layout_for_hideout_cell(123, gx, gz, &bg, &AHashMap::new()) {
                    found = Some((gx, gz));
                    let _ = layout; // just need any cell that places
                    break;
                }
            }
            if found.is_some() { break; }
        }
        let (gx, gz) = found.expect("expected a hideout-bearing cell in the search range");

        // Pretend a village sits right on top of the hideout anchor.
        let (ax, az) = anchor_world_xz(123, gx, gz);
        let mut villages = AHashMap::new();
        villages.insert((0, 0), [ax, 70, az]);
        let layout = layout_for_hideout_cell(123, gx, gz, &bg, &villages);
        assert!(layout.is_none(), "layout must reject when a village sits at the anchor");
    }

    #[test]
    fn layout_accepts_when_village_far_away() {
        let bg = bg(123);
        let mut found: Option<(i32, i32)> = None;
        for gx in 0..50 {
            for gz in 0..50 {
                if layout_for_hideout_cell(123, gx, gz, &bg, &AHashMap::new()).is_some() {
                    found = Some((gx, gz));
                    break;
                }
            }
            if found.is_some() { break; }
        }
        let (gx, gz) = found.expect("expected a hideout-bearing cell");
        let mut villages = AHashMap::new();
        // Place a village 2000 blocks away — well outside the 128-block gate.
        villages.insert((0, 0), [9999, 70, 9999]);
        assert!(layout_for_hideout_cell(123, gx, gz, &bg, &villages).is_some(),
            "layout must accept when villages are far away");
    }

    #[test]
    fn needs_replenish_respects_cooldown() {
        let mut data = HideoutData {
            anchor_world: [0, 64, 0],
            gx: 0,
            gz: 0,
            population: 2,
            population_target: 4,
            has_berserker: false,
            last_replenish_tick: 1000,
            dormant_since: None,
            world_seed: 42,
        };
        // Within cooldown → no replenish.
        assert!(!data.needs_replenish(1500));
        // Cooldown expired → replenish.
        assert!(data.needs_replenish(1000 + REPLENISH_COOLDOWN_TICKS));
        // At target → no replenish even after cooldown.
        data.population = 4;
        assert!(!data.needs_replenish(1000 + REPLENISH_COOLDOWN_TICKS + 1));
    }

    // HP-3 v2 — stockpile replenishment tests.

    fn fresh_hideout(world: &mut crate::world::World, anchor: [i32; 3]) -> (i32, i32) {
        // Construct a hideout entry in the side-table directly, mirroring
        // what `apply_layout_to_column` would write — then place the
        // anchor chunk + cobble + chest so the spawner's chunk-loaded
        // check passes.
        let key = (0, 0);
        let [ax, ay, az] = anchor;
        // Solid chunk under the anchor.
        world.set_block(ax, ay, az, crate::block::COBBLESTONE);
        world.set_block(ax, ay + 1, az, crate::block::CHEST);
        // Stockpile starts FILLED so dormant entry waits on player looting.
        let mut c = crate::chest::ChestData::new();
        c.slots[0] = Some(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Wheat, 2));
        world.insert_chest((ax, ay + 1, az), c);
        world.brigand_hideouts.insert(key, HideoutData {
            anchor_world: anchor,
            gx: key.0,
            gz: key.1,
            population: 0,
            population_target: 4,
            has_berserker: false,
            last_replenish_tick: 0,
            dormant_since: None,
            world_seed: 42,
        });
        key
    }

    #[test]
    fn cleared_hideout_with_full_chest_does_not_enter_dormant() {
        // Population 0 but chest has loot → not dormant (player hasn't
        // looted yet).
        let mut world = crate::world::World::new();
        let key = fresh_hideout(&mut world, [0, 64, 0]);
        let mut ecs = hecs::World::new();
        let _ = tick_hideout_spawning(&mut world, &mut ecs, 100);
        let data = world.brigand_hideouts.get(&key).unwrap();
        assert!(data.dormant_since.is_none(),
            "non-empty chest must not mark dormant; got {:?}", data.dormant_since);
    }

    #[test]
    fn cleared_and_looted_hideout_enters_dormant() {
        let mut world = crate::world::World::new();
        let key = fresh_hideout(&mut world, [0, 64, 0]);
        // Empty the chest to simulate player looting.
        if let Some(chest) = world.chest_at_mut((0, 65, 0)) {
            for slot in chest.slots.iter_mut() {
                *slot = None;
            }
        }
        let mut ecs = hecs::World::new();
        let _ = tick_hideout_spawning(&mut world, &mut ecs, 500);
        let data = world.brigand_hideouts.get(&key).unwrap();
        assert_eq!(data.dormant_since, Some(500),
            "cleared + looted hideout must enter dormant at the current tick");
    }

    #[test]
    fn stockpile_resets_after_cooldown() {
        let mut world = crate::world::World::new();
        let key = fresh_hideout(&mut world, [0, 64, 0]);
        // Empty the chest.
        if let Some(chest) = world.chest_at_mut((0, 65, 0)) {
            for slot in chest.slots.iter_mut() {
                *slot = None;
            }
        }
        let mut ecs = hecs::World::new();
        // Tick at t=0: enters dormant.
        let _ = tick_hideout_spawning(&mut world, &mut ecs, 0);
        let data = world.brigand_hideouts.get(&key).unwrap();
        assert_eq!(data.dormant_since, Some(0));
        // Tick at t = STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS: chest
        // should refill and dormant_since clears.
        let _ = tick_hideout_spawning(
            &mut world,
            &mut ecs,
            STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS,
        );
        let data = world.brigand_hideouts.get(&key).unwrap();
        assert!(data.dormant_since.is_none(),
            "dormant_since should clear after reset");
        let chest = world.chest_at((0, 65, 0)).expect("chest still present");
        assert!(chest.occupied() > 0, "chest should be refilled");
    }

    #[test]
    fn alive_brigands_cancel_dormant_state() {
        let mut world = crate::world::World::new();
        let key = fresh_hideout(&mut world, [0, 64, 0]);
        // Empty chest then mark dormant via tick.
        if let Some(chest) = world.chest_at_mut((0, 65, 0)) {
            for slot in chest.slots.iter_mut() {
                *slot = None;
            }
        }
        let mut ecs = hecs::World::new();
        let _ = tick_hideout_spawning(&mut world, &mut ecs, 100);
        assert!(world.brigand_hideouts.get(&key).unwrap().dormant_since.is_some());
        // A brigand respawns (population > 0 — simulate by direct edit).
        world.brigand_hideouts.get_mut(&key).unwrap().population = 1;
        // Next tick: dormant_since should clear because the hideout is
        // contested again.
        let _ = tick_hideout_spawning(&mut world, &mut ecs, 200);
        assert!(world.brigand_hideouts.get(&key).unwrap().dormant_since.is_none(),
            "alive brigand should cancel dormant tracking");
    }

    // HP-3 v2 — brigand→village treasury bleed tests.

    #[test]
    fn bleed_takes_from_nearest_village_within_radius() {
        let mut world = crate::world::World::new();
        // Hideout at origin; village 100 blocks away — within
        // BRIGAND_STEAL_RADIUS_BLOCKS (256).
        world.village_anchors.insert((0, 0), [100, 64, 0]);
        world.village_treasuries.insert((0, 0), 1_000);
        let bled = bleed_nearest_village_treasury(&mut world, [0, 64, 0], 50);
        assert_eq!(bled, 50);
        assert_eq!(world.village_treasuries.get(&(0, 0)), Some(&950));
    }

    #[test]
    fn bleed_picks_nearest_when_multiple_villages() {
        let mut world = crate::world::World::new();
        world.village_anchors.insert((0, 0), [200, 64, 0]);
        world.village_treasuries.insert((0, 0), 1_000);
        world.village_anchors.insert((1, 0), [50, 64, 0]);
        world.village_treasuries.insert((1, 0), 1_000);
        let bled = bleed_nearest_village_treasury(&mut world, [0, 64, 0], 50);
        assert_eq!(bled, 50);
        // Nearer village (key (1, 0) at distance 50) loses sats; farther
        // village (key (0, 0) at distance 200) stays whole.
        assert_eq!(world.village_treasuries.get(&(1, 0)), Some(&950));
        assert_eq!(world.village_treasuries.get(&(0, 0)), Some(&1_000));
    }

    #[test]
    fn bleed_returns_zero_when_no_village_in_range() {
        let mut world = crate::world::World::new();
        // Village 500 blocks away — outside radius.
        world.village_anchors.insert((0, 0), [500, 64, 500]);
        world.village_treasuries.insert((0, 0), 1_000);
        let bled = bleed_nearest_village_treasury(&mut world, [0, 64, 0], 50);
        assert_eq!(bled, 0);
        assert_eq!(world.village_treasuries.get(&(0, 0)), Some(&1_000));
    }

    #[test]
    fn bleed_capped_by_current_treasury() {
        let mut world = crate::world::World::new();
        world.village_anchors.insert((0, 0), [100, 64, 0]);
        world.village_treasuries.insert((0, 0), 20);
        // Try to bleed 50, but treasury only has 20.
        let bled = bleed_nearest_village_treasury(&mut world, [0, 64, 0], 50);
        assert_eq!(bled, 20);
        assert_eq!(world.village_treasuries.get(&(0, 0)), Some(&0));
    }

    #[test]
    fn bleed_returns_zero_when_treasury_already_empty() {
        let mut world = crate::world::World::new();
        world.village_anchors.insert((0, 0), [100, 64, 0]);
        world.village_treasuries.insert((0, 0), 0);
        let bled = bleed_nearest_village_treasury(&mut world, [0, 64, 0], 50);
        assert_eq!(bled, 0);
    }

    #[test]
    fn stockpile_reset_fires_bleed_against_neighbouring_village() {
        // End-to-end: hideout near a village; clear + loot + wait for
        // reset; the village treasury should drop by the bleed amount.
        let mut world = crate::world::World::new();
        let key = fresh_hideout(&mut world, [0, 64, 0]);
        world.village_anchors.insert((9, 9), [150, 64, 0]);
        world.village_treasuries.insert((9, 9), 500);
        // Loot the chest.
        if let Some(chest) = world.chest_at_mut((0, 65, 0)) {
            for slot in chest.slots.iter_mut() { *slot = None; }
        }
        let mut ecs = hecs::World::new();
        let _ = tick_hideout_spawning(&mut world, &mut ecs, 0);
        assert!(world.brigand_hideouts.get(&key).unwrap().dormant_since.is_some());
        // Reset trigger.
        let _ = tick_hideout_spawning(
            &mut world,
            &mut ecs,
            STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS,
        );
        let post = world.village_treasuries.get(&(9, 9)).copied().unwrap_or(0);
        assert_eq!(post, 500 - BRIGAND_STEAL_AMOUNT_SATS,
            "village should bleed exactly BRIGAND_STEAL_AMOUNT_SATS on reset");
    }

    #[test]
    fn stockpile_should_reset_only_when_dormant_long_enough() {
        let mut data = HideoutData {
            anchor_world: [0, 64, 0],
            gx: 0, gz: 0,
            population: 0,
            population_target: 4,
            has_berserker: false,
            last_replenish_tick: 0,
            dormant_since: Some(1_000),
            world_seed: 42,
        };
        assert!(!data.stockpile_should_reset(1_500));
        assert!(data.stockpile_should_reset(
            1_000 + STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS,
        ));
        data.dormant_since = None;
        assert!(!data.stockpile_should_reset(u64::MAX),
            "never dormant → never reset");
    }
}
