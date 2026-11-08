//! Living world S3 — the client half of the Feast and Commons: the local
//! right-click arms (single-player and a host's own seats), what a client does
//! when it SEES a feast held (confetti and the bloom surge), and the bloom feed
//! of Feast Markers and Commons areas.
//!
//! The rule itself is [`crate::feast`]; a joiner's request is the ordinary
//! `UseBlock` (`GameState::send_block_use`) answered by
//! `HostedServer::serve_feast`. The `fire_challenge(ChallengeEvent::HoldFeast)`
//! call stays in `game_loop.rs` (the trials lint only reads that file and
//! `machine_client.rs`), at the arm that calls [`GameState::use_feast_table`].

use web_time::{Duration, Instant};

use glam::Vec3;

use crate::feast;
use crate::item::Item;
use crate::GameState;

/// How often (in logical ticks) the bloom field is told where the Feast
/// Markers and Commons areas are. Both lists come from per-chunk counters, so
/// the walk visits only chunks that have one.
const BLOOM_FEED_EVERY_TICKS: u64 = 10;
/// A joiner's pending "the Feast of X" toast lapses after this long.
const FEAST_TOAST_WINDOW: Duration = Duration::from_secs(6);

impl GameState {
    fn say(&mut self, text: impl Into<String>, secs: u64) {
        self.toast = Some((text.into(), Instant::now() + Duration::from_secs(secs)));
    }

    /// Hold a feast at the Feast Table `pos`, as seat `pidx`, on the local
    /// path (single-player, or a host's own seat). Guests are the other local
    /// seats, the joiners this host draws (`remote_players`) and the ECS
    /// villagers; the drops are spawned in this world, which on a lent host is
    /// the world the server serves. Returns `true` when the feast was held
    /// (the caller fires `HoldFeast`). A joiner never gets here: it asks the
    /// authority (`send_block_use`).
    pub(crate) fn use_feast_table(&mut self, pidx: usize, pos: [i32; 3]) -> bool {
        let cell = (pos[0], pos[1], pos[2]);
        if !self.is_creative && crate::plot::is_in_foreign_plot(&self.world.plots, pos[0], pos[2], pidx) {
            self.say("This land belongs to another player.", 2);
            return false;
        }
        let hot = self.players[pidx].hotbar_slot;
        let held = self.players[pidx].inventory.hotbar_slot(hot).cloned();
        if matches!(held.as_ref().map(|s| &s.item), Some(Item::Plan(_))) {
            self.say("Put the Plan away first.", 2);
            return false;
        }
        let seats = self
            .players
            .iter()
            .enumerate()
            .filter(|(j, s)| *j != pidx && !s.combat.dead)
            .map(|(j, s)| (j, s.player.pos));
        let base = self.players.len();
        let joiners = self
            .remote_players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.health > 0.0)
            .map(|(k, p)| (base + k, Vec3::new(p.x, p.y, p.z)));
        let guests = feast::gather(feast::cell_centre(cell), seats.chain(joiners), feast::villagers_in(&self.ecs));
        let held_feast = match feast::hold(cell, held.as_ref(), &guests) {
            Ok(f) => f,
            Err(why) => {
                self.say(why.toast(), 3);
                return false;
            }
        };
        if !self.is_creative {
            let _ = self.players[pidx].inventory.take_slot(hot);
        }
        if let Some(stack) = &held {
            feast::spawn_drops(&mut self.ecs, stack, &held_feast);
        }
        let change = feast::convert(&mut self.world, cell, held_feast.guests);
        self.pending_block_changes.push(change);
        self.rebuild_chunk_at(pos[0], pos[1], pos[2]);
        self.audio.play_gem_pickup();
        self.say(feast::toast(&held_feast.name, held_feast.guests), 4);
        true
    }

    /// Use a Feast Marker: its name and how many guests it fed. Read from this
    /// client's own copy of the world (a joiner's meta arrived with the block).
    pub(crate) fn use_feast_marker(&mut self, pos: [i32; 3]) {
        let guests = self.world.meta_at(pos[0], pos[1], pos[2]);
        self.say(feast::toast(&feast::name_at(pos[0], pos[2]), guests), 3);
    }

    /// A joiner asked the authority to hold the feast at `cell`: show the
    /// holder's toast when the table is seen to become a marker.
    pub(crate) fn expect_feast_toast(&mut self, cell: [i32; 3]) {
        self.feast_toast = Some(((cell[0], cell[1], cell[2]), Instant::now() + FEAST_TOAST_WINDOW));
    }

    /// Feasts held in the world since last tick (a table that became a marker:
    /// this client's own, the authority's on a lent world, or a received block
    /// change): confetti at the table, the bloom surge, and the holder's toast
    /// if this client asked for it.
    fn see_feasts(&mut self) {
        let events = std::mem::take(&mut self.world.feast_events);
        if events.is_empty() {
            return;
        }
        let near = |p: Vec3, c: Vec3| p.distance_squared(c) < 96.0 * 96.0;
        for (x, y, z, guests) in events {
            let at = Vec3::new(x as f32 + 0.5, y as f32 + 1.1, z as f32 + 0.5);
            if self.players.iter().any(|s| near(s.player.pos, at)) {
                let seed = (x as u64).wrapping_mul(0x9E37_79B9) ^ (z as u64).wrapping_mul(0x85EB_CA6B) ^ self.tick_counter;
                self.particles.burst_confetti(at, 36, seed);
            }
            self.bloom.note_feast(x, z, guests);
            if let Some((cell, until)) = self.feast_toast
                && cell == (x, y, z)
            {
                self.feast_toast = None;
                if Instant::now() < until {
                    self.say(feast::toast(&feast::name_at(x, z), guests), 4);
                }
            }
        }
    }

    /// Per-tick work for the Feast and Commons, called from `tick_bloom`: see
    /// the feasts held, and keep the bloom field's lists of Feast Markers and
    /// Commons areas current.
    pub(crate) fn tick_gatherings(&mut self) {
        self.see_feasts();
        if self.tick_counter.is_multiple_of(BLOOM_FEED_EVERY_TICKS) {
            self.bloom.set_feast_markers(&self.world.feast_markers());
            self.bloom.set_commons(&self.world.commons_areas());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::block;

    #[test]
    fn the_table_and_the_marker_are_different_blocks() {
        assert_ne!(block::FEAST_TABLE, block::FEAST_MARKER);
    }
}
