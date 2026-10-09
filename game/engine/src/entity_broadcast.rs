//! Server → client entity broadcast (Spec 04 §4.2c "Entity mirror and joiner damage", MP-D2a).
//!
//! Every tick `HostedServer::broadcast_state` asks [`EntityBroadcast::diff`]
//! for the broadcastable entities in the server's ECS — mobs, rail carts,
//! dropped items, projectiles in flight — and turns them into each client's
//! spawn / update / despawn lists through that client's [`ClientInterest`]:
//!
//! - **Ids.** An entity gets a `ProtocolId` the first time the diff sees it;
//!   it keeps it for life (a lent host's ids live on its own ECS).
//! - **Changed-only.** An `EntityUpdate` goes out only when it differs from
//!   the last one broadcast for that entity — position, velocity or yaw past
//!   a small epsilon, or a new `state`/`flags`. The comparison is against the
//!   last update SENT, not last tick's, so a slow drift still crosses the
//!   epsilon. An idle herd costs no bandwidth; the client keeps the last
//!   update it got. A client whose budget held an update back still gets
//!   the latest one later: `state_outbox` keeps the newest unsent update per
//!   id until it goes.
//! - **Interest.** A joiner (a server-simulated client) hears only about
//!   entities within [`INTEREST_ENTER_RADIUS`] blocks (horizontal) of its
//!   body; one already shown to it is withdrawn only past
//!   [`INTEREST_LEAVE_RADIUS`], so a mob on the boundary doesn't flap.
//!   Entering = an `EntitySpawn` built from the entity's CURRENT state plus
//!   its current update; leaving = an `EntityDespawn`. A local in-process
//!   slot (the host's own loopback) hears about everything.
//! - **Late joiners need no backfill.** A fresh interest set is empty, so
//!   everything in range enters on the client's first broadcast — with its
//!   full payload (an item's stack, a tool's durability).
//!
//! Mob count × joiners drives the bandwidth; the per-client byte budget in
//! `state_outbox` caps it.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::Vec3;

use crate::protocol::{entity_flags, EntityKind, EntitySpawn, EntityUpdate, WireItem};

/// A joiner is told about an entity once it comes within this many blocks
/// (horizontal distance) of its body. Past mob-spawning range and well inside
/// the default render distance's fog, so a mob never pops in at arm's length.
pub const INTEREST_ENTER_RADIUS: f32 = 80.0;

/// An entity already shown to a joiner is withdrawn only past this distance:
/// the gap to [`INTEREST_ENTER_RADIUS`] is the hysteresis that stops a mob
/// pacing on the boundary from spawning and despawning every tick.
pub const INTEREST_LEAVE_RADIUS: f32 = 96.0;

/// Position change (blocks, per axis) below which an update is "unchanged".
const POS_EPSILON: f32 = 1e-3;
/// Velocity change (blocks/tick, per axis) below which an update is "unchanged".
const VEL_EPSILON: f32 = 1e-3;
/// Yaw change (radians) below which an update is "unchanged".
const YAW_EPSILON: f32 = 1e-3;

/// What a broadcastable entity is, as far as its spawn needs to know.
#[derive(Clone, Copy, Debug)]
enum Class {
    Mob(crate::mob::MobType),
    Cart,
    Item,
    Projectile,
}

/// One broadcastable entity as it stands this tick: its current update, and
/// where to build the spawn a client is sent when the entity enters its
/// interest. The spawn is built only then ([`Self::spawn`], review D2a
/// LOW-5): an entrant is rare, and a spawn costs a stack encode per dropped
/// item, so a tick's work stays proportional to the population's updates
/// plus the entrants, not a full spawn per entity per tick.
pub(crate) struct LiveEntity {
    entity: hecs::Entity,
    class: Class,
    pub update: EntityUpdate,
}

impl LiveEntity {
    fn pos(&self) -> Vec3 {
        Vec3::new(self.update.x, self.update.y, self.update.z)
    }

    /// The `EntitySpawn` for this entity's current state: position and yaw
    /// from this tick's update (a spawn and its update agree), the rest from
    /// the ECS (`ecs` is the world `EntityBroadcast::diff` read this tick).
    pub fn spawn(&self, ecs: &hecs::World) -> EntitySpawn {
        let u = &self.update;
        let mut spawn = EntitySpawn {
            id: u.id,
            kind: EntityKind::Cart,
            x: u.x,
            y: u.y,
            z: u.z,
            yaw: u.yaw,
            health: 0,
            item_kind: 0,
            item_id: 0,
            item_count: 0,
            full_item: WireItem::None,
        };
        match self.class {
            Class::Mob(kind) => {
                spawn.kind = wire_kind_for(kind);
                spawn.health = crate::mob::mob_def(kind).health;
            }
            Class::Cart => {}
            Class::Projectile => spawn.kind = EntityKind::Projectile,
            Class::Item => {
                spawn.kind = EntityKind::Item;
                if let Ok(item) = ecs.get::<&crate::entity::ItemEntity>(self.entity) {
                    let (item_kind, item_id) =
                        crate::inventory::item_to_ref(&item.stack.item).to_wire();
                    spawn.item_kind = item_kind;
                    spawn.item_id = item_id;
                    spawn.item_count = item.stack.count;
                    spawn.full_item = crate::inventory::item_to_wire_shared(&item.stack.item);
                }
            }
        }
        spawn
    }
}

/// One tick of the server's entity population, diffed against the last.
#[derive(Default)]
pub(crate) struct EntityTick {
    /// Every broadcastable entity alive this tick, by `ProtocolId` (ordered,
    /// so every client's lists come out in the same order).
    pub live: BTreeMap<u32, LiveEntity>,
    /// Ids broadcast before and gone now, ascending.
    pub despawns: Vec<u32>,
    /// Ids whose update differs from the last one broadcast (new ids too).
    pub changed: HashSet<u32>,
}

/// The server-wide half of the broadcast: id assignment and the changed-only
/// baseline. One per `HostedServer`.
pub(crate) struct EntityBroadcast {
    next_id: u32,
    /// The last update broadcast for every entity the previous diff saw —
    /// also the "known" set a despawn is detected against.
    last_sent: HashMap<u32, EntityUpdate>,
}

impl Default for EntityBroadcast {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityBroadcast {
    pub fn new() -> Self {
        // Id 0 is never handed out.
        Self { next_id: 1, last_sent: HashMap::new() }
    }

    /// The id the next newly-seen entity will get.
    #[cfg(test)]
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Diff the server ECS against the previous call: assign a `ProtocolId`
    /// to every broadcastable entity that lacks one, build each one's spawn
    /// and current update, and mark which changed and which are gone. `tick`
    /// is the server's clock (`GameServer::tick_counter`), which judges a
    /// cow's milk and a sheep's wool (`entity_flags::PRODUCT_NOT_READY`).
    pub fn diff(&mut self, ecs: &mut hecs::World, tick: u64) -> EntityTick {
        self.assign_ids(ecs);
        let live = collect_live(ecs, tick);
        let mut changed = HashSet::new();
        for (id, e) in &live {
            let fresh = match self.last_sent.get(id) {
                None => true,
                Some(prev) => update_changed(prev, &e.update),
            };
            if fresh {
                self.last_sent.insert(*id, e.update.clone());
                changed.insert(*id);
            }
        }
        let mut despawns: Vec<u32> =
            self.last_sent.keys().filter(|id| !live.contains_key(id)).copied().collect();
        despawns.sort_unstable();
        for id in &despawns {
            self.last_sent.remove(id);
        }
        EntityTick { live, despawns, changed }
    }

    /// Hand a fresh `ProtocolId` to every mob, cart, dropped item and
    /// projectile that has none yet. Candidates are collected first so the
    /// query borrow is released before `insert_one`.
    fn assign_ids(&mut self, ecs: &mut hecs::World) {
        use crate::cart::CartData;
        use crate::entity::{ItemEntity, MobKind, Position, ProjectileEntity, ProtocolId};

        let mut missing: Vec<hecs::Entity> = Vec::new();
        missing.extend(
            ecs.query::<hecs::Without<(&MobKind, &Position), &ProtocolId>>().iter().map(|(e, _)| e),
        );
        missing.extend(
            ecs.query::<hecs::Without<(&CartData, &Position), &ProtocolId>>()
                .iter()
                .map(|(e, _)| e),
        );
        missing.extend(
            ecs.query::<hecs::Without<(&ItemEntity, &Position), &ProtocolId>>()
                .iter()
                .map(|(e, _)| e),
        );
        missing.extend(
            ecs.query::<hecs::Without<(&ProjectileEntity, &Position), &ProtocolId>>()
                .iter()
                .map(|(e, _)| e),
        );
        for entity in missing {
            let id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            let _ = ecs.insert_one(entity, ProtocolId(id));
        }
    }
}

/// Is `next` different enough from `prev` (the last update broadcast) to send?
fn update_changed(prev: &EntityUpdate, next: &EntityUpdate) -> bool {
    let far = |a: f32, b: f32, eps: f32| (a - b).abs() > eps;
    prev.state != next.state
        || prev.flags != next.flags
        || far(prev.x, next.x, POS_EPSILON)
        || far(prev.y, next.y, POS_EPSILON)
        || far(prev.z, next.z, POS_EPSILON)
        || far(prev.yaw, next.yaw, YAW_EPSILON)
        || far(prev.vx, next.vx, VEL_EPSILON)
        || far(prev.vy, next.vy, VEL_EPSILON)
        || far(prev.vz, next.vz, VEL_EPSILON)
}

/// Every broadcastable entity's current update (and how to build its
/// spawn), by `ProtocolId`, at server tick `tick`.
fn collect_live(ecs: &hecs::World, tick: u64) -> BTreeMap<u32, LiveEntity> {
    use crate::cart::CartData;
    use crate::entity::{ItemEntity, MobKind, Position, ProjectileEntity, ProtocolId, Velocity};
    use crate::mob_ai::{AiState, MobAi};

    let mut live: BTreeMap<u32, LiveEntity> = BTreeMap::new();

    for (e, (pid, kind, pos, vel, ai, health, baby, satoshi)) in ecs
        .query::<(
            &ProtocolId,
            &MobKind,
            &Position,
            &Velocity,
            &MobAi,
            Option<&crate::combat::Health>,
            Option<&crate::breeding::Baby>,
            Option<&crate::satoshi::SatoshiMarker>,
        )>()
        .iter()
    {
        // Yaw: the mob's persistent facing for idle/wander, velocity-derived
        // during chase so the model turns toward the target naturally.
        let yaw = match &ai.state {
            AiState::Chase if vel.0.x.abs() + vel.0.z.abs() > 1e-4 => (-vel.0.x).atan2(-vel.0.z),
            _ => ai.facing,
        };
        let state: u8 = match ai.state {
            AiState::Idle { .. } => 0,
            AiState::Wander { .. } => 1,
            AiState::Chase => 2,
            // Wave 28 — a mob investigating a campfire holds position near
            // it, which reads as idle; the network doesn't model the state.
            AiState::InvestigateCampfire { .. } => 0,
            // Spec 19 phase 8 — a guarding Knight renders as walking.
            AiState::GolemGuard { .. } => 1,
            // P2 — a fleeing animal is moving: walking on the wire.
            AiState::Flee { .. } => 1,
        };
        let mut flags = 0u8;
        if health.is_some_and(crate::combat::Health::is_flashing) {
            flags |= entity_flags::HURT;
        }
        if baby.is_some() {
            flags |= entity_flags::BABY;
        }
        if crate::tameable::pet_owner_of(ecs, e).is_some() {
            flags |= entity_flags::TAMED;
        }
        if satoshi.is_some() {
            flags |= entity_flags::SATOSHI;
        }
        if ecs.get::<&crate::tether::Tethered>(e).is_ok() {
            flags |= entity_flags::TETHERED;
        }
        // FU3 (FU1 verify N8) — by the rule the server milks and shears by.
        if matches!(kind.0, crate::mob::MobType::Cow | crate::mob::MobType::Sheep)
            && !crate::mob_interact::product_ready(ecs, e, tick)
        {
            flags |= entity_flags::PRODUCT_NOT_READY;
        }
        let update = EntityUpdate {
            id: pid.0,
            x: pos.0.x,
            y: pos.0.y,
            z: pos.0.z,
            yaw,
            state,
            vx: vel.0.x,
            vy: vel.0.y,
            vz: vel.0.z,
            flags,
        };
        live.insert(pid.0, LiveEntity { entity: e, class: Class::Mob(kind.0), update });
    }

    // Rail freight Phase 1 — carts ride the same broadcast and the same
    // alive-set, so a persisting cart never reads as despawned. Position is
    // the lerped render anchor; yaw is `CartData.facing`. Carts are not
    // combat entities: neutral health/state, no velocity (the cart sim
    // steps cell to cell, it has no `Velocity`).
    for (e, (pid, cart, pos)) in ecs.query::<(&ProtocolId, &CartData, &Position)>().iter() {
        let update = EntityUpdate {
            id: pid.0,
            x: pos.0.x,
            y: pos.0.y,
            z: pos.0.z,
            yaw: cart.facing,
            ..Default::default()
        };
        live.insert(pid.0, LiveEntity { entity: e, class: Class::Cart, update });
    }

    // Death-drops phase 2 — dropped items, so a joiner can SEE server-side
    // loot. The stack rides the spawn's item_* fields (+ the full-fidelity
    // payload for tools and armour, phase 3); updates carry the
    // settle/magnet motion.
    for (e, (pid, _item, pos, vel)) in ecs
        .query::<(&ProtocolId, &ItemEntity, &Position, Option<&Velocity>)>()
        .iter()
    {
        let v = vel.map_or(Vec3::ZERO, |v| v.0);
        let update = EntityUpdate {
            id: pid.0,
            x: pos.0.x,
            y: pos.0.y,
            z: pos.0.z,
            vx: v.x,
            vy: v.y,
            vz: v.z,
            ..Default::default()
        };
        live.insert(pid.0, LiveEntity { entity: e, class: Class::Item, update });
    }

    // MP-A3 — projectiles in flight (a dedicated server's dispenser arrows):
    // spawn on first sight, an update while they fly, one despawn when a hit
    // or lifetime expiry removes them.
    for (e, (pid, proj, pos, vel)) in ecs
        .query::<(&ProtocolId, &ProjectileEntity, &Position, &Velocity)>()
        .iter()
    {
        let update = EntityUpdate {
            id: pid.0,
            x: pos.0.x,
            y: pos.0.y,
            z: pos.0.z,
            yaw: projectile_yaw(vel.0),
            state: u8::from(proj.is_blunt),
            vx: vel.0.x,
            vy: vel.0.y,
            vz: vel.0.z,
            flags: 0,
        };
        live.insert(pid.0, LiveEntity { entity: e, class: Class::Projectile, update });
    }

    live
}

/// The `MobType` → wire `EntityKind` mapping. Purely mechanical; exhaustive
/// so a new species is a compile error until it gets a wire discriminant.
/// The joiner's inverse is `remote_mobs::mob_type_for`.
pub(crate) fn wire_kind_for(kind: crate::mob::MobType) -> EntityKind {
    use crate::mob::MobType;
    match kind {
        MobType::Cow => EntityKind::Cow,
        MobType::Chicken => EntityKind::Chicken,
        MobType::Pig => EntityKind::Pig,
        MobType::Sheep => EntityKind::Sheep,
        MobType::Villager => EntityKind::Villager,
        MobType::Peddler => EntityKind::WanderingVillager,
        MobType::Wolf => EntityKind::Wolf,
        MobType::Horse => EntityKind::Horse,
        MobType::Rabbit => EntityKind::Rabbit,
        MobType::Goat => EntityKind::Goat,
        MobType::Bee => EntityKind::Bee,
        MobType::Squid => EntityKind::Squid,
        MobType::Nostrich => EntityKind::Nostrich,
        MobType::Bear => EntityKind::Bear,
        MobType::Hyena => EntityKind::Hyena,
        MobType::Brigand => EntityKind::Brigand,
        MobType::Marauder => EntityKind::Marauder,
        MobType::Berserker => EntityKind::Berserker,
        MobType::Knight => EntityKind::Knight,
        MobType::Fish => EntityKind::Fish,
        MobType::Shark => EntityKind::Shark,
        MobType::GlowSquid => EntityKind::GlowSquid,
        MobType::Fox => EntityKind::Fox,
        MobType::PolarBear => EntityKind::PolarBear,
        MobType::Reindeer => EntityKind::Reindeer,
        MobType::Cat => EntityKind::Cat,
        MobType::Parrot => EntityKind::Parrot,
        MobType::Donkey => EntityKind::Donkey,
        MobType::Mule => EntityKind::Mule,
        MobType::Crab => EntityKind::Crab,
    }
}

/// MP-A3 — a projectile's flight heading as a wire yaw: the same
/// `(-vx).atan2(-vz)` the arrow renderer derives from its velocity, so a
/// joiner can point the arrow before its first update arrives.
fn projectile_yaw(vel: Vec3) -> f32 {
    (-vel.x).atan2(-vel.z)
}


/// One client's share of a tick's entity events.
#[derive(Default, Debug)]
pub(crate) struct ClientEntityEvents {
    pub spawns: Vec<EntitySpawn>,
    pub updates: Vec<EntityUpdate>,
    pub despawns: Vec<u32>,
}

/// The entities one client has been told about (spawned and not since
/// despawned). Reset with the client's slot.
#[derive(Default)]
pub(crate) struct ClientInterest {
    shown: HashSet<u32>,
}

impl ClientInterest {
    /// This client's events for `tick`. `ecs` is the world the tick was
    /// diffed from (an entrant's spawn is built from it). `anchor` is where
    /// its body stands (a joiner); `None` for a local slot, which hears about
    /// everything.
    pub fn events(
        &mut self,
        tick: &EntityTick,
        ecs: &hecs::World,
        anchor: Option<Vec3>,
    ) -> ClientEntityEvents {
        let mut ev = ClientEntityEvents::default();
        for id in &tick.despawns {
            if self.shown.remove(id) {
                ev.despawns.push(*id);
            }
        }
        let distance = |e: &LiveEntity| anchor.map(|a| horizontal_distance(a, e.pos()));
        for (id, e) in &tick.live {
            if self.shown.contains(id) {
                if distance(e).is_some_and(|d| d > INTEREST_LEAVE_RADIUS) {
                    self.shown.remove(id);
                    ev.despawns.push(*id);
                } else if tick.changed.contains(id) {
                    ev.updates.push(e.update.clone());
                }
            } else if distance(e).is_none_or(|d| d <= INTEREST_ENTER_RADIUS) {
                // Entering: the spawn carries the entity's state NOW (not
                // where it first appeared), and its full current update
                // follows whether or not it changed this tick.
                self.shown.insert(*id);
                ev.spawns.push(e.spawn(ecs));
                ev.updates.push(e.update.clone());
            }
        }
        ev
    }

    /// Is entity `id` currently shown to this client?
    #[cfg(test)]
    pub fn shows(&self, id: u32) -> bool {
        self.shown.contains(&id)
    }
}

fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{self, ProtocolId};
    use crate::mob::MobType;

    fn fresh() -> (hecs::World, EntityBroadcast) {
        (hecs::World::new(), EntityBroadcast::new())
    }

    /// The unfiltered (local-slot) view of one tick: entities first shown,
    /// the updates sent, the despawns. `all` persists across ticks.
    fn global(
        ecs: &mut hecs::World,
        b: &mut EntityBroadcast,
        all: &mut ClientInterest,
    ) -> (Vec<EntitySpawn>, Vec<EntityUpdate>, Vec<u32>) {
        let tick = b.diff(ecs, 0);
        let ev = all.events(&tick, ecs, None);
        (ev.spawns, ev.updates, ev.despawns)
    }

    fn only_entity(ecs: &hecs::World) -> hecs::Entity {
        ecs.iter().next().unwrap().entity()
    }

    #[test]
    fn first_broadcast_emits_spawn_and_update_for_new_mob() {
        let (mut ecs, mut b) = fresh();
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(3.0, 4.0, 5.0));
        let tick = b.diff(&mut ecs, 0);
        let mut client = ClientInterest::default();
        let ev = client.events(&tick, &ecs, None);

        assert_eq!(ev.spawns.len(), 1, "new mob must produce one EntitySpawn");
        assert_eq!(ev.updates.len(), 1, "…and an EntityUpdate so yaw/state/flags are set");
        assert!(ev.despawns.is_empty());
        let s = &ev.spawns[0];
        assert_eq!(s.kind, EntityKind::Cow);
        assert_eq!((s.x, s.y, s.z), (3.0, 4.0, 5.0));
        assert_eq!(s.id, 1, "first id handed out is 1");
        assert_eq!(b.next_id(), 2, "next id advances");
        assert!(tick.changed.contains(&1));
    }

    #[test]
    fn an_unchanged_mob_sends_no_update_and_a_moved_one_does() {
        let (mut ecs, mut b) = fresh();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 64.0, 0.0));
        let mut client = ClientInterest::default();
        let _ = client.events(&b.diff(&mut ecs, 0), &ecs, None);

        // Nothing moved: changed-only sends nothing.
        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, None);
        assert!(ev.spawns.is_empty(), "already-shown mob must not re-spawn");
        assert!(ev.updates.is_empty(), "an unchanged mob sends no update");
        assert!(ev.despawns.is_empty());

        // A sub-epsilon nudge is still "unchanged"…
        let e = only_entity(&ecs);
        ecs.get::<&mut entity::Position>(e).unwrap().0.x += 1e-4;
        assert!(client.events(&b.diff(&mut ecs, 0), &ecs, None).updates.is_empty());
        // …but drift accumulates against the last SENT update, not last tick's.
        for _ in 0..12 {
            ecs.get::<&mut entity::Position>(e).unwrap().0.x += 1e-4;
            let _ = client.events(&b.diff(&mut ecs, 0), &ecs, None);
        }
        // A real step sends exactly one update carrying the new state.
        ecs.get::<&mut entity::Position>(e).unwrap().0.x = 0.5;
        ecs.get::<&mut entity::Velocity>(e).unwrap().0 = Vec3::new(0.1, 0.0, 0.0);
        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, None);
        assert_eq!(ev.updates.len(), 1);
        assert_eq!(ev.updates[0].x, 0.5);
        assert_eq!(ev.updates[0].vx, 0.1, "velocity rides the update");
    }

    #[test]
    fn drift_below_epsilon_per_tick_still_gets_sent_once_it_adds_up() {
        let (mut ecs, mut b) = fresh();
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let e = only_entity(&ecs);
        let mut client = ClientInterest::default();
        let _ = client.events(&b.diff(&mut ecs, 0), &ecs, None);
        let mut sent = 0;
        for _ in 0..30 {
            ecs.get::<&mut entity::Position>(e).unwrap().0.x += 4e-4;
            sent += client.events(&b.diff(&mut ecs, 0), &ecs, None).updates.len();
        }
        assert!(sent >= 3, "a slow drift crosses the epsilon repeatedly (sent {sent})");
    }

    #[test]
    fn flags_carry_hurt_baby_tamed_satoshi_and_tethered() {
        let (mut ecs, mut b) = fresh();
        let calf = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        ecs.insert_one(calf, crate::breeding::Baby { adult_at_tick: 99 }).unwrap();
        let hurt = entity::spawn_mob(&mut ecs, MobType::Pig, Vec3::new(2.0, 64.0, 0.0));
        ecs.get::<&mut crate::combat::Health>(hurt).unwrap().take_damage(1.0);
        let pup = entity::spawn_mob(&mut ecs, MobType::Wolf, Vec3::new(4.0, 64.0, 0.0));
        ecs.get::<&mut crate::wolf::WolfData>(pup).unwrap().ownership.owner_pubkey =
            "npub-owner".to_string();
        let guide = entity::spawn_mob(&mut ecs, MobType::Villager, Vec3::new(6.0, 64.0, 0.0));
        ecs.insert_one(guide, crate::satoshi::SatoshiMarker).unwrap();
        let leashed = entity::spawn_mob(&mut ecs, MobType::Sheep, Vec3::new(8.0, 64.0, 0.0));
        ecs.insert_one(
            leashed,
            crate::tether::Tethered { target: crate::tether::TetherTarget::Player(0) },
        )
        .unwrap();

        let tick = b.diff(&mut ecs, 0);
        let flags = |e: hecs::Entity| {
            let id = ecs.get::<&ProtocolId>(e).unwrap().0;
            tick.live[&id].update.flags
        };
        assert_eq!(flags(calf), entity_flags::BABY);
        assert_eq!(flags(hurt), entity_flags::HURT);
        assert_eq!(flags(pup), entity_flags::TAMED);
        assert_eq!(flags(guide), entity_flags::SATOSHI);
        assert_eq!(flags(leashed), entity_flags::TETHERED);

        // The flash ending is a change: one update clears the HURT bit.
        for _ in 0..crate::combat::DAMAGE_FLASH_TICKS {
            ecs.get::<&mut crate::combat::Health>(hurt).unwrap().tick();
        }
        let tick = b.diff(&mut ecs, 0);
        let id = ecs.get::<&ProtocolId>(hurt).unwrap().0;
        assert!(tick.changed.contains(&id));
        assert_eq!(tick.live[&id].update.flags, 0);
    }

    /// FU3 (FU1 verify N8) — `PRODUCT_NOT_READY` on a cow just milked and a
    /// sheep just shorn, by the server's own rule and clock; clear on a fresh
    /// animal, on one whose product is back, and on any other species.
    #[test]
    fn product_not_ready_marks_a_milked_cow_and_a_shorn_sheep() {
        use crate::item::{Item, MaterialId};
        let (mut ecs, mut b) = fresh();
        let cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let sheep = entity::spawn_mob(&mut ecs, MobType::Sheep, Vec3::new(2.0, 64.0, 0.0));
        let fresh_cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(4.0, 64.0, 0.0));
        let pig = entity::spawn_mob(&mut ecs, MobType::Pig, Vec3::new(6.0, 64.0, 0.0));
        let at: u64 = 1_000;
        let bucket = Item::Material(MaterialId::Bucket);
        let shears = Item::Tool(crate::crafting::Tool::new(
            crate::crafting::ToolType::Shears,
            crate::crafting::ToolMaterial::Iron,
        ));
        assert!(crate::mob_interact::milk(&mut ecs, cow, MobType::Cow, Some(&bucket), at).is_some_and(|r| r.done));
        assert!(crate::mob_interact::shear(&mut ecs, sheep, MobType::Sheep, Some(&shears), at).is_some_and(|r| r.done));

        fn flags(ecs: &hecs::World, tick: &EntityTick, e: hecs::Entity) -> u8 {
            let id = ecs.get::<&ProtocolId>(e).unwrap().0;
            tick.live[&id].update.flags
        }
        let tick = b.diff(&mut ecs, at + 1);
        assert_eq!(flags(&ecs, &tick, cow), entity_flags::PRODUCT_NOT_READY, "just milked");
        assert_eq!(flags(&ecs, &tick, sheep), entity_flags::PRODUCT_NOT_READY, "just shorn");
        assert_eq!(flags(&ecs, &tick, fresh_cow), 0, "never milked: ready");
        assert_eq!(flags(&ecs, &tick, pig), 0, "no product");

        let later = at + crate::animal_products::COW_MILK_COOLDOWN_TICKS;
        let tick = b.diff(&mut ecs, later);
        let id = ecs.get::<&ProtocolId>(cow).unwrap().0;
        assert!(tick.changed.contains(&id), "ready again is a change, sent");
        assert_eq!(flags(&ecs, &tick, cow), 0);
    }

    #[test]
    fn despawned_mob_despawns_once() {
        let (mut ecs, mut b) = fresh();
        let e = entity::spawn_mob(&mut ecs, MobType::Chicken, Vec3::new(1.0, 2.0, 3.0));
        let mut client = ClientInterest::default();
        let id = client.events(&b.diff(&mut ecs, 0), &ecs, None).spawns[0].id;

        ecs.despawn(e).unwrap();
        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, None);
        assert!(ev.spawns.is_empty() && ev.updates.is_empty());
        assert_eq!(ev.despawns, vec![id]);
        assert!(client.events(&b.diff(&mut ecs, 0), &ecs, None).despawns.is_empty(), "…only once");
    }

    #[test]
    fn a_joiner_hears_only_about_entities_in_range_with_hysteresis() {
        let (mut ecs, mut b) = fresh();
        let near = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(10.0, 64.0, 0.0));
        let far = entity::spawn_mob(&mut ecs, MobType::Pig, Vec3::new(200.0, 64.0, 0.0));
        let id = |ecs: &hecs::World, e| ecs.get::<&ProtocolId>(e).map(|p| p.0).ok();
        let joiner = Some(Vec3::new(0.0, 64.0, 0.0));
        let mut client = ClientInterest::default();

        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, joiner);
        assert_eq!(ev.spawns.len(), 1, "only the in-range mob is sent");
        assert_eq!(Some(ev.spawns[0].id), id(&ecs, near));
        assert!(!client.shows(id(&ecs, far).unwrap()), "an out-of-range mob is never sent");

        // Wandering out past ENTER but inside LEAVE: still shown (hysteresis).
        ecs.get::<&mut entity::Position>(near).unwrap().0.x = INTEREST_ENTER_RADIUS + 8.0;
        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, joiner);
        assert!(ev.despawns.is_empty(), "inside the leave radius it stays");
        assert_eq!(ev.updates.len(), 1, "and keeps getting its updates");

        // Past LEAVE: withdrawn with one despawn.
        ecs.get::<&mut entity::Position>(near).unwrap().0.x = INTEREST_LEAVE_RADIUS + 1.0;
        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, joiner);
        assert_eq!(ev.despawns, vec![id(&ecs, near).unwrap()]);
        // Back inside LEAVE but outside ENTER: not re-sent until it enters.
        ecs.get::<&mut entity::Position>(near).unwrap().0.x = INTEREST_ENTER_RADIUS + 8.0;
        assert!(client.events(&b.diff(&mut ecs, 0), &ecs, joiner).spawns.is_empty());

        // The far pig wanders into range: it ENTERS with its current state.
        ecs.get::<&mut entity::Position>(far).unwrap().0 = Vec3::new(5.0, 70.0, 5.0);
        let ev = client.events(&b.diff(&mut ecs, 0), &ecs, joiner);
        assert_eq!(ev.spawns.len(), 1);
        assert_eq!((ev.spawns[0].x, ev.spawns[0].y), (5.0, 70.0), "spawn = where it is now");
        assert_eq!(ev.updates.len(), 1, "an entering entity's update always follows");
    }

    #[test]
    fn a_late_joiner_gets_everything_in_range_once_with_full_payload() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let (mut ecs, mut b) = fresh();
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(3.0, 64.0, 5.0));
        entity::spawn_item(
            &mut ecs,
            Vec3::new(1.0, 65.0, 2.0),
            crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 3),
            0,
        );
        let mut axe = Tool::new(ToolType::Axe, ToolMaterial::Diamond);
        axe.durability = 91;
        entity::spawn_item(&mut ecs, Vec3::new(2.0, 65.0, 2.0), crate::item::ItemStack::new_tool(axe), 0);
        // The host has been broadcasting for a while before the joiner arrives.
        let mut host = ClientInterest::default();
        for _ in 0..3 {
            let _ = host.events(&b.diff(&mut ecs, 0), &ecs, None);
        }

        let mut late = ClientInterest::default();
        let ev = late.events(&b.diff(&mut ecs, 0), &ecs, Some(Vec3::new(0.0, 64.0, 0.0)));
        assert_eq!(ev.spawns.len(), 3, "every pre-existing entity in range, once");
        let bone = ev
            .spawns
            .iter()
            .find(|s| s.kind == EntityKind::Item && s.item_count == 3)
            .expect("the bone stack");
        let (bk, bid) = crate::inventory::item_to_ref(&crate::item::Item::Material(
            crate::item::MaterialId::Bone,
        ))
        .to_wire();
        assert_eq!((bone.item_kind, bone.item_id), (bk, bid), "the stack rides the spawn");
        assert!(
            ev.spawns.iter().any(|s| crate::inventory::item_from_wire_full(&s.full_item)
                == Some(crate::item::Item::Tool(axe))),
            "a tool arrives at its true durability"
        );
        assert!(late.events(&b.diff(&mut ecs, 0), &ecs, Some(Vec3::ZERO)).spawns.is_empty());
    }

    #[test]
    fn dropped_item_broadcasts_spawn_with_stack_payload_then_despawns_once() {
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        entity::spawn_item(
            &mut ecs,
            Vec3::new(1.0, 65.0, 2.0),
            crate::item::ItemStack::new_material(crate::item::MaterialId::RawBeef, 3),
            7,
        );
        let (spawns, _, despawns) = global(&mut ecs, &mut b, &mut all);
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].kind, EntityKind::Item);
        let (k, id) = crate::inventory::item_to_ref(&crate::item::Item::Material(
            crate::item::MaterialId::RawBeef,
        ))
        .to_wire();
        assert_eq!((spawns[0].item_kind, spawns[0].item_id, spawns[0].item_count), (k, id, 3));
        assert!(despawns.is_empty());

        let ids: Vec<hecs::Entity> =
            ecs.query::<&entity::ItemEntity>().iter().map(|(e, _)| e).collect();
        for e in ids {
            let _ = ecs.despawn(e);
        }
        assert_eq!(global(&mut ecs, &mut b, &mut all).2.len(), 1, "a picked-up item despawns once");
        assert!(global(&mut ecs, &mut b, &mut all).2.is_empty());
    }

    #[test]
    fn dropped_tool_and_armour_broadcast_full_fidelity() {
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        pick.durability = 37;
        entity::spawn_item(&mut ecs, Vec3::new(1.0, 65.0, 2.0), crate::item::ItemStack::new_tool(pick), 0);
        let mut helm = ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Chainmail);
        helm.durability = 5;
        entity::spawn_item(
            &mut ecs,
            Vec3::new(4.0, 65.0, 2.0),
            crate::item::ItemStack { item: crate::item::Item::Armour(helm), count: 1 },
            0,
        );
        let (spawns, _, _) = global(&mut ecs, &mut b, &mut all);
        let decoded: Vec<crate::item::Item> = spawns
            .iter()
            .filter_map(|s| crate::inventory::item_from_wire_full(&s.full_item))
            .collect();
        assert!(decoded.contains(&crate::item::Item::Tool(pick)));
        assert!(decoded.contains(&crate::item::Item::Armour(helm)));
        let tool = spawns.iter().find(|s| s.item_kind == crate::protocol::item_kind::TOOL).unwrap();
        assert_eq!(tool.item_id, 2, "iron tier still rides the legacy pair");
    }

    #[test]
    fn dropped_plan_broadcasts_neither_a_marker_nor_a_body() {
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        let data = crate::plan::PlanData::debug_3x3_stone();
        entity::spawn_item(
            &mut ecs,
            Vec3::new(1.0, 65.0, 2.0),
            crate::item::ItemStack { item: crate::item::Item::Plan(data.clone()), count: 1 },
            0,
        );
        let hashes = crate::plan::MARKER_HASHES.with(std::cell::Cell::get);
        let (spawns, _, _) = global(&mut ecs, &mut b, &mut all);
        let plan = spawns.iter().find(|s| s.kind == EntityKind::Item).unwrap();
        assert_eq!(plan.item_kind, crate::protocol::item_kind::EMPTY);
        // C3c-3-fix (L2): no joiner can use a ground Plan, so it rides no
        // marker (and hashes no body) — as before v83.
        assert_eq!(plan.full_item, WireItem::None);
        assert_eq!(crate::plan::MARKER_HASHES.with(std::cell::Cell::get), hashes, "no body hashed");
    }

    #[test]
    fn protocol_ids_are_stable_across_ticks() {
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::ZERO);
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(10.0, 0.0, 0.0));
        let first: HashSet<u32> = global(&mut ecs, &mut b, &mut all).0.iter().map(|s| s.id).collect();
        assert_eq!(first.len(), 2);
        let _ = global(&mut ecs, &mut b, &mut all);
        let in_ecs: HashSet<u32> = ecs.query::<&ProtocolId>().iter().map(|(_, p)| p.0).collect();
        assert_eq!(first, in_ecs);
    }

    #[test]
    fn cart_broadcasts_then_updates_only_when_it_moves_and_never_spuriously_despawns() {
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        let cart = crate::cart::spawn_cart(&mut ecs, (3, 4, 5));
        let (spawns, updates, despawns) = global(&mut ecs, &mut b, &mut all);
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].kind, EntityKind::Cart);
        assert_eq!(spawns[0].health, 0, "carts broadcast neutral health 0");
        assert!((spawns[0].x - 3.5).abs() < 1e-6);
        assert!((spawns[0].y - (4.0 + crate::cart::CART_Y_OFFSET)).abs() < 1e-6);
        assert_eq!(updates.len(), 1);
        assert!(despawns.is_empty());

        for _ in 0..2 {
            let (s, u, d) = global(&mut ecs, &mut b, &mut all);
            assert!(s.is_empty() && u.is_empty(), "a parked cart sends nothing");
            assert!(d.is_empty(), "a persisting cart is never despawned (flicker bug)");
        }
        ecs.get::<&mut entity::Position>(cart).unwrap().0.x += 0.25;
        assert_eq!(global(&mut ecs, &mut b, &mut all).1.len(), 1, "a rolling cart updates");

        ecs.despawn(cart).unwrap();
        assert_eq!(global(&mut ecs, &mut b, &mut all).2.len(), 1);
        assert!(global(&mut ecs, &mut b, &mut all).2.is_empty());
    }

    #[test]
    fn carts_and_mobs_share_the_broadcast_without_id_collision() {
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::ZERO);
        crate::cart::spawn_cart(&mut ecs, (10, 0, 10));
        let (spawns, updates, despawns) = global(&mut ecs, &mut b, &mut all);
        assert_eq!(spawns.len(), 2);
        assert_eq!(updates.len(), 2);
        assert!(despawns.is_empty());
        let ids: HashSet<u32> = spawns.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 2, "mob and cart must not share a ProtocolId");
    }

    #[test]
    fn projectile_broadcasts_spawn_then_updates_then_despawns_once() {
        let (mut ecs, mut b) = fresh();
        let mut all = ClientInterest::default();
        entity::spawn_arrow(
            &mut ecs,
            Vec3::new(1.0, 70.0, 2.0),
            Vec3::new(0.85, 0.0, 0.0),
            entity::ARROW_DAMAGE,
            None,
        );
        entity::spawn_blunt_projectile(&mut ecs, Vec3::new(5.0, 70.0, 5.0), Vec3::new(0.0, 0.0, 0.5), 1.0, None);

        let (spawns, updates, despawns) = global(&mut ecs, &mut b, &mut all);
        assert_eq!(spawns.len(), 2);
        assert!(spawns.iter().all(|s| s.kind == EntityKind::Projectile));
        let arrow = spawns.iter().find(|s| (s.x - 1.0).abs() < 1e-6).unwrap();
        assert!((arrow.yaw - (-0.85f32).atan2(-0.0)).abs() < 1e-5, "yaw follows the flight");
        assert_eq!(arrow.health, 0);
        let tag = |id: u32| updates.iter().find(|u| u.id == id).unwrap().state;
        let ball = spawns.iter().find(|s| (s.x - 5.0).abs() < 1e-6).unwrap();
        assert_eq!(tag(arrow.id), 0, "state 0 = arrow");
        assert_eq!(tag(ball.id), 1, "state 1 = blunt ball");
        assert!(despawns.is_empty());

        // A late joiner in range sees both arrows already in flight.
        let mut late = ClientInterest::default();
        let ev = late.events(&b.diff(&mut ecs, 0), &ecs, Some(Vec3::new(0.0, 70.0, 0.0)));
        assert_eq!(ev.spawns.iter().filter(|s| s.kind == EntityKind::Projectile).count(), 2);

        let ids: Vec<hecs::Entity> =
            ecs.query::<&entity::ProjectileEntity>().iter().map(|(e, _)| e).collect();
        for e in ids {
            let _ = ecs.despawn(e);
        }
        assert_eq!(global(&mut ecs, &mut b, &mut all).2.len(), 2, "each projectile despawns once");
        assert!(global(&mut ecs, &mut b, &mut all).2.is_empty(), "…and only once");
    }
}
