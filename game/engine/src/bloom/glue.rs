//! The one place the bloom field meets `GameState`.

use glam::Vec3;

use super::field::TickInput;
use crate::server::GameServer;
use crate::GameState;

/// Seconds one logical tick stands for (20 TPS).
const TICK_SECS: f32 = 1.0 / 20.0;

/// The ECS the bloom census reads: this client's own, or on a joined client the
/// mirror of the server's mobs (the same choice crew moods make).
fn census_ecs<'a>(joined: bool, own: &'a hecs::World, mirror: &'a hecs::World) -> &'a hecs::World {
    if joined { mirror } else { own }
}

impl GameState {
    /// Tick the bloom field once. Runs in every client mode (single-player, a
    /// host, a joined client), from this client's own world copy, ECS mobs and
    /// the player positions it knows: its local seats and the remote players it
    /// draws (`remote_players`: a host's joiners, a joiner's fellow players).
    ///
    /// Returns `true` the one time a place in this world has gone grey and the
    /// caller should fire `ChallengeEvent::PlaceWentGrey`. Never in a trial
    /// arena (a scenario running, one about to launch, a flat or Workshop world)
    /// and never on a joined client, which has no world of its own to unlock by.
    pub(crate) fn tick_bloom(&mut self) -> bool {
        // Living world S3 — feasts held, Feast Markers and Commons areas, in
        // every client mode (all read this client's own world copy).
        self.tick_gatherings();
        // Living world Tier 2 S7 — crew moods: presentation only, off this client's
        // bloom field and villagers (a joiner's are the mirrored ones).
        self.tick_crew_moods();
        let local: Vec<Vec3> = self.players.iter().map(|p| p.player.pos).collect();
        let remote: Vec<Vec3> = self
            .remote_players
            .iter()
            .map(|p| Vec3::new(p.x, p.y, p.z))
            .collect();
        let allow_grey_event = !self.joined()
            && self.scenario.is_none()
            && self.pending_scenario_launch.is_none()
            && !self.world.has_flat_floor()
            && !self.world.is_workshop;
        // A trial may set the tempo, but only in its own fresh arena world.
        let scale = self.bloom_time_scale_now();
        self.bloom.set_time_scale(scale);
        // Tier 2 S10 — a joined client has no authorship or use tables; it reads
        // the b and u its server told it.
        let told = self.remote_client.as_ref().map(|rc| rc.region_people());
        // A joiner's mobs live only in the render-only mirror (`remote_mobs`); its own
        // ECS holds none, so the census would count no wildlife there.
        let joined = self.joined();
        let ecs = census_ecs(joined, &self.ecs, self.remote_mobs.ecs());
        // A push-only joiner's own worldgen differs from its host's, so it cannot
        // regenerate the natural terrain it was sent (`NaturalCache`).
        let natural_known = !self.joined_push_only;
        let out = self.bloom.tick(&TickInput {
            world: &self.world,
            ecs,
            biome_gen: &self.biome_gen,
            registry: &self.registry,
            local_players: &local,
            remote_players: &remote,
            creative: self.is_creative,
            dt: TICK_SECS,
            allow_grey_event,
            told_people: told,
            natural_known,
        });
        out.place_went_grey
    }
}

/// How often (in logical ticks) a dedicated server tells its field where the
/// Feast Markers and Commons areas are (the client feeds on the same beat).
const SERVER_FEED_EVERY_TICKS: u64 = 10;

impl GameServer {
    /// Does this server measure bloom? Only a dedicated one: not lent (the
    /// host client has its own field) and streaming its own columns, which only
    /// the dedicated server does (`column_streamer`).
    pub(crate) fn measures_bloom(&self) -> bool {
        !self.lent && self.column_streamer.is_some()
    }

    /// Tick the dedicated server's bloom field once, round its present players.
    /// A no-op anywhere else. Reads the world and ECS and writes nothing to
    /// them; the means go to the `ConsoleSnapshot` (`world_summary`).
    pub(crate) fn tick_bloom(&mut self) {
        if !self.measures_bloom() {
            return;
        }
        if self.tick_counter.is_multiple_of(SERVER_FEED_EVERY_TICKS) {
            self.bloom.set_feast_markers(&self.world.feast_markers());
            self.bloom.set_commons(&self.world.commons_areas());
        }
        for (x, _y, z, guests) in std::mem::take(&mut self.world.feast_events) {
            self.bloom.note_feast(x, z, guests);
        }
        let here: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.is_present_and_alive())
            .map(|p| p.player.pos)
            .collect();
        self.bloom.tick(&TickInput {
            world: &self.world,
            ecs: &self.ecs,
            biome_gen: &self.biome_gen,
            registry: &self.registry,
            local_players: &here,
            remote_players: &[],
            creative: self.is_creative,
            dt: TICK_SECS,
            allow_grey_event: false,
            told_people: None,
            natural_known: true,
        });
    }
}

#[cfg(test)]
mod census_tests {
    use super::census_ecs;

    /// A joined client's mobs live only in the mirror, so that is what the
    /// census must read there; everywhere else its own ECS.
    #[test]
    fn a_joined_clients_census_reads_the_mirror() {
        let own = hecs::World::new();
        let mut mirror = hecs::World::new();
        mirror.spawn((1u8,));
        assert_eq!(census_ecs(true, &own, &mirror).len(), 1);
        assert_eq!(census_ecs(false, &own, &mirror).len(), 0);
    }
}

#[cfg(test)]
mod server_tests {
    use glam::Vec3;

    use crate::server::GameServer;

    /// Spec section 1.8: only a dedicated server (not lent, streaming its own
    /// columns) owns a ticked field; the means it reports are over the regions
    /// round its present players.
    #[test]
    fn only_a_dedicated_server_measures_bloom_round_its_players() {
        let name = "s5-server-bloom";
        let _root = crate::save::WorldsRootGuard::new(name);
        let mut s = GameServer::new(1, name.into(), 42);
        s.world.generate_column(0, 0, &s.biome_gen);
        s.players[0].player.pos = Vec3::new(8.0, 90.0, 8.0);

        // A host-owned server (no column streamer): never ticked.
        assert!(!s.measures_bloom());
        for _ in 0..10 {
            s.tick_bloom();
        }
        assert_eq!(s.bloom.world_summary().regions, 0, "a non-dedicated server measures nothing");

        // Dedicated: the field follows its player.
        s.column_streamer = Some(Default::default());
        assert!(s.measures_bloom());
        assert!(s.players[0].is_present_and_alive());
        for _ in 0..10 {
            s.tick_bloom();
        }
        let sum = s.bloom.world_summary();
        assert!(sum.regions >= 1, "{sum:?}");
        assert!((0.0..=1.0).contains(&sum.mean_level) && (0.0..=1.0).contains(&sum.mean_eco), "{sum:?}");

        // A lent world is its host client's: not measured here even with a streamer.
        s.lent = true;
        assert!(!s.measures_bloom());
    }
}
