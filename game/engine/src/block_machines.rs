//! Shared machine borrows and the dedicated authority adapter (D4 slice5).
//! `sim_machines::step` owns the common machine orchestration. Standalone/web
//! calls it locally; a lending authority borrows the actual world at the early
//! host boundary; this adapter calls it inside a dedicated server tick. The
//! legacy no-lend machine flag/mirror bridge remains explicit.

use crate::block::BlockRegistry;
use crate::fire::FireSystem;
use crate::lava::LavaSystem;
use crate::protocol::BlockChange;
use crate::server::GameServer;
use crate::water::WaterSystem;
use crate::world::World;

/// The disjoint world + simulation borrows a machine that reaches past its own
/// block-entity needs (a dispenser pours liquid, lights fire, spawns entities).
/// Built from field borrows at each call site so the client's `GameState` and
/// the server's `GameServer` share one signature.
pub struct MachineCtx<'a> {
    pub world: &'a mut World,
    pub ecs: &'a mut hecs::World,
    pub water: &'a mut WaterSystem,
    pub lava: &'a mut LavaSystem,
    pub fire: &'a mut FireSystem,
    pub registry: &'a BlockRegistry,
    /// The caller's monotonic tick counter (seeds + fire timestamps).
    pub tick: u64,
}

/// A broadcastable `BlockChange` for each cell, valued from the world as it
/// now stands (block + meta byte), via the shared `broadcast_change` helper.
pub fn cell_changes(world: &World, cells: &[(i32, i32, i32)]) -> Vec<BlockChange> {
    cells
        .iter()
        .map(|&(x, y, z)| {
            crate::game_loop::broadcast_change(world, x, y, z, world.get_block(x, y, z))
        })
        .collect()
}

impl GameServer {
    /// Dedicated/owned machine phase. A lending authority uses the same rules
    /// at its host's early phase instead of running this a second time.
    pub(crate) fn tick_block_machines(&mut self) {
        debug_assert!(self.simulates_block_machines && !self.lent);
        let rules = crate::sim_machines::MachineRules {
            raining: self.tick_counter < self.weather.rain_until,
            seed: self.biome_gen.seed,
            can_detonate: crate::explosion::detonation_permitted(self.explosives_enabled, self.play_mode.can_edit_world()),
            mode: self.play_mode,
        };
        let mut batch = crate::sim_machines::MachineBatch::reserve(false).expect("no local delivery allocation");
        let mut ctx = MachineCtx {
            world: &mut self.world, ecs: &mut self.ecs, water: &mut self.water,
            lava: &mut self.lava, fire: &mut self.fire, registry: &self.registry, tick: self.tick_counter,
        };
        let players = &mut self.players;
        let registry = &self.registry;
        crate::sim_machines::step(&mut ctx, rules, &crate::sim_machines::LocalMachineState::default(),
            &mut batch, &mut self.pending_block_changes, |world, pos| {
                crate::explosion::apply_joiner_blast_damage(players, world, registry, pos, rules.mode);
            });
    }

    pub(crate) fn tick_hoppers(&mut self) {
        debug_assert!(self.simulates_block_machines && !self.lent);
        crate::hopper::tick_hoppers(&mut self.world, self.tick_counter);
    }
}
