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
//! Encoding: SHA-256 per section over an explicitly encoded view, every hash
//! container read in sorted key order, integers little-endian, `f32` as its
//! bits with `-0.0` normalised to `0.0` ([`f32_bits`]). The top digest hashes
//! the section digests in a fixed order, so a test can name the first section
//! that differs ([`GameServer::sim_digest_sections`]).
//!
//! Left out on purpose: `pop_secret` (a secret, never in a digest), `run_id`
//! and the reputation clock's owner id (per-run identities, not simulated
//! state), light (derived from blocks), and per-tick output buffers
//! (`pending_block_changes`, grants: what is sent, not what is).
//!
//! Entity key: the hecs `Entity::to_bits()` (id + generation), sorted. Entity
//! ids are outcome state here — mob AI rolls hash them
//! (`mob_ai::simple_hash(ticks, entity)`) — so a key by position would hide a
//! real divergence. They are deterministic once spawn and despawn order is
//! (the column scatter and stream-out are sorted; ECS queries iterate
//! archetype storage, not a hash map).

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

impl GameServer {
    /// One SHA-256 over [`Self::sim_digest_sections`], in their fixed order.
    /// Game-owned; NOT the Fray contract `state_hash` (see the module docs).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sim_digest(&self) -> [u8; 32] {
        let mut top = Enc::new("axenstax-sim-digest-v1");
        for (name, d) in self.sim_digest_sections() {
            top.bytes(name.as_bytes());
            top.0.update(d);
        }
        top.finish()
    }

    /// The digest's sections, each its own SHA-256: chunks, block entities,
    /// entities, clocks, per-player server state and the side systems (fire,
    /// power, columns). A twin test names the first that differs.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sim_digest_sections(&self) -> Vec<(&'static str, [u8; 32])> {
        vec![
            ("chunks", self.digest_chunks()),
            ("block_entities", self.digest_block_entities()),
            ("entities", self.digest_entities()),
            ("clocks", self.digest_clocks()),
            ("players", self.digest_players()),
            ("systems", self.digest_systems()),
        ]
    }

    /// Every loaded chunk (sorted by position): the canonical block + placed
    /// bytes (`Chunk::as_bytes`, light excluded), then every block's metadata
    /// byte (sorted), then the evicted store as a save sees it.
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
        for (pos, meta) in sorted(self.world.block_meta.iter().map(|(p, m)| (*p, *m))) {
            e.pos(pos);
            e.u8(meta);
        }
        e.finish()
    }

    /// Every block entity (sorted by position) as its bincode bytes —
    /// `BlockEntityData`'s payloads hold no hash container, so bincode is a
    /// fixed encoding — plus the side maps keyed by block position.
    fn digest_block_entities(&self) -> [u8; 32] {
        let mut e = Enc::new("block_entities");
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
        e.finish()
    }

    /// Every ECS entity, sorted by `Entity::to_bits()` (see the module docs
    /// for why the id is the key), with the components that carry outcome
    /// state: body, kind, health, AI, item stacks, timers, projectiles.
    fn digest_entities(&self) -> [u8; 32] {
        use crate::entity::{ItemEntity, Lifetime, MobKind, OnGround, Position, ProjectileEntity, Velocity};
        let mut ids: Vec<hecs::Entity> = self.ecs.iter().map(|r| r.entity()).collect();
        ids.sort_unstable_by_key(|id| id.to_bits());
        let mut e = Enc::new("entities");
        e.u64(ids.len() as u64);
        for id in ids {
            e.u64(id.to_bits().get());
            let Ok(r) = self.ecs.entity(id) else { continue };
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
        }
        e.finish()
    }

    /// The clocks: tick counter, day clock and step, falling cadence, weather
    /// window, night calendar, world active-tick total.
    fn digest_clocks(&self) -> [u8; 32] {
        let mut e = Enc::new("clocks");
        e.u64(self.tick_counter);
        e.u32(self.world_time);
        e.u32(self.world_time_step);
        e.u32(self.falling_tick_counter);
        e.u64(self.weather.rain_until);
        e.u64(self.weather.storm_until);
        e.debug(&self.night_calendar);
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

    /// The side systems: burning fire cells (with their timers), the power
    /// network's energised cells, and the loaded column set.
    fn digest_systems(&self) -> [u8; 32] {
        let mut e = Enc::new("systems");
        let fire = self.fire.digest_view();
        e.u64(fire.len() as u64);
        for (pos, next_event, age) in fire {
            e.pos(pos);
            e.u64(next_event);
            e.u8(age);
        }
        e.u64(self.world.power.energised.len() as u64);
        for (pos, level) in sorted(self.world.power.energised.iter().map(|(p, l)| (*p, *l))) {
            e.pos(pos);
            e.u8(level);
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
}
