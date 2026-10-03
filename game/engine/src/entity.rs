//! Entity system — hecs components, physics tick, and cube vertex generation.

use glam::Vec3;
use crate::block::BlockRegistry;
use crate::mob::{self, MobType};
use crate::world::World;

// --- ECS Components ---

/// Entity foot position (bottom-centre of hitbox).
pub struct Position(pub Vec3);

/// Entity velocity in blocks/tick.
pub struct Velocity(pub Vec3);

/// What kind of mob this is.
pub struct MobKind(pub MobType);

/// AABB hitbox dimensions. Depth equals width.
pub struct Hitbox {
    pub width: f32,
    pub height: f32,
}

/// Whether the entity is standing on solid ground.
pub struct OnGround(pub bool);

/// Stable network-protocol id assigned by the server the first time an
/// entity is broadcast. Clients key their render-only ECS off this.
/// Absent on server-side entities that have not yet been broadcast
/// and on client-side entities that were spawned locally (dual-sim).
pub struct ProtocolId(pub u32);

/// A dropped item floating in the world. Combines with Position + Velocity +
/// Hitbox + OnGround so the existing physics path (`tick_entities`) handles
/// gravity + ground-collision for free.
pub struct ItemEntity {
    pub stack: crate::item::ItemStack,
    /// Ticks until pickup-eligible. Used to give a small grace period so the
    /// player can't instantly re-pick a freshly-spawned drop while still on top
    /// of the kill site.
    pub pickup_delay: u32,
    /// If `Some(idx)`, only player `idx` is blocked from picking this up
    /// during `pickup_delay`; other players can pick it up instantly. Set by
    /// Q-drop (Spec 5 §3.6). `None` for natural drops (mining, mob death) —
    /// those block every player for the delay window.
    pub dropper: Option<u8>,
}

/// Generic transient-entity timer. Ticks down each frame; hits 0 → despawn.
pub struct Lifetime(pub u32);

/// A projectile in flight (Wave 23 — arrows fired from a bow; Rubber
/// 2026-05-23 added rubber balls from a slingshot via `is_blunt`).
/// Carries the damage to apply to the first mob hit. Marker pattern
/// lets the projectile-tick loop find them without overlapping with
/// mob queries.
pub struct ProjectileEntity {
    pub damage: f32,
    /// Rubber feature — true for slingshot rubber-ball projectiles.
    /// On hit, combat consults `slingshot::is_stunnable` for an extra
    /// stun effect alongside the damage application. Arrows leave
    /// this `false`.
    pub is_blunt: bool,
    /// Player index that fired this projectile, if any. On a mob hit
    /// the projectile-tick stamps `LastAttacker(owner)` so kill
    /// attribution (bounties, kill_counter) credits the shooter rather
    /// than falling back to the nearest-living-player proximity guess.
    /// `None` for mob-fired or test projectiles.
    pub owner: Option<usize>,
}

/// Arrow flight constants.
pub const ARROW_HITBOX: f32 = 0.12;
pub const ARROW_HEIGHT: f32 = 0.12;
/// Arrow lifetime: 5 seconds at 20 TPS.
pub const ARROW_LIFETIME_TICKS: u32 = 100;
/// Per-tick gravity on an arrow. Less than mob gravity (0.08) so the
/// arc is gentle.
pub const ARROW_GRAVITY: f32 = 0.025;
/// Damage dealt to the first mob hit.
pub const ARROW_DAMAGE: f32 = 4.0;
/// Initial speed of a fired arrow (blocks/tick — 0.75 = 15 blocks/sec).
pub const ARROW_INITIAL_SPEED: f32 = 0.85;

/// Item entities live for 5 minutes (Minecraft parity) before despawning.
pub const ITEM_LIFETIME_TICKS: u32 = 20 * 5 * 60;
/// Brief grace before pickup so the kill-and-pick-up moment feels deliberate.
/// Natural drops (mining, mob death) block every player during this window.
pub const ITEM_PICKUP_DELAY_TICKS: u32 = 10; // 0.5s
/// Longer grace for Q-drops so the dropper doesn't instantly vacuum the item
/// back up. Only the dropper is blocked; other players pick up immediately.
pub const ITEM_DROP_PICKUP_DELAY_TICKS: u32 = 30; // 1.5s
/// Item AABB — small and squat. Renders as a tiny floating cube.
pub const ITEM_HITBOX: f32 = 0.25;
pub const ITEM_HEIGHT: f32 = 0.25;
/// Player must be within this radius (blocks) to pick up.
pub const ITEM_PICKUP_RADIUS: f32 = 1.4;
/// Items within this radius (and outside pickup radius) drift toward the player.
pub const ITEM_MAGNET_RADIUS: f32 = 2.4;
/// Per-tick pull on a magnetised item.
pub const ITEM_MAGNET_PULL: f32 = 0.16;

// --- Physics ---

const GRAVITY: f32 = 0.08;
const DRAG: f32 = 0.91;
/// Net upward lift (blocks/tick²) applied to a submerged entity on top of
/// cancelling gravity, so a mob knocked into water rises to the surface and
/// swims out rather than drowning at the lakebed (2026-05-30 playtest).
const BUOYANCY: f32 = 0.04;
/// Cap the rise speed so the float-up is a gentle bob, not a launch.
const MAX_RISE: f32 = 0.12;

/// Run one physics tick for all entities. Call at 20 TPS.
pub fn tick_entities(ecs: &mut hecs::World, world: &World, registry: &BlockRegistry) {
    for (_id, (pos, vel, hitbox, on_ground, flying)) in ecs
        .query_mut::<(
            &mut Position,
            &mut Velocity,
            &Hitbox,
            &mut OnGround,
            Option<&Flying>,
        )>()
    {
        // Flyers (bees) are gravity-exempt — their species dispatcher owns the
        // full velocity vector (incl. the vertical hover-bob). Block collision
        // below still applies, so they don't pass through walls. Everything
        // else falls + floats normally.
        let is_flying = flying.is_some();

        // Gravity
        if !is_flying {
            vel.0.y -= GRAVITY;
        }

        // Drag
        vel.0 *= DRAG;

        // Buoyancy: a submerged entity floats up to the surface instead of
        // sinking. Cancel the gravity we just applied and add a little net
        // lift, capped so it's a gentle rise. Lets a mob dragged into water
        // swim out (2026-05-30 playtest). Checked against the foot cell.
        if !is_flying
            && world.is_water(
                pos.0.x.floor() as i32,
                pos.0.y.floor() as i32,
                pos.0.z.floor() as i32,
            )
        {
            vel.0.y += GRAVITY + BUOYANCY;
            if vel.0.y > MAX_RISE {
                vel.0.y = MAX_RISE;
            }
            // Flowing water carries entities downstream (items ride streams,
            // mobs get swept) — the same current the player push and the
            // future water wheel read.
            if let Some(f) = crate::water::flow_vector(
                world,
                pos.0.x.floor() as i32,
                pos.0.y.floor() as i32,
                pos.0.z.floor() as i32,
            ) {
                vel.0.x += f[0] * crate::water::FLOW_PUSH;
                vel.0.z += f[2] * crate::water::FLOW_PUSH;
            }
        }

        // Move
        pos.0 += vel.0;

        let half_w = hitbox.width / 2.0;

        // --- Y-axis collision (gravity/falling) ---
        on_ground.0 = false;
        {
            let min_x = (pos.0.x - half_w).floor() as i32;
            let max_x = (pos.0.x + half_w).floor() as i32;
            let min_z = (pos.0.z - half_w).floor() as i32;
            let max_z = (pos.0.z + half_w).floor() as i32;
            let foot_y = pos.0.y.floor() as i32;

            if vel.0.y <= 0.0 {
                for bx in min_x..=max_x {
                    for bz in min_z..=max_z {
                        if world.is_solid(bx, foot_y, bz, registry) {
                            let block_top = foot_y as f32 + 1.0;
                            if pos.0.y < block_top {
                                pos.0.y = block_top;
                                vel.0.y = 0.0;
                                on_ground.0 = true;
                            }
                        }
                    }
                }
            }
        }

        // --- X-axis collision with 1-block step-up ---
        if vel.0.x.abs() > 0.0001 {
            let foot_y = pos.0.y.floor() as i32;
            let head_y = (pos.0.y + hitbox.height).floor() as i32;
            let min_z = (pos.0.z - half_w).floor() as i32;
            let max_z = (pos.0.z + half_w).floor() as i32;
            let edge_x = if vel.0.x > 0.0 {
                (pos.0.x + half_w).floor() as i32
            } else {
                (pos.0.x - half_w).floor() as i32
            };

            let mut blocked = false;
            for by in foot_y..=head_y {
                for bz in min_z..=max_z {
                    if world.is_solid(edge_x, by, bz, registry) {
                        blocked = true;
                        break;
                    }
                }
            }

            if blocked {
                // Try step-up: is there a 1-block ledge we can walk onto?
                let step_clear = foot_y < head_y && {
                    let mut clear = true;
                    for by in (foot_y + 1)..=(head_y + 1) {
                        for bz in min_z..=max_z {
                            if world.is_solid(edge_x, by, bz, registry) {
                                clear = false;
                                break;
                            }
                        }
                    }
                    clear
                };

                if step_clear && world.is_solid(edge_x, foot_y, min_z, registry) {
                    // Step up
                    pos.0.y = foot_y as f32 + 1.0;
                    on_ground.0 = true;
                } else {
                    // Can't pass — push back
                    if vel.0.x > 0.0 {
                        pos.0.x = edge_x as f32 - half_w;
                    } else {
                        pos.0.x = edge_x as f32 + 1.0 + half_w;
                    }
                    vel.0.x = 0.0;
                }
            }
        }

        // --- Z-axis collision with 1-block step-up ---
        if vel.0.z.abs() > 0.0001 {
            let foot_y = pos.0.y.floor() as i32;
            let head_y = (pos.0.y + hitbox.height).floor() as i32;
            let min_x = (pos.0.x - half_w).floor() as i32;
            let max_x = (pos.0.x + half_w).floor() as i32;
            let edge_z = if vel.0.z > 0.0 {
                (pos.0.z + half_w).floor() as i32
            } else {
                (pos.0.z - half_w).floor() as i32
            };

            let mut blocked = false;
            for by in foot_y..=head_y {
                for bx in min_x..=max_x {
                    if world.is_solid(bx, by, edge_z, registry) {
                        blocked = true;
                        break;
                    }
                }
            }

            if blocked {
                let step_clear = foot_y < head_y && {
                    let mut clear = true;
                    for by in (foot_y + 1)..=(head_y + 1) {
                        for bx in min_x..=max_x {
                            if world.is_solid(bx, by, edge_z, registry) {
                                clear = false;
                                break;
                            }
                        }
                    }
                    clear
                };

                if step_clear && world.is_solid(min_x, foot_y, edge_z, registry) {
                    pos.0.y = foot_y as f32 + 1.0;
                    on_ground.0 = true;
                } else {
                    if vel.0.z > 0.0 {
                        pos.0.z = edge_z as f32 - half_w;
                    } else {
                        pos.0.z = edge_z as f32 + 1.0 + half_w;
                    }
                    vel.0.z = 0.0;
                }
            }
        }

        // Prevent falling through the void
        if pos.0.y < -64.0 {
            pos.0.y = 80.0;
            vel.0 = Vec3::ZERO;
        }
    }
}

// --- Player-entity collision ---

/// Push player out of entity hitboxes. Call after player physics each tick.
pub fn push_player_from_entities(
    ecs: &hecs::World,
    player_pos: &mut Vec3,
    player_vel: &mut Vec3,
) {
    let player_hw = 0.3; // Player half-width

    for (_id, (pos, hitbox)) in ecs.query::<(&Position, &Hitbox)>().iter() {
        let mob_hw = hitbox.width / 2.0;
        let combined_hw = player_hw + mob_hw;

        let dx = player_pos.x - pos.0.x;
        let dz = player_pos.z - pos.0.z;

        // Check vertical overlap (player feet to head vs mob feet to top)
        let player_top = player_pos.y + 1.8;
        let mob_top = pos.0.y + hitbox.height;
        if player_pos.y >= mob_top || player_top <= pos.0.y {
            continue; // No vertical overlap
        }

        // Check horizontal overlap (AABB)
        let overlap_x = combined_hw - dx.abs();
        let overlap_z = combined_hw - dz.abs();

        if overlap_x > 0.0 && overlap_z > 0.0 {
            // Push apart on the axis with least overlap
            if overlap_x < overlap_z {
                let sign = if dx > 0.0 { 1.0 } else { -1.0 };
                player_pos.x += sign * overlap_x;
                player_vel.x = 0.0;
            } else {
                let sign = if dz > 0.0 { 1.0 } else { -1.0 };
                player_pos.z += sign * overlap_z;
                player_vel.z = 0.0;
            }
        }
    }
}

// --- Spawning ---

/// Spawn a mob entity in the ECS world. Returns the spawned entity id
/// so callers (HP-3 hideout spawner, test helpers, …) can attach
/// additional components like `BrigandTier` / `HomeHideout` without an
/// extra query. Pre-HP-3 callers safely discard the return.
pub fn spawn_mob(ecs: &mut hecs::World, kind: MobType, position: Vec3) -> hecs::Entity {
    let def = mob::mob_def(kind);
    let id = ecs.spawn((
        Position(position),
        Velocity(Vec3::ZERO),
        MobKind(kind),
        Hitbox {
            width: def.width,
            height: def.height,
        },
        OnGround(false),
        crate::mob_ai::MobAi::new(),
        crate::combat::Health::new(def.health as f32),
    ));
    // Villager-family mobs carry an extra `VillagerComponent` for profession
    // + workstation claim (Spec 19 phase 3). Iron Golems intentionally
    // excluded — golems don't work, they guard.
    if crate::villager::is_villager_kind(kind) {
        let _ = ecs.insert_one(id, crate::villager::VillagerComponent::default());
    }
    // Spec 28d.nostrich v2 — every Nostrich carries a NostrichData
    // component with ownership + AI state + lay/feather timers.
    // Replaces the v1 global tick-stagger; per-entity timers are
    // tame-aware and survive across the entity's lifetime.
    if kind == MobType::Nostrich {
        let _ = ecs.insert_one(id, crate::nostrich::NostrichData::untamed());
    }
    // Spec 28d.wolves R5 (2026-05-28) — every Wolf carries a WolfData
    // component, lifting the R2/R3-era BRIDGE that wolves had no
    // queryable tame state in the ECS. Untamed at spawn time; the
    // bone-feed right-click path mutates it via `wolf::attempt_tame`.
    if kind == MobType::Wolf {
        let _ = ecs.insert_one(id, crate::wolf::WolfData::untamed());
    }
    // Living-world dispatch (species_ai): each animal carries the data its
    // dormant pure-function AI needs, so the per-species dispatcher can drive
    // its signature movement (rabbit hop, goat charge) instead of falling
    // through to the generic wander.
    if kind == MobType::Rabbit {
        let _ = ecs.insert_one(id, crate::rabbit_ai::RabbitData::new());
    }
    if kind == MobType::Goat {
        let _ = ecs.insert_one(id, crate::goat_ai::GoatData::new());
    }
    if kind == MobType::Bee {
        let _ = ecs.insert_one(id, crate::bee_ai::BeeData::new());
        let _ = ecs.insert_one(id, Flying);
    }
    // Parrot (Companions wave) is gravity-exempt too — it's a shoulder/flying
    // companion, not a ground walker. BeeData stays Bee-only; Parrot doesn't
    // need the hover-bob AI, only the gravity exemption `Flying` grants.
    if kind == MobType::Parrot {
        let _ = ecs.insert_one(id, Flying);
    }
    // Animals Wave 2 — the remaining dormant species AIs (horse herd-wander,
    // squid drift, bear food-raid, hyena day/night pack). Pure functions
    // shipped long ago; these components let `species_ai` finally dispatch them.
    if crate::mob::is_horse_family(kind) {
        // Horse + Donkey + Mule all herd-wander via HorseData (and ride the gallop).
        let _ = ecs.insert_one(id, crate::horse_ai::HorseData::new());
    }
    // Aquatic drifters (Squid + the new Fish + Glow Squid) share the squid
    // drift/suffocate AI via SquidData.
    if matches!(kind, MobType::Squid | MobType::Fish | MobType::GlowSquid) {
        let _ = ecs.insert_one(id, crate::squid_ai::SquidData::new());
    }
    // Companions wave — Cat / Parrot / Fox carry generic tameable CompanionData
    // (untamed at spawn; the right-click food path tames them → they follow).
    if crate::companion::is_companion_species(kind) {
        let _ = ecs.insert_one(id, crate::companion::CompanionData::untamed());
    }
    if kind == MobType::Bear {
        let _ = ecs.insert_one(id, crate::bear_ai::BearData::new());
    }
    if kind == MobType::Hyena {
        let _ = ecs.insert_one(id, crate::hyena_ai::HyenaData::new());
    }
    // Animals Wave 2 — animal-product cadence (egg lay / milk / shear). One
    // AnimalProductState per producing species tracks its own last-action tick.
    if matches!(kind, MobType::Chicken | MobType::Cow | MobType::Sheep) {
        let _ = ecs.insert_one(id, crate::animal_products::AnimalProductState::new());
    }
    // Six-wave 1A — breedable animals carry heritable Genetics. Wild spawns get
    // seeded variation (raw stock to breed from); a bred baby's genetics are
    // overwritten with its inherited genes by the breeding caller.
    if matches!(
        kind,
        MobType::Cow | MobType::Sheep | MobType::Goat | MobType::Pig
            | MobType::Rabbit | MobType::Chicken | MobType::Horse
            | MobType::Donkey | MobType::Mule
    ) {
        let seed = (position.x as i32 as u32).wrapping_mul(2_654_435_761)
            ^ (position.y as i32 as u32).wrapping_mul(40_503)
            ^ (position.z as i32 as u32).wrapping_mul(73_856_093);
        let _ = ecs.insert_one(id, crate::genetics::Genetics::wild(seed));
    }
    id
}

/// Marks a mob spawned by the column scatter (`scatter_mobs_in_column`), as
/// opposed to a villager, iron golem, tamed pet, or quest/raid-spawned mob.
/// **Only `Scattered` mobs are despawned when their column unloads** — so
/// re-entering a column re-scatters a fresh (deterministic) set instead of
/// piling new mobs on top of the old, which grew passive-mob counts without
/// bound (engine audit 2026-06-04, B: the hostile 80-cap is hostile-only).
pub struct Scattered;

/// #129 — marks a world-author **placed** wild mob (e.g. a donkey pinned by its
/// statue, via `/place`). Like a tamed pet it is NOT `Scattered`, so it survives
/// chunk unload; unlike a tamed pet it has no owner. Persisted through the
/// `saved_mobs` list as `SavedTamedPetData::Authored` — `tamed_mobs_to_saved`
/// queries this marker on save, and the load path re-attaches it.
pub struct Authored;

/// Living-world dispatch — marks a flying mob (the Bee). Gravity-exempt in
/// `tick_entities`; its `species_ai` dispatcher drives the full velocity
/// vector including the vertical hover-bob. Block collision still applies.
pub struct Flying;

/// P9 — marks a mob currently being ridden by a player (e.g. a mounted horse).
/// `mob_ai::tick_mob_ai` skips ridden mobs so their wander AI doesn't fight the
/// rider's steering; the rider's input drives the mob's velocity instead.
pub struct Ridden;

/// Generous ceiling on live scattered (passive wildlife) mobs. Despawn-on-unload
/// already bounds the count to the loaded area; this is a backstop against a
/// pathological case (e.g. teleport outrunning the unload pass) and is set high
/// enough never to thin a normally-loaded world.
pub const MAX_SCATTERED_MOBS: usize = 512;

/// Spawn a mob and tag it [`Scattered`] so the column-unload pass can reclaim
/// it. Used only by `scatter_mobs_in_column`.
fn spawn_scattered_mob(ecs: &mut hecs::World, kind: MobType, pos: Vec3) -> hecs::Entity {
    let id = spawn_mob(ecs, kind, pos);
    let _ = ecs.insert_one(id, Scattered);
    id
}

/// Count live scattered mobs across the whole ECS.
pub fn scattered_mob_count(ecs: &hecs::World) -> usize {
    ecs.query::<&Scattered>().iter().count()
}

/// Despawn every [`Scattered`] mob whose position lies in column `(cx, cz)`.
/// Called when a column unloads. Villagers, golems, tamed pets and other
/// non-scattered mobs are left untouched (they aren't tagged `Scattered`).
/// Returns the number despawned.
pub fn despawn_mobs_in_column(ecs: &mut hecs::World, cx: i32, cz: i32) -> usize {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let ids: Vec<hecs::Entity> = ecs
        .query::<(&Position, &Scattered)>()
        .iter()
        .filter(|(_, (pos, _))| {
            (pos.0.x.floor() as i32).div_euclid(cs) == cx
                && (pos.0.z.floor() as i32).div_euclid(cs) == cz
        })
        .map(|(id, _)| id)
        .collect();
    for id in &ids {
        let _ = ecs.despawn(*id);
    }
    ids.len()
}

/// Scatter mobs on the terrain surface within a column.
/// Deterministic placement based on chunk coordinates.
///
/// Scan down from the world ceiling to the real top of the column — the first
/// non-air block. Used to place scattered LAND animals on the authored / flat
/// surface instead of the generator's noise height (#129): authored and flat
/// worlds put their floor above `BiomeGenerator::terrain_height`'s sea-level
/// noise, so trusting it spawned animals embedded in the ground. Mirrors the
/// night / hostile spawner (`spawning.rs`). Returns 0 for an all-air column
/// (the caller's AIR-surface guard then rejects it).
pub fn real_surface_y(world: &World, wx: i32, wz: i32) -> i32 {
    let mut sy = (crate::world::MAX_CHUNK_Y + 1) * crate::chunk::CHUNK_SIZE as i32 - 1;
    while sy > 0 && world.get_block(wx, sy, wz) == crate::block::AIR {
        sy -= 1;
    }
    sy
}

/// Spec 28d scatter fix (2026-05-23) — reads from
/// `mob::biome_passive_spawn_weights` instead of the hardcoded
/// Cow/Sheep/Pig/Chicken table. Brings Horse / Rabbit / Goat / Bee /
/// Squid / Nostrich into the live spawn path; Hyena (HP-2) gets its
/// family pack-spawn here too. Squid spawns inside the water column
/// when the surface biome is Ocean.
pub fn scatter_mobs_in_column(
    ecs: &mut hecs::World,
    cx: i32,
    cz: i32,
    world: &World,
    biome_gen: &crate::biome::BiomeGenerator,
) {
    use crate::biome::Biome;
    use crate::block;

    // Blank-canvas worlds with mobs disabled produce no spawns at all (v1:
    // gates the whole function, including passive mobs; a future pass can
    // split hostile vs passive if required). The Workshop is likewise mob-free
    // — a creative authoring room never scatters passives (2026-06-18).
    if !world.mobs_enabled || world.is_workshop {
        return;
    }

    let cs = crate::chunk::CHUNK_SIZE as i32;

    // Only spawn in ~25% of chunks (1 in 4)
    let h = mob_spawn_hash(cx, cz);
    if !h.is_multiple_of(4) {
        return;
    }
    // Global passive-mob backstop — despawn-on-unload bounds the count to the
    // loaded area, but refuse to add more past the ceiling regardless (engine
    // audit 2026-06-04, B: unbounded passive-mob growth).
    if scattered_mob_count(ecs) >= MAX_SCATTERED_MOBS {
        return;
    }

    // Deterministic position within chunk
    let lx = ((h >> 4) % cs as u32) as i32;
    let lz = ((h >> 12) % cs as u32) as i32;
    let wx = cx * cs + lx;
    let wz = cz * cs + lz;

    let biome = biome_gen.biome_at(wx, wz);
    let weights = crate::mob::biome_passive_spawn_weights(biome);
    if weights.is_empty() {
        return;
    }
    let is_aquatic_biome = matches!(biome, Biome::Ocean);

    // #129 — pick the spawn surface. Land animals use the REAL column top
    // (`real_surface_y` scans down from the ceiling) so they stand on
    // authored / flat floors instead of sinking into them; the generator's
    // `terrain_height` is sea-level noise, NOT the actual top, so trusting it
    // buried animals in any world whose floor differs (the 600 Billion world's
    // y64 floor, flat worlds' y79). Ocean keeps `terrain_height` (the seabed)
    // so Squid still spawns inside the water column at surface + 1.
    let surface = if is_aquatic_biome {
        biome_gen.terrain_height(wx, wz)
    } else {
        real_surface_y(world, wx, wz)
    };

    // Surface-block gating. Ocean biomes can spawn in WATER (for Squid);
    // every other biome rejects WATER + AIR surface cells.
    let surface_block = world.get_block(wx, surface, wz);
    if surface_block == block::AIR {
        return;
    }
    if surface_block == block::WATER && !is_aquatic_biome {
        return;
    }
    // Lake-basin guard (#10a): in a non-ocean biome whose terrain dips below
    // sea level, `surface` is the submerged seabed and `surface_block` is the
    // GRASS/DIRT *under* the water — so the WATER check above passes and the
    // mob would spawn at surface+1, inside the water column (animals bobbing on
    // ponds). Reject any below-sea-level surface, and defensively any column
    // with water directly above the surface block.
    if !is_aquatic_biome
        && (surface < crate::biome::SEA_LEVEL
            || world.get_block(wx, surface + 1, wz) == block::WATER)
    {
        return;
    }

    let kind = pick_mob_from_biome_weights(&weights, h >> 20);

    // Pets wave Task 13 — per-species surface gate. The surface/surface_block
    // picked above satisfies the generic biome-level checks, but a species
    // (currently only Crab) can additionally require a specific surface
    // block within a Y band (a shoreline, not the open Ocean seabed the rest
    // of this biome's roster is happy to spawn on). Reject the whole
    // attempt this tick rather than falling back to a different species —
    // matches the existing "this cell either spawns or doesn't" cadence.
    if !crate::mob::spawn_surface_ok(kind, surface_block, surface) {
        return;
    }

    // Squid lives in water — spawn at surface + 1 which sits inside the
    // water column for Ocean biomes (terrain_height returns the seabed;
    // sea-level water sits above). Other mobs stand on the surface.
    let spawn_y = surface as f32 + 1.0;
    let spawn_pos = Vec3::new(wx as f32 + 0.5, spawn_y, wz as f32 + 0.5);

    // Pack / family handling for grouped species. Hyena (HP-2): 2-4
    // hyenas in a tight cluster. Nostrich (28d.nostrich): family
    // groups of 2-3. Both reuse the deterministic spawn hash so
    // groups stay stable across replays.
    match kind {
        MobType::Hyena => {
            let pack = crate::hyena_ai::pack_size_for_seed(h);
            for (dx, dz) in crate::hyena_ai::pack_offsets(pack) {
                let p = Vec3::new(
                    spawn_pos.x + dx as f32,
                    spawn_pos.y,
                    spawn_pos.z + dz as f32,
                );
                spawn_scattered_mob(ecs, MobType::Hyena, p);
            }
        }
        MobType::Nostrich => {
            // Family group of 2 or 3 — pick from the hash to vary by cell.
            let family = 2 + ((h >> 24) % 2) as i32;
            for i in 0..family {
                let dx = (i - family / 2) as f32 * 0.6;
                let p = Vec3::new(spawn_pos.x + dx, spawn_pos.y, spawn_pos.z);
                spawn_scattered_mob(ecs, MobType::Nostrich, p);
            }
        }
        _ => {
            spawn_scattered_mob(ecs, kind, spawn_pos);
        }
    }
}

/// Pure: weighted pick from a biome spawn table. The caller supplies an
/// already-shifted hash bucket so the choice stays deterministic per
/// chunk. Falls back to the first entry on any pathological inputs
/// (zero total weight, empty slice handled by caller).
pub fn pick_mob_from_biome_weights(
    weights: &[(MobType, u8)],
    hash_bucket: u32,
) -> MobType {
    let total: u32 = weights.iter().map(|(_, w)| *w as u32).sum();
    if total == 0 {
        return weights[0].0;
    }
    let pick = hash_bucket % total;
    let mut cum = 0u32;
    for (m, w) in weights {
        cum += *w as u32;
        if pick < cum {
            return *m;
        }
    }
    weights[weights.len() - 1].0
}

fn mob_spawn_hash(cx: i32, cz: i32) -> u32 {
    let mut h = (cx as u32).wrapping_mul(374761393) ^ (cz as u32).wrapping_mul(668265263);
    h = h.wrapping_add(0x9E3779B9);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

// --- Projectiles (Wave 23) ---

/// Spawn one arrow at `position` flying along `velocity` (blocks/tick) with
/// the given damage payload. The arrow has its own physics tick
/// (`tick_projectiles`) that runs independently of the mob/item physics.
///
/// Note: deliberately omits `OnGround` so the projectile is not picked up by
/// the existing `tick_entities` loop (which would treat it like a mob and
/// apply ground collisions, stopping it dead at the first solid block face).
pub fn spawn_arrow(
    ecs: &mut hecs::World,
    position: Vec3,
    velocity: Vec3,
    damage: f32,
    owner: Option<usize>,
) {
    ecs.spawn((
        Position(position),
        Velocity(velocity),
        Hitbox { width: ARROW_HITBOX, height: ARROW_HEIGHT },
        ProjectileEntity { damage, is_blunt: false, owner },
        Lifetime(ARROW_LIFETIME_TICKS),
    ));
}

/// Rubber feature — spawn a slingshot rubber-ball projectile. Mirrors
/// `spawn_arrow` but stamps `is_blunt: true` on the projectile so the
/// combat hit handler can route the stun effect alongside the damage.
pub fn spawn_blunt_projectile(
    ecs: &mut hecs::World,
    position: Vec3,
    velocity: Vec3,
    damage: f32,
    owner: Option<usize>,
) {
    ecs.spawn((
        Position(position),
        Velocity(velocity),
        Hitbox { width: ARROW_HITBOX, height: ARROW_HEIGHT },
        ProjectileEntity { damage, is_blunt: true, owner },
        Lifetime(ARROW_LIFETIME_TICKS),
    ));
}

/// Run one physics tick for every projectile in the world. Applies gravity,
/// integrates velocity, despawns on solid-block hit, and damages the first
/// mob whose hitbox the projectile overlaps. Despawns the projectile in
/// either hit case.
///
/// `shooter_sneaking` is the live per-slot sneak signal (`GameState.local_sneak`)
/// — 1C no-friendly-fire: an arrow that reaches its own shooter's tamed
/// pet/steed embeds harmlessly (no damage, no `LastAttacker` stamp) unless
/// the shooter was sneaking at the moment of impact, which reads as a
/// deliberate hit. Mirrors the melee bypass in `game_loop`'s attack path.
///
/// Returns the number of projectiles that hit a mob this tick (informational;
/// counts shielded friendly-fire hits too, since the projectile still
/// physically struck something).
pub fn tick_projectiles(
    ecs: &mut hecs::World,
    world: &World,
    registry: &BlockRegistry,
    shooter_sneaking: &std::collections::HashMap<usize, bool>,
) -> u32 {
    // Pass 1: snapshot projectiles + mob targets so we can mutate without
    // overlapping borrows in the second pass.
    let projectiles: Vec<(hecs::Entity, Vec3, Vec3, f32, bool, Option<usize>)> = ecs
        .query::<(&Position, &Velocity, &ProjectileEntity)>()
        .iter()
        .map(|(id, (p, v, pe))| (id, p.0, v.0, pe.damage, pe.is_blunt, pe.owner))
        .collect();
    if projectiles.is_empty() {
        return 0;
    }
    let mob_data: Vec<(hecs::Entity, Vec3, f32, f32)> = ecs
        .query::<(&Position, &Hitbox, &MobKind)>()
        .iter()
        .map(|(id, (p, h, _))| (id, p.0, h.width, h.height))
        .collect();

    // Pass 2: per-projectile decisions.
    let mut updates: Vec<(hecs::Entity, Vec3, Vec3)> = Vec::new();
    let mut to_despawn: Vec<hecs::Entity> = Vec::new();
    let mut hits: Vec<(hecs::Entity, f32, bool, Option<usize>)> = Vec::new();
    let mut hit_count = 0u32;

    for (proj_id, pos, vel, damage, is_blunt, owner) in projectiles {
        let new_vel = Vec3::new(vel.x, vel.y - ARROW_GRAVITY, vel.z);
        let new_pos = pos + new_vel;

        // Block collision — despawn on solid.
        let bx = new_pos.x.floor() as i32;
        let by = new_pos.y.floor() as i32;
        let bz = new_pos.z.floor() as i32;
        if world.is_solid(bx, by, bz, registry) {
            to_despawn.push(proj_id);
            continue;
        }

        // Mob collision — first mob whose extended AABB (mob hitbox plus
        // half the arrow width) the arrow centre enters. Including the
        // arrow's own half-width prevents grazing arrows from passing
        // through edges they should clip.
        let mut hit_mob: Option<hecs::Entity> = None;
        for (mob_id, mob_pos, mw, mh) in &mob_data {
            let half_w = *mw / 2.0 + ARROW_HITBOX / 2.0;
            let dx = new_pos.x - mob_pos.x;
            let dz = new_pos.z - mob_pos.z;
            let dy = new_pos.y - mob_pos.y;
            if dx.abs() < half_w
                && dz.abs() < half_w
                && dy >= -ARROW_HEIGHT
                && dy < *mh
            {
                hit_mob = Some(*mob_id);
                break;
            }
        }
        if let Some(mob_id) = hit_mob {
            hits.push((mob_id, damage, is_blunt, owner));
            to_despawn.push(proj_id);
            hit_count += 1;
            continue;
        }

        updates.push((proj_id, new_pos, new_vel));
    }

    // Pass 3: apply.
    for (id, new_pos, new_vel) in updates {
        if let Ok((p, v)) = ecs.query_one_mut::<(&mut Position, &mut Velocity)>(id) {
            p.0 = new_pos;
            v.0 = new_vel;
        }
    }
    for (mob_id, dmg, is_blunt, owner) in hits {
        // 1C no-friendly-fire — an arrow that reaches the shooter's own
        // tamed pet/steed does nothing unless they were sneaking when it
        // landed (a deliberate hit). Ownerless projectiles (mob-fired /
        // test) can't be a player's own pet's shooter, so they're exempt.
        let shielded = owner.is_some_and(|pidx| {
            !shooter_sneaking.get(&pidx).copied().unwrap_or(false)
                && crate::tameable::is_players_own_pet(
                    ecs,
                    mob_id,
                    &format!("local-player-{pidx}"),
                    pidx,
                )
        });
        if shielded {
            continue;
        }
        if let Ok(mut h) = ecs.get::<&mut crate::combat::Health>(mob_id) {
            h.take_damage(dmg);
        }
        // Stamp the firing player as the last attacker so the kill is
        // attributed to the shooter (bounties / kill_counter), matching
        // the melee path in `combat::player_attack`. Most-recent
        // damaging hit wins, so this overwrites any prior attacker.
        if let Some(pidx) = owner {
            let _ = ecs.insert_one(mob_id, crate::combat::LastAttacker(pidx));
            // Task 13 (bug-hardening, 2026-07-07) — an arrow landing on a
            // Bear or Hyena provokes it the same as a melee hit. See
            // `combat::notify_hit_bear_or_hyena` and its wiring in
            // `combat::player_attack` for the melee side.
            crate::combat::notify_hit_bear_or_hyena(ecs, mob_id, pidx);
        }
        // Rubber feature — slingshot stun. If the projectile is blunt
        // and damage >= midpoint (proxy for "at least half charge"),
        // and the target species is stunnable, force the mob's AI
        // into a brief Wander state interrupting Idle / Wander chains.
        // Hostile mobs already in Chase aren't broken — slingshot
        // doesn't reset an active attack lock.
        if is_blunt
            && dmg >= (crate::slingshot::SLINGSHOT_MIN_DAMAGE
                + crate::slingshot::SLINGSHOT_MAX_DAMAGE) / 2.0
        {
            let kind = ecs.get::<&MobKind>(mob_id).map(|k| k.0).ok();
            if let Some(kind) = kind
                && crate::slingshot::is_stunnable(kind)
                    && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(mob_id)
                        && matches!(ai.state,
                            crate::mob_ai::AiState::Idle { .. }
                                | crate::mob_ai::AiState::Wander { .. })
                        {
                            ai.state = crate::mob_ai::AiState::Wander {
                                timer: crate::slingshot::STUN_TICKS,
                            };
                        }
        }
    }
    for id in to_despawn {
        let _ = ecs.despawn(id);
    }
    hit_count
}

// --- Item drops ---

/// Spawn one stack of items as an ItemEntity at the given world position.
/// `seed_h` controls the small horizontal spread so multiple drops from the
/// same kill scatter slightly rather than stacking in one column.
pub fn spawn_item(
    ecs: &mut hecs::World,
    position: Vec3,
    stack: crate::item::ItemStack,
    seed_h: u32,
) {
    let h = seed_h;
    let dx = ((h % 100) as f32 / 100.0 - 0.5) * 0.25;
    let dz = (((h >> 8) % 100) as f32 / 100.0 - 0.5) * 0.25;
    ecs.spawn((
        Position(position + Vec3::new(0.0, 0.4, 0.0)),
        Velocity(Vec3::new(dx, 0.22, dz)),
        Hitbox { width: ITEM_HITBOX, height: ITEM_HEIGHT },
        OnGround(false),
        ItemEntity { stack, pickup_delay: ITEM_PICKUP_DELAY_TICKS, dropper: None },
        Lifetime(ITEM_LIFETIME_TICKS),
    ));
}

/// Spawn one stack of items as an ItemEntity at `position` with an explicit
/// initial `velocity` (blocks/tick). Used by Q-drop so the item is tossed in
/// the player's look direction rather than dribbling at their feet.
///
/// `dropper_player_index` is the player who tossed it. They are blocked from
/// picking it back up for `ITEM_DROP_PICKUP_DELAY_TICKS` (1.5s); any other
/// player can pick it up immediately.
pub fn spawn_thrown_item(
    ecs: &mut hecs::World,
    position: Vec3,
    velocity: Vec3,
    stack: crate::item::ItemStack,
    dropper_player_index: u8,
) {
    ecs.spawn((
        Position(position),
        Velocity(velocity),
        Hitbox { width: ITEM_HITBOX, height: ITEM_HEIGHT },
        OnGround(false),
        ItemEntity {
            stack,
            pickup_delay: ITEM_DROP_PICKUP_DELAY_TICKS,
            dropper: Some(dropper_player_index),
        },
        Lifetime(ITEM_LIFETIME_TICKS),
    ));
}

/// Decrement Lifetime + ItemEntity.pickup_delay timers, despawn anything
/// whose Lifetime hit zero.
pub fn tick_item_lifetimes(ecs: &mut hecs::World) -> u32 {
    let mut to_despawn: Vec<hecs::Entity> = Vec::new();
    for (id, life) in ecs.query_mut::<&mut Lifetime>() {
        if life.0 == 0 {
            to_despawn.push(id);
        } else {
            life.0 -= 1;
        }
    }
    let count = to_despawn.len() as u32;
    for id in to_despawn {
        let _ = ecs.despawn(id);
    }
    // Pickup delay decays separately so an item with delay still ages out.
    for (_id, item) in ecs.query_mut::<&mut ItemEntity>() {
        if item.pickup_delay > 0 {
            item.pickup_delay -= 1;
        }
    }
    count
}

/// Magnetise items toward nearby players + pick up any that are within
/// `ITEM_PICKUP_RADIUS`. Returns one `(real_player_index, granted_stack)`
/// entry per pickup event — partial fits grant only the units that landed —
/// so the server can relay each grant to its client (`InventoryGrantPacket`,
/// death-drops phase 2b). Callers that only need a count use `.len()`.
///
/// `players` is a slice of `(real_player_index, player_position, &mut Inventory)`.
/// Callers pass only living players. The real index is used to honour
/// `ItemEntity.dropper` — during a Q-drop's pickup_delay window the dropper is
/// blocked but other players can pick up immediately.
///
/// `eligible` gates which stacks may be picked up at all (no magnet, no
/// grant): the client sim passes `|_| true`; the server passes "survives the
/// `ItemRef` wire encoding" so a tool drop is never granted remotely as a
/// wrong-kind/full-durability fabrication.
pub fn tick_item_pickups(
    ecs: &mut hecs::World,
    players: &mut [(usize, Vec3, &mut crate::inventory::Inventory)],
    eligible: impl Fn(&crate::item::Item) -> bool,
) -> Vec<(usize, crate::item::ItemStack)> {
    if players.is_empty() {
        return Vec::new();
    }

    // First pass: collect actions to take. Borrow checker won't let us
    // mutate `players[idx]` from inside the ECS query iterator directly.
    enum Action {
        Magnet(hecs::Entity, Vec3),               // (item, pull-direction)
        TryPickup(hecs::Entity, usize, crate::item::ItemStack),
    }
    let mut actions: Vec<Action> = Vec::new();
    for (id, (pos, item)) in ecs.query::<(&Position, &ItemEntity)>().iter() {
        // Ineligible stacks are inert to this pass: no magnet pull either, or
        // an unpickable drop would jitter around the player forever.
        if !eligible(&item.stack.item) {
            continue;
        }
        // Pickup-delay rules:
        //   dropper = None  → block all players during delay (natural drops).
        //   dropper = Some(idx) → block only that player; others bypass.
        let delay_blocks_all = item.pickup_delay > 0 && item.dropper.is_none();
        if delay_blocks_all {
            continue;
        }
        let mut best: Option<(usize, f32)> = None;
        for (slot, (real_idx, pp, _)) in players.iter().enumerate() {
            // Honour per-player dropper delay.
            if item.pickup_delay > 0
                && item.dropper == Some(*real_idx as u8)
            {
                continue;
            }
            let d = (*pp - pos.0).length();
            if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((slot, d));
            }
        }
        let Some((slot, dist)) = best else { continue; };
        if dist < ITEM_PICKUP_RADIUS {
            actions.push(Action::TryPickup(id, slot, item.stack.clone()));
        } else if dist < ITEM_MAGNET_RADIUS {
            let dir = (players[slot].1 - pos.0).normalize_or_zero();
            actions.push(Action::Magnet(id, dir));
        }
    }

    // Second pass: apply.
    let mut grants: Vec<(usize, crate::item::ItemStack)> = Vec::new();
    let mut to_despawn: Vec<hecs::Entity> = Vec::new();
    for action in actions {
        match action {
            Action::Magnet(id, dir) => {
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0 += dir * ITEM_MAGNET_PULL;
                }
            }
            Action::TryPickup(id, slot, stack) => {
                let real_idx = players[slot].0;
                let offered = stack.clone();
                match players[slot].2.add_item(stack) {
                    None => {
                        // Whole stack fit — pick it up.
                        grants.push((real_idx, offered));
                        to_despawn.push(id);
                    }
                    Some(remainder) => {
                        // Partial (or zero) fit. The ground item must keep
                        // ONLY what didn't land, or the next magnet tick
                        // re-grants the part already taken → duplication.
                        if let Ok(mut item) = ecs.get::<&mut ItemEntity>(id) {
                            if remainder.count < item.stack.count {
                                // Some units landed — grant just those.
                                let mut landed = offered;
                                landed.count = item.stack.count - remainder.count;
                                grants.push((real_idx, landed));
                            }
                            item.stack = remainder;
                        }
                    }
                }
            }
        }
    }
    for id in to_despawn {
        let _ = ecs.despawn(id);
    }
    grants
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;
    use crate::item::{ItemStack, MaterialId};

    fn one_bone_stack() -> ItemStack {
        ItemStack::new_material(MaterialId::Bone, 2)
    }

    #[test]
    fn real_surface_y_finds_authored_floor_not_noise_height() {
        // #129 Bug 1 — animal scatter must place mobs on the REAL column top,
        // not the generator's sea-level noise height. The 600 Billion world
        // authors its floor at y64 (above SEA_LEVEL 62); trusting noise height
        // buried animals in the ground.
        let mut world = World::new();
        world.set_block(3, 64, 7, crate::block::GRASS);
        assert_eq!(real_surface_y(&world, 3, 7), 64, "scans down to the real top block");

        // A taller build column — the highest non-air block wins (so an animal
        // lands on a build's roof, never inside it).
        world.set_block(3, 50, 8, crate::block::GRASS);
        world.set_block(3, 90, 8, crate::block::STONE);
        assert_eq!(real_surface_y(&world, 3, 8), 90, "finds the highest non-air block");

        // All-air column → 0 (the caller's AIR-surface guard then rejects it).
        assert_eq!(real_surface_y(&world, 50, 50), 0, "all-air column returns 0");
    }

    #[test]
    fn entities_float_up_in_water() {
        // 2026-05-30 playtest: a mob knocked into water should rise to the
        // surface and swim out, not sink to the lakebed and drown. Submerge
        // a mob near the bottom of a water column and confirm it rises.
        let mut world = World::new();
        let reg = crate::block::BlockRegistry::new();
        for x in -2..=2 {
            for z in -2..=2 {
                world.set_block(x, 0, z, crate::block::STONE);
                for y in 1..=5 {
                    world.set_block(x, y, z, crate::block::WATER);
                }
            }
        }
        let mut ecs = hecs::World::new();
        let id = spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.5, 1.5, 0.5));
        let start_y = ecs.get::<&Position>(id).unwrap().0.y;
        for _ in 0..40 {
            tick_entities(&mut ecs, &world, &reg);
        }
        let end_y = ecs.get::<&Position>(id).unwrap().0.y;
        assert!(
            end_y > start_y,
            "submerged mob should float up (start {start_y}, end {end_y})",
        );
    }

    #[test]
    fn flowing_water_pushes_item_entities_downstream() {
        // A dropped item in a flowing stream rides the current: the flow
        // vector (level gradient) adds horizontal velocity each tick.
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -2..=4 {
            for z in -2..=2 {
                world.set_block(x, 9, z, crate::block::STONE);
            }
        }
        // Hand-built stream: source (level 0) at x=0, flow levels 1..3 at +x.
        for x in 0..=3 {
            world.set_block(x, 10, 0, crate::block::WATER);
            world.set_meta((x, 10, 0), crate::meta::with_aux(0, x as u8));
        }
        let mut ecs = hecs::World::new();
        spawn_item(
            &mut ecs,
            Vec3::new(1.5, 10.2, 0.5),
            one_bone_stack(),
            0,
        );
        // Kill the spawn jitter so the drift we measure is pure current.
        for (_id, vel) in ecs.query_mut::<&mut Velocity>() {
            vel.0 = Vec3::ZERO;
        }
        for _ in 0..10 {
            tick_entities(&mut ecs, &world, &registry);
        }
        let (_id, pos) = ecs
            .query_mut::<&Position>()
            .into_iter()
            .next()
            .expect("item still alive");
        assert!(
            pos.0.x > 1.55,
            "item drifted downstream (+x): x = {}",
            pos.0.x
        );
    }

    #[test]
    fn spawn_item_attaches_full_component_set() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(1.0, 64.0, 1.0), one_bone_stack(), 42);
        let mut count = 0;
        for (_, (_pos, _vel, _hb, _og, item, life)) in ecs
            .query::<(&Position, &Velocity, &Hitbox, &OnGround, &ItemEntity, &Lifetime)>()
            .iter()
        {
            count += 1;
            assert_eq!(life.0, ITEM_LIFETIME_TICKS);
            assert_eq!(item.pickup_delay, ITEM_PICKUP_DELAY_TICKS);
            assert_eq!(item.stack.count, 2);
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn tick_item_lifetimes_despawns_at_zero() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        // Force lifetime to 1 so the next tick despawns it.
        for (_, life) in ecs.query_mut::<&mut Lifetime>() {
            life.0 = 1;
        }
        // tick_item_lifetimes decrements first; lifetime 1 → 0 (no despawn);
        // next call sees 0 and despawns. Run twice.
        let removed_first = tick_item_lifetimes(&mut ecs);
        assert_eq!(removed_first, 0);
        let removed_second = tick_item_lifetimes(&mut ecs);
        assert_eq!(removed_second, 1);
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn tick_item_lifetimes_decays_pickup_delay() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        let delay = ecs
            .query::<&ItemEntity>()
            .iter()
            .next()
            .map(|(_, item)| item.pickup_delay)
            .unwrap();
        assert_eq!(delay, 0);
    }

    #[test]
    fn pickup_skipped_during_pickup_delay() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        let mut inv = Inventory::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let mut players: Vec<(usize, Vec3, &mut Inventory)> = vec![(0, player_pos, &mut inv)];
        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert!(picked.is_empty(), "pickup_delay must block pickup");
        assert_eq!(inv.slot(0).map(|s| s.count), None);
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 1);
    }

    #[test]
    fn pickup_succeeds_after_delay_when_in_range() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> = vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert_eq!(picked.len(), 1);
        assert_eq!(inv.slot(0).map(|s| s.count), Some(2));
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn pickup_outside_radius_no_op() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> = vec![(0, Vec3::new(10.0, 64.0, 0.0), &mut inv)];
        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert!(picked.is_empty());
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 1);
    }

    #[test]
    fn partial_pickup_leaves_only_remainder_on_ground_no_dupe() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut ecs = hecs::World::new();
        // 64 stone on the ground.
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0),
            ItemStack::new_block(crate::block::STONE, 64), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        // Inventory: 35 non-stacking tools + one 34-stone stack → 30 units of
        // headroom (stone caps at 64) and NO empty slot for spill.
        let mut inv = Inventory::new();
        for i in 0..35 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        inv.set_slot(35, Some(ItemStack::new_block(crate::block::STONE, 34)));
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];

        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert_eq!(picked.len(), 1, "some units landed → counts as a pickup");
        // Ground item must survive, holding ONLY the 34 that didn't fit.
        let ground: Vec<u8> = ecs.query::<&ItemEntity>().iter()
            .map(|(_, it)| it.stack.count).collect();
        assert_eq!(ground, vec![34], "ground keeps only the unplaced remainder");
        let inv_stone: u32 = inv.slots_iter().flatten()
            .filter(|s| matches!(s.item, crate::item::Item::Block(b) if b == crate::block::STONE))
            .map(|s| s.count as u32).sum();
        assert_eq!(inv_stone, 64, "the 34-stack topped up to its 64 cap");
        // Conservation: ground (34) + inventory (64) == original (64 + 34).
        assert_eq!(inv_stone + 34, 64 + 34, "no dupe and no loss");
    }

    #[test]
    fn pickup_returns_granted_stack_keyed_by_real_player_index() {
        // Death-drops phase 2b — the server needs to know WHAT landed for WHOM
        // to send the InventoryGrant packet, not just a count.
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        let mut inv = Inventory::new();
        // Real player index 3 (e.g. a remote slot) — the grant must carry it.
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(3, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let grants = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].0, 3, "grant keyed by real player index");
        assert!(matches!(
            grants[0].1.item,
            crate::item::Item::Material(crate::item::MaterialId::Bone)
        ));
        assert_eq!(grants[0].1.count, 2, "full stack granted");
    }

    #[test]
    fn partial_pickup_grants_only_landed_units() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0),
            ItemStack::new_block(crate::block::STONE, 64), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        // 30 units of headroom (see partial_pickup_leaves_only_remainder…).
        let mut inv = Inventory::new();
        for i in 0..35 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        inv.set_slot(35, Some(ItemStack::new_block(crate::block::STONE, 34)));
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let grants = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].1.count, 30, "grant carries only the landed units");
    }

    #[test]
    fn pickup_filter_leaves_ineligible_stacks_on_the_ground() {
        // The server passes an eligibility filter so stacks that can't ride
        // the wire (tools — tier-only encoding) are never granted remotely.
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(0.0, 64.0, 0.0), one_bone_stack(), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let grants = tick_item_pickups(&mut ecs, &mut players, |item| {
            !matches!(item, crate::item::Item::Material(_))
        });
        assert!(grants.is_empty(), "filtered stack must not be granted");
        assert_eq!(
            ecs.query::<&ItemEntity>().iter().count(),
            1,
            "filtered stack stays on the ground"
        );
        assert_eq!(inv.slot(0).map(|s| s.count), None, "inventory untouched");
    }

    #[test]
    fn despawn_mobs_in_column_removes_only_scattered_mobs_in_that_column() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let cs = crate::chunk::CHUNK_SIZE as f32;
        // Scattered wildlife in column (0,0).
        let wild = spawn_scattered_mob(&mut ecs, MobType::Cow, Vec3::new(2.0, 64.0, 2.0));
        // A NON-scattered mob in the same column (stands in for a villager / golem / tamed pet).
        let resident = spawn_mob(&mut ecs, MobType::Cow, Vec3::new(3.0, 64.0, 3.0));
        // Scattered wildlife in column (1,0).
        let wild_other = spawn_scattered_mob(&mut ecs, MobType::Sheep, Vec3::new(cs + 2.0, 64.0, 2.0));

        let removed = despawn_mobs_in_column(&mut ecs, 0, 0);
        assert_eq!(removed, 1, "only the scattered mob in (0,0)");
        assert!(!ecs.contains(wild), "scattered (0,0) reclaimed");
        assert!(ecs.contains(resident), "non-scattered mob survives unload");
        assert!(ecs.contains(wild_other), "scattered (1,0) untouched");
    }

    #[test]
    fn reloading_a_column_does_not_accumulate_scattered_mobs() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        for i in 0..3 {
            spawn_scattered_mob(&mut ecs, MobType::Cow, Vec3::new(2.0 + i as f32 * 0.1, 64.0, 2.0));
        }
        assert_eq!(scattered_mob_count(&ecs), 3);
        despawn_mobs_in_column(&mut ecs, 0, 0); // column unloads
        assert_eq!(scattered_mob_count(&ecs), 0, "unload reclaims them");
        for i in 0..3 {
            spawn_scattered_mob(&mut ecs, MobType::Cow, Vec3::new(2.0 + i as f32 * 0.1, 64.0, 2.0));
        }
        // The leak kept the old set and re-scattered on top → 6. Fixed → 3.
        assert_eq!(scattered_mob_count(&ecs), 3, "no accumulation across unload/reload");
    }

    // --- Spec 28d scatter fix (2026-05-23) ---

    use crate::mob::MobType;

    #[test]
    fn pick_mob_from_biome_weights_is_deterministic() {
        let weights = vec![
            (MobType::Cow, 25),
            (MobType::Sheep, 25),
            (MobType::Pig, 25),
            (MobType::Chicken, 25),
        ];
        let a = pick_mob_from_biome_weights(&weights, 12345);
        let b = pick_mob_from_biome_weights(&weights, 12345);
        assert_eq!(a, b);
    }

    #[test]
    fn pick_mob_from_biome_weights_picks_within_table() {
        let weights = vec![(MobType::Horse, 10), (MobType::Cow, 5)];
        for bucket in 0u32..200 {
            let picked = pick_mob_from_biome_weights(&weights, bucket);
            assert!(
                matches!(picked, MobType::Horse | MobType::Cow),
                "picker returned {:?} which isn't in the table", picked,
            );
        }
    }

    #[test]
    fn pick_mob_from_biome_weights_biases_to_heavier_entry() {
        // Horse weighted 90 vs Cow weighted 10 — across 1000 buckets,
        // Horse should dominate (~90 %).
        let weights = vec![(MobType::Horse, 90), (MobType::Cow, 10)];
        let mut horse_count = 0;
        let mut cow_count = 0;
        for bucket in 0u32..1000 {
            match pick_mob_from_biome_weights(&weights, bucket) {
                MobType::Horse => horse_count += 1,
                MobType::Cow => cow_count += 1,
                _ => panic!("unexpected pick"),
            }
        }
        assert!(horse_count >= 800, "Horse should dominate, got {horse_count}");
        assert!(cow_count <= 200, "Cow should be minority, got {cow_count}");
    }

    #[test]
    fn pick_mob_from_biome_weights_handles_single_entry() {
        let weights = vec![(MobType::Squid, 60)];
        assert_eq!(pick_mob_from_biome_weights(&weights, 0), MobType::Squid);
        assert_eq!(pick_mob_from_biome_weights(&weights, 999_999), MobType::Squid);
    }

    /// Build a fixture world where every (wx, wz) cell has `block_id`
    /// placed at the biome generator's actual `terrain_height(wx, wz)`
    /// — so the scatter function's `world.get_block(wx, surface, wz)`
    /// returns the expected solid block instead of AIR.
    fn fixture_following_terrain(
        block_id: crate::block::BlockId,
        biome_gen: &crate::biome::BiomeGenerator,
        chunk_range: i32,
    ) -> crate::world::World {
        let mut w = crate::world::World::new();
        let cs = crate::chunk::CHUNK_SIZE as i32;
        for cx in -chunk_range..=chunk_range {
            for cz in -chunk_range..=chunk_range {
                for lx in 0..cs {
                    for lz in 0..cs {
                        let wx = cx * cs + lx;
                        let wz = cz * cs + lz;
                        let y = biome_gen.terrain_height(wx, wz);
                        w.set_block(wx, y, wz, block_id);
                    }
                }
            }
        }
        w
    }

    /// Sweep scatter across a grid of chunks; return every spawned
    /// mob's kind.
    fn sweep_scatter_kinds(
        world: &crate::world::World,
        biome_gen: &crate::biome::BiomeGenerator,
        chunk_range: i32,
    ) -> Vec<MobType> {
        let mut ecs = hecs::World::new();
        for cx in -chunk_range..=chunk_range {
            for cz in -chunk_range..=chunk_range {
                scatter_mobs_in_column(&mut ecs, cx, cz, world, biome_gen);
            }
        }
        ecs.query::<&MobKind>().iter().map(|(_, k)| k.0).collect()
    }

    #[test]
    fn scatter_produces_new_28d_species_after_fix() {
        // The hardcoded Cow/Sheep/Pig/Chicken table is gone. Across a
        // big chunk sweep against a fixture that follows the real
        // terrain heights, at least one of the new 28d species (Horse,
        // Rabbit, Goat, Bee) must appear somewhere — a definitive
        // sign that the biome-driven dispatch is wired.
        let bg = crate::biome::BiomeGenerator::new(42);
        let world = fixture_following_terrain(crate::block::STONE, &bg, 16);
        let kinds = sweep_scatter_kinds(&world, &bg, 16);
        let new_28d = [
            MobType::Horse, MobType::Rabbit, MobType::Goat,
            MobType::Bee,
        ];
        let saw_new_species = kinds.iter().any(|k| new_28d.contains(k));
        assert!(saw_new_species,
            "expected at least one Spec 28d species across the chunk sweep; got {:?}",
            kinds);
    }

    #[test]
    fn scatter_never_produces_mob_for_unsupported_surface() {
        // AIR-surface columns must spawn nothing (no surface to stand on).
        let bg = crate::biome::BiomeGenerator::new(42);
        let world = crate::world::World::new(); // empty → every column is air
        let kinds = sweep_scatter_kinds(&world, &bg, 5);
        assert!(kinds.is_empty(),
            "AIR-surface columns must spawn no mobs; got {:?}", kinds);
    }

    /// #10a lake-basin guard: a non-aquatic column with WATER directly above its
    /// surface block must spawn no mob (animals were bobbing on ponds because
    /// the seabed GRASS passes the surface-block != WATER check). A/B: the SAME
    /// land column above sea level spawns normally with air above, but nothing
    /// with water above — so the water-above branch is provably the cause.
    #[test]
    fn scatter_rejects_submerged_column_with_water_above_surface() {
        use crate::biome::{Biome, SEA_LEVEL};
        let bg = crate::biome::BiomeGenerator::new(42);
        let cs = crate::chunk::CHUNK_SIZE as i32;

        // Find a chunk scatter will actually spawn in: hash%4==0, a non-Ocean
        // biome with passive weights, and NATURAL terrain at/above sea level (so
        // the only thing that can suppress the spawn is the water-above branch,
        // never the `surface < SEA_LEVEL` branch).
        let mut found = None;
        'search: for cx in 0..64 {
            for cz in 0..64 {
                let h = mob_spawn_hash(cx, cz);
                if h % 4 != 0 {
                    continue;
                }
                let lx = ((h >> 4) % cs as u32) as i32;
                let lz = ((h >> 12) % cs as u32) as i32;
                let wx = cx * cs + lx;
                let wz = cz * cs + lz;
                let surface = bg.terrain_height(wx, wz);
                let biome = bg.biome_at(wx, wz);
                if matches!(biome, Biome::Ocean) {
                    continue;
                }
                if crate::mob::biome_passive_spawn_weights(biome).is_empty() {
                    continue;
                }
                if surface < SEA_LEVEL {
                    continue;
                }
                found = Some((cx, cz, wx, wz, surface));
                break 'search;
            }
        }
        let (cx, cz, wx, wz, surface) =
            found.expect("a spawnable non-Ocean land chunk at/above sea level");

        // Control: solid surface, AIR above → scatter spawns.
        let mut control = crate::world::World::new();
        control.set_block(wx, surface, wz, crate::block::GRASS);
        let mut ecs_a = hecs::World::new();
        scatter_mobs_in_column(&mut ecs_a, cx, cz, &control, &bg);
        assert!(
            scattered_mob_count(&ecs_a) > 0,
            "control: a normal land column above sea level must spawn a mob"
        );

        // Guard: same column, WATER directly above the surface → reject.
        let mut submerged = crate::world::World::new();
        submerged.set_block(wx, surface, wz, crate::block::GRASS);
        submerged.set_block(wx, surface + 1, wz, crate::block::WATER);
        let mut ecs_b = hecs::World::new();
        scatter_mobs_in_column(&mut ecs_b, cx, cz, &submerged, &bg);
        assert_eq!(
            scattered_mob_count(&ecs_b),
            0,
            "submerged column (water above surface) must spawn no mob (#10a)"
        );
    }

    #[test]
    fn hyena_when_picked_spawns_a_pack() {
        // The biome-driven picker eventually rolls Hyena in Savanna.
        // When it does the pack-spawn arm fires 2-4 hyenas. Across the
        // sweep: if any hyenas appear, the count must be ≥ 2 (a single
        // hyena would mean the pack arm wasn't wired correctly).
        let bg = crate::biome::BiomeGenerator::new(7);
        let world = fixture_following_terrain(crate::block::STONE, &bg, 16);
        let kinds = sweep_scatter_kinds(&world, &bg, 16);
        let hyena_count = kinds.iter().filter(|k| **k == MobType::Hyena).count();
        if hyena_count > 0 {
            assert!(hyena_count >= 2,
                "Hyenas should come in packs of 2-4, got {hyena_count}");
        }
    }

    // --- Projectiles (Wave 23) ---

    #[test]
    fn spawn_arrow_attaches_projectile_components() {
        let mut ecs = hecs::World::new();
        spawn_arrow(
            &mut ecs,
            Vec3::new(0.0, 70.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            ARROW_DAMAGE,
            None,
        );
        let mut count = 0;
        for (_, (_pos, _vel, _hb, proj, life)) in ecs
            .query::<(&Position, &Velocity, &Hitbox, &ProjectileEntity, &Lifetime)>()
            .iter()
        {
            count += 1;
            assert_eq!(life.0, ARROW_LIFETIME_TICKS);
            assert!((proj.damage - ARROW_DAMAGE).abs() < 1e-3);
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn tick_projectiles_applies_gravity() {
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        spawn_arrow(
            &mut ecs,
            Vec3::new(0.0, 70.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            ARROW_DAMAGE,
            None,
        );
        tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        let v = ecs.query::<&Velocity>().iter().next().map(|(_, v)| v.0).unwrap();
        // Y-velocity must have decreased by ARROW_GRAVITY exactly (started at 0).
        assert!((v.y + ARROW_GRAVITY).abs() < 1e-4);
        // X-velocity unchanged.
        assert!((v.x - 1.0).abs() < 1e-4);
    }

    #[test]
    fn tick_projectiles_despawns_on_solid_block_hit() {
        use crate::block::STONE;
        let mut ecs = hecs::World::new();
        let mut world = crate::world::World::new();
        // Wall at x=2.
        for y in 60..80 { for z in -5..5 { world.set_block(2, y, z, STONE); } }
        let registry = crate::block::BlockRegistry::new();
        // Arrow at x=1.5 flying toward x=2.
        spawn_arrow(
            &mut ecs,
            Vec3::new(1.5, 70.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            ARROW_DAMAGE,
            None,
        );
        tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        // Arrow should have despawned (moved into the wall).
        assert_eq!(ecs.query::<&ProjectileEntity>().iter().count(), 0);
    }

    #[test]
    fn tick_projectiles_damages_mob_on_hit() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        // Cow at (5, 70, 0). Cow has width 0.9 (half=0.45) + height 1.4.
        spawn_mob(&mut ecs, MobType::Cow, Vec3::new(5.0, 70.0, 0.0));
        // Arrow at (4.7, 70.5, 0) flying toward the cow at slow speed so the
        // first integration tick lands the arrow centre inside the cow's
        // extended AABB at (5.0, 70.5, 0).
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.7, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            None,
        );
        let cow_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        let before = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        let hits = tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        let after = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(hits, 1);
        assert!(after < before, "cow should have taken damage (before={before}, after={after})");
        // Arrow despawned.
        assert_eq!(ecs.query::<&ProjectileEntity>().iter().count(), 0);
    }

    #[test]
    fn projectile_hit_stamps_last_attacker_with_owner() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        spawn_mob(&mut ecs, MobType::Cow, Vec3::new(5.0, 70.0, 0.0));
        // Arrow fired by player 3, on a path that lands inside the cow.
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.7, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            Some(3),
        );
        let cow_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        let la = ecs.get::<&crate::combat::LastAttacker>(cow_id);
        assert!(la.is_ok(), "projectile hit must stamp LastAttacker");
        assert_eq!(la.unwrap().0, 3, "must credit the firing player, not proximity");
    }

    #[test]
    fn projectile_hit_without_owner_leaves_no_last_attacker() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        spawn_mob(&mut ecs, MobType::Cow, Vec3::new(5.0, 70.0, 0.0));
        // Ownerless (mob-fired / test) projectile — no attribution stamp,
        // so kill-attribution falls back to proximity as before.
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.7, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            None,
        );
        let cow_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        assert!(
            ecs.get::<&crate::combat::LastAttacker>(cow_id).is_err(),
            "ownerless projectile must not stamp LastAttacker"
        );
    }

    #[test]
    fn arrow_hit_provokes_bear_into_aggro() {
        // Task 13 — an arrow landing on a Bear must provoke it the same as
        // a melee hit (via `combat::notify_hit_bear_or_hyena`).
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let bear = spawn_mob(&mut ecs, MobType::Bear, Vec3::new(5.0, 70.0, 0.0));
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.4, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            Some(6),
        );
        tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        let data = ecs.get::<&crate::bear_ai::BearData>(bear).expect("bear has BearData");
        match data.state {
            crate::bear_ai::BearAiState::Aggro { attacker_pidx, .. } => {
                assert_eq!(attacker_pidx, 6, "should target the shooting player, not proximity");
            }
            other => panic!("expected Aggro after an arrow hit, got {other:?}"),
        }
    }

    #[test]
    fn tick_projectiles_empty_world_no_op() {
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let hits = tick_projectiles(&mut ecs, &world, &registry, &Default::default());
        assert_eq!(hits, 0);
    }

    // --- 1C no-friendly-fire (arrow path) ---

    #[test]
    fn arrow_from_own_shooter_passes_through_own_pet_unless_sneaking() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let wolf = spawn_mob(&mut ecs, MobType::Wolf, Vec3::new(5.0, 70.0, 0.0));
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-3".into();
        let _ = ecs.insert_one(wolf, wd);
        // Arrow fired by player 3 (the wolf's own owner), not sneaking.
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.7, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            Some(3),
        );
        let before = ecs.get::<&crate::combat::Health>(wolf).unwrap().current;
        let not_sneaking: std::collections::HashMap<usize, bool> =
            [(3usize, false)].into_iter().collect();
        let hits = tick_projectiles(&mut ecs, &world, &registry, &not_sneaking);
        let after = ecs.get::<&crate::combat::Health>(wolf).unwrap().current;
        assert_eq!(hits, 1, "arrow still registers a hit-detection pass");
        assert_eq!(after, before, "own pet takes no damage while shooter isn't sneaking");
        assert!(
            ecs.get::<&crate::combat::LastAttacker>(wolf).is_err(),
            "a shielded hit must not stamp LastAttacker"
        );
    }

    #[test]
    fn arrow_from_own_shooter_damages_own_pet_when_sneaking() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let wolf = spawn_mob(&mut ecs, MobType::Wolf, Vec3::new(5.0, 70.0, 0.0));
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-3".into();
        let _ = ecs.insert_one(wolf, wd);
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.7, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            Some(3),
        );
        let before = ecs.get::<&crate::combat::Health>(wolf).unwrap().current;
        let sneaking: std::collections::HashMap<usize, bool> =
            [(3usize, true)].into_iter().collect();
        tick_projectiles(&mut ecs, &world, &registry, &sneaking);
        let after = ecs.get::<&crate::combat::Health>(wolf).unwrap().current;
        assert!(after < before, "sneaking is a deliberate hit — damage must land");
    }

    #[test]
    fn arrow_from_other_player_still_damages_a_pet_they_dont_own() {
        use crate::mob::MobType;
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let wolf = spawn_mob(&mut ecs, MobType::Wolf, Vec3::new(5.0, 70.0, 0.0));
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-3".into();
        let _ = ecs.insert_one(wolf, wd);
        // Fired by player 0, who does not own this wolf — normal PvE damage.
        spawn_arrow(
            &mut ecs,
            Vec3::new(4.7, 70.5, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            5.0,
            Some(0),
        );
        let before = ecs.get::<&crate::combat::Health>(wolf).unwrap().current;
        let not_sneaking: std::collections::HashMap<usize, bool> =
            [(0usize, false)].into_iter().collect();
        tick_projectiles(&mut ecs, &world, &registry, &not_sneaking);
        let after = ecs.get::<&crate::combat::Health>(wolf).unwrap().current;
        assert!(after < before, "a non-owner's arrow must still land");
    }

    #[test]
    fn flying_mob_is_gravity_exempt_grounded_mob_is_not() {
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        // A flyer (bee) and a grounded mob, both starting at rest in open air.
        let flyer = ecs.spawn((
            Position(Vec3::new(0.0, 80.0, 0.0)),
            Velocity(Vec3::ZERO),
            Hitbox { width: 0.4, height: 0.4 },
            OnGround(false),
            Flying,
        ));
        let grounded = ecs.spawn((
            Position(Vec3::new(8.0, 80.0, 0.0)),
            Velocity(Vec3::ZERO),
            Hitbox { width: 0.4, height: 0.4 },
            OnGround(false),
        ));
        tick_entities(&mut ecs, &world, &registry);
        let flyer_vy = ecs.get::<&Velocity>(flyer).unwrap().0.y;
        let grounded_vy = ecs.get::<&Velocity>(grounded).unwrap().0.y;
        assert_eq!(flyer_vy, 0.0, "a Flying mob must not accumulate gravity");
        assert!(grounded_vy < 0.0, "a grounded mob must fall, got vy={grounded_vy}");
    }

    #[test]
    fn thrown_item_blocks_dropper_during_delay() {
        let mut ecs = hecs::World::new();
        spawn_thrown_item(
            &mut ecs,
            Vec3::new(0.0, 64.0, 0.0),
            Vec3::ZERO,
            one_bone_stack(),
            0, // player 0 dropped it
        );
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert!(picked.is_empty(), "dropper must be blocked during delay");
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 1);
    }

    #[test]
    fn thrown_item_picked_up_by_other_player_instantly() {
        let mut ecs = hecs::World::new();
        spawn_thrown_item(
            &mut ecs,
            Vec3::new(0.0, 64.0, 0.0),
            Vec3::ZERO,
            one_bone_stack(),
            0, // player 0 dropped it
        );
        // Player 1 (different from dropper) is right on top of it.
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(1, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert_eq!(picked.len(), 1, "non-dropper must bypass delay");
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn thrown_item_dropper_picks_up_after_delay() {
        let mut ecs = hecs::World::new();
        spawn_thrown_item(
            &mut ecs,
            Vec3::new(0.0, 64.0, 0.0),
            Vec3::ZERO,
            one_bone_stack(),
            0,
        );
        for _ in 0..ITEM_DROP_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> =
            vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        let picked = tick_item_pickups(&mut ecs, &mut players, |_| true);
        assert_eq!(picked.len(), 1, "dropper picks up after 1.5s");
        assert_eq!(ecs.query::<&ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn thrown_item_delay_constant_is_one_and_a_half_seconds() {
        // 20 TPS engine — 1.5s = 30 ticks.
        assert_eq!(ITEM_DROP_PICKUP_DELAY_TICKS, 30);
    }

    #[test]
    fn magnet_pulls_velocity_toward_player() {
        let mut ecs = hecs::World::new();
        spawn_item(&mut ecs, Vec3::new(2.0, 64.0, 0.0), one_bone_stack(), 1);
        for _ in 0..ITEM_PICKUP_DELAY_TICKS {
            tick_item_lifetimes(&mut ecs);
        }
        // Reset velocity so magnet effect is observable on its own.
        for (_, vel) in ecs.query_mut::<&mut Velocity>() {
            vel.0 = Vec3::ZERO;
        }
        let mut inv = Inventory::new();
        let mut players: Vec<(usize, Vec3, &mut Inventory)> = vec![(0, Vec3::new(0.0, 64.0, 0.0), &mut inv)];
        tick_item_pickups(&mut ecs, &mut players, |_| true);
        let vel_x = ecs.query::<&Velocity>().iter().next().map(|(_, v)| v.0.x).unwrap();
        assert!(vel_x < 0.0, "magnet should pull item toward player on -X (got {vel_x})");
    }
}

