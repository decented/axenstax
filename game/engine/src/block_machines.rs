//! Block machines on the authoritative server (audit T1-3, 2026-10-05).
//!
//! The block machines — hives, crops + saplings, dispensers/droppers, pistons,
//! furnaces, composters, Blasting Keg fuses and hoppers — each have ONE pure
//! implementation in their home module (`bee_hive::accumulate_honey`,
//! `growth::tick_growth`, `dispenser::realise_order`, `piston::tick_pistons`,
//! `furnace::tick_all`, `composter::tick_all`, `power::tick_keg_fuses` +
//! `explosion::detonate_keg_core`, `hopper::tick_hoppers`). Two callers drive
//! them, never both on the same world:
//!
//! - **The client loop** (`game_loop.rs`) — single-player and the LAN/online
//!   host. On a host it pushes the results into `pending_block_changes` → the
//!   server → joiners, and `HostedServer::mirror_host_world_state` copies the
//!   host's machine block-entities (furnaces, dispensers, chests, …) INTO the
//!   server world every tick.
//! - **`GameServer::tick`** — only when [`GameServer::simulates_block_machines`]
//!   is set, i.e. when no local host client simulates them: the dedicated server
//!   (`HostedServer::start` with 0 local players — `server_main` and the
//!   WebSocket dedicated path).
//!
//! The flag is the no-double-tick rule: on a LAN host the server must NOT tick
//! these, or every piston would fire twice (once in each world) and the server's
//! copy would fight the mirror. `mirror_host_world_state` debug-asserts the flag
//! is off. Changes the server makes land in `GameServer::pending_block_changes`,
//! which `HostedServer` drains into every `StateUpdatePacket` — the same road
//! leaf decay, falling blocks, fluids and power already take to joiners.
//!
//! Not ticked here (still host-client only): campfires, drying racks, animated
//! construction anchors, villager workstation claims. The furnace Proof-of-Play
//! trickle stays client-side (it reads `PlayerSlot` Charter / Vow / sats policy
//! state the server does not hold), and server-side keg blasts damage mobs but
//! not players (the server applies no player damage today; joiner clients run
//! their own keg sweep).

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
    /// The 4-tick-cadence machine pass, in the client loop's order (hives →
    /// growth → dispensers → pistons → furnaces → composters → keg fuses).
    /// Called from `GameServer::tick` inside the falling-block cadence block,
    /// ONLY when [`GameServer::simulates_block_machines`] is set.
    pub(crate) fn tick_block_machines(&mut self) {
        debug_assert!(
            self.simulates_block_machines,
            "tick_block_machines on a server whose host client owns the machines"
        );
        let tick = self.tick_counter;
        let raining = tick < self.weather.rain_until;

        // Hives — a bee working nearby adds honey (block-entity state only).
        crate::bee_hive::accumulate_honey(&mut self.world, &self.ecs, tick);

        // Crops + saplings.
        let growth = crate::growth::tick_growth(&mut self.world, tick, raining, self.biome_gen.seed);
        self.pending_block_changes.extend(growth.crop_changes);
        let grown = cell_changes(&self.world, &growth.grown_cells);
        self.pending_block_changes.extend(grown);

        // Dispensers / droppers — rising power edges eject.
        for order in crate::dispenser::tick_dispensers(&mut self.world) {
            let mut ctx = MachineCtx {
                world: &mut self.world,
                ecs: &mut self.ecs,
                water: &mut self.water,
                lava: &mut self.lava,
                fire: &mut self.fire,
                registry: &self.registry,
                tick,
            };
            let out = crate::dispenser::realise_order(&mut ctx, order);
            self.pending_block_changes.extend(out.changes);
        }

        // Pistons — read the energised map from the previous `power_tick`.
        let piston_cells = crate::piston::tick_pistons(&mut self.world);
        let pushed = cell_changes(&self.world, &piston_cells);
        self.pending_block_changes.extend(pushed);

        // Furnaces (lit/unlit flips broadcast) + composters (state only).
        let sweep = crate::furnace::tick_all(&mut self.world);
        self.pending_block_changes.extend(sweep.changes);
        crate::composter::tick_all(&mut self.world);

        // Blasting Keg fuses — the same gate the client applies.
        for pos in crate::power::tick_keg_fuses(&mut self.world) {
            if !crate::explosion::detonation_permitted(
                self.explosives_enabled,
                self.play_mode.can_edit_world(),
            ) {
                continue;
            }
            let blast = crate::explosion::detonate_keg_core(
                &mut self.world,
                &mut self.ecs,
                &mut self.water,
                &mut self.lava,
                &self.registry,
                pos,
            );
            self.pending_block_changes.extend(blast.changes);
        }
    }

    /// Hoppers run on their own [`crate::hopper::HOPPER_INTERVAL_TICKS`] cadence
    /// against the monotonic tick counter (outside the 4-tick block, exactly as
    /// the client loop does). Contents only — no `BlockChange`.
    pub(crate) fn tick_hoppers(&mut self) {
        debug_assert!(self.simulates_block_machines);
        crate::hopper::tick_hoppers(&mut self.world, self.tick_counter);
    }
}
