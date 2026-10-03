//! Chunk mesh rebuilds after block-world mutations.
//!
//! Client-only concern — meshes live on the GPU. Simulation logic that mutates
//! blocks (falling physics, water flow, leaf decay, player place/break) emits
//! `BlockChange` records or dirty-chunk sets; this file turns those into
//! `renderer.upload_chunk_mesh` calls.
//!
//! Eventual crate home: `genesis_client`.

use crate::chunk::CHUNK_SIZE;
use crate::mesh::build_chunk_meshes;
use crate::world::World;

/// Task 8 — a companion wolf's `AttackTarget` reach. Loosely mirrors
/// `HOSTILE_MELEE_RANGE` in combat.rs (the same "close enough to bite"
/// distance mobs use against players).
const WOLF_ATTACK_REACH: f32 = 1.8;

/// Task 8 — attack-state wolves close distance faster than a leisurely
/// follow; applied on top of the normal walk speed used for `MoveToward`.
const WOLF_ATTACK_SPEED_MULT: f32 = 1.2;

/// The `WolfAction::AttackTarget` arm of `tick_wolf_companions`, pulled out
/// as a free function over `&mut hecs::World` (mirrors `mob_ai::
/// tick_golem_combat`'s shape) so it's unit-testable without a full
/// `GameState`/renderer.
///
/// `entity_id` is the wolf AI's `u64` target handle — produced from a real
/// target's `hecs::Entity::to_bits()` at the revenge/assist pivot call site
/// (`wolf::maybe_pivot_to_revenge`/`maybe_pivot_to_assist`) — inverted here
/// via `Entity::from_bits`. If the id doesn't decode to a live entity (bad
/// bits, or the target despawned since the pivot), the wolf just stops:
/// `tick_wolf`'s `until_tick` timeout on `AttackHostile`/
/// `AttackRecentAttacker` unwinds the state on its own, no panic needed here.
///
/// Friendly-fire note: `wolf::maybe_pivot_to_revenge`/`maybe_pivot_to_assist`
/// already refuse to target a tamed companion of the same owner (delegating
/// to `tameable::should_pivot_to_revenge`/`should_pivot_to_assist`, covered
/// by `wolf.rs`'s `revenge_ignores_same_owner_tamed_wolf` /
/// `assist_ignores_same_owner_tamed_wolf` tests) — so by the time an
/// `AttackTarget` reaches this function, the target is already known-hostile.
/// No same-owner filter is re-added here.
pub(crate) fn tick_wolf_attack(
    ecs: &mut hecs::World,
    wolf_id: hecs::Entity,
    wolf_pos: glam::Vec3,
    entity_id: u64,
    attack_speed: f32,
    tick: u64,
) {
    use crate::combat::Health;
    use crate::entity::{Position, Velocity};

    let target = hecs::Entity::from_bits(entity_id)
        .and_then(|e| ecs.get::<&Position>(e).ok().map(|p| (e, p.0)));

    let Some((target_entity, target_pos)) = target else {
        if let Ok(mut vel) = ecs.get::<&mut Velocity>(wolf_id) {
            vel.0.x = 0.0;
            vel.0.z = 0.0;
        }
        return;
    };

    let dx = target_pos.x - wolf_pos.x;
    let dy = target_pos.y - wolf_pos.y;
    let dz = target_pos.z - wolf_pos.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    let dir = glam::Vec3::new(dx, 0.0, dz).normalize_or_zero();

    if let Ok(mut vel) = ecs.get::<&mut Velocity>(wolf_id) {
        vel.0.x = dir.x * attack_speed;
        vel.0.z = dir.z * attack_speed;
    }
    if dir.length_squared() > 1e-6
        && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(wolf_id) {
            ai.facing = dir.z.atan2(dir.x);
        }

    if dist <= WOLF_ATTACK_REACH && tick.is_multiple_of(20)
        && let Ok(mut health) = ecs.get::<&mut Health>(target_entity) {
            health.take_damage(crate::wolf::WOLF_ASSIST_DAMAGE);
        }
}

/// Which owner combat event is rallying the wolves (Task 8b — live wiring
/// of the assist/revenge pivots).
enum WolfPivotEvent {
    /// The owner attacked the target (`last_owner_attack_tick` stamp +
    /// `AttackHostile` via `wolf::maybe_pivot_to_assist`).
    Assist,
    /// The owner took damage from the target (`last_owner_damage_tick`
    /// stamp + `AttackRecentAttacker` via `wolf::maybe_pivot_to_revenge`).
    Revenge,
}

/// Task 8b — the owner landed an attack on `target`: rally the owner's tamed
/// wolves to assist. Decision logic stays pure in `wolf::maybe_pivot_to_assist`
/// / `tameable::should_pivot_to_assist` (no friendly fire, no interrupting an
/// active fight, Sit holds); this just feeds the event into every wolf owned
/// by `owner_pubkey` and applies the approved transitions.
/// `target_is_tamed_by_owner` must be computed honestly by the caller
/// (`tameable::is_players_own_pet`) — a deliberate sneak-hit on your own pet
/// must NOT rally the pack against it.
pub(crate) fn pivot_owner_wolves_to_assist(
    ecs: &mut hecs::World,
    owner_pubkey: &str,
    target: hecs::Entity,
    target_is_tamed_by_owner: bool,
    tick: u64,
) {
    pivot_owner_wolves(
        ecs,
        owner_pubkey,
        target,
        target_is_tamed_by_owner,
        tick,
        WolfPivotEvent::Assist,
    );
}

/// Task 8b — `attacker` landed damage on the owner: rally the owner's tamed
/// wolves to revenge. Same contract as [`pivot_owner_wolves_to_assist`].
pub(crate) fn pivot_owner_wolves_to_revenge(
    ecs: &mut hecs::World,
    owner_pubkey: &str,
    attacker: hecs::Entity,
    attacker_is_tamed_by_owner: bool,
    tick: u64,
) {
    pivot_owner_wolves(
        ecs,
        owner_pubkey,
        attacker,
        attacker_is_tamed_by_owner,
        tick,
        WolfPivotEvent::Revenge,
    );
}

fn pivot_owner_wolves(
    ecs: &mut hecs::World,
    owner_pubkey: &str,
    target: hecs::Entity,
    target_is_tamed_by_owner: bool,
    tick: u64,
    event: WolfPivotEvent,
) {
    use crate::wolf::{self, WolfData};
    // Same bit-scheme as the Task 8 resolver in `tick_wolf_attack`
    // (`Entity::from_bits`) so the id round-trips.
    let target_bits = target.to_bits().get();
    for (id, data) in ecs.query_mut::<&mut WolfData>() {
        // A wolf never targets itself (belt-and-braces — the honest
        // own-pet flag already refuses every same-owner pet).
        if id == target {
            continue;
        }
        if !data.ownership.is_owned_by(owner_pubkey) {
            continue;
        }
        // Stamp the owner-event tick on every owned wolf, even when the
        // pivot itself is refused — OwnershipData records what the owner
        // observed, not what the wolf decided to do about it.
        let new_state = match event {
            WolfPivotEvent::Assist => {
                data.ownership.last_owner_attack_tick = tick;
                wolf::maybe_pivot_to_assist(data, target_bits, target_is_tamed_by_owner, tick)
            }
            WolfPivotEvent::Revenge => {
                data.ownership.last_owner_damage_tick = tick;
                wolf::maybe_pivot_to_revenge(data, target_bits, target_is_tamed_by_owner, tick)
            }
        };
        if let Some(state) = new_state {
            data.state = state;
        }
    }
}

impl super::GameState {
    /// P3 — tamed-wolf companion follow. For each tamed Wolf, map its owner
    /// pubkey (`"local-player-{slot}"`) to that player's position and run the
    /// pure [`crate::wolf::tick_wolf`], applying the resulting follow/stop
    /// velocity. Runs after `tick_mob_ai` so it overrides the generic wander
    /// for tamed wolves; untamed wolves are left to wander. Task 8 wires
    /// `WolfAction::AttackTarget` into real movement + contact damage via
    /// `tick_wolf_attack`; a resting/idle companion still just holds position.
    pub(crate) fn tick_wolf_companions(&mut self, player_positions: &[glam::Vec3]) {
        use crate::entity::{MobKind, Position, Velocity};
        use crate::wolf::{self, WolfAction, WolfData};
        let tick = self.tick_counter;
        // Gather tamed wolves first (immutable borrow), then apply (mutable),
        // so we don't alias the ECS while reading.
        let mut wolves: Vec<(hecs::Entity, glam::Vec3, WolfData)> = Vec::new();
        for (id, (pos, kind, data)) in
            self.ecs.query::<(&Position, &MobKind, &WolfData)>().iter()
        {
            if kind.0 == crate::mob::MobType::Wolf && data.is_tamed() {
                wolves.push((id, pos.0, data.clone()));
            }
        }
        if wolves.is_empty() {
            return;
        }
        let speed = crate::mob::mob_def(crate::mob::MobType::Wolf).speed / 20.0;
        for (id, pos, data) in wolves {
            let owner_pos = wolf::owner_slot_from_pubkey(data.owner_pubkey())
                .and_then(|slot| player_positions.get(slot).copied())
                .map(|v| (v.x, v.y, v.z));
            let (next, action) =
                wolf::tick_wolf(&data, (pos.x, pos.y, pos.z), owner_pos, tick);
            if let Ok(mut d) = self.ecs.get::<&mut WolfData>(id) {
                *d = next;
            }
            match action {
                WolfAction::MoveToward { x, z, .. } => {
                    let dir =
                        glam::Vec3::new(x - pos.x, 0.0, z - pos.z).normalize_or_zero();
                    if let Ok(mut vel) = self.ecs.get::<&mut Velocity>(id) {
                        vel.0.x = dir.x * speed;
                        vel.0.z = dir.z * speed;
                    }
                    if dir.length_squared() > 1e-6
                        && let Ok(mut ai) = self.ecs.get::<&mut crate::mob_ai::MobAi>(id) {
                            ai.facing = dir.z.atan2(dir.x);
                        }
                }
                // At rest by the owner, sitting, or waiting out a stale
                // command: stand still rather than drift on the leftover
                // wander velocity.
                WolfAction::NoOp | WolfAction::GiveUpFollow => {
                    if let Ok(mut vel) = self.ecs.get::<&mut Velocity>(id) {
                        vel.0.x = 0.0;
                        vel.0.z = 0.0;
                    }
                }
                // Combat-assist / revenge: close on the target and land
                // periodic contact damage. See `tick_wolf_attack` for the
                // resolve-by-id + despawn-fallback details.
                WolfAction::AttackTarget { entity_id } => {
                    tick_wolf_attack(&mut self.ecs, id, pos, entity_id, speed * WOLF_ATTACK_SPEED_MULT, tick);
                }
            }
        }
    }

    /// Spec 40 §5 (2026-06-18) — reconcile the render-hidden set with the
    /// Workshop's currently blown-up blocks. A block being blown up must not
    /// also render in place (its original texture would z-fight through the cage
    /// corner, or peek out of a carve), so it's render-hidden for the life of
    /// the blow-up and restored after. Render-ONLY: block data is untouched, so
    /// raycasting (charge continuation + sneak-collapse both aim at the real
    /// block), persistence, and physics are all unaffected. Cheap — early
    /// returns when nothing changed, which is every tick once a balloon is up
    /// (or whenever there are none).
    pub(crate) fn reconcile_workshop_render_hidden(&mut self) {
        let desired: ahash::AHashSet<(i32, i32, i32)> = self
            .world
            .workshop
            .blown_up_origins()
            .into_iter()
            .map(|o| (o[0], o[1], o[2]))
            .collect();
        if desired == self.world.render_hidden {
            return;
        }
        // Remesh every chunk whose hidden-status flipped (newly hidden OR newly
        // restored) — the symmetric difference of the old and new sets.
        let changed: Vec<(i32, i32, i32)> = desired
            .symmetric_difference(&self.world.render_hidden)
            .copied()
            .collect();
        self.world.render_hidden = desired;
        for (x, y, z) in changed {
            self.rebuild_chunk_at(x, y, z);
        }
    }

    /// Rebuild the mesh for the chunk containing the given world-space block.
    /// The block's own chunk rebuilds **immediately** (instant edit feedback);
    /// boundary-neighbour chunks are queued for the budgeted drain (Spec 39 A4)
    /// so a boundary edit can't stall the frame.
    pub(crate) fn rebuild_chunk_at(&mut self, bx: i32, by: i32, bz: i32) {
        // INVARIANT for connecting shapes (Wall / Pane, `block_shape::is_connecting`):
        // their geometry is derived from live cardinal neighbours, so a neighbour
        // edit must re-mesh the connecting block. That holds today because this
        // rebuilds the WHOLE owning chunk (a same-chunk neighbour re-meshes) and
        // marks the seam-neighbour chunk dirty below (a cross-chunk neighbour is by
        // definition on the shared seam). If this is ever narrowed to a single-cell
        // remesh, add an explicit "re-mesh my 4 cardinal neighbours" pass for
        // connecting blocks or their connection arms will go stale.
        let (cx, cy, cz) = World::block_to_chunk(bx, by, bz);

        // Primary chunk now, so the placed/broken block updates this frame.
        let meshes = build_chunk_meshes(cx, cy, cz, &self.world, &self.registry);
        self.renderer.upload_chunk((cx, cy, cz), &meshes);
        self.dirty_mesh_chunks.remove(&(cx, cy, cz));

        let cs = CHUNK_SIZE as i32;
        let lx = bx.rem_euclid(cs);
        let ly = by.rem_euclid(cs);
        let lz = bz.rem_euclid(cs);

        // Boundary neighbours: hidden-face culling on the seam may have changed,
        // but it's not visually urgent — defer to the budgeted pass.
        if lx == 0 {
            self.mark_chunk_dirty((cx - 1, cy, cz));
        }
        if lx == cs - 1 {
            self.mark_chunk_dirty((cx + 1, cy, cz));
        }
        if ly == 0 {
            self.mark_chunk_dirty((cx, cy - 1, cz));
        }
        if ly == cs - 1 {
            self.mark_chunk_dirty((cx, cy + 1, cz));
        }
        if lz == 0 {
            self.mark_chunk_dirty((cx, cy, cz - 1));
        }
        if lz == cs - 1 {
            self.mark_chunk_dirty((cx, cy, cz + 1));
        }
    }

    /// Spec 30 bugfix — every chunk within the lighting BFS blast radius
    /// (block-light reaches 14 blocks → at most one chunk in any direction →
    /// 3×3×3 = 27 chunks) needs a remesh so it picks up the new per-cell light.
    /// Spec 39 A4: this used to rebuild all 27 synchronously — the torch-place
    /// stutter. Now it just **queues** them for the budgeted drain; the light
    /// values are already correct, only the baked-in mesh lags a frame or two,
    /// nearest-first, which is imperceptible next to a hard frame stall.
    pub(crate) fn rebuild_chunks_for_lighting(&mut self, bx: i32, by: i32, bz: i32) {
        let (cx, cy, cz) = World::block_to_chunk(bx, by, bz);
        for dcx in -1..=1 {
            for dcy in -1..=1 {
                for dcz in -1..=1 {
                    let pos = (cx + dcx, cy + dcy, cz + dcz);
                    // Skip chunks outside the y range (no need to rebuild
                    // chunks above the build height or below bedrock).
                    if pos.1 < 0 || pos.1 > crate::world::MAX_CHUNK_Y {
                        continue;
                    }
                    self.mark_chunk_dirty(pos);
                }
            }
        }
    }

    /// Spec 49 (Explosives) — detonate a Blasting Keg at `pos`.
    ///
    /// The full blast: a spherical radius cleared via `explosion::resolve_blast`
    /// (bedrock / Satori immune, deepslate tough), **drop-nothing /
    /// no-Proof-of-Play** by construction (it only ever calls `set_block(AIR)`,
    /// never the mine-drop / `add_work` / HMAC path — demolition, not mining),
    /// chain-ignition of kegs caught in the radius, and entity + player damage
    /// with distance falloff + line-of-sight reduction. `explosives_enabled` /
    /// PlayMode gating arrives in P9. Particles wait on a particle framework; the
    /// boom sound fires here.
    pub(crate) fn detonate_keg(&mut self, pos: (i32, i32, i32)) {
        // Spec 49 gating — a no-op detonation (hand-lit OR electrical) when:
        //   • explosives are disabled for this world (the per-region toggle), or
        //   • the play mode is read-only (Adventure/Spectator can't break blocks).
        // The keg stays inert (its fuse is already spent); nothing breaks, no
        // damage lands. Electricity is a trigger, not a bypass. Creative +
        // Survival with explosives on detonate normally.
        if !crate::explosion::detonation_permitted(
            self.explosives_enabled,
            self.play_mode.can_edit_world(),
        ) {
            return;
        }
        let outcome = crate::explosion::resolve_blast(
            &mut self.world,
            pos,
            crate::explosion::BLAST_RADIUS,
            crate::explosion::KEG_BLAST_POWER,
        );
        // Audit 2026-09-27 — blasted fluid cells go through the same source
        // bookkeeping as a pickaxe/bucket removal (no phantom sources; the
        // crater's neighbours wake and flow in).
        crate::explosion::notify_fluids_of_blast(
            &mut self.water,
            &mut self.lava,
            &self.world,
            &outcome.destroyed,
        );
        // Blasted containers spill their contents as item entities.
        for (k, (p, stack)) in outcome.spilled.iter().cloned().enumerate() {
            crate::entity::spawn_item(
                &mut self.ecs,
                glam::Vec3::new(p.0 as f32 + 0.5, p.1 as f32 + 0.5, p.2 as f32 + 0.5),
                stack,
                (k as u32).wrapping_mul(7919) ^ 0xB1A5,
            );
        }
        for p in self.players.iter_mut() {
            if p.open_chest.is_some_and(|c| outcome.destroyed.iter().any(|&(d, _)| d == c)) {
                p.open_chest = None;
            }
        }
        // Relight + re-mesh every cleared cell; broadcast the change.
        for &(p, old) in &outcome.destroyed {
            crate::lighting::update_for_block_change(
                &mut self.world,
                p,
                old,
                crate::block::AIR,
                &self.registry,
            );
            // Spec 48 §2.3 — a blast that clears power blocks must re-evaluate
            // the network, same as a manual break. `resolve_blast` already
            // removed each cell's block-entity (incl. a destroyed device's
            // PowerDevice); this nudges the neighbours so cut cables settle.
            self.world.notify_neighbours(p);
            self.rebuild_chunks_for_lighting(p.0, p.1, p.2);
            #[cfg(not(target_arch = "wasm32"))]
            self.pending_block_changes.push(crate::game_loop::broadcast_change(
                &self.world,
                p.0,
                p.1,
                p.2,
                crate::block::AIR,
            ));
        }
        // Wake the chain-ignited kegs' power devices so their (short) fuses tick.
        for &p in &outcome.chained {
            self.world.mark_dirty(p);
        }
        self.apply_blast_damage(pos);
        // Particles (2026-07-05): the debris + smoke pass this module's doc
        // deferred "until the engine grows a particle framework" — it has one.
        let centre = glam::Vec3::new(pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
        let seed = (pos.0 as u64) << 32 ^ (pos.1 as u64) << 16 ^ pos.2 as u64;
        self.particles.burst_chips(centre, [0.45, 0.42, 0.40], 24, seed);
        self.particles.burst_chips(centre, [0.95, 0.55, 0.15], 12, seed ^ 0xFEED);
        self.particles.burst_smoke(centre, 8, seed ^ 0x51);
        self.particles.burst_embers(centre, 10, seed ^ 0xE0);
        self.audio.play_explosion();
    }

    /// Spec 49 — apply blast damage to players + mobs around a detonation centre,
    /// with distance falloff + line-of-sight reduction (the Spec 05 §6.3 hook).
    /// Computes against immutable borrows first, then applies, to keep the borrow
    /// checker happy.
    fn apply_blast_damage(&mut self, center: (i32, i32, i32)) {
        let c = (
            center.0 as f32 + 0.5,
            center.1 as f32 + 0.5,
            center.2 as f32 + 0.5,
        );
        let radius = crate::explosion::BLAST_RADIUS;
        let max_dmg = crate::explosion::KEG_BLAST_DAMAGE;

        // Players.
        let player_hits: Vec<(usize, f32)> = self
            .players
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                if slot.is_dead() {
                    return None;
                }
                let ep = (slot.player.pos.x, slot.player.pos.y, slot.player.pos.z);
                let los = crate::explosion::line_of_sight_factor(c, ep, |x, y, z| {
                    self.registry.is_solid(self.world.get_block(x, y, z))
                });
                let d = crate::explosion::blast_damage(c, ep, radius, max_dmg, los);
                (d > 0.0).then_some((i, d))
            })
            .collect();
        // Wave-hardening backlog (2026-07-11) — blast damage used to hit
        // `combat.take_damage` raw. Route through equipped armour like every
        // other damage source (Spec 28e) and run the same on-owner-damage
        // follow-up (perched parrots dismount).
        for i in crate::explosion::apply_player_blast_damage(&mut self.players, &player_hits) {
            self.dismount_parrots_on_owner_damage(i);
        }

        // Mobs (ECS).
        let mob_hits: Vec<(hecs::Entity, f32)> = self
            .ecs
            .query::<(&crate::entity::Position, &crate::combat::Health)>()
            .iter()
            .filter_map(|(e, (p, _h))| {
                let ep = (p.0.x, p.0.y, p.0.z);
                let los = crate::explosion::line_of_sight_factor(c, ep, |x, y, z| {
                    self.registry.is_solid(self.world.get_block(x, y, z))
                });
                let d = crate::explosion::blast_damage(c, ep, radius, max_dmg, los);
                (d > 0.0).then_some((e, d))
            })
            .collect();
        for (e, d) in mob_hits {
            if let Ok(mut h) = self.ecs.get::<&mut crate::combat::Health>(e) {
                h.take_damage(d);
            }
        }
    }

    /// Queue a chunk for a budgeted mesh rebuild (Spec 39 A4).
    pub(crate) fn mark_chunk_dirty(&mut self, pos: (i32, i32, i32)) {
        self.dirty_mesh_chunks.insert(pos);
    }

    /// Rebuild up to `budget` queued dirty chunk meshes, nearest to player 0
    /// first (Spec 39 A4). Called once per frame so rebuild storms spread across
    /// frames instead of stalling one. Chunks that have since unloaded are
    /// dropped without a (wasted) empty-mesh upload.
    pub(crate) fn process_dirty_meshes(&mut self, budget: usize) {
        if self.dirty_mesh_chunks.is_empty() {
            return;
        }
        let cs = CHUNK_SIZE as i32;
        let p = self.players[0].player.pos;
        let pcx = (p.x.floor() as i32).div_euclid(cs);
        let pcy = (p.y.floor() as i32).div_euclid(cs);
        let pcz = (p.z.floor() as i32).div_euclid(cs);

        let mut chunks: Vec<(i32, i32, i32)> = self.dirty_mesh_chunks.iter().copied().collect();
        chunks.sort_by_key(|&(cx, cy, cz)| {
            let (dx, dy, dz) = (cx - pcx, cy - pcy, cz - pcz);
            dx * dx + dy * dy + dz * dz
        });

        for &pos in chunks.iter().take(budget) {
            self.dirty_mesh_chunks.remove(&pos);
            if self.world.has_chunk(pos.0, pos.1, pos.2) {
                let m = build_chunk_meshes(pos.0, pos.1, pos.2, &self.world, &self.registry);
                self.renderer.upload_chunk(pos, &m);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::Health;
    use crate::entity::{self, Velocity};
    use crate::mob::MobType;

    /// Task 8 — a wolf 5 blocks from its Brigand target should steer toward
    /// it (velocity direction, not just magnitude) even though it's out of
    /// bite range.
    #[test]
    fn attack_target_drives_velocity_toward_target() {
        let mut ecs = hecs::World::new();
        let wolf = entity::spawn_mob(&mut ecs, MobType::Wolf, glam::Vec3::new(0.0, 64.0, 0.0));
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(5.0, 64.0, 0.0));
        let entity_id = brigand.to_bits().get();

        tick_wolf_attack(&mut ecs, wolf, glam::Vec3::new(0.0, 64.0, 0.0), entity_id, 1.2, 1);

        let vel = ecs.get::<&Velocity>(wolf).unwrap().0;
        assert!(vel.x > 0.0, "wolf should steer toward the brigand at +x, got {vel:?}");
        assert!((vel.z).abs() < 1e-4, "no lateral drift expected, got {vel:?}");
        // Out of WOLF_ATTACK_REACH — no contact damage yet.
        let hp = ecs.get::<&Health>(brigand).unwrap().current;
        assert_eq!(hp, crate::mob::mob_def(MobType::Brigand).health as f32);
    }

    /// Task 8 — adjacent + a tick on the 20-multiple cadence lands
    /// `WOLF_ASSIST_DAMAGE` on the target.
    #[test]
    fn attack_target_lands_damage_when_adjacent_on_cadence_tick() {
        let mut ecs = hecs::World::new();
        let wolf = entity::spawn_mob(&mut ecs, MobType::Wolf, glam::Vec3::new(0.0, 64.0, 0.0));
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(1.0, 64.0, 0.0));
        let entity_id = brigand.to_bits().get();
        let starting_hp = ecs.get::<&Health>(brigand).unwrap().current;

        tick_wolf_attack(&mut ecs, wolf, glam::Vec3::new(0.0, 64.0, 0.0), entity_id, 1.2, 20);

        let hp = ecs.get::<&Health>(brigand).unwrap().current;
        assert_eq!(
            hp,
            starting_hp - crate::wolf::WOLF_ASSIST_DAMAGE,
            "adjacent + tick 20 should land exactly one hit of WOLF_ASSIST_DAMAGE"
        );
    }

    /// Task 8 — adjacent but off the 20-tick cadence: movement still
    /// updates, but no damage lands (throttles the bite rate).
    #[test]
    fn attack_target_no_damage_off_cadence_tick() {
        let mut ecs = hecs::World::new();
        let wolf = entity::spawn_mob(&mut ecs, MobType::Wolf, glam::Vec3::new(0.0, 64.0, 0.0));
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(1.0, 64.0, 0.0));
        let entity_id = brigand.to_bits().get();
        let starting_hp = ecs.get::<&Health>(brigand).unwrap().current;

        tick_wolf_attack(&mut ecs, wolf, glam::Vec3::new(0.0, 64.0, 0.0), entity_id, 1.2, 21);

        let hp = ecs.get::<&Health>(brigand).unwrap().current;
        assert_eq!(hp, starting_hp, "off-cadence tick must not land damage");
    }

    /// Task 8 — if the target entity no longer exists (despawned since the
    /// pivot set the id), the wolf falls back gracefully: velocity zeroed,
    /// no panic. The state machine's own `until_tick` timeout unwinds
    /// AttackHostile/AttackRecentAttacker separately.
    #[test]
    fn attack_target_despawned_zeroes_velocity_without_panic() {
        let mut ecs = hecs::World::new();
        let wolf = entity::spawn_mob(&mut ecs, MobType::Wolf, glam::Vec3::new(0.0, 64.0, 0.0));
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(5.0, 64.0, 0.0));
        let entity_id = brigand.to_bits().get();
        ecs.despawn(brigand).unwrap();
        // Give the wolf some leftover velocity to prove it gets zeroed, not
        // just left alone.
        if let Ok(mut vel) = ecs.get::<&mut Velocity>(wolf) {
            vel.0.x = 3.0;
            vel.0.z = 3.0;
        }

        tick_wolf_attack(&mut ecs, wolf, glam::Vec3::new(0.0, 64.0, 0.0), entity_id, 1.2, 20);

        let vel = ecs.get::<&Velocity>(wolf).unwrap().0;
        assert_eq!(vel.x, 0.0);
        assert_eq!(vel.z, 0.0);
    }

    /// Task 8 — an invalid id (e.g. `0`, which isn't a valid `NonZeroU64`
    /// bit pattern) must not panic either.
    #[test]
    fn attack_target_invalid_id_does_not_panic() {
        let mut ecs = hecs::World::new();
        let wolf = entity::spawn_mob(&mut ecs, MobType::Wolf, glam::Vec3::new(0.0, 64.0, 0.0));

        tick_wolf_attack(&mut ecs, wolf, glam::Vec3::new(0.0, 64.0, 0.0), 0, 1.2, 20);

        let vel = ecs.get::<&Velocity>(wolf).unwrap().0;
        assert_eq!(vel.x, 0.0);
        assert_eq!(vel.z, 0.0);
    }

    // ── Task 8b — live pivot wiring (assist/revenge event appliers) ──────

    use crate::wolf::{WolfAiState, WolfData, ASSIST_WINDOW_TICKS, REVENGE_WINDOW_TICKS};

    /// Spawn a Wolf tamed by `owner` in FollowOwner state.
    fn spawn_tamed_wolf(ecs: &mut hecs::World, owner: &str) -> hecs::Entity {
        let wolf = entity::spawn_mob(ecs, MobType::Wolf, glam::Vec3::new(0.0, 64.0, 0.0));
        {
            let mut d = ecs.get::<&mut WolfData>(wolf).unwrap();
            d.ownership.owner_pubkey = owner.to_string();
            d.state = WolfAiState::FollowOwner;
        }
        wolf
    }

    /// Task 8b — the owner's landed attack rallies their tamed wolf into
    /// AttackHostile, and the target bits round-trip through the Task 8
    /// resolver (`tick_wolf_attack` steers at the same entity).
    #[test]
    fn assist_event_pivots_owned_wolf_and_bits_round_trip() {
        let mut ecs = hecs::World::new();
        let wolf = spawn_tamed_wolf(&mut ecs, "local-player-0");
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(5.0, 64.0, 0.0));

        pivot_owner_wolves_to_assist(&mut ecs, "local-player-0", brigand, false, 100);

        let (state, attack_tick) = {
            let d = ecs.get::<&WolfData>(wolf).unwrap();
            (d.state, d.ownership.last_owner_attack_tick)
        };
        assert_eq!(attack_tick, 100, "owner-attack tick should be stamped");
        let WolfAiState::AttackHostile { target_id, until_tick } = state else {
            panic!("expected AttackHostile, got {state:?}");
        };
        assert_eq!(until_tick, 100 + ASSIST_WINDOW_TICKS);
        assert_eq!(target_id, brigand.to_bits().get(), "bits must round-trip");

        // Round-trip: the resolver steers the wolf at the pivoted-to target.
        tick_wolf_attack(&mut ecs, wolf, glam::Vec3::new(0.0, 64.0, 0.0), target_id, 1.2, 101);
        let vel = ecs.get::<&Velocity>(wolf).unwrap().0;
        assert!(vel.x > 0.0, "wolf should chase the pivoted target at +x, got {vel:?}");
    }

    /// Task 8b — a mob landing damage on the owner rallies the wolf into
    /// AttackRecentAttacker with the revenge window.
    #[test]
    fn revenge_event_pivots_to_attack_recent_attacker() {
        let mut ecs = hecs::World::new();
        let wolf = spawn_tamed_wolf(&mut ecs, "local-player-0");
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(2.0, 64.0, 0.0));

        pivot_owner_wolves_to_revenge(&mut ecs, "local-player-0", brigand, false, 200);

        let d = ecs.get::<&WolfData>(wolf).unwrap();
        assert_eq!(d.ownership.last_owner_damage_tick, 200);
        assert_eq!(
            d.state,
            WolfAiState::AttackRecentAttacker {
                target_id: brigand.to_bits().get(),
                until_tick: 200 + REVENGE_WINDOW_TICKS,
            }
        );
    }

    /// Task 8b — Sit is a deliberate "stay here" command: a sitting wolf
    /// records the owner event but does NOT pivot into combat.
    #[test]
    fn sitting_wolf_stays_sitting_on_owner_events() {
        let mut ecs = hecs::World::new();
        let wolf = spawn_tamed_wolf(&mut ecs, "local-player-0");
        ecs.get::<&mut WolfData>(wolf).unwrap().state = WolfAiState::Sit;
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(2.0, 64.0, 0.0));

        pivot_owner_wolves_to_assist(&mut ecs, "local-player-0", brigand, false, 100);
        assert_eq!(ecs.get::<&WolfData>(wolf).unwrap().state, WolfAiState::Sit);

        pivot_owner_wolves_to_revenge(&mut ecs, "local-player-0", brigand, false, 100);
        let d = ecs.get::<&WolfData>(wolf).unwrap();
        assert_eq!(d.state, WolfAiState::Sit, "Sit must hold through both events");
        // The observation ticks are still recorded.
        assert_eq!(d.ownership.last_owner_attack_tick, 100);
        assert_eq!(d.ownership.last_owner_damage_tick, 100);
    }

    /// Task 8b — another player's wolf ignores this owner's events entirely
    /// (no pivot, no tick stamps).
    #[test]
    fn other_players_wolf_does_not_pivot() {
        let mut ecs = hecs::World::new();
        let other_wolf = spawn_tamed_wolf(&mut ecs, "local-player-1");
        let brigand =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(2.0, 64.0, 0.0));

        pivot_owner_wolves_to_assist(&mut ecs, "local-player-0", brigand, false, 100);
        pivot_owner_wolves_to_revenge(&mut ecs, "local-player-0", brigand, false, 100);

        let d = ecs.get::<&WolfData>(other_wolf).unwrap();
        assert_eq!(d.state, WolfAiState::FollowOwner, "player 1's wolf must not react");
        assert_eq!(d.ownership.last_owner_attack_tick, 0);
        assert_eq!(d.ownership.last_owner_damage_tick, 0);
    }

    /// Task 8b — a deliberate sneak-hit on the owner's OWN pet (honest flag
    /// = true) must not rally the pack against it.
    #[test]
    fn own_pet_target_does_not_rally_the_pack() {
        let mut ecs = hecs::World::new();
        let wolf = spawn_tamed_wolf(&mut ecs, "local-player-0");
        let own_cat_stand_in = spawn_tamed_wolf(&mut ecs, "local-player-0");

        pivot_owner_wolves_to_assist(&mut ecs, "local-player-0", own_cat_stand_in, true, 100);

        let d = ecs.get::<&WolfData>(wolf).unwrap();
        assert_eq!(
            d.state,
            WolfAiState::FollowOwner,
            "no friendly fire — pack must not turn on the owner's own pet"
        );
        // The second pet itself must not self-target either.
        let d2 = ecs.get::<&WolfData>(own_cat_stand_in).unwrap();
        assert_eq!(d2.state, WolfAiState::FollowOwner);
    }

    /// Task 8b — a wolf already in an active fight keeps its current target
    /// (the pure API refuses mid-combat retargets; the wiring must not
    /// bypass it).
    #[test]
    fn wolf_in_active_combat_keeps_current_target() {
        let mut ecs = hecs::World::new();
        let wolf = spawn_tamed_wolf(&mut ecs, "local-player-0");
        let first = entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(2.0, 64.0, 0.0));
        let second =
            entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(4.0, 64.0, 0.0));
        let engaged = WolfAiState::AttackHostile {
            target_id: first.to_bits().get(),
            until_tick: 1000,
        };
        ecs.get::<&mut WolfData>(wolf).unwrap().state = engaged;

        pivot_owner_wolves_to_assist(&mut ecs, "local-player-0", second, false, 100);

        assert_eq!(
            ecs.get::<&WolfData>(wolf).unwrap().state,
            engaged,
            "an active fight must not be interrupted by a new assist target"
        );
    }
}

// Friendly-fire note: `wolf::maybe_pivot_to_revenge` / `maybe_pivot_to_assist`
// already refuse a same-owner tamed target before an `AttackTarget` is ever
// produced (see `revenge_ignores_same_owner_tamed_wolf` /
// `assist_ignores_same_owner_tamed_wolf` in `wolf.rs`), so `tick_wolf_attack`
// intentionally does not re-check ownership — covered above at the top of
// this file's doc comment on `tick_wolf_attack` too.
