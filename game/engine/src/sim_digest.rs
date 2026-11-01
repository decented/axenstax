//! Keep slice 1 — a game-owned digest of the authority's simulated state
//! (`GameServer::sim_digest`), for twin-run determinism tests: two servers
//! with the same seed, fed the same events at the same ticks, must produce
//! the same digest (`test_integration/twin_run.rs`).
//!
//! **This is NOT the Fray contract `state_hash`.** No canonical checkpoint
//! exists yet (Keep handoff §3, `checkpoint()` row), and this view leaves out
//! state a restore would need (the hecs allocator free list, carry-over
//! queues, output state). It is a regression signal for one process, not a
//! cross-process or cross-architecture identity. Its byte layout may change
//! freely; nothing persists or sends it.
//!
//! Encoding: SHA-256 per named section ([`GameServer::sim_digest_sections`])
//! over an explicitly encoded view, every hash container read in sorted key
//! order, integers little-endian, `f32` as its bits with `-0.0` normalised to
//! `0.0` ([`f32_bits`]). The top digest is a SHA-256 over the sorted
//! `(name, digest)` pairs, so a test can compare section by section (a restore
//! test will expect some sections to match and name the ones that don't).
//!
//! Every ECS entity lands in exactly one entity section, first match wins:
//! `projectiles`, `item_drops`, `tamed_entities` (what the save keeps: tamed
//! pets, kept steeds, carts, quest givers, author-placed mobs), `wild_mobs`
//! (every other entity, so nothing falls outside the digest).
//!
//! Left out on purpose: `pop_secret` (a secret, never in a digest), `run_id`
//! and the reputation clock's owner id (per-run identities, not simulated
//! state), light (derived from blocks), and per-tick output buffers
//! (`pending_block_changes`, grants: what is sent, not what is). Species
//! components beyond those [`GameServer::digest_entity`] names (rabbit, bee,
//! villager data) are not encoded: a divergence there shows once it moves a
//! body. Debug-encoded values carry raw hecs ids where the type does (a wolf's
//! attack target), which holds within a run and needs a decision for restore.
//!
//! Entity key: the hecs `Entity::to_bits()` (id + generation), sorted. Entity
//! ids are outcome state here — mob AI rolls hash them
//! (`mob_ai::simple_hash(ticks, entity)`) — so a key by position would hide a
//! real divergence. They are deterministic once spawn and despawn order is
//! (the column scatter and stream-out are sorted; ECS queries iterate
//! archetype storage, not a hash map).

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::server::GameServer;

/// `f32` bits with `-0.0` folded onto `0.0`, so a sign-only difference in a
/// zero never reads as a divergence. NaN payloads are left as they are.
pub(crate) fn f32_bits(v: f32) -> u32 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

struct Enc(Sha256);

impl Enc {
    fn new(tag: &str) -> Self {
        let mut h = Sha256::new();
        h.update((tag.len() as u32).to_le_bytes());
        h.update(tag.as_bytes());
        Self(h)
    }
    fn u8(&mut self, v: u8) {
        self.0.update([v]);
    }
    fn u32(&mut self, v: u32) {
        self.0.update(v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.update(v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.update(v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn f32(&mut self, v: f32) {
        self.u32(f32_bits(v));
    }
    fn vec3(&mut self, v: glam::Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    fn pos(&mut self, p: (i32, i32, i32)) {
        self.i32(p.0);
        self.i32(p.1);
        self.i32(p.2);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u64(b.len() as u64);
        self.0.update(b);
    }
    /// A value with no explicit encoder here, by its `Debug` text. Only for
    /// types with no hash container inside (whose text would be in iteration
    /// order); floats print by value, so twins in one process agree.
    fn debug(&mut self, v: &impl std::fmt::Debug) {
        self.bytes(format!("{v:?}").as_bytes());
    }
    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

fn sorted<K: Ord + Copy, V>(it: impl Iterator<Item = (K, V)>) -> Vec<(K, V)> {
    let mut v: Vec<(K, V)> = it.collect();
    v.sort_unstable_by_key(|(k, _)| *k);
    v
}

/// Which entity section an entity belongs to (see the module docs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum EntityClass {
    Projectile,
    Item,
    Tamed,
    Wild,
}

fn entity_class(r: hecs::EntityRef<'_>) -> EntityClass {
    if r.has::<crate::entity::ProjectileEntity>() {
        return EntityClass::Projectile;
    }
    if r.has::<crate::entity::ItemEntity>() {
        return EntityClass::Item;
    }
    let tamed = r.has::<crate::cart::CartEntity>()
        || r.has::<crate::quest_npcs::QuestNpcId>()
        || r.has::<crate::entity::Authored>()
        || r.get::<&crate::wolf::WolfData>().is_some_and(|d| d.is_tamed())
        || r.get::<&crate::nostrich::NostrichData>().is_some_and(|d| d.is_tamed())
        || r.get::<&crate::horse_ai::HorseData>().is_some_and(|d| d.is_kept())
        || r.get::<&crate::companion::CompanionData>().is_some_and(|d| d.is_tamed());
    if tamed { EntityClass::Tamed } else { EntityClass::Wild }
}

impl GameServer {
    /// One SHA-256 over the sorted `(name, digest)` pairs of
    /// [`Self::sim_digest_sections`]. Game-owned; NOT the Fray contract
    /// `state_hash` (see the module docs).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sim_digest(&self) -> [u8; 32] {
        let mut top = Sha256::new();
        for (name, digest) in self.sim_digest_sections() {
            top.update((name.len() as u32).to_le_bytes());
            top.update(name.as_bytes());
            top.update(digest);
        }
        top.finalize().into()
    }

    /// The digest's named sections, each its own SHA-256, for comparing two
    /// runs section by section: `chunks`, `block_entities`, `tamed_entities`,
    /// `wild_mobs`, `item_drops`, `projectiles`, `fire_fluid_queues`, `clocks`
    /// and `players`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sim_digest_sections(&self) -> BTreeMap<&'static str, [u8; 32]> {
        BTreeMap::from([
            ("chunks", self.digest_chunks()),
            ("block_entities", self.digest_block_entities()),
            ("tamed_entities", self.digest_entities("tamed_entities", EntityClass::Tamed)),
            ("wild_mobs", self.digest_entities("wild_mobs", EntityClass::Wild)),
            ("item_drops", self.digest_entities("item_drops", EntityClass::Item)),
            ("projectiles", self.digest_entities("projectiles", EntityClass::Projectile)),
            ("fire_fluid_queues", self.digest_fire_fluid_queues()),
            ("clocks", self.digest_clocks()),
            ("players", self.digest_players()),
        ])
    }

    /// Every loaded chunk (sorted by position): the canonical block + placed
    /// bytes (`Chunk::as_bytes`, light excluded), then the evicted store as a
    /// save sees it, every block's metadata byte (sorted) and the loaded
    /// column set.
    fn digest_chunks(&self) -> [u8; 32] {
        let mut e = Enc::new("chunks");
        for (pos, chunk) in sorted(self.world.iter_chunks()) {
            e.pos(pos);
            e.bytes(&chunk.as_bytes());
        }
        let mut evicted: Vec<_> = self
            .world
            .persistable_chunks()
            .filter(|(p, _)| !self.world.has_chunk(p.0, p.1, p.2))
            .collect();
        evicted.sort_unstable_by_key(|(p, _)| *p);
        e.u64(evicted.len() as u64);
        for (pos, chunk) in evicted {
            e.pos(pos);
            e.bytes(&chunk.as_bytes());
        }
        e.u64(self.world.block_meta.len() as u64);
        for (pos, meta) in sorted(self.world.block_meta.iter().map(|(p, m)| (*p, *m))) {
            e.pos(pos);
            e.u8(meta);
        }
        let mut cols: Vec<(i32, i32)> = self.loaded_columns.iter().copied().collect();
        cols.sort_unstable();
        e.u64(cols.len() as u64);
        for (cx, cz) in cols {
            e.i32(cx);
            e.i32(cz);
        }
        e.finish()
    }

    /// Every block entity (sorted by position) as its bincode bytes —
    /// `BlockEntityData`'s payloads hold no hash container, so bincode is a
    /// fixed encoding — plus the side maps keyed by block position and the
    /// power network's energised cells.
    fn digest_block_entities(&self) -> [u8; 32] {
        let mut e = Enc::new("block_entities");
        e.u64(self.world.block_entities.len() as u64);
        for (pos, data) in sorted(self.world.block_entities.iter().map(|(p, d)| (*p, d))) {
            e.pos(pos);
            e.bytes(&bincode::serialize(data).expect("block entity encodes"));
        }
        e.u64(self.world.drying_racks.len() as u64);
        for (pos, rack) in sorted(self.world.drying_racks.iter().map(|(p, d)| (*p, d))) {
            e.pos(pos);
            e.debug(rack);
        }
        e.u64(self.world.tapped_rubber_logs.len() as u64);
        for (pos, tick) in sorted(self.world.tapped_rubber_logs.iter().map(|(p, t)| (*p, *t))) {
            e.pos(pos);
            e.u64(tick);
        }
        e.u64(self.world.face_attachments.len() as u64);
        for (pos, faces) in sorted(self.world.face_attachments.iter().map(|(p, f)| (*p, f))) {
            e.pos(pos);
            e.debug(faces);
        }
        e.u64(self.world.power.energised.len() as u64);
        for (pos, level) in sorted(self.world.power.energised.iter().map(|(p, l)| (*p, *l))) {
            e.pos(pos);
            e.u8(level);
        }
        e.finish()
    }

    /// The ECS entities of one class, sorted by `Entity::to_bits()` (see the
    /// module docs for why the id is the key).
    fn digest_entities(&self, tag: &str, class: EntityClass) -> [u8; 32] {
        let mut ids: Vec<hecs::Entity> = self
            .ecs
            .iter()
            .filter(|r| entity_class(*r) == class)
            .map(|r| r.entity())
            .collect();
        ids.sort_unstable_by_key(|id| id.to_bits());
        let mut e = Enc::new(tag);
        e.u64(ids.len() as u64);
        for id in ids {
            e.u64(id.to_bits().get());
            let Ok(r) = self.ecs.entity(id) else { continue };
            Self::digest_entity(&mut e, r);
        }
        e.finish()
    }

    /// The components that carry outcome state, each tagged so an absent one
    /// can't read as a neighbour's: body, kind, health, AI, stack, timers,
    /// projectile, cart, quest identity and the tame species' data.
    fn digest_entity(e: &mut Enc, r: hecs::EntityRef<'_>) {
        use crate::entity::{ItemEntity, Lifetime, MobKind, OnGround, Position, ProjectileEntity, Velocity};
        if let Some(p) = r.get::<&Position>() {
            e.u8(1);
            e.vec3(p.0);
        }
        if let Some(v) = r.get::<&Velocity>() {
            e.u8(2);
            e.vec3(v.0);
        }
        if let Some(k) = r.get::<&MobKind>() {
            e.u8(3);
            e.debug(&k.0);
        }
        if let Some(g) = r.get::<&OnGround>() {
            e.u8(4);
            e.bool(g.0);
        }
        if let Some(h) = r.get::<&crate::combat::Health>() {
            e.u8(5);
            e.f32(h.current);
            e.f32(h.max);
            e.u32(h.flash_timer);
            e.u32(h.invincible_timer);
        }
        if let Some(ai) = r.get::<&crate::mob_ai::MobAi>() {
            e.u8(6);
            e.u32(ai.ticks);
            e.f32(ai.facing);
            e.debug(&ai.state);
            e.debug(&ai.last_attack);
        }
        if let Some(item) = r.get::<&ItemEntity>() {
            e.u8(7);
            e.debug(&item.stack);
            e.u32(item.pickup_delay);
            e.debug(&item.dropper);
        }
        if let Some(l) = r.get::<&Lifetime>() {
            e.u8(8);
            e.u32(l.0);
        }
        if let Some(p) = r.get::<&ProjectileEntity>() {
            e.u8(9);
            e.f32(p.damage);
            e.bool(p.is_blunt);
            e.debug(&p.owner);
        }
        if let Some(c) = r.get::<&crate::cart::CartData>() {
            e.u8(10);
            e.debug(&*c);
        }
        if let Some(q) = r.get::<&crate::quest_npcs::QuestNpcId>() {
            e.u8(11);
            e.debug(&q.0);
        }
        if let Some(d) = r.get::<&crate::wolf::WolfData>() {
            e.u8(12);
            e.debug(&*d);
        }
        if let Some(d) = r.get::<&crate::nostrich::NostrichData>() {
            e.u8(13);
            e.debug(&*d);
        }
        if let Some(d) = r.get::<&crate::horse_ai::HorseData>() {
            e.u8(14);
            e.debug(&*d);
        }
        if let Some(d) = r.get::<&crate::companion::CompanionData>() {
            e.u8(15);
            e.debug(&*d);
        }
    }

    /// The clocks: tick counter, day clock and step, falling cadence, weather
    /// window, lightning (the strike rolled and not yet landed; the roll
    /// itself is a hash of the tick and window), night calendar, the
    /// reputation day's progress and the world's active-tick total.
    fn digest_clocks(&self) -> [u8; 32] {
        let mut e = Enc::new("clocks");
        e.u64(self.tick_counter);
        e.u32(self.world_time);
        e.u32(self.world_time_step);
        e.u32(self.falling_tick_counter);
        e.u64(self.weather.rain_until);
        e.u64(self.weather.storm_until);
        e.debug(&self.pending_strike);
        e.debug(&self.night_calendar);
        e.debug(&self.reputation_day.digest_view());
        e.u64(self.world.total_ticks);
        e.finish()
    }

    /// Per-player server state, in slot order (slots are the server's own
    /// stable index within a run).
    fn digest_players(&self) -> [u8; 32] {
        let mut e = Enc::new("players");
        e.u64(self.players.len() as u64);
        for sp in &self.players {
            e.vec3(sp.player.pos);
            e.vec3(sp.player.velocity);
            e.bool(sp.player.on_ground);
            e.bool(sp.player.flying);
            e.f32(sp.player.fall_distance);
            e.f32(sp.yaw);
            e.f32(sp.pitch);
            e.f32(sp.combat.health);
            e.f32(sp.combat.max_health);
            e.u8(sp.combat.hunger);
            e.bool(sp.combat.dead);
            e.u32(sp.combat.poison_ticks);
            e.bool(sp.server_simulated);
            e.bool(sp.connected);
            e.u64(sp.last_applied_input);
            e.u32(sp.step_credit);
            e.u64(sp.intent_queue.len() as u64);
            for slot in sp.inventory.slots_iter() {
                e.debug(&slot);
            }
        }
        e.finish()
    }

    /// Burning fire cells with their timers, then water's and lava's sources
    /// (sorted) and pending fronts (in queue order, which is itself state),
    /// then the leaf-decay queues (in queue order).
    fn digest_fire_fluid_queues(&self) -> [u8; 32] {
        let mut e = Enc::new("fire_fluid_queues");
        let fire = self.fire.digest_view();
        e.u64(fire.len() as u64);
        for (pos, next_event, age) in fire {
            e.pos(pos);
            e.u64(next_event);
            e.u8(age);
        }
        for (sources, spread, retract) in [self.water.digest_view(), self.lava.digest_view()] {
            e.u64(sources.len() as u64);
            for pos in sources {
                e.pos(pos);
            }
            e.u64(spread.len() as u64);
            for (x, y, z, dist) in spread {
                e.pos((x, y, z));
                e.u8(dist);
            }
            e.u64(retract.len() as u64);
            for pos in retract {
                e.pos(pos);
            }
        }
        let (check, decay, tick) = self.leaf_decay.digest_view();
        e.u64(tick);
        e.u64(check.len() as u64);
        for pos in check {
            e.pos(pos);
        }
        e.u64(decay.len() as u64);
        for (x, y, z, at) in decay {
            e.pos((x, y, z));
            e.u64(at);
        }
        e.finish()
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use crate::entity::Position;
    use crate::item::{ItemStack, MaterialId};
    use crate::mob::MobType;
    use crate::test_harness::{TestConfig, TestHost};

    /// The section names, in the contract's terms.
    const NAMES: [&str; 9] = [
        "block_entities",
        "chunks",
        "clocks",
        "fire_fluid_queues",
        "item_drops",
        "players",
        "projectiles",
        "tamed_entities",
        "wild_mobs",
    ];

    fn host() -> TestHost {
        TestHost::start_seeded(TestConfig::default(), 0)
    }

    /// Run `change` and return the sections it altered.
    fn changed_by(h: &mut TestHost, change: impl FnOnce(&mut TestHost)) -> Vec<&'static str> {
        let before = h.server.sim_digest_sections();
        let top = h.server.sim_digest();
        change(h);
        let after = h.server.sim_digest_sections();
        let moved: Vec<_> = NAMES.into_iter().filter(|n| before[n] != after[n]).collect();
        assert_eq!(top != h.server.sim_digest(), !moved.is_empty(), "the top digest covers the sections");
        moved
    }

    fn shift(h: &mut TestHost, id: hecs::Entity) {
        h.server.ecs.get::<&mut Position>(id).expect("a body").0.x += 1.0;
    }

    #[test]
    fn the_sections_are_exactly_the_named_ones() {
        let names: Vec<_> = host().server.sim_digest_sections().into_keys().collect();
        assert_eq!(names, NAMES);
    }

    /// Each kind of state moves its own section and no other, so a section
    /// by section comparison names what actually differs.
    #[test]
    fn each_change_moves_only_its_own_section() {
        let mut h = host();
        let cow = crate::entity::spawn_mob(&mut h.server.ecs, MobType::Cow, Vec3::new(1.5, 70.0, 1.5));
        let wolf = crate::entity::spawn_mob(&mut h.server.ecs, MobType::Wolf, Vec3::new(2.5, 70.0, 1.5));
        let npc = crate::entity::spawn_mob(&mut h.server.ecs, MobType::Villager, Vec3::new(3.5, 70.0, 1.5));
        h.server.ecs.insert_one(npc, crate::quest_npcs::QuestNpcId([7; 16])).unwrap();
        let cart = h.spawn_cart(4, 70, 1);
        crate::entity::spawn_item(
            &mut h.server.ecs,
            Vec3::new(5.5, 70.0, 1.5),
            ItemStack::new_material(MaterialId::Coal, 1),
            1,
        );
        crate::entity::spawn_arrow(&mut h.server.ecs, Vec3::new(6.5, 70.0, 1.5), Vec3::X, 1.0, None);
        let find = |h: &TestHost, class: fn(hecs::EntityRef<'_>) -> bool| {
            h.server.ecs.iter().find(|r| class(*r)).map(|r| r.entity()).expect("spawned")
        };
        let item = find(&h, |r| r.has::<crate::entity::ItemEntity>());
        let arrow = find(&h, |r| r.has::<crate::entity::ProjectileEntity>());

        assert_eq!(changed_by(&mut h, |h| shift(h, cow)), ["wild_mobs"]);
        assert_eq!(changed_by(&mut h, |h| shift(h, wolf)), ["wild_mobs"], "an untamed wolf is wild");
        assert_eq!(changed_by(&mut h, |h| shift(h, npc)), ["tamed_entities"], "a quest giver");
        assert_eq!(changed_by(&mut h, |h| shift(h, cart)), ["tamed_entities"], "a cart");
        assert_eq!(changed_by(&mut h, |h| shift(h, item)), ["item_drops"]);
        assert_eq!(changed_by(&mut h, |h| shift(h, arrow)), ["projectiles"]);
        assert_eq!(changed_by(&mut h, |h| h.server.tick_counter += 1), ["clocks"]);
        assert_eq!(changed_by(&mut h, |h| h.server.world_time += 1), ["clocks"]);
        assert_eq!(changed_by(&mut h, |h| h.server.pending_strike = Some(9)), ["clocks"], "lightning");
        assert_eq!(changed_by(&mut h, |h| h.server.players[0].player.pos.x += 1.0), ["players"]);
        assert_eq!(changed_by(&mut h, |h| h.server.water.add_source(1, 70, 1)), ["fire_fluid_queues"]);
        assert_eq!(changed_by(&mut h, |h| h.server.lava.add_source(2, 70, 1)), ["fire_fluid_queues"]);
        assert_eq!(changed_by(&mut h, |h| h.set_block(9, 70, 9, crate::block::STONE)), ["chunks"]);
        assert_eq!(
            changed_by(&mut h, |h| h.server.world.insert_chest((9, 71, 9), crate::chest::ChestData::new())),
            ["block_entities"]
        );
    }

    #[test]
    fn a_tamed_wolf_counts_as_tamed() {
        let mut h = host();
        let wolf = crate::entity::spawn_mob(&mut h.server.ecs, MobType::Wolf, Vec3::new(2.5, 70.0, 1.5));
        let moved_wild = changed_by(&mut h, |h| shift(h, wolf));
        assert_eq!(moved_wild, ["wild_mobs"]);
        h.server.ecs.get::<&mut crate::wolf::WolfData>(wolf).unwrap().ownership.owner_pubkey = "aa".repeat(32);
        assert_eq!(changed_by(&mut h, |h| shift(h, wolf)), ["tamed_entities"]);
    }

    #[test]
    fn minus_zero_reads_as_zero() {
        assert_eq!(super::f32_bits(-0.0), super::f32_bits(0.0));
        assert_ne!(super::f32_bits(1.0), super::f32_bits(-1.0));
    }
}
