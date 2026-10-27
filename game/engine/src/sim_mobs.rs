//! D4 slice 3 — the mob simulation, one implementation for every kind of
//! world.
//!
//! The animals' and monsters' tick is a fixed sequence of phases, each a pure
//! function of the parts it is handed (an ECS, a world, the players' bodies),
//! so whichever side owns the world runs the very same code:
//!
//! 1. [`products`] — the Nostrich lay/feather timers and chicken eggs;
//! 2. [`population`] (every 20 ticks) — a village's first villagers, Satoshi's
//!    place, knights, wanderers;
//! 3. [`pre_pass`] — the brigand overrides (detect range, flee gate,
//!    day/night chase gate), then [`golems`] — the village defenders' swings;
//! 4. [`mob_ai`] — the generic AI, chasing the nearest target;
//! 5. [`breed`] — in-love pairs make babies, juveniles grow;
//! 6. [`species`] — tamed wolves follow, then every species' own AI
//!    (rabbits, bees, goats, horses, pets, Nostriches, parrot alarms, squids,
//!    hyenas, sharks, bears), overriding the generic AI's velocities;
//! 7. [`tethers`] — a Lead's pull wins over the species AI;
//! 8. [`steer_local_seats`] (with `server::steer_seats` for joiners) — each
//!    rider steers its mount last, then [`physics`] integrates;
//! 9. [`summon`] — Satoshi walks to whoever summoned him.
//!
//! **Who runs it.** A standalone game (single-player, the web build) and a
//! joined client call these phases from `GameState::tick`, at the places the
//! old inline code stood (the local player physics and the block machines sit
//! between some of them there). A lent world (a LAN/online host, a
//! `--solo-hosted` game) runs them back to back inside its server's tick
//! (`GameServer::tick`, inside the lend window), once per logical tick, and
//! the host client runs none (`SimSystem::lent_owner`: `MobAi`,
//! `EntityPhysics`, `Products`, `Population`, `Breeding`, `Species` and
//! `Tethers` are the server's). A dedicated server runs them in its own tick.
//!
//! **What reaches a player.** A phase never touches a `PlayerSlot`: what it
//! does to a player body comes out as a [`SimEvent`] keyed by player slot.
//! The standalone client applies each phase's events right after the phase,
//! where the old inline code applied them. The server lands a joiner's on the
//! joiner's server body at once (`GameServer::land_hit_on_joiner`) and queues
//! a local slot's — a lent world's host players — in its outbox
//! (`GameServer::sim_events`), which the host client drains straight after
//! the lend window (`GameState::tick_hosted_server`), before the ride follow,
//! the next catch-up tick, its next input or a save. Remote kill and breed
//! credit keep their wire path (`pending_bred_events`); the outbox carries
//! only this machine's players' effects, so no credit is counted twice.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use glam::Vec3;

use crate::block::BlockRegistry;
use crate::world::World;

/// The player bodies the species AI, pet follow and the Leads see, indexed by
/// player slot: a slot with nobody in it holds
/// [`crate::species_ai::ABSENT_PLAYER`]. With each slot's look yaw (a perched
/// parrot keeps to its owner's shoulder) and the pet-owner key (verified
/// npub) → slot of every joiner in the world that has one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobBodies {
    pub positions: Vec<Vec3>,
    pub yaws: Vec<f32>,
    pub remote_owners: Vec<(String, usize)>,
}

impl MobBodies {
    pub fn owners(&self) -> crate::tameable::OwnerBodies<'_> {
        crate::tameable::OwnerBodies { positions: &self.positions, remote_owners: &self.remote_owners }
    }
}

/// One local player's seat for [`steer_local_seats`]: its mount, what it asks
/// of it this tick, and the ride state it carries (which comes back steered).
/// `off`: the rider got off this tick (the mount is already unmounted; the
/// caller settles the body on its feet).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalSeat {
    pub slot: usize,
    pub mount: hecs::Entity,
    pub input: crate::riding::SeatInput,
    pub ride: crate::riding::RideState,
    pub off: Option<crate::riding::Dismount>,
}

/// What the mob simulation did to a player, or to the world in a way a player
/// sees: the typed outbox (module doc).
#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    /// A species attack (a bee sting, a goat's gore, a shark bite) on the body
    /// at `slot`: `damage` before armour, `knock` added to its velocity.
    Hit { slot: usize, damage: f32, cause: crate::survival::DamageCause, knock: Vec3 },
    /// The hostile mobs in melee reach of a local body after this tick's
    /// physics (`combat::hostiles_in_reach`): landed on its `PlayerSlot` by
    /// its own client (armour, i-frames, wear, the pets' revenge).
    HostileContacts { slot: usize, contacts: Vec<crate::combat::MeleeContact> },
    /// A baby was born (its credit inside).
    BabyBorn(crate::breeding::Born),
    /// A tamed parrot squawked an alarm (its cooldown is already set).
    ParrotAlarm { pet: hecs::Entity, pos: Vec3 },
    /// A cell the mobs changed (a bear ate a crop): `block` is what is there now.
    CellChanged { x: i32, y: i32, z: i32, block: crate::block::BlockId },
    /// A local seat, steered.
    Seat(LocalSeat),
}

/// The outbox: [`SimEvent`]s in the order the phases made them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimEvents(Vec<SimEvent>);

impl SimEvents {
    pub fn push(&mut self, event: SimEvent) {
        self.0.push(event);
    }

    /// Everything queued, oldest first, leaving the outbox empty.
    pub fn take(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.0)
    }
}

/// Phase 1 — the Nostrich's own lay and feather timers (Spec 28d.nostrich
/// v2: each bird's `NostrichData` counts down; wild and tamed lay, only
/// tamed shed) and chicken eggs (`animal_products`), each product dropped at
/// the animal's feet as an item. `tick` is the monotonic tick counter, which
/// also seeds the drops.
pub fn products(ecs: &mut hecs::World, tick: u64) {
    let mut nostrich_egg_drops: Vec<(Vec3, u32)> = Vec::new();
    let mut nostrich_feather_drops: Vec<(Vec3, u32)> = Vec::new();
    for (_e, (data, pos)) in ecs
        .query::<(&mut crate::nostrich::NostrichData, &crate::entity::Position)>()
        .iter()
    {
        // Decrement timers. Lay applies to both wild + tamed;
        // feather shed applies only to tamed.
        if data.lay_ticks_remaining > 0 {
            data.lay_ticks_remaining -= 1;
        }
        if data.lay_ticks_remaining == 0 {
            let seed = ((pos.0.x as i32 as u32).wrapping_mul(374761393))
                .wrapping_add((pos.0.z as i32 as u32).wrapping_mul(668265263))
                .wrapping_add(tick as u32);
            nostrich_egg_drops.push((pos.0, seed));
            data.lay_ticks_remaining = crate::nostrich::LAY_PERIOD_TICKS;
        }
        if data.is_tamed() {
            if data.feather_ticks_remaining > 0 {
                data.feather_ticks_remaining -= 1;
            }
            if data.feather_ticks_remaining == 0 {
                let seed = ((pos.0.z as i32 as u32).wrapping_mul(374761393))
                    .wrapping_add((pos.0.x as i32 as u32).wrapping_mul(668265263))
                    .wrapping_add(tick as u32);
                nostrich_feather_drops.push((pos.0, seed));
                data.feather_ticks_remaining = crate::nostrich::FEATHER_PERIOD_TICKS;
            }
        }
    }
    for (pos, seed) in nostrich_egg_drops {
        let stack = crate::item::ItemStack::new_material(crate::item::MaterialId::NostrichEgg, 1);
        crate::entity::spawn_item(ecs, pos, stack, seed);
    }
    for (pos, seed) in nostrich_feather_drops {
        let stack = crate::item::ItemStack::new_material(crate::item::MaterialId::NostrichFeather, 1);
        crate::entity::spawn_item(ecs, pos, stack, seed);
    }
    // Animals Wave 2 — chicken egg laying: each chicken's
    // AnimalProductState times the lay and drops an Egg at its feet.
    let mut chicken_egg_drops: Vec<(Vec3, u32)> = Vec::new();
    for (_e, (state, pos, kind)) in ecs
        .query::<(
            &mut crate::animal_products::AnimalProductState,
            &crate::entity::Position,
            &crate::entity::MobKind,
        )>()
        .iter()
    {
        if kind.0 == crate::mob::MobType::Chicken && crate::animal_products::should_lay_egg(*state, tick) {
            let seed = ((pos.0.x as i32 as u32).wrapping_mul(2_246_822_519))
                .wrapping_add((pos.0.z as i32 as u32).wrapping_mul(668_265_263))
                .wrapping_add(tick as u32);
            chicken_egg_drops.push((pos.0, seed));
            crate::animal_products::record_action(state, tick);
        }
    }
    for (pos, seed) in chicken_egg_drops {
        let stack = crate::item::ItemStack::new_material(crate::item::MaterialId::Egg, 1);
        crate::entity::spawn_item(ecs, pos, stack, seed);
    }
}

/// Is `tick` a [`population`] tick (once a second)?
pub fn population_due(tick: u64) -> bool {
    tick.is_multiple_of(20)
}

/// Phase 2 (call on [`population_due`] ticks) — Spec 19 phase 4: the first
/// villagers of every village whose blocks are loaded and that has none yet
/// (idempotent, `populated_villages`); Satoshi placed in his house near spawn
/// (idempotent); the Knight, the village defender (HP-4); Wandering
/// Villagers spawning and migrating toward bells (Phase 10). The brigand
/// hideouts' replenisher is its own system (`SimSystem::Hideouts`).
pub fn population(
    world: &mut World,
    biome_gen: &crate::biome::BiomeGenerator,
    ecs: &mut hecs::World,
    tick: u64,
) {
    crate::village_gen::spawn_initial_villagers(world, biome_gen, ecs);
    crate::satoshi::tick_spawn(world, ecs);
    crate::village_gen::tick_knight_spawn(world, biome_gen, ecs);
    crate::village_gen::tick_wanderer_migration(world, ecs, tick);
}

/// Phase 3 — HP-3 brigand pre-pass, before the generic AI so its
/// tier-specific detect range, flee gate and day/night chase gate land
/// before chase logic runs. `eff_world_time`: the effective (time-locked)
/// day clock.
pub fn pre_pass(ecs: &mut hecs::World, targets: &[Vec3], eff_world_time: u32) {
    crate::brigand::tick_brigand_overrides(ecs, targets, eff_world_time);
}

/// Phase 3 (after [`pre_pass`]) — the village defenders' swings
/// (`mob_ai::tick_golem_combat`, rate-limited per defender inside).
pub fn golems(ecs: &mut hecs::World) {
    crate::mob_ai::tick_golem_combat(ecs);
}

/// Phase 4 — the generic mob AI, chasing the nearest of `targets` (living
/// player bodies; a spawn-protected joiner is listed as
/// [`crate::species_ai::ABSENT_PLAYER`], `spawn_protection::targetable_pos`).
pub fn mob_ai(ecs: &mut hecs::World, world: &World, registry: &BlockRegistry, targets: &[Vec3]) {
    crate::mob_ai::tick_mob_ai(ecs, world, registry, targets);
}

/// Phase 5 — P5 animal breeding: pair in-love adults into babies and grow
/// juveniles (`breeding::step`); one [`SimEvent::BabyBorn`] per baby.
pub fn breed(ecs: &mut hecs::World, tick: u64, out: &mut SimEvents) {
    for baby in crate::breeding::step(ecs, tick) {
        out.push(SimEvent::BabyBorn(baby));
    }
}

/// Phase 6 — tamed wolves follow their owners (after the generic AI, so they
/// win over its wander, and before the Leads), then the living-world species
/// dispatch: every species' own AI, which owns locomotion during the generic
/// Idle/Wander states and yields to Flee/Chase/Investigate. Attacks on a
/// body come out as [`SimEvent::Hit`] (a bee that stung is gone already), a
/// parrot's alarm as [`SimEvent::ParrotAlarm`], a crop a bear ate as
/// [`SimEvent::CellChanged`]. `world_time`: the raw day clock (hyenas);
/// `difficulty` scales the neutral mobs' bites (W2).
#[allow(clippy::too_many_arguments)]
pub fn species(
    ecs: &mut hecs::World,
    world: &mut World,
    bodies: &MobBodies,
    tick: u64,
    world_time: u32,
    difficulty: crate::survival::Difficulty,
    out: &mut SimEvents,
) {
    use crate::survival::{scale_mob_damage, DamageCause};
    let positions = &bodies.positions;
    // P3 — tamed-wolf companion follow.
    crate::block_interact::tick_wolf_companions(ecs, &bodies.owners(), tick);
    crate::species_ai::dispatch_rabbits(ecs, positions, tick);
    for sting in crate::species_ai::dispatch_bees(ecs, positions, tick) {
        out.push(SimEvent::Hit {
            slot: sting.target_slot,
            damage: scale_mob_damage(crate::species_ai::BEE_STING_DAMAGE, difficulty),
            cause: DamageCause::Mob(crate::mob::MobType::Bee),
            knock: Vec3::ZERO,
        });
        // The bee dies after stinging (Minecraft parity).
        let _ = ecs.despawn(sting.bee);
    }
    for imp in crate::species_ai::dispatch_goats(ecs, positions, tick) {
        let Some(&p) = positions.get(imp.target_slot) else { continue };
        let push = Vec3::new(p.x - imp.goat_pos.x, 0.0, p.z - imp.goat_pos.z).normalize_or_zero();
        out.push(SimEvent::Hit {
            slot: imp.target_slot,
            damage: scale_mob_damage(crate::goat_ai::CHARGE_DAMAGE, difficulty),
            cause: DamageCause::Mob(crate::mob::MobType::Goat),
            knock: Vec3::new(
                push.x * crate::species_ai::GOAT_KNOCKBACK,
                crate::species_ai::GOAT_KNOCKBACK_UP,
                push.z * crate::species_ai::GOAT_KNOCKBACK,
            ),
        });
    }
    // Animals Wave 2 — horse herd-wander (skips ridden).
    crate::species_ai::dispatch_horses(ecs, tick);
    // Task 9 — the perched shoulder anchor needs the owner's yaw.
    crate::species_ai::dispatch_companions(ecs, positions, &bodies.yaws, tick, &bodies.remote_owners);
    // The tamed-Nostrich Follow/Sit machine.
    crate::species_ai::dispatch_nostriches(ecs, positions, tick, &bodies.remote_owners);
    // Pets wave Task 10 — a tamed parrot squawks when a hostile is near,
    // gated by its AlarmCooldown.
    for (pet, pos) in crate::companion::parrot_alarms(ecs, tick) {
        let _ = ecs.insert_one(
            pet,
            crate::companion::AlarmCooldown { until_tick: tick + crate::companion::PARROT_ALARM_COOLDOWN },
        );
        out.push(SimEvent::ParrotAlarm { pet, pos });
    }
    crate::species_ai::dispatch_squids(ecs, world, tick);
    crate::species_ai::dispatch_hyenas(ecs, positions, world_time);
    // Aquatic wave — the Shark hunts players in deep water.
    for bite in crate::species_ai::dispatch_sharks(ecs, world, positions, tick) {
        let Some(&p) = positions.get(bite.target_slot) else { continue };
        let push = Vec3::new(p.x - bite.shark_pos.x, 0.0, p.z - bite.shark_pos.z).normalize_or_zero();
        out.push(SimEvent::Hit {
            slot: bite.target_slot,
            damage: scale_mob_damage(crate::species_ai::SHARK_BITE_DAMAGE, difficulty),
            cause: DamageCause::Mob(crate::mob::MobType::Shark),
            knock: Vec3::new(push.x * 0.4, 0.0, push.z * 0.4),
        });
    }
    for eff in crate::species_ai::dispatch_bears(ecs, world, positions, tick) {
        match eff {
            crate::species_ai::BearWorldEffect::ConsumeCrop { x, y, z } => {
                // A raided crop reverts to tilled soil (the bear ate it).
                world.set_block(x, y, z, crate::block::TILLED_SOIL);
                out.push(SimEvent::CellChanged { x, y, z, block: crate::block::TILLED_SOIL });
            }
            crate::species_ai::BearWorldEffect::RaidChest { x, y, z } => {
                if let Some(c) = world.chest_at_mut((x, y, z)) {
                    c.take_one_food();
                }
            }
        }
    }
}

/// Phase 7 — Spec 36 Lead tethers, after the species AI so the pull wins on a
/// tethered mob (a slack one keeps its AI velocity). A snapped tether drops a
/// Lead item at the snap site and comes off the mob.
pub fn tethers(ecs: &mut hecs::World, bodies: &MobBodies) {
    for (mob_entity, drop_pos) in crate::tether::tick_tethers(ecs, &bodies.positions) {
        let _ = ecs.remove_one::<crate::tether::Tethered>(mob_entity);
        crate::entity::spawn_item(
            ecs,
            drop_pos + Vec3::new(0.0, 0.5, 0.0),
            crate::item::ItemStack::new_material(crate::item::MaterialId::Lead, 1),
            (drop_pos.x as i32 * 31 + drop_pos.z as i32 * 17) as u32,
        );
    }
}

/// Phase 8 — MP-D2c-1: each local seat steers its mount one tick
/// (`riding::steer`, the rule every seat runs), right before [`physics`] and
/// after every phase that writes a mob's velocity, so the rider has the last
/// word. A dismount gesture unmounts here, before physics moves the mount;
/// each seat comes back as a [`SimEvent::Seat`] (its ride state, and `off`).
pub fn steer_local_seats(ecs: &mut hecs::World, world: &World, seats: Vec<LocalSeat>, out: &mut SimEvents) {
    for mut seat in seats {
        if let Some(why) = crate::riding::steer(ecs, world, seat.mount, &seat.input, &mut seat.ride) {
            crate::riding::unmount(ecs, seat.mount, &mut seat.ride);
            seat.off = Some(why);
        }
        out.push(SimEvent::Seat(seat));
    }
}

/// Phase 8 — entity physics (every tick).
pub fn physics(ecs: &mut hecs::World, world: &World, registry: &BlockRegistry) {
    crate::entity::tick_entities(ecs, world, registry);
}

/// Phase 9 — Satoshi co-presence: he walks toward whoever summoned him (the
/// world's first local player, `anchor`), or back to his hut, after the AI
/// and physics so this is the last word on his position. No-op in worlds
/// without Satoshi, and with no anchor (a dedicated server has no local
/// player to summon him).
pub fn summon(world: &World, ecs: &mut hecs::World, anchor: Option<Vec3>) {
    if let Some(anchor) = anchor {
        crate::satoshi::tick_summon(world, ecs, anchor);
    }
}

#[cfg(test)]
mod tests;
