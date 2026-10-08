//! A joiner's mirror of the server's mobs and rail carts (Spec 04 §4.2c,
//! MP-D2a).
//!
//! A client that has joined someone else's world runs NO mob simulation of
//! its own: the server's (the host's lent world, or a dedicated server's)
//! mobs are the only mobs, and they reach the joiner through the entity
//! spawn / update / despawn diff in every `StateUpdatePacket`.
//!
//! **A separate render-only world.** Mirrored mobs live in [`RemoteMobs`]'s
//! own `hecs::World`, never in the client's sim ECS. Every client system that
//! simulates, damages, breeds, tames, rides, trades with or attributes kills
//! of mobs queries the sim ECS, so none of them can reach a mirrored mob — a
//! mirror entity carries only what the renderer reads (`Position`,
//! `Velocity`, `MobKind`, `Hitbox`, `MobAi` facing, `Health` flash, `Baby`,
//! `SatoshiMarker`), and nothing has to tolerate its missing components.
//! The entity renderer takes any ECS, so it draws this world as it draws the
//! host's. Carts mirror as a parked `CartData` whose facing and position the
//! diff sets.
//!
//! **Motion.** Each update starts a one-tick glide from where the mob is
//! drawn now to the server's position (updates come at most once a tick and
//! only on change), then extrapolates along the server's velocity for at
//! most [`MAX_EXTRAPOLATE_TICKS`] — a late update keeps the mob moving, a
//! mob pushing a wall (same position, same velocity, so no update) drifts at
//! most that far.
//!
//! **Interactions** need the server to act on its own entity (MP-D2b): a
//! swing goes to it as an `EntityAttack`, a right-click that would do
//! something to the mob as an `EntityInteract`, and the server decides.
//! [`RemoteMobs::ray_target`] tells the caller a click landed on a mirrored
//! mob (the crosshair ray, clamped to the first block), and
//! [`MirrorTarget::right_click_action`] what a right-click with what is in
//! hand asks for. Riding and villager trading stay refused with a toast
//! (D2c).
//!
//! **No private mobs.** [`purge_private_mobs`] removes any mob that appeared
//! in a joiner's own sim ECS (chunk scatter, a spawn egg, a command): the
//! game loop runs it every tick while joined, after gating the spawners.

use std::collections::HashMap;

use glam::Vec3;

use crate::entity::{Hitbox, MobKind, Position, Velocity};
use crate::mob::MobType;
use crate::protocol::{entity_flags, EntityKind, EntitySpawn, EntityUpdate};

/// Seconds per server tick (20 TPS): an update's glide length.
const TICK_SECS: f32 = 1.0 / 20.0;

/// Ticks a mirrored entity keeps moving along its last velocity after its
/// glide ends, before it holds still waiting for the next update.
pub const MAX_EXTRAPOLATE_TICKS: f32 = 2.0;

/// Mirror bookkeeping carried by every mirrored mob and cart.
#[derive(Clone, Copy, Debug)]
pub struct Mirrored {
    /// The server's `ProtocolId`.
    pub id: u32,
    /// [`entity_flags`] from the latest update.
    pub flags: u8,
    from: Vec3,
    to: Vec3,
    vel: Vec3,
    /// Seconds since the latest update.
    age: f32,
}

impl Mirrored {
    fn new(id: u32, at: Vec3) -> Self {
        Self { id, flags: 0, from: at, to: at, vel: Vec3::ZERO, age: TICK_SECS }
    }

    /// Where the entity is drawn `age` seconds after its latest update.
    fn drawn_at(&self) -> Vec3 {
        let glide = (self.age / TICK_SECS).clamp(0.0, 1.0);
        let extra = ((self.age - TICK_SECS) / TICK_SECS).clamp(0.0, MAX_EXTRAPOLATE_TICKS);
        self.from.lerp(self.to, glide) + self.vel * extra
    }
}

/// What a joiner's click landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MirrorTarget {
    pub id: u32,
    pub kind: MobType,
    pub tamed: bool,
    pub baby: bool,
    /// On a Lead (`entity_flags::TETHERED`, v70).
    pub tethered: bool,
    /// A cow that can't be milked yet, or a shorn sheep
    /// (`entity_flags::PRODUCT_NOT_READY`, FU3): false means ready OR an
    /// older server that never says.
    pub product_not_ready: bool,
}

/// What a joiner's right-click on a mirrored mob means (MP-D2b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorClick {
    /// Ask the server for this interaction (`EntityInteract`).
    Interact(crate::protocol::InteractKind),
    /// Riding (and a steed's pack) or villager trading: not yet for a joiner
    /// (D2c) — refused here with [`JOINED_INTERACTION_TOAST`].
    NotYet,
}

impl MirrorTarget {
    /// What a right-click holding `held` (`None` = empty hand) on this mob
    /// asks for, in single-player's own order of right-click branches (so a
    /// joiner's click means what it would mean in single-player): a Lead on
    /// a passive mob; breeding food (horse family only while sneaking,
    /// never a baby); a bucket on a cow, shears on a sheep (none at all if
    /// the server says it isn't ready, FU3); companion food
    /// (a Cat Treat on a cat); a steed (anything in hand: pack or mount —
    /// D2c); a Lead taken off a tethered mob (anything but a Lead in hand);
    /// a villager (talk / trade — D2c); a Bone on a wolf (Mixed Berries on a
    /// Nostrich never get there: it is a steed, mounted first, as in
    /// single-player); an empty hand on a tamed pet (sit / follow — the
    /// server checks it is ours). `None`: the item's own use goes ahead — eating, a
    /// bow, a bucket at water, a block beside a cow (review D2a LOW-2).
    pub fn right_click_action(
        &self,
        held: Option<&crate::item::Item>,
        sneak: bool,
    ) -> Option<MirrorClick> {
        use crate::item::{Item, MaterialId};
        use crate::protocol::InteractKind;
        let kind = self.kind;
        let mat = match held {
            Some(Item::Material(m)) => Some(*m),
            _ => None,
        };
        let ask = |k| Some(MirrorClick::Interact(k));
        if mat == Some(MaterialId::Lead)
            && crate::mob::mob_def(kind).category == crate::mob::MobCategory::Passive
        {
            return ask(InteractKind::LeadAttach);
        }
        if let Some(m) = mat
            && !self.baby
            && crate::breeding::breeding_food(kind) == Some(m)
            && crate::breeding::breeding_feed_allowed(kind, sneak)
        {
            return ask(InteractKind::Feed);
        }
        // FU3 (FU1 verify N8) — a cow that isn't ready, or a shorn sheep:
        // no mob action at all, so the click goes on to the block (a bucket
        // fills at water beside a cow just milked) and never on to another
        // mob arm (a tethered cow is not untied — the joiner's side of N2).
        if kind == MobType::Cow && mat == Some(MaterialId::Bucket) {
            return (!self.product_not_ready).then_some(MirrorClick::Interact(InteractKind::Milk));
        }
        if kind == MobType::Sheep
            && matches!(held, Some(Item::Tool(t)) if t.tool_type == crate::crafting::ToolType::Shears)
        {
            return (!self.product_not_ready).then_some(MirrorClick::Interact(InteractKind::Shear));
        }
        if let Some(m) = mat
            && !self.tamed
            && ((kind == MobType::Cat && m == MaterialId::CatTreat)
                || crate::companion::tame_food(kind) == Some(m))
        {
            return ask(InteractKind::Tame);
        }
        if crate::mob::is_rideable(kind) {
            return Some(MirrorClick::NotYet);
        }
        if self.tethered && mat != Some(MaterialId::Lead) {
            return ask(InteractKind::LeadDetach);
        }
        if matches!(kind, MobType::Villager | MobType::Peddler) {
            return Some(MirrorClick::NotYet);
        }
        if (kind == MobType::Wolf && mat == Some(MaterialId::Bone))
            || (kind == MobType::Nostrich && mat == Some(MaterialId::Berries))
        {
            return ask(InteractKind::Tame);
        }
        if held.is_none() && self.tamed {
            return ask(InteractKind::SitToggle);
        }
        None
    }
}

/// The joiner's mirror of the server's mobs and carts. Empty unless joined.
#[derive(Default)]
pub struct RemoteMobs {
    ecs: hecs::World,
    ids: HashMap<u32, hecs::Entity>,
}

impl RemoteMobs {
    /// The render-only world (for the entity and cart renderers).
    pub fn ecs(&self) -> &hecs::World {
        &self.ecs
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Drop the whole mirror — the server session ended.
    pub fn clear(&mut self) {
        self.ecs.clear();
        self.ids.clear();
    }

    /// Fold one batch of the server's entity diff into the mirror, in the
    /// order the server sent it: spawns, then updates, then despawns (a
    /// `StateUpdate`'s own order). Items and projectiles are not mobs — the
    /// `remote_entities` tables take those — and an update for an id this
    /// mirror doesn't hold passes through.
    pub fn apply(&mut self, spawns: &[EntitySpawn], updates: &[EntityUpdate], despawns: &[u32]) {
        for s in spawns {
            self.spawn(s);
        }
        for u in updates {
            self.update(u);
        }
        for id in despawns {
            if let Some(e) = self.ids.remove(id) {
                let _ = self.ecs.despawn(e);
            }
        }
    }

    fn spawn(&mut self, s: &EntitySpawn) {
        let at = Vec3::new(s.x, s.y, s.z);
        if !at.is_finite() {
            return;
        }
        let entity = if s.kind == EntityKind::Cart {
            let cell = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
            let e = crate::cart::spawn_cart(&mut self.ecs, cell);
            if let Ok(mut cart) = self.ecs.get::<&mut crate::cart::CartData>(e) {
                cart.facing = s.yaw;
            }
            if let Ok(mut pos) = self.ecs.get::<&mut Position>(e) {
                pos.0 = at;
            }
            e
        } else if let Some(kind) = mob_type_for(s.kind) {
            let def = crate::mob::mob_def(kind);
            let mut ai = crate::mob_ai::MobAi::new();
            ai.facing = s.yaw;
            self.ecs.spawn((
                Position(at),
                Velocity(Vec3::ZERO),
                MobKind(kind),
                Hitbox { width: def.width, height: def.height },
                ai,
                crate::combat::Health::new(f32::from(s.health.max(1))),
            ))
        } else {
            return;
        };
        let _ = self.ecs.insert_one(entity, Mirrored::new(s.id, at));
        // A re-spawn of an id we hold (it left our interest and came back
        // before the despawn was applied) replaces the old copy.
        if let Some(old) = self.ids.insert(s.id, entity) {
            let _ = self.ecs.despawn(old);
        }
    }

    fn update(&mut self, u: &EntityUpdate) {
        let Some(&e) = self.ids.get(&u.id) else {
            return;
        };
        let to = Vec3::new(u.x, u.y, u.z);
        let vel = Vec3::new(u.vx, u.vy, u.vz);
        if !to.is_finite() || !vel.is_finite() || !u.yaw.is_finite() {
            return;
        }
        let drawn = self.ecs.get::<&Position>(e).map(|p| p.0).unwrap_or(to);
        if let Ok(mut m) = self.ecs.get::<&mut Mirrored>(e) {
            m.from = drawn;
            m.to = to;
            m.vel = vel;
            m.age = 0.0;
            m.flags = u.flags;
        }
        if let Ok(mut cart) = self.ecs.get::<&mut crate::cart::CartData>(e) {
            cart.facing = u.yaw;
            return;
        }
        if let Ok(mut v) = self.ecs.get::<&mut Velocity>(e) {
            v.0 = vel;
        }
        if let Ok(mut ai) = self.ecs.get::<&mut crate::mob_ai::MobAi>(e) {
            ai.facing = u.yaw;
        }
        if let Ok(mut h) = self.ecs.get::<&mut crate::combat::Health>(e) {
            h.flash_timer = if u.flags & entity_flags::HURT != 0 {
                crate::combat::DAMAGE_FLASH_TICKS
            } else {
                0
            };
        }
        set_marker(&mut self.ecs, e, u.flags & entity_flags::BABY != 0, || crate::breeding::Baby {
            adult_at_tick: u64::MAX,
        });
        set_marker(&mut self.ecs, e, u.flags & entity_flags::SATOSHI != 0, || {
            crate::satoshi::SatoshiMarker
        });
    }

    /// Move every mirrored entity `dt` seconds along its glide (call once a
    /// frame).
    pub fn advance(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        for (_e, (m, pos)) in self.ecs.query_mut::<(&mut Mirrored, &mut Position)>() {
            m.age = (m.age + dt).min(TICK_SECS * (1.0 + MAX_EXTRAPOLATE_TICKS));
            pos.0 = m.drawn_at();
        }
    }

    /// The mirrored mob under the crosshair: the nearest whose hitbox the
    /// ray from `eye` along `look` enters within `reach` blocks. Callers clamp
    /// `reach` to the first solid block on the ray, so a block in front of
    /// the mob wins. A crosshair ray, not the melee swing's wide cone
    /// (`combat::find_attack_target`): the cone would catch a chicken at
    /// your feet or a cow beside the wall you're mining, and a joiner's
    /// break would stall on every swing it refuses (review D2a MEDIUM-3);
    /// building beside a cow still places blocks.
    pub fn ray_target(&self, eye: Vec3, look: Vec3, reach: f32) -> Option<MirrorTarget> {
        let mut best: Option<(hecs::Entity, MobType, f32)> = None;
        for (e, (pos, hb, kind)) in self.ecs.query::<(&Position, &Hitbox, &MobKind)>().iter() {
            let half = hb.width * 0.5;
            let min = pos.0 - Vec3::new(half, 0.0, half);
            let max = pos.0 + Vec3::new(half, hb.height, half);
            if let Some(t) = ray_aabb(eye, look, min, max)
                && t <= reach
                && best.is_none_or(|(_, _, bt)| t < bt)
            {
                best = Some((e, kind.0, t));
            }
        }
        let (e, kind, _) = best?;
        self.target(e, kind)
    }

    fn target(&self, e: hecs::Entity, kind: MobType) -> Option<MirrorTarget> {
        let m = *self.ecs.get::<&Mirrored>(e).ok()?;
        Some(MirrorTarget {
            id: m.id,
            kind,
            tamed: m.flags & entity_flags::TAMED != 0,
            baby: m.flags & entity_flags::BABY != 0,
            tethered: m.flags & entity_flags::TETHERED != 0,
            product_not_ready: m.flags & entity_flags::PRODUCT_NOT_READY != 0,
        })
    }

    /// The mirror entity for wire id `id` (test hook).
    #[cfg(test)]
    fn entity(&self, id: u32) -> Option<hecs::Entity> {
        self.ids.get(&id).copied()
    }

    /// Where wire id `id` is drawn and as what, if mirrored (test hook).
    #[cfg(test)]
    pub(crate) fn drawn(&self, id: u32) -> Option<(Vec3, Option<MobType>)> {
        let e = self.entity(id)?;
        let pos = self.ecs.get::<&Position>(e).ok()?.0;
        let kind = self.ecs.get::<&MobKind>(e).ok().map(|k| k.0);
        Some((pos, kind))
    }
}

/// Insert (via `make`) or remove a marker component so its presence
/// follows `on`.
fn set_marker<C: hecs::Component>(
    ecs: &mut hecs::World,
    e: hecs::Entity,
    on: bool,
    make: impl FnOnce() -> C,
) {
    let has = ecs.get::<&C>(e).is_ok();
    if on && !has {
        let _ = ecs.insert_one(e, make());
    } else if !on && has {
        let _ = ecs.remove_one::<C>(e);
    }
}

/// Distance along the ray `origin + t·dir` (t ≥ 0) at which it enters the
/// box `[min, max]`, or `None` if it misses (slab test).
fn ray_aabb(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut t_near = 0.0f32;
    let mut t_far = f32::INFINITY;
    for axis in 0..3 {
        let (o, d, lo, hi) = (origin[axis], dir[axis], min[axis], max[axis]);
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let (mut t0, mut t1) = ((lo - o) / d, (hi - o) / d);
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_near = t_near.max(t0);
        t_far = t_far.min(t1);
        if t_near > t_far {
            return None;
        }
    }
    Some(t_near)
}

/// The wire `EntityKind` → `MobType` mapping, the inverse of
/// `entity_broadcast::wire_kind_for`. `None` for the kinds that are not mobs
/// (carts, items, projectiles). Exhaustive, so a new wire kind is a compile
/// error here until it is mapped.
pub fn mob_type_for(kind: EntityKind) -> Option<MobType> {
    Some(match kind {
        EntityKind::Cow => MobType::Cow,
        EntityKind::Chicken => MobType::Chicken,
        EntityKind::Pig => MobType::Pig,
        EntityKind::Sheep => MobType::Sheep,
        EntityKind::Villager => MobType::Villager,
        EntityKind::WanderingVillager => MobType::Peddler,
        EntityKind::Wolf => MobType::Wolf,
        EntityKind::Horse => MobType::Horse,
        EntityKind::Rabbit => MobType::Rabbit,
        EntityKind::Goat => MobType::Goat,
        EntityKind::Bee => MobType::Bee,
        EntityKind::Squid => MobType::Squid,
        EntityKind::Nostrich => MobType::Nostrich,
        EntityKind::Bear => MobType::Bear,
        EntityKind::Hyena => MobType::Hyena,
        EntityKind::Brigand => MobType::Brigand,
        EntityKind::Marauder => MobType::Marauder,
        EntityKind::Berserker => MobType::Berserker,
        EntityKind::Knight => MobType::Knight,
        EntityKind::Fish => MobType::Fish,
        EntityKind::Shark => MobType::Shark,
        EntityKind::GlowSquid => MobType::GlowSquid,
        EntityKind::Fox => MobType::Fox,
        EntityKind::PolarBear => MobType::PolarBear,
        EntityKind::Reindeer => MobType::Reindeer,
        EntityKind::Cat => MobType::Cat,
        EntityKind::Parrot => MobType::Parrot,
        EntityKind::Donkey => MobType::Donkey,
        EntityKind::Mule => MobType::Mule,
        EntityKind::Crab => MobType::Crab,
        EntityKind::Cart | EntityKind::Item | EntityKind::Projectile => return None,
    })
}

/// A joiner keeps no mobs of its own: despawn every mob in its sim ECS
/// (anything a spawner, the column scatter, a spawn egg or a command put
/// there). Returns how many went. The server's mobs are in [`RemoteMobs`].
pub fn purge_private_mobs(ecs: &mut hecs::World) -> usize {
    let mobs: Vec<hecs::Entity> = ecs.query::<&MobKind>().iter().map(|(e, _)| e).collect();
    for &e in &mobs {
        let _ = ecs.despawn(e);
    }
    mobs.len()
}

/// The toast a joiner sees when it tries to ride a steed (or fit its pack)
/// or talk to a villager on someone else's world: riding needs a server-side
/// mount with prediction, trading an offer-list flow (D2c).
pub const JOINED_INTERACTION_TOAST: &str =
    "Riding and trading aren't available in someone else's world yet.";

// C3c-3r — what a joiner sees when it tries a Plan or economy action the
// server can't mirror yet. Each refusal changes nothing on the client.

/// Q on a held Plan while joined: a Plan has no wire form to throw.
pub const JOINED_PLAN_DROP_TOAST: &str = "A Plan stays with you here.";
/// Laying a Latent Plan flat while joined: attachments are the server's
/// from C3c-3b.
pub const JOINED_PLAN_LAY_TOAST: &str = "Lay Plans flat in your own worlds.";
/// The first-strike peel of a laid Blueprint while joined.
pub const JOINED_BLUEPRINT_LIFT_TOAST: &str = "Only the one who laid it can lift it here.";
/// Plan Build's Auto choice while joined (Guided build-along still works).
pub const JOINED_AUTO_BUILD_TOAST: &str = "Auto build works in your own worlds. Use Guided here.";
/// Opening or using a vendor, tip jar, auction, market hub, bazaar, bounty
/// board, commission or repair bench while joined.
pub const JOINED_ECONOMY_TOAST: &str = "Shops and markets work in your own worlds for now.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mob species, for the round-trip check (keep in step with
    /// `MobType`; `wire_kind_for`'s exhaustive match is the compile-time net).
    const ALL_MOBS: [MobType; 30] = [
        MobType::Cow,
        MobType::Chicken,
        MobType::Pig,
        MobType::Sheep,
        MobType::Villager,
        MobType::Peddler,
        MobType::Wolf,
        MobType::Horse,
        MobType::Rabbit,
        MobType::Goat,
        MobType::Bee,
        MobType::Squid,
        MobType::Nostrich,
        MobType::Bear,
        MobType::Hyena,
        MobType::Brigand,
        MobType::Marauder,
        MobType::Berserker,
        MobType::Knight,
        MobType::Fish,
        MobType::Shark,
        MobType::GlowSquid,
        MobType::Fox,
        MobType::PolarBear,
        MobType::Reindeer,
        MobType::Cat,
        MobType::Parrot,
        MobType::Donkey,
        MobType::Mule,
        MobType::Crab,
    ];

    fn spawn(id: u32, kind: EntityKind, at: Vec3) -> EntitySpawn {
        EntitySpawn {
            id,
            kind,
            x: at.x,
            y: at.y,
            z: at.z,
            yaw: 0.0,
            health: 10,
            item_kind: 0,
            item_id: 0,
            item_count: 0,
            full_item: crate::protocol::WireItem::None,
        }
    }

    fn update(id: u32, at: Vec3, flags: u8) -> EntityUpdate {
        EntityUpdate { id, x: at.x, y: at.y, z: at.z, flags, ..Default::default() }
    }

    fn drawn(m: &RemoteMobs, id: u32) -> Vec3 {
        m.ecs.get::<&Position>(m.entity(id).unwrap()).unwrap().0
    }

    #[test]
    fn every_species_round_trips_through_the_wire_kind() {
        for t in ALL_MOBS {
            let kind = crate::entity_broadcast::wire_kind_for(t);
            assert_eq!(mob_type_for(kind), Some(t), "{t:?}");
        }
        for k in [EntityKind::Cart, EntityKind::Item, EntityKind::Projectile] {
            assert_eq!(mob_type_for(k), None);
        }
    }

    #[test]
    fn every_species_mirrors_and_renders_without_its_sim_components() {
        // The mirror holds only what the renderer reads — no WolfData,
        // VillagerComponent, NostrichData… — and every species still draws.
        let mut m = RemoteMobs::default();
        let spawns: Vec<EntitySpawn> = ALL_MOBS
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                spawn(i as u32 + 1, crate::entity_broadcast::wire_kind_for(t), Vec3::new(i as f32 * 3.0, 64.0, 0.0))
            })
            .collect();
        m.apply(&spawns, &[], &[]);
        assert_eq!(m.len(), ALL_MOBS.len());
        let verts = crate::entity_model::build_entity_model_vertices(
            m.ecs(),
            0,
            Vec3::new(0.0, 64.0, -30.0),
            &crate::override_registry::OverrideRegistry::new(),
            |_| (1.0, 1.0),
        );
        assert!(!verts.is_empty(), "the mirror draws through the shared entity renderer");
    }

    #[test]
    fn spawn_move_and_despawn_follow_the_server() {
        let mut m = RemoteMobs::default();
        m.apply(&[spawn(7, EntityKind::Cow, Vec3::new(1.0, 64.0, 1.0))], &[], &[]);
        assert_eq!(drawn(&m, 7), Vec3::new(1.0, 64.0, 1.0));
        let e = m.entity(7).unwrap();
        assert_eq!(m.ecs.get::<&MobKind>(e).unwrap().0, MobType::Cow);

        // An update glides there over one tick.
        m.apply(&[], &[update(7, Vec3::new(2.0, 64.0, 1.0), 0)], &[]);
        m.advance(TICK_SECS * 0.5);
        assert!((drawn(&m, 7).x - 1.5).abs() < 1e-4, "half way after half a tick");
        m.advance(TICK_SECS * 0.5);
        assert!((drawn(&m, 7).x - 2.0).abs() < 1e-4);

        m.apply(&[], &[], &[7]);
        assert!(m.is_empty());
        assert_eq!(m.ecs.iter().count(), 0, "the entity is gone from the mirror");
    }

    #[test]
    fn velocity_extrapolates_a_little_then_holds() {
        let mut m = RemoteMobs::default();
        m.apply(&[spawn(1, EntityKind::Pig, Vec3::ZERO)], &[], &[]);
        let mut u = update(1, Vec3::new(1.0, 0.0, 0.0), 0);
        u.vx = 0.2;
        m.apply(&[], &[u], &[]);
        m.advance(1.0); // far past the glide
        let x = drawn(&m, 1).x;
        assert!((x - (1.0 + 0.2 * MAX_EXTRAPOLATE_TICKS)).abs() < 1e-4, "capped drift, got {x}");
        let v = m.ecs.get::<&Velocity>(m.entity(1).unwrap()).unwrap().0;
        assert_eq!(v.x, 0.2, "velocity drives the walk cycle");
    }

    #[test]
    fn flags_set_and_clear_the_render_markers() {
        let mut m = RemoteMobs::default();
        m.apply(&[spawn(3, EntityKind::Villager, Vec3::ZERO)], &[], &[]);
        let e = m.entity(3).unwrap();
        m.apply(&[], &[update(3, Vec3::ZERO, entity_flags::HURT | entity_flags::BABY | entity_flags::SATOSHI)], &[]);
        assert!(m.ecs.get::<&crate::combat::Health>(e).unwrap().is_flashing());
        assert!(m.ecs.get::<&crate::breeding::Baby>(e).is_ok());
        assert!(m.ecs.get::<&crate::satoshi::SatoshiMarker>(e).is_ok());
        m.apply(&[], &[update(3, Vec3::ZERO, 0)], &[]);
        assert!(!m.ecs.get::<&crate::combat::Health>(e).unwrap().is_flashing());
        assert!(m.ecs.get::<&crate::breeding::Baby>(e).is_err());
        assert!(m.ecs.get::<&crate::satoshi::SatoshiMarker>(e).is_err());
    }

    #[test]
    fn carts_mirror_with_their_facing_and_items_are_left_alone() {
        let mut m = RemoteMobs::default();
        let mut cart = spawn(5, EntityKind::Cart, Vec3::new(3.5, 64.2, 5.5));
        cart.yaw = 1.0;
        m.apply(&[cart, spawn(6, EntityKind::Item, Vec3::ZERO), spawn(8, EntityKind::Projectile, Vec3::ZERO)], &[], &[]);
        assert_eq!(m.len(), 1, "only the cart is a mirror entity");
        let e = m.entity(5).unwrap();
        assert_eq!(m.ecs.get::<&crate::cart::CartData>(e).unwrap().facing, 1.0);
        assert_eq!(drawn(&m, 5), Vec3::new(3.5, 64.2, 5.5));
        let verts = crate::entity_model::build_cart_vertices(m.ecs(), Vec3::new(0.0, 64.0, 0.0), |_| (1.0, 1.0));
        assert!(!verts.is_empty());
    }

    #[test]
    fn unknown_ids_and_non_finite_updates_are_ignored() {
        let mut m = RemoteMobs::default();
        m.apply(&[], &[update(99, Vec3::ONE, 0)], &[42]);
        assert!(m.is_empty());
        m.apply(&[spawn(1, EntityKind::Cow, Vec3::ONE)], &[update(1, Vec3::new(f32::NAN, 0.0, 0.0), 0)], &[]);
        m.advance(1.0);
        assert_eq!(drawn(&m, 1), Vec3::ONE);
        let mut bad = spawn(2, EntityKind::Cow, Vec3::ZERO);
        bad.x = f32::INFINITY;
        m.apply(&[bad], &[], &[]);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn a_respawned_id_replaces_its_old_copy() {
        let mut m = RemoteMobs::default();
        m.apply(&[spawn(4, EntityKind::Cow, Vec3::ZERO)], &[], &[]);
        m.apply(&[spawn(4, EntityKind::Cow, Vec3::new(9.0, 0.0, 0.0))], &[], &[]);
        assert_eq!(m.len(), 1);
        assert_eq!(m.ecs.iter().count(), 1);
        assert_eq!(drawn(&m, 4).x, 9.0);
    }

    #[test]
    fn clicks_find_the_mirrored_mob() {
        let mut m = RemoteMobs::default();
        m.apply(&[spawn(11, EntityKind::Wolf, Vec3::new(0.0, 64.0, -2.0))], &[update(11, Vec3::new(0.0, 64.0, -2.0), entity_flags::TAMED)], &[]);
        m.advance(1.0);
        let eye = Vec3::new(0.0, 64.5, 0.0);
        let look = Vec3::new(0.0, 0.0, -1.0);
        let hit = m.ray_target(eye, look, 5.0).expect("the crosshair is on the wolf");
        assert_eq!(
            hit,
            MirrorTarget { id: 11, kind: MobType::Wolf, tamed: true, baby: false, tethered: false, product_not_ready: false }
        );
        assert_eq!(m.ray_target(eye, Vec3::new(1.0, 0.0, 0.0), 5.0), None, "looking away misses");
        assert_eq!(m.ray_target(eye, look, 1.0), None, "out of reach (or behind a block)");
    }

    /// Review D2a MEDIUM-3 — a chicken at your feet while you dig down, or a
    /// cow beside the wall you mine, is inside the melee swing's 60° cone
    /// but not under the crosshair: it must not be the target of a break.
    #[test]
    fn a_mob_beside_the_crosshair_is_not_the_target() {
        let mut m = RemoteMobs::default();
        let at = Vec3::new(1.2, 63.0, -1.0);
        m.apply(&[spawn(3, EntityKind::Chicken, at)], &[update(3, at, 0)], &[]);
        m.advance(1.0);
        let eye = Vec3::new(0.0, 64.6, 0.0);
        let digging_down = Vec3::new(0.0, -1.0, -0.3).normalize();
        let (cone, _) =
            crate::combat::find_attack_target(m.ecs(), eye, digging_down).expect("in the cone");
        assert!(m.ecs().get::<&Mirrored>(cone).is_ok(), "the wide cone would pick it");
        assert_eq!(m.ray_target(eye, digging_down, crate::combat::ATTACK_REACH), None);
    }

    fn target(kind: MobType) -> MirrorTarget {
        MirrorTarget { id: 1, kind, tamed: false, baby: false, tethered: false, product_not_ready: false }
    }

    /// MP-D2b — a right-click on a mirror asks the server for what it would
    /// do in single-player; only riding and trading stay refused (D2c), and
    /// the item's own use goes ahead (review D2a LOW-2).
    #[test]
    fn a_right_click_asks_for_the_single_player_interaction() {
        use crate::crafting::{ToolMaterial, ToolType};
        use crate::item::{Item, MaterialId};
        use crate::protocol::InteractKind::*;
        let mat = |m| Item::Material(m);
        let ask = |k| Some(MirrorClick::Interact(k));
        let cow = target(MobType::Cow);
        assert_eq!(cow.right_click_action(Some(&mat(MaterialId::Wheat)), false), ask(Feed));
        assert_eq!(cow.right_click_action(Some(&mat(MaterialId::Bucket)), false), ask(Milk));
        assert_eq!(cow.right_click_action(Some(&mat(MaterialId::Lead)), false), ask(LeadAttach));
        assert_eq!(cow.right_click_action(Some(&mat(MaterialId::Bread)), false), None, "eat");
        assert_eq!(cow.right_click_action(Some(&Item::Block(crate::block::STONE)), false), None);
        assert_eq!(cow.right_click_action(None, false), None, "an empty hand on a wild cow");
        let calf = MirrorTarget { baby: true, ..cow };
        assert_eq!(calf.right_click_action(Some(&mat(MaterialId::Wheat)), false), None);
        let leashed = MirrorTarget { tethered: true, ..cow };
        assert_eq!(leashed.right_click_action(None, false), ask(LeadDetach));
        assert_eq!(leashed.right_click_action(Some(&mat(MaterialId::Bread)), false), ask(LeadDetach));
        assert_eq!(leashed.right_click_action(Some(&mat(MaterialId::Wheat)), false), ask(Feed));
        let shears = Item::Tool(crate::crafting::Tool::new(ToolType::Shears, ToolMaterial::Iron));
        assert_eq!(target(MobType::Sheep).right_click_action(Some(&shears), false), ask(Shear));
        assert_eq!(target(MobType::Chicken).right_click_action(Some(&shears), false), None);
        assert_eq!(target(MobType::Wolf).right_click_action(Some(&mat(MaterialId::Bone)), false), ask(Tame));
        // A Nostrich is a steed: single-player mounts it on any right-click
        // before its berry-tame branch is reached, so a joiner's is D2c too.
        assert_eq!(
            target(MobType::Nostrich).right_click_action(Some(&mat(MaterialId::Berries)), false),
            Some(MirrorClick::NotYet)
        );
        assert_eq!(target(MobType::Cat).right_click_action(Some(&mat(MaterialId::CatTreat)), false), ask(Tame));
        let pet = MirrorTarget { tamed: true, ..target(MobType::Wolf) };
        assert_eq!(pet.right_click_action(None, false), ask(SitToggle), "sit / follow");
        // Riding and trading: D2c.
        let stone = Item::Block(crate::block::STONE);
        assert_eq!(target(MobType::Villager).right_click_action(Some(&stone), false), Some(MirrorClick::NotYet));
        assert_eq!(target(MobType::Horse).right_click_action(None, false), Some(MirrorClick::NotYet));
        // A bucket aimed past a fish scoops water, as in single-player.
        assert_eq!(target(MobType::Fish).right_click_action(Some(&mat(MaterialId::Bucket)), false), None);
    }

    /// FU3 (FU1 verify N8) — a cow the server says isn't ready, or a shorn
    /// sheep (`entity_flags::PRODUCT_NOT_READY`): a bucket or shears on it is
    /// no mob action, so the click goes on to the block (the bucket fills at
    /// water) — even on a tethered one, which a later arm would untie. The
    /// bit off (ready, or an older server): asked as before.
    #[test]
    fn a_bucket_or_shears_on_an_animal_that_isnt_ready_goes_to_the_block() {
        use crate::crafting::{ToolMaterial, ToolType};
        use crate::item::{Item, MaterialId};
        use crate::protocol::InteractKind::*;
        let bucket = Item::Material(MaterialId::Bucket);
        let shears = Item::Tool(crate::crafting::Tool::new(ToolType::Shears, ToolMaterial::Iron));
        let milked = MirrorTarget { product_not_ready: true, tethered: true, ..target(MobType::Cow) };
        assert_eq!(milked.right_click_action(Some(&bucket), false), None, "not milk, and not untie");
        assert_eq!(
            milked.right_click_action(Some(&Item::Material(MaterialId::Wheat)), false),
            Some(MirrorClick::Interact(Feed)),
            "only the product's own tool skips it"
        );
        let shorn = MirrorTarget { product_not_ready: true, ..target(MobType::Sheep) };
        assert_eq!(shorn.right_click_action(Some(&shears), false), None);
        assert_eq!(
            target(MobType::Sheep).right_click_action(Some(&shears), false),
            Some(MirrorClick::Interact(Shear)),
            "the bit off: asked as before"
        );

        // From the wire: the flag on the update sets it on the target.
        let mut m = RemoteMobs::default();
        let at = Vec3::new(0.0, 63.0, -2.0);
        m.apply(&[spawn(5, EntityKind::Cow, at)], &[update(5, at, entity_flags::PRODUCT_NOT_READY)], &[]);
        m.advance(1.0);
        let hit = m.ray_target(Vec3::new(0.0, 64.0, 0.0), Vec3::new(0.0, 0.0, -1.0), 5.0).expect("on the cow");
        assert!(hit.product_not_ready);
        assert_eq!(hit.right_click_action(Some(&bucket), false), None);
    }

    #[test]
    fn purge_removes_mobs_and_keeps_everything_else() {
        let mut ecs = hecs::World::new();
        crate::entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::ZERO);
        crate::entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::ONE);
        crate::entity::spawn_item(
            &mut ecs,
            Vec3::ZERO,
            crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 1),
            0,
        );
        crate::cart::spawn_cart(&mut ecs, (0, 64, 0));
        assert_eq!(purge_private_mobs(&mut ecs), 2);
        assert_eq!(ecs.query::<&MobKind>().iter().count(), 0);
        assert_eq!(ecs.iter().count(), 2, "the joiner's own drop and cart stay");
    }
}
