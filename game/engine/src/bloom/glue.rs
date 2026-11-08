//! The one place the bloom field meets `GameState`.

use glam::Vec3;

use super::field::TickInput;
use crate::GameState;

/// Seconds one logical tick stands for (20 TPS).
const TICK_SECS: f32 = 1.0 / 20.0;

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
        let out = self.bloom.tick(&TickInput {
            world: &self.world,
            ecs: &self.ecs,
            biome_gen: &self.biome_gen,
            registry: &self.registry,
            local_players: &local,
            remote_players: &remote,
            creative: self.is_creative,
            dt: TICK_SECS,
            allow_grey_event,
        });
        out.place_went_grey
    }
}
