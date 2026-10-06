//! D1 — one world, one simulation on a host (LAN Host Game / online host).
//!
//! A host client owns the `World` + ECS it renders. When it hosts for others
//! the embedded [`GameServer`](crate::server::GameServer) keeps no second copy:
//! the client LENDS its world to the server for the one `HostedServer::tick`
//! of each logical tick ([`LentSim`]), so joiners' `StateUpdate`s are diffed
//! from the host's real world and entities, and the two-world drift is gone.
//! A dedicated server owns its world as before; single-player runs no server.
//!
//! **Who runs what.** Several world-sim systems sit in BOTH `GameState::tick`
//! and `GameServer::tick`. On one shared world each must run once, so one
//! table decides: [`SimSystem::lent_owner`]. Both sides ask it through their
//! `runs(...)` helper, which also tallies the run on the world it ran on
//! ([`SimTally`]). The game loop compares the tally across a lent tick and
//! debug-asserts every system ran at most once (and the every-tick ones
//! exactly once) — the tripwire for a missed gate, which nothing else can see
//! without a GPU.
//!
//! The table's shape: the server takes the systems whose order against the
//! client's own passes doesn't matter (fluids, fire, leaf decay, spawning,
//! power, carts, …). The client keeps the clock and weather (it owns `/time`
//! and sleeping), the mob locomotion block (brigand pre-pass → `mob_ai` →
//! entity physics: the client's species AI, wolf follow, tethers and the rest
//! overwrite `mob_ai`'s velocities BETWEEN those two passes, so splitting them
//! across the two ticks would erase every override), and the death sweep
//! (the client's single kill-attribution site feeds kill counters, the Vow,
//! raids and challenges). Each row moves to the server as the client-only
//! systems around it do (D4).
//!
//! Native only in practice: the web build has no hosted-server tick, so the
//! lend window is never opened there (the module still compiles, since
//! `GameServer::tick` and `World` carry its table and tally).
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use crate::fire::FireSystem;
use crate::hosted_server::HostedServer;
use crate::lava::LavaSystem;
use crate::leaf_decay::LeafDecaySystem;
use crate::water::WaterSystem;
use crate::world::World;

/// Which side runs a [`SimSystem`] while the host's world is lent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimSide {
    /// `GameState::tick`, on the host's machine, outside the lend window.
    HostClient,
    /// `GameServer::tick`, inside the lend window.
    Server,
}

/// A world-sim system that both `GameState::tick` and `GameServer::tick`
/// carry. Systems only one side has (block machines, species AI, breeding,
/// raids, …; remote-player physics and pickups) are not listed: they cannot
/// double-tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimSystem {
    /// `World::tick_world_clock`: the persisted active-tick total
    /// (`WorldMeta.total_ticks`) — not the day/night clock, which is
    /// [`SimSystem::Clock`].
    ActiveTicks,
    /// `world_time` and the monotonic `tick_counter`.
    Clock,
    /// The rain / storm window (`weather::advance`).
    Weather,
    /// The 400-tick spawn + sun-burn cycle.
    MobSpawning,
    /// Falling blocks (4-tick cadence).
    FallingBlocks,
    /// Water + lava spread/retract and fire (4-tick cadence; they feed each
    /// other, so they move together).
    Fluids,
    /// Leaf decay + its sapling drops (4-tick cadence).
    LeafDecay,
    /// Brigand hideout replenisher.
    Hideouts,
    /// Snowfall painter.
    Snowfall,
    /// Tapped-rubber cooldowns.
    Rubber,
    /// Salt-lick livestock regen.
    SaltLick,
    /// Bounty-board rotation.
    Bounties,
    /// Brigand AI pre-pass + `mob_ai::tick_mob_ai`.
    MobAi,
    /// `entity::tick_entities`.
    EntityPhysics,
    /// Item lifetimes + pickup delay.
    ItemLifetimes,
    /// The electricity sim (`power::power_tick`).
    Power,
    /// Rail carts (`cart::tick_carts`).
    Carts,
    /// Entity `Health` timers.
    EntityHealth,
    /// `combat::despawn_dead` + drops + hideout population.
    DespawnDead,
}

impl SimSystem {
    pub const COUNT: usize = 19;

    pub const ALL: [SimSystem; Self::COUNT] = [
        SimSystem::ActiveTicks,
        SimSystem::Clock,
        SimSystem::Weather,
        SimSystem::MobSpawning,
        SimSystem::FallingBlocks,
        SimSystem::Fluids,
        SimSystem::LeafDecay,
        SimSystem::Hideouts,
        SimSystem::Snowfall,
        SimSystem::Rubber,
        SimSystem::SaltLick,
        SimSystem::Bounties,
        SimSystem::MobAi,
        SimSystem::EntityPhysics,
        SimSystem::ItemLifetimes,
        SimSystem::Power,
        SimSystem::Carts,
        SimSystem::EntityHealth,
        SimSystem::DespawnDead,
    ];

    /// The side that runs this system while the world is lent (module docs).
    pub const fn lent_owner(self) -> SimSide {
        match self {
            SimSystem::Clock
            | SimSystem::Weather
            | SimSystem::MobAi
            | SimSystem::EntityPhysics
            | SimSystem::DespawnDead => SimSide::HostClient,
            SimSystem::ActiveTicks
            | SimSystem::MobSpawning
            | SimSystem::FallingBlocks
            | SimSystem::Fluids
            | SimSystem::LeafDecay
            | SimSystem::Hideouts
            | SimSystem::Snowfall
            | SimSystem::Rubber
            | SimSystem::SaltLick
            | SimSystem::Bounties
            | SimSystem::ItemLifetimes
            | SimSystem::Power
            | SimSystem::Carts
            | SimSystem::EntityHealth => SimSide::Server,
        }
    }

    /// Runs on every tick of a lent world (the rest run on a cadence: the
    /// 4-tick block sims, the 400-tick spawn cycle, the hideout sweep, whose
    /// client copy runs every 20 ticks).
    pub const fn every_tick(self) -> bool {
        !matches!(
            self,
            SimSystem::MobSpawning
                | SimSystem::FallingBlocks
                | SimSystem::Fluids
                | SimSystem::LeafDecay
                | SimSystem::Hideouts
        )
    }

    /// Does `side` run this system? Always, on a world that is not lent.
    pub const fn runs_on(self, side: SimSide, lent: bool) -> bool {
        !lent || matches!(
            (self.lent_owner(), side),
            (SimSide::HostClient, SimSide::HostClient) | (SimSide::Server, SimSide::Server)
        )
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// How many times each [`SimSystem`] has run on one `World`. Lives on the
/// world, so it travels with it through a lend: whichever side runs a system,
/// the count lands on the world it ran on. Never persisted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimTally([u64; SimSystem::COUNT]);

impl SimTally {
    pub fn bump(&mut self, system: SimSystem) {
        let n = &mut self.0[system.index()];
        *n = n.wrapping_add(1);
    }

    pub fn get(&self, system: SimSystem) -> u64 {
        self.0[system.index()]
    }

    /// The systems that broke "once per logical tick" between `before` and
    /// `self`: ran twice or more (a missed gate), or — an every-tick system —
    /// not at all (both sides gated it). Empty when any count went DOWN: the
    /// world was replaced mid-tick, so there is nothing to compare.
    pub fn one_tick_faults(&self, before: &SimTally) -> Vec<(SimSystem, u64)> {
        if SimSystem::ALL.iter().any(|&s| self.get(s) < before.get(s)) {
            return Vec::new();
        }
        SimSystem::ALL
            .iter()
            .map(|&s| (s, self.get(s) - before.get(s)))
            .filter(|&(s, runs)| runs > 1 || (runs == 0 && s.every_tick()))
            .collect()
    }
}

/// The host client's simulation state that is lent: exactly the seven fields
/// `GameState` and `GameServer` both own (`lib.rs` ↔ `server.rs`).
pub struct SimParts<'a> {
    pub world: &'a mut World,
    pub ecs: &'a mut hecs::World,
    pub water: &'a mut WaterSystem,
    pub lava: &'a mut LavaSystem,
    pub fire: &'a mut FireSystem,
    pub leaf_decay: &'a mut LeafDecaySystem,
    pub loaded_columns: &'a mut ahash::AHashSet<(i32, i32)>,
}

/// The host's clock for the window. The host client owns it (`/time`,
/// sleeping, its weather roll); the server reads it and never advances it
/// while lent (Q1 trap 2), so every system the server runs this tick — fire
/// burnout, rubber and hideout cooldowns, the wind sample — reads the same
/// tick the client's systems did.
#[derive(Clone, Copy, Debug)]
pub struct HostClock {
    pub world_time: u32,
    pub tick_counter: u64,
    pub weather: crate::weather::Weather,
}

/// RAII lend: swaps the host's [`SimParts`] into the hosted server's
/// `GameServer` for as long as it lives, and back on drop — including a drop
/// during a panic unwinding out of the tick, so the host never loses its
/// world. Derefs to the `HostedServer`, so the window is
/// `LentSim::lend(hs, parts, clock).tick()`.
pub struct LentSim<'a> {
    host: &'a mut HostedServer,
    parts: SimParts<'a>,
}

impl<'a> LentSim<'a> {
    pub fn lend(host: &'a mut HostedServer, mut parts: SimParts<'a>, clock: HostClock) -> Self {
        debug_assert!(host.lends_host_world(), "LentSim::lend on a server that owns its world");
        let server = &mut host.server;
        swap_parts(server, &mut parts);
        server.world_time = clock.world_time;
        server.tick_counter = clock.tick_counter;
        server.weather = clock.weather;
        server.lent = true;
        // Built before `on_lend`, so a panic there still swaps back.
        let lent = Self { host, parts };
        lent.host.on_lend();
        lent
    }
}

impl std::ops::Deref for LentSim<'_> {
    type Target = HostedServer;
    fn deref(&self) -> &HostedServer {
        self.host
    }
}

impl std::ops::DerefMut for LentSim<'_> {
    fn deref_mut(&mut self) -> &mut HostedServer {
        self.host
    }
}

impl Drop for LentSim<'_> {
    fn drop(&mut self) {
        let server = &mut self.host.server;
        swap_parts(server, &mut self.parts);
        server.lent = false;
    }
}

fn swap_parts(server: &mut crate::server::GameServer, parts: &mut SimParts<'_>) {
    std::mem::swap(&mut server.world, parts.world);
    std::mem::swap(&mut server.ecs, parts.ecs);
    std::mem::swap(&mut server.water, parts.water);
    std::mem::swap(&mut server.lava, parts.lava);
    std::mem::swap(&mut server.fire, parts.fire);
    std::mem::swap(&mut server.leaf_decay, parts.leaf_decay);
    std::mem::swap(&mut server.loaded_columns, parts.loaded_columns);
}

/// An owned set of [`SimParts`] — a stand-in for the host client's fields in
/// tests that drive a lending `HostedServer` without a `GameState`.
#[cfg(test)]
pub(crate) struct OwnedSimParts {
    pub world: World,
    pub ecs: hecs::World,
    pub water: WaterSystem,
    pub lava: LavaSystem,
    pub fire: FireSystem,
    pub leaf_decay: LeafDecaySystem,
    pub loaded_columns: ahash::AHashSet<(i32, i32)>,
    pub clock: HostClock,
}

#[cfg(test)]
impl OwnedSimParts {
    /// Take the world an OWNING server loaded (terrain, scattered mobs) out of
    /// it, as if it had been the host client's all along, and switch the
    /// server to lending. The server is left with empty parts — exactly what a
    /// lending server holds outside the window.
    pub(crate) fn take_from(hs: &mut HostedServer) -> Self {
        let s = &mut hs.server;
        let parts = Self {
            world: std::mem::replace(&mut s.world, World::new()),
            ecs: std::mem::take(&mut s.ecs),
            water: std::mem::replace(&mut s.water, WaterSystem::new()),
            lava: std::mem::replace(&mut s.lava, LavaSystem::new()),
            fire: std::mem::replace(&mut s.fire, FireSystem::new()),
            leaf_decay: std::mem::replace(&mut s.leaf_decay, LeafDecaySystem::new()),
            loaded_columns: std::mem::take(&mut s.loaded_columns),
            clock: HostClock {
                world_time: s.world_time,
                tick_counter: s.tick_counter,
                weather: s.weather,
            },
        };
        hs.set_host_world_for_test(crate::hosted_server::HostWorld::Lent);
        parts
    }

    /// One host tick's lend window: advance the host clock one tick (the
    /// client's job), lend, tick the server, return.
    pub(crate) fn lend_tick(&mut self, hs: &mut HostedServer) {
        self.clock.world_time = (self.clock.world_time + 1) % 24_000;
        self.clock.tick_counter = self.clock.tick_counter.wrapping_add(1);
        let clock = self.clock;
        LentSim::lend(hs, self.parts(), clock).tick();
    }

    /// The host client's column stream-in / stream-out state, as its
    /// `GameState::column_sims` borrows it (`registry` and `biome_gen` are the
    /// server's copies, which are never lent and match the host's).
    pub(crate) fn column_sims<'a>(
        &'a mut self,
        server: &'a crate::server::GameServer,
    ) -> crate::chunk_stream::ColumnSims<'a> {
        crate::chunk_stream::ColumnSims {
            world: &mut self.world,
            loaded: &mut self.loaded_columns,
            registry: &server.registry,
            biome_gen: &server.biome_gen,
            water: &mut self.water,
            lava: &mut self.lava,
            fire: &mut self.fire,
            ecs: &mut self.ecs,
            tick: self.clock.tick_counter,
        }
    }

    pub(crate) fn parts(&mut self) -> SimParts<'_> {
        SimParts {
            world: &mut self.world,
            ecs: &mut self.ecs,
            water: &mut self.water,
            lava: &mut self.lava,
            fire: &mut self.fire,
            leaf_decay: &mut self.leaf_decay,
            loaded_columns: &mut self.loaded_columns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lent_server(tag: &str) -> HostedServer {
        crate::hosted_server::HostedServer::start_host(
            1,
            format!("sim-lend-{tag}-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            crate::hosted_server::HostWorld::Lent,
        )
        .expect("lending server starts")
    }

    fn host_parts() -> OwnedSimParts {
        let mut world = World::new();
        world.set_block(3, 40, 5, crate::block::STONE);
        let mut loaded_columns = ahash::AHashSet::new();
        loaded_columns.insert((0, 0));
        OwnedSimParts {
            world,
            ecs: hecs::World::new(),
            water: WaterSystem::new(),
            lava: LavaSystem::new(),
            fire: FireSystem::new(),
            leaf_decay: LeafDecaySystem::new(),
            loaded_columns,
            clock: HostClock {
                world_time: 1000,
                tick_counter: 5000,
                weather: crate::weather::Weather::CLEAR,
            },
        }
    }

    #[test]
    fn the_ownership_table_keeps_the_clock_ai_and_deaths_on_the_host_client() {
        use SimSide::*;
        for (s, side) in [
            (SimSystem::Clock, HostClient),
            (SimSystem::Weather, HostClient),
            (SimSystem::MobAi, HostClient),
            (SimSystem::EntityPhysics, HostClient),
            (SimSystem::DespawnDead, HostClient),
            (SimSystem::Fluids, Server),
            (SimSystem::Power, Server),
            (SimSystem::MobSpawning, Server),
        ] {
            assert_eq!(s.lent_owner(), side, "{s:?}");
        }
        // Exactly one side runs each system on a lent world; both on an owned one.
        for s in SimSystem::ALL {
            assert_ne!(s.runs_on(HostClient, true), s.runs_on(Server, true), "{s:?}");
            assert!(s.runs_on(HostClient, false) && s.runs_on(Server, false), "{s:?}");
        }
        // ALL lists every variant once, in index order.
        for (i, s) in SimSystem::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
        }
    }

    /// D1 review fix 4, final review nit — every shared sim system runs
    /// exactly once per logical tick on each world, in every mode a server
    /// runs in. Nothing here is a literal: each row ticks a REAL
    /// `HostedServer` (`GameServer::runs` tallies what it ran on its world),
    /// and the host client's side is asked through the one predicate
    /// `GameState::sim_runs` uses — `runs_on(HostClient, hs.lends_host_world())`
    /// — fed by the real server's own lend flag. The faults are found by the
    /// very tripwire `GameState::tick_hosted_server` debug-asserts
    /// (`SimTally::one_tick_faults`):
    /// - lent host — ONE world, ticked by the server (inside the lend window)
    ///   and by the host client, so between them exactly once each;
    /// - owning host (`--no-lend`) — TWO worlds, the server's and the host
    ///   client's, each ticked once by its own side (the documented dual sim);
    /// - dedicated server — the server alone, on its own world.
    /// The single-player row is gone: with no server it reduces to
    /// `runs_on(HostClient, false)`, which is `true` by construction — a
    /// tautology; `test_game_harness` boots a real single-player `GameState`
    /// and ticks it under the same tripwire.
    #[test]
    fn every_shared_system_runs_once_per_tick_on_each_world_in_every_mode() {
        use SimSide::HostClient;
        // The host client's tally for one tick, as `sim_runs` would leave it.
        let client_tick = |hs: &HostedServer, mut tally: SimTally| {
            for s in SimSystem::ALL {
                if s.runs_on(HostClient, hs.lends_host_world()) {
                    tally.bump(s);
                }
            }
            tally
        };
        // A world ticked once between `before` and `after` has no faults, and
        // ticked at all (the check is not vacuously empty).
        let assert_clean = |mode: &str, before: &SimTally, after: &SimTally| {
            let faults = after.one_tick_faults(before);
            assert!(faults.is_empty(), "{mode}: {faults:?}");
            assert!(
                SimSystem::ALL.iter().any(|&s| s.every_tick() && after.get(s) > before.get(s)),
                "{mode}: nothing ran"
            );
        };

        // Lent host: the server's tick inside the window, the client's gates.
        let mut hs = lent_server("modes");
        assert!(hs.lends_host_world() && !hs.server.lent);
        let mut host = host_parts();
        let clock = host.clock;
        let before = host.world.sim_tally;
        LentSim::lend(&mut hs, host.parts(), clock).tick();
        assert!(!hs.server.lent, "cleared when the window closes");
        let server_only = host.world.sim_tally;
        assert!(
            SimSystem::ALL.iter().any(|&s| s.every_tick() && server_only.get(s) == before.get(s)),
            "the server alone leaves the host client's systems to the client"
        );
        assert_clean("lent host", &before, &client_tick(&hs, server_only));

        // Owning host: each side's own world sees every system once.
        let mut hs = crate::hosted_server::HostedServer::start_host(
            1,
            format!("sim-lend-owning-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            crate::hosted_server::HostWorld::Owned,
        )
        .expect("owning host starts");
        assert!(!hs.lends_host_world() && !hs.server.lent);
        let server_before = hs.server.world.sim_tally;
        hs.tick();
        assert!(!hs.server.lent, "an owning server is never lent");
        assert_clean("owning host, server world", &server_before, &hs.server.world.sim_tally);
        let client_before = SimTally::default();
        assert_clean("owning host, client world", &client_before, &client_tick(&hs, client_before));

        // Dedicated server: no host client; the server owns its world.
        let mut hs = crate::hosted_server::HostedServer::start(
            0,
            format!("sim-lend-dedicated-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        assert!(!hs.lends_host_world() && !hs.server.lent);
        let before = hs.server.world.sim_tally;
        hs.tick();
        assert_clean("dedicated server", &before, &hs.server.world.sim_tally);
        assert!(
            crate::hosted_server::HostedServer::start_host(
                0,
                format!("sim-lend-no-host-{}", std::process::id()),
                42,
                0,
                crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
                crate::hosted_server::HostWorld::Lent,
            )
            .is_err(),
            "a dedicated server has no host client to lend it a world"
        );
        // `ALL` is every variant once, in index order (the tally indexes by it).
        for (i, s) in SimSystem::ALL.iter().enumerate() {
            assert_eq!(s.index(), i, "{s:?}");
        }
    }

    #[test]
    fn the_tally_flags_a_double_run_and_a_missing_every_tick_run() {
        let before = SimTally::default();
        let mut after = before;
        for s in SimSystem::ALL.iter().filter(|s| s.every_tick()) {
            after.bump(*s);
        }
        assert!(after.one_tick_faults(&before).is_empty(), "once each: clean");

        let mut doubled = after;
        doubled.bump(SimSystem::Power);
        assert_eq!(doubled.one_tick_faults(&before), vec![(SimSystem::Power, 2)]);

        let mut missing = before;
        for s in SimSystem::ALL.iter().filter(|s| s.every_tick() && **s != SimSystem::Carts) {
            missing.bump(*s);
        }
        assert_eq!(missing.one_tick_faults(&before), vec![(SimSystem::Carts, 0)]);

        // A cadenced system may skip a tick.
        assert!(!SimSystem::Fluids.every_tick());
        // A world replaced mid-tick (counts went down) is not compared.
        assert!(SimTally::default().one_tick_faults(&doubled).is_empty());
    }

    #[test]
    fn the_lend_swaps_the_host_world_in_and_back_out_on_drop() {
        let mut hs = lent_server("swap");
        let mut host = host_parts();
        let clock = host.clock;
        {
            let lent = LentSim::lend(&mut hs, host.parts(), clock);
            assert!(lent.server.lent);
            assert_eq!(lent.server.world.get_block(3, 40, 5), crate::block::STONE, "host world inside");
            assert!(lent.server.loaded_columns.contains(&(0, 0)));
            assert_eq!(lent.server.world_time, 1000);
            assert_eq!(lent.server.tick_counter, 5000);
        }
        assert!(!hs.server.lent);
        assert_eq!(hs.server.world.get_block(3, 40, 5), crate::block::AIR, "server's own (empty) world back");
        assert!(hs.server.loaded_columns.is_empty());
        assert_eq!(host.world.get_block(3, 40, 5), crate::block::STONE, "host world home");
        assert!(host.loaded_columns.contains(&(0, 0)));
    }

    #[test]
    fn a_panic_inside_the_window_still_returns_the_host_world() {
        let mut hs = lent_server("panic");
        let mut host = host_parts();
        let clock = host.clock;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut lent = LentSim::lend(&mut hs, host.parts(), clock);
            lent.server.world.set_block(4, 40, 5, crate::block::GLASS);
            panic!("a system panics mid-tick");
        }));
        assert!(caught.is_err());
        assert!(!hs.server.lent);
        assert_eq!(host.world.get_block(3, 40, 5), crate::block::STONE);
        assert_eq!(host.world.get_block(4, 40, 5), crate::block::GLASS, "the edit made in the window is kept");
        assert_eq!(hs.server.world.get_block(3, 40, 5), crate::block::AIR);
    }
}
