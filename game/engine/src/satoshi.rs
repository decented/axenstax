//! Satoshi — the optional, fully-scripted in-world onboarding guide.
//!
//! Design: `docs/superpowers/specs/2026-06-22-guided-onboarding-satoshi-design.md`
//! v2 design: `docs/superpowers/specs/2026-06-23-satoshi-onboarding-v2-design.md`
//!
//! Satoshi is a `MobType::Villager` + a [`SatoshiMarker`] tag (NOT a new
//! `MobType`), so he rides every existing villager code path for free. He lives
//! in a small **wooden** hut near the world spawn and gives the player a
//! **starter hut schematic** (a `Plan`) to build — Survival greets with food
//! first, Creative goes straight to the plan. He only appears in worlds created
//! to host him ([`SatoshiState::enabled`], set at create for new normal
//! Survival/Creative worlds), never in an existing world.
//!
//! COMPLIANCE (load-bearing, enforced by `corpus_has_no_money_or_earning_words`):
//! every authored line is kid-safe, UK English, and NEVER mentions sats,
//! Bitcoin, earning, payouts, wallets, money, or rewards-as-money. His currency
//! is *fun and care*. There is NO live AI, no network, no free-text input.

use crate::block::{self, BlockId, BlockRegistry};
use crate::chunk::CHUNK_SIZE;
use crate::plan::{CapturedCell, PlanData};
use crate::world::World;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Marks the single villager who is Satoshi in a given world. Satoshi is a
/// `MobType::Villager` + this marker — never a new `MobType` — so he stays on
/// every existing villager code path. `Profession::None` (no workstation claim).
pub struct SatoshiMarker;

/// Per-world Satoshi progress + placement. Persisted as the NEWEST appended
/// field on [`crate::save::WorldSave`] (after `saved_mobs`) and mirrored on
/// [`World`]. Brand-new struct → its internal field order is free *now*; any
/// FUTURE field must be appended (bincode is positional).
#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SatoshiState {
    /// House origin `[x, y, z]` once placed. `None` until the spawn-area ground
    /// is loaded. House *blocks* persist in the chunks; the villager *entity* is
    /// re-derived on load (not saved) — re-spawned here if no marker exists.
    pub home: Option<[i32; 3]>,
    /// He has greeted the player at least once.
    pub has_greeted: bool,
    /// The welcome **food** gift was handed over (Survival care touch).
    pub has_gifted: bool,
    /// The starter **schematic** was handed over (the centrepiece gift).
    pub schematic_gifted: bool,
    /// The player waved him off — pull-only thereafter (never initiates again).
    pub waved_off: bool,
    /// Last tick he initiated contact (frequency-budget guard).
    pub last_initiated_tick: u64,
    /// Whether this world should host Satoshi at all. Mirrored from
    /// `WorldMeta.satoshi_enabled` on load — `true` only for new normal
    /// Survival/Creative worlds. Existing/flat/gallery/workshop worlds and
    /// Adventure/Spectator never set it, so he never appears there.
    pub enabled: bool,
    /// Session-only note (never saved: `serde(skip)`, so the save layout is
    /// unchanged): the last full site search found no place, when and with how
    /// many sections loaded. [`tick_spawn`] searches again only after
    /// [`NO_SITE_RETRY_TICKS`] or when the loaded set changed, not every second.
    #[serde(skip)]
    pub no_site: Option<NoSiteNote>,
}

/// What a search that found no site noted: see [`SatoshiState::no_site`].
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct NoSiteNote {
    /// The tick the search ran.
    pub tick: u64,
    /// How many sections the search's columns held then ([`reach_sections`]).
    pub loaded_sections: usize,
}

/// How long (ticks, ~30 s) a search that found nothing is not repeated while
/// the loaded set stays as it was.
const NO_SITE_RETRY_TICKS: u64 = 600;

/// Minimum ticks between any Satoshi-initiated cue (~1 in-game day) — anti-Navi.
/// MVP keeps initiation off in practice (see `should_initiate` below) —
/// tested directly.
#[cfg_attr(not(test), allow(dead_code))]
pub const SATOSHI_INITIATE_COOLDOWN_TICKS: u64 = 24_000;

// ── House placement (a deterministic search outward from the world spawn) ──
/// Where the search first tries the house origin, relative to the world spawn
/// column: a few blocks off it, as the house always stood off the (0,0) spawn.
const HOUSE_OX: i32 = 4;
const HOUSE_OZ: i32 = 3;
const HUT_W: u8 = 5; // square footprint (x and z)
const HUT_H: u8 = 4; // floor → roof: ry 0..=3 (floor, 2 walls, roof)
/// How far (blocks, on each axis) the site search reaches from the first try.
const SITE_REACH: i32 = 24;
/// The most the dry ground may differ in height across the footprint for the
/// first pass of the search; a second pass takes [`MAX_STEP_RELAXED`].
const MAX_STEP: i32 = 1;
const MAX_STEP_RELAXED: i32 = 3;

/// Centre column of a house whose origin is `(ox, oz)` (for the stand spot).
const fn centre_of(ox: i32, oz: i32) -> (i32, i32) {
    (ox + HUT_W as i32 / 2, oz + HUT_W as i32 / 2)
}

/// Satoshi's stand position: centre of the house at origin `(ox, oz)`, one
/// block above the floor `floor_y`.
fn satoshi_pos(ox: i32, floor_y: i32, oz: i32) -> Vec3 {
    let (cx, cz) = centre_of(ox, oz);
    Vec3::new(cx as f32 + 0.5, floor_y as f32 + 1.0, cz as f32 + 0.5)
}

/// Where the house goes: its origin, its floor height, and the top of the
/// ground under each footprint column (`tops[dz][dx]`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct HouseSite {
    ox: i32,
    oy: i32,
    oz: i32,
    tops: [[i32; HUT_W as usize]; HUT_W as usize],
}

/// The top of the dry ground in column `(x, z)`: the first solid, non-liquid
/// block walking down from the column's top, past air and plants. `None` for a
/// column that is not loaded and one topped by water or lava. A tree (leaves or
/// a trunk above the ground) is no place to build on, so a column with one is
/// `None` too, unless `through_trees`: then the walk goes on past leaves and
/// logs to the ground under them (the last pass of the search, so a forest
/// does not leave Satoshi with no house at all; the stamp clears what is in the
/// footprint's volume).
fn ground_top(world: &World, registry: &BlockRegistry, x: i32, z: i32, through_trees: bool) -> Option<i32> {
    let mut y = world.column_top_y(x, z);
    while y > 0 {
        let b = world.get_block(x, y, z);
        if block::is_fluid(b) {
            return None;
        }
        let tree = block::is_any_leaves(b) || block::is_any_log_block(b);
        if tree && !through_trees {
            return None;
        }
        if b != block::AIR && !tree && registry.is_solid(b) {
            return Some(y);
        }
        y -= 1;
    }
    None
}

/// The house site at origin `(ox, oz)`, if every footprint column is dry
/// ground (`ground_top`) within `max_step` blocks of the others. The floor
/// sits one above the highest of them, so nothing is buried and any lower
/// column is filled up to it by [`build_house`].
fn site_at(
    world: &World,
    registry: &BlockRegistry,
    memo: &mut ahash::AHashMap<(i32, i32), Option<i32>>,
    (ox, oz): (i32, i32),
    max_step: i32,
    through_trees: bool,
) -> Option<HouseSite> {
    let mut tops = [[0; HUT_W as usize]; HUT_W as usize];
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for dz in 0..HUT_W as i32 {
        for dx in 0..HUT_W as i32 {
            let (x, z) = (ox + dx, oz + dz);
            let top = *memo
                .entry((x, z))
                .or_insert_with(|| ground_top(world, registry, x, z, through_trees));
            let top = top?;
            tops[dz as usize][dx as usize] = top;
            lo = lo.min(top);
            hi = hi.max(top);
        }
    }
    (hi - lo <= max_step).then_some(HouseSite { ox, oy: hi + 1, oz, tops })
}

/// What a site search came to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SiteSearch {
    /// A chunk column the search reads is not loaded: try again later.
    Waiting,
    /// Every column is loaded and nothing in reach is acceptable.
    NoSite,
    Found(HouseSite),
}

impl SiteSearch {
    #[cfg(test)]
    fn site(self) -> Option<HouseSite> {
        match self {
            Self::Found(site) => Some(site),
            _ => None,
        }
    }
}

/// Is every chunk column a search from `first` can read held (blocks in it)?
/// The reach is `SITE_REACH` each way from the first try, a footprint wide
/// and one block of halo, so about 5 x 5 columns. Without this the answer
/// would depend on which columns happen to have streamed in by then (and
/// where the player came from), not on the seed and the world's edits alone.
fn search_columns_held(world: &World, first: (i32, i32)) -> bool {
    let (mut xs, zs) = search_columns(first);
    xs.all(|cx| zs.clone().all(|cz| world.column_holds_blocks(cx, cz)))
}

/// The chunk columns (x range, z range) a search from `first` reads.
fn search_columns(first: (i32, i32)) -> (std::ops::RangeInclusive<i32>, std::ops::RangeInclusive<i32>) {
    let cs = CHUNK_SIZE as i32;
    let (reach, far) = (SITE_REACH + 1, SITE_REACH + HUT_W as i32);
    (
        (first.0 - reach).div_euclid(cs)..=(first.0 + far).div_euclid(cs),
        (first.1 - reach).div_euclid(cs)..=(first.1 + far).div_euclid(cs),
    )
}

/// How many sections the search's columns hold: what changes when ground
/// streams in or out, so a search that found nothing runs again only then.
fn reach_sections(world: &World, first: (i32, i32)) -> usize {
    let (xs, zs) = search_columns(first);
    xs.map(|cx| {
        zs.clone()
            .map(|cz| (0..=crate::world::MAX_CHUNK_Y).filter(|&cy| world.has_chunk(cx, cy, cz)).count())
            .sum::<usize>()
    })
    .sum()
}

/// Would building on `site` touch anything a player made? Looks at every cell
/// of the footprint and its one-block halo, from each column's ground up to the
/// top of the house: a block a player placed, a block entity (a chest, a
/// furnace...), or any section edited since generation (a field, torches,
/// decoration: whatever a save keeps). Natural terrain and trees pass. The
/// stamp clears its volume and buries what is under its floor, so a world with
/// someone's base on it never gets a house over it.
fn site_touches_player_work(world: &World, registry: &BlockRegistry, site: &HouseSite) -> bool {
    let cs = CHUNK_SIZE as i32;
    let w = HUT_W as i32;
    let top = site.oy + HUT_H as i32;
    // Each column's lowest cell to look at: the ground under a footprint
    // column (the foundation is written above it); a halo column's ground, or
    // the floor level when it has none (water, unloaded), never above the floor.
    let mut cols: Vec<(i32, i32, i32)> = Vec::with_capacity(((w + 2) * (w + 2)) as usize);
    for dz in -1..=w {
        for dx in -1..=w {
            let (x, z) = (site.ox + dx, site.oz + dz);
            let from = if (0..w).contains(&dx) && (0..w).contains(&dz) {
                site.tops[dz as usize][dx as usize]
            } else {
                ground_top(world, registry, x, z, true).unwrap_or(site.oy - 1).min(site.oy - 1)
            };
            cols.push((x, z, from));
        }
    }
    // Sections first: it is the cheap test, and the usual reason.
    let low = cols.iter().map(|c| c.2).min().unwrap_or(site.oy);
    for cy in low.div_euclid(cs)..=top.div_euclid(cs) {
        for cx in (site.ox - 1).div_euclid(cs)..=(site.ox + w).div_euclid(cs) {
            for cz in (site.oz - 1).div_euclid(cs)..=(site.oz + w).div_euclid(cs) {
                if world.section_edited(cx, cy, cz) {
                    return true;
                }
            }
        }
    }
    cols.iter().any(|&(x, z, from)| {
        (from.max(0)..=top).any(|y| world.is_placed(x, y, z) || world.block_entities.contains_key(&(x, y, z)))
    })
}

/// Find the house site near the world spawn column `spawn`: the first try is
/// `spawn + (HOUSE_OX, HOUSE_OZ)`, then square rings outward (a fixed order,
/// so the answer is the same for a world on every machine) on the first pass's
/// flat enough ground, then again with the relaxed step, then once more with
/// the relaxed step looking through trees (never through water or lava).
///
/// Every chunk column the search reads must be loaded first
/// ([`search_columns_held`]), or [`SiteSearch::Waiting`]: nothing here
/// generates. A site is rejected when its footprint plus one block of halo
/// holds a cell in `avoid` (the spawn column, the player's real start, every
/// live player) or touches anything a player made
/// ([`site_touches_player_work`]); if nothing acceptable is in reach, the
/// answer is [`SiteSearch::NoSite`] and no house is built.
fn find_site(world: &World, registry: &BlockRegistry, spawn: (i32, i32), avoid: &[(i32, i32)]) -> SiteSearch {
    let first = (spawn.0 + HOUSE_OX, spawn.1 + HOUSE_OZ);
    if !search_columns_held(world, first) {
        return SiteSearch::Waiting;
    }
    let near_avoided = |origin: (i32, i32)| {
        std::iter::once(&spawn).chain(avoid).any(|&(x, z)| {
            (origin.0 - 1..=origin.0 + HUT_W as i32).contains(&x) && (origin.1 - 1..=origin.1 + HUT_W as i32).contains(&z)
        })
    };
    for (max_step, through_trees) in [(MAX_STEP, false), (MAX_STEP_RELAXED, false), (MAX_STEP_RELAXED, true)] {
        // The ground of a column depends on whether trees are looked through.
        let mut memo = ahash::AHashMap::new();
        for ring in 0..=SITE_REACH {
            for dz in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs().max(dz.abs()) != ring {
                        continue;
                    }
                    let origin = (first.0 + dx, first.1 + dz);
                    if near_avoided(origin) {
                        continue;
                    }
                    if let Some(site) = site_at(world, registry, &mut memo, origin, max_step, through_trees)
                        && !site_touches_player_work(world, registry, &site)
                    {
                        return SiteSearch::Found(site);
                    }
                }
            }
        }
    }
    SiteSearch::NoSite
}

/// The structural cells of the humble **wooden** hut, relative to the lowest-XZ
/// corner at `ry = 0` (floor). Oak-plank floor + roof, oak-log corner posts +
/// plank walls, a 2-high doorway on the front (`rz = 0`) centre column.
/// SHARED by [`build_house`] (Satoshi's home) and [`starter_hut_plan`] (the
/// gifted schematic), so "build one like mine" is literally true. "Start with
/// wood." Non-air cells only (air = omitted, e.g. the doorway).
pub fn hut_cells() -> Vec<CapturedCell> {
    let w = HUT_W;
    let h = HUT_H;
    let dc = w / 2; // door centre column
    let mut cells = Vec::new();
    for rz in 0..w {
        for rx in 0..w {
            cells.push(CapturedCell { rx, ry: 0, rz, block_id: block::OAK_PLANKS }); // floor
            cells.push(CapturedCell { rx, ry: h - 1, rz, block_id: block::OAK_PLANKS }); // roof
        }
    }
    // Walls (ry 1..=h-2), perimeter only; log corners + plank infill; door gap.
    for ry in 1..(h - 1) {
        for rz in 0..w {
            for rx in 0..w {
                let perim = rx == 0 || rx == w - 1 || rz == 0 || rz == w - 1;
                if !perim {
                    continue;
                }
                if rz == 0 && rx == dc {
                    continue; // doorway (front wall, centre, full height)
                }
                let corner = (rx == 0 || rx == w - 1) && (rz == 0 || rz == w - 1);
                let blk = if corner { block::OAK_LOG } else { block::OAK_PLANKS };
                cells.push(CapturedCell { rx, ry, rz, block_id: blk });
            }
        }
    }
    cells
}

/// The gifted starter schematic — the same wooden hut Satoshi lives in.
pub fn starter_hut_plan() -> PlanData {
    PlanData::from_imported(
        "Satoshi's Starter Hut".to_string(),
        HUT_W,
        HUT_W,
        HUT_H,
        hut_cells(),
    )
}

/// Writes to the world that remember what each cell was, so the caller can
/// relight and remesh exactly what changed: one entry per cell, the block it
/// held before the first write and the block it holds now (a cell written twice
/// is one entry; one that ends as it began is none).
struct Stamp<'a> {
    world: &'a mut World,
    cells: Vec<crate::lighting::CellChange>,
    index: ahash::AHashMap<(i32, i32, i32), usize>,
}

impl<'a> Stamp<'a> {
    fn new(world: &'a mut World) -> Self {
        Self { world, cells: Vec::new(), index: ahash::AHashMap::new() }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, to: BlockId) {
        match self.index.get(&(x, y, z)) {
            Some(&i) => self.cells[i].2 = to,
            None => {
                let prev = self.world.get_block(x, y, z);
                self.index.insert((x, y, z), self.cells.len());
                self.cells.push(((x, y, z), prev, to));
            }
        }
        self.world.set_block(x, y, z, to);
    }

    /// The cells whose block really changed, in the order first written.
    fn finish(self) -> Vec<crate::lighting::CellChange> {
        self.cells.into_iter().filter(|(_, prev, now)| prev != now).collect()
    }
}

/// Build Satoshi's wooden hut on `site`: clear the volume, fill any column
/// lower than the floor up to it with a cobblestone foundation, place the shared
/// [`hut_cells`], add glass windows on the three non-door walls + a roof torch
/// beacon. Idempotent in effect (writes the same blocks for a given site).
/// Returns every cell it changed (`lighting::CellChange`): the caller owns the
/// relight and the remesh, because a raw `World::set_block` does neither.
fn build_house(world: &mut World, site: &HouseSite) -> Vec<crate::lighting::CellChange> {
    let (ox, oy, oz) = (site.ox, site.oy, site.oz);
    let w = HUT_W as i32;
    let h = HUT_H as i32;
    let mut stamp = Stamp::new(world);
    // Clear the interior + walls volume so terrain doesn't intrude.
    for dy in 0..=h {
        for dz in 0..w {
            for dx in 0..w {
                stamp.set(ox + dx, oy + dy, oz + dz, block::AIR);
            }
        }
    }
    // A column lower than the floor stands on a foundation, so every floor cell
    // has solid ground under it.
    for dz in 0..w {
        for dx in 0..w {
            for y in (site.tops[dz as usize][dx as usize] + 1)..oy {
                stamp.set(ox + dx, y, oz + dz, block::COBBLESTONE);
            }
        }
    }
    // Place the structural hut (identical to the gifted plan).
    for c in hut_cells() {
        stamp.set(ox + c.rx as i32, oy + c.ry as i32, oz + c.rz as i32, c.block_id);
    }
    // Glass windows on the three non-door walls (mid-height) + a roof torch.
    let dc = HUT_W as i32 / 2;
    stamp.set(ox + dc, oy + 2, oz + w - 1, block::GLASS); // back wall
    stamp.set(ox, oy + 2, oz + dc, block::GLASS); // left wall
    stamp.set(ox + w - 1, oy + 2, oz + dc, block::GLASS); // right wall
    stamp.set(ox + dc, oy + h, oz + dc, block::TORCH); // beacon on the roof
    // Interior hearth-glow — a warm corner light so the hut reads "someone wise
    // lives here" from within (atmosphere; entities don't sample block-light, so
    // this lights the room, not Satoshi himself — his glow is his amulet).
    stamp.set(ox + 1, oy + 1, oz + 1, block::TORCH);
    stamp.finish()
}

/// Spawn the Satoshi villager entity + tag it. `VillagerComponent`
/// (`Profession::None`) is auto-attached by `entity::spawn_mob`.
pub fn spawn_satoshi(ecs: &mut hecs::World, pos: Vec3) -> hecs::Entity {
    let id = crate::entity::spawn_mob(ecs, crate::mob::MobType::Villager, pos);
    let _ = ecs.insert_one(id, SatoshiMarker);
    id
}

/// True if a Satoshi entity currently exists in the ECS.
fn satoshi_exists(ecs: &hecs::World) -> bool {
    ecs.query::<&SatoshiMarker>().iter().next().is_some()
}

/// Session-only tag (never persisted — the entity is re-spawned on load):
/// Satoshi has been summoned and is walking to the player. Without it he heads
/// back to his hut. Toggled by the summon key (H).
pub struct Summoned;

/// Walk speed toward the summon/dismiss target (blocks/tick; ≈2.4 blocks/sec).
const SUMMON_SPEED: f32 = 0.12;
/// How close he stops to the player when summoned (so he doesn't crowd them).
const SUMMON_STOP: f32 = 1.8;

/// Toggle Satoshi's summoned state. `Some(true)` = he's now coming to you,
/// `Some(false)` = he's heading home, `None` = there's no Satoshi in this world.
pub fn toggle_summon(ecs: &mut hecs::World) -> Option<bool> {
    let id = ecs.query::<&SatoshiMarker>().iter().next().map(|(id, _)| id)?;
    if ecs.get::<&Summoned>(id).is_ok() {
        let _ = ecs.remove_one::<Summoned>(id);
        Some(false)
    } else {
        let _ = ecs.insert_one(id, Summoned);
        Some(true)
    }
}

/// Move Satoshi a step toward the player (summoned) or back to his hut
/// (dismissed) this tick, snapping to the ground. Direct stepping rather than
/// mob physics, so he reads as walking without falling/sticking — call AFTER the
/// mob-AI + entity-physics passes so it's the last word on his position. He also
/// turns to face the target. No-op when Satoshi isn't enabled or spawned.
pub fn tick_summon(world: &World, ecs: &mut hecs::World, player_pos: Vec3) {
    if !world.satoshi.enabled {
        return;
    }
    let Some(id) = ecs.query::<&SatoshiMarker>().iter().next().map(|(id, _)| id) else {
        return;
    };
    if crate::combat::entity_is_dead(ecs, id) { return; }
    let summoned = ecs.get::<&Summoned>(id).is_ok();
    let (tx, tz) = if summoned {
        (player_pos.x, player_pos.z)
    } else {
        // Dismissed → walk back to his STAND spot (the house centre), NOT the
        // origin corner stored in `home`. The corner is a roofed log post, so
        // targeting it (and snapping to its highest block) would put him on the
        // roof. The centre is where he spawns.
        match world.satoshi.home {
            Some([hx, _, hz]) => {
                let (cx, cz) = centre_of(hx, hz);
                (cx as f32 + 0.5, cz as f32 + 0.5)
            }
            None => return,
        }
    };
    let cur = match ecs.get::<&crate::entity::Position>(id) {
        Ok(p) => p.0,
        Err(_) => return,
    };
    let (dx, dz) = (tx - cur.x, tz - cur.z);
    let dist = (dx * dx + dz * dz).sqrt();
    // Always face the target (so he looks at you even once he's arrived).
    if dist > 0.01
        && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(id) {
            ai.facing = dz.atan2(dx);
        }
    let stop = if summoned { SUMMON_STOP } else { 0.4 };
    if dist <= stop {
        return;
    }
    let step = SUMMON_SPEED.min(dist - stop);
    let nx = cur.x + dx / dist * step;
    let nz = cur.z + dz / dist * step;
    let (ncx, ncz) = (nx.floor() as i32, nz.floor() as i32);
    // Ground-snap. His hut roof spans the whole footprint, so a naive
    // highest-block probe inside the hut would stand him ON the roof — there,
    // use the known floor height; elsewhere follow the terrain surface.
    let ny = match world.satoshi.home {
        Some([hx, fy, hz])
            if (hx..hx + HUT_W as i32).contains(&ncx)
                && (hz..hz + HUT_W as i32).contains(&ncz) =>
        {
            fy as f32 + 1.0
        }
        _ => world
            .highest_block(ncx, ncz)
            .map(|(y, _)| y as f32 + 1.0)
            .unwrap_or(cur.y),
    };
    if let Ok(mut p) = ecs.get::<&mut crate::entity::Position>(id) {
        p.0 = Vec3::new(nx, ny, nz);
    }
}

/// Once-per-cadence spawn pass (call on the village tick). Places Satoshi in his
/// hut on first entry to an enabled world, then re-spawns the (non-persisted)
/// entity at his home on reload. Idempotent. Only acts in worlds explicitly
/// created to host him — never an existing world, never the Workshop.
///
/// A house not built yet goes on dry ground near the world spawn column
/// `spawn` ([`find_site`], keeping clear of `avoid`: the player's real start and
/// every live player's cell); one already built stays where `home` says. Returns
/// the cells the new house changed: the caller relights and remeshes them (and,
/// on a hosted world, tells the joiners). Empty on every other pass.
pub fn tick_spawn(
    world: &mut World,
    ecs: &mut hecs::World,
    spawn: (i32, i32),
    registry: &BlockRegistry,
    avoid: &[(i32, i32)],
    tick: u64,
) -> Vec<crate::lighting::CellChange> {
    if !world.satoshi.enabled || world.is_workshop {
        return Vec::new();
    }
    match world.satoshi.home {
        None => {
            // A search that found nothing is not repeated every second: only
            // after about 30 s, or once the loaded set has changed.
            let loaded_sections = reach_sections(world, (spawn.0 + HOUSE_OX, spawn.1 + HOUSE_OZ));
            if let Some(note) = &world.satoshi.no_site
                && tick.saturating_sub(note.tick) < NO_SITE_RETRY_TICKS
                && note.loaded_sections == loaded_sections
            {
                return Vec::new();
            }
            let site = match find_site(world, registry, spawn, avoid) {
                SiteSearch::Found(site) => site,
                // Ground not loaded yet: try again next cadence.
                SiteSearch::Waiting => return Vec::new(),
                // No place in reach: no house (he stays unplaced, as he does
                // while the spawn ground never loads).
                SiteSearch::NoSite => {
                    world.satoshi.no_site = Some(NoSiteNote { tick, loaded_sections });
                    return Vec::new();
                }
            };
            world.satoshi.no_site = None;
            let changes = build_house(world, &site);
            spawn_satoshi(ecs, satoshi_pos(site.ox, site.oy, site.oz));
            world.satoshi.home = Some([site.ox, site.oy, site.oz]);
            changes
        }
        Some([ox, floor_y, oz]) => {
            if satoshi_exists(ecs) {
                return Vec::new();
            }
            let (cx, cz) = centre_of(ox, oz);
            let cs = CHUNK_SIZE as i32;
            if !world.has_chunk(cx.div_euclid(cs), floor_y.div_euclid(cs), cz.div_euclid(cs)) {
                return Vec::new();
            }
            spawn_satoshi(ecs, satoshi_pos(ox, floor_y, oz));
            Vec::new()
        }
    }
}

// ── Authored dialogue corpus (100% scripted; scanned by the compliance test) ──
//
// COMPLIANCE: UK English, kid-safe, care-first. NEVER mentions sats, Bitcoin,
// earning, payouts, wallets, money, or rewards-as-money.

/// His name. (Contains "sat", but the compliance test matches whole words.)
pub const SATOSHI_NAME: &str = "Satoshi";

/// Warm sage greeting — care first (leads to the food gift). Obi-Wan-calm:
/// wise and gentle, never stuffy. Short (the onboarding minimal-reading rule).
pub const SATOSHI_GREETING: &str =
    "Ah — there you are. I had a feeling the wind would bring someone today. \
     You look weary from the road; let me see to that.";

/// Creative greeting — a fresh canvas (leads straight to the schematic).
pub const SATOSHI_CREATIVE_GREETING: &str =
    "Ah — a fresh world, and a maker's hands. Shall we begin?";

/// Said as he hands over the welcome food (Survival) — framed as a kindness.
pub const SATOSHI_GIFT_LINE: &str =
    "Take this. A small kindness for the road ahead.";

/// Said as he hands over the starter schematic (both modes).
pub const SATOSHI_SCHEMATIC_LINE: &str =
    "And this — a plan, old as these parts. Lay it down and build along; \
     in Survival you'll gather what it asks for as you go.";

/// Warm line when the player returns after the gifts (pull-on-demand).
pub const SATOSHI_RETURN_LINE: &str =
    "Back again? Good. My door is always open to those who seek it.";

/// The gardener's ambient lines (living world D17): what his greeting may say
/// when the player stands in a grey place or a blooming one. He never explains
/// and never lectures; he notices, the way a gardener does.
pub const SATOSHI_GARDENER_GREY: [&str; 2] = [
    "This place could do with some company.",
    "Nothing growing here, and yet it feels as if it's listening.",
];
pub const SATOSHI_GARDENER_BLOOMING: &str = "Listen to that. Somebody's been looking after it.";

/// Below this bloom level a place counts as grey for his ambient line.
pub const GARDENER_GREY_BELOW: f32 = 0.25;
/// Above this one it counts as blooming.
pub const GARDENER_BLOOMING_ABOVE: f32 = 0.8;

/// The ambient gardener line for a player standing where the bloom level is
/// `level`, or `None` in between. `pick` varies the grey line (any number; a
/// slowly changing one keeps a line steady while a dialogue is open).
pub fn gardener_line(level: f32, pick: u64) -> Option<&'static str> {
    if level < GARDENER_GREY_BELOW {
        Some(SATOSHI_GARDENER_GREY[(pick % SATOSHI_GARDENER_GREY.len() as u64) as usize])
    } else if level > GARDENER_BLOOMING_ABOVE {
        Some(SATOSHI_GARDENER_BLOOMING)
    } else {
        None
    }
}

/// Label on the food-accept action.
pub const SATOSHI_GIFT_LABEL: &str = "Thank you!";
/// Label on the schematic-accept action.
pub const SATOSHI_SCHEMATIC_LABEL: &str = "Take the plan";
/// Label on the always-present dismissal action.
pub const SATOSHI_DISMISS_LABEL: &str = "Thanks — I'll explore";

/// Test Lab — shown when the player has been through every mission Satoshi has
/// for this build. Warm + open-ended (he restocks each build).
pub const SATOSHI_MISSIONS_DONE: &str =
    "That is all I have for you for now — and you've done well. \
     Come and find me again when you're ready; there is always more to learn.";

/// The welcome food gift (a kindness, never a payout): a little bread.
pub fn welcome_food() -> crate::item::ItemStack {
    crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2)
}

/// Hand over the welcome food exactly once (Survival). Returns `true` if gifted
/// this call. Marks greeted + gifted.
pub fn give_welcome_gift(state: &mut SatoshiState, inv: &mut crate::inventory::Inventory) -> bool {
    state.has_greeted = true;
    if state.has_gifted {
        return false;
    }
    // Only burn the one-shot flag if the food actually landed. A full inventory
    // returns the leftover; setting `has_gifted` then would silently eat the
    // gift forever. Leave it unset so the player can take it after making room.
    if inv.add_item(welcome_food()).is_some() {
        return false;
    }
    state.has_gifted = true;
    true
}

/// Hand over the starter-hut schematic exactly once (both modes). Returns `true`
/// if gifted this call. Marks greeted + schematic_gifted.
pub fn give_schematic(state: &mut SatoshiState, inv: &mut crate::inventory::Inventory) -> bool {
    state.has_greeted = true;
    if state.schematic_gifted {
        return false;
    }
    // The plan is a UNIQUE item with no other source — never burn the one-shot
    // flag unless it actually landed. A full inventory hands the stack back; set
    // the flag only when nothing comes back, so the gift can be re-offered.
    let leftover = inv.add_item(crate::item::ItemStack {
        item: crate::item::Item::Plan(starter_hut_plan()),
        count: 1,
    });
    if leftover.is_some() {
        return false;
    }
    state.schematic_gifted = true;
    true
}

/// What the player picked in the Satoshi dialogue this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SatoshiAction {
    None,
    /// Accept the welcome food (Survival).
    TakeGift,
    /// Accept the starter schematic.
    TakeSchematic,
    /// "I'll explore" — dismiss (pull-only thereafter).
    Dismiss,
}

/// What the player picked in a Test Lab mission dialogue this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionAction {
    None,
    /// "Next one" → advance to the next mission.
    Skip,
    /// Close the dialogue (come back to Satoshi later).
    Close,
}

/// Which Satoshi panel to show, derived from state + mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SatoshiView {
    /// Survival first meeting: greeting + the food gift.
    Greeting,
    /// Offer the starter schematic (after food in Survival; straight away in Creative).
    SchematicOffer,
    /// Both gifts done: a warm return line, nothing pending.
    Idle,
}

/// Decide which panel to show. Survival greets with food first, then the
/// schematic; Creative goes straight to the schematic. Pure.
pub fn view_for(s: &SatoshiState, is_creative: bool) -> SatoshiView {
    if s.schematic_gifted {
        SatoshiView::Idle
    } else if !is_creative && !s.has_gifted {
        SatoshiView::Greeting
    } else {
        SatoshiView::SchematicOffer
    }
}

/// Whether Satoshi may *initiate* a gentle approach this tick — the anti-Navi
/// frequency budget. Pull-first: only while never-greeted and never-waved-off,
/// and never inside the cooldown. MVP keeps initiation off in practice; this is
/// the guard for any future cue.
#[cfg_attr(not(test), allow(dead_code))]
pub fn should_initiate(s: &SatoshiState, now_tick: u64) -> bool {
    if s.has_greeted || s.waved_off {
        return false;
    }
    s.last_initiated_tick == 0
        || now_tick.saturating_sub(s.last_initiated_tick) >= SATOSHI_INITIATE_COOLDOWN_TICKS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One spawn pass for a world whose spawn column is (0, 0): the house is
    /// first tried at (HOUSE_OX, HOUSE_OZ). Nobody to keep clear of, tick 0.
    fn spawn_pass(world: &mut World, ecs: &mut hecs::World) -> Vec<crate::lighting::CellChange> {
        tick_spawn(world, ecs, (0, 0), &BlockRegistry::new(), &[], 0)
    }

    /// `find_site` for a spawn column (0, 0), nobody to keep clear of.
    fn site_of(world: &World) -> SiteSearch {
        find_site(world, &BlockRegistry::new(), (0, 0), &[])
    }

    /// The block span a search from the first try reads (its reach, a footprint
    /// and a halo, and a little more): every chunk column in it must be held.
    fn reach_span() -> (std::ops::RangeInclusive<i32>, std::ops::RangeInclusive<i32>) {
        let more = 2;
        (
            (HOUSE_OX - SITE_REACH - more)..=(HOUSE_OX + SITE_REACH + HUT_W as i32 + more),
            (HOUSE_OZ - SITE_REACH - more)..=(HOUSE_OZ + SITE_REACH + HUT_W as i32 + more),
        )
    }

    /// A world configured to host Satoshi (as a new normal world would be) and
    /// flat grass at height `y` over the whole span a search reads, as a freshly
    /// generated world: nothing in it has been edited.
    fn flat_ground(world: &mut World, y: i32) {
        world.satoshi.enabled = true;
        let (xs, zs) = reach_span();
        for z in zs {
            for x in xs.clone() {
                world.set_block(x, y, z, block::GRASS);
            }
        }
        world.forget_edits();
    }

    /// Nothing a house stands on or in may be water, lava or a log, in any
    /// column of its footprint, from the ground to the roof.
    fn assert_footprint_is_dry_and_clear(world: &World, site: &HouseSite) {
        for dz in 0..HUT_W as usize {
            for dx in 0..HUT_W as usize {
                let (x, z) = (site.ox + dx as i32, site.oz + dz as i32);
                for y in site.tops[dz][dx]..=site.oy + HUT_H as i32 {
                    let b = world.get_block(x, y, z);
                    assert!(
                        !block::is_fluid(b) && !block::is_any_log_block(b),
                        "column ({x}, {z}) holds block {b} at y {y} (site {site:?})"
                    );
                }
            }
        }
    }

    /// Is block column `(x, z)` in the footprint of `site` or its one-block halo?
    fn in_halo(site: &HouseSite, (x, z): (i32, i32)) -> bool {
        (site.ox - 1..=site.ox + HUT_W as i32).contains(&x) && (site.oz - 1..=site.oz + HUT_W as i32).contains(&z)
    }

    /// Every authored, player-facing string — the compliance test scans these.
    fn authored_strings() -> Vec<&'static str> {
        vec![
            SATOSHI_NAME,
            SATOSHI_GREETING,
            SATOSHI_CREATIVE_GREETING,
            SATOSHI_GIFT_LINE,
            SATOSHI_SCHEMATIC_LINE,
            SATOSHI_RETURN_LINE,
            SATOSHI_GIFT_LABEL,
            SATOSHI_SCHEMATIC_LABEL,
            SATOSHI_DISMISS_LABEL,
            SATOSHI_MISSIONS_DONE,
            SATOSHI_GARDENER_GREY[0],
            SATOSHI_GARDENER_GREY[1],
            SATOSHI_GARDENER_BLOOMING,
        ]
    }

    #[test]
    fn the_gardener_speaks_only_in_a_grey_or_a_blooming_place() {
        assert!(gardener_line(0.1, 0).is_some());
        assert_ne!(gardener_line(0.1, 0), gardener_line(0.1, 1), "two grey lines to vary");
        assert_eq!(gardener_line(0.9, 3), Some(SATOSHI_GARDENER_BLOOMING));
        for mid in [0.25, 0.5, 0.8] {
            assert_eq!(gardener_line(mid, 0), None, "{mid}: in between he says nothing extra");
        }
    }

    #[test]
    fn toggle_summon_flips_and_reports_state() {
        let mut ecs = hecs::World::new();
        assert_eq!(toggle_summon(&mut ecs), None, "no Satoshi to summon → None");
        let id = spawn_satoshi(&mut ecs, Vec3::new(5.0, 64.0, 5.0));
        assert_eq!(toggle_summon(&mut ecs), Some(true), "first press summons");
        assert!(ecs.get::<&Summoned>(id).is_ok());
        assert_eq!(toggle_summon(&mut ecs), Some(false), "second press dismisses");
        assert!(ecs.get::<&Summoned>(id).is_err());
    }

    #[test]
    fn tick_summon_walks_toward_player_then_stops() {
        let mut world = World::new();
        flat_ground(&mut world, 64);
        world.satoshi.home = Some([HOUSE_OX, 65, HOUSE_OZ]);
        let mut ecs = hecs::World::new();
        let start = Vec3::new(HOUSE_OX as f32 + 0.5, 65.0, HOUSE_OZ as f32 + 0.5);
        let id = spawn_satoshi(&mut ecs, start);
        let _ = ecs.insert_one(id, Summoned);
        let player = Vec3::new(HOUSE_OX as f32 + 6.0, 65.0, HOUSE_OZ as f32 + 0.5);
        let before = ecs.get::<&crate::entity::Position>(id).unwrap().0.x;
        tick_summon(&world, &mut ecs, player);
        let after = ecs.get::<&crate::entity::Position>(id).unwrap().0.x;
        assert!(after > before, "a summoned Satoshi steps toward the player");
        // Many ticks → he settles ~SUMMON_STOP away, not on top of the player.
        for _ in 0..200 {
            tick_summon(&world, &mut ecs, player);
        }
        let p = ecs.get::<&crate::entity::Position>(id).unwrap().0;
        let d = ((p.x - player.x).powi(2) + (p.z - player.z).powi(2)).sqrt();
        assert!((d - SUMMON_STOP).abs() < 0.25, "stops ~{SUMMON_STOP} from player, got {d}");
    }

    #[test]
    fn dismissed_satoshi_returns_to_his_floor_not_his_roof() {
        // Regression: dismiss used to target the origin CORNER (a roofed log
        // post) and ground-snap via highest_block, which stood him ON the roof.
        // He must walk back to his centre stand spot, on his floor.
        let mut world = World::new();
        flat_ground(&mut world, 64);
        let mut ecs = hecs::World::new();
        spawn_pass(&mut world, &mut ecs); // builds the hut (roof + walls) + spawns him
        let id = ecs
            .query::<&SatoshiMarker>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        let floor_y = world.satoshi.home.unwrap()[1];
        // Summon him well outside the hut, then dismiss and let him walk home.
        let player = Vec3::new(HOUSE_OX as f32 + 14.0, floor_y as f32, HOUSE_OZ as f32 + 0.5);
        let _ = ecs.insert_one(id, Summoned);
        for _ in 0..600 {
            tick_summon(&world, &mut ecs, player);
        }
        let _ = ecs.remove_one::<Summoned>(id);
        for _ in 0..600 {
            tick_summon(&world, &mut ecs, player);
        }
        let p = ecs.get::<&crate::entity::Position>(id).unwrap().0;
        assert!(
            (p.y - (floor_y as f32 + 1.0)).abs() < 0.01,
            "stands on his floor (y={}), not the roof — got y={}",
            floor_y + 1,
            p.y
        );
        let (cx, cz) = centre_of(HOUSE_OX, HOUSE_OZ);
        let d = ((p.x - (cx as f32 + 0.5)).powi(2) + (p.z - (cz as f32 + 0.5)).powi(2)).sqrt();
        assert!(d < 0.5, "returns to his centre stand spot, got dist {d}");
    }

    #[test]
    fn gifts_are_not_lost_when_the_inventory_is_full() {
        // Regression: the one-shot gift flags were set even when add_item handed
        // the item back (full inventory), silently eating the gift — worst case
        // the UNIQUE starter plan. Flags must stay unset until the gift lands.
        let mut state = SatoshiState::default();
        // Fill every slot (add_item returns the leftover once it's full).
        let mut full = crate::inventory::Inventory::new();
        while full
            .add_item(crate::item::ItemStack::new_block(block::STONE, 64))
            .is_none()
        {}
        assert!(!give_welcome_gift(&mut state, &mut full), "full inv → not gifted");
        assert!(!state.has_gifted, "flag stays unset so it can be re-offered");
        assert!(!give_schematic(&mut state, &mut full), "full inv → plan not gifted");
        assert!(!state.schematic_gifted, "plan flag stays unset — it's unique");
        // A roomy inventory → the gifts land and the flags are set.
        let mut roomy = crate::inventory::Inventory::new();
        assert!(give_welcome_gift(&mut state, &mut roomy), "room → gifted");
        assert!(state.has_gifted);
        assert!(give_schematic(&mut state, &mut roomy), "room → plan gifted");
        assert!(state.schematic_gifted);
    }

    /// LOAD-BEARING compliance gate: Satoshi's currency is fun and care — never
    /// money or earning. Whole-word match so "Satoshi" passes while
    /// "sats"/"earn"/… are caught.
    #[test]
    fn corpus_has_no_money_or_earning_words() {
        // The shared list (`copy_lint::BANNED_MONEY_WORDS`) plus Satoshi's own
        // stricter extension: his currency is fun and care, so "reward" is out
        // of his mouth too (quest UI legitimately says "Reward:", so the
        // extension lives here, not in the shared list).
        let banned = crate::copy_lint::words_of(&[
            crate::copy_lint::BANNED_MONEY_WORDS,
            &["reward", "rewards", "rewarded"],
        ]);
        for s in authored_strings() {
            let hits = crate::copy_lint::banned_in(&s, &banned);
            assert!(
                hits.is_empty(),
                "Satoshi corpus must never use a money/earning word — found {hits:?} in {s:?}",
            );
        }
    }

    #[test]
    fn spawn_pass_places_exactly_one_satoshi_in_a_house() {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);

        spawn_pass(&mut world, &mut ecs);

        assert_eq!(
            ecs.query::<&SatoshiMarker>().iter().count(),
            1,
            "exactly one Satoshi after the first spawn pass"
        );
        assert_eq!(world.satoshi.home, Some([HOUSE_OX, 65, HOUSE_OZ]));
        let (cx, cz) = centre_of(HOUSE_OX, HOUSE_OZ);
        assert_eq!(world.get_block(cx, 65, cz), block::OAK_PLANKS, "house floor laid");
        assert_eq!(
            world.get_block(HOUSE_OX + 1, 66, HOUSE_OZ + 1),
            block::TORCH,
            "the hut is lit from within (interior hearth torch)"
        );

        spawn_pass(&mut world, &mut ecs);
        assert_eq!(
            ecs.query::<&SatoshiMarker>().iter().count(),
            1,
            "spawn pass is idempotent — still exactly one Satoshi"
        );
    }

    /// A world whose origin is open sea, with the columns round its real spawn
    /// column and the origin generated as a new world has them.
    fn sea_origin_world() -> (World, crate::biome::BiomeGenerator, u32) {
        let seed = crate::spawn_search::sea_origin_seed();
        let bg = crate::biome::BiomeGenerator::new(seed, crate::biome::Generator::V3);
        let mut world = World::new();
        let (sx, sz) = crate::chunk_stream::world_spawn_xz(&world, &bg);
        let (cx, cz) = (sx.div_euclid(16), sz.div_euclid(16));
        for dx in -3..=3 {
            for dz in -3..=3 {
                world.generate_column(cx + dx, cz + dz, &bg);
            }
        }
        // And every column a search from the origin reads (a v2 world's spawn).
        let (xs, zs) = search_columns((HOUSE_OX, HOUSE_OZ));
        for cx in xs {
            for cz in zs.clone() {
                world.generate_column(cx, cz, &bg);
            }
        }
        world.satoshi.enabled = true;
        (world, bg, seed)
    }

    /// Satoshi's house on a world whose origin is open sea: it goes near the
    /// world spawn, on dry ground, and every cell of its floor has solid,
    /// non-liquid ground under it, never water.
    #[test]
    fn a_house_on_a_sea_origin_world_stands_on_dry_ground_near_the_spawn() {
        let (mut world, bg, seed) = sea_origin_world();
        let mut ecs = hecs::World::new();
        let spawn = crate::chunk_stream::world_spawn_xz(&world, &bg);
        assert_ne!(spawn, (0, 0), "control: seed {seed} spawns away from its sea origin");
        let registry = BlockRegistry::new();
        let start = crate::chunk_stream::world_spawn_point(&world, &bg);
        let start = (start.x.floor() as i32, start.z.floor() as i32);
        let changes = tick_spawn(&mut world, &mut ecs, spawn, &registry, &[start], 0);
        assert!(!changes.is_empty(), "the house was built");
        let [ox, oy, oz] = world.satoshi.home.expect("the house was placed");
        assert!(
            (ox - spawn.0).abs() <= SITE_REACH + HOUSE_OX && (oz - spawn.1).abs() <= SITE_REACH + HOUSE_OZ,
            "seed {seed}: the house at {:?} is near the spawn {spawn:?}",
            [ox, oy, oz]
        );
        for dz in 0..HUT_W as i32 {
            for dx in 0..HUT_W as i32 {
                let under = world.get_block(ox + dx, oy - 1, oz + dz);
                assert!(
                    registry.is_solid(under) && !block::is_fluid(under),
                    "seed {seed}: floor cell ({dx},{dz}) of the house at {:?} sits on block {under}",
                    [ox, oy, oz]
                );
                assert_eq!(world.get_block(ox + dx, oy, oz + dz), block::OAK_PLANKS, "the floor is laid");
            }
        }
        assert!(satoshi_exists(&ecs), "Satoshi stands in it");
    }

    /// The old fixed site, a few blocks off the origin, is the sea: the pass
    /// must not build there. Every column the search reads round the origin is
    /// loaded, so the answer is conclusive, and it is asserted which one it is:
    /// either a site on dry ground (found, and then every floor cell stands on
    /// solid, non-liquid ground), or no site at all (nothing built, no Satoshi,
    /// the "no site" note kept). Never "ground not loaded" (that would make
    /// this test say nothing).
    #[test]
    fn a_sea_origin_world_gets_no_house_over_the_water() {
        let (mut world, bg, seed) = sea_origin_world();
        let registry = BlockRegistry::new();
        // The spawn passed is the origin, as a v2 world's.
        let found = find_site(&world, &registry, (0, 0), &[]);
        assert_ne!(found, SiteSearch::Waiting, "seed {seed}: every column the search reads is loaded");
        let mut ecs = hecs::World::new();
        let changes = tick_spawn(&mut world, &mut ecs, (0, 0), &registry, &[], 0);
        match found {
            SiteSearch::Found(site) => {
                assert!(!changes.is_empty() && satoshi_exists(&ecs), "seed {seed}: a site was found, so it is built");
                for dz in 0..HUT_W as i32 {
                    for dx in 0..HUT_W as i32 {
                        let under = world.get_block(site.ox + dx, site.oy - 1, site.oz + dz);
                        assert!(
                            registry.is_solid(under) && !block::is_fluid(under),
                            "seed {seed}: ({dx},{dz}) of the house at {site:?} sits on block {under}"
                        );
                    }
                }
            }
            SiteSearch::NoSite => {
                assert!(changes.is_empty() && !satoshi_exists(&ecs), "seed {seed}: no site: nothing built, no Satoshi");
                assert_eq!(world.satoshi.home, None);
                assert!(world.satoshi.no_site.is_some(), "and the search is noted, not repeated every second");
            }
            SiteSearch::Waiting => unreachable!(),
        }
        let _ = bg;
    }

    /// The changes `build_house` returns are exactly the cells that changed,
    /// every one of them (so none is left invisible), and each names the block
    /// it held and the block it holds now.
    #[test]
    fn the_stamp_reports_every_cell_it_changes() {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);
        // Rock under the grass, a hollow in the grass the foundation fills, and
        // a plant on the ground the clearing replaces.
        for z in (HOUSE_OZ - 2)..(HOUSE_OZ + HUT_W as i32 + 2) {
            for x in (HOUSE_OX - 2)..(HOUSE_OX + HUT_W as i32 + 2) {
                for y in 58..64 {
                    world.set_block(x, y, z, block::STONE);
                }
            }
        }
        world.set_block(HOUSE_OX + 4, 64, HOUSE_OZ + 4, block::AIR);
        world.set_block(HOUSE_OX + 1, 65, HOUSE_OZ + 3, block::TALL_GRASS);
        world.forget_edits();
        let region = |w: &World| {
            let mut m = std::collections::BTreeMap::new();
            for y in 60..75 {
                for z in (HOUSE_OZ - 3)..(HOUSE_OZ + HUT_W as i32 + 3) {
                    for x in (HOUSE_OX - 3)..(HOUSE_OX + HUT_W as i32 + 3) {
                        m.insert((x, y, z), w.get_block(x, y, z));
                    }
                }
            }
            m
        };
        let before = region(&world);
        let changes = spawn_pass(&mut world, &mut ecs);
        let after = region(&world);
        let changed: std::collections::BTreeSet<_> =
            before.iter().filter(|(p, b)| after[*p] != **b).map(|(p, _)| *p).collect();
        let reported: std::collections::BTreeSet<_> = changes.iter().map(|c| c.0).collect();
        assert_eq!(reported, changed, "the changes cover every cell the house changed, and no other");
        assert_eq!(reported.len(), changes.len(), "each cell once");
        for &(pos, prev, now) in &changes {
            assert_eq!(before[&pos], prev, "{pos:?}: the block it held");
            assert_eq!(after[&pos], now, "{pos:?}: the block it holds");
        }
        assert!(
            changes.iter().any(|&(p, _, now)| p == (HOUSE_OX + 4, 64, HOUSE_OZ + 4) && now == block::COBBLESTONE),
            "the hollow under the floor is filled"
        );
        // Every section the changes sit in is marked for remesh, and the rest
        // within one of them are queued (the light reaches them).
        let plan = crate::block_interact::runtime_edit_remesh(changes.iter().map(|c| c.0));
        for (pos, ..) in &changes {
            assert!(plan.now.contains(&World::block_to_chunk(pos.0, pos.1, pos.2)), "{pos:?}");
        }
        assert!(plan.later.is_disjoint(&plan.now));
        assert!(!plan.later.is_empty());
    }

    /// A house site over water, lava or a trunk is never taken: the first try
    /// is rejected, a site elsewhere is found (the reach is flat grass all
    /// round, so one exists), and no column of what is chosen holds water, lava
    /// or a log from the ground to the roof.
    #[test]
    fn a_house_is_never_sited_over_water_or_lava_or_trees() {
        let first_try = |site: &HouseSite| (site.ox, site.oz) == (HOUSE_OX, HOUSE_OZ);
        // A pond over the first try's footprint.
        let mut world = World::new();
        flat_ground(&mut world, 64);
        for z in HOUSE_OZ..HOUSE_OZ + HUT_W as i32 {
            for x in HOUSE_OX..HOUSE_OX + HUT_W as i32 {
                world.set_block(x, 65, z, block::WATER);
            }
        }
        world.forget_edits();
        let site = site_of(&world).site().expect("dry ground is in reach");
        assert!(!first_try(&site), "not over the pond: {site:?}");
        assert_footprint_is_dry_and_clear(&world, &site);
        // Lava is no ground either: one lava cell in the first try's footprint.
        let mut lava = World::new();
        flat_ground(&mut lava, 64);
        lava.set_block(HOUSE_OX + 2, 65, HOUSE_OZ + 2, block::LAVA);
        lava.forget_edits();
        let site = site_of(&lava).site().expect("dry ground is in reach");
        assert!(!first_try(&site), "not over the lava: {site:?}");
        assert_footprint_is_dry_and_clear(&lava, &site);
        // A tree in the footprint: the first pass does not take it.
        let mut wood = World::new();
        flat_ground(&mut wood, 64);
        wood.set_block(HOUSE_OX + 1, 65, HOUSE_OZ + 1, block::OAK_LOG);
        wood.forget_edits();
        let site = site_of(&wood).site().expect("clear ground is in reach");
        assert!(!first_try(&site), "not through the trunk: {site:?}");
        assert_footprint_is_dry_and_clear(&wood, &site);
    }

    /// Satoshi's house never goes over a player's work. A block a player placed
    /// in the first try's footprint moves the house; one in its halo does too;
    /// the stamp's whole volume and a one-block margin are protected.
    #[test]
    fn a_house_is_never_built_over_a_block_a_player_placed() {
        for (what, cell) in [
            ("in the footprint", (HOUSE_OX + 2, 65, HOUSE_OZ + 2)),
            ("on the ground of the footprint", (HOUSE_OX + 4, 64, HOUSE_OZ)),
            ("in the halo", (HOUSE_OX - 1, 65, HOUSE_OZ + 2)),
            ("high in the volume", (HOUSE_OX + 2, 65 + HUT_H as i32, HOUSE_OZ + 2)),
        ] {
            let mut world = World::new();
            flat_ground(&mut world, 64);
            world.place_player_block(cell.0, cell.1, cell.2, block::STONE);
            world.forget_edits(); // only the placed bit is left to see
            assert!(world.is_placed(cell.0, cell.1, cell.2), "control");
            let site = site_of(&world).site().expect("a clear site is in reach");
            assert_ne!((site.ox, site.oz), (HOUSE_OX, HOUSE_OZ), "{what}: not the first try");
            assert!(!in_halo(&site, (cell.0, cell.2)), "{what}: the house at {site:?} covers the placed block");
            let mut ecs = hecs::World::new();
            spawn_pass(&mut world, &mut ecs);
            assert!(world.is_placed(cell.0, cell.1, cell.2));
            assert_ne!(world.get_block(cell.0, cell.1, cell.2), block::AIR, "{what}: the player's block is still there");
        }
    }

    /// A block entity (a chest) is player work too, whether or not anything
    /// marked its section.
    #[test]
    fn a_house_is_never_built_over_a_block_entity() {
        let mut world = World::new();
        flat_ground(&mut world, 64);
        let cell = (HOUSE_OX + 3, 65, HOUSE_OZ + 1);
        world.set_block(cell.0, cell.1, cell.2, block::CHEST);
        world.insert_chest(cell, Default::default());
        world.forget_edits();
        let site = site_of(&world).site().expect("a clear site is in reach");
        assert!(!in_halo(&site, (cell.0, cell.2)), "{site:?} covers the chest");
    }

    /// A section edited since generation is not built in at all: whatever is in
    /// it (a field, torches), the stamp would wreck it. A block high above the
    /// house in the same section is an edit, and the house goes elsewhere.
    #[test]
    fn a_house_is_never_built_in_a_chunk_that_was_edited() {
        let mut world = World::new();
        flat_ground(&mut world, 64);
        // One edit, in the section the first try is in, above the house.
        let edit = (HOUSE_OX + 2, 75, HOUSE_OZ + 2);
        world.set_block(edit.0, edit.1, edit.2, block::TORCH); // not ground: only the edit is in the way
        let (cx, cy, cz) = World::block_to_chunk(edit.0, edit.1, edit.2);
        assert!(world.section_edited(cx, cy, cz), "control: the edit marks its section");
        let site = site_of(&world).site().expect("a site in an untouched section is in reach");
        // Not one block column of the house or its halo is in the edited chunk's
        // columns (its section spans the heights the house uses).
        let (x0, z0) = (cx * 16, cz * 16);
        let overlaps = (site.ox - 1..=site.ox + HUT_W as i32).any(|x| (x0..x0 + 16).contains(&x))
            && (site.oz - 1..=site.oz + HUT_W as i32).any(|z| (z0..z0 + 16).contains(&z));
        assert!(!overlaps, "the house at {site:?} stands in the edited chunk");
        // A section of the same column that the house does not reach (high up)
        // is no reason to refuse: the volume is what counts.
        let mut other = World::new();
        flat_ground(&mut other, 64);
        other.set_block(HOUSE_OX + 2, 200, HOUSE_OZ + 2, block::TORCH);
        assert_eq!(site_of(&other).site().map(|s| (s.ox, s.oz)), Some((HOUSE_OX, HOUSE_OZ)));
    }

    /// With every section in reach edited there is nowhere to build: no house,
    /// no Satoshi, and the search is not repeated every second: it waits about
    /// 30 s, or for the loaded set to change.
    #[test]
    fn with_no_acceptable_site_nothing_is_built_and_the_search_is_not_repeated() {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        flat_ground(&mut world, 64);
        // One edit in the house's height band of every chunk column in reach.
        for cx in -2..=2 {
            for cz in -2..=2 {
                world.set_block(cx * 16 + 1, 70, cz * 16 + 1, block::TORCH); // not ground: only the edit is in the way
            }
        }
        let mut ecs = hecs::World::new();
        let mut pass = |world: &mut World, tick: u64| tick_spawn(world, &mut ecs, (0, 0), &registry, &[], tick);
        assert!(pass(&mut world, 100).is_empty());
        assert_eq!(world.satoshi.home, None, "no house when no site is acceptable");
        assert_eq!(world.satoshi.no_site.as_ref().map(|n| n.tick), Some(100), "noted");
        // Inside the wait, with the same ground: not searched again.
        pass(&mut world, 100 + NO_SITE_RETRY_TICKS - 1);
        assert_eq!(world.satoshi.no_site.as_ref().map(|n| n.tick), Some(100));
        // After it: searched again (and noted again).
        pass(&mut world, 100 + NO_SITE_RETRY_TICKS);
        assert_eq!(world.satoshi.no_site.as_ref().map(|n| n.tick), Some(100 + NO_SITE_RETRY_TICKS));
        // The loaded set changing inside the wait: searched at once.
        let t = 100 + NO_SITE_RETRY_TICKS + 5;
        world.set_block(HOUSE_OX, 140, HOUSE_OZ, block::TORCH); // a new section in reach
        pass(&mut world, t);
        assert_eq!(world.satoshi.no_site.as_ref().map(|n| n.tick), Some(t), "the loaded set changed: searched again");
        // The note is session-only: never in a save.
        let bytes = bincode::serialize(&world.satoshi).expect("serialize");
        let back: SatoshiState = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.no_site, None);
    }

    /// A site never covers the player's real start or a live player, halo
    /// included, however far from the spawn column that is.
    #[test]
    fn a_house_keeps_clear_of_the_start_and_of_every_player() {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        flat_ground(&mut world, 64);
        for cell in [(HOUSE_OX + 2, HOUSE_OZ + 2), (HOUSE_OX - 1, HOUSE_OZ - 1), (HOUSE_OX + 5, HOUSE_OZ + 5)] {
            let site = find_site(&world, &registry, (0, 0), &[cell]).site().expect("a site in reach");
            assert!(!in_halo(&site, cell), "{cell:?}: the house at {site:?} covers it");
            assert_ne!((site.ox, site.oz), (HOUSE_OX, HOUSE_OZ), "{cell:?}: the first try is covered");
        }
        // Several at once.
        let cells = [(HOUSE_OX + 2, HOUSE_OZ + 2), (HOUSE_OX + 9, HOUSE_OZ + 2), (HOUSE_OX + 2, HOUSE_OZ + 9)];
        let site = find_site(&world, &registry, (0, 0), &cells).site().expect("a site in reach");
        assert!(cells.iter().all(|&c| !in_halo(&site, c)), "{site:?}");
    }

    /// The site depends on the world and its edits, not on which columns have
    /// streamed in: a column the search reads that is not loaded means no
    /// answer yet; once all are loaded the answer is the same whether or not
    /// columns beyond the reach are loaded as well, or in what order.
    #[test]
    fn the_site_waits_for_every_column_it_reads_and_then_does_not_depend_on_more() {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        world.satoshi.enabled = true;
        let (xs, zs) = reach_span();
        let missing = (32..48, 32..48); // the chunk column (2, 2), inside the reach
        for z in zs.clone() {
            for x in xs.clone() {
                if !(missing.0.contains(&x) && missing.1.contains(&z)) {
                    world.set_block(x, 64, z, block::GRASS);
                }
            }
        }
        world.forget_edits();
        assert_eq!(site_of(&world), SiteSearch::Waiting, "a column in reach is missing");
        let mut ecs = hecs::World::new();
        assert!(spawn_pass(&mut world, &mut ecs).is_empty(), "nothing built while it waits");
        assert_eq!(world.satoshi.no_site, None, "and waiting is not noted as no site");
        world.set_block(40, 64, 40, block::GRASS);
        world.forget_edits();
        let a = site_of(&world).site().expect("every column is loaded now");
        // The same ground, built in the opposite order and with columns far
        // beyond the reach as well.
        let mut other = World::new();
        other.satoshi.enabled = true;
        for z in zs.rev() {
            for x in xs.clone().rev() {
                if !(missing.0.contains(&x) && missing.1.contains(&z)) {
                    other.set_block(x, 64, z, block::GRASS);
                }
            }
        }
        other.set_block(40, 64, 40, block::GRASS);
        for far in [(120, 120), (-200, 40), (300, -300)] {
            other.set_block(far.0, 64, far.1, block::GRASS);
        }
        other.forget_edits();
        assert_eq!(site_of(&other).site(), Some(a), "the same world gives the same house");
    }

    /// A forest does not leave Satoshi without a house: with a canopy over the
    /// whole reach the last pass looks through it to the ground, and the stamp
    /// clears the footprint's volume.
    #[test]
    fn a_house_is_still_built_under_a_canopy() {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);
        // Two leaf layers over the whole reach, one under the roof line.
        let (xs, zs) = reach_span();
        for z in zs {
            for x in xs.clone() {
                world.set_block(x, 68, z, block::OAK_LEAVES);
                world.set_block(x, 66, z, block::OAK_LEAVES);
            }
        }
        // A trunk standing in the footprint.
        world.set_block(HOUSE_OX + 2, 65, HOUSE_OZ + 2, block::OAK_LOG);
        world.forget_edits();
        assert!(site_of(&world).site().is_some(), "found through the canopy");
        let changes = spawn_pass(&mut world, &mut ecs);
        assert!(!changes.is_empty() && satoshi_exists(&ecs), "the house is built");
        let [ox, oy, oz] = world.satoshi.home.expect("placed");
        for dz in 0..HUT_W as i32 {
            for dx in 0..HUT_W as i32 {
                let under = world.get_block(ox + dx, oy - 1, oz + dz);
                assert!(registry.is_solid(under) && !block::is_fluid(under), "floor ({dx},{dz}) on {under}");
            }
        }
        assert_eq!(world.get_block(HOUSE_OX + 2, 65, HOUSE_OZ + 2), block::OAK_PLANKS, "the trunk is gone");
    }

    /// A house already built stays where it is, whatever the world spawn says,
    /// and an old save's home (the fixed origin offset) keeps working.
    #[test]
    fn a_house_already_built_stays_where_it_was() {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);
        spawn_pass(&mut world, &mut ecs);
        let home = world.satoshi.home;
        assert_eq!(home, Some([HOUSE_OX, 65, HOUSE_OZ]));
        ecs = hecs::World::new();
        // The world spawn is somewhere else entirely now: the entity returns home.
        let changes = tick_spawn(&mut world, &mut ecs, (500, -500), &BlockRegistry::new(), &[], 0);
        assert!(changes.is_empty(), "nothing is rebuilt");
        assert_eq!(world.satoshi.home, home);
        assert!(satoshi_exists(&ecs));
    }

    #[test]
    fn respawns_at_home_after_a_reload_drops_the_entity() {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);
        spawn_pass(&mut world, &mut ecs);
        ecs = hecs::World::new(); // simulate a reload: entity gone, `home` kept
        assert!(!satoshi_exists(&ecs));

        spawn_pass(&mut world, &mut ecs);
        assert!(satoshi_exists(&ecs), "Satoshi re-spawns at his home on reload");
    }

    #[test]
    fn disabled_world_never_spawns_satoshi() {
        // An existing/flat/etc. world (enabled=false) must never get Satoshi.
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        for z in 0..12 {
            for x in 0..12 {
                world.set_block(x, 64, z, block::GRASS);
            }
        }
        // NOTE: enabled deliberately left false.
        spawn_pass(&mut world, &mut ecs);
        assert!(!satoshi_exists(&ecs), "no Satoshi in a world not created to host him");
        assert_eq!(world.satoshi.home, None);
    }

    #[test]
    fn skips_the_workshop_world() {
        let mut world = World::new();
        world.is_workshop = true;
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64); // sets enabled=true
        spawn_pass(&mut world, &mut ecs);
        assert!(!satoshi_exists(&ecs), "no Satoshi in the Workshop editor");
        assert_eq!(world.satoshi.home, None);
    }

    #[test]
    fn welcome_gift_lands_exactly_once() {
        use crate::item::MaterialId;
        let mut state = SatoshiState::default();
        let mut inv = crate::inventory::Inventory::new();
        assert!(give_welcome_gift(&mut state, &mut inv));
        let after = inv.count_material(MaterialId::Bread);
        assert!(after > 0 && state.has_greeted && state.has_gifted);
        assert!(!give_welcome_gift(&mut state, &mut inv), "no double food gift");
        assert_eq!(inv.count_material(MaterialId::Bread), after);
    }

    #[test]
    fn schematic_gift_lands_exactly_once() {
        let mut state = SatoshiState::default();
        let mut inv = crate::inventory::Inventory::new();
        assert!(give_schematic(&mut state, &mut inv));
        assert!(state.schematic_gifted);
        let plans = inv
            .slots_iter()
            .flatten()
            .filter(|s| matches!(s.item, crate::item::Item::Plan(_)))
            .count();
        assert_eq!(plans, 1, "exactly one starter-hut plan gifted");
        assert!(!give_schematic(&mut state, &mut inv), "no double schematic gift");
    }

    #[test]
    fn starter_plan_matches_the_hut_and_is_buildable() {
        let plan = starter_hut_plan();
        assert_eq!(plan.width, HUT_W);
        assert_eq!(plan.height, HUT_H);
        assert_eq!(plan.cells, hut_cells(), "gifted plan == the hut he lives in");
        assert!(!plan.cells.is_empty());
    }

    #[test]
    fn view_flows_survival_food_then_schematic_creative_straight() {
        let mut s = SatoshiState::default();
        // Survival: food first, then schematic, then idle.
        assert_eq!(view_for(&s, false), SatoshiView::Greeting);
        s.has_gifted = true;
        assert_eq!(view_for(&s, false), SatoshiView::SchematicOffer);
        s.schematic_gifted = true;
        assert_eq!(view_for(&s, false), SatoshiView::Idle);
        // Creative: straight to the schematic (no food), then idle.
        let mut c = SatoshiState::default();
        assert_eq!(view_for(&c, true), SatoshiView::SchematicOffer);
        c.schematic_gifted = true;
        assert_eq!(view_for(&c, true), SatoshiView::Idle);
    }

    #[test]
    fn frequency_budget_goes_pull_only_after_wave_off() {
        let s = SatoshiState::default();
        assert!(should_initiate(&s, 0));
        let waved = SatoshiState { waved_off: true, ..Default::default() };
        assert!(!should_initiate(&waved, 1_000_000));
        let greeted = SatoshiState { has_greeted: true, ..Default::default() };
        assert!(!should_initiate(&greeted, 1_000_000));
        let cooled = SatoshiState { last_initiated_tick: 1_000, ..Default::default() };
        assert!(!should_initiate(&cooled, 1_000 + SATOSHI_INITIATE_COOLDOWN_TICKS - 1));
        assert!(should_initiate(&cooled, 1_000 + SATOSHI_INITIATE_COOLDOWN_TICKS));
    }
}
