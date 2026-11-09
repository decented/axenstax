//! Keep slice 1 — a game-owned digest of the authority's simulated state
//! (`GameServer::sim_digest`), for twin-run determinism tests: two servers
//! with the same seed, fed the same events at the same ticks, must produce
//! the same digest (`test_integration/twin_run.rs`).
//!
//! **This is NOT the Fray contract `state_hash`**, which is a hash of the
//! canonical checkpoint (`authority_checkpoint`, Keep slice 2b). This view leaves
//! out state a restore would need (the hecs allocator free list, carry-over
//! queues, output state). It is a regression signal, not a cross-architecture
//! identity. Its byte layout may change freely; nothing persists or sends it,
//! and it compiles only for tests.
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
//! ## What is left out, and why
//!
//! Walked field by field from the struct definitions; a new field belongs in a
//! section or in this list.
//!
//! **`GameServer`.**
//! - Secret or per-run identity: `pop_secret` (a secret, never in a digest),
//!   `run_id`, `calendar_id`, the reputation clock's owner id.
//! - Config, set when the server is built or by an admin event, read by the
//!   tick: `world_name`, `seed` (shows through `chunks`), `registry`,
//!   `biome_gen`, `fire_spread_enabled`, `explosives_enabled`,
//!   `simulates_block_machines`, `column_refill_per_tick`, `rolls_lightning`,
//!   `animal_life_simulated`, `spawn_column`, `play_mode`, `is_creative`,
//!   `render_distance`, `difficulty`, `lent`.
//! - Lending mode only (the twins are dedicated): `clock_adopted`,
//!   `clock_phase_open`, `death_phase_open`, `machine_phase_open`,
//!   `hopper_phase_open`, `projectile_phase_open`, `lent_strike`,
//!   `lent_day_owner`, `lent_day_offer`, `lent_day_consumer`,
//!   `lent_local_seats`.
//! - Output, what is sent and not what is: `wire_tick_seed`, `pending_block_changes`,
//!   `pending_item_grants`, `pending_kill_events`, `pending_bred_events`,
//!   `released_joiners`, `released_mounts`, `sim_events`.
//! - Join and handover event paths: `failed_joiner_reads`, `next_attach_gen`,
//!   `tables_gone` (feeds a joiner's table verdict only),
//!   `selected_local_personal`, `gameplay_preparation`.
//!
//! **`World`.**
//! - `quest_scope`: a random per-world identity set by an attach event.
//! - Derived, or runtime flags with no meaning of their own: `evicted_columns`
//!   (from `evicted`), `salt_licks` and `procgen_plaque_sources` (from blocks),
//!   `worldgen_depth` and `worldgen_top` (only differ inside `generate_column`).
//! - Output logs drained every tick and save bookkeeping: `edited_columns`,
//!   `tables_gone`, `attachment_log`, `disk_chunks`.
//! - Content and client-side: `plan_registry`, `micro_registry`,
//!   `override_registry`, `player_wardrobe`, `workshop`, `beacon_publish_pending`,
//!   `is_workshop`, `beacon_browse_cache`, `render_hidden`.
//! - Config mirrored from the world's meta at entry: `world_type`, `ground`,
//!   `water_depth`, `time_lock`, `mobs_enabled`, `keep_inventory`, `generator`.
//! - `sim_tally`: a diagnostic count.
//!
//! **`Chunk`.** `mesh_dirty` and `persist` (runtime flags; `persist` shows
//! through the evicted store), `non_air`, `pistons`, `hoppers` (counts derived
//! from `blocks`). An evicted chunk's light: its light reads as 0, writes to it
//! are dropped, and a restore reruns the column's light pass.
//!
//! **Systems.** `LeafDecaySystem`: `check_set` and `decay_set` (from the
//! queues), `pending_sapling_drops` (output). `UpdateScheduler::in_pending`
//! (from `pending`).
//!
//! **`ServerPlayer`.** The fixture's joiners have no connection, so these
//! event-fed, per-connection states stay out:
//! - Action, pack and quest lanes: `inventory_mode`, `action_frontier`,
//!   `action_queue`, `action_results`, `pack_session`, `pack_flight`,
//!   `quest_session`, `gameplay_clock`, `personal`, `personal_read_failed`,
//!   `personal_delivery`.
//! - Window-mirror models and log-only counters: `window_events`, `obligations`,
//!   `container_sent`, `block_views`, `refused_uses`, `possession`,
//!   `strict_refusals`, `strict_corrections`.
//! - Identity and social settings fixed at join: `skin_key`, `verified_pubkey`,
//!   `display_name`, `comms`, `contacts` (a `HashMap`, so its `Debug` text is in
//!   iteration order), `chat_rate`, `worldgen_mismatch`.
//! - `QueuedInput::intent`: the keys of a queued input (its sequence number,
//!   look and health delta are encoded). It shows once it moves the body.
//!
//! **Entity components** beyond those [`GameServer::digest_entity`] names:
//! `Hitbox`, `Shooter`, `ProtocolId`, `Mirrored`, `PackRevision`, `Scattered`,
//! `NightSpawn`, `Authored`, `Summoned`, `Flying`, `Ridden`, `Baby`, `Genetics`,
//! `InLove`, `BreedCooldown`, `AlarmCooldown`, `AnimalProductState`,
//! `DeliberateCull`, `HomeHideout`, `LastAttacker`, `SatoshiMarker`, `Tethered`,
//! `VillagerComponent`, and the data of bears, bees, goats, hyenas, rabbits and
//! squid. A divergence there shows once it moves a body or changes a block; a
//! difference that never does is not seen. Debug-encoded values carry raw hecs
//! ids where the type does (a wolf's attack target), which holds within a run.
//! A restore doesn't keep hecs ids, so the entity sections are on its ledger
//! (`test_integration/authority_checkpoint.rs`, `NOT_YET_RESTORED`).
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
    /// runs section by section: `chunks`, `light`, `block_entities`,
    /// `world_tables`, `tamed_entities`, `wild_mobs`, `item_drops`,
    /// `projectiles`, `fire_fluid_queues`, `clocks` and `players`.
    pub fn sim_digest_sections(&self) -> BTreeMap<&'static str, [u8; 32]> {
        BTreeMap::from([
            ("chunks", self.digest_chunks()),
            ("light", self.digest_light()),
            ("world_tables", self.digest_world_tables()),
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

    /// The packed light array of every loaded chunk that isn't wholly dark
    /// (sorted by position). A dark chunk reads like an absent one, so a block
    /// write that conjures a chunk doesn't read as a light change.
    fn digest_light(&self) -> [u8; 32] {
        let mut e = Enc::new("light");
        for (pos, chunk) in sorted(self.world.iter_chunks()) {
            let light = chunk.light_bytes();
            if light.iter().any(|&l| l != 0) {
                e.pos(pos);
                e.bytes(light);
            }
        }
        e.finish()
    }

    /// The world's side tables that a tick, an input or an event writes and a
    /// later roll or save reads, every hash container in sorted key order.
    fn digest_world_tables(&self) -> [u8; 32] {
        let w = &self.world;
        let mut e = Enc::new("world_tables");
        e.debug(&w.bounties);
        e.u32(w.bounty_next_id);
        e.u64(w.bounty_last_refresh_tick);
        e.u64(w.brigand_hideouts.len() as u64);
        for (cell, data) in sorted(w.brigand_hideouts.iter().map(|(c, d)| (*c, d))) {
            e.i32(cell.0);
            e.i32(cell.1);
            e.debug(data);
        }
        e.u64(w.construction_anchors.len() as u64);
        for (pos, data) in sorted(w.construction_anchors.iter().map(|(p, d)| (*p, d))) {
            e.pos(pos);
            e.bytes(&bincode::serialize(data).expect("construction anchor encodes"));
        }
        e.debug(&w.scheduler.digest_view());
        let mut mains: Vec<_> = w.power.mains.iter().copied().collect();
        mains.sort_unstable();
        e.debug(&mains);
        e.debug(&w.satoshi);
        e.debug(&w.active_raids);
        e.debug(&w.raid_scheduler);
        e.debug(&sorted(w.raid_kills.iter().map(|(k, n)| (*k, *n))));
        e.debug(&sorted(w.pop_exposure.iter().map(|(p, t)| (*p, *t))));
        e.debug(&sorted(w.village_anchors.iter().map(|(c, a)| (*c, *a))));
        e.debug(&sorted(w.village_treasuries.iter().map(|(c, n)| (*c, *n))));
        let mut populated: Vec<_> = w.populated_villages.iter().copied().collect();
        populated.sort_unstable();
        e.debug(&populated);
        e.debug(&w.village_bells);
        e.u64(w.architect_plaques.len() as u64);
        for (pos, data) in sorted(w.architect_plaques.iter().map(|(p, d)| (*p, d))) {
            e.pos(pos);
            e.bytes(&bincode::serialize(data).expect("plaque encodes"));
        }
        e.debug(&w.pending_legacy_meat_drops);
        e.debug(&w.hostile_acts);
        e.debug(&w.plots);
        e.debug(&w.market_hubs);
        e.debug(&w.waypoints);
        e.debug(&w.exhibits);
        e.debug(&w.rigs);
        // Living world S2 — authorship, in the canonical (sorted) saved form, so
        // a lent host and its twin agree whatever order their hash maps hold.
        e.bytes(&bincode::serialize(&w.authors.to_saved()).expect("authorship encodes"));
        // Living world Tier 2 S3 — the use record (a sorted map, canonical). The
        // runtime wear counts are not digested: they are not world state.
        e.bytes(&bincode::serialize(&w.use_record.to_saved()).expect("use record encodes"));
        // Living world Tier 2 S4 — Plan build counts (a sorted map, canonical).
        e.bytes(&bincode::serialize(&w.plan_builds.to_saved()).expect("plan builds encode"));
        // Living world Tier 2 S5 — last seen and passed-on Plots (ordered sets, canonical).
        e.bytes(&bincode::serialize(&w.plot_succession.to_saved()).expect("plot succession encodes"));
        // Living world Tier 2 S7 — each Site Board's list (a sorted map, canonical).
        e.bytes(&bincode::serialize(&crate::site_board::to_saved(&w.site_boards)).expect("site boards encode"));
        e.u64(w.total_work);
        e.u32(self.pop_epoch);
        e.debug(&self.column_streamer);
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
        e.debug(&self.weather_lock_applied);
        e.finish()
    }

    /// Per-player server state, in slot order (slots are the server's own
    /// stable index within a run).
    fn digest_players(&self) -> [u8; 32] {
        let mut e = Enc::new("players");
        e.u64(self.players.len() as u64);
        for sp in &self.players {
            let p = &sp.player;
            e.vec3(p.pos);
            e.vec3(p.velocity);
            e.bool(p.on_ground);
            e.bool(p.flying);
            e.bool(p.in_water);
            e.f32(p.sprint_boots_mult);
            e.f32(p.fall_distance);
            e.debug(&p.pending_landing);
            e.f32(sp.yaw);
            e.f32(sp.pitch);
            e.vec3(sp.spawn_pos);

            let c = &sp.combat;
            e.f32(c.health);
            e.f32(c.max_health);
            e.u32(c.attack_cooldown);
            e.u32(c.flash_timer);
            e.u32(c.invincible_timer);
            e.f32(c.last_hit_damage);
            e.bool(c.dead);
            e.bool(c.respawn_requested);
            e.bool(c.just_died);
            e.u32(c.poison_ticks);
            e.u8(c.hunger);
            e.u8(c.max_hunger);
            e.u32(c.hunger_drain_ticks);
            e.u32(c.regen_ticks);
            e.u32(c.starvation_ticks);
            e.f32(c.starvation_floor);
            e.debug(&c.last_damage);
            e.debug(&c.breath);

            e.bool(sp.server_simulated);
            e.bool(sp.connected);
            e.bool(sp.awaiting_join);
            e.bool(sp.death_handled);
            e.u32(sp.dead_ticks);
            e.u32(sp.respawns);
            e.u32(sp.spawn_protection);
            e.u8(sp.armour_points);
            e.u8(sp.armour_wear_hits);
            e.u32(sp.interact_cooldown);
            e.u32(sp.eat_cooldown);
            e.u32(sp.attach_gen);
            e.u64(sp.next_swing_tick);
            e.u64(sp.next_shot_tick);
            e.debug(&sp.fishing);
            e.debug(&sp.drop_bucket);
            e.debug(&sp.slept_night);

            e.u64(sp.last_applied_input);
            e.u64(sp.last_input_tick);
            e.u32(sp.step_credit);
            e.bool(sp.last_sneak);
            e.bool(sp.last_acting);
            e.f32(sp.last_move_mag);
            e.bool(sp.pending_intent.is_some());
            e.u64(sp.intent_queue.len() as u64);
            for input in sp.pending_intent.iter().chain(sp.intent_queue.iter()) {
                e.u64(input.seq);
                e.f32(input.yaw);
                e.f32(input.pitch);
                e.f32(input.health_delta);
            }

            e.u64(sp.hotbar_slot as u64);
            e.u32(sp.held_item as u32);
            e.u8(sp.held_kind);
            e.u32(sp.held_id as u32);
            for slot in sp.inventory.slots_iter() {
                e.debug(&slot);
            }
            e.debug(&sp.armour);
            e.debug(&sp.cursor);
            e.debug(&sp.craft_grid);
            e.debug(&sp.station);
            e.debug(&sp.table_gone_ticks);
            e.debug(&sp.open_container);
            e.u32(sp.last_window_op_seq);

            e.debug(&sp.riding.map(|mount| mount.to_bits().get()));
            e.debug(&sp.ride);
            e.debug(&sp.seat_input);
            e.bool(sp.local_seated);

            e.bool(sp.progression_tracked);
            e.u32(sp.progression_credits);
            e.u64(sp.quest_next_incarnation);
            e.u64(sp.pack_next_incarnation);
            e.debug(&sp.pack_unfit_feedback);
        }
        e.finish()
    }

    /// Burning fire cells with their timers, then water's and lava's sources
    /// (sorted) and pending fronts (in queue order, which is itself state),
    /// and lava's spread phase, then the leaf-decay queues (in queue order).
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
        e.u8(self.lava.digest_slow());
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
    const NAMES: [&str; 11] = [
        "block_entities",
        "chunks",
        "clocks",
        "fire_fluid_queues",
        "item_drops",
        "light",
        "players",
        "projectiles",
        "tamed_entities",
        "wild_mobs",
        "world_tables",
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
        assert_eq!(changed_by(&mut h, |h| h.server.world.set_block_light_at(9, 71, 9, 5)), ["light"]);
        assert_eq!(
            changed_by(&mut h, |h| {
                let world = &mut h.server.world;
                let _ = h.server.lava.tick_spread(world);
            }),
            ["fire_fluid_queues"],
            "lava's spread phase"
        );
        assert_eq!(
            changed_by(&mut h, |h| h.server.world.insert_chest((9, 71, 9), crate::chest::ChestData::new())),
            ["block_entities"]
        );
    }

    /// A difference in any of these parts the digest in `world_tables`, even if
    /// it never reaches a block or a body.
    #[test]
    fn the_world_tables_section_sees_each_table() {
        let mut h = host();
        let hideout = crate::brigand_hideout_gen::HideoutData {
            anchor_world: [0, 70, 0],
            gx: 0,
            gz: 0,
            population: 4,
            population_target: 4,
            has_berserker: false,
            last_replenish_tick: 0,
            dormant_since: None,
            world_seed: 1,
        };
        let changes: [(&str, fn(&mut TestHost)); 9] = [
            ("bounty_next_id", |h| h.server.world.bounty_next_id += 1),
            ("bounty_last_refresh_tick", |h| h.server.world.bounty_last_refresh_tick += 1),
            ("scheduler", |h| h.server.world.scheduler.enqueue((1, 70, 1))),
            ("mains", |h| {
                h.server.world.power.mains.insert((2, 70, 1));
            }),
            ("satoshi", |h| h.server.world.satoshi.has_greeted = true),
            ("raid_scheduler", |h| h.server.world.raid_scheduler.next_raid_id += 1),
            ("pop_exposure", |h| {
                h.server.world.pop_exposure.insert((3, 70, 1), 9);
            }),
            ("populated_villages", |h| {
                h.server.world.populated_villages.insert((1, 1));
            }),
            ("total_work", |h| h.server.world.total_work += 1),
        ];
        for (name, change) in changes {
            assert_eq!(changed_by(&mut h, change), ["world_tables"], "{name}");
        }
        h.server.world.brigand_hideouts.insert((0, 0), hideout);
        assert_eq!(
            changed_by(&mut h, |h| {
                h.server.world.brigand_hideouts.get_mut(&(0, 0)).unwrap().dormant_since = Some(5);
            }),
            ["world_tables"],
            "a hideout's dormancy"
        );
    }

    /// The tick-mutated player fields the first version of the digest missed.
    #[test]
    fn the_players_section_sees_timers_and_the_window() {
        let mut h = host();
        let changes: [(&str, fn(&mut TestHost)); 10] = [
            ("hunger_drain_ticks", |h| h.server.players[0].combat.hunger_drain_ticks += 1),
            ("regen_ticks", |h| h.server.players[0].combat.regen_ticks += 1),
            ("starvation_ticks", |h| h.server.players[0].combat.starvation_ticks += 1),
            ("breath", |h| h.server.players[0].combat.breath.air -= 1),
            ("spawn_protection", |h| h.server.players[0].spawn_protection += 1),
            ("dead_ticks", |h| h.server.players[0].dead_ticks += 1),
            ("hotbar_slot", |h| h.server.players[0].hotbar_slot += 1),
            ("cursor", |h| {
                h.server.players[0].cursor = Some(ItemStack::new_material(MaterialId::Coal, 1));
            }),
            ("craft_grid", |h| {
                h.server.players[0].craft_grid[0][0] = Some(ItemStack::new_material(MaterialId::Coal, 1));
            }),
            ("armour_points", |h| h.server.players[0].armour_points += 1),
        ];
        for (name, change) in changes {
            assert_eq!(changed_by(&mut h, change), ["players"], "{name}");
        }
        let horse = crate::entity::spawn_mob(&mut h.server.ecs, MobType::Horse, Vec3::new(1.5, 70.0, 1.5));
        h.server.players[0].riding = Some(horse);
        let with_mount = h.server.sim_digest_sections()["players"];
        h.server.players[0].riding = None;
        assert_ne!(with_mount, h.server.sim_digest_sections()["players"], "riding");
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
