//! Local policy and presentation consumers for the shared machine facts.
use crate::{
    GameState,
    sim_lend::SimSystem,
    sim_machines::{LocalMachineState, MachineBatch, MachineFact, MachineRules},
};
use web_time::{Duration, Instant};

impl GameState {
    pub(crate) fn tick_machines(&mut self) {
        if self.joined() {
            return;
        }
        let lent = self.sim_lent();
        if !lent {
            self.sim_runs(SimSystem::Machines);
        }
        let mut locals = LocalMachineState::default();
        for (slot, player) in self.players.iter().take(4).enumerate() {
            if !player.is_dead() {
                locals.positions[slot] = Some(player.player.pos);
            }
            locals.chests[slot] = player.open_chest;
        }
        let rules = MachineRules {
            raining: self.tick_counter < self.weather_rain_until,
            seed: self.biome_gen.seed,
            can_detonate: crate::explosion::detonation_permitted(
                self.explosives_enabled,
                self.play_mode.can_edit_world(),
            ),
            mode: self.play_mode,
        };
        let mut ctx = crate::block_machines::MachineCtx {
            world: &mut self.world,
            ecs: &mut self.ecs,
            water: &mut self.water,
            lava: &mut self.lava,
            fire: &mut self.fire,
            registry: &self.registry,
            tick: self.tick_counter,
        };
        let (changes, events) = if let Some(hs) = self.hosted_server.as_mut() {
            let changes = hs
                .server
                .run_host_machine_phase(&mut ctx, rules, &locals, lent);
            if lent {
                hs.queue_host_broadcast(changes.iter().cloned());
            }
            (changes, hs.server.sim_events.take())
        } else {
            let Some(mut batch) = MachineBatch::reserve(true) else {
                return;
            };
            let mut changes = Vec::new();
            crate::sim_machines::step(
                &mut ctx,
                rules,
                &locals,
                &mut batch,
                &mut changes,
                |_, _| {},
            );
            (changes, vec![crate::sim_mobs::SimEvent::Machines(batch)])
        };
        // Relighting reaches beyond a changed cell. Use the existing light
        // neighbourhood rebuild for settled machine changes, including smoke.
        let mut dirty = ahash::AHashSet::new();
        for change in &changes {
            let (cx, cy, cz) = crate::world::World::block_to_chunk(change.x, change.y, change.z);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let chunk = (cx + dx, cy + dy, cz + dz);
                        if (0..=crate::world::MAX_CHUNK_Y).contains(&chunk.1) {
                            dirty.insert(chunk);
                        }
                    }
                }
            }
        }
        for chunk in dirty {
            self.mark_chunk_dirty(chunk);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !lent {
            self.pending_block_changes.extend(changes);
        }
        self.apply_sim_events(events);
    }

    pub(crate) fn tick_hopper_phase(&mut self) {
        if self.joined() {
            return;
        }
        if self.sim_lent() {
            if let Some(hs) = self.hosted_server.as_mut() {
                hs.server
                    .run_host_hopper_phase(&mut self.world, self.tick_counter);
            }
        } else if self.sim_runs(SimSystem::Hoppers) {
            crate::hopper::tick_hoppers(&mut self.world, self.tick_counter);
        }
    }

    pub(crate) fn tick_projectile_phase(&mut self) {
        let sneaking = self.shooter_sneaking();
        if self.sim_lent() {
            if let Some(hs) = self.hosted_server.as_mut() {
                hs.server.run_host_projectile_phase(
                    &mut self.world,
                    &mut self.ecs,
                    &self.registry,
                    &sneaking,
                );
            }
        } else if self.sim_runs(SimSystem::Projectiles) {
            crate::entity::tick_projectiles(&mut self.ecs, &self.world, &self.registry, &sneaking);
        }
    }

    pub(crate) fn apply_machine_batch(&mut self, batch: MachineBatch) {
        for &fact in batch.facts() {
            match fact {
                MachineFact::FurnaceCompleted(pos) => self.machine_furnace_completed(pos),
                MachineFact::PistonUsed => {
                    self.fire_challenge(crate::scenario::ChallengeEvent::UsePiston)
                }
                MachineFact::Keg {
                    pos,
                    hits,
                    close_chests,
                    detonated,
                } => {
                    if detonated {
                        for (slot, player) in self.players.iter_mut().take(4).enumerate() {
                            if close_chests & (1 << slot) != 0 {
                                player.open_chest = None;
                            }
                        }
                        let hits: Vec<_> = hits
                            .into_iter()
                            .enumerate()
                            .filter(|(_, damage)| *damage > 0.0)
                            .collect();
                        for slot in
                            crate::explosion::apply_player_blast_damage(&mut self.players, &hits)
                        {
                            self.dismount_parrots_on_owner_damage(slot);
                        }
                        self.present_machine_blast(pos);
                    }
                    // Preserve the existing expired-fuse challenge even when
                    // world policy gates the physical detonation.
                    self.fire_challenge(crate::scenario::ChallengeEvent::Detonate);
                }
            }
        }
        for ejection in batch.ejections {
            match ejection.effect {
                crate::dispenser::EjectEffect::Poured { .. }
                | crate::dispenser::EjectEffect::Ignited { .. } => self.audio.play_place(),
                crate::dispenser::EjectEffect::Bonemealed { cell, seed } => {
                    self.audio.play_place();
                    self.particles.poof_green(
                        glam::Vec3::new(
                            cell.0 as f32 + 0.5,
                            cell.1 as f32 + 0.5,
                            cell.2 as f32 + 0.5,
                        ),
                        4,
                        seed,
                    );
                }
                _ => {}
            }
            self.particles.puff(
                ejection.spawn,
                ejection.dir,
                4,
                self.tick_counter ^ ((ejection.pos.0 as u64) << 16) ^ ejection.pos.2 as u64,
            );
            self.audio.play_place();
        }
        for (x, y, z) in batch.saplings {
            self.particles.poof_green(
                glam::Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5),
                2,
                self.tick_counter ^ ((x as u64) << 20) ^ y as u64,
            );
        }
        if batch.rack_ready {
            self.toast = Some((
                "A seasoned log is ready on your drying rack.".to_string(),
                Instant::now() + Duration::from_secs(4),
            ));
        }
        if batch.building_completed {
            self.toast = Some((
                format!("\u{1F4DC} «{}» built — plaque placed", batch.build_name),
                Instant::now() + Duration::from_secs(4),
            ));
        }
    }

    fn present_machine_blast(&mut self, pos: crate::sim_machines::Cell) {
        let centre = glam::Vec3::new(pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
        let seed = (pos.0 as u64) << 32 ^ (pos.1 as u64) << 16 ^ pos.2 as u64;
        self.particles
            .burst_chips(centre, [0.45, 0.42, 0.40], 24, seed);
        self.particles
            .burst_chips(centre, [0.95, 0.55, 0.15], 12, seed ^ 0xFEED);
        self.particles.burst_smoke(centre, 8, seed ^ 0x51);
        self.particles.burst_embers(centre, 10, seed ^ 0xE0);
        self.audio.play_explosion();
    }
    fn machine_furnace_completed(&mut self, pos: crate::sim_machines::Cell) {
        // Audio cue on recipe completion. Tied into the
        // campfire's existing play_place hook for consistency.
        self.audio.play_place();

        // Spec 20 Phase 10 — Proof-of-Play sats trickle.
        // Find the closest player within
        // `POP_TRICKLE_RADIUS_BLOCKS` and credit them
        // `PROOF_OF_PLAY_TRICKLE_SATS` through the unified
        // helper. Charter + server-policy + tax/drain
        // gating happens inside `apply_sats_payout`; off-
        // Bitcoin servers and guardian-disabled players
        // see no sats line. No payout if no player is in
        // range (presence-gated by design — keeps the
        // mechanic from being a background farm).
        let fx = pos.0 as f32 + 0.5;
        let fy = pos.1 as f32 + 0.5;
        let fz = pos.2 as f32 + 0.5;
        // UX polish sweep Task 1 — smoke puffing out of the
        // furnace top so a smelt-complete beat actually reads.
        self.particles.burst_smoke(
            glam::Vec3::new(fx, fy + 1.0, fz),
            4,
            self.tick_counter ^ ((pos.0 as u64) << 16) ^ pos.2 as u64,
        );
        let radius_sq =
            crate::furnace::POP_TRICKLE_RADIUS_BLOCKS * crate::furnace::POP_TRICKLE_RADIUS_BLOCKS;
        let mut best: Option<(usize, f32)> = None;
        for (pidx, slot) in self.players.iter().enumerate() {
            // Post-merge review #7 — dead players don't
            // earn the PoP trickle. A corpse lying in the
            // heat radius shouldn't credit sats.
            if slot.is_dead() {
                continue;
            }
            let dx = slot.player.pos.x - fx;
            let dy = slot.player.pos.y - fy;
            let dz = slot.player.pos.z - fz;
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 <= radius_sq && best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((pidx, d2));
            }
        }
        if let Some((pidx, _)) = best {
            // Spec 28d.nostrich — the Vow suppresses every
            // sats payout for the player. Hashes still emit
            // (Proof-of-Play stays educational); settlement
            // is paused until the vow lifts.
            let charter = self.players[pidx].charter_allows_sats
                && !crate::nostrich_vow::is_vow_active(self.players[pidx].nostrich_vow.as_ref());
            let payout = crate::economy::apply_sats_payout(
                crate::furnace::PROOF_OF_PLAY_TRICKLE_SATS,
                crate::economy::PayoutKind::ProofOfPlay,
                &self.sats_policy,
                charter,
            );
            if !payout.suppressed && payout.credited > 0 {
                // No toast — the trickle is a passive
                // background credit; spamming "+1 sat" on
                // every smelt would drown the HUD. The
                // Treasury panel (Spec 16 Phase 4) shows
                // cumulative PoP earnings.
                log::debug!(
                    "PoP smelt trickle: pidx={pidx} furnace=({},{},{}) credited={}",
                    pos.0,
                    pos.1,
                    pos.2,
                    payout.credited,
                );
            }
        }
    }
}
