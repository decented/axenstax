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
//! power, carts, …), and — D4 slice 2 — the clock and weather: the server's
//! clock is the world's, stepped once per logical tick by a clock phase the
//! host client asks for at the point of its tick where it used to step its
//! own (`GameServer::run_clock_phase`, phase-token guarded; `sim_clock`), and
//! read back by the client; `/time`, the bed and a trial's weather lock write
//! the server's. D4 slice 3 — and the mob simulation: the whole locomotion
//! block (brigand pre-pass → `mob_ai` → breeding → wolf follow → species AI
//! → tethers → seat steering → entity physics), the animals' products and a
//! village's population run once, in the server's tick, from the one shared
//! implementation a standalone game calls in its own (`sim_mobs`); the host
//! client hands its seats' input over before the window and drains what the
//! mobs did to its players from the server's outbox after it
//! (`GameServer::sim_events`). Since D4 slice 4, the authority also owns the
//! early death phase (`server_death`/`sim_death`), before client projectiles;
//! its local kill and rescue effects drain immediately through that outbox.
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
    /// The brigand pre-pass, the defenders' swings and `mob_ai::tick_mob_ai`
    /// (`sim_mobs::pre_pass`, `golems`, `mob_ai`).
    MobAi,
    /// The seats' steering and `entity::tick_entities`, then Satoshi's walk
    /// (`sim_mobs::steer_local_seats`, `physics`, `summon`).
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
    /// C3c-3b — laid Blueprints developing in the sun
    /// (`latent_print::tick_develop_attachments`, with the legacy
    /// `tick_develop`). A joined client runs none (C3c-3r): its copies are
    /// render stubs, and the flips reach it through the attachment stream.
    Develop,
    /// D4 slice 3 — the animals' products: Nostrich eggs and feathers,
    /// chicken eggs (`sim_mobs::products`).
    Products,
    /// D4 slice 3 — a village's population (`sim_mobs::population`, once a
    /// second).
    Population,
    /// D4 slice 3 — breeding (`sim_mobs::breed`).
    Breeding,
    /// D4 slice 3 — tamed wolves following and every species' own AI
    /// (`sim_mobs::species`).
    Species,
    /// D4 slice 3 — Lead tethers (`sim_mobs::tethers`).
    Tethers,
}

impl SimSystem {
    pub const COUNT: usize = 25;

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
        SimSystem::Develop,
        SimSystem::Products,
        SimSystem::Population,
        SimSystem::Breeding,
        SimSystem::Species,
        SimSystem::Tethers,
    ];

    /// The side that runs this system while the world is lent (module docs).
    pub const fn lent_owner(self) -> SimSide {
        match self {
            SimSystem::DespawnDead
            | SimSystem::MobAi
            | SimSystem::EntityPhysics
            | SimSystem::Products
            | SimSystem::Population
            | SimSystem::Breeding
            | SimSystem::Species
            | SimSystem::Tethers
            | SimSystem::ActiveTicks
            | SimSystem::Clock
            | SimSystem::Weather
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
            | SimSystem::EntityHealth
            | SimSystem::Develop => SimSide::Server,
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
                | SimSystem::Population
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
    /// A world frozen for a handover runs no shared system at all: every
    /// system that ran since `before` is a fault.
    pub fn frozen_tick_faults(&self, before: &SimTally) -> Vec<(SimSystem, u64)> {
        SimSystem::ALL
            .iter()
            .map(|&s| (s, self.get(s).wrapping_sub(before.get(s))))
            .filter(|&(_, runs)| runs != 0)
            .collect()
    }

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
/// `GameState` and `GameServer` both own (`lib.rs` ↔ `server.rs`), plus the
/// actual runtime clock owner of those borrowed parts.
pub struct SimParts<'a> {
    pub(crate) reputation_owner: crate::reputation_day::OwnerId,
    pub world: &'a mut World,
    pub ecs: &'a mut hecs::World,
    pub water: &'a mut WaterSystem,
    pub lava: &'a mut LavaSystem,
    pub fire: &'a mut FireSystem,
    pub leaf_decay: &'a mut LeafDecaySystem,
    pub loaded_columns: &'a mut ahash::AHashSet<(i32, i32)>,
}

/// What the host client hands its server for the window. D4 slice 2: the
/// clock and weather here are a SEED only — a server that has not taken its
/// host's clock yet adopts them (`GameServer::adopt_host_clock`); once it
/// has, the server's clock is the world's (stepped by the clock phase before
/// the window opens, read back by the client) and these are never copied
/// over it. The reputation-day offer is this tick's, as before.
#[derive(Clone, Copy, Debug)]
pub struct HostClock {
    pub world_time: u32,
    pub tick_counter: u64,
    pub(crate) reputation_owner: crate::reputation_day::OwnerId,
    pub(crate) reputation_offer: Option<crate::reputation_day::DayOffer>,
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
        server.adopt_host_clock(&crate::sim_clock::ClockSeed {
            world_time: clock.world_time,
            world_time_step: server.world_time_step,
            tick_counter: clock.tick_counter,
            weather: clock.weather,
            weather_lock_applied: None,
        });
        server.lent_day_owner = Some(parts.reputation_owner);
        server.lent_day_offer = clock.reputation_offer.filter(|offer| {
            clock.reputation_owner == parts.reputation_owner
                && offer.run_id == server.run_id && offer.pulse.owner == parts.reputation_owner
                && offer.pulse.tick == server.tick_counter
        });
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
        // The window ends the logical tick: its phase token never outlives it.
        server.clock_phase_open = false;
        server.death_phase_open = false;
        // D4 slice 3 — this tick's seats were this tick's only.
        server.lent_local_seats.clear();
        server.lent_day_offer = None;
        server.lent_day_owner = None;
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
    pub(crate) reputation_day: crate::reputation_day::DayClock,
    /// D4 slice 3 — what the server's mob simulation did to the host's
    /// players, drained from its outbox after each window as the host client
    /// drains it (`GameState::tick_hosted_server`), oldest first.
    pub events: Vec<crate::sim_mobs::SimEvent>,
}

#[cfg(test)]
impl OwnedSimParts {
    /// Take the world an OWNING server loaded (terrain, scattered mobs) out of
    /// it, as if it had been the host client's all along, and switch the
    /// server to lending. The server is left with empty parts — exactly what a
    /// lending server holds outside the window.
    pub(crate) fn take_from(hs: &mut HostedServer) -> Self {
        let s = &mut hs.server;
        let reputation_day = std::mem::replace(&mut s.reputation_day, crate::reputation_day::DayClock::new());
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
                reputation_owner: reputation_day.owner(),
                reputation_offer: None,
                weather: s.weather,
            },
            reputation_day,
            events: Vec::new(),
        };
        hs.set_host_world_for_test(crate::hosted_server::HostWorld::Lent);
        parts
    }

    /// One host tick's lend window, as `GameState::tick` +
    /// `tick_hosted_server` run it: the server's clock phase (D4 slice 2: the
    /// server's clock is the world's), read back into this stand-in's
    /// `clock`, then lend and tick the server.
    pub(crate) fn lend_tick(&mut self, hs: &mut HostedServer) {
        // A frozen handover source runs no tick at all, its clock included
        // (`GameState::tick` returns before the phase while play is held).
        #[cfg(not(target_arch = "wasm32"))]
        if hs.handover_frozen() {
            let clock = self.clock;
            LentSim::lend(hs, self.parts(), clock).tick();
            return;
        }
        let seed = crate::sim_clock::ClockSeed {
            world_time: self.clock.world_time,
            world_time_step: 1,
            tick_counter: self.clock.tick_counter,
            weather: self.clock.weather,
            weather_lock_applied: None,
        };
        let has_weather = crate::weather::world_has_weather(self.world.is_workshop);
        let back = hs
            .server
            .run_clock_phase(&seed, has_weather, None, &mut self.world.sim_tally)
            .expect("one clock phase per lend tick");
        self.clock.world_time = back.world_time;
        self.clock.tick_counter = back.tick_counter;
        self.clock.weather = back.weather;
        self.clock.reputation_offer = self.reputation_day.advance(self.clock.tick_counter, 1).filter(|pulse| pulse.days > 0).map(|pulse| crate::reputation_day::DayOffer { run_id: hs.server.run_id, pulse });
        self.death_phase(hs);
        let clock = self.clock;
        LentSim::lend(hs, self.parts(), clock).tick();
        self.events.extend(hs.server.sim_events.take());
        self.clock.reputation_offer = None;
        self.clock.weather = hs.server.weather;
    }

    /// The real host runs this early boundary before its projectile pass.
    pub(crate) fn death_phase(&mut self, hs: &mut HostedServer) {
        let locals: Vec<_> = hs.server.players.iter().map(|sp|
            (sp.player.pos, sp.server_simulated || !sp.connected || sp.combat.dead)).collect();
        hs.server.run_host_death_phase(&mut self.world, &mut self.ecs, &locals,
            (self.clock.tick_counter, self.clock.world_time), true);
        self.events.extend(hs.server.sim_events.take());
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
            reputation_owner: self.reputation_day.owner(),
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
        let reputation_day = crate::reputation_day::DayClock::new();
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
                reputation_owner: reputation_day.owner(),
                reputation_offer: None,
                weather: crate::weather::Weather::CLEAR,
            },
            reputation_day,
            events: Vec::new(),
        }
    }

    #[test]
    fn the_ownership_table_gives_the_clock_and_the_mobs_to_the_server_and_keeps_deaths_on_the_host_client() {
        use SimSide::*;
        for (s, side) in [
            // D4 slice 2 — the server's clock and weather are the world's.
            (SimSystem::Clock, Server),
            (SimSystem::Weather, Server),
            // D4 slice 3 — the mob simulation is the server's.
            (SimSystem::MobAi, Server),
            (SimSystem::EntityPhysics, Server),
            (SimSystem::Products, Server),
            (SimSystem::Population, Server),
            (SimSystem::Breeding, Server),
            (SimSystem::Species, Server),
            (SimSystem::Tethers, Server),
            (SimSystem::DespawnDead, Server),
            (SimSystem::Fluids, Server),
            (SimSystem::Power, Server),
            (SimSystem::MobSpawning, Server),
            // C3c-3b — the server's world develops laid Blueprints, so their
            // flips stream to joiners from one owner.
            (SimSystem::Develop, Server),
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

        // Lent host: the server's clock phase (before the window, tallied on
        // the host's world), its tick inside the window, the client's gates.
        let mut hs = lent_server("modes");
        assert!(hs.lends_host_world() && !hs.server.lent);
        let mut host = host_parts();
        let clock = host.clock;
        let before = host.world.sim_tally;
        hs.server
            .run_clock_phase(&seed_of(&host), true, None, &mut host.world.sim_tally)
            .expect("the phase runs");
        host.death_phase(&mut hs);
        LentSim::lend(&mut hs, host.parts(), clock).tick();
        assert!(!hs.server.lent, "cleared when the window closes");
        let server_only = host.world.sim_tally;
        assert!(
            SimSystem::ALL.iter().filter(|s| s.every_tick()).all(|s| server_only.get(*s) == before.get(*s) + 1),
            "the authority phases and window own every shared system"
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

    fn seed_of(host: &OwnedSimParts) -> crate::sim_clock::ClockSeed {
        crate::sim_clock::ClockSeed {
            world_time: host.clock.world_time,
            world_time_step: 1,
            tick_counter: host.clock.tick_counter,
            weather: host.clock.weather,
            weather_lock_applied: None,
        }
    }

    /// D4 slice 2 — the clock phase is token-guarded: one per logical tick
    /// (phase … window), a second refused with nothing stepped or tallied;
    /// the window closes the token. A lent world's phase plus its window
    /// tally Clock and Weather exactly once, and a second phase forced past
    /// the guard would show in the tally as a double run.
    #[test]
    fn a_second_clock_phase_in_one_logical_tick_is_refused() {
        let mut hs = lent_server("token");
        let mut host = host_parts();
        let seed = seed_of(&host);
        let before = host.world.sim_tally;
        let back = hs.server.run_clock_phase(&seed, true, None, &mut host.world.sim_tally).expect("first phase");
        assert_eq!((back.world_time, back.tick_counter), (1001, 5001), "adopted the host's clock, stepped once");
        assert!(hs.server.run_clock_phase(&seed, true, None, &mut host.world.sim_tally).is_none(), "second refused");
        assert_eq!(hs.server.tick_counter, 5001, "the refused phase stepped nothing");
        let clock = host.clock;
        LentSim::lend(&mut hs, host.parts(), clock).tick();
        assert_eq!(hs.server.tick_counter, 5001, "the world phase does not step the clock again");
        let after = host.world.sim_tally;
        assert_eq!(after.get(SimSystem::Clock) - before.get(SimSystem::Clock), 1);
        assert_eq!(after.get(SimSystem::Weather) - before.get(SimSystem::Weather), 1);
        // The window closed the token: the next logical tick's phase runs.
        assert!(hs.server.run_clock_phase(&seed, true, None, &mut host.world.sim_tally).is_some());
        assert_eq!(hs.server.tick_counter, 5002);
        // A double run (a phase past the guard) is what the tripwire flags.
        let mut doubled = after;
        doubled.bump(SimSystem::Clock);
        assert!(doubled.one_tick_faults(&before).contains(&(SimSystem::Clock, 2)));
    }

    /// D4 slice 2 — the lend no longer copies the host's clock over the
    /// server's: the server adopts it once, then its own clock is the
    /// world's; a host world change makes it adopt the new world's.
    #[test]
    fn the_server_adopts_the_hosts_clock_once_and_the_lend_never_overwrites_it() {
        let mut hs = lent_server("adopt");
        let mut host = host_parts();
        host.lend_tick(&mut hs);
        assert_eq!((hs.server.world_time, hs.server.tick_counter), (1001, 5001));
        // A stale client copy handed to the lend changes nothing.
        let mut stale = host.clock;
        stale.world_time = 9;
        stale.tick_counter = 9;
        stale.weather = crate::weather::Weather { rain_until: 99_999, storm_until: 99_999 };
        LentSim::lend(&mut hs, host.parts(), stale).tick();
        assert_eq!((hs.server.world_time, hs.server.tick_counter), (1001, 5001));
        assert_eq!(hs.server.weather, crate::weather::Weather::CLEAR);
        // The host changed world: the next phase takes the new world's clock.
        hs.server.forget_host_clock();
        host.clock.world_time = 7500;
        host.clock.tick_counter = 0;
        host.lend_tick(&mut hs);
        assert_eq!((hs.server.world_time, hs.server.tick_counter), (7501, 1));
        assert_eq!((host.clock.world_time, host.clock.tick_counter), (7501, 1), "read back");
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
