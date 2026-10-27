//! Authority routing around the shared death rules. Local effects use the
//! early host outbox; remote kills keep their existing quest/wire path.
use crate::{
    server::GameServer,
    sim_death::{self, DeathBatch, DeathContext, DeathNotice},
    sim_mobs::SimEvent,
};
use glam::Vec3;

impl GameServer {
    /// Reserve worst-case remote fan-out and actual owned bytes, as well as
    /// the local batch/slot, before rescue, drops, raid or quest mutations.
    fn reserve_death_delivery(&mut self) -> Option<DeathBatch> {
        let count = sim_death::MAX_DEATHS_PER_PHASE;
        if self.pending_kill_events.len().checked_add(count)? > sim_death::MAX_REMOTE_EVENTS {
            return None;
        }
        self.pending_kill_events.try_reserve_exact(count).ok()?;
        let bytes = self.pending_kill_events.capacity()
            * std::mem::size_of::<(usize, crate::protocol::KillEventPacket, u32)>();
        if bytes > sim_death::MAX_REMOTE_BYTES {
            return None;
        }
        self.sim_events.reserve_deaths()
    }

    /// The host calls this at its established death boundary, before client
    /// projectiles and pickups. This borrows its actual world/ECS without
    /// opening a network tick or moving any other world phase earlier.
    /// `lent`: also owns/tallies the authority's one death phase; --no-lend
    /// keeps its separate client world and existing client tally.
    pub(crate) fn run_host_death_phase(
        &mut self,
        world: &mut crate::world::World,
        ecs: &mut hecs::World,
        locals: &[(Vec3, bool)],
        (tick, world_time): (u64, u32),
        lent: bool,
    ) {
        if lent {
            if self.death_phase_open {
                log::error!("D4: a second death phase in one logical tick was refused");
                return;
            }
            self.death_phase_open = true;
            world
                .sim_tally
                .bump(crate::sim_lend::SimSystem::DespawnDead);
        }
        sim_death::observe_pending(ecs, tick);
        let Some(mut batch) = self.reserve_death_delivery() else {
            return;
        };
        Self::forget_released_in(ecs, &self.released_joiners);
        if lent {
            self.released_joiners.clear();
        }
        let joiners = self.joiner_bodies();
        sim_death::sweep(
            world,
            ecs,
            DeathContext {
                tick,
                world_time,
                locals,
                joiners: &joiners,
            },
            &mut batch,
        );
        self.route_deaths(batch, true);
    }

    /// An owned world's established late boundary. --no-lend retains its
    /// legacy separate worlds: local kills belong to its client copy, so
    /// this copy produces only remote credit, as before.
    pub(crate) fn despawn_dead_with_drops(&mut self) {
        sim_death::observe_pending(&self.ecs, self.tick_counter);
        let Some(mut batch) = self.reserve_death_delivery() else {
            return;
        };
        let joiners = self.joiner_bodies();
        sim_death::sweep(
            &mut self.world,
            &mut self.ecs,
            DeathContext {
                tick: self.tick_counter,
                world_time: self.world_time,
                locals: &[],
                joiners: &joiners,
            },
            &mut batch,
        );
        self.route_deaths(batch, false);
    }

    fn route_deaths(&mut self, mut batch: DeathBatch, deliver_local: bool) {
        for notice in batch.notices() {
            if let DeathNotice::Killed {
                death,
                location,
                tamed,
                credit:
                    crate::combat::KillCredit::Remote {
                        slot,
                        generation,
                        nearest,
                    },
            } = *notice
            {
                self.queue_kill_event_at(slot, generation, death.kind, location, tamed, nearest);
            }
        }
        if deliver_local {
            batch.retain_local();
            if !batch.is_empty() {
                self.sim_events.push(SimEvent::Deaths(batch));
            }
        }
    }
}
