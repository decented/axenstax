//! Mob AI — idle/wander for all mobs, chase for hostile mobs.
//!
//! No pathfinding yet — direct-line movement with ground following,
//! 1-block step-up, and edge avoidance for passive mobs.

use std::borrow::Cow;

use glam::Vec3;
use crate::block::BlockRegistry;
use crate::companion;
use crate::entity::{Hitbox, MobKind, OnGround, Position, Velocity};
use crate::mob::{self, MobCategory, MobType};
use crate::world::World;

/// Detection range for hostile mobs to notice the player.
const DETECT_RANGE: f32 = 16.0;
/// Distance at which hostile mobs give up chasing.
const LOSE_RANGE: f32 = 32.0;
/// How close to target before considering "arrived".
/// Hostile mobs re-check for player every N ticks.
const SCAN_INTERVAL: u32 = 10;
/// Detection range for any mob to spot a lit campfire (smoke + glow).
/// Beacons attract from further than the player-detect range — light
/// carries.
const CAMPFIRE_DETECT_RANGE: f32 = 12.0;
/// How many ticks a mob holds at the heat boundary before losing
/// interest (10 s @ 20 TPS).
const CAMPFIRE_INVESTIGATE_HOLD_TICKS: u32 = 200;

#[derive(Clone, Debug)]
pub enum AiState {
    /// Standing still, waiting for timer to expire.
    Idle { timer: u32 },
    /// Walking forward in the mob's facing direction for a set number of ticks.
    Wander { timer: u32 },
    /// Hostile mob chasing the player.
    Chase,
    /// Prey animal bolting away from the nearest player after being struck
    /// (P2 gap-closure). Runs directly away at a species-boosted speed until
    /// the timer expires, then returns to Idle. Triggered by `combat` stamping
    /// this state on a struck prey mob ([`mob::flees_when_attacked`]).
    Flee { timer: u32 },
    /// Investigating a lit campfire. Mob walks toward it but stops at
    /// the heat-radius. Hostile mobs in this state still scan for
    /// nearby players (player nearer than the campfire = Chase). Held
    /// for [`CAMPFIRE_INVESTIGATE_HOLD_TICKS`] ticks then back to Idle.
    /// Spec 18 — beacon-attracts / heat-holds-back behaviour.
    InvestigateCampfire {
        target: [i32; 3],
        heat_radius: f32,
        hold_ticks: u32,
    },
    /// Spec 19 phase 8 — village defender (the Knight) guarding a village.
    /// Patrols within
    /// `GOLEM_PATROL_RADIUS` of the home, scans every `SCAN_INTERVAL` ticks
    /// for hostile mobs within `GOLEM_DEFEND_RADIUS`. The home position is
    /// the world-space anchor of the village it spawned for (Phase 4's
    /// `VillageLayout::anchor_world`).
    GolemGuard {
        home_x: i32,
        home_z: i32,
    },
}

/// How long a struck prey animal bolts before settling back to Idle
/// (3 s @ 20 TPS). Long enough to clear the player's reach, short enough
/// that a herd doesn't stampede off the map.
pub const FLEE_TICKS: u32 = 60;

/// How far an Iron Golem strays from its home village while patrolling.
pub const GOLEM_PATROL_RADIUS: f32 = 14.0;
/// How far an Iron Golem will reach to defend its village from hostiles.
pub const GOLEM_DEFEND_RADIUS: f32 = 20.0;
/// Reach at which a village defender deals melee damage on a hostile mob.
pub const GOLEM_MELEE_REACH: f32 = 1.8;
/// Legacy village-defender base damage (the Iron Golem's old 7-HP swing).
/// Retained for reference; the live Knight defender uses `defender_damage`.
#[allow(dead_code)]
pub const GOLEM_DAMAGE: f32 = 7.0;
/// Ticks between village-defender melee swings.
pub const GOLEM_ATTACK_COOLDOWN: u32 = 20;

/// AI component attached to each mob entity.
#[derive(Clone, Debug)]
pub struct MobAi {
    pub state: AiState,
    /// Ticks since spawn (for scan timing).
    pub ticks: u32,
    /// Current facing direction in radians (persists across idle/wander).
    pub facing: f32,
    /// `ticks` value at the mob's last melee attack (village-defender path).
    /// `None` = never attacked → ready. Readiness compares against this rather
    /// than a spawn-relative `ticks % cooldown`, which fired only on cadence-
    /// aligned ticks and could double-hit or skip swings (engine audit
    /// 2026-06-04, E: golem/knight attack cadence).
    pub last_attack: Option<u32>,
}

impl MobAi {
    pub fn new() -> Self {
        Self {
            state: AiState::Idle { timer: 40 },
            ticks: 0,
            facing: 0.0,
            last_attack: None,
        }
    }
}

/// Find the nearest player position from a mob's location. An empty slice
/// (every candidate filtered out — see the Cat-ward `targets` filter below)
/// reports back as "infinitely far", i.e. no target at all, rather than
/// panicking on `player_positions[0]`.
fn nearest_player(mob_pos: Vec3, player_positions: &[Vec3]) -> (Vec3, f32) {
    if player_positions.is_empty() {
        return (mob_pos, f32::INFINITY);
    }
    let mut best_pos = player_positions[0];
    let mut best_dist = (best_pos - mob_pos).length();
    for &pp in &player_positions[1..] {
        let d = (pp - mob_pos).length();
        if d < best_dist {
            best_dist = d;
            best_pos = pp;
        }
    }
    (best_pos, best_dist)
}

/// Pets wave Task 9 — the explicit set of hostile species that respect a
/// tamed Cat's threat-ward. Deliberately narrower than `MobCategory::Hostile`
/// (which also covers Bear) — a Cat wards off human-descended looters, not a
/// wild predator that doesn't reason about a housecat.
fn respects_cat_ward(kind: MobType) -> bool {
    matches!(kind, MobType::Brigand | MobType::Marauder | MobType::Berserker | MobType::Hyena)
}

/// Run one AI tick for all mobs. Call at 20 TPS.
pub fn tick_mob_ai(
    ecs: &mut hecs::World,
    world: &World,
    registry: &BlockRegistry,
    player_positions: &[Vec3],
) {
    if player_positions.is_empty() { return; }

    // Pets wave Task 9 — every tamed cat's position, computed once per tick
    // (not per mob) for the threat-ward check below.
    let cats = companion::tamed_cat_positions(ecs);

    // Collect entity updates to avoid borrow conflicts
    let mut updates: Vec<(hecs::Entity, AiState, Vec3)> = Vec::new();

    for (id, (pos, vel, kind, hitbox, _on_ground, ai, ridden)) in ecs
        .query_mut::<(
            &Position,
            &mut Velocity,
            &MobKind,
            &Hitbox,
            &OnGround,
            &mut MobAi,
            Option<&crate::entity::Ridden>,
        )>()
    {
        // P9 — a ridden mob is steered by its rider, not its wander AI.
        if ridden.is_some() {
            continue;
        }
        ai.ticks = ai.ticks.wrapping_add(1);
        let def = mob::mob_def(kind.0);
        let speed_per_tick = def.speed / 20.0;

        // Swim-to-shore override. can_walk_to now refuses to step into
        // water, so a mob is only ever submerged via combat knockback or a
        // spawn right at the water's edge. While its feet are in water,
        // skip the normal state machine and steer straight for the nearest
        // dry land; buoyancy (entity::tick_entities) floats it up to the
        // surface as it goes. Velocity is set directly because can_walk_to
        // would veto every water-cell step (2026-05-30 playtest).
        //
        // #10 — aquatic mobs (Squid) LIVE in water and suffocate on land, so
        // they must be exempt from the swim-to-shore march.
        let is_aquatic = matches!(
            kind.0,
            crate::mob::MobType::Squid
                | crate::mob::MobType::Fish
                | crate::mob::MobType::Shark
                | crate::mob::MobType::GlowSquid
        );
        if !is_aquatic && world.is_water(
            pos.0.x.floor() as i32,
            pos.0.y.floor() as i32,
            pos.0.z.floor() as i32,
        ) {
            if let Some(facing) = swim_dir_to_land(pos.0, world, registry) {
                ai.facing = facing;
                let dir = Vec3::new(facing.cos(), 0.0, facing.sin());
                vel.0.x = dir.x * speed_per_tick;
                vel.0.z = dir.z * speed_per_tick;
            } else {
                // Open water, no land within reach (#10b): hold still and float
                // rather than running the wander state machine, which made a
                // stranded mob thrash/bob in place on a big lake.
                vel.0.x = 0.0;
                vel.0.z = 0.0;
            }
            continue;
        }

        // Pets wave Task 9 — a hostile that respects the Cat ward and is
        // standing inside one steers directly away from the nearest cat at
        // half speed, overriding the state machine for this tick (mirrors
        // the `AiState::Flee` away-from-target steering above, and the
        // swim-to-shore override's use of `continue`).
        if respects_cat_ward(kind.0) && !cats.is_empty() && companion::warded(pos.0, &cats) {
            let nearest_cat = cats
                .iter()
                .copied()
                .min_by(|a, b| {
                    (*a - pos.0).length_squared()
                        .partial_cmp(&(*b - pos.0).length_squared())
                        .unwrap()
                })
                .expect("cats non-empty, checked above");
            let away = pos.0 - nearest_cat;
            let mut dir = Vec3::new(away.x, 0.0, away.z).normalize_or_zero();
            if dir.length_squared() < 1e-6 {
                // Standing right on top of the cat — keep current facing
                // rather than dividing by zero into a NaN velocity.
                dir = Vec3::new(ai.facing.cos(), 0.0, ai.facing.sin());
            }
            ai.facing = dir.z.atan2(dir.x);
            let flee_speed = speed_per_tick * 0.5;
            vel.0.x = dir.x * flee_speed;
            vel.0.z = dir.z * flee_speed;
            continue;
        }

        // Pets wave Task 9 — for a ward-respecting hostile, target
        // selection must skip any player standing inside a tamed cat's
        // ward. `Cow` avoids an allocation for the common case (no cats,
        // or a non-ward species) where the raw `player_positions` is used
        // unfiltered.
        let targets: Cow<[Vec3]> = if respects_cat_ward(kind.0) && !cats.is_empty() {
            Cow::Owned(
                player_positions
                    .iter()
                    .copied()
                    .filter(|&p| !companion::warded(p, &cats))
                    .collect(),
            )
        } else {
            Cow::Borrowed(player_positions)
        };

        match &ai.state {
            AiState::Idle { timer } => {
                // Stop horizontal movement while idle
                vel.0.x = 0.0;
                vel.0.z = 0.0;

                if *timer == 0 {
                    // Hostile mobs: check for nearest player
                    if def.category == MobCategory::Hostile {
                        let (_, dist) = nearest_player(pos.0, &targets);
                        if dist < DETECT_RANGE {
                            updates.push((id, AiState::Chase, Vec3::ZERO));
                            continue;
                        }
                    }

                    // No player to chase — check for a lit campfire to
                    // investigate. Both hostile and passive mobs are drawn
                    // by smoke + glow (Spec 18). Heat holds them back at
                    // a radius proportional to fuel level.
                    if let Some((cf_pos, heat_radius)) = crate::campfire::nearest_lit_campfire(
                        world, pos.0, CAMPFIRE_DETECT_RANGE,
                    ) {
                        updates.push((id, AiState::InvestigateCampfire {
                            target: [cf_pos.0, cf_pos.1, cf_pos.2],
                            heat_radius,
                            hold_ticks: 0,
                        }, Vec3::ZERO));
                        continue;
                    }

                    // Pick a new facing: small turn from current direction (±60°)
                    let h = simple_hash(ai.ticks, id);
                    let turn = ((h % 120) as f32 - 60.0) * std::f32::consts::PI / 180.0;
                    ai.facing += turn;
                    // Salt feature — livestock species near a SALT_LICK
                    // blend 25% of their facing toward the nearest lick
                    // within 16 blocks. Gentle pull, not pathfinding.
                    if let Some(bias) = crate::salt_lick::wander_bias_for_salt_lick(
                        world, pos.0, kind.0,
                    ) {
                        let cur = Vec3::new(ai.facing.cos(), 0.0, ai.facing.sin());
                        let bias_xz = Vec3::new(bias.x, 0.0, bias.z).normalize_or_zero();
                        let blended = (cur * 0.75) + (bias_xz * 0.25);
                        if blended.length_squared() > 1e-6 {
                            ai.facing = blended.z.atan2(blended.x);
                        }
                    }
                    let walk_ticks = 40 + h % 80; // Walk for 2-6 seconds
                    updates.push((id, AiState::Wander { timer: walk_ticks }, Vec3::ZERO));
                } else {
                    ai.state = AiState::Idle { timer: timer - 1 };
                }
            }

            AiState::Wander { timer } => {
                if *timer == 0 {
                    // Done walking — go back to idle
                    vel.0.x = 0.0;
                    vel.0.z = 0.0;
                    let idle_time = 60 + simple_hash(ai.ticks, id) % 100;
                    updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                    continue;
                }
                ai.state = AiState::Wander { timer: timer - 1 };

                // Walk forward in facing direction
                let dir = Vec3::new(ai.facing.cos(), 0.0, ai.facing.sin());
                let new_vel = dir * speed_per_tick;

                // Check walkability
                let next_pos = pos.0 + new_vel;
                if can_walk_to(next_pos, hitbox, world, registry, def.category == MobCategory::Passive) {
                    vel.0.x = new_vel.x;
                    vel.0.z = new_vel.z;
                } else {
                    // Stuck — go back to idle
                    vel.0.x = 0.0;
                    vel.0.z = 0.0;
                    let idle_time = 30 + simple_hash(ai.ticks, id) % 60;
                    updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                }

                // Hostile mobs: periodically check for nearest player while wandering
                if def.category == MobCategory::Hostile && ai.ticks % SCAN_INTERVAL == 0 {
                    let (_, dist) = nearest_player(pos.0, &targets);
                    if dist < DETECT_RANGE {
                        updates.push((id, AiState::Chase, Vec3::ZERO));
                    }
                }
            }

            AiState::Chase => {
                let (target_pos, dist) = nearest_player(pos.0, &targets);
                let to_player = target_pos - pos.0;

                // Give up if all players are too far
                if dist > LOSE_RANGE {
                    let idle_time = 60 + simple_hash(ai.ticks, id) % 100;
                    updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                    continue;
                }

                // Face toward nearest player
                ai.facing = to_player.z.atan2(to_player.x);

                // Move toward player
                let dir = Vec3::new(to_player.x, 0.0, to_player.z).normalize_or_zero();
                let new_vel = dir * speed_per_tick;

                let next_pos = pos.0 + new_vel;
                if can_walk_to(next_pos, hitbox, world, registry, false) {
                    vel.0.x = new_vel.x;
                    vel.0.z = new_vel.z;
                } else {
                    // Try to step up
                    let step_pos = Vec3::new(next_pos.x, next_pos.y + 1.0, next_pos.z);
                    if can_walk_to(step_pos, hitbox, world, registry, false) {
                        vel.0.x = new_vel.x;
                        vel.0.z = new_vel.z;
                    } else {
                        vel.0.x = 0.0;
                        vel.0.z = 0.0;
                    }
                }

                // Re-check player distance periodically to re-acquire
                if ai.ticks % SCAN_INTERVAL == 0 && dist > DETECT_RANGE {
                    let idle_time = 40 + simple_hash(ai.ticks, id) % 40;
                    updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                }
            }

            AiState::Flee { timer } => {
                // P2 — struck prey bolts away from the nearest player.
                if *timer == 0 {
                    vel.0.x = 0.0;
                    vel.0.z = 0.0;
                    let idle_time = 40 + simple_hash(ai.ticks, id) % 40;
                    updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                    continue;
                }
                ai.state = AiState::Flee { timer: timer - 1 };

                let (target_pos, _dist) = nearest_player(pos.0, player_positions);
                let away = pos.0 - target_pos;
                let mut dir = Vec3::new(away.x, 0.0, away.z).normalize_or_zero();
                if dir.length_squared() < 1e-6 {
                    // Player directly overhead — keep current facing so we
                    // don't divide by zero into a NaN velocity.
                    dir = Vec3::new(ai.facing.cos(), 0.0, ai.facing.sin());
                }
                ai.facing = dir.z.atan2(dir.x);
                let flee_speed = speed_per_tick * mob::flee_speed_mult(kind.0);
                let new_vel = dir * flee_speed;
                let next_pos = pos.0 + new_vel;
                // Edge-avoid (true): a panicked animal shouldn't bolt off a cliff.
                if can_walk_to(next_pos, hitbox, world, registry, true) {
                    vel.0.x = new_vel.x;
                    vel.0.z = new_vel.z;
                } else {
                    let step_pos = Vec3::new(next_pos.x, next_pos.y + 1.0, next_pos.z);
                    if can_walk_to(step_pos, hitbox, world, registry, true) {
                        vel.0.x = new_vel.x;
                        vel.0.z = new_vel.z;
                    } else {
                        // Cornered — veer rather than grind into the wall.
                        let h = simple_hash(ai.ticks, id);
                        let turn = ((h % 120) as f32 - 60.0) * std::f32::consts::PI / 180.0;
                        ai.facing += turn;
                        vel.0.x = 0.0;
                        vel.0.z = 0.0;
                    }
                }
            }

            AiState::InvestigateCampfire { target, heat_radius, hold_ticks } => {
                // Spec 18 — mob attracted to a lit campfire. Walks toward
                // it, stops at the heat radius, holds for a while, then
                // loses interest.
                let target_centre = Vec3::new(
                    target[0] as f32 + 0.5,
                    target[1] as f32 + 0.5,
                    target[2] as f32 + 0.5,
                );
                let to_target = target_centre - pos.0;
                let dist = to_target.length();

                // Bail if the campfire is no longer lit (or has been
                // destroyed) — defence in depth.
                let still_lit = world
                    .campfire_at((target[0], target[1], target[2]))
                    .map(|cf| cf.is_lit())
                    .unwrap_or(false);
                if !still_lit {
                    let idle_time = 40 + simple_hash(ai.ticks, id) % 40;
                    updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                    continue;
                }

                // Hostile-mob priority: if a player is closer than the
                // campfire (and within DETECT_RANGE), switch to Chase.
                // Beacon held them in the area; the prey is the reward.
                if def.category == MobCategory::Hostile {
                    let (_, player_dist) = nearest_player(pos.0, &targets);
                    if player_dist < DETECT_RANGE && player_dist < dist {
                        updates.push((id, AiState::Chase, Vec3::ZERO));
                        continue;
                    }
                }

                // Outside the heat radius — walk toward the fire.
                if dist > heat_radius + 0.5 {
                    ai.facing = to_target.z.atan2(to_target.x);
                    let dir = Vec3::new(to_target.x, 0.0, to_target.z).normalize_or_zero();
                    let new_vel = dir * speed_per_tick;
                    let next_pos = pos.0 + new_vel;
                    if can_walk_to(next_pos, hitbox, world, registry, false) {
                        vel.0.x = new_vel.x;
                        vel.0.z = new_vel.z;
                    } else {
                        vel.0.x = 0.0;
                        vel.0.z = 0.0;
                    }
                    // Keep hold_ticks at 0 while still approaching.
                    ai.state = AiState::InvestigateCampfire {
                        target: *target,
                        heat_radius: *heat_radius,
                        hold_ticks: 0,
                    };
                } else {
                    // At the heat boundary — too hot to advance. Hold
                    // for a while, then lose interest.
                    vel.0.x = 0.0;
                    vel.0.z = 0.0;
                    if *hold_ticks >= CAMPFIRE_INVESTIGATE_HOLD_TICKS {
                        let idle_time = 60 + simple_hash(ai.ticks, id) % 100;
                        updates.push((id, AiState::Idle { timer: idle_time }, Vec3::ZERO));
                    } else {
                        ai.state = AiState::InvestigateCampfire {
                            target: *target,
                            heat_radius: *heat_radius,
                            hold_ticks: hold_ticks + 1,
                        };
                    }
                }
            }

            AiState::GolemGuard { home_x, home_z } => {
                // Spec 19 phase 8 — guard wander. Targeting hostile mobs +
                // actually damaging them is handled in `tick_golem_combat`
                // since this query already mutates `&mut MobAi`/`&mut Velocity`
                // and we can't open a second mutable borrow on the ECS here.
                let home = Vec3::new(*home_x as f32 + 0.5, pos.0.y, *home_z as f32 + 0.5);
                let to_home = home - pos.0;
                let dist_home = to_home.length();

                // Wander home if we've strayed too far from the village.
                if dist_home > GOLEM_PATROL_RADIUS {
                    ai.facing = to_home.z.atan2(to_home.x);
                    let dir = Vec3::new(to_home.x, 0.0, to_home.z).normalize_or_zero();
                    let new_vel = dir * speed_per_tick;
                    let next_pos = pos.0 + new_vel;
                    if can_walk_to(next_pos, hitbox, world, registry, false) {
                        vel.0.x = new_vel.x;
                        vel.0.z = new_vel.z;
                    }
                } else {
                    // Inside patrol radius: walk forward at a slow plod.
                    let dir = Vec3::new(ai.facing.cos(), 0.0, ai.facing.sin());
                    let new_vel = dir * speed_per_tick * 0.6;
                    let next_pos = pos.0 + new_vel;
                    if can_walk_to(next_pos, hitbox, world, registry, false) {
                        vel.0.x = new_vel.x;
                        vel.0.z = new_vel.z;
                    } else {
                        // Bumped into something — turn deterministically.
                        let h = simple_hash(ai.ticks, id);
                        let turn = ((h % 180) as f32 - 90.0) * std::f32::consts::PI / 180.0;
                        ai.facing += turn;
                        vel.0.x = 0.0;
                        vel.0.z = 0.0;
                    }
                }
            }
        }
    }

    // Apply state transitions
    for (id, new_state, _) in updates {
        if let Ok(mut ai) = ecs.get::<&mut MobAi>(id) {
            ai.state = new_state;
        }
    }
}

/// HP-4 — village-defender kinds. The Knight is the sole defender after
/// the fantasy roster (incl. the Iron Golem) was excised. Used by
/// `tick_golem_combat` to pick which entities loop the defender
/// pattern; engine-generic so future tiers (e.g. Captain Knight v2)
/// only need an enum arm.
pub fn is_village_defender(kind: MobType) -> bool {
    matches!(kind, MobType::Knight)
}

/// HP-4 — per-defender melee damage. The Knight hits 8 — reads as the
/// "humans with sharper steel" feel that replaced the Iron Golem's swing.
pub fn defender_damage(kind: MobType) -> f32 {
    match kind {
        MobType::Knight => 8.0,
        _ => 0.0,
    }
}

/// Spec 19 phase 8 / HP-4 — Village-defender combat. Per tick: find
/// every defender (the Knight); for each, find the nearest
/// hostile mob within `GOLEM_DEFEND_RADIUS`; if within `GOLEM_MELEE_REACH`,
/// apply the kind's `defender_damage` (rate-limited by `GOLEM_ATTACK_COOLDOWN`
/// on the defender's `MobAi`). Returns the number of hits this tick.
pub fn tick_golem_combat(ecs: &mut hecs::World) -> u32 {
    use crate::combat::Health;

    // Collect defenders + hostile mobs first to avoid query overlap during damage.
    let mut defenders: Vec<(hecs::Entity, Vec3, MobType)> = Vec::new();
    let mut hostiles: Vec<(hecs::Entity, Vec3, f32)> = Vec::new();
    for (id, (pos, kind)) in ecs.query::<(&Position, &MobKind)>().iter() {
        if is_village_defender(kind.0) {
            defenders.push((id, pos.0, kind.0));
        } else if mob::mob_def(kind.0).category == MobCategory::Hostile {
            hostiles.push((id, pos.0, mob::mob_def(kind.0).height));
        }
    }
    if defenders.is_empty() || hostiles.is_empty() {
        return 0;
    }

    let mut hits = 0u32;
    for (gid, gpos, gkind) in defenders {
        let (ready, gticks) = match ecs.get::<&MobAi>(gid) {
            Ok(ai) => (
                ai.last_attack
                    .is_none_or(|la| ai.ticks.wrapping_sub(la) >= GOLEM_ATTACK_COOLDOWN),
                ai.ticks,
            ),
            Err(_) => (false, 0),
        };
        if !ready {
            continue;
        }
        let mut best: Option<(hecs::Entity, f32)> = None;
        for (hid, hpos, _) in &hostiles {
            let d = (*hpos - gpos).length();
            if d <= GOLEM_DEFEND_RADIUS
                && best.map(|(_, bd)| d < bd).unwrap_or(true)
            {
                best = Some((*hid, d));
            }
        }
        let Some((target, dist)) = best else { continue };
        if dist <= GOLEM_MELEE_REACH
            && let Ok(mut health) = ecs.get::<&mut Health>(target) {
                let _ = health.take_damage(defender_damage(gkind));
                hits += 1;
                // Stamp the actual hit so the cooldown tracks real swings, not a
                // spawn-relative modulo.
                if let Ok(mut ai) = ecs.get::<&mut MobAi>(gid) {
                    ai.last_attack = Some(gticks);
                }
            }
    }
    hits
}

/// When a mob is standing in water, pick a facing angle (radians, in the
/// same convention as `MobAi::facing` → `dir = (cos, 0, sin)`) toward the
/// nearest dry, standable land within a short radius. Returns `None` if no
/// land is found (open water), letting the caller fall back to normal AI.
/// Used by the swim-to-shore override (2026-05-30 playtest: "if dragged
/// into water they should swim to the edge and get out").
pub(crate) fn swim_dir_to_land(pos: Vec3, world: &World, registry: &BlockRegistry) -> Option<f32> {
    // #10b — raised 8 → 16: on lakes wider than the old radius the scan found
    // no shore and the mob fell back to wander, reading as "bobbing in place".
    const MAX_R: i32 = 16;
    let px = pos.x.floor() as i32;
    let pz = pos.z.floor() as i32;
    let feet_y = pos.y.floor() as i32;
    let mut best: Option<(i32, i32, i32)> = None; // (dist², dx, dz)
    for dx in -MAX_R..=MAX_R {
        for dz in -MAX_R..=MAX_R {
            if dx == 0 && dz == 0 {
                continue;
            }
            let (cx, cz) = (px + dx, pz + dz);
            // Dry, standable: the foot cell isn't water and there's solid
            // ground directly beneath it.
            if !world.is_water(cx, feet_y, cz) && world.is_solid(cx, feet_y - 1, cz, registry) {
                let d2 = dx * dx + dz * dz;
                if best.is_none_or(|(bd, _, _)| d2 < bd) {
                    best = Some((d2, dx, dz));
                }
            }
        }
    }
    best.map(|(_, dx, dz)| (dz as f32).atan2(dx as f32))
}

/// Check if a mob can walk to a position.
fn can_walk_to(
    pos: Vec3,
    hitbox: &Hitbox,
    world: &World,
    registry: &BlockRegistry,
    avoid_edges: bool,
) -> bool {
    let half_w = hitbox.width / 2.0;
    let foot_y = (pos.y - 0.01).floor() as i32;

    // Check corners for ground below
    let corners = [
        (pos.x - half_w, pos.z - half_w),
        (pos.x + half_w, pos.z - half_w),
        (pos.x - half_w, pos.z + half_w),
        (pos.x + half_w, pos.z + half_w),
    ];

    let mut has_ground = false;
    for (cx, cz) in corners {
        let bx = cx.floor() as i32;
        let bz = cz.floor() as i32;

        // Check for solid ground below feet
        if world.is_solid(bx, foot_y, bz, registry) {
            has_ground = true;
        }

        // Check for obstruction at feet and head height
        let feet_y = foot_y + 1;
        let head_y = (pos.y + hitbox.height).floor() as i32;
        for y in feet_y..=head_y {
            if world.is_solid(bx, y, bz, registry) {
                return false; // Wall in the way
            }
            // Refuse to wade in: water is non-solid so it slips past the
            // obstruction check, but a step whose body cell is water means
            // the mob would be standing in the water. Mobs avoid water by
            // choice (2026-05-30 playtest); a mob already submerged escapes
            // via the swim-to-shore override, which bypasses can_walk_to.
            if world.is_water(bx, y, bz) {
                return false;
            }
        }
    }

    if !has_ground {
        if avoid_edges {
            return false; // Passive mobs don't walk off edges
        }
        // Hostile mobs: check drop distance
        let mut drop = 0;
        let check_x = pos.x.floor() as i32;
        let check_z = pos.z.floor() as i32;
        for dy in 1..=4 {
            if world.is_solid(check_x, foot_y - dy, check_z, registry) {
                drop = dy;
                break;
            }
        }
        if drop == 0 || drop > 3 {
            return false; // Too high a drop or bottomless
        }
    }

    true
}

/// Simple hash for deterministic randomness.
fn simple_hash(ticks: u32, entity: hecs::Entity) -> u32 {
    let id = entity.id();
    let mut h = ticks.wrapping_mul(374761393) ^ id.wrapping_mul(668265263);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockRegistry, STONE};
    use crate::entity;
    use crate::mob::MobType;

    fn stone_floor(world: &mut World, y: i32) {
        for x in -20..=20 {
            for z in -20..=20 {
                world.set_block(x, y, z, STONE);
            }
        }
    }

    #[test]
    fn defender_filter_covers_knight_only() {
        assert!(is_village_defender(MobType::Knight));
        assert!(!is_village_defender(MobType::Villager));
        assert!(!is_village_defender(MobType::Brigand));
    }

    #[test]
    fn knight_attack_uses_last_attack_cooldown_not_spawn_modulo() {
        use crate::combat::Health;
        let mut ecs = hecs::World::new();
        let knight = entity::spawn_mob(&mut ecs, MobType::Knight, Vec3::new(0.0, 64.0, 0.0));
        let brigand = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(1.0, 64.0, 0.0));
        // Make the brigand tanky so it survives several hits (isolate cadence).
        if let Ok(mut h) = ecs.get::<&mut Health>(brigand) { *h = Health::new(1000.0); }

        // Never attacked → ready → one hit.
        assert_eq!(tick_golem_combat(&mut ecs), 1, "first swing lands");
        // Immediately again WITHOUT advancing the clock: on cooldown. The old
        // `ticks % 20 == 0` check stayed true at ticks==0 and would double-hit.
        assert_eq!(tick_golem_combat(&mut ecs), 0, "still on cooldown — no double-hit");
        // Advance the knight's local clock past the cooldown → ready again.
        if let Ok(mut ai) = ecs.get::<&mut MobAi>(knight) { ai.ticks = GOLEM_ATTACK_COOLDOWN; }
        assert_eq!(tick_golem_combat(&mut ecs), 1, "ready again once the cooldown elapses");
    }

    #[test]
    fn defender_damage_knight_is_eight() {
        assert_eq!(defender_damage(MobType::Knight), 8.0);
    }

    #[test]
    fn defender_damage_non_defender_is_zero() {
        assert_eq!(defender_damage(MobType::Villager), 0.0);
        assert_eq!(defender_damage(MobType::Brigand), 0.0);
    }

    #[test]
    fn swim_to_land_reaches_within_raised_radius() {
        // #10b — the swim-to-shore search radius was raised 8 -> 16 so a mob on
        // a big lake still finds a shore instead of falling back to wander
        // ("bobbing in place"). A dry cell 12 out is beyond the OLD radius but
        // inside the new one and must now be reachable.
        let registry = BlockRegistry::new();
        let mut world = World::new();
        let fy = 64;
        for dx in -20..=20 {
            for dz in -20..=20 {
                world.set_block(dx, fy - 1, dz, STONE);
                world.set_block(dx, fy, dz, crate::block::WATER);
            }
        }
        let pos = Vec3::new(0.5, fy as f32, 0.5);
        // All water within the window → no shore → fall back (None).
        assert!(swim_dir_to_land(pos, &world, &registry).is_none());
        // Carve a dry, standable cell 12 out (8 < 12 < 16).
        world.set_block(12, fy, 0, crate::block::AIR);
        let dir = swim_dir_to_land(pos, &world, &registry)
            .expect("land at distance 12 must be reachable with MAX_R=16");
        // Faces roughly toward +x where the land is.
        assert!(dir.cos() > 0.5, "should steer toward the land at +x, got angle {dir}");
    }

    #[test]
    fn tick_golem_combat_lets_knight_hit_brigand_in_melee() {
        // Spawn a Knight + a Brigand in melee reach; tick combat at
        // the cooldown boundary; assert the Brigand HP dropped by 8.
        let mut ecs = hecs::World::new();
        let knight_id = entity::spawn_mob(&mut ecs, MobType::Knight, Vec3::new(0.0, 5.0, 0.0));
        // Knight needs a fresh MobAi.ticks of 0 (== cooldown == 20 boundary).
        if let Ok(mut ai) = ecs.get::<&mut MobAi>(knight_id) {
            ai.ticks = 0;
        }
        let brigand_id = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(1.0, 5.0, 0.0));
        let start_hp = ecs.get::<&crate::combat::Health>(brigand_id).unwrap().current;
        let hits = tick_golem_combat(&mut ecs);
        let end_hp = ecs.get::<&crate::combat::Health>(brigand_id).unwrap().current;
        assert!(hits >= 1, "expected at least one hit, got {hits}");
        assert!((start_hp - end_hp - 8.0).abs() < 1e-3,
            "knight should hit for 8 (start {start_hp}, end {end_hp})");
    }

    #[test]
    fn can_walk_to_rejects_stepping_into_water() {
        // 2026-05-30 playtest: "animals walking into water — they
        // shouldn't want to". Water is non-solid, so it never tripped the
        // obstruction check; can_walk_to must explicitly refuse a step
        // whose body would occupy a water cell.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        world.set_block(5, 5, 5, crate::block::WATER); // flood one surface cell
        let reg = BlockRegistry::new();
        let hitbox = crate::entity::Hitbox { width: 0.6, height: 1.8 };
        let dry = Vec3::new(0.5, 5.0, 0.5);
        assert!(
            can_walk_to(dry, &hitbox, &world, &reg, true),
            "dry land should stay walkable",
        );
        let wet = Vec3::new(5.5, 5.0, 5.5);
        assert!(
            !can_walk_to(wet, &hitbox, &world, &reg, true),
            "must refuse a step into a water cell",
        );
    }

    #[test]
    fn swim_dir_to_land_points_away_from_water() {
        // A mob in water should steer toward the nearest dry land. Build a
        // pool with land only on the +x side and assert the chosen facing
        // points roughly +x.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        // Flood x in -3..=2 at the surface (y=5); x>=3 stays dry land.
        for x in -3..=2 {
            for z in -3..=3 {
                world.set_block(x, 5, z, crate::block::WATER);
            }
        }
        let reg = BlockRegistry::new();
        let pos = Vec3::new(0.5, 5.0, 0.5); // sitting in the pool
        let facing = swim_dir_to_land(pos, &world, &reg)
            .expect("should find dry land to head for");
        // +x direction is angle 0; cos(facing) should be strongly positive.
        assert!(
            facing.cos() > 0.5,
            "expected to head toward +x land, facing={facing} cos={}",
            facing.cos(),
        );
    }

    #[test]
    fn nearest_player_picks_closest_across_three() {
        let mob = Vec3::new(0.0, 0.0, 0.0);
        let players = vec![
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),   // closest
            Vec3::new(-5.0, 0.0, 0.0),
        ];
        let (pos, dist) = nearest_player(mob, &players);
        assert_eq!(pos, Vec3::new(2.0, 0.0, 0.0));
        assert!((dist - 2.0).abs() < 1e-4);
    }

    #[test]
    fn empty_players_no_state_change() {
        // With no players, the AI tick must be a no-op (not panic on players[0]).
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        // No panic on empty slice.
        tick_mob_ai(&mut ecs, &world, &reg, &[]);
    }

    #[test]
    fn hostile_mob_within_detect_range_switches_to_chase() {
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        // Force mob into Idle{timer:0} so the next tick checks for chase.
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::Idle { timer: 0 };
        }
        // Player well within DETECT_RANGE (16).
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(5.0, 5.0, 0.0)]);

        let state = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(state.state, AiState::Chase),
            "expected Chase, got {:?}", state.state);
    }

    #[test]
    fn chasing_mob_gives_up_when_player_too_far() {
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::Chase;
        }
        // Player beyond LOSE_RANGE (32).
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(100.0, 5.0, 0.0)]);

        let state = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(state.state, AiState::Idle { .. }),
            "expected Idle after losing target, got {:?}", state.state);
    }

    // --- Spec 18 — Wave 28 InvestigateCampfire behaviour ---

    fn place_lit_campfire(world: &mut World, x: i32, y: i32, z: i32, fuel_ticks: u32) {
        world.set_block(x, y, z, crate::block::CAMPFIRE);
        let mut cf = crate::campfire::CampfireData::default();
        cf.fuel_ticks = fuel_ticks;
        world.insert_campfire((x, y, z), cf);
    }

    #[test]
    fn idle_mob_investigates_nearby_lit_campfire() {
        // No player in range — but a lit campfire is nearby. Mob should
        // transition to InvestigateCampfire on the next tick.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        place_lit_campfire(&mut world, 5, 5, 0, 1200);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::Idle { timer: 0 };
        }
        // Player far away (out of DETECT_RANGE) so the campfire wins.
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(100.0, 5.0, 0.0)]);

        let state = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(state.state, AiState::InvestigateCampfire { .. }),
            "expected InvestigateCampfire, got {:?}", state.state);
    }

    #[test]
    fn investigating_hostile_mob_switches_to_chase_if_player_closer() {
        // Hostile-mob priority: a player closer than the campfire while
        // investigating wins the AI's attention. Spec 18 mob behaviour.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        place_lit_campfire(&mut world, 10, 5, 0, 1200);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::InvestigateCampfire {
                target: [10, 5, 0],
                heat_radius: 5.0,
                hold_ticks: 0,
            };
        }
        // Player at (3,5,0) — closer than the campfire at (10,5,0).
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(3.0, 5.0, 0.0)]);

        let state = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(state.state, AiState::Chase),
            "expected Chase (player closer than campfire), got {:?}", state.state);
    }

    #[test]
    fn investigating_mob_loses_interest_after_hold_window() {
        // After holding at the heat boundary for CAMPFIRE_INVESTIGATE_HOLD_TICKS,
        // the mob goes back to Idle.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        place_lit_campfire(&mut world, 2, 5, 0, 1200);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        // Spawn the mob already at the heat boundary so it goes straight
        // into "holding" rather than walking.
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(2.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::InvestigateCampfire {
                target: [2, 5, 0],
                heat_radius: 10.0,  // generous radius so dist < heat_radius
                hold_ticks: CAMPFIRE_INVESTIGATE_HOLD_TICKS,
            };
        }
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(100.0, 5.0, 0.0)]);

        let state = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(state.state, AiState::Idle { .. }),
            "expected Idle after hold expires, got {:?}", state.state);
    }

    #[test]
    fn investigating_mob_bails_if_campfire_goes_out() {
        // Defence in depth: campfire extinguishes while a mob is
        // investigating → mob back to Idle, doesn't get stuck.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        // Place an UNLIT campfire (no fuel) so still_lit check fails.
        world.set_block(5, 5, 0, crate::block::CAMPFIRE_UNLIT);
        world.insert_campfire((5, 5, 0), crate::campfire::CampfireData::default());
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::InvestigateCampfire {
                target: [5, 5, 0],
                heat_radius: 4.0,
                hold_ticks: 0,
            };
        }
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(100.0, 5.0, 0.0)]);

        let state = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(state.state, AiState::Idle { .. }),
            "expected Idle when campfire no longer lit, got {:?}", state.state);
    }

    #[test]
    fn fleeing_prey_runs_away_from_player() {
        // P2 — a cow in Flee state with the player to its +x runs toward -x.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>().iter().next().map(|(id, _)| id).unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::Flee { timer: FLEE_TICKS };
        }
        // Player at +x → the cow should bolt toward -x.
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(6.0, 5.0, 0.0)]);
        let vel = ecs.get::<&Velocity>(mob_id).unwrap();
        assert!(vel.0.x < 0.0, "fleeing cow should move away from the player (got vx={})", vel.0.x);
        let ai = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(ai.state, AiState::Flee { timer } if timer == FLEE_TICKS - 1),
            "flee timer should tick down once");
    }

    #[test]
    fn flee_expires_back_to_idle() {
        // When the flee timer hits zero the prey settles back to Idle.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Sheep, Vec3::new(0.0, 5.0, 0.0));
        let mob_id: hecs::Entity = ecs
            .query::<&MobKind>().iter().next().map(|(id, _)| id).unwrap();
        {
            let mut ai = ecs.get::<&mut MobAi>(mob_id).unwrap();
            ai.state = AiState::Flee { timer: 0 };
        }
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(6.0, 5.0, 0.0)]);
        let ai = ecs.get::<&MobAi>(mob_id).unwrap();
        assert!(matches!(ai.state, AiState::Idle { .. }),
            "flee timer 0 should return to Idle, got {:?}", ai.state);
    }

    // ── Pets wave Task 9 — Cat threat-ward ────────────────────────────

    fn tame_cat_at(ecs: &mut hecs::World, pos: Vec3) -> hecs::Entity {
        let cat = entity::spawn_mob(ecs, MobType::Cat, pos);
        ecs.get::<&mut crate::companion::CompanionData>(cat)
            .unwrap()
            .ownership
            .owner_pubkey = "local-player-0".to_string();
        cat
    }

    #[test]
    fn hostile_skips_warded_player_and_gives_up_chase() {
        // A Brigand already chasing a player who then becomes warded (inside
        // a tamed cat's radius, but the cat itself is well clear of the
        // Brigand) must lose the target as if no player were nearby, rather
        // than keep closing in.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        let brigand = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 5.0, 0.0));
        {
            let mut ai = ecs.get::<&mut MobAi>(brigand).unwrap();
            ai.state = AiState::Chase;
        }
        // Cat 20 away from the Brigand (not warding the Brigand itself) but
        // only 2 from the player (well inside CAT_WARD_RADIUS = 12).
        tame_cat_at(&mut ecs, Vec3::new(20.0, 5.0, 0.0));
        let player_pos = Vec3::new(18.0, 5.0, 0.0); // 18 from Brigand — within LOSE_RANGE pre-filter
        tick_mob_ai(&mut ecs, &world, &reg, &[player_pos]);
        let ai = ecs.get::<&MobAi>(brigand).unwrap();
        assert!(matches!(ai.state, AiState::Idle { .. }),
            "warded player must not be chased — expected give-up to Idle, got {:?}", ai.state);
    }

    #[test]
    fn hostile_flees_cat_when_standing_in_its_ward() {
        // A Brigand standing inside a tamed cat's ward radius steers
        // directly away from the nearest cat at half its normal speed,
        // regardless of AI state.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        tame_cat_at(&mut ecs, Vec3::new(0.0, 5.0, 0.0));
        let brigand = entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(5.0, 5.0, 0.0));
        // Player far enough away that Chase/detect logic can't interfere.
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(500.0, 5.0, 0.0)]);
        let vel = ecs.get::<&Velocity>(brigand).unwrap();
        assert!(vel.0.x > 0.0,
            "brigand standing in the cat's ward should flee AWAY from the cat (+x), got vx={}", vel.0.x);
        let brigand_speed = mob::mob_def(MobType::Brigand).speed / 20.0;
        assert!((vel.0.x - brigand_speed * 0.5).abs() < 1e-4,
            "flee speed should be half the mob's normal speed: got {}, want {}", vel.0.x, brigand_speed * 0.5);
    }

    #[test]
    fn bear_ignores_the_cat_ward() {
        // The ward is an explicit species allowlist (Brigand/Marauder/
        // Berserker/Hyena) — Bear is Hostile-category too but must NOT
        // flee a tamed cat or skip a warded player. Cat at x=0, Bear at
        // x=5 (inside the cat's 12-radius ward — if Bear wrongly obeyed
        // the ward it would flee toward +x), player at x=3 (between them,
        // also inside the cat's ward — if Bear wrongly filtered warded
        // targets it would lose/ignore this player). Chasing the real
        // player means moving toward -x; fleeing the cat would mean +x —
        // these disagree, so the sign of vx tells them apart.
        let mut world = World::new();
        stone_floor(&mut world, 4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        tame_cat_at(&mut ecs, Vec3::new(0.0, 5.0, 0.0));
        let bear = entity::spawn_mob(&mut ecs, MobType::Bear, Vec3::new(5.0, 5.0, 0.0));
        {
            let mut ai = ecs.get::<&mut MobAi>(bear).unwrap();
            ai.state = AiState::Chase;
        }
        tick_mob_ai(&mut ecs, &world, &reg, &[Vec3::new(3.0, 5.0, 0.0)]);
        let ai = ecs.get::<&MobAi>(bear).unwrap();
        assert!(matches!(ai.state, AiState::Chase),
            "Bear must keep chasing through the ward, got {:?}", ai.state);
        let vel = ecs.get::<&Velocity>(bear).unwrap();
        assert!(vel.0.x < 0.0,
            "Bear should still close in on the (warded) player at -x, got vx={}", vel.0.x);
    }
}
