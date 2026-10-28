//! Early authority phases at the host's established machine/projectile beats.
use crate::{
    block_machines::MachineCtx,
    server::GameServer,
    sim_lend::SimSystem,
    sim_machines::{LocalMachineState, MachineRules},
};

impl GameServer {
    pub(crate) fn run_host_machine_phase(
        &mut self,
        ctx: &mut MachineCtx<'_>,
        rules: MachineRules,
        locals: &LocalMachineState,
        lent: bool,
    ) -> Vec<crate::protocol::BlockChange> {
        if lent {
            if self.machine_phase_open {
                log::error!("D4: a second machine phase in one logical tick was refused");
                return Vec::new();
            }
            self.machine_phase_open = true;
            ctx.world.sim_tally.bump(SimSystem::Machines);
        }
        let Some(mut batch) = self.sim_events.reserve_machines() else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        let players = &mut self.players;
        let registry = ctx.registry;
        crate::sim_machines::step(
            ctx,
            rules,
            locals,
            &mut batch,
            &mut changes,
            |world, pos| {
                crate::explosion::apply_joiner_blast_damage(
                    players, world, registry, pos, rules.mode,
                );
            },
        );
        // The host adapter queues these already-applied results through
        // queue_host_broadcast, separately from actual power transitions.
        self.sim_events
            .push(crate::sim_mobs::SimEvent::Machines(batch));
        changes
    }

    pub(crate) fn run_host_hopper_phase(&mut self, world: &mut crate::world::World, tick: u64) {
        if self.hopper_phase_open {
            log::error!("D4: a second hopper phase in one logical tick was refused");
            return;
        }
        self.hopper_phase_open = true;
        world.sim_tally.bump(SimSystem::Hoppers);
        crate::hopper::tick_hoppers(world, tick);
    }

    pub(crate) fn run_host_projectile_phase(
        &mut self,
        world: &mut crate::world::World,
        ecs: &mut hecs::World,
        registry: &crate::block::BlockRegistry,
        sneaking: &std::collections::HashMap<usize, bool>,
    ) {
        if self.projectile_phase_open {
            log::error!("D4: a second projectile phase in one logical tick was refused");
            return;
        }
        self.projectile_phase_open = true;
        world.sim_tally.bump(SimSystem::Projectiles);
        crate::entity::tick_projectiles(ecs, world, registry, sneaking);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d4_machines_authority_backpressure_defers_and_duplicate_phase_is_refused() {
        let mut host =
            crate::test_harness::TestHost::start_with(crate::test_harness::TestConfig::default());
        let (mut world, mut ecs) = (crate::world::World::new(), hecs::World::new());
        let (mut water, mut lava, mut fire) = (
            crate::water::WaterSystem::new(),
            crate::lava::LavaSystem::new(),
            crate::fire::FireSystem::new(),
        );
        let registry = crate::block::BlockRegistry::new();
        let pos = (0, 64, 0);
        world.set_block(pos.0, pos.1, pos.2, crate::block::CAMPFIRE);
        world.insert_campfire(
            pos,
            crate::campfire::CampfireData {
                fuel_ticks: 1,
                ..Default::default()
            },
        );
        let pet = ecs.spawn(());
        host.server
            .sim_events
            .push(crate::sim_mobs::SimEvent::ParrotAlarm {
                pet,
                pos: glam::Vec3::ZERO,
            });
        let mut ctx = MachineCtx {
            world: &mut world,
            ecs: &mut ecs,
            water: &mut water,
            lava: &mut lava,
            fire: &mut fire,
            registry: &registry,
            tick: 4,
        };
        let rules = MachineRules {
            raining: false,
            seed: 42,
            can_detonate: false,
            mode: crate::play_mode::PlayMode::Survival,
        };
        let locals = LocalMachineState::default();
        host.server
            .run_host_machine_phase(&mut ctx, rules, &locals, false);
        assert_eq!(
            ctx.world.campfire_at(pos).unwrap().fuel_ticks,
            1,
            "pending critical delivery blocks all machine mutation"
        );
        host.server.sim_events.take();
        host.server
            .run_host_machine_phase(&mut ctx, rules, &locals, true);
        assert_eq!(ctx.world.campfire_at(pos).unwrap().fuel_ticks, 0);
        assert_eq!(
            host.server.sim_events.take().len(),
            1,
            "one immediate local batch"
        );
        host.server
            .run_host_machine_phase(&mut ctx, rules, &locals, true);
        assert_eq!(
            ctx.world.get_block(pos.0, pos.1, pos.2),
            crate::block::CAMPFIRE,
            "duplicate phase cannot extinguish it early"
        );
        assert_eq!(ctx.world.sim_tally.get(SimSystem::Machines), 1);
        assert!(host.server.sim_events.take().is_empty());
    }
}
